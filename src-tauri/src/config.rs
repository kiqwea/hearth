//! Настройки приложения (`%APPDATA%\Hearth\config.json`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::Result;
use crate::fsx;
use crate::paths::Paths;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AppConfig {
    /// Своя папка для серверов (например, на игровом SSD). `null` = папка по умолчанию.
    pub servers_dir: Option<String>,
    /// Сворачивать в трей при закрытии окна вместо выхода.
    pub close_to_tray: bool,
    /// Язык интерфейса.
    pub language: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            servers_dir: None,
            close_to_tray: true,
            language: "ru".into(),
        }
    }
}

impl AppConfig {
    /// Читает настройки. Если файла нет или он повреждён, возвращает значения
    /// по умолчанию: из-за битого конфига приложение не должно перестать запускаться.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("config.json повреждён ({e}), использую настройки по умолчанию");
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!("Не удалось прочитать config.json: {e}");
                Self::default()
            }
        }
    }

    pub async fn save(&self, path: &Path) -> Result<()> {
        let data = serde_json::to_vec_pretty(self).expect("config serializes");
        fsx::write_atomic(path, &data).await?;
        Ok(())
    }

    pub fn servers_dir(&self, paths: &Paths) -> PathBuf {
        match &self.servers_dir {
            Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
            _ => paths.default_servers_dir(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let cfg: AppConfig = serde_json::from_str(r#"{"language":"en"}"#).unwrap();
        assert!(cfg.close_to_tray);
        assert_eq!(cfg.language, "en");
        assert_eq!(cfg.servers_dir, None);
    }

    #[test]
    fn broken_file_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(AppConfig::load(&path), AppConfig::default());
    }
}
