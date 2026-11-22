//! Резервные копии мира: zip в `servers/<id>/backups/`.
//!
//! Пока Hearth делает их на остановленном сервере (перед пиратским режимом и
//! другими опасными изменениями). Бэкапы по расписанию на работающем сервере
//! (`save-off` → `save-all flush` → копия → `save-on`) появятся позже.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::error::{Error, Result};

/// Папки мира: основной мир и, у Paper, отдельные папки ада и энда.
pub fn world_dirs(game_dir: &Path, level_name: &str) -> Vec<PathBuf> {
    [String::new(), "_nether".into(), "_the_end".into()]
        .iter()
        .map(|suffix| game_dir.join(format!("{level_name}{suffix}")))
        .filter(|p| p.is_dir())
        .collect()
}

/// Упаковывает папки мира в zip. Возвращает путь к архиву или `None`, если мира
/// ещё нет (сервер ни разу не запускался).
pub async fn backup_world(game_dir: &Path, level_name: &str, backups_dir: &Path, reason: &str) -> Result<Option<PathBuf>> {
    let dirs = world_dirs(game_dir, level_name);
    if dirs.is_empty() {
        return Ok(None);
    }
    tokio::fs::create_dir_all(backups_dir).await?;
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let name: String = reason
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    let dest = backups_dir.join(format!("{stamp}_{name}.zip"));
    let game_dir = game_dir.to_path_buf();
    let dest_clone = dest.clone();
    tokio::task::spawn_blocking(move || write_zip(&game_dir, &dirs, &dest_clone)).await??;
    tracing::info!("Бэкап мира: {}", dest.display());
    Ok(Some(dest))
}

fn write_zip(base: &Path, dirs: &[PathBuf], dest: &Path) -> Result<()> {
    let part = dest.with_extension("zip.part");
    let file = File::create(&part)?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    let mut buf = vec![0u8; 64 * 1024];
    for dir in dirs {
        let mut stack = vec![dir.clone()];
        while let Some(current) = stack.pop() {
            for entry in std::fs::read_dir(&current)? {
                let entry = entry?;
                let path = entry.path();
                let rel = path
                    .strip_prefix(base)
                    .map_err(|_| Error::other("Внутренняя ошибка пути при бэкапе"))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    zip.add_directory(format!("{rel}/"), options)?;
                    stack.push(path);
                } else if kind.is_file() {
                    // session.lock держит работающий сервер; в архиве он не нужен.
                    if path.file_name().is_some_and(|n| n == "session.lock") {
                        continue;
                    }
                    zip.start_file(rel, options)?;
                    let mut src = File::open(&path)?;
                    loop {
                        let n = src.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        zip.write_all(&buf[..n])?;
                    }
                }
            }
        }
    }
    zip.finish()?.flush()?;
    std::fs::rename(&part, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn backs_up_all_world_folders() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("server");
        std::fs::create_dir_all(game.join("world/playerdata")).unwrap();
        std::fs::create_dir_all(game.join("world_nether/DIM-1")).unwrap();
        std::fs::write(game.join("world/level.dat"), b"level").unwrap();
        std::fs::write(game.join("world/session.lock"), b"lock").unwrap();
        std::fs::write(game.join("world/playerdata/a.dat"), b"player").unwrap();
        std::fs::write(game.join("world_nether/DIM-1/r.0.0.mca"), b"region").unwrap();
        std::fs::write(game.join("server.jar"), b"jar").unwrap();

        let zip_path = backup_world(&game, "world", &dir.path().join("backups"), "pirate on")
            .await
            .unwrap()
            .unwrap();
        assert!(zip_path.file_name().unwrap().to_string_lossy().ends_with("_pirate-on.zip"));
        let mut archive = zip::ZipArchive::new(File::open(&zip_path).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().to_string()).collect();
        assert!(names.contains(&"world/level.dat".to_string()));
        assert!(names.contains(&"world/playerdata/a.dat".to_string()));
        assert!(names.contains(&"world_nether/DIM-1/r.0.0.mca".to_string()));
        assert!(!names.iter().any(|n| n.ends_with("session.lock") || n == "server.jar"));
    }

    #[tokio::test]
    async fn no_world_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        assert!(backup_world(dir.path(), "world", &dir.path().join("b"), "x").await.unwrap().is_none());
    }
}
