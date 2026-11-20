//! TradeLens core: everything that knows nothing of the UI.
//!
//! - [`model`]   MT5 accounts, deals, trades, journal entries, rules
//! - [`trades`]  deals → trades (by position, with partial closes and reversals)
//! - [`metrics`] statistics, balance curve, drawdown, breakdowns
//! - [`rules`]   discipline rules → violations
//! - [`time`]    server time → local time, sessions
//! - [`db`]      SQLite storage and migrations

pub mod db;
pub mod metrics;
pub mod model;
pub mod rules;
pub mod time;
pub mod trades;

pub use db::{Db, DbError};
pub use model::*;
