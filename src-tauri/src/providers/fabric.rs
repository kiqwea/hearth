//! Fabric через meta.fabricmc.net: готовый server launcher jar.
//!
//! Лаунчер Fabric при первом запуске сам качает ванильный server.jar. Мы кладём
//! его заранее (с проверкой sha1 и прогрессом), лаунчер находит его по имени
//! `server.jar` и ничего не скачивает.

use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

use super::{vanilla, DownloadSpec, GameVersion, ServerArtifact};
use crate::error::{Error, Result};
use crate::java;
use crate::net::Http;

const SERVICE: &str = "FabricMC";
const META: &str = "https://meta.fabricmc.net/v2";
const TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Deserialize, Debug)]
pub struct GameEntry {
    pub version: String,
    pub stable: bool,
}

#[derive(Deserialize, Debug)]
pub struct LoaderForGame {
    pub loader: VersionEntry,
}

#[derive(Deserialize, Debug)]
pub struct VersionEntry {
    pub version: String,
    pub stable: bool,
}

pub async fn list_versions(http: &Http, include_unstable: bool) -> Result<Vec<GameVersion>> {
    let games: Vec<GameEntry> = http
        .get_json_cached(SERVICE, &format!("{META}/versions/game"), "fabric-game.json", Some(TTL))
        .await?;
    Ok(games
        .into_iter()
        .filter(|g| include_unstable || g.stable)
        .map(|g| GameVersion {
            id: g.version,
            stable: g.stable,
            release_time: None,
        })
        .collect())
}

/// Первая стабильная версия, а если стабильных нет, просто первая (самая новая).
pub fn pick<'a>(entries: impl IntoIterator<Item = &'a VersionEntry> + Clone) -> Option<&'a str> {
    entries
        .clone()
        .into_iter()
        .find(|e| e.stable)
        .or_else(|| entries.into_iter().next())
        .map(|e| e.version.as_str())
}

fn meta_url(segments: &[&str]) -> Result<String> {
    let mut url = Url::parse(META).expect("valid base url");
    url.path_segments_mut()
        .map_err(|_| Error::other("Некорректный адрес Fabric meta"))?
        .extend(segments);
    Ok(url.into())
}

pub async fn resolve(http: &Http, game: &str) -> Result<ServerArtifact> {
    let loaders_url = meta_url(&["versions", "loader", game])?;
    let cache = format!("fabric-loaders-{}.json", sanitize(game));
    let loaders: Vec<LoaderForGame> = http
        .get_json_cached(SERVICE, &loaders_url, &cache, Some(TTL))
        .await?;
    let loader = pick(loaders.iter().map(|l| &l.loader))
        .ok_or_else(|| Error::VersionNotFound {
            loader: "Fabric".into(),
            version: game.into(),
        })?
        .to_string();

    let installers: Vec<VersionEntry> = http
        .get_json_cached(
            SERVICE,
            &format!("{META}/versions/installer"),
            "fabric-installers.json",
            Some(TTL),
        )
        .await?;
    let installer = pick(installers.iter())
        .ok_or_else(|| Error::other("Fabric meta не вернул ни одной версии установщика"))?
        .to_string();

    let (vanilla_jar, java_major) = vanilla::server_download(http, game).await?;
    let jar_url = meta_url(&["versions", "loader", game, &loader, &installer, "server", "jar"])?;

    Ok(ServerArtifact {
        jar: DownloadSpec {
            url: jar_url,
            file_name: "fabric-server-launch.jar".into(),
            // Fabric meta не публикует хэш лаунчера.
            checksum: None,
        },
        extra: vec![vanilla_jar],
        java_major,
        jvm_flags: java::default_jvm_flags(java_major),
        loader_version: Some(loader),
        installer_args: None,
    })
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_first_stable_loader() {
        let loaders: Vec<LoaderForGame> = serde_json::from_str(
            r#"[
              {"loader":{"separator":".","build":0,"maven":"x","version":"0.21.0-beta","stable":false},"intermediary":{}},
              {"loader":{"separator":".","build":1,"maven":"x","version":"0.20.1","stable":true},"intermediary":{}},
              {"loader":{"separator":".","build":0,"maven":"x","version":"0.20.0","stable":false},"intermediary":{}}
            ]"#,
        )
        .unwrap();
        assert_eq!(pick(loaders.iter().map(|l| &l.loader)), Some("0.20.1"));
    }

    #[test]
    fn falls_back_to_newest_when_nothing_stable() {
        let v = vec![
            VersionEntry { version: "2".into(), stable: false },
            VersionEntry { version: "1".into(), stable: false },
        ];
        assert_eq!(pick(v.iter()), Some("2"));
    }

    #[test]
    fn encodes_url_segments() {
        let url = meta_url(&["versions", "loader", "1.14 Pre-Release 1"]).unwrap();
        assert_eq!(url, "https://meta.fabricmc.net/v2/versions/loader/1.14%20Pre-Release%201");
    }
}
