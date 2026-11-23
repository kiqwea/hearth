//! Провайдеры ядер: откуда брать список версий и серверный jar.
//!
//! Внешние API меняются (Paper уже отключал v2), поэтому каждый источник
//! живёт в своём модуле, а остальной код видит только `list_versions` и
//! `resolve`. Чистая логика выбора (какую сборку брать) вынесена в отдельные
//! функции и покрыта тестами на сохранённых ответах.

pub mod fabric;
pub mod paper;
pub mod purpur;
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
    /// Форк Paper с дополнительными настройками; плагины те же.
    Purpur,
    Fabric,
}

impl Loader {
    pub fn display_name(self) -> &'static str {
        match self {
            Loader::Vanilla => "Vanilla",
            Loader::Paper => "Paper",
            Loader::Purpur => "Purpur",
            Loader::Fabric => "Fabric",
        }
    }

    /// Paper и его форки: плагины Bukkit в папке `plugins`, команды `minecraft:`, `tps`.
    pub fn is_paper_like(self) -> bool {
        matches!(self, Loader::Paper | Loader::Purpur)
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
        Loader::Purpur => purpur::list_versions(http, include_unstable).await,
        Loader::Fabric => fabric::list_versions(http, include_unstable).await,
    }
}

/// Стабильные версии новее `current`, новые первыми. Списки провайдеров уже
/// отсортированы от новых к старым. Если текущей версии в списке нет
/// (например, снапшот), обновляться не на что.
pub fn newer_stable(list: &[GameVersion], current: &str) -> Vec<String> {
    let Some(pos) = list.iter().position(|v| v.id == current) else {
        return Vec::new();
    };
    list[..pos].iter().filter(|v| v.stable).map(|v| v.id.clone()).collect()
}

pub async fn resolve(http: &Http, loader: Loader, game_version: &str) -> Result<ServerArtifact> {
    match loader {
        Loader::Vanilla => vanilla::resolve(http, game_version).await,
        Loader::Paper => paper::resolve(http, game_version).await,
        Loader::Purpur => purpur::resolve(http, game_version).await,
        Loader::Fabric => fabric::resolve(http, game_version).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: &str, stable: bool) -> GameVersion {
        GameVersion { id: id.into(), stable, release_time: None }
    }

    #[test]
    fn newer_stable_versions() {
        let list = [v("26.4", true), v("26.4-rc-1", false), v("26.3.1", true), v("26.3", true), v("26.2", true)];
        assert_eq!(newer_stable(&list, "26.3"), ["26.4", "26.3.1"]);
        assert!(newer_stable(&list, "26.4").is_empty());
        assert!(newer_stable(&list, "26.5-snapshot").is_empty());
    }
}
