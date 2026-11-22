//! Пиратский режим: вход с нелицензионных клиентов (TLauncher и др.).
//!
//! - `online-mode=false` и `enforce-secure-profile=false` (иначе такие клиенты
//!   не смогут писать в чат на новых версиях);
//! - плагин или мод авторизации, без которого любой зашёл бы под чужим ником,
//!   включая ник администратора: AuthMe Reloaded (Paper), EasyAuth (Fabric);
//! - перенос вещей игроков: в пиратском режиме у игрока другой UUID, и без
//!   переноса его инвентарь, достижения и права «пропадут».
//!
//! UUID в пиратском режиме считается из ника так же, как это делает сервер:
//! `UUID.nameUUIDFromBytes("OfflinePlayer:" + ник)` (MD5, версия 3).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::modrinth;
use crate::net::Http;
use crate::providers::Loader;

/// UUID игрока на сервере с `online-mode=false`.
pub fn offline_uuid(name: &str) -> Uuid {
    let digest: [u8; 16] = Md5::digest(format!("OfflinePlayer:{name}").as_bytes()).into();
    uuid::Builder::from_md5_bytes(digest).into_uuid()
}

#[derive(Deserialize)]
struct CacheEntry {
    name: String,
    uuid: String,
}

/// Игроки, которых сервер уже видел: `usercache.json` (ник → UUID).
pub fn known_players(game_dir: &Path) -> Vec<(String, Uuid)> {
    let Ok(text) = std::fs::read_to_string(game_dir.join("usercache.json")) else {
        return Vec::new();
    };
    let entries: Vec<CacheEntry> = serde_json::from_str(&text).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter_map(|e| Some((e.name, Uuid::parse_str(&e.uuid).ok()?)))
        .filter(|(name, _)| seen.insert(name.to_lowercase()))
        .collect()
}

/// Переименовывает файлы игроков (`<uuid>.dat`, `.dat_old`, `.json`) по всей
/// папке мира: так переносятся инвентарь, достижения и статистика при любой
/// раскладке папок мира. Существующие файлы не перезаписываются.
/// Возвращает, сколько игроков перенесено.
pub fn migrate_player_files(world_dir: &Path, map: &HashMap<Uuid, Uuid>) -> usize {
    let mut moved = std::collections::HashSet::new();
    let mut stack = vec![world_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                // Регионы большие и UUID в имени не содержат: пропускаем ради скорости.
                if !matches!(path.file_name().and_then(|n| n.to_str()), Some("region" | "entities" | "poi")) {
                    stack.push(path);
                }
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let Some((stem, ext)) = name.split_once('.') else { continue };
            if !matches!(ext, "dat" | "dat_old" | "json") {
                continue;
            }
            let Ok(from) = Uuid::parse_str(stem) else { continue };
            let Some(to) = map.get(&from) else { continue };
            let dest = path.with_file_name(format!("{}.{ext}", to.hyphenated()));
            if dest.exists() {
                continue;
            }
            match std::fs::rename(&path, &dest) {
                Ok(()) => {
                    moved.insert(from);
                }
                Err(e) => tracing::warn!("Не удалось перенести {}: {e}", path.display()),
            }
        }
    }
    moved.len()
}

/// Меняет UUID в ops.json, whitelist.json, banned-players.json.
pub fn migrate_lists(game_dir: &Path, map: &HashMap<Uuid, Uuid>) {
    for file in ["ops.json", "whitelist.json", "banned-players.json"] {
        let path = game_dir.join(file);
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(mut list) = serde_json::from_str::<Vec<Value>>(&text) else { continue };
        let mut changed = false;
        for item in &mut list {
            let Some(old) = item.get("uuid").and_then(Value::as_str).and_then(|u| Uuid::parse_str(u).ok()) else {
                continue;
            };
            if let Some(new) = map.get(&old) {
                item["uuid"] = Value::String(new.hyphenated().to_string());
                changed = true;
            }
        }
        if changed {
            if let Ok(text) = serde_json::to_string_pretty(&list) {
                let _ = std::fs::write(&path, text);
            }
        }
    }
}

