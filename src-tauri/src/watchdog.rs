//! Процесс-сторож: сохраняет миры, если Hearth упал.
//!
//! Hearth при запуске стартует копию самого себя с аргументом `--watchdog` и
//! держит её стандартный ввод. После каждого запуска и остановки сервера Hearth
//! пишет туда строку JSON со списком работающих серверов (PID, порт и пароль
//! RCON). Сторож молча читает.
//!
//! Если ввод закрылся, Hearth завершился. При обычном выходе Hearth сначала
//! останавливает серверы, и последний список пуст: сторожу делать нечего. Если
//! же список не пуст, Hearth упал или его убили: сторож отправляет каждому
//! серверу `stop` через RCON и ждёт, пока тот сохранит мир и завершится.
//!
//! Серверы при этом не умирают вместе с Hearth: сторож открывает тот же Job
//! Object и держит его, а когда закончит, закрывает — и Windows завершает всё,
//! что осталось (например, агент playit). Если сторожа тоже убили, всё как
//! раньше: Job Object завершает серверы сразу.

use std::io::{BufRead, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::process::job::Job;
use crate::rcon::Rcon;

/// Аргумент командной строки, с которым Hearth запускается как сторож.
pub const ARG: &str = "--watchdog";

/// Сколько ждать, пока поднимется RCON (сервер мог упасть вместе с Hearth ещё на запуске).
const RCON_WAIT: Duration = Duration::from_secs(120);
/// Сколько ждать сохранения мира после `stop`.
const SAVE_WAIT: Duration = Duration::from_secs(120);

/// Работающий сервер, за которым присматривает сторож.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Guarded {
    pub id: String,
    pub pid: u32,
    pub rcon_port: u16,
    pub rcon_password: String,
}

/// Сторона Hearth: процесс-сторож и канал к нему.
pub struct Watchdog {
    command: Mutex<Command>,
    child: Mutex<Option<(Child, ChildStdin)>>,
}

impl Watchdog {
    /// Запускает сторожа. `command` — как его запустить (обычно Hearth.exe
    /// `--watchdog <имя job>`); stdin/stdout/stderr настраиваются здесь.
    pub fn spawn(mut command: Command) -> std::io::Result<Self> {
        command.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let watchdog = Self { command: Mutex::new(command), child: Mutex::new(None) };
        watchdog.respawn()?;
        Ok(watchdog)
    }

    /// Команда запуска сторожа из этого же exe.
    pub fn self_command(job_name: &str) -> std::io::Result<Command> {
        let mut command = Command::new(std::env::current_exe()?);
        command.arg(ARG).arg(job_name);
        Ok(command)
    }

    fn respawn(&self) -> std::io::Result<()> {
        let mut child = self.command.lock().unwrap_or_else(|p| p.into_inner()).spawn()?;
        let stdin = child.stdin.take().expect("stdin piped");
        if let Some((mut old, _)) = self.child.lock().unwrap_or_else(|p| p.into_inner()).replace((child, stdin)) {
            let _ = old.kill();
            let _ = old.wait();
        }
        Ok(())
    }

    /// Сообщает сторожу актуальный список работающих серверов. Если сторож
    /// куда-то пропал, запускает нового.
    pub fn update(&self, servers: &[Guarded]) {
        let line = match serde_json::to_string(servers) {
            Ok(line) => line + "\n",
            Err(_) => return,
        };
        let write = |this: &Self| -> std::io::Result<()> {
            let mut guard = this.child.lock().unwrap_or_else(|p| p.into_inner());
            let (_, stdin) = guard.as_mut().ok_or_else(|| std::io::Error::other("сторож не запущен"))?;
            stdin.write_all(line.as_bytes())?;
            stdin.flush()
        };
        if write(self).is_ok() {
            return;
        }
        tracing::warn!("Сторож не отвечает, запускаю заново");
        if let Err(e) = self.respawn().and_then(|_| write(self)) {
            tracing::warn!("Не удалось запустить сторожа: {e}");
        }
    }
}

/// Сторона сторожа: основной цикл процесса `Hearth.exe --watchdog <job>`.
pub fn serve(job_name: &str) {
    // Держим job открытым, пока не закончим: тогда серверы не завершатся вместе с Hearth.
    let job = match Job::open(job_name) {
        Ok(job) => Some(job),
        Err(e) => {
            tracing::warn!("Сторож: не удалось открыть job {job_name}: {e}");
            None
        }
    };

    let mut last: Vec<Guarded> = Vec::new();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if let Ok(servers) = serde_json::from_str::<Vec<Guarded>>(&line) {
            last = servers;
        }
    }

    if last.is_empty() {
        return;
    }
    tracing::warn!("Hearth завершился, а серверы работают: сохраняю миры ({} шт.)", last.len());
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            tracing::error!("Сторож: не удалось создать runtime: {e}");
            return;
        }
    };
    runtime.block_on(async {
        futures_util::future::join_all(last.iter().map(stop_server)).await;
    });
    drop(job);
}

/// Отправляет серверу `stop` через RCON и ждёт, пока он завершится.
async fn stop_server(server: &Guarded) {
    let started = Instant::now();
    loop {
        if !is_alive(server.pid) {
            tracing::info!("Сторож: сервер {} уже завершился", server.id);
            return;
        }
        match Rcon::connect(server.rcon_port, &server.rcon_password).await {
            Ok(mut rcon) => {
                // Ответа на stop может не быть: сервер закрывает соединение.
                let _ = rcon.command("stop").await;
                tracing::info!("Сторож: сервер {} получил stop", server.id);
                break;
            }
            Err(e) if started.elapsed() < RCON_WAIT => {
                tracing::debug!("Сторож: RCON сервера {} пока недоступен: {e}", server.id);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                tracing::warn!("Сторож: не удалось достучаться до сервера {}: {e}", server.id);
                return;
            }
        }
    }
    let waited = Instant::now();
    while is_alive(server.pid) {
        if waited.elapsed() > SAVE_WAIT {
            tracing::warn!("Сторож: сервер {} не завершился за {:?}", server.id, SAVE_WAIT);
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    tracing::info!("Сторож: сервер {} сохранил мир и завершился", server.id);
}

#[cfg(windows)]
fn is_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
            return false;
        };
        let alive = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        let _ = CloseHandle(handle);
        alive
    }
}

#[cfg(not(windows))]
fn is_alive(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_survives_json_roundtrip() {
        let list = vec![Guarded { id: "a".into(), pid: 42, rcon_port: 25575, rcon_password: "p\"q".into() }];
        let line = serde_json::to_string(&list).unwrap();
        assert!(!line.contains('\n'), "одна строка на список");
        assert_eq!(serde_json::from_str::<Vec<Guarded>>(&line).unwrap(), list);
    }

    #[cfg(windows)]
    #[test]
    fn detects_dead_process() {
        assert!(is_alive(std::process::id()));
        let mut child = Command::new("cmd").args(["/C", "exit"]).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        // Пока дескриптор `child` открыт, процесс-«зомби» ещё виден, но уже завершён.
        assert!(!is_alive(pid));
    }
}
