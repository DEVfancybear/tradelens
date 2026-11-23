//! Notion trading journals: a database exported as "Markdown & CSV" (one row per trade,
//! columns named by the user). The import is automatic: [`infer`] reads the file, finds the
//! trade columns by their names and their contents, turns every other column into a property
//! of the trade form, and returns a [`NotionPlan`] to show; [`crate::Db::import_notion`] then
//! stores it. [`NotionMapping`] is what was detected, and can also override it.
//!
//! The same mapping serves rows read through the Notion API (`tradelens-cloud`).

use std::path::Path;

use crate::catalog::{CustomValues, FieldKind};
use crate::model::{AccountInfo, Deal, DealBatch};

use super::ImportError;

mod cells;
mod detect;
mod properties;
mod rows;
mod sheet;

/// The trade fields a Notion column can fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Field {
    OpenTime,
    CloseTime,
    Symbol,
    Side,
    Volume,
    OpenPrice,
    ClosePrice,
    StopLoss,
    TakeProfit,
    Profit,
    Commission,
    Swap,
    Setup,
    SetupGrade,
    Tags,
    Mistakes,
    Emotion,
    EmotionAfter,
    ExitReason,
    Market,
    Rating,
    FollowedPlan,
    ReviewLater,
    Notes,
    Screenshot,
}

impl Field {
    /// Every field, in the order the summary lists them.
    pub const ALL: [Self; 25] = [
        Self::OpenTime,
        Self::CloseTime,
        Self::Symbol,
        Self::Side,
        Self::Volume,
        Self::OpenPrice,
        Self::ClosePrice,
        Self::StopLoss,
        Self::TakeProfit,
        Self::Profit,
        Self::Commission,
        Self::Swap,
        Self::Setup,
        Self::SetupGrade,
        Self::Tags,
        Self::Mistakes,
        Self::Emotion,
        Self::EmotionAfter,
        Self::ExitReason,
        Self::Market,
        Self::Rating,
        Self::FollowedPlan,
        Self::ReviewLater,
        Self::Notes,
        Self::Screenshot,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenTime => "Thời gian vào",
            Self::CloseTime => "Thời gian ra",
            Self::Symbol => "Symbol",
            Self::Side => "Buy/Sell",
            Self::Volume => "Khối lượng",
            Self::OpenPrice => "Giá vào",
            Self::ClosePrice => "Giá ra",
            Self::StopLoss => "Stop loss",
            Self::TakeProfit => "Take profit",
            Self::Profit => "Lãi/lỗ",
            Self::Commission => "Phí",
            Self::Swap => "Swap",
            Self::Setup => "Setup",
            Self::SetupGrade => "Hạng setup",
            Self::Tags => "Tag",
            Self::Mistakes => "Lỗi",
            Self::Emotion => "Cảm xúc",
            Self::EmotionAfter => "Cảm xúc sau lệnh",
            Self::ExitReason => "Lý do thoát lệnh",
            Self::Market => "Trạng thái thị trường",
            Self::Rating => "Đánh giá",
            Self::FollowedPlan => "Đúng kế hoạch",
            Self::ReviewLater => "Xem lại sau",
            Self::Notes => "Ghi chú",
            Self::Screenshot => "Ảnh",
        }
    }

    /// Column names (English and Vietnamese) that usually hold the field. They are compared
    /// folded: case, diacritics, punctuation and units in brackets do not matter.
    #[expect(clippy::too_many_lines, reason = "a table of column names, one arm per field")]
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::OpenTime => &[
                "date",
                "open time",
                "open date",
                "entry time",
                "entry date",
                "time",
                "datetime",
                "opened",
                "ngày",
                "ngày vào",
                "giờ vào",
                "thời gian vào",
                "thời gian mở",
                "ngày mở",
            ],
            Self::CloseTime => &[
                "close time",
                "close date",
                "exit time",
                "exit date",
                "closed",
                "end time",
                "ngày đóng",
                "ngày ra",
                "giờ ra",
                "thời gian ra",
                "thời gian đóng",
            ],
            Self::Symbol => {
                &["symbol", "pair", "instrument", "ticker", "asset", "market pair", "cặp", "mã", "cặp tiền"]
            }
            Self::Side => &[
                "side",
                "direction",
                "type",
                "position",
                "long/short",
                "buy/sell",
                "order type",
                "chiều",
                "hướng",
                "loại lệnh",
            ],
            Self::Volume => {
                &["volume", "lot", "lots", "lot size", "size", "quantity", "qty", "position size", "khối lượng"]
            }
            Self::OpenPrice => &["entry", "entry price", "open price", "open", "giá vào", "giá mở"],
            Self::ClosePrice => &["exit", "exit price", "close price", "close", "giá ra", "giá đóng"],
            Self::StopLoss => &["sl", "stop", "stop loss", "stoploss", "cắt lỗ", "giá sl"],
            Self::TakeProfit => &["tp", "target", "take profit", "takeprofit", "chốt lời", "giá tp"],
            Self::Profit => &[
                "p&l",
                "pnl",
                "p/l",
                "pl",
                "profit",
                "net profit",
                "profit/loss",
                "result",
                "net",
                "net pnl",
                "realized pnl",
                "gain",
                "lãi/lỗ",
                "lãi lỗ",
                "lợi nhuận",
                "kết quả",
            ],
            Self::Commission => &["commission", "fee", "fees", "phí", "hoa hồng", "phí giao dịch"],
            Self::Swap => &["swap", "rollover", "phí qua đêm"],
            Self::Setup => &["setup", "strategy", "pattern", "playbook", "model", "chiến lược", "mô hình"],
            Self::SetupGrade => &["setup grade", "grade", "setup quality", "hạng setup", "chất lượng setup"],
            Self::Tags => &["tags", "tag", "labels", "nhãn", "thẻ"],
            Self::Mistakes => &["mistakes", "mistake", "errors", "error", "lỗi", "sai lầm"],
            Self::Emotion => {
                &["emotion", "emotions", "emotion before", "feeling", "feelings", "cảm xúc", "cảm xúc trước lệnh"]
            }
            Self::EmotionAfter => &["emotion after", "feeling after", "cảm xúc sau", "cảm xúc sau lệnh"],
            Self::ExitReason => &["exit reason", "close reason", "reason for exit", "lý do thoát", "lý do thoát lệnh"],
            Self::Market => &["market condition", "market state", "market", "trạng thái thị trường", "thị trường"],
            Self::Rating => &["rating", "stars", "execution", "execution rating", "đánh giá", "sao", "điểm thực thi"],
            Self::FollowedPlan => &[
                "followed plan",
                "plan followed",
                "follow plan",
                "followed rules",
                "đúng kế hoạch",
                "tuân thủ kế hoạch",
                "theo kế hoạch",
            ],
            Self::ReviewLater => &["review later", "to review", "xem lại sau", "cần xem lại"],
            Self::Notes => &[
                "notes",
                "note",
                "comment",
                "comments",
                "review",
                "description",
                "journal",
                "reflection",
                "lesson",
                "lessons",
                "ghi chú",
                "nhận xét",
                "mô tả",
                "bài học",
            ],
            Self::Screenshot => &[
                "screenshot",
                "screenshots",
                "chart",
                "image",
                "images",
                "photo",
                "ảnh",
                "hình ảnh",
                "ảnh chart",
                "link ảnh",
            ],
        }
    }
}

