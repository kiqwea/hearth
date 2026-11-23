//! Игроки и статистика: разбор ответов сервера и файлов со списками.
//!
//! - кто онлайн: команда `minecraft:list` (одинакова на Vanilla, Paper, Fabric);
//! - TPS: `tps` на Paper, `tick query` на Vanilla/Fabric 1.20.3+;
//! - операторы, белый список, баны: `ops.json`, `whitelist.json`,
//!   `banned-players.json`; известные игроки — `usercache.json`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::{Error, Result};

#[derive(Serialize, Clone, Debug, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerStats {
    /// Сколько памяти сейчас занимает процесс сервера.
    pub memory_mb: Option<u32>,
    /// Загрузка процессора сервером, % от всего процессора.
    pub cpu_percent: Option<f32>,
    /// Тиков в секунду: 20 — норма, меньше — сервер не успевает.
    pub tps: Option<f32>,
    pub online: Vec<String>,
    pub max_players: Option<u32>,
    /// Сколько памяти миру реально нужно: куча Java после последней сборки
    /// мусора (из GC-лога). `memory_mb` для этого не годится: Java сразу
    /// занимает всю выделенную память (`-Xms = -Xmx`, `AlwaysPreTouch`).
    pub heap_used_mb: Option<u32>,
    /// Наибольшая занятость кучи перед сборкой мусора с прошлого замера.
    pub heap_peak_mb: Option<u32>,
}

#[derive(Serialize, Clone, Debug, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatsEvent {
    pub server_id: String,
    pub stats: ServerStats,
}

/// Точка графика: замер статистики раз в несколько секунд.
#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatsSample {
    /// Время замера, мс с 1970 года.
    #[ts(type = "number")]
    pub at: i64,
    pub heap_used_mb: Option<u32>,
    pub heap_peak_mb: Option<u32>,
    pub cpu_percent: Option<f32>,
    pub tps: Option<f32>,
    pub online: u32,
}

impl StatsSample {
    pub fn from_stats(stats: &ServerStats) -> Self {
        Self {
            at: chrono::Utc::now().timestamp_millis(),
            heap_used_mb: stats.heap_used_mb,
            heap_peak_mb: stats.heap_peak_mb,
            cpu_percent: stats.cpu_percent,
            tps: stats.tps,
            online: stats.online.len() as u32,
        }
    }
}

/// Файл GC-лога в папке сервера (пересоздаётся при каждом запуске).
pub const GC_LOG: &str = "hearth-gc.log";

/// Строка GC-лога Java: `... Pause Young (Normal) (G1 Evacuation Pause) 1024M->512M(6144M) 3.4ms`
/// → (занято до сборки, после, размер кучи) в МБ.
pub fn parse_gc_line(line: &str) -> Option<(u32, u32, u32)> {
    fn mb(num: &str, unit: char) -> Option<u32> {
        let n: f64 = num.parse().ok()?;
        Some(match unit {
            'K' => n / 1024.0,
            'M' => n,
            'G' => n * 1024.0,
            _ => return None,
        } as u32)
    }
    // Ищем «число+единица -> число+единица (число+единица)».
    let arrow = line.find("->")?;
    let left = &line[..arrow];
    let start = left.rfind(|c: char| !(c.is_ascii_digit() || c == '.' || "KMG".contains(c))).map_or(0, |i| i + 1);
    let before = &left[start..];
    let rest = &line[arrow + 2..];
    let open = rest.find('(')?;
    let close = rest[open..].find(')')? + open;
    let after = &rest[..open];
    let total = &rest[open + 1..close];
    let split = |s: &str| -> Option<(String, char)> {
        let unit = s.chars().last()?;
        Some((s[..s.len() - unit.len_utf8()].to_string(), unit))
    };
    let (b, bu) = split(before)?;
    let (a, au) = split(after)?;
    let (t, tu) = split(total)?;
    Some((mb(&b, bu)?, mb(&a, au)?, mb(&t, tu)?))
}

#[derive(Serialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerInfo {
    pub name: String,
    pub online: bool,
    pub op: bool,
    pub whitelisted: bool,
    pub banned: bool,
    pub ban_reason: Option<String>,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PlayerAction {
    Kick,
    Ban,
    Pardon,
    Op,
    Deop,
    WhitelistAdd,
    WhitelistRemove,
}

