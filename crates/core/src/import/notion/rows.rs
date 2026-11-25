//! From rows of text to trades: the deals of each row, its journal, and the rows that
//! cannot be read, each with its line and its reason.

use std::collections::{HashMap, HashSet};

use chrono::{Duration, NaiveDate, NaiveDateTime};

use crate::catalog::{CustomValues, FieldKind, FieldValue};
use crate::import::ImportError;
use crate::model::{AccountInfo, Deal, DealEntry, DealKind, Gaps, JOURNAL_TICKET_BASE, Side};
use crate::time::to_ms;

use super::cells::{
    parse_bool, parse_datetime, parse_number, parse_range_end, parse_rating, parse_side, split_files, split_list,
    stable_hash,
};
use super::detect::Layout;
use super::properties::value_of;
use super::sheet::{Row, Sheet};
use super::{Field, NotionMapping, NotionPlan, PlannedJournal, PlannedProperty, PlannedRow, RowIssue, Verdict};

/// The login of the account Notion trades go to when the caller names none.
const DEFAULT_LOGIN: &str = "notion";

/// A trade's id is 40 bits of a hash.
const ID_MASK: u64 = (1 << 40) - 1;

/// What a column the file lacks stands in for.
const DEFAULT_VOLUME: f64 = 1.0;

/// Whether an empty cell is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
    Required,
    Optional,
}

pub(super) fn build(
    sheet: &Sheet,
    layout: &Layout,
    mut properties: Vec<(usize, PlannedProperty)>,
    overrides: &NotionMapping,
) -> Result<NotionPlan, ImportError> {
    let mut rows = Vec::new();
    let mut issues = Vec::new();
    let mut seen_titles: HashMap<(String, i64), usize> = HashMap::new();
    let mut used_ids: HashSet<u64> = HashSet::new();
    let mut per_day: HashMap<NaiveDate, i64> = HashMap::new();
    let mut seen_options: Vec<HashSet<String>> = vec![HashSet::new(); properties.len()];
    let mut option_order: Vec<Vec<String>> = vec![Vec::new(); properties.len()];

    for row in &sheet.rows {
        match read_row(sheet, layout, row) {
            Ok(mut parsed) => {
                // Trades dated without a time keep the order of the rows within their day.
                if parsed.date_only {
                    let slot = per_day.entry(parsed.trade.open.date()).or_insert(0);
                    let shift = Duration::seconds(*slot);
                    *slot += 1;
                    parsed.trade.open += shift;
                    parsed.trade.close += shift;
                }
                let key = (parsed.title.clone(), parsed.open_ms);
                let occurrence = seen_titles.entry(key).or_insert(0);
                let mut id =
                    stable_hash(&[&parsed.title, &parsed.open_ms.to_string(), &occurrence.to_string()]) & ID_MASK;
                *occurrence += 1;
                while !used_ids.insert(id) {
                    id = (id + 1) & ID_MASK;
                }
                let mut journal = parsed.journal;
                for (index, (column, property)) in properties.iter_mut().enumerate() {
                    let Some(value) = value_of(property.kind, row.cell(*column), sheet.locale) else {
                        continue;
                    };
                    for option in choices_of(&value) {
                        if seen_options[index].insert(option.clone()) {
                            option_order[index].push(option);
                        }
                    }
                    property.filled += 1;
                    journal.custom.insert(property.key.clone(), value);
                }
                let verdict = verdict_of(layout, row, &parsed.trade);
                rows.push(parsed.trade.into_row(row.line, parsed.title, id, verdict, journal));
            }
            Err(reason) => issues.push(RowIssue { line: row.line, reason }),
        }
    }
    for (index, (_, property)) in properties.iter_mut().enumerate() {
        if matches!(property.kind, FieldKind::Select | FieldKind::MultiSelect) {
            property.options = std::mem::take(&mut option_order[index]);
        }
    }
    if rows.is_empty() {
        let why: Vec<String> = issues.iter().take(3).map(ToString::to_string).collect();
        return Err(ImportError::Invalid(format!("không đọc được dòng nào trong file ({})", why.join("; "))));
    }

    let name_of = |column: usize| sheet.headers[column].clone();
    let account = if overrides.account_login.trim().is_empty() {
        AccountInfo {
            login: DEFAULT_LOGIN.to_owned(),
            broker: "Notion".to_owned(),
            name: "Nhật ký Notion".to_owned(),
            currency: "USD".to_owned(),
            ..AccountInfo::default()
        }
    } else {
        AccountInfo { login: overrides.account_login.trim().to_owned(), ..AccountInfo::default() }
    };
    let unit = if rows.iter().any(|row| row.result_r().is_some()) { Unit::R } else { Unit::Money };
    let warnings = warnings(layout, unit);
    Ok(NotionPlan {
        account,
        headers: sheet.headers.clone(),
        mapping: NotionMapping {
            columns: layout.fields.iter().map(|&(field, column)| (field, name_of(column))).collect(),
            account_login: overrides.account_login.clone(),
        },
        title_column: layout.title.map(name_of),
        outcome: layout.outcome_column(sheet),
        ignored_columns: layout.ignored.iter().map(|&c| name_of(c)).collect(),
        properties: properties.into_iter().map(|(_, property)| property).collect(),
        rows,
        issues,
        warnings,
        total_rows: sheet.rows.len(),
    })
}

