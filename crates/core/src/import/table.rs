//! The shared reading of a report's rows, whichever file they came from: find the
//! account header, the Deals table and the Positions table, then parse the cells.
//!
//! The terminal translates the column titles and the section names, so nothing here
//! looks at them. Rows are recognised by what their cells hold, and a cell's column is
//! its distance from the row's anchor cells:
//!
//! - a **deal** row starts with a date-time, then an integer ticket, a symbol, a type
//!   (`buy`, `sell`, `balance`, …) and, for a buy or sell, the direction (`in`, `out`, …);
//! - a **position** row starts the same way but has a volume after the type, then the
//!   close date-time followed by a close price;
//! - the **account** row is `12345678 (USD, Broker-Server, real, Hedge)`, between the
//!   name and the company.
//!
//! The values of the type and direction cells (`buy`, `in`, `out by`, …) are the same in
//! every language.

use std::collections::BTreeMap;

use crate::model::{AccountInfo, Deal, DealBatch, DealEntry, DealKind, PositionLevels, Side};
use crate::time::parse_mt5_time;

use super::{ImportError, link};

/// A report as rows of cell texts.
pub(crate) type Rows = Vec<Vec<String>>;

/// Cells after the type column in a deal row: direction, volume, price, order, commission,
/// fee, swap, profit, balance, comment.
const WIDTH_WITH_FEE: usize = 10;

/// The account row, parsed.
struct Header {
    info: AccountInfo,
    accounting: link::Accounting,
}

/// A deal row and the index of its type cell.
struct DealRow<'a> {
    cells: &'a [String],
    type_at: usize,
}

/// Turns a report's rows into a batch.
///
/// Columns are found by their content (a date-time, a ticket, `in`/`out`), not only by
/// their titles, because the terminal writes the titles in its own language.
pub(crate) fn batch_from_rows(rows: &Rows) -> Result<DealBatch, ImportError> {
    let rows: Vec<&[String]> =
        rows.iter().filter(|row| row.iter().any(|cell| !cell.is_empty())).map(Vec::as_slice).collect();
    let deal_rows: Vec<DealRow<'_>> =
        rows.iter().filter_map(|cells| deal_type_at(cells).map(|type_at| DealRow { cells, type_at })).collect();
    if deal_rows.is_empty() {
        return Err(ImportError::NoDeals);
    }
    let Header { mut info, accounting } =
        header(&rows).ok_or_else(|| ImportError::Invalid(crate::tr!("core.import.no_account_row")))?;

    let layout = Layout::of(&deal_rows);
    let mut deals = deal_rows.iter().map(|row| deal(row, layout)).collect::<Result<Vec<_>, _>>()?;
    info.balance = deal_rows.iter().rev().find_map(|row| balance_of(row, layout));

    let positions: Vec<link::Position> = rows.iter().filter_map(|cells| position(cells)).collect();
    link::assign(&mut deals, &positions, accounting);
    let levels = levels(&positions);
    for deal in deals.iter_mut().filter(|d| d.kind.is_trade() && matches!(d.entry, DealEntry::In | DealEntry::None)) {
        if let Some(level) = levels.iter().find(|l| l.position_id == deal.position_id) {
            deal.sl = level.sl;
            deal.tp = level.tp;
        }
    }
    Ok(DealBatch { account: info, deals, positions: levels })
}

/// `1 234.56`, `-12.50`, `0.10 / 0.10` → a number (the first one).
pub(crate) fn number(cell: &str) -> Option<f64> {
    let first = cell.split('/').next()?.trim();
    let cleaned: String = first
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}')
        .map(|c| if c == '\u{2212}' { '-' } else { c })
        .collect();
    if cleaned.is_empty() { None } else { cleaned.parse().ok() }
}

