//! Hearth: локальные серверы Minecraft в пару кликов.
//!
//! Ядро (всё, кроме `app`) не зависит от Tauri и тестируется отдельно.

pub mod backups;
pub mod config;
pub mod direct;
pub mod error;
pub mod events;
pub mod friendpack;
pub mod fsx;
pub mod java;
pub mod manager;
pub mod mods;
pub mod modrinth;
pub mod net;
pub mod paths;
pub mod pirate;
pub mod presets;
pub mod players;
pub mod process;
pub mod providers;
pub mod rcon;
pub mod servers;
pub mod system;
pub mod tunnel;
pub mod upgrade;

mod app;

pub use app::run;
