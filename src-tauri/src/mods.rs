//! Плагины (Paper, Purpur) и моды (Fabric) через Modrinth.
//!
//! Источник правды — сама папка `plugins` или `mods`: Hearth не ведёт свой
//! список, а опознаёт jar по sha1 через Modrinth. Так видны и обновляются и
//! файлы, которые пользователь положил вручную. Без интернета список всё равно
//! показывается, просто по именам файлов.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use ts_rs::TS;

use crate::error::{Error, Result};
use crate::modrinth::{self, ModFile};
use crate::net::Http;
use crate::providers::Loader;

/// Сколько уровней зависимостей ставим (зависимость зависимости и т.д.).
const MAX_DEPTH: usize = 4;

/// Нужен ли мод игрокам. Для плагинов всегда `ServerOnly`.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ClientSide {
    /// Без него игроки не смогут зайти.
    Required,
    /// Игрокам можно поставить, но необязательно.
    Optional,
    /// Только на сервере.
    ServerOnly,
    Unknown,
}

impl ClientSide {
    fn from_modrinth(loader: Loader, value: &str) -> Self {
        if loader.is_paper_like() {
            return ClientSide::ServerOnly;
        }
        match value {
            "required" => ClientSide::Required,
            "optional" => ClientSide::Optional,
            "unsupported" => ClientSide::ServerOnly,
            _ => ClientSide::Unknown,
        }
    }
}

/// Плагин или мод в папке сервера.
#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledMod {
    /// Имя файла: по нему мод удаляют.
    pub file: String,
    /// Проект Modrinth, если файл оттуда.
    pub project_id: Option<String>,
    pub slug: Option<String>,
    pub title: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub version: Option<String>,
    pub client_side: ClientSide,
    /// Новая версия под эту версию игры, если вышла.
    pub update: Option<String>,
    /// Защита ников, которую поставил Hearth: убирается в настройках сервера, а не здесь.
    pub nick_protection: bool,
    /// Скины Ely.by, которые поставил Hearth: тоже убираются в настройках сервера.
    pub ely_skins: bool,
    #[ts(type = "number")]
    pub size_bytes: u64,
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModList {
    pub items: Vec<InstalledMod>,
    /// Удалось ли спросить Modrinth (иначе названия — просто имена файлов).
    pub online: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModHit {
    pub project_id: String,
    pub slug: Option<String>,
    pub title: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub author: String,
    #[ts(type = "number")]
    pub downloads: u64,
    pub client_side: ClientSide,
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModSearch {
    pub hits: Vec<ModHit>,
    pub total: u32,
}

/// Папка для плагинов или модов. У Vanilla их нет.
pub fn folder_name(loader: Loader) -> Option<&'static str> {
    match loader {
        Loader::Paper | Loader::Purpur => Some("plugins"),
        Loader::Fabric | Loader::NeoForge | Loader::Forge => Some("mods"),
        Loader::Vanilla => None,
    }
}

/// Загрузчики Modrinth, подходящие ядру; первый — самый «родной».
pub fn modrinth_loaders(loader: Loader) -> &'static [&'static str] {
    match loader {
        Loader::Paper => &["paper", "purpur", "spigot", "bukkit"],
        Loader::Purpur => &["purpur", "paper", "spigot", "bukkit"],
        Loader::Fabric => &["fabric"],
        Loader::NeoForge => &["neoforge"],
        Loader::Forge => &["forge"],
        Loader::Vanilla => &[],
    }
}

fn project_type(loader: Loader) -> &'static str {
    if loader.is_paper_like() { "plugin" } else { "mod" }
}

fn require_folder(loader: Loader) -> Result<&'static str> {
    folder_name(loader).ok_or_else(|| Error::invalid("На Vanilla нельзя ставить моды и плагины: выберите Paper, Purpur или Fabric"))
}

/// Jar-файлы в папке с их sha1 и размером.
fn scan(folder: &Path) -> Result<Vec<(String, String, u64)>> {
    use sha1::{Digest, Sha1};
    use std::io::Read;
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".jar") || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let mut file = std::fs::File::open(entry.path())?;
        let mut hasher = Sha1::new();
        let mut size = 0u64;
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            size += n as u64;
            hasher.update(&buf[..n]);
        }
        out.push((name, hex::encode(hasher.finalize().as_slice()), size));
    }
    out.sort_by_key(|(name, _, _)| name.to_lowercase());
    Ok(out)
}