/// Ник игрока: 1–16 символов из латиницы, цифр и `_` (так требует Minecraft).
/// Заодно защищает от подстановки лишних команд.
pub fn validate_name(name: &str) -> Result<&str> {
    let name = name.trim();
    let ok = (1..=16).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(name)
    } else {
        Err(Error::invalid("Ник — это от 1 до 16 латинских букв, цифр или «_»"))
    }
}

/// Приставка к стандартным командам Minecraft. На Paper с ней вызываются
/// именно ванильные команды, а не одноимённые из Bukkit; Vanilla 26.x через
/// RCON такие команды не принимает («Unknown or incomplete command»).
pub fn command_prefix(is_paper: bool) -> &'static str {
    if is_paper { "minecraft:" } else { "" }
}

/// Команда серверу для действия над игроком.
pub fn action_command(action: PlayerAction, name: &str, reason: Option<&str>, prefix: &str) -> Result<String> {
    let name = validate_name(name)?;
    let reason = reason
        .map(|r| r.replace(['\n', '\r'], " ").trim().chars().take(100).collect::<String>())
        .filter(|r| !r.is_empty());
    let with_reason = |cmd: &str| match &reason {
        Some(r) => format!("{prefix}{cmd} {name} {r}"),
        None => format!("{prefix}{cmd} {name}"),
    };
    Ok(match action {
        PlayerAction::Kick => with_reason("kick"),
        PlayerAction::Ban => with_reason("ban"),
        PlayerAction::Pardon => format!("{prefix}pardon {name}"),
        PlayerAction::Op => format!("{prefix}op {name}"),
        PlayerAction::Deop => format!("{prefix}deop {name}"),
        PlayerAction::WhitelistAdd => format!("{prefix}whitelist add {name}"),
        PlayerAction::WhitelistRemove => format!("{prefix}whitelist remove {name}"),
    })
}

/// «There are 2 of a max of 10 players online: Steve, Alex».
pub fn parse_list(text: &str) -> Option<(Vec<String>, u32)> {
    let text = text.trim();
    let (head, names) = text.split_once(':').unwrap_or((text, ""));
    let numbers: Vec<u32> = head
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .collect();
    let [count, max] = numbers[..] else { return None };
    let online: Vec<String> = names
        .split([',', '\n'])
        .map(|n| n.trim())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect();
    // Если сервер прислал только число (имён нет), список имён будет пустым.
    if online.len() as u32 > count {
        return None;
    }
    Some((online, max))
}

/// Paper: «TPS from last 1m, 5m, 15m: 20.0, 20.0, 20.0» (бывает «*20.0»).
pub fn parse_paper_tps(text: &str) -> Option<f32> {
    let (_, values) = text.split_once("15m:")?;
    values.split(',').next()?.trim().trim_start_matches('*').parse().ok()
}

/// Vanilla/Fabric `tick query`: «Target tick rate: 20.0 per second.
/// Average time per tick: 3.4ms (Target: 50.0ms)».
pub fn parse_tick_query(text: &str) -> Option<f32> {
    let target: f32 = after(text, "Target tick rate:")
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .unwrap_or(20.0);
    let mspt: f32 = after(text, "Average time per tick:")?
        .split_whitespace()
        .next()?
        .trim_end_matches("ms")
        .parse()
        .ok()?;
    if mspt <= 0.0 {
        return Some(target);
    }
    Some((1000.0 / mspt).min(target))
}

fn after<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    text.find(marker).map(|i| &text[i + marker.len()..])
}

