//! NeoForge и Forge: ядра для больших модпаков.
//!
//! В отличие от остальных ядер, готового jar сервера нет: скачивается
//! установщик, Hearth запускает его с `--install-server`, а он сам качает
//! Minecraft и библиотеки и кладёт файл аргументов запуска
//! `libraries/.../win_args.txt`. Сервер потом запускается как
//! `java @libraries/.../win_args.txt nogui`.
//!
//! Версии: NeoForge — из maven-metadata.xml (под 26.x номер вида `26.3.0.115`,
//! раньше `21.1.77` для 1.21.1); Forge — из promotions_slim.json
//! (рекомендованная сборка, иначе последняя).

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use serde::Deserialize;

use super::{DownloadSpec, GameVersion, ServerArtifact};
use crate::error::{Error, Result};
use crate::java;
use crate::net::{Checksum, Http};

const NEO_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const FORGE_PROMOS: &str = "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const TTL: Duration = Duration::from_secs(60 * 60);

/// Старше этих версий Forge ставится и запускается иначе (без файла аргументов).
const FORGE_MIN: [u32; 3] = [1, 20, 1];

/// Файл аргументов запуска под эту ОС.
fn args_file_name() -> &'static str {
    if cfg!(windows) { "win_args.txt" } else { "unix_args.txt" }
}

/// `1.21.1` → [1, 21, 1]; для сравнения версий игры.
fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Версия игры по номеру NeoForge и стабильность.
/// `26.3.0.115` → 26.3, `26.1.2.128` → 26.1.2, `21.1.77` → 1.21.1,
/// `21.0.5-beta` → 1.21 (бета).
pub fn neoforge_game_version(v: &str) -> Option<(String, bool)> {
    let stable = !v.contains('-');
    let base = v.split('-').next()?;
    let parts: Vec<u32> = base.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let game = match parts.as_slice() {
        // Новая нумерация Mojang: номер NeoForge начинается с версии игры.
        [major, minor, patch, _build] if *major >= 25 => {
            if *patch == 0 { format!("{major}.{minor}") } else { format!("{major}.{minor}.{patch}") }
        }
        [major, minor, _build] if (20..25).contains(major) => {
            if *minor == 0 { format!("1.{major}") } else { format!("1.{major}.{minor}") }
        }
        _ => return None,
    };
    Some((game, stable))
}

/// Для каждой версии игры — последний номер NeoForge (стабильный, если есть).
pub fn neoforge_index(versions: &[String]) -> BTreeMap<String, (String, bool)> {
    let mut best: HashMap<String, (String, bool)> = HashMap::new();
    for v in versions {
        let Some((game, stable)) = neoforge_game_version(v) else { continue };
        let replace = match best.get(&game) {
            None => true,
            Some((_, cur_stable)) if stable && !cur_stable => true,
            Some((cur, cur_stable)) => stable == *cur_stable && version_key(v.split('-').next().unwrap_or(v)) > version_key(cur.split('-').next().unwrap_or(cur)),
        };
        if replace {
            best.insert(game, (v.clone(), stable));
        }
    }
    best.into_iter().collect()
}

