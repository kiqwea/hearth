//! Управление серверами: установка, запуск, остановка, консоль.
//!
//! Правила, которые здесь соблюдаются:
//! - сервер останавливается только командой `stop` с ожиданием сохранения мира;
//!   принудительное завершение — отдельная операция `kill`;
//! - процесс привязан к Job Object, чтобы не пережить падение Hearth;
//! - перед запуском проверяется, свободен ли порт.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{oneshot, watch};
use ts_rs::TS;

use crate::error::{Error, Result};
use crate::events::{
    ConsoleEvent, Event, EventSink, InstallEvent, InstallProgress, InstallStage, StatusEvent,
};
use crate::java::{self, JavaManager};
use crate::net::Http;
use crate::paths::Paths;
use crate::process::{self, job::Job, ConsoleBuffer, ConsoleLine, ConsoleStream, ServerStatus};
use crate::providers;
use crate::servers::{
    self, CreateServerRequest, PropertyEntry, ServerMeta, ServerPatch, ServerStore,
};

/// Сколько ждём сохранения мира после `stop`, прежде чем предложить принудительное завершение.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerSummary {
    pub meta: ServerMeta,
    pub status: ServerStatus,
}

struct RunningServer {
    stdin: tokio::sync::Mutex<ChildStdin>,
    kill_tx: Mutex<Option<oneshot::Sender<()>>>,
    /// Пользователь (или Hearth) попросил остановку: выход процесса не считается падением.
    stop_requested: AtomicBool,
    exited: watch::Receiver<bool>,
    port: u16,
}

#[derive(Default)]
struct Inner {
    running: HashMap<String, Arc<RunningServer>>,
    /// Серверы, которые сейчас устанавливаются, запускаются или удаляются.
    busy: HashSet<String>,
    statuses: HashMap<String, ServerStatus>,
    consoles: HashMap<String, ConsoleBuffer>,
}

pub struct ServerManager {
    http: Http,
    java: JavaManager,
    store: RwLock<ServerStore>,
    sink: Arc<dyn EventSink>,
    inner: Mutex<Inner>,
    job: Option<Job>,
}

/// Снимает отметку «занят», когда операция закончилась (в том числе с ошибкой).
struct BusyGuard<'a> {
    manager: &'a ServerManager,
    id: String,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.manager.lock().busy.remove(&self.id);
    }
}

impl ServerManager {
    pub fn new(paths: &Paths, http: Http, servers_dir: PathBuf, sink: Arc<dyn EventSink>) -> Arc<Self> {
        let job = match Job::new() {
            Ok(job) => Some(job),
            Err(e) => {
                tracing::warn!("Не удалось создать Job Object: {e}");
                None
            }
        };
        Arc::new(Self {
            java: JavaManager::new(paths.java_dir(), &paths.cache_dir(), http.clone()),
            http,
            store: RwLock::new(ServerStore::new(servers_dir)),
            sink,
            inner: Mutex::new(Inner::default()),
            job,
        })
    }

    pub fn http(&self) -> &Http {
        &self.http
    }

    pub fn java(&self) -> &JavaManager {
        &self.java
    }

