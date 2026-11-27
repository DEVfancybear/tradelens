//! TradeLens core: everything that knows nothing of the UI.
//!
//! - [`attachments`] chart images, many per entry
//! - [`catalog`] the trader's own option lists and fields
//! - [`missed`]  setups seen and not taken, and what they would have made
//! - [`news`]    the high-impact economic news of Forex Factory's weekly calendar
//! - [`model`]   MT5 accounts, deals, trades, journal entries, rules
//! - [`legacy`]  moving the data of an earlier version into the app's own folder
//! - [`journal`] the plan written at entry and how the trade compares with it
//! - [`trades`]  deals → trades (by position, with partial closes and reversals)
//! - [`metrics`] statistics, balance curve, drawdown, breakdowns
//! - [`playbook`] the trader's setups, with rules, examples and statistics
//! - [`report`]  the share card (PNG) of a trade or a period and the monthly PDF report, with what each privacy level shows
//! - [`review`]  weekly / monthly reviews and the queue of trades to look at again
//! - [`rules`]   discipline rules → violations
//! - [`search`]  full-text search over notes, plans, reviews and the playbook
//! - [`symbol`]  instrument codes as MetaTrader writes them
//! - [`time`]    server time → local time, sessions
//! - [`db`]      SQLite storage and migrations
//! - [`i18n`]    the language of the text, and the tables of text in both languages
//! - [`form`]    forms the trader shapes like Notion: rename, reorder, hide, add, convert, delete
//! - [`import`]  MT5 reports (HTML, XLSX), Notion exports (CSV) and JSON batches

// The crate's text, from `locales/`; [`tr!`] reads it in the language of the process.
rust_i18n::i18n!("locales", fallback = "vi");

/// The text of `key` in the crate's `locales/`, in the language of the process
/// ([`i18n::current`]), with `name = value` in its `%{name}`s: `tr!("db.not_implemented", what = name)`.
#[macro_export]
macro_rules! tr {
    ($key:expr $(, $name:ident = $value:expr)* $(,)?) => {
        ::rust_i18n::t!($key, locale = $crate::i18n::current().key() $(, $name = $value)*).into_owned()
    };
}

pub mod attachments;
pub mod catalog;
pub mod db;
pub mod form;
pub mod i18n;
pub mod import;
pub mod journal;
pub mod legacy;
pub mod metrics;
pub mod missed;
pub mod model;
pub mod news;
pub mod playbook;
pub mod report;
pub mod review;
pub mod rules;
pub mod search;
pub mod symbol;
pub mod sync;
pub mod time;
pub mod trades;

pub use db::{Db, DbError};
pub use model::{
    Account, AccountInfo, ChecklistItem, DayNote, Deal, DealBatch, DealEntry, DealKind, Gaps, JournalEntry,
    MANUAL_LOGIN, MANUAL_NAME, ManualTrade, NOTION_LOGIN, NOTION_NAME, Origin, PositionLevels, Rules, Side, Trade,
    TradeKey,
};
