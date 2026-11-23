//! Сборка модов для друзей (Fabric): те моды сервера, которые нужны игрокам,
//! их зависимости и, по желанию, «Замена OptiFine».
//!
//! Два формата:
//! - `.mrpack` для Prism Launcher и Modrinth App: только ссылки на Modrinth с
//!   хэшами, лаунчер сам скачает моды и Fabric. Внутри ещё `servers.dat`,
//!   чтобы сервер сразу был в списке серверов.
//! - zip для TLauncher и других: папка `mods` с самими jar и инструкция.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::error::{Error, Result};
use crate::modrinth::{self, Version};
use crate::mods::{self, ClientSide};
use crate::net::{Checksum, Http};
use crate::providers::Loader;
use crate::servers::ServerMeta;

/// «Замена OptiFine»: производительность и шейдеры. Id на Modrinth и название.
pub const OPTIFINE_PACK: &[(&str, &str)] = &[
    ("AANobbMI", "Sodium"),
    ("YL57xq9U", "Iris Shaders"),
    ("NNAgCjsB", "Entity Culling"),
    ("5ZwdcRci", "ImmediatelyFast"),
    ("uXXizFIs", "FerriteCore"),
];

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PackFormat {
    /// Prism Launcher, Modrinth App.
    Mrpack,
    /// TLauncher и любые другие: папка mods.
    Zip,
}

#[derive(Deserialize, Clone, Copy, Debug, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PackOptions {
    /// Добавить моды «игрокам по желанию».
    pub optional: bool,
    /// Добавить «Замену OptiFine».
    pub optifine: bool,
}

/// Что попало в сборку.
#[derive(Serialize, Clone, Debug, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PackSummary {
    /// Названия модов в сборке.
    pub mods: Vec<String>,
    /// Что не попало и почему.
    pub skipped: Vec<String>,
    /// Сколько модов сервера «по желанию» (для подписи к галочке).
    pub optional_available: u32,
    /// Адрес сервера, который попадёт в список серверов (.mrpack).
    pub address: Option<String>,
}

/// Файл мода в сборке.
#[derive(Debug, Clone)]
struct PackFile {
    title: String,
    filename: String,
    url: String,
    sha1: String,
    sha512: Option<String>,
    size: u64,
    /// Тот же файл уже лежит на сервере: в zip копируем его, а не качаем.
    local: Option<PathBuf>,
}

impl PackFile {
    fn from_version(title: &str, v: &Version, local: Option<PathBuf>) -> Option<Self> {
        let f = v.primary_file()?;
        Some(Self {
            title: title.to_string(),
            filename: f.filename.clone(),
            url: f.url.clone(),
            sha1: f.hashes.sha1.clone()?,
            sha512: f.hashes.sha512.clone(),
            size: f.size,
            local,
        })
    }
}

