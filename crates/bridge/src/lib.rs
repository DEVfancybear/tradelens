//! The bridge: a small HTTP server on `127.0.0.1` that receives [`DealBatch`]es from
//! the EA (`WebRequest`) and the Python script, checks the token, writes them through
//! [`tradelens_core::Db`] and bumps a revision the UI watches.
//!
//! Contract: `docs/sync-api.md`.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

pub mod api;
mod server;

pub use tradelens_core::DealBatch;

/// The default port; the EA's input and the settings screen use it too.
pub const DEFAULT_PORT: u16 = 7878;

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub db_path: PathBuf,
    pub port: u16,
    /// Sent by clients in `X-TradeLens-Token`.
    pub token: String,
}

/// Shared with the UI: increases each time the bridge stores something.
#[derive(Debug, Clone, Default)]
pub struct Revision(Arc<AtomicU64>);

impl Revision {
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    pub fn bump(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

/// What the bridge reports to the data sources screen.
#[derive(Debug, Clone, PartialEq)]
pub enum BridgeEvent {
    Listening { port: u16 },
    Received { source: String, account: String, inserted: usize, updated: usize },
    Rejected { reason: String },
    Stopped { error: Option<String> },
}

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("không mở được cổng {port}: {message}")]
    Bind { port: u16, message: String },
    #[error(transparent)]
    Db(#[from] tradelens_core::DbError),
}

/// A running bridge; dropping it stops the server.
pub struct Bridge {
    pub revision: Revision,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Bridge {
    /// Starts the server on its own thread. `on_event` is called from that thread.
    pub fn start(
        config: BridgeConfig,
        revision: Revision,
        on_event: impl Fn(BridgeEvent) + Send + 'static,
    ) -> Result<Self, BridgeError> {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread = server::spawn(config, revision.clone(), stop.clone(), Box::new(on_event))?;
        Ok(Self { revision, stop, thread: Some(thread) })
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A new random token: 32 hex characters.
pub fn new_token() -> String {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("the OS has a random source");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (super::new_token(), super::new_token());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