/// Which column fills which field: what [`infer`] detected, or what a caller imposes on it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotionMapping {
    /// `(field, column name as in the export)`.
    pub columns: Vec<(Field, String)>,
    /// The login of the account the trades go to; empty for the default Notion account.
    pub account_login: String,
}

impl NotionMapping {
    /// A guess from the header row alone (names only; [`infer`] also reads the contents).
    pub fn guess(headers: &[&str]) -> Self {
        let columns = Field::ALL
            .iter()
            .filter_map(|&field| {
                let aliases: Vec<String> = field.aliases().iter().map(|a| cells::fold(a)).collect();
                headers.iter().find(|h| aliases.contains(&cells::header_key(h))).map(|h| (field, (*h).to_owned()))
            })
            .collect();
        Self { columns, account_login: String::new() }
    }

    pub fn column(&self, field: Field) -> Option<&str> {
        self.columns.iter().find(|(f, _)| *f == field).map(|(_, c)| c.as_str())
    }
}

/// A row the import could not use, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowIssue {
    /// The line of the file the row starts on (the header is line 1).
    pub line: usize,
    /// Vietnamese, ready to show.
    pub reason: String,
}

impl std::fmt::Display for RowIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dòng {}: {}", self.line, self.reason)
    }
}

/// A column that is not a trade field, to become a property of the trade form.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedProperty {
    /// The key of this property in [`PlannedJournal::custom`]; the stored key may differ
    /// when the property already exists.
    pub key: String,
    /// The column's name, which names the property.
    pub label: String,
    /// The kind inferred from the values.
    pub kind: FieldKind,
    /// The choices of a select or multi-select, in the order the file first uses them.
    pub options: Vec<String>,
    /// How many imported trades have a value.
    pub filled: usize,
}

