//! Скины Ely.by на сервере для пиратских клиентов.
//!
//! Сервер без проверки лицензии (`online-mode=false`) скинов игроков не знает:
//! все бегают «Стивами». Hearth ставит плагин или мод, который при входе
//! игрока берёт его скин с Ely.by по нику (а если там нет — лицензионный с
//! тем же ником), и все видят скины друг друга:
//! - Paper/Purpur: плагин SkinsRestorer, настройка `api.elyByEnabled: true`;
//! - Fabric, NeoForge, Forge: мод SkinRestorer, источники при входе
//!   `["ely.by", "mojang"]`.
//!
//! Скины Ely.by подписаны так же, как лицензионные, поэтому их показывает
//! обычный клиент Minecraft без модов.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::net::Http;
use crate::pirate;
use crate::providers::Loader;

/// Плагин или мод скинов под ядро.
pub struct SkinPlugin {
    pub title: &'static str,
    project: &'static str,
    loaders: &'static [&'static str],
    folder: &'static str,
}

pub fn skin_plugin_for(loader: Loader) -> Option<SkinPlugin> {
    let modded = |loaders| SkinPlugin { title: "SkinRestorer", project: "skinrestorer", loaders, folder: "mods" };
    match loader {
        Loader::Paper | Loader::Purpur => Some(SkinPlugin {
            title: "SkinsRestorer",
            project: "skinsrestorer",
            loaders: &["paper", "bukkit", "spigot"],
            folder: "plugins",
        }),
        Loader::Fabric => Some(modded(&["fabric"])),
        Loader::NeoForge => Some(modded(&["neoforge"])),
        Loader::Forge => Some(modded(&["forge"])),
        Loader::Vanilla => None,
    }
}

/// Папка, где лежит jar плагина скинов для этого ядра.
pub fn folder(game_dir: &Path, loader: Loader) -> Option<PathBuf> {
    skin_plugin_for(loader).map(|p| game_dir.join(p.folder))
}

/// Ставит плагин скинов и включает в нём Ely.by. Возвращает имя файла.
pub async fn install(http: &Http, loader: Loader, game_dir: &Path, game_version: &str) -> Result<String> {
    let plugin = skin_plugin_for(loader)
        .ok_or_else(|| Error::invalid(format!("Для {} скины Ely.by не ставятся: нужен Paper, Purpur, Fabric, NeoForge или Forge", loader.display_name())))?;
    let file = pirate::install_project(http, plugin.title, plugin.project, plugin.loaders, &game_dir.join(plugin.folder), game_version)
        .await?
        .ok_or_else(|| Error::other(format!("{} пока не поддерживает Minecraft {game_version}", plugin.title)))?;
    configure(game_dir, loader)?;
    Ok(file)
}

/// Включает Ely.by в настройках плагина. Если плагин их ещё не создал,
/// Hearth кладёт свои: недостающее плагин допишет сам при запуске. Безопасно
/// вызывать перед каждым запуском сервера.
pub fn configure(game_dir: &Path, loader: Loader) -> Result<()> {
    match loader {
        Loader::Paper | Loader::Purpur => configure_skinsrestorer(&game_dir.join("plugins").join("SkinsRestorer").join("config.yml")),
        Loader::Fabric | Loader::NeoForge | Loader::Forge => {
            configure_skinrestorer(&game_dir.join("config").join("skinrestorer").join("config.json"))
        }
        Loader::Vanilla => Ok(()),
    }
}

/// SkinsRestorer: `api.elyByEnabled: true` в config.yml.
fn configure_skinsrestorer(path: &Path) -> Result<()> {
    let Ok(text) = std::fs::read_to_string(path) else {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, "api:\n  elyByEnabled: true\n")?;
        return Ok(());
    };
    let mut found = false;
    let mut changed = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(value) = trimmed.strip_prefix("elyByEnabled:") {
                found = true;
                if value.trim() != "true" {
                    changed = true;
                    let indent = &line[..line.len() - trimmed.len()];
                    return format!("{indent}elyByEnabled: true");
                }
            }
            line.to_string()
        })
        .collect();
    if !found {
        // Старый config.yml без этой настройки: дописываем в раздел api.
        match lines.iter().position(|l| l.trim_end() == "api:") {
            Some(i) => lines.insert(i + 1, "  elyByEnabled: true".into()),
            None => lines.extend(["api:".to_string(), "  elyByEnabled: true".to_string()]),
        }
        changed = true;
    }
    if changed {
        std::fs::write(path, lines.join("\n") + "\n")?;
    }
    Ok(())
}

