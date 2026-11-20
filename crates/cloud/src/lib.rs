//! Sign-in and cloud sync. No server of TradeLens's own: the user signs in with
//! Google or Microsoft (OAuth 2.0, PKCE, loopback redirect), and the app keeps its
//! files in the provider's app-only folder.
//!
//! - [`ai`]          the opt-in AI review of a week, with the trader's own key
//! - [`auth`]        the OAuth flow and the providers' endpoints and scopes
//! - [`token_store`] tokens in the OS credential store, never in the database
//! - [`drive`]       the [`drive::CloudDrive`] trait: list, upload, download, delete
//! - [`google`], [`onedrive`] its two implementations
//! - [`sync`]        push / pull of the change log, snapshots, merging
//! - [`notion`]      the Notion API, to import a trading journal database
//!
//! Design: `docs/cloud-sync.md`.

pub mod ai;
pub mod auth;
pub mod drive;
pub mod google;
pub mod notion;
pub mod onedrive;
pub mod sync;
pub mod token_store;

pub use auth::Provider;

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("chưa đăng nhập")]
    SignedOut,
    #[error("đăng nhập bị huỷ hoặc thất bại: {0}")]
    Auth(String),
    #[error("lỗi mạng: {0}")]
    Network(String),
    #[error("dịch vụ trả lỗi {status}: {message}")]
    Service { status: u16, message: String },
    #[error(transparent)]
    Db(#[from] tradelens_core::DbError),
    #[error("chưa làm: {0}")]
    NotImplemented(&'static str),
}

pub type Result<T> = std::result::Result<T, CloudError>;
