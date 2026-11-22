//! RCON: служебный канал к серверу Minecraft.
//!
//! Через него Hearth спрашивает список игроков и TPS и выполняет команды из
//! вкладки «Игроки», не засоряя консоль пользователя. Пароль случайный и
//! меняется при каждом запуске, порт тоже случайный; на роутере он не открыт.
//!
//! Протокол (Source RCON): пакет = длина (i32 LE) + id (i32) + тип (i32) +
//! текст + два нулевых байта. Тип 3 — вход, 2 — команда, 0 — ответ.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{Error, Result};

const LOGIN: i32 = 3;
const COMMAND: i32 = 2;
const TIMEOUT: Duration = Duration::from_secs(5);

pub struct Rcon {
    stream: TcpStream,
    next_id: i32,
}

fn rcon_error(message: impl Into<String>) -> Error {
    Error::other(format!("Служебный канал сервера (RCON): {}", message.into()))
}

impl Rcon {
    pub async fn connect(port: u16, password: &str) -> Result<Self> {
        let stream = tokio::time::timeout(TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
            .await
            .map_err(|_| rcon_error("сервер не ответил"))??;
        stream.set_nodelay(true)?;
        let mut rcon = Self { stream, next_id: 1 };
        let id = rcon.send(LOGIN, password).await?;
        let (reply_id, _) = rcon.read_packet().await?;
        if reply_id == -1 || reply_id != id {
            return Err(rcon_error("неверный пароль"));
        }
        Ok(rcon)
    }

    async fn send(&mut self, kind: i32, body: &str) -> Result<i32> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let mut packet = Vec::with_capacity(14 + body.len());
        packet.extend_from_slice(&((10 + body.len()) as i32).to_le_bytes());
        packet.extend_from_slice(&id.to_le_bytes());
        packet.extend_from_slice(&kind.to_le_bytes());
        packet.extend_from_slice(body.as_bytes());
        packet.extend_from_slice(&[0, 0]);
        tokio::time::timeout(TIMEOUT, self.stream.write_all(&packet))
            .await
            .map_err(|_| rcon_error("сервер не принял команду"))??;
        Ok(id)
    }

    async fn read_packet(&mut self) -> Result<(i32, String)> {
        let read = async {
            let len = self.stream.read_i32_le().await?;
            if !(10..=1_048_576).contains(&len) {
                return Err(rcon_error("неожиданный ответ"));
            }
            let mut buf = vec![0u8; len as usize];
            self.stream.read_exact(&mut buf).await?;
            let id = i32::from_le_bytes(buf[0..4].try_into().expect("4 bytes"));
            let body = String::from_utf8_lossy(&buf[8..buf.len() - 2]).into_owned();
            Ok((id, body))
        };
        tokio::time::timeout(TIMEOUT, read)
            .await
            .map_err(|_| rcon_error("сервер не ответил вовремя"))?
    }

    /// Выполняет команду (без слэша) и возвращает ответ сервера без цветовых кодов.
    pub async fn command(&mut self, command: &str) -> Result<String> {
        let id = self.send(COMMAND, command).await?;
        loop {
            let (reply_id, body) = self.read_packet().await?;
            if reply_id == id {
                return Ok(strip_colors(&body));
            }
        }
    }
}

/// Убирает коды цвета Minecraft (`§a`, `§l` и т.п.).
pub fn strip_colors(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// Случайный пароль RCON (меняется при каждом запуске сервера).
pub fn random_password() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// Свободный порт для RCON на этом компьютере.
pub fn free_port() -> Option<u16> {
    std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?.local_addr().ok().map(|a| a.port())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn strips_color_codes() {
        assert_eq!(strip_colors("§6TPS: §a20.0, §e18.5"), "TPS: 20.0, 18.5");
    }

    /// Мини-сервер RCON: проверяет пароль и отвечает эхом.
    #[tokio::test]
    async fn talks_to_fake_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            loop {
                let Ok(len) = s.read_i32_le().await else { break };
                let mut buf = vec![0u8; len as usize];
                s.read_exact(&mut buf).await.unwrap();
                let id = i32::from_le_bytes(buf[0..4].try_into().unwrap());
                let kind = i32::from_le_bytes(buf[4..8].try_into().unwrap());
                let body = String::from_utf8_lossy(&buf[8..buf.len() - 2]).into_owned();
                let (reply_id, reply) = match kind {
                    3 if body == "secret" => (id, String::new()),
                    3 => (-1, String::new()),
                    _ => (id, format!("§aэхо: {body}")),
                };
                let mut p = Vec::new();
                p.extend_from_slice(&((10 + reply.len()) as i32).to_le_bytes());
                p.extend_from_slice(&reply_id.to_le_bytes());
                p.extend_from_slice(&0i32.to_le_bytes());
                p.extend_from_slice(reply.as_bytes());
                p.extend_from_slice(&[0, 0]);
                s.write_all(&p).await.unwrap();
            }
        });
        let mut rcon = Rcon::connect(port, "secret").await.unwrap();
        assert_eq!(rcon.command("list").await.unwrap(), "эхо: list");
        assert_eq!(rcon.command("tps").await.unwrap(), "эхо: tps");
    }
}