/// A whole number, also as the spreadsheet writes it (`81234567.0`).
fn integer(cell: &str) -> Option<i64> {
    let cell = cell.trim();
    let whole = match cell.split_once('.') {
        Some((whole, fraction)) if fraction.bytes().all(|b| b == b'0') => whole,
        Some(_) => return None,
        None => cell,
    };
    whole.parse().ok()
}

fn is_datetime(cell: &str) -> bool {
    cell.contains(':') && parse_mt5_time(cell).is_some()
}

fn side_of(cell: &str) -> Option<Side> {
    match cell.trim().to_ascii_lowercase().as_str() {
        "buy" => Some(Side::Long),
        "sell" => Some(Side::Short),
        _ => None,
    }
}

/// `in`, `out`, `in/out` or `out by`. A spreadsheet's `1` is a volume, not an exit.
fn is_direction(cell: &str) -> bool {
    cell.contains(|c: char| c.is_ascii_alphabetic()) && DealEntry::parse(cell) != DealEntry::None
}

/// The index of the row's date-time when it opens the row and an integer ticket follows.
fn lead(cells: &[String]) -> Option<usize> {
    let at = cells.iter().position(|cell| !cell.is_empty())?;
    let ticket = cells.get(at + 1)?;
    (is_datetime(&cells[at]) && integer(ticket).is_some()).then_some(at)
}

/// The index of the type cell when the row is a deal: a buy or sell followed by its
/// direction, or a cash movement (`balance`, `credit`, …) which has no symbol.
fn deal_type_at(cells: &[String]) -> Option<usize> {
    let type_at = lead(cells)? + 3;
    let kind = cells.get(type_at)?;
    let symbol = &cells[type_at - 1];
    if side_of(kind).is_some() {
        return cells.get(type_at + 1).is_some_and(|cell| is_direction(cell)).then_some(type_at);
    }
    (symbol.is_empty() && !kind.is_empty() && number(kind).is_none() && !is_datetime(kind)).then_some(type_at)
}

/// Whether the deal rows carry the Fee column (older builds of the terminal do not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    WithFee,
    NoFee,
}

impl Layout {
    /// Judged by the usual width of a deal row.
    fn of(rows: &[DealRow<'_>]) -> Self {
        let mut widths: BTreeMap<usize, usize> = BTreeMap::new();
        for row in rows {
            *widths.entry(row.cells.len().saturating_sub(row.type_at + 1)).or_default() += 1;
        }
        match widths.into_iter().max_by_key(|&(width, count)| (count, width)) {
            Some((width, _)) if width < WIDTH_WITH_FEE => Self::NoFee,
            _ => Self::WithFee,
        }
    }
}

/// The cells of a deal row by their name.
struct Columns {
    type_at: usize,
    layout: Layout,
}

impl Columns {
    fn new(row: &DealRow<'_>, layout: Layout) -> Self {
        Self { type_at: row.type_at, layout }
    }

    /// The index of the cell `after` the type cell.
    fn at(&self, after: usize) -> usize {
        self.type_at + after
    }

    fn time(&self) -> usize {
        self.type_at - 3
    }

    fn ticket(&self) -> usize {
        self.type_at - 2
    }

    fn symbol(&self) -> usize {
        self.type_at - 1
    }

    fn swap(&self) -> usize {
        self.at(match self.layout {
            Layout::WithFee => 7,
            Layout::NoFee => 6,
        })
    }

    fn profit(&self) -> usize {
        self.swap() + 1
    }

    fn balance(&self) -> usize {
        self.swap() + 2
    }

