//! Миры и резервные копии: zip в `servers/<id>/backups/`, восстановление,
//! импорт мира из одиночной игры, экспорт мира в zip.
//!
//! Здесь только работа с файлами. Как сделать копию на работающем сервере
//! (`save-off` → `save-all flush` → копия → `save-on`), решает `manager`.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;
use ts_rs::TS;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::{Error, Result};
use crate::fsx;

/// Суффиксы папок ада и энда у Paper (у Vanilla и Fabric они внутри мира).
const DIM_SUFFIXES: [&str; 2] = ["_nether", "_the_end"];

/// Файлы, которые не нужны в копии: их держит работающий сервер.
fn skip_file(name: &std::ffi::OsStr) -> bool {
    name == "session.lock"
}

/// Зачем сделана копия. Определяется по имени файла.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BackupKind {
    /// По расписанию.
    Auto,
    /// Кнопкой «Сделать копию».
    Manual,
    /// Перед восстановлением другой копии.
    BeforeRestore,
    /// Перед импортом другого мира.
    BeforeImport,
    /// Перед включением или выключением пиратского режима.
    Pirate,
    Other,
}

impl BackupKind {
    pub fn tag(self) -> &'static str {
        match self {
            BackupKind::Auto => "auto",
            BackupKind::Manual => "manual",
            BackupKind::BeforeRestore => "before-restore",
            BackupKind::BeforeImport => "before-import",
            BackupKind::Pirate => "pirate",
            BackupKind::Other => "other",
        }
    }

    fn from_tag(tag: &str) -> Self {
        match tag {
            "auto" => BackupKind::Auto,
            "manual" => BackupKind::Manual,
            "before-restore" => BackupKind::BeforeRestore,
            "before-import" => BackupKind::BeforeImport,
            t if t.starts_with("pirate") => BackupKind::Pirate,
            _ => BackupKind::Other,
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupInfo {
    /// Имя файла в папке бэкапов: по нему копию восстанавливают и удаляют.
    pub file: String,
    pub kind: BackupKind,
    pub created_at: DateTime<Utc>,
    #[ts(type = "number")]
    pub size_bytes: u64,
}

/// Мир из одиночной игры, найденный в папке лаунчера.
#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedWorld {
    pub name: String,
    pub path: String,
    /// Откуда мир: «Minecraft», «Prism Launcher: <сборка>» и т.п.
    pub source: String,
    pub last_played: Option<DateTime<Utc>>,
    #[ts(type = "number")]
    pub size_bytes: u64,
}

/// Папки мира: основной мир и, у Paper, отдельные папки ада и энда.
pub fn world_dirs(game_dir: &Path, level_name: &str) -> Vec<PathBuf> {
    std::iter::once(String::new())
        .chain(DIM_SUFFIXES.iter().map(|s| s.to_string()))
        .map(|suffix| game_dir.join(format!("{level_name}{suffix}")))
        .filter(|p| p.is_dir())
        .collect()
}

pub fn has_world(game_dir: &Path, level_name: &str) -> bool {
    game_dir.join(level_name).is_dir()
}

/// Упаковывает папки мира в zip. Возвращает путь к архиву или `None`, если мира
/// ещё нет (сервер ни разу не запускался).
pub async fn backup_world(game_dir: &Path, level_name: &str, backups_dir: &Path, kind: BackupKind) -> Result<Option<PathBuf>> {
    let dirs = world_dirs(game_dir, level_name);
    if dirs.is_empty() {
        return Ok(None);
    }
    tokio::fs::create_dir_all(backups_dir).await?;
    let stamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let mut dest = backups_dir.join(format!("{stamp}_{}.zip", kind.tag()));
    // Две копии в одну секунду (например, ручная сразу после авто).
    let mut n = 2;
    while dest.exists() {
        dest = backups_dir.join(format!("{stamp}_{}-{n}.zip", kind.tag()));
        n += 1;
    }
    let sources: Vec<ZipSource> = dirs
        .into_iter()
        .map(|dir| {
            let prefix = format!("{}/", dir.file_name().unwrap_or_default().to_string_lossy());
            ZipSource { dir, prefix, root_files: true }
        })
        .collect();
    let dest_clone = dest.clone();
    tokio::task::spawn_blocking(move || write_zip(&sources, &dest_clone)).await??;
    tracing::info!("Бэкап мира: {}", dest.display());
    Ok(Some(dest))
}

/// Мир одним zip для одиночной игры: папка `<name>/`, ад и энд Paper
/// переносятся внутрь мира, как их ждёт обычный Minecraft.
pub async fn export_world(game_dir: &Path, level_name: &str, name: &str, dest: &Path) -> Result<()> {
    let main = game_dir.join(level_name);
    if !main.is_dir() {
        return Err(Error::invalid("Мира ещё нет: запустите сервер хотя бы раз"));
    }
    let folder = safe_folder_name(name);
    let mut sources = vec![ZipSource { dir: main, prefix: format!("{folder}/"), root_files: true }];
    for suffix in DIM_SUFFIXES {
        let dir = game_dir.join(format!("{level_name}{suffix}"));
        if dir.is_dir() {
            // Из папки измерения Paper берём только содержимое (DIM-1/, DIM1/),
            // её level.dat и uid.dat одиночной игре не нужны.
            sources.push(ZipSource { dir, prefix: format!("{folder}/"), root_files: false });
        }
    }
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || write_zip(&sources, &dest)).await??;
    Ok(())
}

