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
