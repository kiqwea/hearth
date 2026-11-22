//! События, которые ядро отправляет наружу (во фронтенд или в тесты).
//!
//! Ядро не знает про Tauri: оно получает `EventSink`, а приложение подставляет
//! реализацию, которая шлёт событие `hearth://event` в окно.

use serde::Serialize;
use ts_rs::TS;

use crate::process::{ConsoleLine, ServerStatus};
use crate::tunnel::PlayitStatus;

pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

/// Ничего не делает. Для тестов и для кода, которому события не нужны.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: Event) {}
}

#[derive(Serialize, Clone, Debug, TS)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
#[ts(export)]
pub enum Event {
    Console(ConsoleEvent),
    Status(StatusEvent),
    Install(InstallEvent),
    /// Изменилось состояние playit: подключение аккаунта, агент, адреса туннелей.
    Playit(PlayitStatus),
}

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConsoleEvent {
    pub server_id: String,
    pub line: ConsoleLine,
}

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatusEvent {
    pub server_id: String,
    pub status: ServerStatus,
}

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallEvent {
    pub server_id: String,
    pub progress: InstallProgress,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum InstallStage {
    Resolving,
    Java,
    Server,
    Done,
    Failed,
}

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallProgress {
    pub stage: InstallStage,
    pub message: String,
    #[ts(type = "number | null")]
    pub downloaded: Option<u64>,
    #[ts(type = "number | null")]
    pub total: Option<u64>,
}

impl InstallProgress {
    pub fn step(stage: InstallStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            downloaded: None,
            total: None,
        }
    }
}