/// What the journal of an imported trade holds, before names become catalog keys.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlannedJournal {
    pub setup: String,
    pub setup_grade: String,
    pub tags: Vec<String>,
    pub mistakes: Vec<String>,
    pub emotion_before: String,
    pub emotion_after: String,
    pub exit_reason: String,
    pub market: String,
    /// 1 to 5; 0 when not rated.
    pub rating: u8,
    pub followed_plan: Option<bool>,
    pub review_later: Option<bool>,
    pub notes: String,
    /// Links or file names of the screenshots column.
    pub screenshots: Vec<String>,
    /// Values of the planned properties, by [`PlannedProperty::key`].
    pub custom: CustomValues,
}

/// One trade of the file: an entry and an exit deal (negative tickets, as a manual trade)
/// and its journal.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedRow {
    /// The line of the file the row starts on.
    pub line: usize,
    /// The row's name (the title column), shown in the summary.
    pub title: String,
    /// The entry deal's ticket, which identifies the trade: made of the title and the open time,
    /// so importing the same file again finds the same trade.
    pub position_id: i64,
    pub entry: Deal,
    pub exit: Deal,
    pub journal: PlannedJournal,
}

impl PlannedRow {
    /// Profit after commission and swap, as the CSV states them.
    pub fn net(&self) -> f64 {
        self.exit.profit + self.exit.commission + self.exit.swap
    }
}

/// What a Notion file holds, as [`infer`] understood it. Nothing is stored yet.
#[derive(Debug, Clone, PartialEq)]
pub struct NotionPlan {
    /// The account the trades go to; [`NotionPlan::use_account`] changes it.
    pub account: AccountInfo,
    /// The columns of the file.
    pub headers: Vec<String>,
    /// Which column was taken for which trade field.
    pub mapping: NotionMapping,
    /// The title column (`Name`), when there is one.
    pub title_column: Option<String>,
    /// Columns that are not imported: empty ones and Notion's own (`Created time`, …).
    pub ignored_columns: Vec<String>,
    /// The properties the other columns become.
    pub properties: Vec<PlannedProperty>,
    /// The rows that can be imported, in file order.
    pub rows: Vec<PlannedRow>,
    /// The rows that cannot, with the reason.
    pub issues: Vec<RowIssue>,
    /// What the file lacks and what was assumed instead (Vietnamese).
    pub warnings: Vec<String>,
    /// The data rows of the file, importable or not.
    pub total_rows: usize,
}

impl NotionPlan {
    /// Sends the trades to an existing account instead of the default Notion one.
    pub fn use_account(&mut self, account: &crate::model::Account) {
        self.account = AccountInfo {
            login: account.login.clone(),
            server: account.server.clone(),
            broker: account.broker.clone(),
            name: account.name.clone(),
            currency: account.currency.clone(),
            ..AccountInfo::default()
        };
    }

    /// The trades as a plain batch of deals (the journals are left out).
    pub fn batch(&self) -> DealBatch {
        DealBatch {
            account: self.account.clone(),
            deals: self.rows.iter().flat_map(|row| [row.entry.clone(), row.exit.clone()]).collect(),
            positions: Vec::new(),
        }
    }

    /// The sum of the trades' profit after costs, to check against the file.
    pub fn net_total(&self) -> f64 {
        self.rows.iter().map(PlannedRow::net).sum()
    }

    /// The first open time and the last close time, in server milliseconds.
    pub fn period(&self) -> Option<(i64, i64)> {
        let first = self.rows.iter().map(|r| r.entry.time_msc).min()?;
        let last = self.rows.iter().map(|r| r.exit.time_msc).max()?;
        Some((first, last))
    }
}

/// What [`crate::Db::import_notion`] did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NotionImportReport {
    pub account_id: i64,
    /// Trades stored for the first time.
    pub created: usize,
    /// Trades of an earlier import whose deals or journal changed.
    pub updated: usize,
    /// Trades already stored exactly so.
    pub unchanged: usize,
    /// Of the trades above, those that were already in the account as MT5 trades (same
    /// symbol, side and open time within two minutes): only their journal was taken.
    pub matched_mt5: usize,
    /// Properties added to the trade form, with their kinds.
    pub properties_created: Vec<PropertyCreated>,
    /// Properties of the form that already had the column's name and took its values.
    pub properties_reused: Vec<String>,
    /// Tags, mistakes, emotions, setup grades, exit reasons and market states the file used
    /// that the option lists did not have yet.
    pub options_added: usize,
    /// The rows of the file that were not imported, with the reason.
    pub skipped: Vec<RowIssue>,
    pub warnings: Vec<String>,
}