/// Собирает список: моды сервера для игроков, их зависимости, «Замена OptiFine».
async fn collect(http: &Http, meta: &ServerMeta, game_dir: &Path, opts: PackOptions) -> Result<(Vec<PackFile>, PackSummary)> {
    if meta.loader != Loader::Fabric {
        return Err(Error::invalid("Сборка для друзей нужна только Fabric: плагины Paper ставить игрокам не нужно"));
    }
    let folder = game_dir.join("mods");
    let list = mods::list(http, meta.loader, &meta.game_version, game_dir).await?;
    if !list.online {
        return Err(Error::Network("не удалось связаться с Modrinth".into()));
    }
    // Версии модов сервера по хэшу: ссылки, sha512 и зависимости.
    let hashes: Vec<String> = mods_hashes(&folder).await?;
    let by_hash = modrinth::versions_by_hash(http, &hashes).await?;
    let by_project: HashMap<String, Version> = by_hash.into_values().map(|v| (v.project_id.clone(), v)).collect();

    let mut summary = PackSummary {
        optional_available: list
            .items
            .iter()
            .filter(|m| m.client_side == ClientSide::Optional && !server_side_only(m.project_id.as_deref()))
            .count() as u32,
        ..Default::default()
    };
    let mut files: Vec<PackFile> = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();
    let mut deps: Vec<String> = Vec::new();

    for item in &list.items {
        let wanted = match item.client_side {
            ClientSide::Required => true,
            // Серверные оптимизации из пресета игрокам ни к чему, даже если автор пометил их «по желанию».
            ClientSide::Optional => opts.optional && !server_side_only(item.project_id.as_deref()),
            ClientSide::ServerOnly => false,
            ClientSide::Unknown => {
                summary.skipped.push(format!("{}: файл не с Modrinth, если он нужен игрокам, передайте его отдельно", item.title));
                false
            }
        };
        // Защита ников (EasyAuth) работает только на сервере.
        if !wanted || item.nick_protection {
            continue;
        }
        let Some(version) = item.project_id.as_ref().and_then(|p| by_project.get(p)) else { continue };
        if let Some(file) = PackFile::from_version(&item.title, version, Some(folder.join(&item.file))) {
            taken.insert(version.project_id.clone());
            deps.extend(required(version));
            files.push(file);
        }
    }

    if opts.optifine {
        for (project, title) in OPTIFINE_PACK {
            if taken.contains(*project) {
                continue;
            }
            match pick_client_version(http, meta, project).await? {
                Some(v) => {
                    if let Some(file) = PackFile::from_version(title, &v, None) {
                        taken.insert(v.project_id.clone());
                        deps.extend(required(&v));
                        files.push(file);
                    }
                }
                None => summary.skipped.push(format!("{title}: нет версии под {}", meta.game_version)),
            }
        }
    }

    // Зависимости (обычно Fabric API): берём версию с сервера, если она там есть.
    let mut depth = 0;
    while !deps.is_empty() && depth < 4 {
        let mut next = Vec::new();
        for dep in std::mem::take(&mut deps) {
            if !taken.insert(dep.clone()) {
                continue;
            }
            let (version, local) = match by_project.get(&dep) {
                Some(v) => {
                    let local = list.items.iter().find(|m| m.project_id.as_deref() == Some(dep.as_str())).map(|m| folder.join(&m.file));
                    (Some(v.clone()), local)
                }
                None => (pick_client_version(http, meta, &dep).await?, None),
            };
            let Some(version) = version else {
                summary.skipped.push(format!("Зависимость {dep}: нет версии под {}", meta.game_version));
                continue;
            };
            let title = modrinth::projects(http, &[dep.as_str()]).await?.into_iter().next().map(|p| p.title).unwrap_or(dep.clone());
            next.extend(required(&version));
            if let Some(file) = PackFile::from_version(&title, &version, local) {
                files.push(file);
            }
        }
        deps = next;
        depth += 1;
    }

    files.sort_by_key(|f| f.title.to_lowercase());
    summary.mods = files.iter().map(|f| f.title.clone()).collect();
    Ok((files, summary))
}

/// Моды, которые ускоряют сервер, но игрокам не нужны (пресет «Производительность» и C2ME).
fn server_side_only(project: Option<&str>) -> bool {
    project.is_some_and(|p| crate::presets::FABRIC_MODS.iter().any(|(id, _)| *id == p) || p == "VSNURh3q")
}

fn required(v: &Version) -> Vec<String> {
    v.dependencies
        .iter()
        .filter(|d| d.dependency_type == "required")
        .filter_map(|d| d.project_id.clone())
        .collect()
}

async fn mods_hashes(folder: &Path) -> Result<Vec<String>> {
    let folder = folder.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        use sha1::{Digest, Sha1};
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&folder) else { return Ok(out) };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().to_ascii_lowercase().ends_with(".jar") {
                let data = std::fs::read(entry.path())?;
                out.push(hex::encode(Sha1::digest(&data).as_slice()));
            }
        }
        Ok(out)
    })
    .await?
}