/// Имя папки мира в архиве: без символов, запрещённых в Windows.
fn safe_folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').to_string();
    if cleaned.is_empty() { "world".into() } else { cleaned }
}

struct ZipSource {
    dir: PathBuf,
    /// Путь в архиве, с `/` на конце.
    prefix: String,
    /// Класть ли файлы из корня папки (иначе только вложенные папки).
    root_files: bool,
}

fn write_zip(sources: &[ZipSource], dest: &Path) -> Result<()> {
    let mut part_name = dest.file_name().unwrap_or_default().to_os_string();
    part_name.push(".part");
    let part = dest.with_file_name(part_name);
    let result = write_zip_to(sources, &part);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    fsx::rename_with_retry_sync(&part, dest)?;
    Ok(())
}

fn write_zip_to(sources: &[ZipSource], path: &Path) -> Result<()> {
    let file = File::create(path)?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    let mut written: HashSet<String> = HashSet::new();
    let mut buf = vec![0u8; 64 * 1024];
    for source in sources {
        if written.insert(source.prefix.clone()) {
            zip.add_directory(source.prefix.clone(), options)?;
        }
        let mut stack = vec![source.dir.clone()];
        while let Some(current) = stack.pop() {
            let at_root = current == source.dir;
            for entry in std::fs::read_dir(&current)? {
                let entry = entry?;
                let path = entry.path();
                let rel = path
                    .strip_prefix(&source.dir)
                    .map_err(|_| Error::other("Внутренняя ошибка пути при упаковке мира"))?
                    .to_string_lossy()
                    .replace('\\', "/");
                let name = format!("{}{rel}", source.prefix);
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    if written.insert(format!("{name}/")) {
                        zip.add_directory(format!("{name}/"), options)?;
                    }
                    stack.push(path);
                } else if kind.is_file() {
                    if skip_file(&entry.file_name()) || (at_root && !source.root_files) || !written.insert(name.clone()) {
                        continue;
                    }
                    zip.start_file(name, options)?;
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
    Ok(())
}

// ---- список копий ----

/// Копии сервера, новые первыми.
pub fn list(backups_dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = std::fs::read_dir(backups_dir) else {
        return Vec::new();
    };
    // Время из имени точно до секунды; копии в одну секунду упорядочиваем по времени записи файла.
    let mut list: Vec<(BackupInfo, Option<std::time::SystemTime>)> = entries
        .flatten()
        .filter_map(|e| {
            let file = e.file_name().to_string_lossy().into_owned();
            if !file.to_ascii_lowercase().ends_with(".zip") {
                return None;
            }
            let meta = e.metadata().ok().filter(|m| m.is_file())?;
            let modified = meta.modified().ok();
            let (created, kind) = parse_name(&file);
            let created_at = created.or_else(|| modified.map(DateTime::<Utc>::from)).unwrap_or_else(Utc::now);
            Some((BackupInfo { file, kind, created_at, size_bytes: meta.len() }, modified))
        })
        .collect();
    list.sort_by(|(a, am), (b, bm)| {
        b.created_at.cmp(&a.created_at).then_with(|| bm.cmp(am)).then_with(|| b.file.cmp(&a.file))
    });
    list.into_iter().map(|(b, _)| b).collect()
}

/// `2026-11-23_18-00-00_auto.zip` → (время, Auto).
fn parse_name(file: &str) -> (Option<DateTime<Utc>>, BackupKind) {
    let stem = file.strip_suffix(".zip").unwrap_or(file);
    let (Some(stamp), Some(rest)) = (stem.get(..19), stem.get(20..)) else {
        return (None, BackupKind::Other);
    };
    let created = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d_%H-%M-%S")
        .ok()
        .and_then(|naive| Local.from_local_datetime(&naive).earliest())
        .map(|dt| dt.with_timezone(&Utc));
    // «auto-2» — вторая копия в ту же секунду.
    let tag = rest.rsplit_once('-').filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit())).map_or(rest, |(t, _)| t);
    (created, BackupKind::from_tag(tag))
}

/// Удаляет старые автоматические копии сверх `keep`. Ручные и защитные копии
/// не трогает: их удаляет только пользователь.
pub fn prune_auto(backups_dir: &Path, keep: usize) -> usize {
    let mut removed = 0;
    for old in list(backups_dir).into_iter().filter(|b| b.kind == BackupKind::Auto).skip(keep) {
        match std::fs::remove_file(backups_dir.join(&old.file)) {
            Ok(()) => removed += 1,
            Err(e) => tracing::warn!("Не удалось удалить старый бэкап {}: {e}", old.file),
        }
    }
    removed
}

/// Путь к копии по имени из интерфейса. Имя не должно уводить из папки бэкапов.
pub fn backup_path(backups_dir: &Path, file: &str) -> Result<PathBuf> {
    let ok = !file.is_empty()
        && !file.contains(['/', '\\', ':'])
        && file != ".."
        && file.to_ascii_lowercase().ends_with(".zip");
    let path = backups_dir.join(file);
    if !ok || !path.is_file() {
        return Err(Error::invalid("Такой резервной копии нет"));
    }
    Ok(path)
}

/// Когда сделана последняя автоматическая копия.
pub fn last_auto(backups_dir: &Path) -> Option<DateTime<Utc>> {
    list(backups_dir).into_iter().find(|b| b.kind == BackupKind::Auto).map(|b| b.created_at)
}

// ---- восстановление и импорт ----

/// Откуда взять мир: zip (копия Hearth или архив мира) или папка одиночной игры.
pub enum WorldSource<'a> {
    Zip(&'a Path),
    Folder(&'a Path),
}

/// Ставит мир из `source` на место текущего (`<level>`, `<level>_nether`,
/// `<level>_the_end`). Сначала всё распаковывается рядом во временную папку,
/// и только потом старый мир заменяется: битый архив не оставит сервер без мира.
pub async fn replace_world(source: WorldSource<'_>, game_dir: &Path, level_name: &str, staging: &Path) -> Result<()> {
    let game_dir = game_dir.to_path_buf();
    let level = level_name.to_string();
    let staging = staging.to_path_buf();
    let source = match source {
        WorldSource::Zip(p) => (p.to_path_buf(), true),
        WorldSource::Folder(p) => (p.to_path_buf(), false),
    };
    tokio::task::spawn_blocking(move || {
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        std::fs::create_dir_all(&staging)?;
        let result = (|| -> Result<()> {
            if source.1 {
                extract_world_zip(&source.0, &staging, &level)?;
            } else {
                copy_world_folder(&source.0, &staging.join(&level))?;
            }
            for old in world_dirs(&game_dir, &level) {
                std::fs::remove_dir_all(&old)?;
            }
            for entry in std::fs::read_dir(&staging)? {
                let entry = entry?;
                fsx::rename_with_retry_sync(&entry.path(), &game_dir.join(entry.file_name()))?;
            }
            Ok(())
        })();
        let _ = std::fs::remove_dir_all(&staging);
        result
    })
    .await?
}

/// Проверяет, что папка похожа на мир Minecraft.
pub fn check_world_folder(dir: &Path) -> Result<()> {
    if !dir.join("level.dat").is_file() {
        return Err(Error::invalid("В этой папке нет мира Minecraft (не найден level.dat)"));
    }
    Ok(())
}

fn copy_world_folder(src: &Path, dest: &Path) -> Result<()> {
    check_world_folder(src)?;
    let mut stack = vec![(src.to_path_buf(), dest.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        std::fs::create_dir_all(&to)?;
        for entry in std::fs::read_dir(&from)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let target = to.join(entry.file_name());
            if kind.is_dir() {
                stack.push((entry.path(), target));
            } else if kind.is_file() && !skip_file(&entry.file_name()) {
                std::fs::copy(entry.path(), target)?;
            }
        }
    }
    Ok(())
}

/// Где в архиве лежит мир: префикс папки с level.dat (`""` — в корне архива).
/// Если в архиве несколько миров с level.dat (копия Paper: `world/`,
/// `world_nether/`, `world_the_end/`), главный — не ад и не энд, ближе к корню.
fn find_world_prefix(names: &[String]) -> Option<String> {
    let mut candidates: Vec<String> = names
        .iter()
        .filter(|n| !n.starts_with("__MACOSX/"))
        .filter_map(|n| {
            if n == "level.dat" {
                Some(String::new())
            } else {
                n.strip_suffix("/level.dat").map(|p| format!("{p}/"))
            }
        })
        .collect();
    let is_dim = |p: &String| {
        let folder = p.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        DIM_SUFFIXES.iter().any(|s| folder.ends_with(s))
    };
    if candidates.iter().any(|p| !is_dim(p)) {
        candidates.retain(|p| !is_dim(p));
    }
    candidates.sort_by_key(|p| (p.matches('/').count(), p.len()));
    candidates.into_iter().next()
}

/// Безопасный относительный путь из архива (без `..`, дисков и абсолютных путей).
fn safe_relative(rel: &str) -> Option<PathBuf> {
    let path = Path::new(rel);
    path.components().all(|c| matches!(c, Component::Normal(_))).then(|| path.to_path_buf())
}

fn extract_world_zip(zip_path: &Path, staging: &Path, level: &str) -> Result<()> {
    let file = File::open(zip_path)?;
    let mut archive = ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| Error::invalid(format!("Это не zip-архив или он повреждён: {e}")))?;
    let names: Vec<String> = archive
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.replace('\\', "/")))
        .collect();
    let main = find_world_prefix(&names)
        .ok_or_else(|| Error::invalid("В архиве нет мира Minecraft (не найден level.dat)"))?;
    // Папки ада и энда Paper рядом с главной: `world_nether/` для `world/`.
    let mut mapping = vec![(main.clone(), format!("{level}/"))];
    if let Some(base) = main.strip_suffix('/') {
        for suffix in DIM_SUFFIXES {
            let sibling = format!("{base}{suffix}/");
            if names.iter().any(|n| n.starts_with(&sibling)) {
                mapping.push((sibling, format!("{level}{suffix}/")));
            }
        }
    }
    let mut buf = vec![0u8; 64 * 1024];
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name()?.replace('\\', "/");
        if name.starts_with("__MACOSX/") {
            continue;
        }
        // Для мира в корне архива (main = "") подходит всё; иначе самый длинный префикс.
        let Some((from, to)) = mapping
            .iter()
            .filter(|(from, _)| name.starts_with(from.as_str()))
            .max_by_key(|(from, _)| from.len())
        else {
            continue;
        };
        let rel = &name[from.len()..];
        if rel.is_empty() || rel.rsplit('/').next() == Some("session.lock") {
            continue;
        }
        let Some(rel) = safe_relative(&format!("{to}{rel}")) else {
            tracing::warn!("Пропускаю подозрительный путь в архиве: {name}");
            continue;
        };
        let target = staging.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = BufWriter::new(File::create(&target)?);
        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
        }
        out.flush()?;
    }
    if !staging.join(level).join("level.dat").is_file() {
        return Err(Error::invalid("Не удалось достать мир из архива"));
    }
    Ok(())
}