    fn comment(&self) -> usize {
        self.swap() + 3
    }
}

fn deal(row: &DealRow<'_>, layout: Layout) -> Result<Deal, ImportError> {
    let columns = Columns::new(row, layout);
    let cell = |at: usize| row.cells.get(at).map_or("", String::as_str);
    let ticket = integer(cell(columns.ticket())).unwrap_or_default();
    let amount = |at: usize, column: &str| -> Result<f64, ImportError> {
        match cell(at) {
            "" | "-" => Ok(0.0),
            text => number(text).ok_or_else(|| {
                ImportError::Invalid(crate::tr!(
                    "core.import.deal_not_number",
                    ticket = ticket,
                    column = column,
                    text = text
                ))
            }),
        }
    };
    let time = cell(columns.time());
    let time_msc = parse_mt5_time(time)
        .ok_or_else(|| ImportError::Invalid(crate::tr!("core.import.deal_bad_time", ticket = ticket, time = time)))?;
    let kind = DealKind::parse(cell(columns.at(0)));
    let entry = if kind.is_trade() { DealEntry::parse(cell(columns.at(1))) } else { DealEntry::None };
    let comment = cell(columns.comment()).to_owned();
    let reason = if kind.is_trade() && entry != DealEntry::In { exit_reason(&comment) } else { None };
    let fee = match layout {
        Layout::WithFee => amount(columns.at(6), "Fee")?,
        Layout::NoFee => 0.0,
    };
    Ok(Deal {
        ticket,
        order: integer(cell(columns.at(4))).unwrap_or_default(),
        position_id: 0,
        time_msc,
        symbol: cell(columns.symbol()).to_owned(),
        kind,
        entry,
        volume: amount(columns.at(2), "Volume")?,
        price: amount(columns.at(3), "Price")?,
        commission: amount(columns.at(5), "Commission")?,
        swap: amount(columns.swap(), "Swap")?,
        fee,
        profit: amount(columns.profit(), "Profit")?,
        magic: 0,
        reason,
        comment,
        sl: 0.0,
        tp: 0.0,
        risk: None,
        result_r: None,
        gaps: crate::model::Gaps::NONE,
    })
}

fn balance_of(row: &DealRow<'_>, layout: Layout) -> Option<f64> {
    row.cells.get(Columns::new(row, layout).balance()).and_then(|cell| number(cell))
}

/// The server writes `[sl 1.09500]`, `[tp 1.10500]` or `[so 20.5%/…]` in the comment of a
/// deal that a stop, a target or a stop-out closed.
fn exit_reason(comment: &str) -> Option<String> {
    let word: String =
        comment.trim_start().trim_start_matches('[').chars().take_while(char::is_ascii_alphabetic).collect();
    let word = word.to_ascii_lowercase();
    matches!(word.as_str(), "sl" | "tp" | "so").then_some(word)
}

/// A closed position: its time and ticket, then symbol, type, volume, price, S/L, T/P,
/// the close time, the close price and the money. Orders look alike but have a state
/// (`filled`, …) where the close price is.
fn position(cells: &[String]) -> Option<link::Position> {
    let at = lead(cells)?;
    let type_at = at + 3;
    let side = side_of(cells.get(type_at)?)?;
    if cells.get(type_at + 1).is_some_and(|cell| is_direction(cell)) {
        return None;
    }
    let close_at = (type_at + 1..cells.len()).find(|&i| is_datetime(&cells[i]))?;
    if close_at < type_at + 5 {
        return None;
    }
    let level = |i: usize| cells.get(i).and_then(|cell| number(cell)).unwrap_or_default();
    Some(link::Position {
        id: integer(&cells[at + 1])?,
        symbol: cells[at + 2].clone(),
        side,
        open_ms: parse_mt5_time(&cells[at])?,
        open_price: number(&cells[close_at - 3])?,
        close_ms: parse_mt5_time(&cells[close_at])?,
        close_price: number(cells.get(close_at + 1)?)?,
        sl: level(close_at - 2),
        tp: level(close_at - 1),
    })
}

/// The stops and targets of the positions that have any, one entry per position.
fn levels(positions: &[link::Position]) -> Vec<PositionLevels> {
    let mut levels: Vec<PositionLevels> = Vec::new();
    for position in positions.iter().filter(|p| p.sl > 0.0 || p.tp > 0.0) {
        match levels.iter_mut().find(|l| l.position_id == position.id) {
            Some(known) => {
                if known.sl <= 0.0 {
                    known.sl = position.sl;
                }
                if known.tp <= 0.0 {
                    known.tp = position.tp;
                }
            }
            None => levels.push(PositionLevels { position_id: position.id, sl: position.sl, tp: position.tp }),
        }
    }
    levels
}

/// The account row and its neighbours: the name is in the row before it, the company in
/// the row after, each as a label (ending in `:`) followed by a value.
fn header(rows: &[&[String]]) -> Option<Header> {
    let (at, login, details) = rows.iter().enumerate().find_map(|(i, row)| {
        let (login, details) = row.iter().find_map(|cell| account_cell(cell))?;
        Some((i, login, details))
    })?;
    let value_of = |row: Option<&&[String]>| row.and_then(|r| labelled_value(r)).unwrap_or_default();
    let detail = |i: usize| details.get(i).cloned().unwrap_or_default();
    let info = AccountInfo {
        login,
        currency: detail(0),
        server: detail(1),
        name: value_of(at.checked_sub(1).and_then(|i| rows.get(i))),
        broker: value_of(rows.get(at + 1)),
        ..AccountInfo::default()
    };
    let accounting = if details.iter().any(|d| d.eq_ignore_ascii_case("netting")) {
        link::Accounting::Netting
    } else {
        link::Accounting::Hedging
    };
    Some(Header { info, accounting })
}

/// `12345678 (USD, Broker-Server, real, Hedge)` → the login and the parts in brackets.
fn account_cell(cell: &str) -> Option<(String, Vec<String>)> {
    let (login, rest) = cell.split_once('(')?;
    let login = login.trim();
    if login.is_empty() || !login.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let details: Vec<String> = rest.trim_end().strip_suffix(')')?.split(',').map(|d| d.trim().to_owned()).collect();
    (details.len() >= 2).then(|| (login.to_owned(), details))
}

/// The value of a `Label: value` row (empty when only the label is there).
fn labelled_value(row: &[String]) -> Option<String> {
    let mut cells = row.iter().filter(|cell| !cell.is_empty());
    let label = cells.next()?;
    label.ends_with([':', '：']).then(|| cells.next_back().cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| (*c).to_owned()).collect()
    }

