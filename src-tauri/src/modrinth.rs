//! Modrinth API v2 (`https://api.modrinth.com/v2`): поиск плагинов и модов,
//! подходящий файл под ядро и версию игры, опознание файлов по sha1 и
//! проверка обновлений. Ключ не нужен, User-Agent обязателен (его ставит общий HTTP-клиент).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::net::{Checksum, Http};

const API: &str = "https://api.modrinth.com/v2";
const SERVICE: &str = "Modrinth";

#[derive(Deserialize, Debug, Clone)]
pub struct Version {
    pub id: String,
    pub project_id: String,
    pub version_number: String,
    #[serde(default)]
    pub version_type: String,
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    pub hashes: Hashes,
    #[serde(default)]
    pub size: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Hashes {
    pub sha1: Option<String>,
    pub sha512: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Dependency {
    pub project_id: Option<String>,
    pub dependency_type: String,
}

impl Version {
    /// Основной файл версии (у некоторых проектов есть ещё исходники и т.п.).
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files.iter().find(|f| f.primary).or_else(|| self.files.first())
    }
}

/// Файл, который нужно скачать.
#[derive(Debug, Clone)]
pub struct ModFile {
    pub project_id: String,
    pub version_id: String,
    pub version: String,
    pub url: String,
    pub filename: String,
    pub checksum: Option<Checksum>,
    /// Обязательные зависимости (id проектов Modrinth).
    pub required: Vec<String>,
    /// Канал: `release`, `beta` или `alpha`.
    pub channel: String,
    /// Релиз, а не альфа или бета.
    pub stable: bool,
}

impl ModFile {
    fn from_version(v: &Version) -> Option<Self> {
        let file = v.primary_file()?;
        Some(Self {
            project_id: v.project_id.clone(),
            version_id: v.id.clone(),
            version: v.version_number.clone(),
            stable: v.version_type == "release",
            channel: v.version_type.clone(),
            url: file.url.clone(),
            filename: file.filename.clone(),
            checksum: file.hashes.sha1.clone().map(Checksum::Sha1),
            required: v
                .dependencies
                .iter()
                .filter(|d| d.dependency_type == "required")
                .filter_map(|d| d.project_id.clone())
                .collect(),
        })
    }
}

/// Список для query-параметров Modrinth: `["a","b"]`, закодированный для URL.
fn json_list(items: &[&str]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format!("%22{}%22", encode(s))).collect();
    format!("%5B{}%5D", inner.join(","))
}

/// Процентное кодирование для query-параметров.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Последний подходящий файл проекта для ядра (`loaders`) и версии игры.
/// `None`, если под эту версию игры ничего нет.
pub async fn latest_file(http: &Http, project: &str, loaders: &[&str], game_version: &str) -> Result<Option<ModFile>> {
    let url = format!(
        "{API}/project/{}/version?loaders={}&game_versions={}",
        encode(project),
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
    Ok(version.and_then(ModFile::from_version))
}

// ---- поиск ----

#[derive(Deserialize, Debug, Clone)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    #[serde(default)]
    pub total_hits: u32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub icon_url: Option<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub client_side: String,
    #[serde(default)]
    pub server_side: String,
}

/// Поиск проектов для ядра и версии игры, которые работают на сервере.
/// Пустой запрос — самые популярные.
pub async fn search(
    http: &Http,
    query: &str,
    categories: &[&str],
    project_type: &str,
    game_version: &str,
    offset: u32,
    limit: u32,
) -> Result<SearchResponse> {
    let loaders: Vec<String> = categories.iter().map(|c| format!("categories:{c}")).collect();
    let loaders: Vec<&str> = loaders.iter().map(String::as_str).collect();
    let version = format!("versions:{game_version}");
    let kind = format!("project_type:{project_type}");
    // Фасеты: внутри списка — «или», между списками — «и».
    let facets = format!(
        "%5B{},{},{},{}%5D",
        json_list(&loaders),
        json_list(&[&version]),
        json_list(&[&kind]),
        json_list(&["server_side:required", "server_side:optional"]),
    );
    let index = if query.trim().is_empty() { "downloads" } else { "relevance" };
    let url = format!(
        "{API}/search?query={}&facets={facets}&index={index}&offset={offset}&limit={limit}",
        encode(query.trim())
    );
    http.get_json(SERVICE, &url).await
}

// ---- проекты и файлы ----

