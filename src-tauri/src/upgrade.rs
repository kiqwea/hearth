//! Обновление сервера: новая сборка ядра той же версии игры или переход на
//! новую версию Minecraft. Здесь только типы для интерфейса; порядок действий
//! (бэкап → ядро → моды) — в `manager`.

use serde::Serialize;
use ts_rs::TS;

use crate::backups::BackupInfo;
use crate::mods::ClientSide;
use crate::servers::ServerMeta;

/// Что можно обновить.
#[derive(Serialize, Clone, Debug, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInfo {
    pub current_version: String,
    /// Сборка Paper/Purpur или версия загрузчика Fabric.
    pub current_build: Option<String>,
    /// Новая сборка той же версии игры, если вышла.
    pub new_build: Option<String>,
    /// Стабильные версии игры новее текущей, новые первыми.
    pub newer_versions: Vec<String>,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ModCompatStatus {
    /// Есть версия под новую версию игры.
    Ready,
    /// Автор ещё не обновил.
    Missing,
    /// Файл не с Modrinth: проверить нельзя.
    Unknown,
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModCompat {
    pub title: String,
    pub file: String,
    pub status: ModCompatStatus,
    pub new_version: Option<String>,
    pub client_side: ClientSide,
}

/// Что будет при переходе на версию `target`.
#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdatePlan {
    pub target: String,
    pub mods: Vec<ModCompat>,
    /// Моды без новой версии будут убраны в папку `mods-disabled` (Fabric не
    /// запустится с несовместимым модом). Плагины Paper остаются: часто они
    /// работают и на новой версии.
    pub disable_missing: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateResult {
    pub meta: ServerMeta,
    /// Копия мира до обновления.
    pub backup: Option<BackupInfo>,
    /// Обновлённые моды и плагины (имена файлов).
    pub updated: Vec<String>,
    /// Убраны в `mods-disabled`.
    pub disabled: Vec<String>,
    /// Пояснения: что не удалось и что проверить.
    pub notes: Vec<String>,
}