fn choices_of(value: &FieldValue) -> Vec<String> {
    match value {
        FieldValue::Choice(c) => vec![c.clone()],
        FieldValue::Choices(cs) => cs.clone(),
        _ => Vec::new(),
    }
}

/// What the file states its results in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Money,
    R,
}

/// What the file lacks, and what the import assumes instead.
fn warnings(layout: &Layout, unit: Unit) -> Vec<String> {
    let mut notes = Vec::new();
    if unit == Unit::R {
        notes.push(
            "File ghi kết quả theo R, không có tiền: số tiền của các lệnh này hiện «—» và thống kê tính theo R."
                .to_owned(),
        );
    }
    // The lot and the prices a file lacks are recorded as gaps on the deals and show as a dash, so one
    // line says it for all of them.
    let missing: Vec<&str> =
        [(Field::Volume, "khối lượng"), (Field::OpenPrice, "giá vào"), (Field::ClosePrice, "giá ra")]
            .into_iter()
            .filter(|&(field, _)| layout.column(field).is_none())
            .map(|(_, name)| name)
            .collect();
    if !missing.is_empty() {
        notes.push(format!("File không có {}: các ô này hiện «—».", missing.join(" / ")));
    }
    if layout.column(Field::Profit).is_none() && unit != Unit::R {
        notes.push(format!("File không có cột {}: lãi/lỗ để 0.", Field::Profit.label()));
    }
    notes
}

/// The numbers and times of a row, before they become deals.
struct Trade {
    symbol: String,
    side: Side,
    volume: f64,
    open: NaiveDateTime,
    close: NaiveDateTime,
    open_price: f64,
    close_price: f64,
    sl: f64,
    tp: f64,
    profit: f64,
    /// The result in R of a file that has no money.
    result_r: Option<f64>,
    commission: f64,
    swap: f64,
    /// What the file did not tell: the lot, the prices, the time of day.
    gaps: Gaps,
}

struct Parsed {
    title: String,
    open_ms: i64,
    /// The open was a date with no time of day.
    date_only: bool,
    trade: Trade,
    journal: PlannedJournal,
}

