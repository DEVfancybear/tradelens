//! The language the trader reads TradeLens in, for the text the crates make.
//!
//! The crates' text (errors, statuses, the monthly report) is read with
//! [rust-i18n](https://github.com/longbridge/rust-i18n) from each crate's `locales/` files, in the
//! format NSH reads the app's own with (`_version: 2`, every language side by side), through the
//! crate's `tr!` macro, which gives it in [`current`]. The app sets that language ([`set_current`])
//! whenever the trader picks one, so text made on any thread (a database error on a reading thread,
//! a terminal's status) follows it. Until then it is Vietnamese.

use std::sync::atomic::{AtomicU8, Ordering};

/// A language of the app's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    /// Vietnamese, the default.
    #[default]
    Vi,
    /// English.
    En,
}

impl Lang {
    /// Every language, in the order the settings list them.
    pub const ALL: [Self; 2] = [Self::Vi, Self::En];

    /// The language's tag: how it is stored (the `language` setting) and the locale of its text.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Vi => "vi",
            Self::En => "en",
        }
    }

    /// The language of `tag` (`en`, `en-US`, `vi`); anything else is Vietnamese.
    #[must_use]
    pub fn parse(tag: &str) -> Self {
        let language = tag.trim().split(['-', '_']).next().unwrap_or_default();
        if language.eq_ignore_ascii_case("en") { Self::En } else { Self::Vi }
    }

    /// The language's name in that language, as a list of languages shows it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Vi => "Tiếng Việt",
            Self::En => "English",
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Vi => 0,
            Self::En => 1,
        }
    }
}

/// The language of the process's text, as [`Lang::code`] gives it.
static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The language the crates' text is given in now.
#[must_use]
pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::En.code() { Lang::En } else { Lang::Vi }
}

/// Makes `lang` the language of every text the crates give from now on, on every thread.
pub fn set_current(lang: Lang) {
    CURRENT.store(lang.code(), Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_language_reads_back_and_anything_else_is_vietnamese() {
        for lang in Lang::ALL {
            assert_eq!(Lang::parse(lang.key()), lang);
        }
        assert_eq!(Lang::parse("en-US"), Lang::En);
        assert_eq!(Lang::parse(""), Lang::Vi);
        assert_eq!(Lang::parse("fr"), Lang::Vi);
        assert_eq!(Lang::default(), Lang::Vi);
    }

    #[test]
    fn the_crates_text_is_vietnamese_until_the_app_chooses() {
        assert_eq!(current(), Lang::Vi, "no test of this crate changes the language of the process");
    }
}