/// Лицензионные UUID по никам (Mojang). Ники без лицензии в ответ не попадают.
pub async fn online_uuids(http: &Http, names: &[String]) -> Result<HashMap<String, Uuid>> {
    #[derive(Deserialize)]
    struct Profile {
        id: String,
        name: String,
    }
    let mut result = HashMap::new();
    for chunk in names.chunks(10) {
        let resp = http
            .client()
            .post("https://api.minecraftservices.com/minecraft/profile/lookup/bulk/byname")
            .json(chunk)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Api {
                service: "Mojang".into(),
                status: resp.status().as_u16(),
                message: "не удалось узнать лицензионные UUID игроков".into(),
            });
        }
        let profiles: Vec<Profile> = resp.json().await?;
        for p in profiles {
            if let Ok(id) = Uuid::parse_str(&p.id) {
                result.insert(p.name.to_lowercase(), id);
            }
        }
    }
    Ok(result)
}

/// Плагин или мод авторизации под ядро.
pub struct AuthPlugin {
    pub title: &'static str,
    project: &'static str,
    loaders: &'static [&'static str],
    folder: &'static str,
}

pub fn auth_plugin_for(loader: Loader) -> Option<AuthPlugin> {
    match loader {
        Loader::Paper => Some(AuthPlugin {
            title: "AuthMe Reloaded",
            project: "authmereloaded",
            loaders: &["paper", "bukkit", "spigot"],
            folder: "plugins",
        }),
        Loader::Fabric => Some(AuthPlugin {
            title: "EasyAuth",
            project: "easyauth",
            loaders: &["fabric"],
            folder: "mods",
        }),
        Loader::Vanilla => None,
    }
}

/// Ставит плагин авторизации и его обязательные зависимости (например, Fabric API).
/// Возвращает имя файла самого плагина (чтобы потом его можно было убрать).
pub async fn install_auth(http: &Http, plugin: &AuthPlugin, game_dir: &Path, game_version: &str) -> Result<String> {
    // Сначала сборка именно под это ядро (у AuthMe есть отдельная для Paper), потом совместимые.
    let mut found = None;
    for loaders in [&plugin.loaders[..1], plugin.loaders] {
        found = modrinth::latest_file(http, plugin.project, loaders, game_version).await?;
        if found.is_some() {
            break;
        }
    }
    let file = found
        .ok_or_else(|| {
            Error::AuthUnavailable(format!(
                "{} пока не поддерживает Minecraft {game_version}",
                plugin.title
            ))
        })?;
    let folder = game_dir.join(plugin.folder);
    tokio::fs::create_dir_all(&folder).await?;
    for dep in &file.required {
        if dep_present(&folder, dep) {
            continue;
        }
        match modrinth::latest_file(http, dep, plugin.loaders, game_version).await? {
            Some(dep_file) => {
                http.download(&dep_file.url, &folder.join(&dep_file.filename), dep_file.checksum.as_ref(), |_, _| {})
                    .await?;
            }
            None => tracing::warn!("Зависимость {dep} для {} не найдена под {game_version}", plugin.title),
        }
    }
    remove_old_copies(&folder, plugin.project, &file.filename);
    http.download(&file.url, &folder.join(&file.filename), file.checksum.as_ref(), |_, _| {})
        .await?;
    Ok(file.filename)
}

/// Fabric API (P7dR8mSH) часто уже стоит: не качаем второй раз.
fn dep_present(folder: &Path, project_id: &str) -> bool {
    let prefix = match project_id {
        "P7dR8mSH" => "fabric-api",
        _ => return false,
    };
    jar_names(folder).iter().any(|n| n.to_lowercase().starts_with(prefix))
}

fn jar_names(folder: &Path) -> Vec<String> {
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.to_lowercase().ends_with(".jar"))
        .collect()
}