    fn header_rows() -> Rows {
        vec![
            row(&["Trade History Report"]),
            row(&["Name:", "", "Nguyen Van A"]),
            row(&["Account:", "", "52012345 (USD, ICMarketsSC-Demo, demo, Hedge)"]),
            row(&["Company:", "", "Raw Trading Ltd"]),
            row(&["Date:", "", "2026.11.20 23:59"]),
        ]
    }

    #[test]
    fn reads_report_numbers() {
        assert_eq!(number("1 234.56"), Some(1234.56));
        assert_eq!(number("-12.50"), Some(-12.5));
        assert_eq!(number("−12.50"), Some(-12.5));
        assert_eq!(number("0.10 / 0.10"), Some(0.1));
        assert_eq!(number(""), None);
        assert_eq!(number("filled"), None);
    }

    #[test]
    fn reads_integers_as_a_spreadsheet_writes_them() {
        assert_eq!(integer("81234567"), Some(81_234_567));
        assert_eq!(integer(" 81234567.0 "), Some(81_234_567));
        assert_eq!(integer("1.5"), None);
        assert_eq!(integer("Deal"), None);
    }

    #[test]
    fn finds_the_account_by_its_shape() {
        let rows = header_rows();
        let rows: Vec<&[String]> = rows.iter().map(Vec::as_slice).collect();
        let Header { info, accounting } = header(&rows).expect("the account row is there");
        assert_eq!(
            (info.login.as_str(), info.currency.as_str(), info.server.as_str()),
            ("52012345", "USD", "ICMarketsSC-Demo")
        );
        assert_eq!((info.name.as_str(), info.broker.as_str()), ("Nguyen Van A", "Raw Trading Ltd"));
        assert_eq!(accounting, link::Accounting::Hedging);
        assert!(account_cell("Account:").is_none());
        assert!(account_cell("12 (x)").is_none());
    }