// ---- миры одиночной игры ----

/// Ищет миры одиночной игры: обычный лаунчер (и TLauncher, у него та же
/// папка `.minecraft`), Prism/MultiMC, Modrinth App, CurseForge.
pub fn saved_worlds() -> Vec<SavedWorld> {
    let mut roots: Vec<(PathBuf, String)> = Vec::new();
    if let Some(appdata) = dirs::config_dir() {
        roots.push((appdata.join(".minecraft").join("saves"), "Minecraft".into()));
        for (launcher, title) in [("PrismLauncher", "Prism Launcher"), ("MultiMC", "MultiMC")] {
            for (instance, dir) in subdirs(&appdata.join(launcher).join("instances")) {
                for mc in [".minecraft", "minecraft"] {
                    roots.push((dir.join(mc).join("saves"), format!("{title}: {instance}")));
                }
            }
        }
        for (profile, dir) in subdirs(&appdata.join("ModrinthApp").join("profiles")) {
            roots.push((dir.join("saves"), format!("Modrinth App: {profile}")));
        }
    }
    if let Some(home) = dirs::home_dir() {
        for (instance, dir) in subdirs(&home.join("curseforge").join("minecraft").join("Instances")) {
            roots.push((dir.join("saves"), format!("CurseForge: {instance}")));
        }
    }
    let mut worlds: Vec<SavedWorld> = roots
        .into_iter()
        .flat_map(|(saves, source)| {
            subdirs(&saves)
                .into_iter()
                .filter(|(_, dir)| dir.join("level.dat").is_file())
                .map(move |(name, dir)| SavedWorld {
                    last_played: std::fs::metadata(dir.join("level.dat"))
                        .and_then(|m| m.modified())
                        .ok()
                        .map(DateTime::<Utc>::from),
                    size_bytes: dir_size(&dir),
                    path: dir.to_string_lossy().into_owned(),
                    name,
                    source: source.clone(),
                })
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.last_played));
    worlds
}

