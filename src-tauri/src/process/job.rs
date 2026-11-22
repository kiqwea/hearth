//! Windows Job Object: если Hearth упадёт или его закроют через диспетчер задач,
//! Windows сама завершит процессы серверов, и они не останутся висеть без хозяина.
//!
//! Обычный выход из Hearth сначала останавливает серверы командой `stop`;
//! Job Object срабатывает только как страховка.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::os::windows::io::RawHandle;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Job(HANDLE);

    // Дескриптор Job Object можно использовать из любого потока.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> std::io::Result<Self> {
            unsafe {
                let handle = CreateJobObjectW(None, PCWSTR::null()).map_err(to_io)?;
                let job = Job(handle);
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .map_err(to_io)?;
                Ok(job)
            }
        }

        pub fn assign(&self, process: RawHandle) -> std::io::Result<()> {
            unsafe { AssignProcessToJobObject(self.0, HANDLE(process)).map_err(to_io) }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
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
