//! The app's state: what the pages read and change, over `tradelens-core`.
//! A `Copy` handle of signals, provided once by the root.

#![expect(dead_code, reason = "the skeleton's pages read these from v0.1")]

use nsh_core::prelude::*;

/// Which accounts the pages show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AccountFilter {
    #[default]
    All,
    One(i64),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Store {
    pub(crate) accounts: Signal<AccountFilter>,
    /// Bumped by the bridge and the cloud sync when the database changed.
    pub(crate) revision: Signal<u64>,
}

impl Store {
    fn new() -> Self {
        Self { accounts: signal(AccountFilter::All), revision: signal(0) }
    }
}

/// Provides the store to everything the root makes after it.
pub(crate) fn provide() {
    provide_context(Store::new());
}

/// The store; read while building, never in a listener.
pub(crate) fn store() -> Store {
    use_context::<Store>().expect("the root provides the store")
}