/// Старые версии этого же плагина (иначе сервер загрузит две копии).
fn remove_old_copies(folder: &Path, project: &str, keep: &str) {
    let prefix = match project {
        "authmereloaded" => "authme-",
        "easyauth" => "easyauth-",
        _ => return,
    };
    for name in jar_names(folder) {
        if name != keep && name.to_lowercase().starts_with(prefix) {
            let _ = std::fs::remove_file(folder.join(&name));
        }
    }
}

/// Папка, где лежит jar плагина авторизации для этого ядра.
pub fn auth_folder(game_dir: &Path, loader: Loader) -> Option<PathBuf> {
    auth_plugin_for(loader).map(|p| game_dir.join(p.folder))
}

/// AuthMe по умолчанию пишет игрокам по-английски: включаем русский. Делается
/// один раз, после первого запуска, когда AuthMe создаст свой config.yml.
pub fn authme_russian(game_dir: &Path) -> bool {
    let path = game_dir.join("plugins").join("AuthMe").join("config.yml");
    let Ok(text) = std::fs::read_to_string(&path) else { return false };
    let mut changed = false;
    let out: Vec<String> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(value) = trimmed.strip_prefix("messagesLanguage:") {
                if value.trim().trim_matches(['\'', '"']) == "en" {
                    changed = true;
                    let indent = &line[..line.len() - trimmed.len()];
                    return format!("{indent}messagesLanguage: ru");
                }
            }
            line.to_string()
        })
        .collect();
    if changed {
        let _ = std::fs::write(&path, out.join("\n") + "\n");
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_java() {
        // Эталон: UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8)).
        assert_eq!(offline_uuid("Notch").to_string(), "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert_eq!(offline_uuid("Notch").get_version_num(), 3);
    }

    #[test]
    fn migrates_player_files_and_lists() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path();
        let world = game.join("world");
        let online = Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap();
        let offline = offline_uuid("Notch");
        for sub in ["playerdata", "advancements", "stats", "region"] {
            std::fs::create_dir_all(world.join(sub)).unwrap();
        }
        std::fs::write(world.join(format!("playerdata/{online}.dat")), b"inv").unwrap();
        std::fs::write(world.join(format!("playerdata/{online}.dat_old")), b"inv").unwrap();
        std::fs::write(world.join(format!("advancements/{online}.json")), b"{}").unwrap();
        std::fs::write(world.join(format!("stats/{online}.json")), b"{}").unwrap();
        std::fs::write(world.join("region/r.0.0.mca"), b"r").unwrap();
        std::fs::write(
            game.join("ops.json"),
            format!(r#"[{{"uuid":"{online}","name":"Notch","level":4,"bypassesPlayerLimit":false}}]"#),
        )
        .unwrap();

        let map = HashMap::from([(online, offline)]);
        assert_eq!(migrate_player_files(&world, &map), 1);
        migrate_lists(game, &map);
        assert!(world.join(format!("playerdata/{offline}.dat")).exists());
        assert!(world.join(format!("playerdata/{offline}.dat_old")).exists());
        assert!(world.join(format!("advancements/{offline}.json")).exists());
        assert!(!world.join(format!("playerdata/{online}.dat")).exists());
        let ops = std::fs::read_to_string(game.join("ops.json")).unwrap();
        assert!(ops.contains(&offline.to_string()) && !ops.contains(&online.to_string()));
    }

    #[test]
    fn reads_usercache() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("usercache.json"),
            r#"[{"name":"Notch","uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","expiresOn":"2026-12-22 10:00:00 +0100"},
                {"name":"notch","uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","expiresOn":"x"}]"#,
        )
        .unwrap();
        let players = known_players(dir.path());
        assert_eq!(players.len(), 1);
        assert_eq!(players[0].0, "Notch");
    }

    #[test]
    fn authme_switches_to_russian_once() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("plugins/AuthMe");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("config.yml"), "settings:\n    messagesLanguage: 'en'\n    other: 1\n").unwrap();
        assert!(authme_russian(dir.path()));
        let text = std::fs::read_to_string(cfg.join("config.yml")).unwrap();
        assert!(text.contains("    messagesLanguage: ru\n"));
        assert!(!authme_russian(dir.path()));
    }
}
