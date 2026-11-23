//! Язык сообщений ядра: русский или английский.
//!
//! Тексты для пользователя пишутся парой прямо на месте: `tr!("русский",
//! "English")`. Вычисляется только выбранная ветка, поэтому внутри можно
//! писать `format!`. Язык меняется на ходу из настроек Hearth.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_language(language: &str) {
    ENGLISH.store(language == "en", Ordering::Relaxed);
}

pub fn is_english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Язык по умолчанию для новой установки: русский для России и соседей,
/// где русский понимают почти все, иначе английский.
pub fn system_default() -> &'static str {
    let locale = sys_locale::get_locale().unwrap_or_default().to_lowercase();
    if ["ru", "uk", "be", "kk", "ky", "uz", "hy", "az", "tg"].iter().any(|l| locale.starts_with(l)) {
        "ru"
    } else {
        "en"
    }
}

/// `tr!("русский", "English")`: текст на языке интерфейса.
#[macro_export]
macro_rules! tr {
    ($ru:expr, $en:expr $(,)?) => {
        if $crate::lang::is_english() { $en } else { $ru }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn switches_language() {
        super::set_language("en");
        assert_eq!(tr!("Привет", "Hello"), "Hello");
        let n = 3;
        assert_eq!(tr!(format!("{n} мода"), format!("{n} mods")), "3 mods");
        super::set_language("ru");
        assert_eq!(tr!("Привет", "Hello"), "Привет");
    }
}