    pub fn store(&self) -> ServerStore {
        self.store.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Смена корневой папки серверов. Серверы не переносятся: интерфейс должен
    /// предупредить, что старые останутся в прежней папке.
    pub fn set_servers_dir(&self, dir: PathBuf) -> Result<()> {
        {
            let inner = self.lock();
            if !inner.running.is_empty() || !inner.busy.is_empty() {
                return Err(Error::invalid(
                    "Остановите все серверы, прежде чем менять папку серверов",
                ));
            }
        }
        *self.store.write().unwrap_or_else(|p| p.into_inner()) = ServerStore::new(dir);
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn begin_busy(&self, id: &str) -> Result<BusyGuard<'_>> {
        let mut inner = self.lock();
        if inner.running.contains_key(id) {
            return Err(Error::AlreadyRunning);
        }
        if !inner.busy.insert(id.to_string()) {
            return Err(Error::Busy);
        }
        Ok(BusyGuard {
            manager: self,
            id: id.to_string(),
        })
    }

    fn running(&self, id: &str) -> Option<Arc<RunningServer>> {
        self.lock().running.get(id).cloned()
    }

    pub fn has_running(&self) -> bool {
        !self.lock().running.is_empty()
    }

    pub fn status(&self, id: &str) -> ServerStatus {
        self.lock()
            .statuses
            .get(id)
            .cloned()
            .unwrap_or(ServerStatus::Stopped)
    }

    fn set_status(&self, id: &str, status: ServerStatus) {
        self.lock().statuses.insert(id.to_string(), status.clone());
        self.sink.emit(Event::Status(StatusEvent {
            server_id: id.to_string(),
            status,
        }));
    }

    fn push_line(&self, id: &str, stream: ConsoleStream, text: String) {
        let line = self
            .lock()
            .consoles
            .entry(id.to_string())
            .or_default()
            .push(stream, text);
        self.sink.emit(Event::Console(ConsoleEvent {
            server_id: id.to_string(),
            line,
        }));
    }

    fn emit_install(&self, id: &str, progress: InstallProgress) {
        emit_install(&self.sink, id, progress);
    }

    pub fn console(&self, id: &str) -> Vec<ConsoleLine> {
        self.lock()
            .consoles
            .get(id)
            .map(ConsoleBuffer::snapshot)
            .unwrap_or_default()
    }

    // ---- серверы на диске ----

    pub fn list(&self) -> Vec<ServerSummary> {
        self.store()
            .list()
            .into_iter()
            .map(|meta| ServerSummary {
                status: self.status(&meta.id),
                meta,
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<ServerSummary> {
        let meta = self.store().load(id)?;
        Ok(ServerSummary {
            status: self.status(id),
            meta,
        })
    }

    pub async fn create(&self, req: CreateServerRequest) -> Result<ServerMeta> {
        self.store().create(req).await
    }

    pub async fn update(&self, id: &str, patch: ServerPatch) -> Result<ServerMeta> {
        let store = self.store();
        let mut meta = store.load(id)?;
        if let Some(name) = patch.name {
            meta.name = servers::validate_name(&name)?;
        }
        if let Some(mb) = patch.memory_mb {
            meta.memory_mb = servers::validate_memory(mb)?;
        }
        store.save(&meta).await?;
        Ok(meta)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let _guard = self.begin_busy(id)?;
        self.store().delete(id).await?;
        let mut inner = self.lock();
        inner.statuses.remove(id);
        inner.consoles.remove(id);
        Ok(())
    }

    pub fn properties(&self, id: &str) -> Result<Vec<PropertyEntry>> {
        Ok(self
            .store()
            .read_properties(id)?
            .entries()
            .into_iter()
            .map(|(key, value)| PropertyEntry { key, value })
            .collect())
    }

    /// Меняет server.properties. На работающем сервере изменения вступят в силу
    /// после перезапуска.
    pub async fn set_properties(&self, id: &str, entries: Vec<PropertyEntry>) -> Result<()> {
        let store = self.store();
        let mut meta = store.load(id)?;
        let mut props = store.read_properties(id)?;
        let mut port_changed = false;
        for entry in entries {
            let key = entry.key.trim();
            if key.is_empty() || key.contains(['\n', '\r']) || entry.value.contains(['\n', '\r']) {
                return Err(Error::invalid(format!("Некорректная настройка «{}»", entry.key)));
            }
            if key == "server-port" {
                let port: u16 = entry
                    .value
                    .trim()
                    .parse()
                    .ok()
                    .filter(|p| *p >= 1024)
                    .ok_or_else(|| Error::invalid("Порт должен быть числом от 1024 до 65535"))?;
                if port != meta.port {
                    meta.port = port;
                    port_changed = true;
                }
            }
            props.set(key, &entry.value);
        }
        store.write_properties(id, &props).await?;
        if port_changed {
            store.save(&meta).await?;
        }
        Ok(())
    }

    // ---- установка ----

    pub async fn install(&self, id: &str) -> Result<ServerMeta> {
        let _guard = self.begin_busy(id)?;
        self.install_locked(id).await
    }

    /// Установка, когда отметка «занят» уже взята вызывающим.
    async fn install_locked(&self, id: &str) -> Result<ServerMeta> {
        self.set_status(id, ServerStatus::Installing);
        let result = self.do_install(id).await;
        match &result {
            Ok(_) => self.emit_install(id, InstallProgress::step(InstallStage::Done, "Сервер установлен")),
            Err(e) => {
                self.emit_install(id, InstallProgress::step(InstallStage::Failed, e.to_string()));
                self.push_line(id, ConsoleStream::System, format!("Установка не удалась: {e}"));
            }
        }
        self.set_status(id, ServerStatus::Stopped);
        result
    }

    async fn do_install(&self, id: &str) -> Result<ServerMeta> {
        let store = self.store();
        let mut meta = store.load(id)?;
        let title = format!("{} {}", meta.loader.display_name(), meta.game_version);

        self.emit_install(
            id,
            InstallProgress::step(InstallStage::Resolving, format!("Получаю данные о {title}")),
        );
        let artifact = providers::resolve(&self.http, meta.loader, &meta.game_version).await?;

        let java_msg = format!("Подготавливаю Java {}", artifact.java_major);
        self.emit_install(id, InstallProgress::step(InstallStage::Java, java_msg.clone()));
        let (java_major, _) = self
            .java
            .ensure(artifact.java_major, progress_reporter(&self.sink, id, InstallStage::Java, java_msg))
            .await?;

        let game_dir = store.game_dir(id);
        for spec in artifact.extra.iter().chain(std::iter::once(&artifact.jar)) {
            let msg = format!("Скачиваю {}", spec.file_name);
            self.emit_install(id, InstallProgress::step(InstallStage::Server, msg.clone()));
            self.http
                .download(
                    &spec.url,
                    &game_dir.join(&spec.file_name),
                    spec.checksum.as_ref(),
                    progress_reporter(&self.sink, id, InstallStage::Server, msg),
                )
                .await?;
        }

        meta.jar = Some(artifact.jar.file_name.clone());
        meta.java_major = Some(java_major);
        meta.jvm_flags = artifact.jvm_flags;
        meta.loader_version = artifact.loader_version;
        meta.installed = true;
        store.save(&meta).await?;
        Ok(meta)
    }

    // ---- запуск и остановка ----

    pub async fn start(self: &Arc<Self>, id: &str) -> Result<()> {
        let _guard = self.begin_busy(id)?;
        let store = self.store();
        let mut meta = store.load(id)?;
        let accepted_at = meta.eula_accepted_at.ok_or(Error::EulaNotAccepted)?;
        let game_dir = store.game_dir(id);

        let jar_missing = meta
            .jar
            .as_ref()
            .is_none_or(|jar| !game_dir.join(jar).is_file());
        if !meta.installed || jar_missing {
            meta = self.install_locked(id).await?;
        }
        let jar = meta
            .jar
            .clone()
            .ok_or_else(|| Error::other("Не удалось определить файл сервера"))?;

        // Источник правды для порта — server.properties: его могли поправить вручную.
        let port = store
            .read_properties(id)?
            .get("server-port")
            .and_then(|p| p.trim().parse::<u16>().ok())
            .unwrap_or(meta.port);
        if self.lock().running.values().any(|r| r.port == port) || !port_is_free(port) {
            return Err(Error::PortBusy(port));
        }

        servers::write_eula(&game_dir, accepted_at).await?;
        let required = meta
            .java_major
            .unwrap_or_else(|| java::fallback_major(&meta.game_version));
        let (java_major, java_exe) = self.java.ensure(required, |_, _| {}).await?;
        let args = process::build_args(&meta, &jar);

        if let Some(console) = self.lock().consoles.get_mut(id) {
            console.clear();
        }
        self.push_line(
            id,
            ConsoleStream::System,
            format!(
                "Запуск {} {} (Java {java_major}, память {} МБ, порт {port})",
                meta.loader.display_name(),
                meta.game_version,
                meta.memory_mb
            ),
        );
        self.set_status(id, ServerStatus::Starting);

        let mut cmd = tokio::process::Command::new(&java_exe);
        cmd.args(&args)
            .current_dir(&game_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.push_line(id, ConsoleStream::System, format!("Не удалось запустить Java: {e}"));
                self.set_status(id, ServerStatus::Stopped);
                return Err(Error::other(format!("Не удалось запустить Java: {e}")));
            }
        };

        #[cfg(windows)]
        if let (Some(job), Some(handle)) = (&self.job, child.raw_handle()) {
            if let Err(e) = job.assign(handle) {
                tracing::warn!("Не удалось привязать сервер {id} к Job Object: {e}");
            }
        }

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let (exit_tx, exit_rx) = watch::channel(false);
        let running = Arc::new(RunningServer {
            stdin: tokio::sync::Mutex::new(stdin),
            kill_tx: Mutex::new(Some(kill_tx)),
            stop_requested: AtomicBool::new(false),
            exited: exit_rx,
            port,
        });
        self.lock().running.insert(id.to_string(), running.clone());

        let out_task = self.spawn_reader(id, stdout, ConsoleStream::Stdout);
        let err_task = self.spawn_reader(id, stderr, ConsoleStream::Stderr);

        let this = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let kill = async move {
                // Ошибка значит, что отправитель уничтожен без команды: ждём обычного выхода.
                if kill_rx.await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            let exit = tokio::select! {
                status = child.wait() => status,
                _ = kill => {
                    let _ = child.kill().await;
                    child.wait().await
                }
            };
            // Даём дочитать последние строки вывода.
            let _ = tokio::time::timeout(Duration::from_secs(3), async {
                let _ = out_task.await;
                let _ = err_task.await;
            })
            .await;

            let code = exit.ok().and_then(|s| s.code());
            let requested = running.stop_requested.load(Ordering::SeqCst);
            this.lock().running.remove(&id);
            let (status, message) = if requested || code == Some(0) {
                (ServerStatus::Stopped, "Сервер остановлен".to_string())
            } else {
                let code_text = code.map_or("неизвестен".to_string(), |c| c.to_string());
                (
                    ServerStatus::Crashed { code },
                    format!("Сервер неожиданно завершился (код {code_text}). Подробности в строках выше."),
                )
            };
            this.push_line(&id, ConsoleStream::System, message);
            this.set_status(&id, status);
            let _ = exit_tx.send(true);
        });
        Ok(())
    }

    fn spawn_reader<R>(self: &Arc<Self>, id: &str, reader: R, stream: ConsoleStream) -> tokio::task::JoinHandle<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let this = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut buf = Vec::with_capacity(256);
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf).await {
                    Ok(0) => break,
                    Ok(_) => this.on_output(&id, stream, process::decode_line(&buf)),
                    Err(e) => {
                        tracing::warn!("Чтение вывода сервера {id} прервано: {e}");
                        break;
                    }
                }
            }
        })
    }

    fn on_output(&self, id: &str, stream: ConsoleStream, text: String) {
        let ready = process::is_ready_line(&text);
        let hint = process::hint_for_line(&text);
        self.push_line(id, stream, text);
        if let Some(hint) = hint {
            self.push_line(id, ConsoleStream::System, hint.to_string());
        }
        if ready && self.status(id) == ServerStatus::Starting {
            self.set_status(id, ServerStatus::Running);
        }
    }

    /// Отправляет `stop` и ждёт, пока сервер сохранит мир и завершится.
    pub async fn stop(&self, id: &str, timeout: Duration) -> Result<()> {
        let running = self.running(id).ok_or(Error::NotRunning)?;
        if !running.stop_requested.swap(true, Ordering::SeqCst) {
            self.set_status(id, ServerStatus::Stopping);
            self.push_line(id, ConsoleStream::Input, "stop".into());
            if let Err(e) = write_line(&running, "stop").await {
                tracing::warn!("Не удалось отправить stop серверу {id}: {e}");
            }
        }
        wait_exit(&running, timeout).await
    }

    /// Принудительное завершение. Интерфейс вызывает его только после
    /// предупреждения о том, что несохранённый прогресс может пропасть.
    pub fn kill(&self, id: &str) -> Result<()> {
        let running = self.running(id).ok_or(Error::NotRunning)?;
        running.stop_requested.store(true, Ordering::SeqCst);
        self.push_line(id, ConsoleStream::System, "Принудительное завершение сервера".into());
        if let Some(tx) = running.kill_tx.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = tx.send(());
        }
        Ok(())
    }

    pub async fn restart(self: &Arc<Self>, id: &str) -> Result<()> {
        if self.running(id).is_some() {
            self.stop(id, STOP_TIMEOUT).await?;
        }
        self.start(id).await
    }

    pub async fn send_command(&self, id: &str, command: &str) -> Result<()> {
        let command = command.lines().next().unwrap_or("").trim();
        // В консоли сервера слэш не нужен, а старые версии на него ругаются.
        let command = command.strip_prefix('/').unwrap_or(command).trim();
        if command.is_empty() {
            return Err(Error::invalid("Введите команду"));
        }
        let running = self.running(id).ok_or(Error::NotRunning)?;
        if command.eq_ignore_ascii_case("stop") && !running.stop_requested.swap(true, Ordering::SeqCst) {
            self.set_status(id, ServerStatus::Stopping);
        }
        self.push_line(id, ConsoleStream::Input, command.to_string());
        write_line(&running, command)
            .await
            .map_err(|e| Error::other(format!("Не удалось отправить команду: {e}")))
    }

    /// Останавливает все серверы при выходе из приложения. Сервер, который не
    /// успел сохраниться за `timeout`, завершается принудительно.
    pub async fn stop_all(&self, timeout: Duration) {
        let ids: Vec<String> = self.lock().running.keys().cloned().collect();
        let tasks = ids.iter().map(|id| async move {
            if let Err(e) = self.stop(id, timeout).await {
                tracing::warn!("Сервер {id} не остановился ({e}), завершаю принудительно");
                if let Some(running) = self.running(id) {
                    let _ = self.kill(id);
                    let _ = wait_exit(&running, Duration::from_secs(10)).await;
                }
            }
        });
        futures_util::future::join_all(tasks).await;
    }
}

async fn write_line(running: &RunningServer, line: &str) -> std::io::Result<()> {
    let mut stdin = running.stdin.lock().await;
    stdin.write_all(format!("{line}\n").as_bytes()).await?;
    stdin.flush().await
}

async fn wait_exit(running: &RunningServer, timeout: Duration) -> Result<()> {
    let mut rx = running.exited.clone();
    let waited = tokio::time::timeout(timeout, async move {
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    waited.map_err(|_| Error::StopTimeout)
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).is_ok()
}

fn emit_install(sink: &Arc<dyn EventSink>, id: &str, progress: InstallProgress) {
    sink.emit(Event::Install(InstallEvent {
        server_id: id.to_string(),
        progress,
    }));
}

/// Колбэк прогресса скачивания, который превращает байты в события установки.
fn progress_reporter(
    sink: &Arc<dyn EventSink>,
    id: &str,
    stage: InstallStage,
    message: String,
) -> impl FnMut(u64, Option<u64>) + Send + use<> {
    let sink = sink.clone();
    let id = id.to_string();
    move |downloaded, total| {
        emit_install(
            &sink,
            &id,
            InstallProgress {
                stage,
                message: message.clone(),
                downloaded: Some(downloaded),
                total,
            },
        );
    }
}
