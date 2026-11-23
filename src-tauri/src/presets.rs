//! Пресет «Производительность» и предгенерация мира через Chunky.
//!
//! Пресет не меняет игру: только моды-оптимизации (Fabric), настройки
//! server.properties и, у Paper/Purpur, известные безопасные ключи
//! `config/paper-world-defaults.yml`. Что именно поменялось, возвращается
//! списком, чтобы интерфейс показал это пользователю.

use serde::Serialize;
use ts_rs::TS;

use crate::providers::Loader;
use crate::servers::Properties;

/// Моды пресета для Fabric: id или slug на Modrinth и название.
/// C2ME не берём: под новые версии он выходит альфой.
pub const FABRIC_MODS: &[(&str, &str)] = &[
    ("gvQqBUqZ", "Lithium"),
    ("uXXizFIs", "FerriteCore"),
    ("nmDcB62a", "ModernFix"),
    ("fQEb0iXm", "Krypton"),
    ("l6YH9Als", "Spark"),
];

/// Chunky на Modrinth (есть и для Fabric, и для Paper).
pub const CHUNKY: &str = "fALzjamp";

/// Ключи `paper-world-defaults.yml`: путь, значение, что это даёт.
/// Только то, что не меняет привычную игру.
pub const PAPER_WORLD: &[(&[&str], &str, &str)] = &[
    (&["environment", "optimize-explosions"], "true", "Быстрые взрывы"),
    (&["collisions", "max-entity-collisions"], "2", "Меньше расчётов столкновений мобов в толпе"),
    (&["tick-rates", "mob-spawner"], "2", "Спаунеры проверяются через тик"),
    (&["tick-rates", "grass-spread"], "4", "Трава разрастается реже"),
    (&["hopper", "ignore-occluding-blocks"], "true", "Воронки не ищут предметы в сплошных блоках"),
    (&["chunks", "prevent-moving-into-unloaded-chunks"], "true", "Нельзя улететь в незагруженные чанки"),
    (&["chunks", "entity-per-chunk-save-limit", "arrow"], "16", "Не больше 16 стрел на чанк"),
    (&["chunks", "entity-per-chunk-save-limit", "experience_orb"], "16", "Не больше 16 сфер опыта на чанк"),
    (&["chunks", "entity-per-chunk-save-limit", "snowball"], "8", "Не больше 8 снежков на чанк"),
];

/// Итог применения пресета.
#[derive(Serialize, Clone, Debug, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PresetResult {
    /// Поставленные файлы модов.
    pub installed: Vec<String>,
    /// Что уже стояло.
    pub already: Vec<String>,
    /// Что не поставлено и почему: «ModernFix: нет версии под 26.3».
    pub skipped: Vec<String>,
    /// Изменённые настройки, по-человечески.
    pub settings: Vec<String>,
    /// Настройки Paper появятся после первого запуска: пресет стоит применить ещё раз.
    pub paper_config_missing: bool,
}

/// Дальность симуляции (тики мобов, растений) не больше 8 чанков: видимость не меняется.
const MAX_SIMULATION: u32 = 8;

/// Правки server.properties. Возвращает описания изменений.
pub fn tune_properties(loader: Loader, props: &mut Properties) -> Vec<String> {
    let mut changes = Vec::new();
    let simulation = props.get("simulation-distance").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(10);
    if simulation > MAX_SIMULATION {
        props.set("simulation-distance", &MAX_SIMULATION.to_string());
        changes.push(format!("Дальность симуляции: {simulation} → {MAX_SIMULATION} чанков (дальность прорисовки не меняется)"));
    }
    // У Paper запись и так асинхронная.
    if !loader.is_paper_like() && props.get("sync-chunk-writes").is_none_or(|v| v.trim() != "false") {
        props.set("sync-chunk-writes", "false");
        changes.push("Чанки записываются на диск без ожидания".into());
    }
    changes
}

/// Меняет значение существующего ключа YAML по пути (`["chunks", "max"]`).
/// Новые ключи не добавляет: Paper сам пишет полный файл с настройками по
/// умолчанию, а неизвестную структуру лучше не трогать. `None`, если ключа нет;
/// `Some(false)`, если значение уже такое.
pub fn set_yaml_scalar(text: &mut String, path: &[&str], value: &str) -> Option<bool> {
    let mut depth = 0; // сколько элементов пути уже найдено
    let mut parent: Option<usize> = None; // отступ найденного родителя
    let mut level: Option<usize> = Some(0); // отступ его прямых детей (узнаём по первому)
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let content = line.trim_end_matches(['\n', '\r']);
        let trimmed = content.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = content.len() - trimmed.len();
        // Вышли из найденного родителя: дальше искать нечего.
        if parent.is_some_and(|p| indent <= p) {
            return None;
        }
        let expected = *level.get_or_insert(indent);
        // Только прямые дети родителя, не внуки.
        if indent != expected {
            continue;
        }
        let Some((key, rest)) = trimmed.split_once(':') else { continue };
        let key = key.trim().trim_matches(['"', '\'']);
        if key != path[depth] {
            continue;
        }
        if depth + 1 < path.len() {
            parent = Some(indent);
            level = None;
            depth += 1;
            continue;
        }
        let current = rest.split('#').next().unwrap_or("").trim();
        if current.is_empty() {
            return None; // это не скаляр, а раздел
        }
        if current.trim_matches(['"', '\'']) == value {
            return Some(false);
        }
        let value_start = start + indent + trimmed.find(':').expect("есть двоеточие") + 1;
        let line_end = start + content.len();
        let comment = rest.find(" #").map(|i| &rest[i..]).unwrap_or("");
        let replaced = format!(" {value}{comment}");
        text.replace_range(value_start..line_end, &replaced);
        return Some(true);
    }
    None
}

