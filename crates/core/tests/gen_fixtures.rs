//! Writes the MT5 report fixtures in `tests/fixtures/` from the scenario in `common`.
//!
//! The fixtures are committed; regenerate them (after changing the scenario or a
//! renderer) with:
//!
//! ```text
//! cargo test -p tradelens-core --test gen_fixtures -- --ignored
//! ```
//!
//! One report, five files: the English HTML the way the terminal saves it (UTF-16 LE with
//! a BOM), the same report in Russian (UTF-16 LE) and in Vietnamese (UTF-8, with sloppy
//! uppercase markup and no closing tags), and the XLSX with typed cells and with text
//! cells only.

mod common;

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use common::{
    CLOSED, CLOSED_TRADES, COMPANY, CURRENCY, Closed, EVENTS, Event, FINAL_BALANCE, LOGIN, NAME, REPORT_DATE, SERVER,
    TOTAL_NET_PROFIT,
};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};

/// The columns of the widest table (Deals); titles and label rows span them all.
const WIDTH: usize = 14;
/// The GBPUSD position, the one that pays a fee.
const GBPUSD_POSITION: i64 = 91_000_006;

/// The words the terminal translates. Values (`buy`, `in`, `balance`) are not translated.
struct Words {
    title: &'static str,
    name: &'static str,
    account: &'static str,
    company: &'static str,
    date: &'static str,
    positions: &'static str,
    orders: &'static str,
    deals: &'static str,
    results: &'static str,
    position_head: [&'static str; 13],
    order_head: [&'static str; 11],
    deal_head: [&'static str; 14],
    net_profit: &'static str,
    gross_profit: &'static str,
    gross_loss: &'static str,
    total_trades: &'static str,
    profit_trades: &'static str,
    loss_trades: &'static str,
    filled: &'static str,
}

const ENGLISH: Words = Words {
    title: "Trade History Report",
    name: "Name:",
    account: "Account:",
    company: "Company:",
    date: "Date:",
    positions: "Positions",
    orders: "Orders",
    deals: "Deals",
    results: "Results",
    position_head: [
        "Time",
        "Position",
        "Symbol",
        "Type",
        "Volume",
        "Price",
        "S / L",
        "T / P",
        "Time",
        "Price",
        "Commission",
        "Swap",
        "Profit",
    ],
    order_head: [
        "Open Time",
        "Order",
        "Symbol",
        "Type",
        "Volume",
        "Price",
        "S / L",
        "T / P",
        "Time",
        "State",
        "Comment",
    ],
    deal_head: [
        "Time",
        "Deal",
        "Symbol",
        "Type",
        "Direction",
        "Volume",
        "Price",
        "Order",
        "Commission",
        "Fee",
        "Swap",
        "Profit",
        "Balance",
        "Comment",
    ],
    net_profit: "Total Net Profit:",
    gross_profit: "Gross Profit:",
    gross_loss: "Gross Loss:",
    total_trades: "Total Trades:",
    profit_trades: "Profit Trades (% of total):",
    loss_trades: "Loss Trades (% of total):",
    filled: "filled",
};

const RUSSIAN: Words = Words {
    title: "Отчет о торговой истории",
    name: "Имя:",
    account: "Счет:",
    company: "Компания:",
    date: "Дата:",
    positions: "Позиции",
    orders: "Ордера",
    deals: "Сделки",
    results: "Результаты",
    position_head: [
        "Время",
        "Позиция",
        "Символ",
        "Тип",
        "Объем",
        "Цена",
        "S / L",
        "T / P",
        "Время",
        "Цена",
        "Комиссия",
        "Своп",
        "Прибыль",
    ],
    order_head: [
        "Время открытия",
        "Ордер",
        "Символ",
        "Тип",
        "Объем",
        "Цена",
        "S / L",
        "T / P",
        "Время",
        "Состояние",
        "Комментарий",
    ],
    deal_head: [
        "Время",
        "Сделка",
        "Символ",
        "Тип",
        "Направление",
        "Объем",
        "Цена",
        "Ордер",
        "Комиссия",
        "Сбор",
        "Своп",
        "Прибыль",
        "Баланс",
        "Комментарий",
    ],
    net_profit: "Чистая прибыль:",
    gross_profit: "Общая прибыль:",
    gross_loss: "Общий убыток:",
    total_trades: "Всего сделок:",
    profit_trades: "Прибыльных сделок (% от всех):",
    loss_trades: "Убыточных сделок (% от всех):",
    filled: "исполнен",
};

const VIETNAMESE: Words = Words {
    title: "Báo cáo lịch sử giao dịch",
    name: "Tên:",
    account: "Tài khoản:",
    company: "Công ty:",
    date: "Ngày:",
    positions: "Vị thế",
    orders: "Lệnh",
    deals: "Giao dịch",
    results: "Kết quả",
    position_head: [
        "Thời gian",
        "Vị thế",
        "Ký hiệu",
        "Loại",
        "Khối lượng",
        "Giá",
        "S / L",
        "T / P",
        "Thời gian",
        "Giá",
        "Hoa hồng",
        "Swap",
        "Lợi nhuận",
    ],
    order_head: [
        "Thời gian mở",
        "Lệnh",
        "Ký hiệu",
        "Loại",
        "Khối lượng",
        "Giá",
        "S / L",
        "T / P",
        "Thời gian",
        "Trạng thái",
        "Ghi chú",
    ],
    deal_head: [
        "Thời gian",
        "Giao dịch",
        "Ký hiệu",
        "Loại",
        "Hướng",
        "Khối lượng",
        "Giá",
        "Lệnh",
        "Hoa hồng",
        "Phí",
        "Swap",
        "Lợi nhuận",
        "Số dư",
        "Ghi chú",
    ],
    net_profit: "Lợi nhuận ròng:",
    gross_profit: "Tổng lãi:",
    gross_loss: "Tổng lỗ:",
    total_trades: "Tổng số giao dịch:",
    profit_trades: "Giao dịch có lãi (% tổng):",
    loss_trades: "Giao dịch thua lỗ (% tổng):",
    filled: "đã khớp",
};

/// What a cell holds, so the XLSX can type it and the HTML can print it.
enum Value {
    Empty,
    Text(String),
    Int(i64),
    Number { value: f64, text: String },
    Time(String),
}

struct Cell {
    value: Value,
    span: usize,
}

impl Cell {
    fn of(value: Value) -> Self {
        Self { value, span: 1 }
    }

    fn text(text: impl Into<String>) -> Self {
        Self::of(Value::Text(text.into()))
    }

    fn empty() -> Self {
        Self::of(Value::Empty)
    }

    fn int(n: i64) -> Self {
        Self::of(Value::Int(n))
    }

    fn time(at: &str) -> Self {
        Self::of(Value::Time(at.to_owned()))
    }

    /// A money amount: `-1 234.50`.
    fn money(value: f64) -> Self {
        Self::of(Value::Number { value: Self::as_printed(value, 2), text: grouped(value, 2) })
    }

    /// A volume or a price with a fixed number of decimals.
    fn fixed(value: f64, decimals: usize) -> Self {
        Self::of(Value::Number { value: Self::as_printed(value, decimals), text: format!("{value:.decimals$}") })
    }

    /// The number a reader gets from the printed text (a running sum carries float noise).
    fn as_printed(value: f64, decimals: usize) -> f64 {
        format!("{value:.decimals$}").parse().expect("a formatted float parses")
    }

    fn spanning(mut self, span: usize) -> Self {
        self.span = span;
        self
    }

    /// The text the HTML report prints.
    fn printed(&self) -> String {
        match &self.value {
            Value::Empty => String::new(),
            Value::Text(text) | Value::Time(text) | Value::Number { text, .. } => text.clone(),
            Value::Int(n) => n.to_string(),
        }
    }
}

/// `-1 234.50`: thousands grouped with a space, no sign on a zero.
fn grouped(value: f64, decimals: usize) -> String {
    let plain = format!("{:.decimals$}", value.abs());
    let (whole, fraction) = plain.split_once('.').unwrap_or((plain.as_str(), ""));
    let mut out = String::new();
    if value < 0.0 && plain.chars().any(|c| c.is_ascii_digit() && c != '0') {
        out.push('-');
    }
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(digit);
    }
    if !fraction.is_empty() {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Title,
    Label,
    Head,
    Data,
    Spacer,
}

struct Row {
    kind: Kind,
    cells: Vec<Cell>,
}

impl Row {
    fn title(text: &str) -> Self {
        Self { kind: Kind::Title, cells: vec![Cell::text(text).spanning(WIDTH)] }
    }

    /// `Name:` and its value, side by side.
    fn label(label: &str, value: &str) -> Self {
        Self { kind: Kind::Label, cells: vec![Cell::text(label).spanning(3), Cell::text(value).spanning(WIDTH - 3)] }
    }

    fn head(titles: &[&str]) -> Self {
        Self { kind: Kind::Head, cells: titles.iter().map(|t| Cell::text(*t)).collect() }
    }

    fn data(cells: Vec<Cell>) -> Self {
        Self { kind: Kind::Data, cells }
    }

    fn spacer() -> Self {
        Self { kind: Kind::Spacer, cells: vec![Cell::empty().spanning(WIDTH)] }
    }
}

fn decimals(symbol: &str) -> usize {
    if symbol == "XAUUSD" { 2 } else { 5 }
}

/// A stop or target; the terminal prints `0.00000` when none was set.
fn level(symbol: &str, value: f64) -> Cell {
    Cell::fixed(value, decimals(symbol))
}

fn positions_table(words: &Words, rows: &mut Vec<Row>) {
    rows.push(Row::title(words.positions));
    rows.push(Row::head(&words.position_head));
    for c in &CLOSED {
        let places = decimals(c.symbol);
        rows.push(Row::data(vec![
            Cell::time(c.opened_at),
            Cell::int(c.position),
            Cell::text(c.symbol),
            Cell::text(c.kind),
            Cell::fixed(c.volume, 2),
            Cell::fixed(c.open_price, places),
            level(c.symbol, c.sl),
            level(c.symbol, c.tp),
            Cell::time(c.ended_at),
            Cell::fixed(c.close_price, places),
            Cell::money(c.commission),
            Cell::money(c.swap),
            Cell::money(c.profit),
        ]));
    }
    rows.push(Row::spacer());
}

fn orders_table(words: &Words, rows: &mut Vec<Row>) {
    rows.push(Row::title(words.orders));
    rows.push(Row::head(&words.order_head));
    let mut seen: Vec<i64> = Vec::new();
    for e in EVENTS.iter().filter(|e| e.order != 0) {
        // The two deals of a close-by share one order.
        if seen.contains(&e.order) {
            continue;
        }
        seen.push(e.order);
        let closed: Option<&Closed> = CLOSED.iter().find(|c| c.position == e.order);
        let (sl, tp) = closed.map_or((0.0, 0.0), |c| (c.sl, c.tp));
        rows.push(Row::data(vec![
            Cell::time(e.at),
            Cell::int(e.order),
            Cell::text(e.symbol),
            Cell::text(e.kind),
            Cell::text(format!("{:.2} / {:.2}", e.volume, e.volume)),
            Cell::fixed(e.price, decimals(e.symbol)),
            if sl > 0.0 { level(e.symbol, sl) } else { Cell::empty() },
            if tp > 0.0 { level(e.symbol, tp) } else { Cell::empty() },
            Cell::time(e.at),
            Cell::text(words.filled),
            Cell::text(e.comment),
        ]));
    }
    rows.push(Row::spacer());
}

fn deal_cells(e: &Event, balance: f64) -> Vec<Cell> {
    let mut cells = vec![Cell::time(e.at), Cell::int(e.ticket), Cell::text(e.symbol), Cell::text(e.kind)];
    if e.entry.is_empty() {
        // Cash: no direction, volume, price, order, commission, fee or swap.
        cells.push(Cell::empty().spanning(7));
    } else {
        cells.extend([
            Cell::text(e.entry),
            Cell::fixed(e.volume, 2),
            Cell::fixed(e.price, decimals(e.symbol)),
            Cell::int(e.order),
            Cell::money(e.commission),
            Cell::money(e.fee),
            Cell::money(e.swap),
        ]);
    }
    cells.extend([Cell::money(e.profit), Cell::money(balance), Cell::text(e.comment)]);
    cells
}

fn deals_table(words: &Words, rows: &mut Vec<Row>) {
    rows.push(Row::title(words.deals));
    rows.push(Row::head(&words.deal_head));
    let mut balance = 0.0;
    for e in &EVENTS {
        balance += e.profit + e.commission + e.fee + e.swap;
        rows.push(Row::data(deal_cells(e, balance)));
    }
    let sum = |field: fn(&Event) -> f64| EVENTS.iter().map(field).sum::<f64>();
    rows.push(Row::data(vec![
        Cell::empty().spanning(8),
        Cell::money(sum(|e| e.commission)),
        Cell::money(sum(|e| e.fee)),
        Cell::money(sum(|e| e.swap)),
        Cell::money(sum(|e| e.profit)),
        Cell::money(balance),
        Cell::empty(),
    ]));
    rows.push(Row::spacer());
}

fn results_table(words: &Words, rows: &mut Vec<Row>) {
    // The fee of the GBPUSD trade is booked on its exit deal and not in the Positions table.
    let fees: f64 = EVENTS.iter().map(|e| e.fee).sum();
    let nets: Vec<f64> = CLOSED
        .iter()
        .map(|c| c.profit + c.commission + c.swap + if c.position == GBPUSD_POSITION { fees } else { 0.0 })
        .collect();
    let wins: f64 = nets.iter().filter(|n| **n > 0.0).sum();
    let losses: f64 = nets.iter().filter(|n| **n <= 0.0).sum();
    let share = |n: usize| {
        let (n, total) = (u32::try_from(n).expect("a few trades"), u32::try_from(nets.len()).expect("a few trades"));
        format!("{n} ({:.2}%)", f64::from(n) * 100.0 / f64::from(total))
    };
    let won = nets.iter().filter(|n| **n > 0.0).count();
    let lost = nets.len() - won;
    let pair = |label: &str, value: String, label_span: usize| {
        [Cell::text(label).spanning(label_span), Cell::text(value).spanning(2)]
    };
    rows.push(Row::title(words.results));
    for cells in [
        [
            pair(words.net_profit, grouped(TOTAL_NET_PROFIT, 2), 3),
            pair(words.gross_profit, grouped(wins, 2), 3),
            pair(words.gross_loss, grouped(losses, 2), 2),
        ],
        [
            pair(words.total_trades, CLOSED_TRADES.to_string(), 3),
            pair(words.profit_trades, share(won), 3),
            pair(words.loss_trades, share(lost), 2),
        ],
    ] {
        rows.push(Row::data(cells.into_iter().flatten().collect()));
    }
}

fn document(words: &Words) -> Vec<Row> {
    let mut rows = vec![
        Row::title(words.title),
        Row::label(words.name, NAME),
        Row::label(words.account, &format!("{LOGIN} ({CURRENCY}, {SERVER}, demo, Hedge)")),
        Row::label(words.company, COMPANY),
        Row::label(words.date, REPORT_DATE),
        Row::spacer(),
    ];
    positions_table(words, &mut rows);
    orders_table(words, &mut rows);
    deals_table(words, &mut rows);
    results_table(words, &mut rows);
    rows
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// How the markup is written.
#[derive(Clone, Copy)]
enum Markup {
    /// What the terminal writes: lower-case, quoted, closed.
    Terminal,
    /// A hand-edited or re-saved file: upper-case, unquoted, no closing tags.
    Sloppy,
}

fn html(rows: &[Row], words: &Words, markup: Markup) -> String {
    let mut out = String::new();
    match markup {
        Markup::Terminal => {
            let _ = write!(
                out,
                "<html>\r\n<head>\r\n<title>{title}</title>\r\n\
                 <meta http-equiv=\"Content-Type\" content=\"text/html; charset=UTF-16LE\">\r\n\
                 <style type=\"text/css\" media=\"screen\">\r\n\
                 td {{ font: 8pt Tahoma,Arial; }}\r\nth {{ font: 10pt Tahoma,Arial; }}\r\n</style>\r\n</head>\r\n\
                 <body topmargin=1 marginheight=1>\r\n<div align=center>\r\n\
                 <div style=\"font: 20pt Tahoma\"><b>{title}</b></div>\r\n\
                 <table cellspacing=1 cellpadding=3 border=0>\r\n",
                title = escape(words.title),
            );
        }
        Markup::Sloppy => {
            let _ = write!(
                out,
                "<!DOCTYPE html>\n<HTML><HEAD><META CHARSET=utf-8><TITLE>{title}</TITLE>\n\
                 <!-- saved again by hand, closing tags dropped -->\n</HEAD>\n<BODY>\n<TABLE BORDER=0 CELLPADDING=3>\n",
                title = escape(words.title),
            );
        }
    }
    let mut backgrounds = ["#FFFFFF", "#F7F7F7"].into_iter().cycle();
    for row in rows {
        match markup {
            Markup::Terminal => {
                let background = if row.kind == Kind::Data { backgrounds.next() } else { None };
                terminal_row(&mut out, row, background.unwrap_or("#FFFFFF"));
            }
            Markup::Sloppy => sloppy_row(&mut out, row),
        }
    }
    out.push_str(match markup {
        Markup::Terminal => "</table>\r\n</div>\r\n</body>\r\n</html>\r\n",
        Markup::Sloppy => "</TABLE>\n</BODY>\n</HTML>\n",
    });
    out
}

fn span_attr(cell: &Cell, markup: Markup) -> String {
    match (cell.span, markup) {
        (1, _) => String::new(),
        (n, Markup::Terminal) => format!(" colspan={n}"),
        (n, Markup::Sloppy) => format!(" COLSPAN={n}"),
    }
}

fn terminal_row(out: &mut String, row: &Row, background: &str) {
    let _ = match row.kind {
        Kind::Title | Kind::Label => write!(out, "<tr align=left>"),
        Kind::Head => write!(out, "<tr align=center bgcolor=\"#E5F0FC\">"),
        Kind::Data => write!(out, "<tr bgcolor=\"{background}\" align=right>"),
        Kind::Spacer => write!(out, "<tr>"),
    };
    for cell in &row.cells {
        let span = span_attr(cell, Markup::Terminal);
        let text = escape(&cell.printed());
        let _ = match row.kind {
            Kind::Title | Kind::Label => {
                write!(out, "<th{span} nowrap align=left><div style=\"font: 10pt Tahoma\"><b>{text}</b></div></th>")
            }
            Kind::Head => write!(out, "<td{span}><b>{text}</b></td>"),
            Kind::Data | Kind::Spacer if text.is_empty() && cell.span == 1 => write!(out, "<td>&nbsp;</td>"),
            Kind::Data | Kind::Spacer if text.is_empty() => write!(out, "<td{span}></td>"),
            Kind::Data | Kind::Spacer => write!(out, "<td{span} nowrap>{text}</td>"),
        };
    }
    out.push_str("</tr>\r\n");
}

fn sloppy_row(out: &mut String, row: &Row) {
    out.push_str(if row.kind == Kind::Data { "<TR ALIGN=RIGHT>\n" } else { "<TR>\n" });
    for cell in &row.cells {
        let span = span_attr(cell, Markup::Sloppy);
        let text = escape(&cell.printed());
        let _ = match row.kind {
            Kind::Title | Kind::Label => writeln!(out, "<TH{span} ALIGN=LEFT><DIV><B>{text}</B></DIV>"),
            Kind::Head => writeln!(out, "<TD{span}><B>{text}</B><BR>"),
            Kind::Data | Kind::Spacer if text.is_empty() => writeln!(out, "<TD{span}>&nbsp;"),
            Kind::Data | Kind::Spacer => writeln!(out, "<TD{span} NOWRAP>{text}"),
        };
    }
}

/// UTF-16 LE with a byte-order mark, as the terminal saves reports.
fn utf16_le(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    bytes
}

/// Whether the XLSX keeps numbers and times as typed cells or writes everything as text.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cells {
    Typed,
    TextOnly,
}

/// `2026.11.16 09:00:12` → an Excel date-time.
fn excel_time(at: &str) -> Result<ExcelDateTime, Box<dyn Error>> {
    let parts: Vec<u32> = at.split(['.', ' ', ':']).map(str::parse).collect::<Result<_, _>>()?;
    let [year, month, day, hour, minute, second] = parts[..] else { return Err(format!("bad time {at}").into()) };
    Ok(ExcelDateTime::from_ymd(year.try_into()?, month.try_into()?, day.try_into()?)?.and_hms(
        hour.try_into()?,
        minute.try_into()?,
        second,
    )?)
}

fn write_xlsx(rows: &[Row], cells: Cells, path: &Path) -> Result<(), Box<dyn Error>> {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let plain = Format::new();
    let datetime = Format::new().set_num_format("yyyy.mm.dd hh:mm:ss");
    for (r, row) in (0_u32..).zip(rows) {
        let mut col = 0_u16;
        for cell in &row.cells {
            let width = u16::try_from(cell.span)?;
            if cell.span > 1 {
                sheet.merge_range(r, col, r, col + width - 1, &cell.printed(), &plain)?;
            } else {
                match (&cell.value, cells) {
                    (Value::Empty, _) => {}
                    (Value::Int(n), Cells::Typed) => {
                        sheet.write_number(r, col, f64::from(i32::try_from(*n)?))?;
                    }
                    (Value::Number { value, .. }, Cells::Typed) => {
                        sheet.write_number(r, col, *value)?;
                    }
                    (Value::Time(at), Cells::Typed) => {
                        sheet.write_datetime_with_format(r, col, excel_time(at)?, &datetime)?;
                    }
                    _ => {
                        sheet.write_string(r, col, cell.printed())?;
                    }
                }
            }
            col += width;
        }
    }
    workbook.save(path)?;
    Ok(())
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

#[test]
#[ignore = "rewrites tests/fixtures; run it on purpose to regenerate them"]
fn regenerate_fixtures() -> Result<(), Box<dyn Error>> {
    let dir = fixtures_dir();
    fs::create_dir_all(&dir)?;

    let english = document(&ENGLISH);
    fs::write(dir.join("mt5_en_hedge.html"), utf16_le(&html(&english, &ENGLISH, Markup::Terminal)))?;
    write_xlsx(&english, Cells::Typed, &dir.join("mt5_en_hedge.xlsx"))?;
    write_xlsx(&english, Cells::TextOnly, &dir.join("mt5_en_hedge_text.xlsx"))?;

    let russian = document(&RUSSIAN);
    fs::write(dir.join("mt5_ru_hedge.html"), utf16_le(&html(&russian, &RUSSIAN, Markup::Terminal)))?;

    let vietnamese = document(&VIETNAMESE);
    fs::write(dir.join("mt5_vi_hedge.html"), html(&vietnamese, &VIETNAMESE, Markup::Sloppy))?;

    fs::write(dir.join("notion_r_journal.zip"), notion_r_journal_zip()?)?;
    Ok(())
}

/// The CSV of a small journal kept in R, as Notion exports it (UTF-8 with a BOM; the date has no time):
/// five invented trades, six columns that say nothing about money and one that lists the images.
const R_JOURNAL_CSV: &str = "\u{feff}Tên giao dịch,Cặp tiền,Ghi chú,Loại lệnh,Nhóm,Có chờ?,Ngày giao dịch,Đủ điều kiện?,Nhận định,Risk/Reward,Lưu ý,Bối cảnh,Đúng giờ?,Trạng thái,Ảnh\n\
BTC/USD LONG,BTC/USD,Ghi chú mẫu một,Buy (Long),Bitcoin,No,\"April 16, 2026\",Yes,Phân tích mẫu,2,Rủi ro mẫu,Thuận trend,Yes,Thắng,image.png\n\
XAU/USD SHORT,XAU/USD,,Sell (Short),XAU,Yes,\"April 16, 2026\",No,,-1,,Ngược trend,No,Thua,image%201.png\n\
USD/JPY LONG,USD/JPY,,Buy (Long),Forex,No,\"April 15, 2026\",Yes,,0,,Thuận trend,Yes,Hòa,\n\
EUR/USD LONG,EUR/USD,,Buy (Long),Forex,No,\"April 14, 2026\",Yes,,0.6,,Thuận trend,No,Thắng,\"image.png, image%201.png\"\n\
BTC/USD SHORT,BTC/USD,,Sell (Short),Bitcoin,No,\"April 13, 2026\",No,,-0.2,,Ngược trend,Yes,Thua,image%202.png\n";

/// Two different pictures: one pixel each, differing in the colour byte.
const PICTURE_ONE: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7,
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// The archive as Notion writes it: the database's CSV twice (with and without the `_all` suffix: the
/// second has fewer rows), the two pictures at the root, and file names that are not flagged as UTF-8,
/// so a reader that trusts the decoded name finds nothing it expects.
fn notion_r_journal_zip() -> Result<Vec<u8>, Box<dyn Error>> {
    use std::io::{Cursor, Write};
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let one_row: String = R_JOURNAL_CSV.lines().take(2).collect::<Vec<_>>().join("\n");
    let mut other = PICTURE_ONE.to_vec();
    other[41] ^= 0x01;
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in [
        ("Nhat ky giao dich 0000.csv", one_row.as_bytes()),
        ("Nhat ky giao dich 0000_all.csv", R_JOURNAL_CSV.as_bytes()),
        ("image.png", PICTURE_ONE),
        ("image 1.png", other.as_slice()),
    ] {
        zip.start_file(name, options)?;
        zip.write_all(bytes)?;
    }
    let mut archive = zip.finish()?.into_inner();
    // Replace the first "a" of the CSV names by 0x85 (`à` in code page 437): the names are no longer
    // valid UTF-8 and the archive does not say they are UTF-8, as Notion's does not.
    let marker = b"Nhat ky giao dich 0000";
    let mut at = 0;
    while let Some(found) = archive[at..].windows(marker.len()).position(|w| w == marker) {
        archive[at + found + 2] = 0x85;
        at += found + marker.len();
    }
    Ok(archive)
}

/// The scenario's constants are the sums of its rows, so a typo shows up here.
#[test]
fn the_scenario_adds_up() {
    let net: f64 =
        EVENTS.iter().filter(|e| !e.entry.is_empty()).map(|e| e.profit + e.commission + e.fee + e.swap).sum();
    assert!((net - TOTAL_NET_PROFIT).abs() < 1e-9, "net {net}");
    assert!((net + 10_000.0 - 500.0 - FINAL_BALANCE).abs() < 1e-9, "balance {net}");
    assert_eq!(CLOSED.len(), CLOSED_TRADES);
    let closed_net: f64 = CLOSED.iter().map(|c| c.profit + c.commission + c.swap).sum();
    let fees: f64 = EVENTS.iter().map(|e| e.fee).sum();
    assert!((closed_net + fees - TOTAL_NET_PROFIT).abs() < 1e-9, "closed net {closed_net}");
    for c in &CLOSED {
        let volume_in: f64 =
            EVENTS.iter().filter(|e| e.position == c.position && e.entry == "in").map(|e| e.volume).sum();
        assert!((volume_in - c.volume).abs() < 1e-9, "position {}", c.position);
    }
}
