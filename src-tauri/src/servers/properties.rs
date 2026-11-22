//! Чтение и запись `server.properties` с сохранением порядка строк и комментариев.
//!
//! Формат: Java `.properties`. Не-ASCII символы при записи кодируем как
//! `\uXXXX`: так файл одинаково читают и старые версии Minecraft (Latin-1),
//! и новые (UTF-8).

use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
enum Line {
    /// Комментарий, пустая строка или то, что не удалось разобрать: пишем как было.
    Raw(String),
    Entry { key: String, value: String },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Properties {
    lines: Vec<Line>,
}

impl Properties {
    pub fn parse(text: &str) -> Self {
        let mut lines = Vec::new();
        let mut physical = text.lines().peekable();
        while let Some(line) = physical.next() {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
                lines.push(Line::Raw(line.to_string()));
                continue;
            }
            // Продолжение строки: нечётное число обратных слэшей в конце.
            let mut logical = trimmed.to_string();
            while ends_with_continuation(&logical) {
                logical.pop();
                match physical.next() {
                    Some(next) => logical.push_str(next.trim_start()),
                    None => break,
                }
            }
            let (key, value) = split_entry(&logical);
            lines.push(Line::Entry {
                key: unescape(&key),
                value: unescape(&value),
            });
        }
        Self { lines }
    }

    /// Читает байты файла: UTF-8, а если файл в другой кодировке, как Latin-1
    /// (так его читала бы Java).
    pub fn parse_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self::parse(text.strip_prefix('\u{feff}').unwrap_or(text)),
            Err(_) => Self::parse(&bytes.iter().map(|&b| b as char).collect::<String>()),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|l| match l {
            Line::Entry { key: k, value } if k == key => Some(value.as_str()),
            _ => None,
        })
    }

    pub fn set(&mut self, key: &str, value: &str) {
        for line in self.lines.iter_mut().rev() {
            if let Line::Entry { key: k, value: v } = line {
                if k == key {
                    *v = value.to_string();
                    return;
                }
            }
        }
        self.lines.push(Line::Entry {
            key: key.to_string(),
            value: value.to_string(),
        });
    }

    /// Все пары ключ-значение в порядке файла (при повторах побеждает последнее).
    pub fn entries(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for line in &self.lines {
            if let Line::Entry { key, value } = line {
                if let Some(existing) = out.iter_mut().find(|(k, _)| k == key) {
                    existing.1 = value.clone();
                } else {
                    out.push((key.clone(), value.clone()));
                }
            }
        }
        out
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Raw(raw) => out.push_str(raw),
                Line::Entry { key, value } => {
                    out.push_str(&escape(key, true));
                    out.push('=');
                    out.push_str(&escape(value, false));
                }
            }
            out.push('\n');
        }
        out
    }
}

fn ends_with_continuation(s: &str) -> bool {
    s.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1
}

/// Ключ заканчивается на первом неэкранированном `=`, `:` или пробеле.
fn split_entry(line: &str) -> (String, String) {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut key = String::new();
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            key.push(c);
            key.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == '=' || c == ':' || c.is_whitespace() {
            break;
        }
        key.push(c);
        i += 1;
    }
    // Пропускаем пробелы, один разделитель и снова пробелы.
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i < chars.len() && (chars[i] == '=' || chars[i] == ':') {
        i += 1;
    }
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    (key, chars[i..].iter().collect())
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut pending_high: Option<u16> = None;
    while let Some(c) = chars.next() {
        if c != '\\' {
            flush_high(&mut out, &mut pending_high);
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u16::from_str_radix(&hex, 16) {
                    Ok(unit) if (0xD800..0xDC00).contains(&unit) => {
                        flush_high(&mut out, &mut pending_high);
                        pending_high = Some(unit);
                    }
                    Ok(unit) if (0xDC00..0xE000).contains(&unit) => {
                        if let Some(high) = pending_high.take() {
                            let decoded = char::decode_utf16([high, unit])
                                .next()
                                .and_then(|r| r.ok())
                                .unwrap_or('\u{fffd}');
                            out.push(decoded);
                        } else {
                            out.push('\u{fffd}');
                        }
                    }
                    Ok(unit) => {
                        flush_high(&mut out, &mut pending_high);
                        out.push(char::from_u32(unit as u32).unwrap_or('\u{fffd}'));
                    }
                    Err(_) => {
                        flush_high(&mut out, &mut pending_high);
                        out.push('u');
                        out.push_str(&hex);
                    }
                }
            }
            Some(other) => {
                flush_high(&mut out, &mut pending_high);
                out.push(match other {
                    't' => '\t',
                    'n' => '\n',
                    'r' => '\r',
                    'f' => '\u{c}',
                    c => c,
                });
            }
            None => {}
        }
    }
    flush_high(&mut out, &mut pending_high);
    out
}

fn flush_high(out: &mut String, pending: &mut Option<u16>) {
    if pending.take().is_some() {
        out.push('\u{fffd}');
    }
}

fn escape(s: &str, is_key: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            ' ' if is_key || i == 0 => out.push_str("\\ "),
            '=' | ':' | '#' | '!' if is_key => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    let _ = write!(out, "\\u{:04X}", unit);
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "#Minecraft server properties\n#Sat Nov 22 10:00:00 MSK 2026\nmotd=A Minecraft Server\nserver-port=25565\nlevel-seed=\nonline-mode=true\n";

    #[test]
    fn roundtrip_keeps_comments_and_order() {
        let p = Properties::parse(SAMPLE);
        assert_eq!(p.to_text(), SAMPLE);
        assert_eq!(p.get("server-port"), Some("25565"));
        assert_eq!(p.get("level-seed"), Some(""));
    }

    #[test]
    fn set_replaces_in_place_or_appends() {
        let mut p = Properties::parse(SAMPLE);
        p.set("server-port", "25570");
        p.set("max-players", "8");
        let text = p.to_text();
        assert!(text.contains("server-port=25570\nlevel-seed"));
        assert!(text.ends_with("max-players=8\n"));
    }

    #[test]
    fn cyrillic_is_written_as_unicode_escapes_and_read_back() {
        let mut p = Properties::default();
        p.set("motd", "Сервер Димы");
        let text = p.to_text();
        assert!(text.is_ascii());
        assert!(text.contains("\\u0421"));
        assert_eq!(Properties::parse(&text).get("motd"), Some("Сервер Димы"));
    }

    #[test]
    fn emoji_uses_surrogate_pairs() {
        let mut p = Properties::default();
        p.set("motd", "🔥 огонь");
        let back = Properties::parse(&p.to_text());
        assert_eq!(back.get("motd"), Some("🔥 огонь"));
    }

    #[test]
    fn reads_utf8_written_by_new_minecraft() {
        let p = Properties::parse_bytes("motd=Привет\n".as_bytes());
        assert_eq!(p.get("motd"), Some("Привет"));
    }

    #[test]
    fn handles_colon_separator_escapes_and_continuation() {
        let p = Properties::parse("a\\:b : c\\=d\nlong=one \\\n    two\n");
        assert_eq!(p.get("a:b"), Some("c=d"));
        assert_eq!(p.get("long"), Some("one two"));
    }

    #[test]
    fn leading_space_in_value_is_preserved() {
        let mut p = Properties::default();
        p.set("motd", "  hi");
        assert_eq!(Properties::parse(&p.to_text()).get("motd"), Some("  hi"));
    }
}
