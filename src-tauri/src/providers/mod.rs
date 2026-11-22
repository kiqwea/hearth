//! Провайдеры ядер: откуда брать список версий и серверный jar.
//!
//! Внешние API меняются (Paper уже отключал v2), поэтому каждый источник
//! живёт в своём модуле, а остальной код видит только `list_versions` и
//! `resolve`. Чистая логика выбора (какую сборку брать) вынесена в отдельные
//! функции и покрыта тестами на сохранённых ответах.

pub mod fabric;
pub mod paper;
pub mod vanilla;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::Result;
use crate::net::{Checksum, Http};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Loader {
    Vanilla,
    Paper,
    Fabric,
}

impl Loader {
    pub fn display_name(self) -> &'static str {
        match self {
            Loader::Vanilla => "Vanilla",
            Loader::Paper => "Paper",
            Loader::Fabric => "Fabric",
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GameVersion {
    pub id: String,
    /// `false` для снапшотов, пре-релизов и RC.
    pub stable: bool,
    pub release_time: Option<String>,
}

/// Один файл, который нужно скачать в папку сервера.
#[derive(Clone, Debug)]
pub struct DownloadSpec {
    pub url: String,
    pub file_name: String,
    pub checksum: Option<Checksum>,
}

/// Всё, что нужно для установки сервера выбранной версии.
#[derive(Clone, Debug)]
pub struct ServerArtifact {
    /// Jar, который запускаем через `java -jar`.
    pub jar: DownloadSpec,
    /// Дополнительные файлы (для Fabric заранее кладём ванильный server.jar,
    /// чтобы лаунчер не качал его сам, без прогресса и проверки хэша).
    pub extra: Vec<DownloadSpec>,
    pub java_major: u32,
    pub jvm_flags: Vec<String>,
    /// Сборка Paper или версия загрузчика Fabric.
    pub loader_version: Option<String>,
}

pub async fn list_versions(
    http: &Http,
    loader: Loader,
    include_unstable: bool,
) -> Result<Vec<GameVersion>> {
    match loader {
        Loader::Vanilla => vanilla::list_versions(http, include_unstable).await,
        Loader::Paper => paper::list_versions(http, include_unstable).await,
        Loader::Fabric => fabric::list_versions(http, include_unstable).await,
    }
}

pub async fn resolve(http: &Http, loader: Loader, game_version: &str) -> Result<ServerArtifact> {
    match loader {
        Loader::Vanilla => vanilla::resolve(http, game_version).await,
        Loader::Paper => paper::resolve(http, game_version).await,
        Loader::Fabric => fabric::resolve(http, game_version).await,
    }
}
