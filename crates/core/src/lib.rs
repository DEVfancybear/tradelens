//! TradeLens core: everything that knows nothing of the UI.
//!
//! - [`attachments`] chart images, many per entry
//! - [`catalog`] the trader's own option lists and fields
//! - [`missed`]  setups seen and not taken, and what they would have made
//! - [`model`]   MT5 accounts, deals, trades, journal entries, rules
//! - [`journal`] the plan written at entry and how the trade compares with it
//! - [`trades`]  deals → trades (by position, with partial closes and reversals)
//! - [`metrics`] statistics, balance curve, drawdown, breakdowns
//! - [`playbook`] the trader's setups, with rules, examples and statistics
//! - [`report`]  the share card (PNG) of a trade or a period and the monthly PDF report, with what each privacy level shows
//! - [`review`]  weekly / monthly reviews and the queue of trades to look at again
//! - [`rules`]   discipline rules → violations
//! - [`search`]  full-text search over notes, plans, reviews and the playbook
//! - [`time`]    server time → local time, sessions
//! - [`db`]      SQLite storage and migrations
//! - [`form`]    forms the trader shapes like Notion: rename, reorder, hide, add, convert, delete
//! - [`import`]  MT5 reports (HTML, XLSX), Notion exports (CSV) and JSON batches

pub mod attachments;
pub mod catalog;
pub mod db;
pub mod form;
pub mod import;
pub mod journal;
pub mod metrics;
pub mod missed;
pub mod model;
pub mod playbook;
pub mod report;
pub mod review;
pub mod rules;
pub mod search;
pub mod sync;
pub mod time;
pub mod trades;

pub use db::{Db, DbError};
pub use model::{
    Account, AccountInfo, ChecklistItem, DayNote, Deal, DealBatch, DealEntry, DealKind, JournalEntry, ManualTrade,
    PositionLevels, Rules, Side, Trade, TradeKey,
};
