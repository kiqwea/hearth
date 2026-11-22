//! Vanilla: манифест Mojang → JSON версии → `downloads.server`.

use std::time::Duration;

use serde::Deserialize;

use super::{DownloadSpec, GameVersion, ServerArtifact};
use crate::error::{Error, Result};
use crate::java;
use crate::net::{Checksum, Http};

const SERVICE: &str = "Mojang";
const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const MANIFEST_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Deserialize, Debug)]
pub struct Manifest {
    pub versions: Vec<ManifestEntry>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub release_time: String,
    pub sha1: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VersionJson {
    pub java_version: Option<JavaVersion>,
    pub downloads: Downloads,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    pub major_version: u32,
}

#[derive(Deserialize, Debug)]
pub struct Downloads {
    pub server: Option<DownloadEntry>,
}

#[derive(Deserialize, Debug)]
pub struct DownloadEntry {
    pub sha1: String,
    pub url: String,
}

pub async fn manifest(http: &Http) -> Result<Manifest> {
    http.get_json_cached(SERVICE, MANIFEST_URL, "mojang-manifest.json", Some(MANIFEST_TTL))
        .await
}

/// Релизы, а при `include_unstable` ещё и снапшоты. Альфы и беты 2010 года
/// не показываем: серверов для них почти нет.
pub fn versions_from_manifest(manifest: &Manifest, include_unstable: bool) -> Vec<GameVersion> {
    manifest
        .versions
        .iter()
        .filter(|v| v.kind == "release" || (include_unstable && v.kind == "snapshot"))
        .map(|v| GameVersion {
            id: v.id.clone(),
            stable: v.kind == "release",
            release_time: Some(v.release_time.clone()),
        })
        .collect()
}

pub async fn list_versions(http: &Http, include_unstable: bool) -> Result<Vec<GameVersion>> {
    Ok(versions_from_manifest(&manifest(http).await?, include_unstable))
}

async fn version_json(http: &Http, id: &str) -> Result<VersionJson> {
    let manifest = manifest(http).await?;
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == id)
        .ok_or_else(|| Error::VersionNotFound {
            loader: "Vanilla".into(),
            version: id.into(),
        })?;
    // Содержимое JSON версии не меняется, пока не меняется его sha1,
    // поэтому кэшируем навсегда под именем с хэшем.
    let cache_name = format!("mojang-version-{}.json", entry.sha1);
    http.get_json_cached(SERVICE, &entry.url, &cache_name, None).await
}

/// Нужная версия Java по данным Mojang, с запасным вариантом по таблице.
pub async fn java_major(http: &Http, id: &str) -> u32 {
    match version_json(http, id).await {
        Ok(v) => v
            .java_version
            .map(|j| j.major_version)
            .unwrap_or_else(|| java::fallback_major(id)),
        Err(e) => {
            tracing::warn!("Не удалось узнать версию Java для {id} у Mojang: {e}");
            java::fallback_major(id)
        }
    }
}

/// Ванильный server.jar: используется и самим Vanilla, и как основа для Fabric.
pub async fn server_download(http: &Http, id: &str) -> Result<(DownloadSpec, u32)> {
    let v = version_json(http, id).await?;
    let server = v.downloads.server.ok_or_else(|| {
        Error::other(format!("Для версии {id} у Mojang нет серверного файла"))
    })?;
    let java = v
        .java_version
        .map(|j| j.major_version)
        .unwrap_or_else(|| java::fallback_major(id));
    Ok((
        DownloadSpec {
            url: server.url,
            file_name: "server.jar".into(),
            checksum: Some(Checksum::Sha1(server.sha1)),
        },
        java,
    ))
}

pub async fn resolve(http: &Http, id: &str) -> Result<ServerArtifact> {
    let (jar, java_major) = server_download(http, id).await?;
    Ok(ServerArtifact {
        jar,
        extra: Vec::new(),
        java_major,
        jvm_flags: java::default_jvm_flags(java_major),
        loader_version: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{
      "latest": {"release": "26.3", "snapshot": "26.4-snapshot-10"},
      "versions": [
        {"id": "26.4-snapshot-10", "type": "snapshot", "url": "https://example/1.json", "time": "2026-11-18T10:59:19+00:00", "releaseTime": "2026-11-17T16:02:02+00:00", "sha1": "cdbd", "complianceLevel": 1},
        {"id": "26.3", "type": "release", "url": "https://example/2.json", "time": "2026-11-01T00:00:00+00:00", "releaseTime": "2026-10-28T10:00:00+00:00", "sha1": "aaaa", "complianceLevel": 1},
        {"id": "1.16.5", "type": "release", "url": "https://example/3.json", "time": "2021-01-14T00:00:00+00:00", "releaseTime": "2021-01-14T16:05:32+00:00", "sha1": "bbbb", "complianceLevel": 0},
        {"id": "b1.7.3", "type": "old_beta", "url": "https://example/4.json", "time": "2011-07-08T00:00:00+00:00", "releaseTime": "2011-07-07T22:00:00+00:00", "sha1": "cccc", "complianceLevel": 0}
      ]
    }"#;

    const VERSION: &str = r#"{
      "javaVersion": {"component": "java-runtime-epsilon", "majorVersion": 25},
      "downloads": {
        "client": {"sha1": "x", "size": 1, "url": "https://example/client.jar"},
        "server": {"sha1": "33680f5f2ac32864d6d7cf5e56a705fdb3e05f4c", "size": 62294556, "url": "https://piston-data.mojang.com/v1/objects/33680f5f2ac32864d6d7cf5e56a705fdb3e05f4c/server.jar"}
      }
    }"#;

    #[test]
    fn filters_releases_and_snapshots() {
        let m: Manifest = serde_json::from_str(MANIFEST).unwrap();
        let stable = versions_from_manifest(&m, false);
        assert_eq!(
            stable.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            ["26.3", "1.16.5"]
        );
        let all = versions_from_manifest(&m, true);
        assert_eq!(all.len(), 3);
        assert!(!all[0].stable);
    }

    #[test]
    fn parses_version_json() {
        let v: VersionJson = serde_json::from_str(VERSION).unwrap();
        assert_eq!(v.java_version.unwrap().major_version, 25);
        assert!(v.downloads.server.unwrap().url.ends_with("server.jar"));
    }
}
