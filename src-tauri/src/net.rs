//! Общий HTTP-клиент: User-Agent, кэш ответов API, скачивание с проверкой хэша.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};
use crate::fsx;

pub const USER_AGENT: &str = concat!(
    "Hearth/",
    env!("CARGO_PKG_VERSION"),
    " (desktop Minecraft server manager)"
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checksum {
    Sha1(String),
    Sha256(String),
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    cache_dir: PathBuf,
}

impl Http {
    pub fn new(cache_dir: PathBuf) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self { client, cache_dir })
    }

    pub async fn get_text(&self, service: &str, url: &str) -> Result<String> {
        let resp = self
            .client
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(api_error(service, status.as_u16(), &body));
        }
        Ok(resp.text().await?)
    }

    pub async fn get_json<T: DeserializeOwned>(&self, service: &str, url: &str) -> Result<T> {
        let text = self.get_text(service, url).await?;
        parse_json(service, &text)
    }

    /// JSON с кэшем на диске.
    ///
    /// `ttl = None` означает «кэшировать навсегда» (для неизменяемых данных,
    /// например JSON версии Mojang с хэшем в имени). Если сети нет, отдаём
    /// устаревший кэш: список версий лучше старый, чем никакой.
    pub async fn get_json_cached<T: DeserializeOwned>(
        &self,
        service: &str,
        url: &str,
        cache_name: &str,
        ttl: Option<Duration>,
    ) -> Result<T> {
        let path = self.cache_dir.join("api").join(cache_name);
        if let Some(text) = read_fresh(&path, ttl).await {
            if let Ok(value) = serde_json::from_str(&text) {
                return Ok(value);
            }
        }
        match self.get_text(service, url).await {
            Ok(text) => {
                let value = parse_json(service, &text)?;
                if let Err(e) = fsx::write_atomic(&path, text.as_bytes()).await {
                    tracing::warn!("Не удалось сохранить кэш {cache_name}: {e}");
                }
                Ok(value)
            }
            Err(err @ Error::Network(_)) => {
                if let Ok(text) = tokio::fs::read_to_string(&path).await {
                    if let Ok(value) = serde_json::from_str(&text) {
                        tracing::warn!("{service} недоступен, использую сохранённый кэш {cache_name}");
                        return Ok(value);
                    }
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }

    /// Скачивает файл во временный `*.part`, проверяет хэш и только потом
    /// переименовывает в `dest`. `progress(скачано, всего)` вызывается не чаще
    /// раза в 100 мс и один раз в конце.
    pub async fn download(
        &self,
        url: &str,
        dest: &Path,
        expected: Option<&Checksum>,
        mut progress: impl FnMut(u64, Option<u64>) + Send,
    ) -> Result<()> {
        let file_name = dest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "файл".into());
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let part = dest.with_file_name(format!("{file_name}.part"));

        let resp = self.client.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let host = resp.url().host_str().unwrap_or("сервер").to_string();
            let body = resp.text().await.unwrap_or_default();
            return Err(api_error(&host, status.as_u16(), &body));
        }
        let total = resp.content_length();

        let mut file = tokio::fs::File::create(&part).await?;
        let mut hasher = Hasher::for_checksum(expected);
        let mut stream = resp.bytes_stream();
        let mut done: u64 = 0;
        let mut last_report = Instant::now();
        let write_result: Result<()> = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                file.write_all(&chunk).await?;
                hasher.update(&chunk);
                done += chunk.len() as u64;
                if last_report.elapsed() >= Duration::from_millis(100) {
                    progress(done, total);
                    last_report = Instant::now();
                }
            }
            file.flush().await?;
            Ok(())
        }
        .await;
        drop(file);
        if let Err(e) = write_result {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(e);
        }
        progress(done, total);

        if let (Some(expected), Some(actual)) = (expected, hasher.finish()) {
            let expected_hex = match expected {
                Checksum::Sha1(h) | Checksum::Sha256(h) => h,
            };
            if !expected_hex.eq_ignore_ascii_case(&actual) {
                let _ = tokio::fs::remove_file(&part).await;
                tracing::warn!("{file_name}: ожидался хэш {expected_hex}, получен {actual}");
                return Err(Error::Checksum { file: file_name });
            }
        }
        fsx::rename_with_retry(&part, dest).await?;
        Ok(())
    }
}

enum Hasher {
    None,
    Sha1(Sha1),
    Sha256(Sha256),
}

impl Hasher {
    fn for_checksum(c: Option<&Checksum>) -> Self {
        match c {
            None => Hasher::None,
            Some(Checksum::Sha1(_)) => Hasher::Sha1(Sha1::new()),
            Some(Checksum::Sha256(_)) => Hasher::Sha256(Sha256::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Hasher::None => {}
            Hasher::Sha1(h) => h.update(data),
            Hasher::Sha256(h) => h.update(data),
        }
    }

    fn finish(self) -> Option<String> {
        match self {
            Hasher::None => None,
            Hasher::Sha1(h) => Some(hex::encode(h.finalize().as_slice())),
            Hasher::Sha256(h) => Some(hex::encode(h.finalize().as_slice())),
        }
    }
}

async fn read_fresh(path: &Path, ttl: Option<Duration>) -> Option<String> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    if let Some(ttl) = ttl {
        let modified = meta.modified().ok()?;
        let age = SystemTime::now().duration_since(modified).unwrap_or_default();
        if age > ttl {
            return None;
        }
    }
    tokio::fs::read_to_string(path).await.ok()
}

fn parse_json<T: DeserializeOwned>(service: &str, text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| Error::Parse {
        service: service.to_string(),
        message: e.to_string(),
    })
}

/// Вытаскивает человекочитаемое сообщение из тела ошибки API, если оно там есть.
fn api_error(service: &str, status: u16, body: &str) -> Error {
    let from_json = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| {
        ["message", "error", "description"]
            .iter()
            .find_map(|k| v.get(*k).and_then(|m| m.as_str()).map(str::to_string))
    });
    let message = from_json.unwrap_or_else(|| match status {
        404 => "не найдено".into(),
        403 => "доступ запрещён".into(),
        429 => "слишком много запросов, попробуйте через минуту".into(),
        500..=599 => "временная ошибка на стороне сервиса".into(),
        _ => body.chars().take(200).collect(),
    });
    Error::Api {
        service: service.to_string(),
        status,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_prefers_json_message() {
        let err = api_error(
            "Paper",
            404,
            r#"{"ok":false,"error":"version_not_found","message":"No version was found."}"#,
        );
        assert!(err.is_not_found());
        assert!(err.to_string().contains("No version was found."));
    }

    #[test]
    fn sha256_of_known_string() {
        let mut h = Hasher::for_checksum(Some(&Checksum::Sha256(String::new())));
        h.update(b"abc");
        assert_eq!(
            h.finish().unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha1_of_known_string() {
        let mut h = Hasher::for_checksum(Some(&Checksum::Sha1(String::new())));
        h.update(b"abc");
        assert_eq!(h.finish().unwrap(), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }
}
