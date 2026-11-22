//! Modrinth API v2 (`https://api.modrinth.com/v2`): поиск подходящего файла
//! плагина или мода под ядро и версию игры. Ключ не нужен, User-Agent обязателен
//! (его ставит общий HTTP-клиент).

use serde::Deserialize;

use crate::error::Result;
use crate::net::{Checksum, Http};

const API: &str = "https://api.modrinth.com/v2";
const SERVICE: &str = "Modrinth";

#[derive(Deserialize)]
struct Version {
    version_number: String,
    #[serde(default)]
    version_type: String,
    files: Vec<VersionFile>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct VersionFile {
    url: String,
    filename: String,
    #[serde(default)]
    primary: bool,
    hashes: Hashes,
}

#[derive(Deserialize)]
struct Hashes {
    sha1: Option<String>,
}

#[derive(Deserialize)]
struct Dependency {
    project_id: Option<String>,
    dependency_type: String,
}

/// Файл, который нужно скачать.
#[derive(Debug, Clone)]
pub struct ModFile {
    pub version: String,
    pub url: String,
    pub filename: String,
    pub checksum: Option<Checksum>,
    /// Обязательные зависимости (id проектов Modrinth).
    pub required: Vec<String>,
}

fn json_list(items: &[&str]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format!("%22{s}%22")).collect();
    format!("%5B{}%5D", inner.join(","))
}

/// Последний подходящий файл проекта для ядра (`loaders`) и версии игры.
/// `None`, если под эту версию игры ничего нет.
pub async fn latest_file(http: &Http, project: &str, loaders: &[&str], game_version: &str) -> Result<Option<ModFile>> {
    let url = format!(
        "{API}/project/{project}/version?loaders={}&game_versions={}",
        json_list(loaders),
        json_list(&[game_version])
    );
    let versions: Vec<Version> = match http.get_json(SERVICE, &url).await {
        Ok(v) => v,
        Err(e) if e.is_not_found() => return Ok(None),
        Err(e) => return Err(e),
    };
    // Стабильные версии в приоритете; список уже отсортирован от новых к старым.
    let version = versions
        .iter()
        .find(|v| v.version_type == "release")
        .or_else(|| versions.first());
    let Some(version) = version else { return Ok(None) };
    let Some(file) = version.files.iter().find(|f| f.primary).or_else(|| version.files.first()) else {
        return Ok(None);
    };
    Ok(Some(ModFile {
        version: version.version_number.clone(),
        url: file.url.clone(),
        filename: file.filename.clone(),
        checksum: file.hashes.sha1.clone().map(Checksum::Sha1),
        required: version
            .dependencies
            .iter()
            .filter(|d| d.dependency_type == "required")
            .filter_map(|d| d.project_id.clone())
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_query_lists() {
        assert_eq!(json_list(&["paper", "bukkit"]), "%5B%22paper%22,%22bukkit%22%5D");
    }

    #[tokio::test]
    #[ignore]
    async fn live_finds_auth_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let http = Http::new(dir.path().to_path_buf()).unwrap();
        let authme = latest_file(&http, "authmereloaded", &["paper"], "26.3").await.unwrap().unwrap();
        assert!(authme.filename.contains("Paper"), "{}", authme.filename);
        println!("AuthMe {} {}", authme.version, authme.filename);
        let easyauth = latest_file(&http, "easyauth", &["fabric"], "26.3").await.unwrap().unwrap();
        println!("EasyAuth {} {} deps {:?}", easyauth.version, easyauth.filename, easyauth.required);
        assert!(easyauth.required.contains(&"P7dR8mSH".to_string()));
        assert!(latest_file(&http, "authmereloaded", &["paper"], "1.0").await.unwrap().is_none());
    }
}