/// Версия формата config.json SkinRestorer, под которую пишет Hearth.
const SKINRESTORER_CONFIG_VERSION: i64 = 3;

/// SkinRestorer: при входе брать скин с Ely.by, потом лицензионный; сообщения по-русски.
fn configure_skinrestorer(path: &Path) -> Result<()> {
    let mut config = match std::fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(value) if value.is_object() => value,
            _ => json!({ "version": SKINRESTORER_CONFIG_VERSION }),
        },
        Err(_) => json!({ "version": SKINRESTORER_CONFIG_VERSION, "language": "ru_ru" }),
    };
    // Файл старого формата (без версии) мод сначала переделает сам: не трогаем.
    if config.get("version").and_then(Value::as_i64).unwrap_or(1) < SKINRESTORER_CONFIG_VERSION {
        return Ok(());
    }
    let before = config.clone();
    let root = config.as_object_mut().expect("объект");
    let join = root.entry("join").or_insert_with(|| json!({}));
    if !join.is_object() {
        *join = json!({});
    }
    let auto = join.as_object_mut().expect("объект").entry("autoFetch").or_insert_with(|| json!({}));
    if !auto.is_object() {
        *auto = json!({});
    }
    let auto = auto.as_object_mut().expect("объект");
    auto.insert("enabled".into(), json!(true));
    auto.insert("providers".into(), json!(["ely.by", "mojang"]));
    if config != before || !path.is_file() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&config).map_err(|e| Error::other(e.to_string()))?;
        std::fs::write(path, text + "\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_for_every_loader_but_vanilla() {
        assert_eq!(skin_plugin_for(Loader::Paper).unwrap().project, "skinsrestorer");
        assert_eq!(skin_plugin_for(Loader::Purpur).unwrap().project, "skinsrestorer");
        for loader in [Loader::Fabric, Loader::NeoForge, Loader::Forge] {
            assert_eq!(skin_plugin_for(loader).unwrap().project, "skinrestorer");
        }
        assert!(skin_plugin_for(Loader::Vanilla).is_none());
    }

    #[test]
    fn skinsrestorer_new_and_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        configure(dir.path(), Loader::Paper).unwrap();
        let path = dir.path().join("plugins/SkinsRestorer/config.yml");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "api:\n  elyByEnabled: true\n");

        // Плагин переписал файл целиком и выключил настройку.
        std::fs::write(&path, "commands:\n  perSkinPermissions: false\napi:\n  mineskinAPIKey: key\n  elyByEnabled: false\n").unwrap();
        configure(dir.path(), Loader::Paper).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "commands:\n  perSkinPermissions: false\napi:\n  mineskinAPIKey: key\n  elyByEnabled: true\n"
        );

        // Старый файл без настройки.
        std::fs::write(&path, "api:\n  mineskinAPIKey: key\n").unwrap();
        configure(dir.path(), Loader::Paper).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "api:\n  elyByEnabled: true\n  mineskinAPIKey: key\n");
    }

    #[test]
    fn skinrestorer_new_and_existing_config() {
        let dir = tempfile::tempdir().unwrap();
        configure(dir.path(), Loader::Fabric).unwrap();
        let path = dir.path().join("config/skinrestorer/config.json");
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["version"], 3);
        assert_eq!(value["language"], "ru_ru");
        assert_eq!(value["join"]["autoFetch"]["providers"], json!(["ely.by", "mojang"]));

        // Мод дописал свои настройки: они остаются, язык не трогаем.
        std::fs::write(
            &path,
            r#"{"language":"en_us","join":{"refreshSkin":true,"autoFetch":{"enabled":false,"overrideExisting":false,"providers":["mojang"]}},"version":3}"#,
        )
        .unwrap();
        configure(dir.path(), Loader::NeoForge).unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["language"], "en_us");
        assert_eq!(value["join"]["refreshSkin"], true);
        assert_eq!(value["join"]["autoFetch"]["enabled"], true);
        assert_eq!(value["join"]["autoFetch"]["providers"], json!(["ely.by", "mojang"]));

        // Старый формат без версии не трогаем.
        std::fs::write(&path, r#"{"fetchSkinOnFirstJoin":true}"#).unwrap();
        configure(dir.path(), Loader::Forge).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"fetchSkinOnFirstJoin":true}"#);
    }
}
