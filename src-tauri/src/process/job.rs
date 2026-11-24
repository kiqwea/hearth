//! Windows Job Object: если Hearth упадёт или его закроют через диспетчер задач,
//! Windows сама завершит процессы серверов, и они не останутся висеть без хозяина.
//!
//! Обычный выход из Hearth сначала останавливает серверы командой `stop`;
//! Job Object срабатывает только как страховка.
//!
//! У job есть имя: процесс-сторож (`crate::watchdog`) открывает его и держит свой
//! дескриптор. Если Hearth упадёт, job живёт, пока сторож сохраняет миры, и
//! закрывается (завершая всё, что осталось) только вместе со сторожем.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::io::RawHandle;
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows::core::HSTRING;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, OpenJobObjectW,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    /// Право JOB_OBJECT_QUERY: сторожу достаточно просто держать job открытым.
    const JOB_OBJECT_QUERY: u32 = 0x0004;

    pub struct Job {
        handle: HANDLE,
        name: String,
    }

    // Дескриптор Job Object можно использовать из любого потока.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> std::io::Result<Self> {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let name = format!(
                "Local\\Hearth-servers-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            unsafe {
                let handle = CreateJobObjectW(None, &HSTRING::from(name.as_str())).map_err(to_io)?;
                let job = Job { handle, name };
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job.handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .map_err(to_io)?;
                Ok(job)
            }
        }

        /// Открывает существующий job по имени (так делает сторож). Пока этот
        /// дескриптор открыт, процессы job не завершатся из-за падения Hearth.
        pub fn open(name: &str) -> std::io::Result<Self> {
            unsafe {
                let handle = OpenJobObjectW(JOB_OBJECT_QUERY, false, &HSTRING::from(name)).map_err(to_io)?;
                Ok(Job { handle, name: name.to_string() })
            }
        }

        pub fn name(&self) -> &str {
            &self.name
        }

        pub fn assign(&self, process: RawHandle) -> std::io::Result<()> {
            unsafe { AssignProcessToJobObject(self.handle, HANDLE(process)).map_err(to_io) }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.handle);
            }
        }
    }

    fn to_io(e: windows::core::Error) -> std::io::Error {
        std::io::Error::other(e.message())
    }
}

#[cfg(not(windows))]
mod imp {
    /// На других ОС Job Object нет; Hearth собирается только под Windows,
    /// заглушка нужна, чтобы код компилировался.
    pub struct Job;

    impl Job {
        pub fn new() -> std::io::Result<Self> {
            Ok(Job)
        }

        pub fn open(_name: &str) -> std::io::Result<Self> {
            Ok(Job)
        }

        pub fn name(&self) -> &str {
            ""
        }
    }
}

pub use imp::Job;

#[cfg(all(test, windows))]
mod tests {
    use std::os::windows::io::AsRawHandle;
    use std::time::{Duration, Instant};

    use super::Job;

    /// Закрытие последнего дескриптора Job Object (так происходит, когда процесс
    /// Hearth падает или его убивают) завершает все процессы в нём.
    #[test]
    fn closing_job_kills_assigned_process() {
        let job = Job::new().expect("job created");
        // ping с большим числом повторов живёт ~30 секунд, если его не убить.
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("ping started");
        job.assign(child.as_raw_handle()).expect("assigned to job");
        assert!(child.try_wait().unwrap().is_none(), "процесс работает до закрытия job");

        drop(job);

        let started = Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(5), "процесс пережил закрытие job");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn is_alive(pid: u32) -> bool {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .expect("tasklist");
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
    }

    /// Вспомогательный «Hearth»: создаёт job, запускает в нём долгий процесс,
    /// печатает его PID и ждёт, пока его убьют. Запускается только из теста ниже.
    #[test]
    fn helper_holds_job() {
        let Ok(pid_file) = std::env::var("HEARTH_JOB_HELPER") else {
            return;
        };
        let job = Job::new().unwrap();
        let child = std::process::Command::new("ping")
            .args(["-n", "60", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        job.assign(child.as_raw_handle()).unwrap();
        std::fs::write(pid_file, child.id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(60));
    }

    /// Как в жизни: процесс Hearth убит снаружи (диспетчер задач, падение),
    /// и Windows сама завершает сервер, который он запускал.
    #[test]
    fn killing_owner_process_kills_server() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let mut helper = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["process::job::tests::helper_holds_job", "--exact", "--test-threads=1"])
            .env("HEARTH_JOB_HELPER", &pid_file)
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("helper started");
        let started = Instant::now();
        let pid: u32 = loop {
            if let Some(pid) = std::fs::read_to_string(&pid_file).ok().and_then(|s| s.trim().parse().ok()) {
                break pid;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "помощник не запустил процесс");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(is_alive(pid), "процесс сервера запущен");

        helper.kill().expect("owner killed"); // TerminateProcess, как «Снять задачу»
        let _ = helper.wait();

        let started = Instant::now();
        while is_alive(pid) {
            assert!(started.elapsed() < Duration::from_secs(5), "процесс пережил смерть владельца");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