/// A property [`crate::Db::import_notion`] added to the trade form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyCreated {
    pub label: String,
    pub kind: FieldKind,
}

/// A name as compared across languages: lowercase, no diacritics, words separated by one space.
pub(crate) fn fold_name(text: &str) -> String {
    cells::fold(text)
}

/// A storage key made of a name (`Khung thời gian` → `khung_thoi_gian`).
pub(crate) fn key_of(text: &str) -> String {
    cells::slug(text)
}

/// Reads a Notion CSV export, detecting everything. Equal to [`infer_with`] with no overrides.
///
/// # Errors
///
/// [`ImportError::Invalid`] when the file is not CSV, has no data rows, has no column that can
/// be the open time, the symbol or the side, or none of its rows can be read.
pub fn infer(bytes: &[u8]) -> Result<NotionPlan, ImportError> {
    infer_with(bytes, &NotionMapping::default())
}

/// Reads a Notion CSV export. The columns `overrides` names are taken as given; the other
/// fields are detected from the headers and the contents.
///
/// # Errors
///
/// As [`infer`], and [`ImportError::Invalid`] when an override names a column the file lacks.
pub fn infer_with(bytes: &[u8], overrides: &NotionMapping) -> Result<NotionPlan, ImportError> {
    plan_sheet(&sheet::read(bytes)?, overrides)
}

/// Reads a table already in memory: the rows of a database fetched through the Notion API,
/// every cell flattened to text the way the CSV export writes it (numbers with `.` as the
/// decimal point). Detection and inference are those of [`infer`].
///
/// # Errors
///
/// As [`infer`]; also [`ImportError::Invalid`] when `headers` is empty.
pub fn infer_table(headers: &[String], rows: &[Vec<String>]) -> Result<NotionPlan, ImportError> {
    infer_table_with(headers, rows, &NotionMapping::default())
}

/// [`infer_table`] with columns imposed, as [`infer_with`].
///
/// # Errors
///
/// As [`infer_with`].
pub fn infer_table_with(
    headers: &[String],
    rows: &[Vec<String>],
    overrides: &NotionMapping,
) -> Result<NotionPlan, ImportError> {
    plan_sheet(&sheet::from_table(headers, rows)?, overrides)
}

fn plan_sheet(sheet: &sheet::Sheet, overrides: &NotionMapping) -> Result<NotionPlan, ImportError> {
    let layout = detect::detect(sheet, overrides)?;
    let planned = properties::infer(sheet, &layout);
    rows::build(sheet, &layout, planned, overrides)
}

/// [`infer`] on a file.
///
/// # Errors
///
/// [`ImportError::Io`] when the file cannot be read, else as [`infer`].
pub fn read_file(path: &Path) -> Result<NotionPlan, ImportError> {
    infer(&std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_a_typical_notion_journal() {
        let headers = ["Name", "Date", "Pair", "Direction", "Lot", "Entry", "Exit", "P&L", "Setup", "Notes"];
        let mapping = NotionMapping::guess(&headers);
        assert_eq!(mapping.column(Field::OpenTime), Some("Date"));
        assert_eq!(mapping.column(Field::Symbol), Some("Pair"));
        assert_eq!(mapping.column(Field::Side), Some("Direction"));
        assert_eq!(mapping.column(Field::Profit), Some("P&L"));
        assert_eq!(mapping.column(Field::StopLoss), None);
    }

    #[test]
    fn guesses_vietnamese_headers_without_diacritics_or_units() {
        let headers = ["Ngay vao", "Cặp", "Chiều", "Lãi/lỗ ($)", "Chiến lược"];
        let mapping = NotionMapping::guess(&headers);
        assert_eq!(mapping.column(Field::OpenTime), Some("Ngay vao"));
        assert_eq!(mapping.column(Field::Symbol), Some("Cặp"));
        assert_eq!(mapping.column(Field::Side), Some("Chiều"));
        assert_eq!(mapping.column(Field::Profit), Some("Lãi/lỗ ($)"));
        assert_eq!(mapping.column(Field::Setup), Some("Chiến lược"));
    }
}
