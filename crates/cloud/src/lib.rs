//! Sign-in and cloud sync. No server of TradeLens's own: the user signs in with
//! Google or Microsoft (OAuth 2.0, PKCE, loopback redirect), and the app keeps its
//! files in the provider's app-only folder.
//!
//! - [`ai`]          the opt-in AI review of a week, with the trader's own key
//! - [`auth`]        the OAuth flow and the providers' endpoints and scopes
//! - [`token_store`] tokens in the OS credential store, never in the database
//! - [`drive`]       the [`drive::CloudDrive`] trait: list, upload, download, delete
//! - [`google`], [`onedrive`] its two implementations
//! - [`sync`]        push / pull of the change log, snapshots, restore (the merge rules are the core's)
//! - [`memory`]      a drive in memory, for tests and the demo
//! - [`notion`]      the Notion API, to import a trading journal database
//! - [`news`]        the week's economic calendar, fetched from Forex Factory's public feed
//!
//! Everything here blocks on the network: call it from a background thread, never from
//! the UI thread. Nothing here opens a window; the caller passes a function that opens the
//! browser. Base URLs can all be replaced, which is how the tests run against a local server.
//!
//! Design: `docs/cloud-sync.md`.

// The crate's text, from `locales/`; `tr!` reads it in the language of the process (the core's).
rust_i18n::i18n!("locales", fallback = "vi");

/// The text of `key` in `locales/`, in the language of the process, with `name = value` in its `%{name}`s.
macro_rules! tr {
    ($key:expr $(, $name:ident = $value:expr)* $(,)?) => {
        ::rust_i18n::t!($key, locale = ::tradelens_core::i18n::current().key() $(, $name = $value)*).into_owned()
    };
}
pub(crate) use tr;

pub mod ai;
mod api;
pub mod auth;
pub mod drive;
pub mod google;
mod http;
pub mod memory;
pub mod news;
pub mod notion;
pub mod onedrive;
pub mod sync;
pub mod token_store;

pub use api::UploadLimits;
pub use auth::Provider;
pub use http::RetryPolicy;

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("{}", crate::tr!("cloud.error.signed_out"))]
    SignedOut,
    #[error("{}", crate::tr!("cloud.error.auth", error = .0))]
    Auth(String),
    /// The user closed the sign-in or pressed cancel; not an error to show.
    #[error("{}", crate::tr!("cloud.error.cancelled"))]
    Cancelled,
    #[error("{}", crate::tr!("cloud.error.config", error = .0))]
    Config(String),
    #[error("{}", crate::tr!("cloud.error.network", error = .0))]
    Network(String),
    #[error("{}", crate::tr!("cloud.error.service", status = .status, message = .message))]
    Service { status: u16, message: String },
    #[error("{}", crate::tr!("cloud.error.store", error = .0))]
    Store(String),
    #[error(transparent)]
    Db(#[from] tradelens_core::DbError),
    #[error(transparent)]
    Import(#[from] tradelens_core::import::ImportError),
    #[error(transparent)]
    News(tradelens_core::news::FeedError),
    /// The user unticked Google's Drive permission on the consent screen (or took it back later);
    /// the message is the one for Google Drive because only Google reports it this way.
    #[error("{}", crate::tr!("cloud.error.drive_not_granted"))]
    DriveNotGranted,
    #[error("{}", crate::tr!("cloud.error.no_snapshot"))]
    NoSnapshot,
    #[error("{}", crate::tr!("cloud.error.corrupt", error = .0))]
    Corrupt(String),
    #[error("{}", crate::tr!("cloud.error.io", error = .0))]
    Io(#[from] std::io::Error),
    #[error("{}", crate::tr!("cloud.error.not_implemented", what = .0))]
    NotImplemented(&'static str),
}

pub type Result<T> = std::result::Result<T, CloudError>;

#[cfg(test)]
mod locales {
    #[test]
    fn every_key_has_its_vietnamese_and_its_english_text() {
        let problems = tradelens_core::i18n::testing::problems(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
            &|locale, key| rust_i18n::t!(key, locale = locale).into_owned(),
            &["cloud."],
        );
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn a_message_comes_in_the_language_asked_for() {
        let vi = rust_i18n::t!("cloud.error.signed_out", locale = "vi");
        let en = rust_i18n::t!("cloud.error.signed_out", locale = "en");
        assert_eq!((vi.as_ref(), en.as_ref()), ("chưa đăng nhập", "not signed in"));
        assert_eq!(super::CloudError::SignedOut.to_string(), "chưa đăng nhập", "the process speaks Vietnamese");
    }
}