impl Trade {
    /// The entry deal and the exit deal of a hand-entered trade (see [`crate::Db::add_manual_trade`]):
    /// the entry has the lower ticket, costs and profit are on the exit.
    fn into_row(
        self,
        line: usize,
        title: String,
        id: u64,
        verdict: Option<Verdict>,
        journal: PlannedJournal,
    ) -> PlannedRow {
        let exit_ticket = -(JOURNAL_TICKET_BASE + 2 * i64::try_from(id).expect("a 40-bit id fits i64"));
        let entry_ticket = exit_ticket - 1;
        let (entry_kind, exit_kind) = match self.side {
            Side::Long => (DealKind::Buy, DealKind::Sell),
            Side::Short => (DealKind::Sell, DealKind::Buy),
        };
        let comment = if title.is_empty() { "Notion".to_owned() } else { title.clone() };
        let deal = |ticket: i64, kind: DealKind, entry: DealEntry, at: NaiveDateTime, price: f64| Deal {
            ticket,
            order: ticket,
            position_id: entry_ticket,
            time_msc: to_ms(at),
            symbol: self.symbol.clone(),
            kind,
            entry,
            volume: self.volume,
            price,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit: 0.0,
            magic: 0,
            comment: comment.clone(),
            sl: self.sl,
            tp: self.tp,
            risk: None,
            result_r: None,
            gaps: self.gaps,
            reason: None,
        };
        let entry = deal(entry_ticket, entry_kind, DealEntry::In, self.open, self.open_price);
        let exit = Deal {
            commission: self.commission,
            swap: self.swap,
            profit: self.profit,
            result_r: self.result_r,
            ..deal(exit_ticket, exit_kind, DealEntry::Out, self.close, self.close_price)
        };
        PlannedRow { line, title, position_id: entry_ticket, entry, exit, verdict, journal }
    }
}

/// How the trade ended: by its status cell when the file has a status column and the cell holds a
/// value it learned, else by the sign of the result.
fn verdict_of(layout: &Layout, row: &Row, trade: &Trade) -> Option<Verdict> {
    if let Some((column, values)) = &layout.outcome {
        let cell = row.cell(*column);
        if let Some((_, verdict)) = values.iter().find(|(value, _)| value.eq_ignore_ascii_case(cell)) {
            return Some(*verdict);
        }
    }
    let score = trade.result_r.unwrap_or(trade.profit + trade.commission + trade.swap);
    let known = trade.result_r.is_some() || layout.column(Field::Profit).is_some();
    if !known {
        return None;
    }
    Some(if score > 0.0 {
        Verdict::Win
    } else if score < 0.0 {
        Verdict::Loss
    } else {
        Verdict::BreakEven
    })
}

/// Whether a time cell is a date with no time of day: it has no `:` and reads as midnight.
fn is_date_only(cell: &str, at: NaiveDateTime) -> bool {
    !cell.contains(':') && at.time() == chrono::NaiveTime::MIN
}

/// What a row's file left out: the lot, the prices (either one), the time of day. Recorded on the
/// deals, so that the placeholders they hold are never shown or counted.
fn gaps_of(volume: Option<f64>, prices: (Option<f64>, Option<f64>), time: Gaps) -> Gaps {
    let mut gaps = time;
    if volume.is_none() {
        gaps = gaps.with(Gaps::VOLUME);
    }
    if prices.0.is_none() || prices.1.is_none() {
        gaps = gaps.with(Gaps::PRICE);
    }
    gaps
}

