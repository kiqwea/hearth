//! Paper через Fill API v3 (`https://fill.papermc.io/v3`).
//!
//! Ссылку на скачивание и sha256 берём из ответа API, сами ничего не собираем.
//! Из ответа же берём минимальную Java и рекомендованные флаги JVM.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use super::{DownloadSpec, GameVersion, ServerArtifact};
use crate::error::{Error, Result};
use crate::java;
use crate::net::{Checksum, Http};

const SERVICE: &str = "PaperMC";
const API: &str = "https://fill.papermc.io/v3/projects/paper";
const VERSIONS_TTL: Duration = Duration::from_secs(60 * 60);
const BUILDS_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Deserialize, Debug)]
pub struct VersionsResponse {
    pub versions: Vec<VersionItem>,
}

#[derive(Deserialize, Debug)]
pub struct VersionItem {
    pub version: VersionInfo,
    #[serde(default)]
    pub builds: Vec<u32>,
}

#[derive(Deserialize, Debug)]
pub struct VersionInfo {
    pub id: String,
    pub java: Option<JavaInfo>,
}

#[derive(Deserialize, Debug)]
pub struct JavaInfo {
    pub version: Option<JavaRequirement>,
    pub flags: Option<JavaFlags>,
}

#[derive(Deserialize, Debug)]
pub struct JavaRequirement {
    pub minimum: u32,
}

#[derive(Deserialize, Debug)]
pub struct JavaFlags {
    #[serde(default)]
    pub recommended: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct Build {
    pub id: u32,
    pub channel: String,
    #[serde(default)]
    pub downloads: HashMap<String, BuildDownload>,
}

#[derive(Deserialize, Debug)]
pub struct BuildDownload {
    pub name: String,
    pub checksums: Option<Checksums>,
    pub url: String,
}

#[derive(Deserialize, Debug)]
pub struct Checksums {
    pub sha256: Option<String>,
}

/// RC, pre-release и прочие предварительные версии содержат дефис: `26.3-rc-3`.
fn is_stable_id(id: &str) -> bool {
    !id.contains('-')
}

async fn versions(http: &Http) -> Result<VersionsResponse> {
    http.get_json_cached(
        SERVICE,
        &format!("{API}/versions"),
        "paper-versions.json",
        Some(VERSIONS_TTL),
    )
    .await
}

pub fn versions_from_response(resp: &VersionsResponse, include_unstable: bool) -> Vec<GameVersion> {
    resp.versions
        .iter()
        .filter(|v| !v.builds.is_empty())
        .filter(|v| include_unstable || is_stable_id(&v.version.id))
        .map(|v| GameVersion {
            id: v.version.id.clone(),
            stable: is_stable_id(&v.version.id),
            release_time: None,
        })
        .collect()
}

pub async fn list_versions(http: &Http, include_unstable: bool) -> Result<Vec<GameVersion>> {
    Ok(versions_from_response(&versions(http).await?, include_unstable))
}

/// Самая свежая сборка из списка.
pub fn pick_build(builds: Vec<Build>) -> Option<Build> {
    builds.into_iter().max_by_key(|b| b.id)
}

/// Файл сервера из сборки: основной ключ `server:default`, иначе любой.
pub fn build_download(build: &Build) -> Option<&BuildDownload> {
    build
        .downloads
        .get("server:default")
        .or_else(|| build.downloads.values().next())
}

async fn builds(http: &Http, id: &str, stable_only: bool) -> Result<Vec<Build>> {
    let (url, cache) = if stable_only {
        (
            format!("{API}/versions/{id}/builds?channel=STABLE"),
            format!("paper-builds-{id}-stable.json"),
        )
    } else {
        (
            format!("{API}/versions/{id}/builds"),
            format!("paper-builds-{id}-all.json"),
        )
    };
    http.get_json_cached(SERVICE, &url, &cache, Some(BUILDS_TTL))
        .await
        .map_err(|e| not_found_to_version(e, id))
}

fn not_found_to_version(e: Error, id: &str) -> Error {
    if e.is_not_found() {
        Error::VersionNotFound {
            loader: "Paper".into(),
            version: id.into(),
        }
    } else {
        e
    }
}

pub async fn resolve(http: &Http, id: &str) -> Result<ServerArtifact> {
    let all = versions(http).await?;
    let info = all
        .versions
        .iter()
        .find(|v| v.version.id == id)
        .ok_or_else(|| Error::VersionNotFound {
            loader: "Paper".into(),
            version: id.into(),
        })?;

    let mut stable = builds(http, id, true).await?;
    if stable.is_empty() && !is_stable_id(id) {
        // У предварительных версий стабильных сборок обычно нет вовсе.
        stable = builds(http, id, false).await?;
    }
    let build = pick_build(stable).ok_or_else(|| {
        Error::other(format!(
            "Для Paper {id} пока нет стабильной сборки. Выберите другую версию или подождите выхода сборки."
        ))
    })?;
    if build.channel != "STABLE" {
        tracing::warn!("Paper {id}: используется нестабильная сборка {} ({})", build.id, build.channel);
    }
    let download = build_download(&build)
        .ok_or_else(|| Error::other(format!("В сборке Paper {id}#{} нет файла сервера", build.id)))?;

    let java_info = info.version.java.as_ref();
    let java_major = java_info
        .and_then(|j| j.version.as_ref())
        .map(|v| v.minimum)
        .unwrap_or_else(|| java::fallback_major(id));
    let flags = java_info
        .and_then(|j| j.flags.as_ref())
        .map(|f| f.recommended.clone())
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| java::default_jvm_flags(java_major));