/// Клиентская версия проекта под версию игры: релиз или бета (у Sodium под
/// свежие версии часто только беты), но не альфа.
async fn pick_client_version(http: &Http, meta: &ServerMeta, project: &str) -> Result<Option<Version>> {
    let url = format!(
        "https://api.modrinth.com/v2/project/{project}/version?loaders=%5B%22fabric%22%5D&game_versions=%5B%22{}%22%5D",
        meta.game_version
    );
    let versions: Vec<Version> = match http.get_json("Modrinth", &url).await {
        Ok(v) => v,
        Err(e) if e.is_not_found() => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(versions
        .iter()
        .find(|v| v.version_type == "release")
        .or_else(|| versions.iter().find(|v| v.version_type == "beta"))
        .cloned())
}

/// Только посмотреть, что попадёт в сборку (без скачивания).
pub async fn preview(http: &Http, meta: &ServerMeta, game_dir: &Path, opts: PackOptions, address: Option<String>) -> Result<PackSummary> {
    let (_, mut summary) = collect(http, meta, game_dir, opts).await?;
    summary.address = address;
    Ok(summary)
}

/// Собирает файл сборки в `dest`.
pub async fn build(
    http: &Http,
    meta: &ServerMeta,
    game_dir: &Path,
    opts: PackOptions,
    format: PackFormat,
    dest: &Path,
    address: Option<String>,
) -> Result<PackSummary> {
    let (files, mut summary) = collect(http, meta, game_dir, opts).await?;
    if files.is_empty() {
        return Err(Error::invalid("Игрокам ничего ставить не нужно: в сборку нечего положить"));
    }
    let loader_version = meta
        .loader_version
        .clone()
        .ok_or_else(|| Error::invalid("Сервер ещё не установлен: запустите его хотя бы раз"))?;

    // Для zip нужны сами файлы: с сервера или из кэша скачанных.
    let mut contents: Vec<(String, PathBuf)> = Vec::new();
    if format == PackFormat::Zip {
        let cache = http.cache_dir().join("pack-mods");
        for f in &files {
            let path = match &f.local {
                Some(p) if p.is_file() => p.clone(),
                _ => {
                    let path = cache.join(format!("{}-{}", &f.sha1[..12.min(f.sha1.len())], f.filename));
                    if !path.is_file() {
                        http.download(&f.url, &path, Some(&Checksum::Sha1(f.sha1.clone())), |_, _| {}).await?;
                    }
                    path
                }
            };
            contents.push((f.filename.clone(), path));
        }
    }

    let name = format!("{} — моды", meta.name);
    let readme = readme(meta, &loader_version, &files, address.as_deref());
    let index = (format == PackFormat::Mrpack).then(|| mrpack_index(meta, &name, &loader_version, &files));
    let servers_dat = (format == PackFormat::Mrpack).then(|| address.as_deref().map(|a| servers_dat(&meta.name, a))).flatten();
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut part_name = dest.file_name().unwrap_or_default().to_os_string();
        part_name.push(".part");
        let part = dest.with_file_name(part_name);
        let result = (|| -> Result<()> {
            let mut zip = ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(&part)?));
            let o = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            if let Some(index) = &index {
                zip.start_file("modrinth.index.json", o)?;
                zip.write_all(serde_json::to_string_pretty(index).expect("json").as_bytes())?;
                if let Some(dat) = &servers_dat {
                    zip.start_file("overrides/servers.dat", o)?;
                    zip.write_all(dat)?;
                }
            } else {
                zip.start_file("КАК УСТАНОВИТЬ.txt", o)?;
                zip.write_all(readme.as_bytes())?;
                zip.add_directory("mods/", o)?;
                for (filename, path) in &contents {
                    zip.start_file(format!("mods/{filename}"), o)?;
                    std::io::copy(&mut std::fs::File::open(path)?, &mut zip)?;
                }
            }
            zip.finish()?.flush()?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        crate::fsx::rename_with_retry_sync(&part, &dest)?;
        Ok(())
    })
    .await??;
    summary.address = address;
    Ok(summary)
}

/// `modrinth.index.json` по спецификации формата `.mrpack` (formatVersion 1).
fn mrpack_index(meta: &ServerMeta, name: &str, loader_version: &str, files: &[PackFile]) -> serde_json::Value {
    let files: Vec<serde_json::Value> = files
        .iter()
        .map(|f| {
            let mut hashes = BTreeMap::new();
            hashes.insert("sha1", f.sha1.clone());
            if let Some(h) = &f.sha512 {
                hashes.insert("sha512", h.clone());
            }
            json!({
                "path": format!("mods/{}", f.filename),
                "hashes": hashes,
                "env": { "client": "required", "server": "unsupported" },
                "downloads": [f.url],
                "fileSize": f.size,
            })
        })
        .collect();
    json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": chrono::Local::now().format("%Y.%m.%d-%H%M").to_string(),
        "name": name,
        "summary": format!("Моды для сервера «{}». Собрано в Hearth.", meta.name),
        "files": files,
        "dependencies": { "minecraft": meta.game_version, "fabric-loader": loader_version },
    })
}

fn readme(meta: &ServerMeta, loader_version: &str, files: &[PackFile], address: Option<&str>) -> String {
    let game = &meta.game_version;
    let mut lines = vec![
        format!("Моды для сервера «{}»", meta.name),
        String::new(),
        format!("1. Установите Fabric для Minecraft {game} (загрузчик {loader_version} или новее)."),
        format!("   TLauncher: выберите в списке версий «Fabric {game}»."),
        "   Обычный лаунчер: скачайте установщик с https://fabricmc.net/use/installer/".into(),
        "2. Откройте папку игры (.minecraft) и положите все файлы из папки mods этого архива".into(),
        r"   в папку .minecraft\mods (если её нет, создайте).".into(),
        format!("3. Запустите игру с версией Fabric {game}."),
    ];
    if let Some(address) = address {
        lines.push(String::new());
        lines.push(format!("Адрес сервера: {address}"));
    }
    lines.push(String::new());
    lines.push("Моды в сборке:".into());
    lines.extend(files.iter().map(|f| format!("  - {}", f.title)));
    lines.push(String::new());
    lines.join("\r\n")
}

