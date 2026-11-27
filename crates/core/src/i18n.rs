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

/// The check each crate runs over its own `locales/`: both languages are there for every key, the
/// arguments agree, and the keys the source reads are the keys the files hold.
#[cfg(any(test, feature = "test-util"))]
pub mod testing {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::Path,
    };

    use super::Lang;

    /// What `crate_dir`'s `locales/` get wrong, one line each; empty when they are sound.
    ///
    /// `translate(locale, key)` is the crate's own lookup. `prefixes` are the starts of the keys the
    /// crate's source writes as string literals (`"mt5."`).
    #[must_use]
    pub fn problems(crate_dir: &Path, translate: &dyn Fn(&str, &str) -> String, prefixes: &[&str]) -> Vec<String> {
        let mut problems = Vec::new();
        let mut keys: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for file in files(&crate_dir.join("locales"), "yml") {
            read_table(&file, &mut keys, &mut problems);
        }
        for (key, texts) in &keys {
            for lang in Lang::ALL {
                let tag = lang.key();
                let Some(text) = texts.get(tag) else {
                    problems.push(format!("{key}: no {tag} text"));
                    continue;
                };
                if text.trim().is_empty() {
                    problems.push(format!("{key}: the {tag} text is empty"));
                }
                if translate(tag, key) != *text {
                    problems.push(format!("{key}: the {tag} text is not the one the lookup gives"));
                }
            }
            let arguments = |tag: &str| texts.get(tag).map_or_default(|text| arguments_of(text));
            if arguments("vi") != arguments("en") {
                problems.push(format!("{key}: the two languages take different arguments"));
            }
        }
        let mut read = BTreeSet::new();
        for file in files(&crate_dir.join("src"), "rs") {
            let Ok(source) = fs::read_to_string(&file) else { continue };
            // What follows the first test module is not the program's.
            let program = source.split("#[cfg(test)]").next().unwrap_or_default();
            read.extend(literal_keys(program, prefixes));
        }
        problems.extend(
            read.iter()
                .filter(|key| !keys.contains_key(*key))
                .map(|key| format!("{key}: read by the source, not in the files")),
        );
        problems.extend(
            keys.keys().filter(|key| !read.contains(*key)).map(|key| format!("{key}: in the files, read by nothing")),
        );
        problems
    }

    fn files(dir: &Path, extension: &str) -> Vec<std::path::PathBuf> {
        let mut found = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else { return found };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files(&path, extension));
            } else if path.extension().is_some_and(|e| e == extension) {
                found.push(path);
            }
        }
        found.sort();
        found
    }

    /// The `key:` lines of a table with their `vi:` and `en:` texts under them.
    fn read_table(file: &Path, keys: &mut BTreeMap<String, BTreeMap<String, String>>, problems: &mut Vec<String>) {
        let Ok(text) = fs::read_to_string(file) else { return };
        let mut current: Option<String> = None;
        for line in text.lines() {
            if line.trim().is_empty() || line.trim_start().starts_with('#') || line.starts_with("_version") {
                continue;
            }
            if !line.starts_with(' ') {
                let key = line.trim_end().trim_end_matches(':').to_owned();
                if keys.insert(key.clone(), BTreeMap::new()).is_some() {
                    problems.push(format!("{key}: written twice"));
                }
                current = Some(key);
            } else if let (Some(key), Some((tag, value))) = (&current, line.trim().split_once(": ")) {
                let value = value.trim();
                let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
                let value = unescape(value);
                if let Some(texts) = keys.get_mut(key) {
                    texts.insert(tag.to_owned(), value);
                }
            }
        }
    }

    /// A double-quoted YAML text without its quotes, its escapes read as what they stand for.
    fn unescape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
        out
    }

    /// The `%{name}`s of a text.
    fn arguments_of(text: &str) -> BTreeSet<String> {
        text.split("%{").skip(1).filter_map(|rest| rest.split_once('}')).map(|(name, _)| name.to_owned()).collect()
    }

    /// The string literals of `source` that are keys: a quote, then one of `prefixes` and only
    /// lower-case letters, digits, `_` and `.` (not ending with a dot), then a quote.
    fn literal_keys(source: &str, prefixes: &[&str]) -> BTreeSet<String> {
        let is_key_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.';
        source
            .match_indices('"')
            .filter_map(|(at, _)| {
                let rest = &source[at + 1..];
                let length = rest.find(|c: char| !is_key_char(c))?;
                let literal = &rest[..length];
                (rest[length..].starts_with('"')
                    && !literal.ends_with('.')
                    && prefixes.iter().any(|prefix| literal.starts_with(prefix)))
                .then(|| literal.to_owned())
            })
            .collect()
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
    fn every_key_has_its_vietnamese_and_its_english_text() {
        let problems = testing::problems(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
            &|locale, key| rust_i18n::t!(key, locale = locale).into_owned(),
            &["core.", "db."],
        );
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn a_text_in_english_is_the_english_one() {
        assert_eq!(rust_i18n::t!("db.not_implemented", locale = "en", what = "x"), "not implemented yet: x");
        assert_eq!(rust_i18n::t!("db.not_implemented", locale = "vi", what = "x"), "chưa làm: x");
    }

    #[test]
    fn the_crates_text_is_vietnamese_until_the_app_chooses() {
        assert_eq!(current(), Lang::Vi, "no test of this crate changes the language of the process");
    }
}
