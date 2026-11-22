//! Сведения о компьютере для мастера создания сервера.

use serde::Serialize;
use ts_rs::TS;

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SystemInfo {
    pub total_memory_mb: u32,
    /// Сколько памяти предложить серверу по умолчанию с учётом того,
    /// что пользователь, скорее всего, играет на этом же ПК.
    pub recommended_memory_mb: u32,
    pub app_version: String,
}

pub fn system_info() -> SystemInfo {
    let total_memory_mb = match sysinfo::System::new() {
        Ok(mut sys) => {
            sys.refresh_memory();
            (sys.total_memory() / 1024 / 1024) as u32
        }
        Err(e) => {
            tracing::warn!("Не удалось узнать объём памяти: {e}");
            0
        }
    };
    SystemInfo {
        total_memory_mb,
        recommended_memory_mb: recommended_memory_mb(total_memory_mb),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NetworkInfo {
    /// IP этого компьютера в домашней сети (для друзей на том же Wi-Fi).
    pub lan_ip: Option<String>,
}

/// Локальный IP узнаём «подключив» UDP-сокет к внешнему адресу: для UDP это
/// только выбор маршрута, ни один пакет никуда не отправляется.
pub fn network_info() -> NetworkInfo {
    let lan_ip = std::net::UdpSocket::bind(("0.0.0.0", 0))
        .and_then(|socket| {
            socket.connect(("8.8.8.8", 80))?;
            socket.local_addr()
        })
        .ok()
        .map(|addr| addr.ip())
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .map(|ip| ip.to_string());
    NetworkInfo { lan_ip }
}

pub fn recommended_memory_mb(total_mb: u32) -> u32 {
    match total_mb {
        0..=8192 => 2048,
        8193..=16384 => 4096,
        16385..=32768 => 6144,
        _ => 8192,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recommendation_leaves_room_for_the_game() {
        assert_eq!(recommended_memory_mb(8000), 2048);
        assert_eq!(recommended_memory_mb(16_000), 4096);
        assert_eq!(recommended_memory_mb(32_000), 6144);
        assert_eq!(recommended_memory_mb(65_000), 8192);
    }
}