fn subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect()
}

pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(t) if t.is_file() => total += entry.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_names(path: &Path) -> Vec<String> {
        let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
        (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().to_string()).collect()
    }

    fn paper_world(game: &Path) {
        std::fs::create_dir_all(game.join("world/playerdata")).unwrap();
        std::fs::create_dir_all(game.join("world_nether/DIM-1/region")).unwrap();
        std::fs::write(game.join("world/level.dat"), b"level").unwrap();
        std::fs::write(game.join("world/session.lock"), b"lock").unwrap();
        std::fs::write(game.join("world/playerdata/a.dat"), b"player").unwrap();
        std::fs::write(game.join("world_nether/level.dat"), b"nether-level").unwrap();
        std::fs::write(game.join("world_nether/DIM-1/region/r.0.0.mca"), b"region").unwrap();
        std::fs::write(game.join("server.jar"), b"jar").unwrap();
    }

    #[tokio::test]
    async fn backs_up_all_world_folders() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("server");
        paper_world(&game);

        let backups = dir.path().join("backups");
        let zip_path = backup_world(&game, "world", &backups, BackupKind::Auto).await.unwrap().unwrap();
        assert!(zip_path.file_name().unwrap().to_string_lossy().ends_with("_auto.zip"));
        let names = zip_names(&zip_path);
        assert!(names.contains(&"world/level.dat".to_string()));
        assert!(names.contains(&"world/playerdata/a.dat".to_string()));
        assert!(names.contains(&"world_nether/DIM-1/region/r.0.0.mca".to_string()));
        assert!(!names.iter().any(|n| n.ends_with("session.lock") || n == "server.jar"));

        // Вторая копия в ту же секунду не затирает первую.
        let second = backup_world(&game, "world", &backups, BackupKind::Auto).await.unwrap().unwrap();
        assert_ne!(second, zip_path);
        let listed = list(&backups);
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|b| b.kind == BackupKind::Auto && b.size_bytes > 0));
    }

    #[tokio::test]
    async fn no_world_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        assert!(backup_world(dir.path(), "world", &dir.path().join("b"), BackupKind::Manual).await.unwrap().is_none());
    }

    #[test]
    fn names_are_parsed() {
        let (at, kind) = parse_name("2026-11-23_18-05-00_auto.zip");
        assert_eq!(kind, BackupKind::Auto);
        assert_eq!(at.unwrap().with_timezone(&Local).format("%H:%M").to_string(), "18:05");
        assert_eq!(parse_name("2026-11-23_18-05-00_auto-2.zip").1, BackupKind::Auto);
        assert_eq!(parse_name("2026-11-23_18-05-00_pirate-on.zip").1, BackupKind::Pirate);
        assert_eq!(parse_name("2026-11-23_18-05-00_before-restore.zip").1, BackupKind::BeforeRestore);
        assert_eq!(parse_name("my world.zip"), (None, BackupKind::Other));
    }

    #[test]
    fn prune_keeps_newest_auto_and_all_manual() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "2026-11-20_10-00-00_auto.zip",
            "2026-11-21_10-00-00_auto.zip",
            "2026-11-22_10-00-00_auto.zip",
            "2026-11-19_10-00-00_manual.zip",
        ] {
            std::fs::write(dir.path().join(name), b"zip").unwrap();
        }
        assert_eq!(prune_auto(dir.path(), 2), 1);
        let left: Vec<String> = list(dir.path()).into_iter().map(|b| b.file).collect();
        assert_eq!(
            left,
            ["2026-11-22_10-00-00_auto.zip", "2026-11-21_10-00-00_auto.zip", "2026-11-19_10-00-00_manual.zip"]
        );
        assert!(last_auto(dir.path()).is_some());
    }

    #[test]
    fn backup_path_stays_in_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.zip"), b"zip").unwrap();
        assert!(backup_path(dir.path(), "a.zip").is_ok());
        assert!(backup_path(dir.path(), "..\\a.zip").is_err());
        assert!(backup_path(dir.path(), "../a.zip").is_err());
        assert!(backup_path(dir.path(), "C:a.zip").is_err());
        assert!(backup_path(dir.path(), "missing.zip").is_err());
    }

    #[test]
    fn world_prefix_in_archives() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(find_world_prefix(&names(&["level.dat", "region/r.0.0.mca"])), Some(String::new()));
        assert_eq!(
            find_world_prefix(&names(&["Мой мир/level.dat", "Мой мир/DIM-1/region/r.mca", "Мой мир/datapacks/x/level.dat"])),
            Some("Мой мир/".into())
        );
        assert_eq!(
            find_world_prefix(&names(&["world_nether/level.dat", "world/level.dat", "world_the_end/level.dat"])),
            Some("world/".into())
        );
        assert_eq!(find_world_prefix(&names(&["readme.txt"])), None);
    }

    #[tokio::test]
    async fn restore_replaces_world_with_backup() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("server");
        paper_world(&game);
        let backups = dir.path().join("backups");
        let zip = backup_world(&game, "world", &backups, BackupKind::Manual).await.unwrap().unwrap();

        // Мир изменился: новый файл, пропал ад.
        std::fs::write(game.join("world/new.dat"), b"new").unwrap();
        std::fs::remove_dir_all(game.join("world_nether")).unwrap();

        replace_world(WorldSource::Zip(&zip), &game, "world", &dir.path().join("staging")).await.unwrap();
        assert!(!game.join("world/new.dat").exists());
        assert_eq!(std::fs::read(game.join("world/playerdata/a.dat")).unwrap(), b"player");
        assert!(game.join("world_nether/DIM-1/region/r.0.0.mca").is_file());
        assert!(game.join("server.jar").is_file(), "файлы сервера не тронуты");
        assert!(!dir.path().join("staging").exists());
    }

    #[tokio::test]
    async fn import_singleplayer_folder_and_zip() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("server");
        paper_world(&game);

        // Папка мира из .minecraft/saves: ад внутри мира, свой session.lock.
        let save = dir.path().join("saves/Остров");
        std::fs::create_dir_all(save.join("DIM-1/region")).unwrap();
        std::fs::write(save.join("level.dat"), b"island").unwrap();
        std::fs::write(save.join("session.lock"), b"lock").unwrap();
        std::fs::write(save.join("DIM-1/region/r.0.0.mca"), b"hell").unwrap();
        replace_world(WorldSource::Folder(&save), &game, "world", &dir.path().join("staging")).await.unwrap();
        assert_eq!(std::fs::read(game.join("world/level.dat")).unwrap(), b"island");
        assert!(game.join("world/DIM-1/region/r.0.0.mca").is_file());
        assert!(!game.join("world/session.lock").exists());
        assert!(!game.join("world_nether").exists(), "старый ад удалён вместе со старым миром");

        // Zip, где мир лежит во вложенной папке.
        let zip_path = dir.path().join("world.zip");
        {
            let mut zip = ZipWriter::new(File::create(&zip_path).unwrap());
            let o = SimpleFileOptions::default();
            zip.start_file("download/Город/level.dat", o).unwrap();
            zip.write_all(b"city").unwrap();
            zip.start_file("download/Город/region/r.0.0.mca", o).unwrap();
            zip.write_all(b"r").unwrap();
            zip.start_file("download/readme.txt", o).unwrap();
            zip.write_all(b"hi").unwrap();
            zip.start_file("download/Город/../../evil.txt", o).unwrap();
            zip.write_all(b"evil").unwrap();
            zip.finish().unwrap();
        }
        replace_world(WorldSource::Zip(&zip_path), &game, "world", &dir.path().join("staging")).await.unwrap();
        assert_eq!(std::fs::read(game.join("world/level.dat")).unwrap(), b"city");
        assert!(game.join("world/region/r.0.0.mca").is_file());
        assert!(!game.join("world/DIM-1").exists());
        assert!(!game.join("readme.txt").exists() && !dir.path().join("evil.txt").exists());
    }

    #[tokio::test]
    async fn broken_import_keeps_old_world() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("server");
        paper_world(&game);
        let not_zip = dir.path().join("x.zip");
        std::fs::write(&not_zip, b"not a zip").unwrap();
        assert!(replace_world(WorldSource::Zip(&not_zip), &game, "world", &dir.path().join("s")).await.is_err());
        let empty = dir.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(replace_world(WorldSource::Folder(&empty), &game, "world", &dir.path().join("s")).await.is_err());
        assert!(game.join("world/level.dat").is_file());
        assert!(game.join("world_nether/DIM-1/region/r.0.0.mca").is_file());
    }

    #[tokio::test]
    async fn export_puts_paper_dimensions_inside_world() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("server");
        paper_world(&game);
        let dest = dir.path().join("out.zip");
        export_world(&game, "world", "Сервер: друзья", &dest).await.unwrap();
        let names = zip_names(&dest);
        assert!(names.contains(&"Сервер_ друзья/level.dat".to_string()), "{names:?}");
        assert!(names.contains(&"Сервер_ друзья/DIM-1/region/r.0.0.mca".to_string()));
        // level.dat ада Paper не затирает level.dat мира.
        let mut archive = ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let mut level = String::new();
        archive.by_name("Сервер_ друзья/level.dat").unwrap().read_to_string(&mut level).unwrap();
        assert_eq!(level, "level");
        assert!(!dir.path().join("out.zip.part").exists());
    }
}