#[derive(Deserialize, Debug, Clone)]
pub struct Project {
    pub id: String,
    pub slug: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub icon_url: Option<String>,
    #[serde(default)]
    pub client_side: String,
    #[serde(default)]
    pub server_side: String,
}

pub async fn projects(http: &Http, ids: &[&str]) -> Result<Vec<Project>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    http.get_json(SERVICE, &format!("{API}/projects?ids={}", json_list(ids))).await
}

#[derive(Serialize)]
struct HashesRequest<'a> {
    hashes: &'a [String],
    algorithm: &'static str,
}

/// Какие версии каких проектов лежат в папке: sha1 файла → версия Modrinth.
/// Файлов не с Modrinth в ответе нет.
pub async fn versions_by_hash(http: &Http, sha1: &[String]) -> Result<HashMap<String, Version>> {
    if sha1.is_empty() {
        return Ok(HashMap::new());
    }
    http.post_json(SERVICE, &format!("{API}/version_files"), &HashesRequest { hashes: sha1, algorithm: "sha1" })
        .await
}

#[derive(Serialize)]
struct UpdatesRequest<'a> {
    hashes: &'a [String],
    algorithm: &'static str,
    loaders: &'a [&'a str],
    game_versions: [&'a str; 1],
}

/// Самая свежая версия для каждого файла под это ядро и версию игры: sha1 → версия.
pub async fn latest_by_hash(
    http: &Http,
    sha1: &[String],
    loaders: &[&str],
    game_version: &str,
) -> Result<HashMap<String, Version>> {
    if sha1.is_empty() {
        return Ok(HashMap::new());
    }
    let body = UpdatesRequest { hashes: sha1, algorithm: "sha1", loaders, game_versions: [game_version] };
    http.post_json(SERVICE, &format!("{API}/version_files/update"), &body).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_query_lists() {
        assert_eq!(json_list(&["paper", "bukkit"]), "%5B%22paper%22,%22bukkit%22%5D");
        assert_eq!(encode("world edit/ё"), "world%20edit%2F%D1%91");
    }

    #[test]
    fn primary_file_and_dependencies() {
        let v: Version = serde_json::from_str(
            r#"{"id":"v1","project_id":"p1","version_number":"1.0","version_type":"release",
                "files":[{"url":"https://x/src.jar","filename":"src.jar","primary":false,"hashes":{"sha1":"a"}},
                         {"url":"https://x/mod.jar","filename":"mod.jar","primary":true,"hashes":{"sha1":"b"},"size":10}],
                "dependencies":[{"project_id":"P7dR8mSH","dependency_type":"required"},{"project_id":"x","dependency_type":"optional"}]}"#,
        )
        .unwrap();
        let f = ModFile::from_version(&v).unwrap();
        assert_eq!(f.filename, "mod.jar");
        assert_eq!(f.checksum, Some(Checksum::Sha1("b".into())));
        assert_eq!(f.required, ["P7dR8mSH"]);
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

    #[tokio::test]
    #[ignore]
    async fn live_search_and_hash_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let http = Http::new(dir.path().to_path_buf()).unwrap();
        let found = search(&http, "", &["fabric"], "mod", "26.3", 0, 5).await.unwrap();
        assert!(!found.hits.is_empty() && found.total_hits > 5);
        println!("Fabric популярные: {:?}", found.hits.iter().map(|h| &h.title).collect::<Vec<_>>());
        let plugins = search(&http, "luckperms", &["paper", "purpur", "spigot", "bukkit"], "plugin", "26.3", 0, 5).await.unwrap();
        println!("Плагины: {:?}", plugins.hits.iter().map(|h| &h.title).collect::<Vec<_>>());
        assert!(plugins.hits.iter().any(|h| h.title.contains("LuckPerms")));

        let api = latest_file(&http, "P7dR8mSH", &["fabric"], "26.3").await.unwrap().unwrap();
        let sha1 = match api.checksum.unwrap() {
            Checksum::Sha1(h) => h,
            _ => unreachable!(),
        };
        let known = versions_by_hash(&http, std::slice::from_ref(&sha1)).await.unwrap();
        assert_eq!(known[&sha1].project_id, "P7dR8mSH");
        let latest = latest_by_hash(&http, std::slice::from_ref(&sha1), &["fabric"], "26.3").await.unwrap();
        assert_eq!(latest[&sha1].id, api.version_id, "последняя версия совпадает");
        let p = projects(&http, &["P7dR8mSH"]).await.unwrap();
        assert_eq!(p[0].title, "Fabric API");
    }
}