fn read_json_list(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn names_in(list: &[Value]) -> impl Iterator<Item = (&str, &Value)> {
    list.iter().filter_map(|v| Some((v.get("name")?.as_str()?, v)))
}

/// Все игроки, о которых знает сервер: кто заходил, операторы, белый список, баны.
pub fn read_players(game_dir: &Path, online: &[String]) -> Vec<PlayerInfo> {
    let mut players: BTreeMap<String, PlayerInfo> = BTreeMap::new();
    fn entry<'a>(players: &'a mut BTreeMap<String, PlayerInfo>, name: &str) -> &'a mut PlayerInfo {
        players.entry(name.to_lowercase()).or_insert_with(|| PlayerInfo {
            name: name.to_string(),
            online: false,
            op: false,
            whitelisted: false,
            banned: false,
            ban_reason: None,
        })
    }
    for (name, _) in names_in(&read_json_list(&game_dir.join("usercache.json"))) {
        entry(&mut players, name);
    }
    for (name, _) in names_in(&read_json_list(&game_dir.join("ops.json"))) {
        entry(&mut players, name).op = true;
    }
    for (name, _) in names_in(&read_json_list(&game_dir.join("whitelist.json"))) {
        entry(&mut players, name).whitelisted = true;
    }
    for (name, v) in names_in(&read_json_list(&game_dir.join("banned-players.json"))) {
        let p = entry(&mut players, name);
        p.banned = true;
        p.ban_reason = v
            .get("reason")
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty() && *r != "Banned by an operator.")
            .map(str::to_string);
    }
    for name in online {
        entry(&mut players, name).online = true;
    }
    let mut list: Vec<PlayerInfo> = players.into_values().collect();
    list.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_log_lines() {
        assert_eq!(
            parse_gc_line("[12.345s][info][gc] GC(5) Pause Young (Normal) (G1 Evacuation Pause) 1024M->512M(6144M) 3.456ms"),
            Some((1024, 512, 6144))
        );
        assert_eq!(parse_gc_line("[1.0s][info][gc] GC(0) Pause Young (Concurrent Start) 20480K->10240K(4G) 1.2ms"), Some((20, 10, 4096)));
        assert_eq!(parse_gc_line("[3.0s][info][gc] GC(2) Concurrent Mark Cycle 12.3ms"), None);
        assert_eq!(parse_gc_line("[0.01s][info][gc] Using G1"), None);
    }

    #[test]
    fn parses_list() {
        assert_eq!(
            parse_list("There are 2 of a max of 10 players online: Steve, Alex"),
            Some((vec!["Steve".into(), "Alex".into()], 10))
        );
        assert_eq!(parse_list("There are 0 of a max of 20 players online: "), Some((vec![], 20)));
        assert_eq!(parse_list("Unknown command"), None);
    }

    #[test]
    fn parses_tps() {
        assert_eq!(parse_paper_tps("TPS from last 1m, 5m, 15m: *20.0, 19.98, 19.5"), Some(20.0));
        assert_eq!(parse_paper_tps("TPS from last 1m, 5m, 15m: 17.3, 19.0, 19.9"), Some(17.3));
        let q = "The game is running normally\nTarget tick rate: 20.0 per second.\nAverage time per tick: 3.4ms (Target: 50.0ms)";
        assert_eq!(parse_tick_query(q), Some(20.0));
        let slow = "Target tick rate: 20.0 per second.\nAverage time per tick: 80.0ms (Target: 50.0ms)";
        assert_eq!(parse_tick_query(slow), Some(12.5));
        assert_eq!(parse_tick_query("Unknown or incomplete command"), None);
    }

    #[test]
    fn commands_are_safe() {
        assert_eq!(action_command(PlayerAction::Op, "Steve", None, "minecraft:").unwrap(), "minecraft:op Steve");
        assert_eq!(action_command(PlayerAction::WhitelistAdd, "Steve", None, "").unwrap(), "whitelist add Steve");
        assert_eq!(
            action_command(PlayerAction::Ban, "Steve", Some("гриф\nstop"), "").unwrap(),
            "ban Steve гриф stop"
        );
        assert!(action_command(PlayerAction::Kick, "Steve; stop", None, "").is_err());
        assert!(action_command(PlayerAction::Kick, "", None, "").is_err());
        assert!(action_command(PlayerAction::Kick, "a_very_long_nickname_123", None, "").is_err());
    }

    #[test]
    fn merges_player_lists() {
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path();
        std::fs::write(g.join("usercache.json"), r#"[{"name":"Steve","uuid":"x"},{"name":"Alex","uuid":"y"}]"#).unwrap();
        std::fs::write(g.join("ops.json"), r#"[{"name":"Steve","uuid":"x","level":4}]"#).unwrap();
        std::fs::write(g.join("whitelist.json"), r#"[{"name":"Notch","uuid":"z"}]"#).unwrap();
        std::fs::write(g.join("banned-players.json"), r#"[{"name":"Alex","uuid":"y","reason":"Гриф"}]"#).unwrap();
        let players = read_players(g, &["steve".into()]);
        let names: Vec<&str> = players.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Steve", "Alex", "Notch"], "онлайн первыми, потом по алфавиту");
        assert!(players[0].online && players[0].op);
        assert!(players[1].banned && players[1].ban_reason.as_deref() == Some("Гриф"));
        assert!(players[2].whitelisted && !players[2].online);
    }
}
