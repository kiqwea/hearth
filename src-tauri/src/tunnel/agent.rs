//! Официальный агент playit (`playitd.exe`): скачивание с GitHub и запуск.
//!
//! Hearth не распространяет агент вместе с собой, а скачивает подписанную
//! сборку из релизов github.com/playit-cloud/playit-agent и проверяет SHA-256,
//! который GitHub отдаёт для каждого файла релиза. Берём только версии 2.x:
//! под их параметры запуска написан этот модуль.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::net::{Checksum, Http};

const RELEASES_URL: &str = "https://api.github.com/repos/playit-cloud/playit-agent/releases?per_page=30";
const MAJOR_PREFIX: &str = "v2.";
/// Подписанная сборка в приоритете, обычная на случай, если подписанной нет.
const ASSETS: [&str; 2] = ["playitd-windows-x86_64-signed.exe", "playitd-windows-x86_64.exe"];
const FILE_PREFIX: &str = "playitd-";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

/// Подходящий релиз: последний стабильный 2.x, в котором есть агент для Windows x64.
fn pick_release(releases: &[Release]) -> Option<(&Release, &Asset)> {
    releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease && r.tag_name.starts_with(MAJOR_PREFIX))
        .find_map(|r| {
            ASSETS
                .iter()
                .find_map(|name| r.assets.iter().find(|a| &a.name == name))
                .map(|a| (r, a))
        })
}

fn parse_digest(digest: Option<&str>) -> Option<Checksum> {
    let hex = digest?.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| Checksum::Sha256(hex.to_ascii_lowercase()))
}

/// Тег из имени файла `playitd-v2.0.2.exe`, только для безопасных имён.
fn sanitize_tag(tag: &str) -> Option<&str> {
    (!tag.is_empty() && tag.len() <= 32 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .then_some(tag)
}

fn version_key(tag: &str) -> Vec<u64> {
    tag.trim_start_matches('v')
        .split(['.', '-'])
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

/// Уже скачанные агенты, новые первыми.
fn local_binaries(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let tag = name.strip_prefix(FILE_PREFIX)?.strip_suffix(".exe")?.to_string();
            tag.starts_with(MAJOR_PREFIX).then(|| (tag, entry.path()))
        })
        .collect();
    found.sort_by(|a, b| version_key(&b.0).cmp(&version_key(&a.0)));
    found
}

/// Путь к агенту: скачивает новую версию при необходимости. Если GitHub
/// недоступен, а агент уже скачан раньше, работаем со старым.
pub async fn ensure_binary(http: &Http, dir: &Path) -> Result<PathBuf> {
    let releases = http
        .get_json_cached::<Vec<Release>>("GitHub", RELEASES_URL, "playit-releases.json", Some(Duration::from_secs(24 * 3600)))
        .await;
    let local = local_binaries(dir);
    let releases = match releases {
        Ok(r) => r,
        Err(e) => {
            if let Some((tag, path)) = local.into_iter().next() {
                tracing::warn!("Не удалось проверить версию агента playit ({e}), использую {tag}");
                return Ok(path);
            }
            return Err(e);
        }
    };
    let Some((release, asset)) = pick_release(&releases) else {
        if let Some((_, path)) = local.into_iter().next() {
            return Ok(path);
        }
        return Err(Error::other("В релизах playit на GitHub не нашлось агента для Windows"));
    };
    let tag = sanitize_tag(&release.tag_name)
        .ok_or_else(|| Error::other("Неожиданное имя версии агента playit"))?;
    let path = dir.join(format!("{FILE_PREFIX}{tag}.exe"));
    if path.is_file() {
        return Ok(path);
    }
    tracing::info!("Скачиваю агент playit {tag}");
    http.download(&asset.browser_download_url, &path, parse_digest(asset.digest.as_deref()).as_ref(), |_, _| {})
        .await?;
    // Старые версии больше не нужны. Если какая-то сейчас запущена, удалится в следующий раз.
    for (old_tag, old_path) in local {
        if old_tag != tag {
            let _ = std::fs::remove_file(old_path);
        }
    }
    Ok(path)
}

/// Запускает агент без окна. Ключ передаётся аргументом: агент не сохраняет
/// его на диск, а Hearth не пишет командную строку в логи.
pub fn spawn(binary: &Path, secret: &str, dir: &Path) -> Result<tokio::process::Child> {
    let logs = dir.join("logs");
    std::fs::create_dir_all(&logs)?;
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg("--secret")
        .arg(secret)
        // Свой канал управления, чтобы не пересекаться с playit, если он установлен отдельно.
        .arg("--socket-path")
        .arg(format!(r"\\.\pipe\hearth-playit-{}", std::process::id()))
        .arg("--log-path")
        .arg(logs.join("playitd.log"))
        .arg("--cert-dir")
        .arg(dir.join("certs"))
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map_err(|e| Error::other(format!("Не удалось запустить агент playit: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            assets: assets
                .iter()
                .map(|n| Asset {
                    name: n.to_string(),
                    browser_download_url: format!("https://example/{n}"),
                    digest: Some(format!("sha256:{}", "a".repeat(64))),
                })
                .collect(),
        }
    }

    #[test]
    fn picks_newest_stable_v2_with_signed_build() {
        let releases = vec![
            release("v3.0.0", false, &["playitd-windows-x86_64-signed.exe"]),
            release("v2.1.0-rc1", true, &["playitd-windows-x86_64-signed.exe"]),
            release("v2.0.2", false, &["playitd-windows-x86_64.exe", "playitd-windows-x86_64-signed.exe"]),
            release("v2.0.1", false, &["playitd-windows-x86_64-signed.exe"]),
        ];
        let (r, a) = pick_release(&releases).unwrap();
        assert_eq!(r.tag_name, "v2.0.2");
        assert_eq!(a.name, "playitd-windows-x86_64-signed.exe");
    }

    #[test]
    fn digest_parsing() {
        let hex = "6e62da9f56a3bcf72e0c62c29e94609c847cc9b110c460f68b20035525985238";
        assert_eq!(parse_digest(Some(&format!("sha256:{hex}"))), Some(Checksum::Sha256(hex.into())));
        assert_eq!(parse_digest(Some("sha1:abc")), None);
        assert_eq!(parse_digest(None), None);
    }

    #[test]
    fn tags_are_sanitized_and_sorted() {
        assert_eq!(sanitize_tag("v2.0.2"), Some("v2.0.2"));
        assert_eq!(sanitize_tag("../evil"), None);
        let dir = tempfile::tempdir().unwrap();
        for name in ["playitd-v2.0.2.exe", "playitd-v2.0.10.exe", "playitd-v1.9.exe", "other.exe"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        let tags: Vec<String> = local_binaries(dir.path()).into_iter().map(|(t, _)| t).collect();
        assert_eq!(tags, vec!["v2.0.10", "v2.0.2"]);
    }
}
