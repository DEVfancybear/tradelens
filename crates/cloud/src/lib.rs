//! Sign-in and cloud sync. No server of TradeLens's own: the user signs in with
//! Google or Microsoft (OAuth 2.0, PKCE, loopback redirect), and the app keeps its
//! files in the provider's app-only folder.
//!
//! - [`auth`]        the OAuth flow and the providers' endpoints and scopes
//! - [`token_store`] tokens in the OS credential store, never in the database
//! - [`drive`]       the [`drive::CloudDrive`] trait: list, upload, download, delete
//! - [`google`], [`onedrive`] its two implementations
//! - [`sync`]        push / pull of the change log, snapshots, merging
//!
//! Design: `docs/cloud-sync.md`.

pub mod auth;
pub mod drive;
pub mod google;
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
}

pub type Result<T> = std::result::Result<T, CloudError>;
