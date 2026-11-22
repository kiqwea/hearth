//! Hearth: локальные серверы Minecraft в пару кликов.
//!
//! Ядро (всё, кроме `app`) не зависит от Tauri и тестируется отдельно.

pub mod config;
pub mod direct;
pub mod error;
pub mod events;
pub mod fsx;
pub mod java;
pub mod manager;
pub mod net;
pub mod paths;
pub mod process;
pub mod providers;
pub mod servers;
pub mod system;
pub mod tunnel;

mod app;

pub use app::run;