/// Reads one row, or says in Vietnamese why it cannot be.
fn read_row(sheet: &Sheet, layout: &Layout, row: &Row) -> Result<Parsed, String> {
    let locale = sheet.locale;
    let text = |field: Field| layout.column(field).map_or("", |column| row.cell(column));
    let number = |field: Field, need: Need| -> Result<Option<f64>, String> {
        let cell = text(field);
        if cell.is_empty() {
            return match need {
                Need::Required if layout.column(field).is_some() => Err(format!("thiếu {}", field.label())),
                _ => Ok(None),
            };
        }
        parse_number(cell, locale.comma).map(Some).ok_or_else(|| format!("{} «{cell}» không phải là số", field.label()))
    };
    let positive = |field: Field| -> Result<Option<f64>, String> {
        match number(field, Need::Required)? {
            Some(value) if value <= 0.0 => {
                Err(format!("{} phải là số dương, không phải «{}»", field.label(), text(field)))
            }
            other => Ok(other),
        }
    };

    let open_cell = text(Field::OpenTime);
    let open = parse_datetime(open_cell, locale.dates).ok_or_else(|| {
        if open_cell.is_empty() {
            "thiếu thời gian vào".to_owned()
        } else {
            format!("không đọc được thời gian vào «{open_cell}»")
        }
    })?;
    let close_cell = text(Field::CloseTime);
    let close = if close_cell.is_empty() {
        parse_range_end(open_cell, locale.dates).unwrap_or(open)
    } else {
        parse_datetime(close_cell, locale.dates).ok_or_else(|| format!("không đọc được thời gian ra «{close_cell}»"))?
    };
    if close < open {
        return Err("thời gian ra sớm hơn thời gian vào".to_owned());
    }
    let symbol = crate::symbol::normalise(text(Field::Symbol));
    if symbol.is_empty() {
        return Err("thiếu symbol".to_owned());
    }
    let side_cell = text(Field::Side);
    let side = parse_side(side_cell).ok_or_else(|| {
        if side_cell.is_empty() {
            "thiếu chiều lệnh (Buy/Sell)".to_owned()
        } else {
            format!("chiều lệnh «{side_cell}» không phải Buy/Sell hoặc Long/Short")
        }
    })?;

    // What the file leaves out is recorded, so that the placeholders below are never shown or counted.
    let volume = positive(Field::Volume)?;
    let open_price = positive(Field::OpenPrice)?;
    let close_price = positive(Field::ClosePrice)?;
    let date_only = is_date_only(open_cell, open);
    let day_only_close = !close_cell.is_empty() && is_date_only(close_cell, close);
    let time = if date_only || day_only_close { Gaps::TIME } else { Gaps::NONE };
    let gaps = gaps_of(volume, (open_price, close_price), time);

    let trade = Trade {
        symbol,
        side,
        volume: volume.unwrap_or(DEFAULT_VOLUME),
        open,
        close,
        open_price: open_price.unwrap_or(0.0),
        close_price: close_price.unwrap_or(0.0),
        sl: number(Field::StopLoss, Need::Optional)?.unwrap_or(0.0),
        tp: number(Field::TakeProfit, Need::Optional)?.unwrap_or(0.0),
        profit: number(Field::Profit, Need::Required)?.unwrap_or(0.0),
        // A cell that is not a number (a note, a dash) is a trade whose R is not known.
        result_r: parse_number(text(Field::ResultR), locale.comma),
        commission: number(Field::Commission, Need::Optional)?.unwrap_or(0.0),
        swap: number(Field::Swap, Need::Optional)?.unwrap_or(0.0),
        gaps,
    };

    let first_item = |field: Field| split_list(text(field)).into_iter().next().unwrap_or_default();
    let journal = PlannedJournal {
        setup: text(Field::Setup).to_owned(),
        setup_grade: text(Field::SetupGrade).to_owned(),
        tags: split_list(text(Field::Tags)),
        mistakes: split_list(text(Field::Mistakes)),
        emotion_before: first_item(Field::Emotion),
        emotion_after: first_item(Field::EmotionAfter),
        exit_reason: text(Field::ExitReason).to_owned(),
        market: text(Field::Market).to_owned(),
        rating: parse_rating(text(Field::Rating)).unwrap_or(0),
        followed_plan: parse_bool(text(Field::FollowedPlan)),
        review_later: parse_bool(text(Field::ReviewLater)),
        notes: text(Field::Notes).to_owned(),
        screenshots: split_files(text(Field::Screenshot)),
        attachments: Vec::new(),
        missing_files: Vec::new(),
        custom: CustomValues::new(),
    };
    let title = row.cell(layout.key_column).to_owned();
    Ok(Parsed { title, open_ms: to_ms(open), date_only, trade, journal })
}