    Ok(ServerArtifact {
        jar: DownloadSpec {
            url: download.url.clone(),
            // Имя без номера сборки: при обновлении не копятся старые jar.
            file_name: "paper.jar".into(),
            checksum: download
                .checksums
                .as_ref()
                .and_then(|c| c.sha256.clone())
                .map(Checksum::Sha256),
        },
        extra: Vec::new(),
        java_major,
        jvm_flags: flags,
        loader_version: Some(build.id.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSIONS: &str = r#"{"versions":[
      {"version":{"id":"26.3","support":{"status":"SUPPORTED"},"java":{"version":{"minimum":25},"flags":{"recommended":["-XX:+UseG1GC","-XX:+AlwaysPreTouch"]}}},"builds":[3,4,258]},
      {"version":{"id":"26.3-rc-3","support":{"status":"UNSUPPORTED","end":"2026-09-15"},"java":{"version":{"minimum":25},"flags":{"recommended":[]}}},"builds":[1]},
      {"version":{"id":"1.16.5","support":{"status":"UNSUPPORTED"},"java":{"version":{"minimum":8},"flags":{"recommended":["-XX:+UseG1GC"]}}},"builds":[794]},
      {"version":{"id":"1.8.8","support":{"status":"UNSUPPORTED"}},"builds":[]}
    ]}"#;

    const BUILDS: &str = r#"[
      {"id":131,"time":"2026-05-03T16:08:07Z","channel":"STABLE","commits":[],"downloads":{"server:default":{"name":"paper-1.21.11-131.jar","checksums":{"sha256":"aaa"},"size":1,"url":"https://fill-data.papermc.io/v1/objects/aaa/paper-1.21.11-131.jar"}}},
      {"id":132,"time":"2026-05-11T11:43:09Z","channel":"STABLE","commits":[],"downloads":{"server:default":{"name":"paper-1.21.11-132.jar","checksums":{"sha256":"5ffef465eeeb5f2a3c23a24419d97c51afd7dbb4923ff42df9a3f58bba1ccfba"},"size":54846016,"url":"https://fill-data.papermc.io/v1/objects/5ffe/paper-1.21.11-132.jar"}}}
    ]"#;

    #[test]
    fn lists_only_versions_with_builds() {
        let r: VersionsResponse = serde_json::from_str(VERSIONS).unwrap();
        let stable: Vec<_> = versions_from_response(&r, false).into_iter().map(|v| v.id).collect();
        assert_eq!(stable, ["26.3", "1.16.5"]);
        let all = versions_from_response(&r, true);
        assert_eq!(all.len(), 3);
        assert!(!all[1].stable);
    }

    #[test]
    fn picks_latest_build_and_its_hash() {
        let builds: Vec<Build> = serde_json::from_str(BUILDS).unwrap();
        let build = pick_build(builds).unwrap();
        assert_eq!(build.id, 132);
        let d = build_download(&build).unwrap();
        assert_eq!(
            d.checksums.as_ref().unwrap().sha256.as_deref(),
            Some("5ffef465eeeb5f2a3c23a24419d97c51afd7dbb4923ff42df9a3f58bba1ccfba")
        );
    }
}
