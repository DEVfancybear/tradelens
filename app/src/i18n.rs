//! The language of the app's text: the trader's choice, kept in the `language` setting.
//!
//! - The text is NSH's translations (`nsh_core::i18n`): every text the trader sees is a key of
//!   `locales/` (rust-i18n's `_version: 2` files, one per part of the app, Vietnamese and English
//!   side by side), read with `t!("settings.title")` where it is shown; `%{name}` in a text is the
//!   argument `name`.
//! - `nsh run` and `nsh package` hand `locales/` to the root themselves; `cargo` builds read it in
//!   `build.rs`, and [`install`] adds it to the surface, so the headless tests have it too.
//! - The language is Vietnamese until the setting says otherwise, whatever the system's language.
//!   Choosing another builds the window's content again ([`translated`]), so a page reads its text
//!   where it builds it, and the store reads the database again for text made while reading. The
//!   crates' text follows through [`tradelens_core::i18n::set_current`].

use std::collections::HashMap;

use nsh_core::{platform::Locale, prelude::*};
pub(crate) use tradelens_core::i18n::Lang;
use tradelens_core::{Db, i18n::set_current};

use crate::{
    store::{Writer, data_changed, query, writer},
    ui::controls::root_owner,
};

/// The app's translations as `build.rs` read them from `locales/`: languages, then keys and text.
pub(crate) const TRANSLATIONS: &str = include_str!(concat!(env!("OUT_DIR"), "/translations.json"));

/// The settings key of the language the trader reads the app in.
pub(crate) const LANGUAGE_KEY: &str = "language";

/// The locale of `lang`.
fn locale_of(lang: Lang) -> Option<Locale> {
    Locale::parse(lang.key())
}

/// The language the window shows (tracked where it is read).
pub(crate) fn language() -> Lang {
    Lang::parse(locale().as_str())
}

/// Gives the window's text and the crates' in `lang`.
fn show(lang: Lang) {
    set_current(lang);
    set_locale(locale_of(lang));
    // Text made while reading (a snapshot's labels, a reading's error) is made again in `lang`.
    data_changed();
}

/// Adds the app's translations to the surface and shows it in Vietnamese; call once, in the root,
/// before anything reads a text.
///
/// # Panics
///
/// When `build.rs` wrote translations that are not JSON; a test reads them.
pub(crate) fn install() {
    let map: HashMap<String, HashMap<String, String>> =
        serde_json::from_str(TRANSLATIONS).expect("build.rs writes the translations as JSON");
    add_translations(map);
    show(Lang::Vi);
}

/// Keeps the window in the language the trader chose: call it once, in the root, after the store.
pub(crate) fn follow_language() {
    let saved = query(|db, _| db.setting(LANGUAGE_KEY));
    provide_context(Picker { write: writer(), owner: root_owner() });
    // A memo: only a stored value that changed is followed, so a read made before the trader's own
    // write lands cannot put the previous language back.
    let stored = computed(move || saved.value().and_then(|value| value.as_deref().map(Lang::parse)));
    effect(move || {
        let Some(lang) = stored.get() else { return };
        if untracked(language) != lang {
            show(lang);
        }
    });
}

/// `content`, built again whenever the window's language changes.
pub(crate) fn translated<E: IntoElement + 'static>(content: impl Fn() -> E + 'static) -> impl IntoElement {
    dynamic(move || {
        let lang = language();
        untracked(|| view().flex_1().key(lang.key()).child(content()))
    })
}

/// What the settings change the language with; take it while building ([`picker`]).
#[derive(Clone, Copy)]
pub(crate) struct Picker {
    write: Writer,
    owner: Owner,
}

impl std::fmt::Debug for Picker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picker").finish_non_exhaustive()
    }
}

/// The handle the settings page changes the language with; call while building.
///
/// # Panics
///
/// When the root has not called [`follow_language`].
pub(crate) fn picker() -> Picker {
    use_context::<Picker>().expect("the root follows the language")
}

