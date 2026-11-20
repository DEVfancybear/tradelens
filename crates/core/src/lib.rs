//! TradeLens core: everything that knows nothing of the UI.
//!
//! - [`model`]   MT5 accounts, deals, trades, journal entries, rules
//! - [`trades`]  deals → trades (by position, with partial closes and reversals)
//! - [`time`]    server time → local time, sessions

pub mod model;
pub mod time;
pub mod trades;

pub use model::*;
