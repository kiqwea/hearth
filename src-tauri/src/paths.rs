//! Где Hearth хранит данные.
//!
//! - `%APPDATA%\Hearth\config.json`: настройки приложения (лёгкие, можно в roaming).
//! - `%LOCALAPPDATA%\Hearth\`: всё тяжёлое (JRE, кэш, логи, агент playit, серверы по умолчанию).

use std::path::PathBuf;

use crate::error::{Error, Result};

pub const APP_DIR: &str = "Hearth";

#[derive(Clone, Debug)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn from_system() -> Result<Self> {
        let config = dirs::config_dir()
            .ok_or_else(|| Error::other("Не удалось определить папку %APPDATA%"))?;
        let data = dirs::data_local_dir()
            .ok_or_else(|| Error::other("Не удалось определить папку %LOCALAPPDATA%"))?;
        Ok(Self::new(config.join(APP_DIR), data.join(APP_DIR)))
    }

    pub fn new(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self { config_dir, data_dir }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.json")
    }

    pub fn java_dir(&self) -> PathBuf {
        self.data_dir.join("java")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// Агент playit.gg, его логи и кэш адресов туннелей.
    pub fn playit_dir(&self) -> PathBuf {
        self.data_dir.join("playit")
    }

    pub fn default_servers_dir(&self) -> PathBuf {
        self.data_dir.join("servers")
    }
}