async fn scan_async(folder: &Path) -> Result<Vec<(String, String, u64)>> {
    let folder = folder.to_path_buf();
    tokio::task::spawn_blocking(move || scan(&folder)).await?
}

/// Название по имени файла, если Modrinth о нём не знает: `worldedit-7.3.jar` → `worldedit-7.3`.
fn title_from_file(file: &str) -> String {
    file.strip_suffix(".jar").or_else(|| file.strip_suffix(".JAR")).unwrap_or(file).to_string()
}

/// Что лежит в папке сервера, с названиями, иконками и доступными обновлениями.
pub async fn list(http: &Http, loader: Loader, game_version: &str, game_dir: &Path) -> Result<ModList> {
    let folder = game_dir.join(require_folder(loader)?);
    let files = scan_async(&folder).await?;
    let hashes: Vec<String> = files.iter().map(|(_, h, _)| h.clone()).collect();

    let looked_up = async {
        let known = modrinth::versions_by_hash(http, &hashes).await?;
        let mut ids: Vec<&str> = known.values().map(|v| v.project_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        let projects = modrinth::projects(http, &ids).await?;
        let latest = modrinth::latest_by_hash(http, &hashes, modrinth_loaders(loader), game_version)
            .await
            .unwrap_or_default();
        Ok::<_, Error>((known, projects, latest))
    }
    .await;
    let (known, projects, latest, online) = match looked_up {
        Ok((k, p, l)) => (k, p, l, true),
        Err(e) => {
            tracing::warn!("Modrinth недоступен, показываю моды по именам файлов: {e}");
            (HashMap::new(), Vec::new(), HashMap::new(), false)
        }
    };
    let projects: HashMap<&str, &modrinth::Project> = projects.iter().map(|p| (p.id.as_str(), p)).collect();

    let mut items: Vec<InstalledMod> = files
        .into_iter()
        .map(|(file, hash, size_bytes)| {
            let version = known.get(&hash);
            let project = version.and_then(|v| projects.get(v.project_id.as_str()));
            let update = match (version, latest.get(&hash)) {
                (Some(cur), Some(new)) if new.id != cur.id => Some(new.version_number.clone()),
                _ => None,
            };
            InstalledMod {
                project_id: version.map(|v| v.project_id.clone()),
                slug: project.and_then(|p| p.slug.clone()),
                title: project.map(|p| p.title.clone()).unwrap_or_else(|| title_from_file(&file)),
                description: project.map(|p| p.description.clone()).unwrap_or_default(),
                icon_url: project.and_then(|p| p.icon_url.clone()),
                version: version.map(|v| v.version_number.clone()),
                client_side: project
                    .map(|p| ClientSide::from_modrinth(loader, &p.client_side))
                    .unwrap_or(if loader.is_paper_like() { ClientSide::ServerOnly } else { ClientSide::Unknown }),
                update,
                nick_protection: false,
                ely_skins: false,
                size_bytes,
                file,
            }
        })
        .collect();
    items.sort_by_key(|m| m.title.to_lowercase());
    Ok(ModList { items, online })
}

/// Поиск на Modrinth. Пустой запрос — самые популярные под это ядро и версию.
pub async fn search(http: &Http, loader: Loader, game_version: &str, query: &str, offset: u32) -> Result<ModSearch> {
    require_folder(loader)?;
    let resp = modrinth::search(http, query, modrinth_loaders(loader), project_type(loader), game_version, offset, 20).await?;
    Ok(ModSearch {
        total: resp.total_hits,
        hits: resp
            .hits
            .into_iter()
            .map(|h| ModHit {
                client_side: ClientSide::from_modrinth(loader, &h.client_side),
                project_id: h.project_id,
                slug: h.slug,
                title: h.title,
                description: h.description,
                icon_url: h.icon_url,
                author: h.author,
                downloads: h.downloads,
            })
            .collect(),
    })
}

/// Подходящий файл проекта: сначала сборка под «родной» загрузчик, потом совместимые.
pub async fn find_file(http: &Http, loader: Loader, game_version: &str, project: &str) -> Result<Option<ModFile>> {
    let loaders = modrinth_loaders(loader);
    if let Some(file) = modrinth::latest_file(http, project, &loaders[..1], game_version).await? {
        return Ok(Some(file));
    }
    if loaders.len() > 1 {
        return modrinth::latest_file(http, project, loaders, game_version).await;
    }
    Ok(None)
}

/// Скачанный файл: какой проект и какой старый файл он заменил.
#[derive(Debug, Clone, PartialEq)]
pub struct InstalledFile {
    pub project_id: String,
    pub file: String,
    pub replaced: Option<String>,
}

/// Ставит проект с обязательными зависимостями. Если проект уже стоит другой
/// версией, это обновление: старый файл удаляется после скачивания нового.
/// Возвращает скачанные файлы (пусто, если всё уже стояло); первый — сам проект.
pub async fn install(
    http: &Http,
    loader: Loader,
    game_version: &str,
    game_dir: &Path,
    project: &str,
    allow_replace: bool,
) -> Result<Vec<InstalledFile>> {
    let folder = game_dir.join(require_folder(loader)?);
    tokio::fs::create_dir_all(&folder).await?;

    // Какие проекты уже стоят и в каких файлах.
    let files = scan_async(&folder).await?;
    let hashes: Vec<String> = files.iter().map(|(_, h, _)| h.clone()).collect();
    let known = modrinth::versions_by_hash(http, &hashes).await.unwrap_or_default();
    let mut present: HashMap<String, (String, String)> = HashMap::new(); // проект → (файл, версия)
    for (file, hash, _) in &files {
        if let Some(v) = known.get(hash) {
            present.insert(v.project_id.clone(), (file.clone(), v.id.clone()));
        }
    }

    let mut installed = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: Vec<(String, usize)> = vec![(project.to_string(), 0)];
    while let Some((id, depth)) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let file = find_file(http, loader, game_version, &id).await?;
        let Some(file) = file else {
            if depth == 0 {
                return Err(Error::invalid(format!(
                    "Нет версии под {} {game_version}. Возможно, автор ещё не обновил его.",
                    loader.display_name()
                )));
            }
            tracing::warn!("Зависимость {id} не найдена под {} {game_version}", loader.display_name());
            continue;
        };
        // Обычный ответ API — id проекта, а пользователь мог передать slug: сверяем по id из файла.
        let existing = present.get(&file.project_id).cloned();
        let up_to_date = existing.as_ref().is_some_and(|(_, version)| *version == file.version_id);
        // Зависимость, которая уже стоит, не трогаем, даже если вышла новее.
        if !up_to_date && !(depth > 0 && existing.is_some()) {
            if existing.is_some() && !allow_replace {
                return Err(Error::invalid("Остановите сервер, чтобы обновить: пока он работает, файл занят"));
            }
            let dest = folder.join(safe_file_name(&file.filename)?);
            http.download(&file.url, &dest, file.checksum.as_ref(), |_, _| {}).await?;
            let replaced = existing.map(|(old, _)| old).filter(|old| *old != file.filename);
            if let Some(old) = &replaced
                && let Err(e) = tokio::fs::remove_file(folder.join(old)).await
            {
                tracing::warn!("Не удалось удалить старую версию {old}: {e}");
            }
            present.insert(file.project_id.clone(), (file.filename.clone(), file.version_id.clone()));
            installed.push(InstalledFile { project_id: file.project_id.clone(), file: file.filename.clone(), replaced });
        }
        if depth < MAX_DEPTH {
            for dep in &file.required {
                if !present.contains_key(dep) {
                    queue.push((dep.clone(), depth + 1));
                }
            }
        }
    }
    Ok(installed)
}

