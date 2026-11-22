//! Single cells of a Notion export: names folded for comparison, dates in the formats
//! Notion writes, numbers with currency symbols and separators, buy/sell words,
//! checkboxes and lists.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

use crate::model::Side;

/// Which of two numbers in `20/11/2026` is the day when nothing in the file says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DateOrder {
    DayFirst,
    MonthFirst,
}

/// What a lone comma in a number means: `1,234` is a thousand in a comma-delimited
/// file, `1,5` is one and a half in a semicolon-delimited one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CommaRole {
    Thousands,
    Decimal,
}

/// How the file writes dates and numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Locale {
    pub(super) dates: DateOrder,
    pub(super) comma: CommaRole,
}

/// Lowercase, without diacritics, with every run of other characters as one space:
/// `Lãi/lỗ` and `lai lo` fold alike.
pub(super) fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.nfd().filter(|&c| !is_combining_mark(c)) {
        let c = if matches!(c, 'đ' | 'Đ') { 'd' } else { c };
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// A column header as compared with names: units in brackets (`P&L ($)`) are dropped.
pub(super) fn header_key(header: &str) -> String {
    let mut depth = 0_u32;
    let visible: String = header
        .chars()
        .filter(|&c| match c {
            '(' | '[' => {
                depth += 1;
                false
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect();
    fold(&visible)
}

/// A key made of a name: folded, with `_` between words; never empty.
pub(super) fn slug(text: &str) -> String {
    let key = fold(text).replace(' ', "_");
    if key.is_empty() { "field".to_owned() } else { key }
}

/// A stable 64-bit hash (FNV-1a), so that ids made of it never change between versions.
pub(super) fn stable_hash(parts: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain(std::iter::once(0x1f)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

// --- dates -------------------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const WEEKDAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

const FILLERS: [&str; 9] = ["thang", "ngay", "nam", "at", "luc", "vao", "on", "of", "the"];

/// A date range as Notion writes it (`start → end`): the start and, when present, the end.
pub(super) fn split_range(raw: &str) -> (&str, Option<&str>) {
    for separator in ["→", "->"] {
        if let Some((start, end)) = raw.split_once(separator) {
            return (start.trim(), Some(end.trim()));
        }
    }
    (raw.trim(), None)
}

/// The moment a cell names; for a range, its start. A date alone is midnight.
pub(super) fn parse_datetime(raw: &str, order: DateOrder) -> Option<NaiveDateTime> {
    parse_moment(split_range(raw).0, order)
}

/// The end of a date range, when the cell is one.
pub(super) fn parse_range_end(raw: &str, order: DateOrder) -> Option<NaiveDateTime> {
    split_range(raw).1.and_then(|end| parse_moment(end, order))
}

fn parse_moment(text: &str, order: DateOrder) -> Option<NaiveDateTime> {
    let text = strip_zone_note(text.trim());
    if let Ok(moment) = DateTime::parse_from_rfc3339(text) {
        return Some(moment.naive_local());
    }
    let spaced = text.replace(',', " ");
    let mut tokens: Vec<String> = Vec::new();
    for token in spaced.split_whitespace() {
        match split_iso(token) {
            Some((date, time)) => tokens.extend([date.to_owned(), time.to_owned()]),
            None => tokens.push(token.to_owned()),
        }
    }
    let mut time: Option<(u32, u32, u32)> = None;
    let mut meridiem: Option<Meridiem> = None;
    let mut date_tokens: Vec<&str> = Vec::new();
    for token in &tokens {
        if token.contains(':') {
            if time.is_some() {
                return None;
            }
            let (clock, suffix) = parse_clock(token)?;
            time = Some(clock);
            meridiem = meridiem.or(suffix);
        } else if let Some(m) = Meridiem::of(token).filter(|_| time.is_some()) {
            meridiem = Some(m);
        } else {
            date_tokens.push(token);
        }
    }
    let date = parse_date_tokens(&date_tokens, order)?;
    let (mut hour, minute, second) = time.unwrap_or_default();
    match meridiem {
        Some(Meridiem::Pm) if hour < 12 => hour += 12,
        Some(Meridiem::Am) if hour == 12 => hour = 0,
        _ => {}
    }
    Some(date.and_time(NaiveTime::from_hms_opt(hour, minute, second)?))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Meridiem {
    Am,
    Pm,
}

impl Meridiem {
    fn of(token: &str) -> Option<Self> {
        match fold(token).as_str() {
            "am" | "sa" => Some(Self::Am),
            "pm" | "ch" => Some(Self::Pm),
            _ => None,
        }
    }
}

/// `(GMT+7)` after a date is a note about the zone; the wall clock stays as written.
fn strip_zone_note(text: &str) -> &str {
    match text.rfind('(') {
        Some(at) if text.ends_with(')') => text[..at].trim_end(),
        _ => text,
    }
}

/// `2026-11-20T09:30:00` into its date and time.
fn split_iso(token: &str) -> Option<(&str, &str)> {
    let at = token.find('T')?;
    let (date, time) = (&token[..at], &token[at + 1..]);
    (date.ends_with(|c: char| c.is_ascii_digit()) && time.starts_with(|c: char| c.is_ascii_digit()))
        .then_some((date, time))
}

/// `9:30`, `09:30:15`, `9:30PM`, `09:30:00Z`, `09:30:00+07:00`.
fn parse_clock(token: &str) -> Option<((u32, u32, u32), Option<Meridiem>)> {
    let token = token.split('+').next()?;
    let letters_at = token.find(|c: char| c.is_alphabetic()).unwrap_or(token.len());
    let (clock, suffix) = token.split_at(letters_at);
    let meridiem = match suffix {
        "" | "Z" | "z" => None,
        other => Some(Meridiem::of(other)?),
    };
    let mut parts = clock.split(':');
    let hour: u32 = parts.next()?.parse().ok()?;
    let minute: u32 = parts.next()?.parse().ok()?;
    let second: u32 = match parts.next() {
        Some(seconds) => seconds.split('.').next()?.parse().ok()?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some(((hour, minute, second), meridiem))
}

fn parse_date_tokens(tokens: &[&str], order: DateOrder) -> Option<NaiveDate> {
    match tokens {
        [single] if single.contains(['/', '-', '.']) => parse_numeric_date(single, order),
        _ => parse_worded_date(tokens),
    }
}

/// `2026-11-20`, `20/11/2026`, `11/20/2026`, `20.11.26`.
fn parse_numeric_date(token: &str, order: DateOrder) -> Option<NaiveDate> {
    let parts: Vec<&str> = token.split(['/', '-', '.']).collect();
    let [a, b, c] = parts.as_slice() else {
        return None;
    };
    if ![a, b, c].iter().all(|p| !p.is_empty() && p.bytes().all(|byte| byte.is_ascii_digit())) {
        return None;
    }
    if a.len() == 4 {
        return NaiveDate::from_ymd_opt(a.parse().ok()?, b.parse().ok()?, c.parse().ok()?);
    }
    let year: i32 = match c.len() {
        4 => c.parse().ok()?,
        2 => 2000 + c.parse::<i32>().ok()?,
        _ => return None,
    };
    let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
    let (day, month) = if a > 12 {
        (a, b)
    } else if b > 12 {
        (b, a)
    } else if order == DateOrder::DayFirst {
        (a, b)
    } else {
        (b, a)
    };
    NaiveDate::from_ymd_opt(year, month, day)
}

/// `November 20 2026`, `Nov 20th 2026`, `20 November 2026`, `20 tháng 11 2026`.
fn parse_worded_date(tokens: &[&str]) -> Option<NaiveDate> {
    let mut month: Option<u32> = None;
    let mut numbers: Vec<(u32, usize)> = Vec::new();
    for token in tokens {
        let word = fold(token);
        if word.is_empty() {
            continue;
        }
        if let Some(found) = month_of(&word) {
            if month.replace(found).is_some() {
                return None;
            }
        } else if !is_filler(&word) {
            let digits = word.trim_end_matches(|c: char| c.is_alphabetic());
            if !matches!(&word[digits.len()..], "" | "st" | "nd" | "rd" | "th") {
                return None;
            }
            numbers.push((digits.parse().ok()?, digits.len()));
        }
    }
    let year_of = |value: u32| i32::try_from(value).ok();
    match (month, numbers.as_slice()) {
        (Some(month), [(first, first_len), (second, second_len)]) => {
            let (day, year) = if *first_len == 4 { (*second, *first) } else { (*first, *second) };
            if (*first_len == 4) == (*second_len == 4) {
                return None;
            }
            NaiveDate::from_ymd_opt(year_of(year)?, month, day)
        }
        (None, [(first, first_len), (second, _), (third, _)]) => {
            if *first_len == 4 {
                NaiveDate::from_ymd_opt(year_of(*first)?, *second, *third)
            } else {
                NaiveDate::from_ymd_opt(year_of(*third)?, *second, *first)
            }
        }
        _ => None,
    }
}

fn month_of(word: &str) -> Option<u32> {
    if word.len() < 3 {
        return None;
    }
    let index = MONTHS.iter().position(|month| month.starts_with(word))?;
    u32::try_from(index + 1).ok()
}

fn is_filler(word: &str) -> bool {
    FILLERS.contains(&word) || (word.len() >= 3 && WEEKDAYS.iter().any(|day| day.starts_with(word)))
}

/// Guesses which number of `20/11/2026` is the day from every cell of the file: a first
/// part above 12 can only be a day, a second part above 12 only a day too.
pub(super) fn detect_date_order<'a>(cells: impl IntoIterator<Item = &'a str>) -> DateOrder {
    let mut order = DateOrder::DayFirst;
    for cell in cells {
        let first = split_range(cell).0;
        let parts: Vec<&str> = first.split(['/', '-', '.']).collect();
        if let [a, b, c] = parts.as_slice()
            && a.len() <= 2
            && (c.len() == 2 || c.len() == 4 || c.contains(' '))
            && let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>())
        {
            if a > 12 {
                return DateOrder::DayFirst;
            }
            if b > 12 {
                order = DateOrder::MonthFirst;
            }
        }
    }
    order
}

// --- numbers -----------------------------------------------------------------------

/// A number with its currency symbol, unit or separators: `$1,234.50`, `1.234,50 €`,
/// `-12`, `(45.5)`, `1 234,5`, `0.5%`. `None` for anything else, dates and codes like `M15` included.
pub(super) fn parse_number(raw: &str, comma: CommaRole) -> Option<f64> {
    let joined = number_words(raw)?;
    let mut negative = joined.starts_with('(') && joined.ends_with(')');
    let kept: String = joined
        .chars()
        .filter(|c| !matches!(c, '\'' | '’' | '$' | '€' | '£' | '¥' | '₫' | '₹' | '%'))
        .map(|c| if c == '−' { '-' } else { c })
        .collect();
    let mut core = kept.trim_matches(|c: char| c == '(' || c == ')');
    if let Some(rest) = core.strip_prefix('-') {
        negative = !negative;
        core = rest;
    } else if let Some(rest) = core.strip_prefix('+') {
        core = rest;
    }
    if let Some(rest) = core.strip_suffix('-') {
        negative = true;
        core = rest;
    }
    if !core.bytes().any(|b| b.is_ascii_digit()) || !core.bytes().all(|b| b.is_ascii_digit() || b == b'.' || b == b',')
    {
        return None;
    }
    let plain = plain_digits(core, comma)?;
    let value: f64 = plain.parse().ok()?;
    value.is_finite().then_some(if negative { -value } else { value })
}

/// The words of a cell that make up the number, joined. A unit or currency set apart by a space
/// (`125 USD`, `$ 40`) is dropped; other spaces only group thousands (`1 234,5`), so `Nov 20 2026`
/// and `May 20, 2026` are not numbers.
fn number_words(text: &str) -> Option<String> {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    let is_unit = |word: &str| {
        word.chars().all(|c| matches!(c, '$' | '€' | '£' | '¥' | '₫' | '₹' | '%'))
            || (word.chars().all(char::is_alphabetic) && word.chars().count() <= 4)
    };
    if words.len() > 1 && words.first().is_some_and(|w| is_unit(w)) {
        words.remove(0);
    }
    if words.len() > 1 && words.last().is_some_and(|w| is_unit(w)) {
        words.pop();
    }
    if words.windows(2).any(|pair| pair[0].ends_with(['.', ',']) || pair[1].starts_with(['.', ','])) {
        return None;
    }
    let leading_digits = |word: &str| word.chars().take_while(char::is_ascii_digit).count();
    if words.iter().skip(1).any(|word| leading_digits(word) != 3) {
        return None;
    }
    Some(words.concat())
}

/// `1,234.50` and `1.234,50` as `1234.50`; `None` when the separators make no sense.
fn plain_digits(core: &str, comma: CommaRole) -> Option<String> {
    let dots = core.matches('.').count();
    let commas = core.matches(',').count();
    match (dots, commas) {
        (0, 0) => Some(core.to_owned()),
        (1, 0) => {
            let (before, after) = core.split_once('.')?;
            if comma == CommaRole::Decimal && is_thousand_split(before, after) {
                Some(format!("{before}{after}"))
            } else {
                Some(core.to_owned())
            }
        }
        (_, 0) => grouped(core, '.').map(|digits| digits.concat()),
        (0, 1) => {
            let (before, after) = core.split_once(',')?;
            if comma == CommaRole::Thousands && is_thousand_split(before, after) {
                Some(format!("{before}{after}"))
            } else {
                Some(format!("{before}.{after}"))
            }
        }
        (0, _) => grouped(core, ',').map(|digits| digits.concat()),
        _ => {
            // Both: the separator that comes last is the decimal point.
            let at = core.rfind(['.', ','])?;
            let (whole, fraction) = (&core[..at], &core[at + 1..]);
            let group_separator = if core.as_bytes()[at] == b'.' { ',' } else { '.' };
            if whole.contains(char::from(core.as_bytes()[at])) {
                return None;
            }
            let whole = grouped(whole, group_separator)?.concat();
            Some(format!("{whole}.{fraction}"))
        }
    }
}

/// `1,234` is a thousand split when it has a short non-zero head and three digits after.
fn is_thousand_split(before: &str, after: &str) -> bool {
    after.len() == 3 && (1..=3).contains(&before.len()) && !before.starts_with('0')
}

/// `1.234.567` into `["1", "234", "567"]`; `None` unless every group after the first has three digits.
fn grouped(text: &str, separator: char) -> Option<Vec<&str>> {
    let groups: Vec<&str> = text.split(separator).collect();
    let (head, tail) = groups.split_first()?;
    ((1..=3).contains(&head.len()) && tail.iter().all(|g| g.len() == 3)).then_some(groups)
}

/// An execution rating: 0 to 5, as a number (`4`, `4/5`) or as stars (`⭐⭐⭐`).
pub(super) fn parse_rating(raw: &str) -> Option<u8> {
    let text = raw.trim();
    let stars = text.chars().filter(|c| matches!(c, '⭐' | '★' | '🌟')).count();
    if stars > 0 {
        return u8::try_from(stars.min(5)).ok();
    }
    let number = text.split_once('/').map_or(text, |(top, _)| top);
    let value = parse_number(number, CommaRole::Decimal)?;
    let whole = value.round();
    if (value - whole).abs() < 1e-9 && (0.0..=5.0).contains(&whole) { whole.to_string().parse().ok() } else { None }
}

// --- words -------------------------------------------------------------------------

/// Buy, long, mua (and sell, short, bán), whatever surrounds the word (`🟢 Long`, `Buy Limit`).
pub(super) fn parse_side(raw: &str) -> Option<Side> {
    let word = fold(raw);
    let first = word.split(' ').next()?;
    match first {
        "long" | "buy" | "mua" => Some(Side::Long),
        "short" | "sell" | "ban" => Some(Side::Short),
        _ => None,
    }
}

/// Notion's checkbox in a CSV (`Yes`, `No`) and its look-alikes.
pub(super) fn parse_bool(raw: &str) -> Option<bool> {
    let text = raw.trim();
    if text.chars().any(|c| matches!(c, '✓' | '✔' | '☑' | '✅')) {
        return Some(true);
    }
    if text.chars().any(|c| matches!(c, '✗' | '✘' | '☐' | '❌' | '✖')) {
        return Some(false);
    }
    match fold(text).as_str() {
        "yes" | "true" | "checked" | "co" | "x" => Some(true),
        "no" | "false" | "unchecked" | "khong" => Some(false),
        _ => None,
    }
}

/// A multi-select cell (`a, b`) as its items.
pub(super) fn split_list(raw: &str) -> Vec<String> {
    raw.split(',').map(str::trim).filter(|item| !item.is_empty()).map(str::to_owned).collect()
}

pub(super) fn is_url(raw: &str) -> bool {
    let text = raw.trim().to_ascii_lowercase();
    (text.starts_with("http://") || text.starts_with("https://") || text.starts_with("www.")) && !text.contains(' ')
}

/// The files of a Notion files cell: `name.png (https://…), other.png` as the links or names.
pub(super) fn split_files(raw: &str) -> Vec<String> {
    split_list(raw)
        .into_iter()
        .map(|item| match item.find("(http") {
            Some(at) => item[at + 1..].trim_end_matches(')').to_owned(),
            None => item,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moment(text: &str, order: DateOrder) -> Option<String> {
        parse_datetime(text, order).map(|m| m.format("%Y-%m-%d %H:%M:%S").to_string())
    }

    #[test]
    fn folds_names_across_languages() {
        assert_eq!(fold("Lãi/lỗ"), "lai lo");
        assert_eq!(fold("  P&L  "), "p l");
        assert_eq!(header_key("Đã xem lại (Y/N)"), "da xem lai");
        assert_eq!(slug("Khung thời gian"), "khung_thoi_gian");
    }

    #[test]
    fn reads_the_dates_notion_writes() {
        let d = DateOrder::DayFirst;
        for (text, want) in [
            ("November 20, 2026 9:30 AM", "2026-11-20 09:30:00"),
            ("November 20, 2026 9:30 PM", "2026-11-20 21:30:00"),
            ("November 20, 2026 12:05 AM", "2026-11-20 00:05:00"),
            ("Nov 20, 2026", "2026-11-20 00:00:00"),
            ("20 November 2026 14:05", "2026-11-20 14:05:00"),
            ("November 20th, 2026 9:30PM", "2026-11-20 21:30:00"),
            ("2026-11-20", "2026-11-20 00:00:00"),
            ("2026-11-20 09:30", "2026-11-20 09:30:00"),
            ("2026-11-20T09:30:15", "2026-11-20 09:30:15"),
            ("2026-11-20T09:30:15+07:00", "2026-11-20 09:30:15"),
            ("2026/11/20 09:30", "2026-11-20 09:30:00"),
            ("2026.11.20 09:30:15", "2026-11-20 09:30:15"),
            ("20/11/2026 09:30", "2026-11-20 09:30:00"),
            ("20.11.2026", "2026-11-20 00:00:00"),
            ("05/11/2026", "2026-11-05 00:00:00"),
            ("20 tháng 11, 2026", "2026-11-20 00:00:00"),
            ("November 20, 2026 9:30 AM (GMT+7)", "2026-11-20 09:30:00"),
            ("November 20, 2026 9:30 AM → November 20, 2026 11:00 AM", "2026-11-20 09:30:00"),
        ] {
            assert_eq!(moment(text, d).as_deref(), Some(want), "{text}");
        }
        assert_eq!(moment("11/20/2026", d).as_deref(), Some("2026-11-20 00:00:00"), "a month above 12 is a day");
        assert_eq!(moment("05/11/2026", DateOrder::MonthFirst).as_deref(), Some("2026-05-11 00:00:00"));
        for text in ["", "hello", "12", "1.085", "13/13/2026", "2026", "Long"] {
            assert_eq!(moment(text, d), None, "{text}");
        }
        assert_eq!(
            parse_range_end("November 20, 2026 9:30 AM → November 20, 2026 11:00 AM", d).map(|m| m.to_string()),
            Some("2026-11-20 11:00:00".to_owned())
        );
    }

    #[test]
    fn detects_which_part_is_the_day() {
        assert_eq!(detect_date_order(["05/11/2026", "20/11/2026"]), DateOrder::DayFirst);
        assert_eq!(detect_date_order(["05/11/2026", "11/20/2026"]), DateOrder::MonthFirst);
        assert_eq!(detect_date_order(["05/11/2026"]), DateOrder::DayFirst);
        assert_eq!(detect_date_order(["November 20, 2026"]), DateOrder::DayFirst);
    }

    #[test]
    fn reads_numbers_in_every_style() {
        use CommaRole::{Decimal, Thousands};
        for (text, comma, want) in [
            ("12", Thousands, 12.0),
            ("-12.5", Thousands, -12.5),
            ("+7", Thousands, 7.0),
            ("$1,234.50", Thousands, 1234.5),
            ("-$1,234.50", Thousands, -1234.5),
            ("1,234", Thousands, 1234.0),
            ("1,234,567", Thousands, 1_234_567.0),
            ("1.234,50", Decimal, 1234.5),
            ("1.234.567,5", Decimal, 1_234_567.5),
            ("1,5", Decimal, 1.5),
            ("1,5", Thousands, 1.5),
            ("1,085", Decimal, 1.085),
            ("0,085", Thousands, 0.085),
            ("1 234,5", Decimal, 1234.5),
            ("1.234", Decimal, 1234.0),
            ("1.085", Thousands, 1.085),
            ("(45.5)", Thousands, -45.5),
            ("−3", Thousands, -3.0),
            ("125 USD", Thousands, 125.0),
            ("₫1,500,000", Thousands, 1_500_000.0),
            ("0.5%", Thousands, 0.5),
            ("2650.25", Thousands, 2650.25),
        ] {
            let got = parse_number(text, comma).unwrap_or(f64::NAN);
            assert!((got - want).abs() < 1e-9, "{text}: got {got}, want {want}");
        }
        for text in [
            "",
            "abc",
            "-",
            "12abc34",
            "20.11.2026",
            "2026-11-20",
            "1,23,456",
            "9:30",
            "1.2.3",
            "M15",
            "H4",
            "May 20, 2026",
            "Nov 20 2026",
        ] {
            assert_eq!(parse_number(text, CommaRole::Thousands), None, "{text}");
        }
    }

    #[test]
    fn reads_words_and_lists() {
        assert_eq!(parse_side("Long"), Some(Side::Long));
        assert_eq!(parse_side("🟢 Buy Limit"), Some(Side::Long));
        assert_eq!(parse_side("Bán"), Some(Side::Short));
        assert_eq!(parse_side("SELL"), Some(Side::Short));
        assert_eq!(parse_side("EURUSD"), None);
        assert_eq!(parse_bool("Yes"), Some(true));
        assert_eq!(parse_bool("No"), Some(false));
        assert_eq!(parse_bool("✓"), Some(true));
        assert_eq!(parse_bool("Không"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
        assert_eq!(parse_rating("4"), Some(4));
        assert_eq!(parse_rating("⭐⭐⭐"), Some(3));
        assert_eq!(parse_rating("3/5"), Some(3));
        assert_eq!(parse_rating("7"), None);
        assert_eq!(parse_rating("3.5"), None);
        assert_eq!(split_list("a, b ,, c"), ["a", "b", "c"]);
        assert_eq!(
            split_files("chart.png (https://x.io/a.png?v=1), other.png"),
            ["https://x.io/a.png?v=1", "other.png"]
        );
        assert!(is_url("https://www.tradingview.com/x/abc/"));
        assert!(!is_url("see https://x.io"));
    }

    #[test]
    fn hashes_are_stable() {
        assert_eq!(stable_hash(&["a", "b"]), stable_hash(&["a", "b"]));
        assert_ne!(stable_hash(&["a", "b"]), stable_hash(&["ab", ""]));
        assert_eq!(stable_hash(&[]), 0xcbf2_9ce4_8422_2325);
    }
}
