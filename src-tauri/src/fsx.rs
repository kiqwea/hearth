//! Мелкие файловые помощники.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Пишет файл через временный и переименование, чтобы при сбое не остался
/// наполовину записанный `server.json` или `config.json`.
pub async fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = tmp_path(path);
    tokio::fs::write(&tmp, data).await?;
    rename_with_retry(&tmp, path).await
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    path.with_file_name(name)
}

/// На Windows антивирус или индексатор иногда держат только что созданный
/// файл, и переименование падает с «Отказано в доступе». Пробуем несколько раз.
pub async fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match tokio::fs::rename(from, to).await {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 10 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Синхронный вариант для кода внутри `spawn_blocking`.
pub fn rename_with_retry_sync(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 10 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(300));
            }
            Err(e) => return Err(e),
        }
    }
}
