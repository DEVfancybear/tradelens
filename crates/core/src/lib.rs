//! TradeLens core: everything that knows nothing of the UI.
//!
//! - [`model`]   MT5 accounts, deals, trades, journal entries, rules
//! - [`time`]    server time → local time, sessions

pub mod model;
pub mod time;

pub use model::*;
