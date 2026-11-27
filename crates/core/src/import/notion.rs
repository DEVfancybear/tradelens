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

mod archive;
mod cells;
mod detect;
mod properties;
mod rows;
mod sheet;

pub use archive::{ImageCounts, NotionArchive, read_archive};

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
    ResultR,
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
    pub const ALL: [Self; 26] = [
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
        Self::ResultR,
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

    /// What the trader calls the field, in the language of the process.
    pub fn label(self) -> String {
        let key = match self {
            Self::OpenTime => "core.notion.field.open_time",
            Self::CloseTime => "core.notion.field.close_time",
            Self::Symbol => "core.notion.field.symbol",
            Self::Side => "core.notion.field.side",
            Self::Volume => "core.notion.field.volume",
            Self::OpenPrice => "core.notion.field.open_price",
            Self::ClosePrice => "core.notion.field.close_price",
            Self::StopLoss => "core.notion.field.stop_loss",
            Self::TakeProfit => "core.notion.field.take_profit",
            Self::Profit => "core.notion.field.profit",
            Self::ResultR => "core.notion.field.result_r",
            Self::Commission => "core.notion.field.commission",
            Self::Swap => "core.notion.field.swap",
            Self::Setup => "core.notion.field.setup",
            Self::SetupGrade => "core.notion.field.setup_grade",
            Self::Tags => "core.notion.field.tags",
            Self::Mistakes => "core.notion.field.mistakes",
            Self::Emotion => "core.notion.field.emotion",
            Self::EmotionAfter => "core.notion.field.emotion_after",
            Self::ExitReason => "core.notion.field.exit_reason",
            Self::Market => "core.notion.field.market",
            Self::Rating => "core.notion.field.rating",
            Self::FollowedPlan => "core.notion.field.followed_plan",
            Self::ReviewLater => "core.notion.field.review_later",
            Self::Notes => "core.notion.field.notes",
            Self::Screenshot => "core.notion.field.screenshot",
        };
        crate::tr!(key)
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
            Self::ResultR => {
                &["r", "rr", "r r", "r multiple", "risk reward", "result r", "realized r", "realised r", "kết quả r"]
            }
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
    /// Ready to show, in the language the trader had when the import ran.
    pub reason: String,
}

impl std::fmt::Display for RowIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", crate::tr!("core.notion.row_issue", line = self.line, reason = self.reason))
    }
}

/// How a trade ended, as the file says it: by the sign of its result, or by its status column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Win,
    Loss,
    BreakEven,
}

impl Verdict {
    pub fn label(self) -> String {
        match self {
            Self::Win => crate::tr!("core.notion.verdict.win"),
            Self::Loss => crate::tr!("core.notion.verdict.loss"),
            Self::BreakEven => crate::tr!("core.notion.verdict.break_even"),
        }
    }
}

/// A status column (win, loss, breakeven…) and what each of its values was learned to mean: the
/// values that go with positive results are wins, those with negative results losses, those with
/// results of zero breakeven. No word list is involved.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeColumn {
    pub column: String,
    pub values: Vec<(String, Verdict)>,
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
    /// Images found for the file names above (see [`NotionPlan::attach_images`]).
    pub attachments: Vec<crate::attachments::Attachment>,
    /// Of the file names above, those no image was found for.
    pub missing_files: Vec<String>,
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
    /// Won, lost or broke even: by the status column when there is one, else by the result.
    pub verdict: Option<Verdict>,
    pub journal: PlannedJournal,
}

impl PlannedRow {
    /// Profit after commission and swap, as the CSV states them.
    pub fn net(&self) -> f64 {
        self.exit.profit + self.exit.commission + self.exit.swap
    }

    /// The result in R when the file states it in R and has no money.
    pub fn result_r(&self) -> Option<f64> {
        self.exit.result_r
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
    /// The status column and what its values mean, when one was recognised.
    pub outcome: Option<OutcomeColumn>,
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

    /// Stores the images the rows name, reading each one with `read` (given the name as the file
    /// writes it, percent-encoded) and keeping them in `store`, the data folder's `attachments/`.
    /// A name `read` finds nothing for, or whose image is refused, is listed as missing on its row.
    ///
    /// # Errors
    ///
    /// [`ImportError::Io`] when the store cannot be written.
    pub fn attach_images(
        &mut self,
        read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
        store: &Path,
        now_ms: i64,
    ) -> Result<ImageCounts, ImportError> {
        use crate::attachments::{self, AttachError};
        let mut counts = ImageCounts::default();
        for row in &mut self.rows {
            row.journal.attachments.clear();
            row.journal.missing_files.clear();
            for name in row.journal.screenshots.clone() {
                let decoded = cells::percent_decode(&name);
                let stored = read(&name).map(|bytes| attachments::import_bytes(&decoded, &bytes, store, now_ms));
                match stored {
                    Some(Ok(image)) => {
                        attachments::append(&mut row.journal.attachments, [image]);
                        counts.attached += 1;
                    }
                    Some(Err(AttachError::Io { source, .. })) => return Err(ImportError::Io(source)),
                    Some(Err(_)) | None => {
                        row.journal.missing_files.push(name);
                        counts.missing += 1;
                    }
                }
            }
        }
        Ok(counts)
    }

    /// The sum of the trades' profit after costs, to check against the file.
    pub fn net_total(&self) -> f64 {
        self.rows.iter().map(PlannedRow::net).sum()
    }

    /// Whether the file states its results in R only: the trades have no money.
    pub fn is_in_r(&self) -> bool {
        self.rows.iter().any(|row| row.result_r().is_some())
    }

    /// The sum of the results in R of the trades that state one.
    pub fn total_r(&self) -> f64 {
        self.rows.iter().filter_map(PlannedRow::result_r).sum()
    }

    /// How many trades won, lost and broke even.
    pub fn verdicts(&self) -> (usize, usize, usize) {
        let count = |wanted: Verdict| self.rows.iter().filter(|row| row.verdict == Some(wanted)).count();
        (count(Verdict::Win), count(Verdict::Loss), count(Verdict::BreakEven))
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
    /// Images stored on the imported trades (from a Notion export archive), and file names of the
    /// files column that the archive did not hold.
    pub images_attached: usize,
    pub images_missing: usize,
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

/// Reads a Notion export, a bare CSV or the `.zip` Notion writes: the plan, and for a zip the
/// archive the rows' images are read from (see [`NotionArchive::attach_images`]).
///
/// # Errors
///
/// As [`read_file`] and [`read_archive`].
pub fn read_source(path: &Path) -> Result<(NotionPlan, Option<NotionArchive>), ImportError> {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        let (plan, archive) = read_archive(path)?;
        Ok((plan, Some(archive)))
    } else {
        Ok((read_file(path)?, None))
    }
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
