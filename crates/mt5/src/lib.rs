//! The direct MT5 connection: for each account the trader signs in to, a portable copy of their
//! MetaTrader 5 terminal runs hidden and a Python worker drives it through the official
//! `MetaTrader5` package. No EA is installed in the terminal.
//!
//! - [`Terminal`] is what a connection asks of a terminal; [`python::Worker`] is the real one,
//!   [`memory::Broker`] a broker in memory for tests and the demo.
//! - [`Link`] keeps one account connected on a thread of its own: it logs in, reads the deals and
//!   the open positions every few seconds and tells the app through [`Event`]s, and it carries the
//!   trader's orders (close, move the stop or the target).
//! - [`install`] finds the MT5 terminals installed on the machine and makes an account's portable
//!   copy; [`python`] finds a Python that has the package. [`installer`] installs MetaTrader 5
//!   itself with MetaQuotes' installer when the machine has none.
//!
//! Nothing here touches the database or the UI: the app writes what a poll brings.

// The crate's text, from `locales/`; `tr!` reads it in the language of the process (the core's).
rust_i18n::i18n!("locales", fallback = "vi");

/// The text of `key` in `locales/`, in the language of the process, with `name = value` in its `%{name}`s.
macro_rules! tr {
    ($key:expr $(, $name:ident = $value:expr)* $(,)?) => {
        ::rust_i18n::t!($key, locale = ::tradelens_core::i18n::current().key() $(, $name = $value)*).into_owned()
    };
}
pub(crate) use tr;

pub mod install;
pub mod installer;
mod link;
pub mod memory;
mod protocol;
pub mod python;
pub mod runtime;

pub use link::{Event, Link, LinkSettings, LinkState, Listener, Opener};
pub use protocol::{AccountRef, Direction, Levels, Login, Mt5Error, Password, Poll, Position, Terminal};

#[cfg(test)]
mod locales {
    #[test]
    fn every_key_has_its_vietnamese_and_its_english_text() {
        let problems = tradelens_core::i18n::testing::problems(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
            &|locale, key| rust_i18n::t!(key, locale = locale).into_owned(),
            &["mt5."],
        );
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn a_message_comes_in_the_language_asked_for() {
        let vi = rust_i18n::t!("mt5.link.not_connected", locale = "vi");
        let en = rust_i18n::t!("mt5.link.not_connected", locale = "en");
        assert_eq!((vi.as_ref(), en.as_ref()), ("tài khoản chưa kết nối", "the account is not connected"));
        assert_eq!(crate::tr!("mt5.link.not_connected"), "tài khoản chưa kết nối", "the process speaks Vietnamese");
    }
}