/// Список версий из maven-metadata.xml (простой разбор `<version>…</version>`).
fn parse_maven_versions(xml: &str) -> Vec<String> {
    xml.split("<version>")
        .skip(1)
        .filter_map(|chunk| chunk.split("</version>").next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

async fn neoforge_versions(http: &Http) -> Result<BTreeMap<String, (String, bool)>> {
    let xml = http.get_text("NeoForge", &format!("{NEO_MAVEN}/maven-metadata.xml")).await?;
    Ok(neoforge_index(&parse_maven_versions(&xml)))
}

#[derive(Deserialize, Debug)]
pub struct Promotions {
    pub promos: HashMap<String, String>,
}

/// Для каждой версии игры — сборка Forge: рекомендованная, иначе последняя.
pub fn forge_index(promos: &Promotions) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (key, build) in &promos.promos {
        let Some((game, kind)) = key.rsplit_once('-') else { continue };
        if version_key(game) < FORGE_MIN.to_vec() {
            continue;
        }
        match kind {
            "recommended" => {
                out.insert(game.to_string(), build.clone());
            }
            "latest" => {
                out.entry(game.to_string()).or_insert_with(|| build.clone());
            }
            _ => {}
        }
    }
    out
}

async fn forge_versions(http: &Http) -> Result<BTreeMap<String, String>> {
    let promos: Promotions = http.get_json_cached("Forge", FORGE_PROMOS, "forge-promotions.json", Some(TTL)).await?;
    Ok(forge_index(&promos))
}

fn sorted_desc(mut list: Vec<GameVersion>) -> Vec<GameVersion> {
    list.sort_by_key(|v| std::cmp::Reverse(version_key(&v.id)));
    list
}

pub async fn list_neoforge(http: &Http, include_unstable: bool) -> Result<Vec<GameVersion>> {
    let index = neoforge_versions(http).await?;
    Ok(sorted_desc(
        index
            .into_iter()
            .filter(|(_, (_, stable))| include_unstable || *stable)
            .map(|(id, (_, stable))| GameVersion { id, stable, release_time: None })
            .collect(),
    ))
}

pub async fn list_forge(http: &Http) -> Result<Vec<GameVersion>> {
    let index = forge_versions(http).await?;
    Ok(sorted_desc(index.into_keys().map(|id| GameVersion { id, stable: true, release_time: None }).collect()))
}

/// sha1 установщика из файла `.sha1` рядом в maven (если есть).
async fn maven_sha1(http: &Http, service: &str, url: &str) -> Option<Checksum> {
    let text = http.get_text(service, &format!("{url}.sha1")).await.ok()?;
    let hash = text.split_whitespace().next()?.to_string();
    (hash.len() == 40).then_some(Checksum::Sha1(hash))
}

fn artifact(installer_url: String, checksum: Option<Checksum>, args_file: String, game: &str, version: String) -> ServerArtifact {
    let java_major = java::fallback_major(game);
    ServerArtifact {
        jar: DownloadSpec { url: installer_url, file_name: "hearth-installer.jar".into(), checksum },
        extra: Vec::new(),
        java_major,
        jvm_flags: java::default_jvm_flags(java_major),
        loader_version: Some(version),
        installer_args: Some(args_file),
    }
}

pub async fn resolve_neoforge(http: &Http, game: &str) -> Result<ServerArtifact> {
    let (version, _) = neoforge_versions(http)
        .await?
        .remove(game)
        .ok_or_else(|| Error::VersionNotFound { loader: "NeoForge".into(), version: game.into() })?;
    let url = format!("{NEO_MAVEN}/{version}/neoforge-{version}-installer.jar");
    let checksum = maven_sha1(http, "NeoForge", &url).await;
    let args = format!("libraries/net/neoforged/neoforge/{version}/{}", args_file_name());
    Ok(artifact(url, checksum, args, game, version))
}

pub async fn resolve_forge(http: &Http, game: &str) -> Result<ServerArtifact> {
    let build = forge_versions(http)
        .await?
        .remove(game)
        .ok_or_else(|| Error::VersionNotFound { loader: "Forge".into(), version: game.into() })?;
    let full = format!("{game}-{build}");
    let url = format!("{FORGE_MAVEN}/{full}/forge-{full}-installer.jar");
    let checksum = maven_sha1(http, "Forge", &url).await;
    let args = format!("libraries/net/minecraftforge/forge/{full}/{}", args_file_name());
    Ok(artifact(url, checksum, args, game, build))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_numbers_to_game_versions() {
        assert_eq!(neoforge_game_version("26.3.0.115"), Some(("26.3".into(), true)));
        assert_eq!(neoforge_game_version("26.1.2.128"), Some(("26.1.2".into(), true)));
        assert_eq!(neoforge_game_version("21.1.77"), Some(("1.21.1".into(), true)));
        assert_eq!(neoforge_game_version("21.0.5-beta"), Some(("1.21".into(), false)));
        assert_eq!(neoforge_game_version("20.4.237"), Some(("1.20.4".into(), true)));
        assert_eq!(neoforge_game_version("47.1.82"), None, "старый формат Forge-форка");
    }

    #[test]
    fn neoforge_picks_latest_stable() {
        let versions: Vec<String> = ["21.1.1-beta", "21.1.5", "21.1.10", "21.1.12-beta", "26.3.0.99", "26.3.0.115"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let index = neoforge_index(&versions);
        assert_eq!(index["1.21.1"], ("21.1.10".to_string(), true));
        assert_eq!(index["26.3"], ("26.3.0.115".to_string(), true));
        let xml = "<metadata><versioning><versions><version>26.3.0.1</version>\n<version>21.1.5</version></versions></versioning></metadata>";
        assert_eq!(parse_maven_versions(xml), ["26.3.0.1", "21.1.5"]);
    }

    #[test]
    fn forge_prefers_recommended() {
        let promos: Promotions = serde_json::from_str(
            r#"{"promos":{"26.3-latest":"66.1.9","26.3-recommended":"66.1.8","26.2-latest":"65.1.6","1.20.1-recommended":"47.4.0","1.16.5-latest":"36.2.42"}}"#,
        )
        .unwrap();
        let index = forge_index(&promos);
        assert_eq!(index["26.3"], "66.1.8");
        assert_eq!(index["26.2"], "65.1.6");
        assert_eq!(index["1.20.1"], "47.4.0");
        assert!(!index.contains_key("1.16.5"), "старый Forge не поддерживаем");
        let sorted = sorted_desc(index.keys().map(|id| GameVersion { id: id.clone(), stable: true, release_time: None }).collect());
        assert_eq!(sorted.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["26.3", "26.2", "1.20.1"]);
    }
}
