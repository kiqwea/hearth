//! Purpur через API v2 (`https://api.purpurmc.org/v2/purpur`).
//!
//! Purpur — форк Paper с дополнительными настройками: плагины те же, команды
//! те же. В API нет требований к Java, поэтому их берём у Paper той же версии.

use std::time::Duration;

use serde::Deserialize;

use super::{DownloadSpec, GameVersion, ServerArtifact};
use crate::error::{Error, Result};
use crate::java;
use crate::net::{Checksum, Http};

const SERVICE: &str = "PurpurMC";
const API: &str = "https://api.purpurmc.org/v2/purpur";
const VERSIONS_TTL: Duration = Duration::from_secs(60 * 60);
const BUILDS_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Deserialize, Debug)]
pub struct VersionsResponse {
    pub versions: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct BuildsResponse {
    pub builds: Builds,
}

#[derive(Deserialize, Debug)]
pub struct Builds {
    pub latest: Option<String>,
    #[serde(default)]
    pub all: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct BuildInfo {
    pub build: String,
    #[serde(default)]
    pub result: String,
    pub md5: Option<String>,
}

/// Snapshot-, pre- и rc-версии содержат дефис.
fn is_stable_id(id: &str) -> bool {
    !id.contains('-')
}

/// API отдаёт версии от старых к новым; Hearth показывает новые первыми.
pub fn versions_from_response(resp: &VersionsResponse, include_unstable: bool) -> Vec<GameVersion> {
    resp.versions
        .iter()
        .rev()
        .filter(|id| include_unstable || is_stable_id(id))
        .map(|id| GameVersion {
            id: id.clone(),
            stable: is_stable_id(id),
            release_time: None,
        })
        .collect()
}

pub async fn list_versions(http: &Http, include_unstable: bool) -> Result<Vec<GameVersion>> {
    let resp: VersionsResponse = http
        .get_json_cached(SERVICE, API, "purpur-versions.json", Some(VERSIONS_TTL))
        .await?;
    Ok(versions_from_response(&resp, include_unstable))
}

/// Последняя сборка: `latest`, иначе самая большая из списка.
pub fn pick_build(builds: &Builds) -> Option<String> {
    builds
        .latest
        .clone()
        .or_else(|| builds.all.iter().max_by_key(|b| b.parse::<u64>().unwrap_or(0)).cloned())
}

fn not_found(e: Error, id: &str) -> Error {
    if e.is_not_found() {
        Error::VersionNotFound { loader: "Purpur".into(), version: id.into() }
    } else {
        e
    }
}

pub async fn resolve(http: &Http, id: &str) -> Result<ServerArtifact> {
    let builds: BuildsResponse = http
        .get_json_cached(SERVICE, &format!("{API}/{id}"), &format!("purpur-builds-{id}.json"), Some(BUILDS_TTL))
        .await
        .map_err(|e| not_found(e, id))?;
    let build = pick_build(&builds.builds)
        .ok_or_else(|| Error::other(format!("Для Purpur {id} пока нет сборок. Выберите другую версию.")))?;
    let info: BuildInfo = http
        .get_json(SERVICE, &format!("{API}/{id}/{build}"))
        .await
        .map_err(|e| not_found(e, id))?;
    if !info.result.is_empty() && info.result != "SUCCESS" {
        tracing::warn!("Purpur {id}#{}: сборка с результатом {}", info.build, info.result);
    }

    let (java_major, jvm_flags) = match super::paper::java_for(http, id).await {
        Some(found) => found,
        None => {
            let major = java::fallback_major(id);
            (major, java::default_jvm_flags(major))
        }
    };

    Ok(ServerArtifact {
        jar: DownloadSpec {
            // Ссылка на конкретную сборку: хэш из ответа выше относится именно к ней.
            url: format!("{API}/{id}/{}/download", info.build),
            file_name: "purpur.jar".into(),
            checksum: info.md5.map(Checksum::Md5),
        },
        extra: Vec::new(),
        java_major,
        jvm_flags,
        loader_version: Some(info.build),
        installer_args: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_versions_first() {
        let r: VersionsResponse =
            serde_json::from_str(r#"{"project":"purpur","metadata":{"current":"26.3"},"versions":["1.21.11","26.1.2","26.3-rc-1","26.3"]}"#).unwrap();
        let ids: Vec<String> = versions_from_response(&r, false).into_iter().map(|v| v.id).collect();
        assert_eq!(ids, ["26.3", "26.1.2", "1.21.11"]);
        assert_eq!(versions_from_response(&r, true).len(), 4);
    }

    #[test]
    fn picks_latest_build() {
        let r: BuildsResponse = serde_json::from_str(
            r#"{"project":"purpur","version":"26.3","builds":{"latest":"2660","all":["2634","2660","2659"]}}"#,
        )
        .unwrap();
        assert_eq!(pick_build(&r.builds).as_deref(), Some("2660"));
        let no_latest = Builds { latest: None, all: vec!["9".into(), "10".into()] };
        assert_eq!(pick_build(&no_latest).as_deref(), Some("10"));
        let info: BuildInfo = serde_json::from_str(
            r#"{"project":"purpur","version":"26.3","build":"2660","result":"SUCCESS","md5":"e86279fc738723c5e406417887f7aa33"}"#,
        )
        .unwrap();
        assert_eq!(info.md5.as_deref(), Some("e86279fc738723c5e406417887f7aa33"));
    }
}