/// Имя файла из ответа Modrinth не должно уводить из папки.
fn safe_file_name(name: &str) -> Result<&str> {
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
        return Err(Error::other(format!("Modrinth вернул странное имя файла: {name}")));
    }
    Ok(name)
}

/// Путь к файлу мода по имени из интерфейса.
pub fn mod_path(game_dir: &Path, loader: Loader, file: &str) -> Result<PathBuf> {
    let folder = game_dir.join(require_folder(loader)?);
    let ok = !file.is_empty() && !file.contains(['/', '\\', ':']) && file != ".." && file.to_ascii_lowercase().ends_with(".jar");
    let path = folder.join(file);
    if !ok || !path.is_file() {
        return Err(Error::invalid("Такого файла нет в папке сервера"));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_and_loaders() {
        assert_eq!(folder_name(Loader::Purpur), Some("plugins"));
        assert_eq!(folder_name(Loader::Fabric), Some("mods"));
        assert_eq!(folder_name(Loader::Vanilla), None);
        assert_eq!(modrinth_loaders(Loader::Purpur)[0], "purpur");
        assert_eq!(ClientSide::from_modrinth(Loader::Paper, "required"), ClientSide::ServerOnly);
        assert_eq!(ClientSide::from_modrinth(Loader::Fabric, "required"), ClientSide::Required);
        assert_eq!(ClientSide::from_modrinth(Loader::Fabric, "unsupported"), ClientSide::ServerOnly);
    }

    #[test]
    fn scan_hashes_only_jars() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.jar"), b"abc").unwrap();
        std::fs::write(dir.path().join("A.JAR"), b"").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("Plugin")).unwrap();
        let files = scan(dir.path()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "A.JAR");
        assert_eq!(files[1], ("b.jar".into(), "a9993e364706816aba3e25717850c26c9cd0d89d".into(), 3));
        assert!(scan(&dir.path().join("missing")).unwrap().is_empty());
    }

    #[test]
    fn paths_stay_in_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("mods")).unwrap();
        std::fs::write(dir.path().join("mods/a.jar"), b"x").unwrap();
        assert!(mod_path(dir.path(), Loader::Fabric, "a.jar").is_ok());
        assert!(mod_path(dir.path(), Loader::Fabric, "../a.jar").is_err());
        assert!(mod_path(dir.path(), Loader::Paper, "a.jar").is_err(), "у Paper другая папка");
        assert!(mod_path(dir.path(), Loader::Vanilla, "a.jar").is_err());
        assert!(safe_file_name("..\\x.jar").is_err());
        assert_eq!(title_from_file("worldedit-7.3.jar"), "worldedit-7.3");
    }

    #[tokio::test]
    #[ignore]
    async fn live_install_update_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let http = Http::new(dir.path().join("cache")).unwrap();
        let game = dir.path().join("server");
        // Lithium не требует зависимостей; Sodium Extra требует Sodium — проверим цепочку.
        let first = install(&http, Loader::Fabric, "26.3", &game, "gvQqBUqZ", true).await.unwrap().into_iter().map(|f| f.file).collect::<Vec<_>>();
        println!("поставлено: {first:?}");
        assert_eq!(first.len(), 1);
        assert!(install(&http, Loader::Fabric, "26.3", &game, "gvQqBUqZ", true).await.unwrap().is_empty(), "повторно не качает");
        let listing = list(&http, Loader::Fabric, "26.3", &game).await.unwrap();
        assert!(listing.online);
        let lithium = &listing.items[0];
        println!("{lithium:?}");
        assert_eq!(lithium.project_id.as_deref(), Some("gvQqBUqZ"));
        assert_eq!(lithium.title, "Lithium");
        assert!(lithium.update.is_none());

        // Старая версия вместо новой: Hearth видит обновление и заменяет файл.
        let versions: Vec<modrinth::Version> = http
            .get_json(
                "Modrinth",
                "https://api.modrinth.com/v2/project/gvQqBUqZ/version?loaders=%5B%22fabric%22%5D&game_versions=%5B%2226.3%22%5D",
            )
            .await
            .unwrap();
        let old = versions.last().unwrap();
        assert_ne!(old.id, versions[0].id, "у Lithium под 26.3 больше одной версии");
        let old_file = old.primary_file().unwrap();
        std::fs::remove_file(game.join("mods").join(&first[0])).unwrap();
        http.download(&old_file.url, &game.join("mods").join(&old_file.filename), None, |_, _| {}).await.unwrap();
        let listed = list(&http, Loader::Fabric, "26.3", &game).await.unwrap();
        println!("старая {} → обновление {:?}", old.version_number, listed.items[0].update);
        assert!(listed.items[0].update.is_some());
        assert!(install(&http, Loader::Fabric, "26.3", &game, "gvQqBUqZ", false).await.is_err(), "на работающем не обновляет");
        let updated = install(&http, Loader::Fabric, "26.3", &game, "gvQqBUqZ", true).await.unwrap();
        assert_eq!(updated[0].replaced.as_deref(), Some(old_file.filename.as_str()), "заменил старый файл");
        assert_eq!(updated[0].file, first[0]);
        let names: Vec<String> = std::fs::read_dir(game.join("mods")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, first, "старый файл удалён");

        // Плагин с зависимостью на Paper.
        let plugins = install(&http, Loader::Paper, "26.3", &game, "luckperms", true).await.unwrap();
        println!("плагины: {plugins:?}");
        assert!(game.join("plugins").join(&plugins[0].file).is_file());
        let found = search(&http, Loader::Purpur, "26.3", "", 0).await.unwrap();
        assert!(found.hits.iter().all(|h| h.client_side == ClientSide::ServerOnly));
    }
}
