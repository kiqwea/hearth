//! Где лежит секретный ключ агента playit.
//!
//! В Windows это Диспетчер учётных данных (Credential Manager): ключ не
//! попадает ни в файлы Hearth, ни в логи. Для тестов есть хранилище в памяти.

use std::sync::Mutex;

use crate::error::Result;

pub trait SecretStore: Send + Sync {
    fn load(&self) -> Result<Option<String>>;
    fn save(&self, secret: &str) -> Result<()>;
    fn delete(&self) -> Result<()>;
}

/// Хранилище в памяти: для тестов.
#[derive(Default)]
pub struct MemoryStore(Mutex<Option<String>>);

impl SecretStore for MemoryStore {
    fn load(&self) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap_or_else(|p| p.into_inner()).clone())
    }

    fn save(&self, secret: &str) -> Result<()> {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(secret.to_string());
        Ok(())
    }

    fn delete(&self) -> Result<()> {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(())
    }
}

#[cfg(windows)]
pub use windows_store::CredentialStore;

#[cfg(windows)]
mod windows_store {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_NOT_FOUND;
    use windows::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    use super::SecretStore;
    use crate::error::{Error, Result};

    /// Запись в Диспетчере учётных данных Windows («Учётные данные Windows» → «Общие»).
    pub struct CredentialStore {
        target: Vec<u16>,
        user: Vec<u16>,
    }

    impl CredentialStore {
        pub fn new(target: &str) -> Self {
            Self {
                target: wide(target),
                user: wide("playit-agent"),
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn not_found(e: &windows::core::Error) -> bool {
        e.code() == ERROR_NOT_FOUND.to_hresult()
    }

    fn to_error(action: &str, e: windows::core::Error) -> Error {
        Error::other(format!("Не удалось {action} ключ playit в диспетчере учётных данных Windows: {}", e.message()))
    }

    impl SecretStore for CredentialStore {
        fn load(&self) -> Result<Option<String>> {
            let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
            unsafe {
                if let Err(e) = CredReadW(PCWSTR(self.target.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred) {
                    return if not_found(&e) { Ok(None) } else { Err(to_error("прочитать", e)) };
                }
                let c = &*cred;
                let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
                CredFree(cred as *const _);
                let secret = String::from_utf8(bytes).map_err(|_| Error::other("Ключ playit в диспетчере учётных данных повреждён"))?;
                Ok(Some(secret).filter(|s| !s.trim().is_empty()))
            }
        }

        fn save(&self, secret: &str) -> Result<()> {
            let mut blob = secret.as_bytes().to_vec();
            let mut target = self.target.clone();
            let mut user = self.user.clone();
            let cred = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target.as_mut_ptr()),
                CredentialBlobSize: blob.len() as u32,
                CredentialBlob: blob.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                UserName: PWSTR(user.as_mut_ptr()),
                ..Default::default()
            };
            unsafe { CredWriteW(&cred, 0) }.map_err(|e| to_error("сохранить", e))
        }

        fn delete(&self) -> Result<()> {
            match unsafe { CredDeleteW(PCWSTR(self.target.as_ptr()), CRED_TYPE_GENERIC, None) } {
                Ok(()) => Ok(()),
                Err(e) if not_found(&e) => Ok(()),
                Err(e) => Err(to_error("удалить", e)),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn roundtrip_in_credential_manager() {
            let store = CredentialStore::new(&format!("Hearth/test-{}", uuid::Uuid::new_v4()));
            assert_eq!(store.load().unwrap(), None);
            store.save("secret-123").unwrap();
            assert_eq!(store.load().unwrap().as_deref(), Some("secret-123"));
            store.delete().unwrap();
            assert_eq!(store.load().unwrap(), None);
            store.delete().unwrap();
        }
    }
}
