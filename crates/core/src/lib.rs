//! TradeLens core: everything that knows nothing of the UI.
//!
//! - [`catalog`] the trader's own option lists and fields
//! - [`model`]   MT5 accounts, deals, trades, journal entries, rules
//! - [`journal`] the plan written at entry and how the trade compares with it
//! - [`trades`]  deals → trades (by position, with partial closes and reversals)
//! - [`metrics`] statistics, balance curve, drawdown, breakdowns
//! - [`rules`]   discipline rules → violations
//! - [`time`]    server time → local time, sessions
//! - [`db`]      SQLite storage and migrations
//! - [`import`]  MT5 reports (HTML, XLSX), Notion exports (CSV) and JSON batches

pub mod catalog;
pub mod db;
pub mod import;
pub mod journal;
pub mod metrics;
pub mod model;
pub mod rules;
pub mod time;
pub mod trades;

pub use db::{Db, DbError};
pub use model::{
    Account, AccountInfo, ChecklistItem, DayNote, Deal, DealBatch, DealEntry, DealKind, JournalEntry, ManualTrade,
    PositionLevels, Rules, Side, Trade, TradeKey,
};