/// `servers.dat` (несжатый NBT) с одним сервером: он появится в списке серверов.
pub fn servers_dat(name: &str, address: &str) -> Vec<u8> {
    fn put_str(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u16).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
    }
    let mut out = vec![0x0A]; // корневой TAG_Compound
    put_str(&mut out, "");
    out.push(0x09); // TAG_List "servers"
    put_str(&mut out, "servers");
    out.push(0x0A); // из TAG_Compound
    out.extend_from_slice(&1i32.to_be_bytes());
    for (key, value) in [("name", name), ("ip", address)] {
        out.push(0x08); // TAG_String
        put_str(&mut out, key);
        put_str(&mut out, value);
    }
    out.push(0x00); // конец сервера
    out.push(0x00); // конец корня
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_dat_layout() {
        let dat = servers_dat("Мой", "play.example.net");
        assert_eq!(&dat[..3], &[0x0A, 0, 0]);
        assert_eq!(dat[3], 0x09);
        assert_eq!(&dat[4..6], &[0, 7]);
        assert_eq!(&dat[6..13], b"servers");
        assert_eq!(dat[13], 0x0A);
        assert_eq!(&dat[14..18], &[0, 0, 0, 1]);
        assert!(dat.windows("Мой".len()).any(|w| w == "Мой".as_bytes()));
        assert!(dat.windows(16).any(|w| w == b"play.example.net"));
        assert_eq!(&dat[dat.len() - 2..], &[0, 0]);
    }

    #[tokio::test]
    #[ignore]
    async fn live_build_packs() {
        let dir = tempfile::tempdir().unwrap();
        let http = Http::new(dir.path().join("cache")).unwrap();
        let game = dir.path().join("server");
        // Simple Voice Chat нужен игрокам и тянет Fabric API? Ставим заведомо клиентский мод и серверный.
        for project in ["9eGKb6K1", "gvQqBUqZ"] {
            mods::install(&http, Loader::Fabric, "26.3", &game, project, true).await.unwrap();
        }
        let meta: ServerMeta = serde_json::from_value(json!({
            "id": "x", "name": "Тест", "loader": "fabric", "gameVersion": "26.3", "loaderVersion": "0.20.1",
            "memoryMb": 2048, "port": 25565, "eulaAcceptedAt": null, "createdAt": "2026-11-23T00:00:00Z"
        }))
        .unwrap();
        let opts = PackOptions { optional: true, optifine: true };
        let summary = preview(&http, &meta, &game, opts, None).await.unwrap();
        println!("{summary:#?}");
        assert!(summary.mods.iter().any(|m| m.contains("Voice")), "мод «по желанию» в сборке");
        assert!(summary.mods.iter().any(|m| m == "Sodium"), "замена OptiFine");
        assert!(!summary.mods.iter().any(|m| m == "Lithium"), "серверная оптимизация не попала");

        let mrpack = dir.path().join("pack.mrpack");
        build(&http, &meta, &game, opts, PackFormat::Mrpack, &mrpack, Some("play.example.net".into())).await.unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&mrpack).unwrap()).unwrap();
        let mut index = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("modrinth.index.json").unwrap(), &mut index).unwrap();
        let index: serde_json::Value = serde_json::from_str(&index).unwrap();
        assert_eq!(index["dependencies"]["fabric-loader"], "0.20.1");
        assert!(index["files"].as_array().unwrap().iter().all(|f| f["hashes"]["sha512"].is_string()));
        assert!(archive.by_name("overrides/servers.dat").is_ok());

        let zip_path = dir.path().join("pack.zip");
        build(&http, &meta, &game, opts, PackFormat::Zip, &zip_path, None).await.unwrap();
        let archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        let names: Vec<String> = archive.file_names().filter_map(|n| n.ok().map(|n| n.to_string())).collect();
        println!("{names:?}");
        assert!(names.iter().any(|n| n.starts_with("mods/sodium")));
        assert!(names.iter().any(|n| n == "КАК УСТАНОВИТЬ.txt"));
    }
}