/// Состояние предгенерации из ответа `chunky progress`.
#[derive(Serialize, Clone, Debug, Default, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PregenStatus {
    pub running: bool,
    pub percent: Option<f32>,
    /// Сколько осталось: «0:12:34».
    pub eta: Option<String>,
    /// Ответ Chunky как есть (для подсказки).
    pub message: String,
}

/// Разбирает ответ Chunky: `Task running for minecraft:overworld. Processed: 1234
/// chunks (12.34%), ETA: 0:01:23, Rate: 100.0 cps, Current: 4, -2`.
pub fn parse_progress(reply: &str) -> PregenStatus {
    let text = reply.trim();
    let percent = text.find('%').and_then(|end| {
        let start = text[..end].rfind(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).map_or(0, |i| i + 1);
        text[start..end].replace(',', ".").parse::<f32>().ok()
    });
    let eta = text.find("ETA:").map(|i| {
        text[i + 4..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == ':')
            .collect::<String>()
    });
    let lower = text.to_lowercase();
    let running = lower.contains("task running")
        || (percent.is_some_and(|p| p < 100.0) && !lower.contains("finished") && !lower.contains("no task"));
    PregenStatus { running, percent, eta: eta.filter(|e| !e.is_empty()), message: text.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_preset() {
        let mut props = Properties::parse_bytes(b"simulation-distance=10\nview-distance=12\n");
        let changes = tune_properties(Loader::Fabric, &mut props);
        assert_eq!(changes.len(), 2);
        assert_eq!(props.get("simulation-distance"), Some("8"));
        assert_eq!(props.get("view-distance"), Some("12"), "прорисовку не трогаем");
        assert_eq!(props.get("sync-chunk-writes"), Some("false"));
        assert!(tune_properties(Loader::Fabric, &mut props).is_empty(), "повторно ничего не меняет");

        let mut paper = Properties::parse_bytes(b"simulation-distance=6\n");
        assert!(tune_properties(Loader::Paper, &mut paper).is_empty(), "меньшую дальность не увеличиваем");
        assert_eq!(paper.get("sync-chunk-writes"), None);
    }

    const PAPER: &str = "\
_version: 31
chunks:
  auto-save-interval: default
  entity-per-chunk-save-limit:
    arrow: -1
    experience_orb: -1
  prevent-moving-into-unloaded-chunks: false
collisions:
  max-entity-collisions: 8 # default
environment:
  optimize-explosions: false
tick-rates:
  grass-spread: 1
  mob-spawner: 1
";

    #[test]
    fn yaml_scalar_setter() {
        let mut text = PAPER.to_string();
        assert_eq!(set_yaml_scalar(&mut text, &["environment", "optimize-explosions"], "true"), Some(true));
        assert_eq!(set_yaml_scalar(&mut text, &["environment", "optimize-explosions"], "true"), Some(false));
        assert_eq!(set_yaml_scalar(&mut text, &["chunks", "entity-per-chunk-save-limit", "arrow"], "16"), Some(true));
        assert_eq!(set_yaml_scalar(&mut text, &["collisions", "max-entity-collisions"], "2"), Some(true));
        assert_eq!(set_yaml_scalar(&mut text, &["tick-rates", "mob-spawner"], "2"), Some(true));
        // Разделы и несуществующие ключи не трогаем.
        assert_eq!(set_yaml_scalar(&mut text, &["chunks", "entity-per-chunk-save-limit"], "1"), None);
        assert_eq!(set_yaml_scalar(&mut text, &["chunks", "snowball"], "1"), None);
        assert_eq!(set_yaml_scalar(&mut text, &["environment", "missing"], "1"), None);
        // `auto-save-interval` внутри chunks, но ключ `arrow` на другом уровне — путь важен.
        assert_eq!(set_yaml_scalar(&mut text, &["arrow"], "1"), None);
        assert!(text.contains("  optimize-explosions: true\n"));
        assert!(text.contains("    arrow: 16\n"));
        assert!(text.contains("    experience_orb: -1\n"));
        assert!(text.contains("  max-entity-collisions: 2 # default\n"));
        assert!(text.contains("  grass-spread: 1\n  mob-spawner: 2\n"));
        assert!(text.starts_with("_version: 31\n"));
    }

    #[test]
    fn chunky_progress() {
        let s = parse_progress("[Chunky] Task running for minecraft:overworld. Processed: 1234 chunks (12.34%), ETA: 0:01:23, Rate: 100.0 cps, Current: 4, -2");
        assert!(s.running);
        assert_eq!(s.percent, Some(12.34));
        assert_eq!(s.eta.as_deref(), Some("0:01:23"));
        let done = parse_progress("Task finished for minecraft:overworld. Processed: 5000 chunks (100.00%), Total time: 0:02:00");
        assert!(!done.running);
        assert_eq!(done.percent, Some(100.0));
        let none = parse_progress("No tasks running.");
        assert!(!none.running && none.percent.is_none());
    }
}