impl Picker {
    /// Shows the app in `lang` and keeps the choice for the next run.
    ///
    /// The write and the new window both run under the root, a frame later for the window: the
    /// control that was pressed is part of the window, and must not be disposed while its own
    /// handler runs, nor take the write with it.
    pub(crate) fn choose(self, lang: Lang) {
        let write = self.write;
        self.owner.run(move || {
            write.run(
                move |db: &mut Db| db.set_setting(LANGUAGE_KEY, lang.key()),
                |result| {
                    if let Err(error) = result {
                        log::warn!("the language was not stored: {error}");
                    }
                },
            );
            spawn(async move {
                next_frame().await;
                if untracked(language) != lang {
                    show(lang);
                }
            });
        });
    }
}

/// Runs `check` where `t!` gives the app's text in `lang`, and returns what it gave: for unit tests
/// of code that reads text.
///
/// `t!` reads the surface's language, which only a running engine has, so `check` runs while a
/// headless engine builds a probe component. The engine opens in `en-US`, and the probe's
/// translations give the texts of `lang` as English's, so no change of locale is waited for. One
/// probe runs at a time.
///
/// # Panics
///
/// When the engine does not start, or `check` panics (the engine swallows the panic, and the
/// answer never comes).
#[cfg(test)]
pub(crate) fn with_text<R: Send + 'static>(lang: Lang, check: impl FnOnce() -> R + Send + 'static) -> R {
    use std::{sync::mpsc, time::Duration};

    // A guard of nothing: a probe that failed must not fail the probes after it.
    let _one = probe::ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (answer, answered) = mpsc::channel();
    *probe::JOB.lock().expect("no probe panicked while holding its job") = Some(Box::new(move || {
        let mut map: HashMap<String, HashMap<String, String>> =
            serde_json::from_str(TRANSLATIONS).expect("build.rs writes the translations as JSON");
        let texts = map.remove(lang.key()).expect("the language has its text");
        add_translations(HashMap::from([("en".to_owned(), texts)]));
        drop(answer.send(check()));
    }));
    let config = nsh_engine::EngineConfig::new(nsh_testing::windowed_root::<probe::TextProbe>());
    let _engine = nsh_testing::Headless::start(config).expect("the probe's engine starts");
    answered.recv_timeout(Duration::from_secs(30)).expect("the check ran in the probe without panicking")
}

#[cfg(test)]
mod probe {
    use std::sync::Mutex;

    use nsh_core::prelude::*;

    /// What the next probe runs while it is built.
    pub(super) type Job = Box<dyn FnOnce() + Send>;

    pub(super) static JOB: Mutex<Option<Job>> = Mutex::new(None);
    pub(super) static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    /// Runs the job of [`super::with_text`] where `t!` can be read.
    #[component]
    pub(super) fn TextProbe() -> impl IntoElement {
        if let Some(job) = JOB.lock().expect("no probe panicked while holding its job").take() {
            job();
        }
        view()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_its_vietnamese_and_its_english_text() {
        let map: HashMap<String, HashMap<String, String>> =
            serde_json::from_str(TRANSLATIONS).expect("build.rs writes the translations as JSON");
        let vi = map.get("vi").expect("there is Vietnamese text");
        let en = map.get("en").expect("there is English text");
        let mut only_vi: Vec<&String> = vi.keys().filter(|key| !en.contains_key(*key)).collect();
        let mut only_en: Vec<&String> = en.keys().filter(|key| !vi.contains_key(*key)).collect();
        only_vi.sort();
        only_en.sort();
        assert!(only_vi.is_empty(), "keys without English text: {only_vi:?}");
        assert!(only_en.is_empty(), "keys without Vietnamese text: {only_en:?}");
        assert_eq!(map.len(), 2, "only Vietnamese and English: {:?}", map.keys().collect::<Vec<_>>());
    }

    #[test]
    fn a_unit_test_reads_the_text_of_the_language_it_asks_for() {
        assert_eq!(with_text(Lang::Vi, || t!("settings.title")), "Cài đặt");
        assert_eq!(with_text(Lang::En, || t!("settings.title")), "Settings");
    }
}