    #[test]
    fn a_report_without_deals_has_no_deals_table() {
        assert!(matches!(batch_from_rows(&header_rows()), Err(ImportError::NoDeals)));
        assert!(matches!(batch_from_rows(&Rows::new()), Err(ImportError::NoDeals)));
    }

    #[test]
    fn deals_without_an_account_are_refused() {
        let rows =
            vec![row(&["2026.11.18 09:00:00", "1", "", "balance", "", "", "", "", "", "", "", "100.00", "100.00", ""])];
        assert!(matches!(batch_from_rows(&rows), Err(ImportError::Invalid(_))));
    }

    #[test]
    fn reads_a_report_without_the_fee_column() {
        let mut rows = header_rows();
        rows.push(row(&[
            "2026.11.18 09:00:00",
            "1",
            "",
            "balance",
            "",
            "",
            "",
            "",
            "",
            "",
            "5 000.00",
            "5 000.00",
            "",
        ]));
        rows.push(row(&[
            "2026.11.18 09:05:00",
            "2",
            "EURUSD",
            "buy",
            "in",
            "0.10",
            "1.10000",
            "61",
            "-0.70",
            "0.00",
            "0.00",
            "5 000.00",
            "",
        ]));
        let batch = batch_from_rows(&rows).expect("a readable report");
        assert_eq!(batch.deals.len(), 2);
        let buy = &batch.deals[1];
        assert_eq!((buy.commission, buy.fee, buy.swap, buy.profit), (-0.7, 0.0, 0.0, 0.0));
        assert_eq!((buy.order, buy.position_id), (61, 61));
        assert_eq!(batch.account.balance, Some(5000.0));
        assert_eq!(batch.deals[0].profit, 5000.0);
    }

    #[test]
    fn names_the_column_and_the_deal_of_a_bad_number() {
        let mut rows = header_rows();
        rows.push(row(&[
            "2026.11.18 09:05:00",
            "7",
            "EURUSD",
            "buy",
            "in",
            "0.1x",
            "1.10000",
            "7",
            "0.00",
            "0.00",
            "0.00",
            "0.00",
            "5 000.00",
            "",
        ]));
        let Err(ImportError::Invalid(message)) = batch_from_rows(&rows) else { panic!("expected an invalid report") };
        assert!(message.contains("deal 7") && message.contains("Volume"), "{message}");
    }

    #[test]
    fn orders_and_positions_are_not_deals() {
        let order = row(&[
            "2026.11.18 09:05:00",
            "7",
            "EURUSD",
            "buy",
            "0.10 / 0.10",
            "1.10000",
            "",
            "",
            "2026.11.18 09:05:01",
            "filled",
            "",
        ]);
        assert!(deal_type_at(&order).is_none());
        assert!(position(&order).is_none());
        let closed = row(&[
            "2026.11.18 09:05:00",
            "7",
            "EURUSD",
            "buy",
            "0.10",
            "1.10000",
            "1.09500",
            "1.11000",
            "2026.11.18 10:00:00",
            "1.10500",
            "-0.70",
            "0.00",
            "5.00",
        ]);
        assert!(deal_type_at(&closed).is_none());
        let found = position(&closed).expect("a closed position");
        assert_eq!((found.id, found.sl, found.tp, found.close_price), (7, 1.095, 1.11, 1.105));
    }

    #[test]
    fn reads_the_reason_the_server_writes_in_a_comment() {
        assert_eq!(exit_reason("[sl 1.09500]").as_deref(), Some("sl"));
        assert_eq!(exit_reason("[TP 1.10500]").as_deref(), Some("tp"));
        assert_eq!(exit_reason("so: 20.5%/10.0").as_deref(), Some("so"));
        assert_eq!(exit_reason("slippage"), None);
        assert_eq!(exit_reason(""), None);
    }
}
