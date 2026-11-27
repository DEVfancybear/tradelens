//! Text for numbers and times, the way MT5 shows them: a dot for decimals, a comma
//! between thousands, a real minus sign. Pure functions, so the pages stay declarative.

use chrono::{NaiveDateTime, Weekday};
use nsh_core::t;

/// The minus sign (U+2212), as the design shows losses.
const MINUS: char = '\u{2212}';

/// What a signed amount looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sign {
    /// `+` for gains, a minus for losses, nothing for zero.
    Always,
    /// Only a minus for losses.
    OnlyNegative,
}

/// The digits of `value` with `decimals` places and commas between the thousands.
fn grouped(value: f64, decimals: usize) -> String {
    let fixed = format!("{:.*}", decimals, value.abs());
    let (whole, fraction) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
    let mut out = String::with_capacity(fixed.len() + whole.len() / 3);
    for (i, digit) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if !fraction.is_empty() {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

/// Whether `value` rounds to a non-zero amount at `decimals` places.
fn is_nonzero(value: f64, decimals: usize) -> bool {
    format!("{:.*}", decimals, value.abs()).chars().any(|c| matches!(c, '1'..='9'))
}

/// An amount of money: `+1,126.40`, `−116.35`. The currency follows as a code unless it is
/// USD (shown as `$` before the digits) or unknown.
pub(crate) fn money(value: f64, sign: Sign, currency: &str) -> String {
    let negative = value < 0.0 && is_nonzero(value, 2);
    let lead = match (negative, sign) {
        (true, _) => MINUS.to_string(),
        (false, Sign::Always) if is_nonzero(value, 2) => "+".to_owned(),
        (false, _) => String::new(),
    };
    let digits = grouped(value, 2);
    match currency {
        "" | "USD" => format!("{lead}${digits}"),
        code => format!("{lead}{digits} {code}"),
    }
}

/// [`money`] rounded to whole units, for the small cells of a calendar: `+$330`, `−$1,249`.
pub(crate) fn money_whole(value: f64, sign: Sign, currency: &str) -> String {
    let negative = value < 0.0 && is_nonzero(value, 0);
    let lead = match (negative, sign) {
        (true, _) => MINUS.to_string(),
        (false, Sign::Always) if is_nonzero(value, 0) => "+".to_owned(),
        (false, _) => String::new(),
    };
    let digits = grouped(value, 0);
    match currency {
        "" | "USD" => format!("{lead}${digits}"),
        code => format!("{lead}{digits} {code}"),
    }
}

/// A result in R: `+1.8R`, `−1.2R`, or a dash when the trade has no stop to measure by.
pub(crate) fn r_text(r: Option<f64>) -> String {
    let Some(r) = r else { return "—".to_owned() };
    let lead = if r < 0.0 && is_nonzero(r, 1) {
        MINUS.to_string()
    } else if r > 0.0 && is_nonzero(r, 1) {
        "+".to_owned()
    } else {
        String::new()
    };
    format!("{lead}{:.1}R", r.abs())
}

/// An average result in R with two decimals: `+0.42R`.
pub(crate) fn r_average(r: Option<f64>) -> String {
    let Some(r) = r else { return "—".to_owned() };
    let lead = if r < 0.0 && is_nonzero(r, 2) {
        MINUS.to_string()
    } else if r > 0.0 && is_nonzero(r, 2) {
        "+".to_owned()
    } else {
        String::new()
    };
    format!("{lead}{:.2}R", r.abs())
}

/// A share between 0 and 1 as a percentage with one decimal: `58.3%`.
pub(crate) fn percent(share: f64) -> String {
    format!("{:.1}%", share * 100.0)
}

/// A volume in lots: `0.20`.
pub(crate) fn lots(volume: f64) -> String {
    format!("{volume:.2}")
}

/// The decimals `value` has, up to five: `1.0850` has 3, `1.08505` has 5, `2648.2` has 1.
pub(crate) fn decimals(value: f64) -> usize {
    let text = format!("{value:.5}");
    text.trim_end_matches('0').split_once('.').map_or(0, |(_, fraction)| fraction.len())
}

/// A price with the decimals it has, at least two: `2650.25`, `1.08123`. A symbol's prices side
/// by side use [`price_to`] with the symbol's decimals, so that they line up.
pub(crate) fn price(value: f64) -> String {
    price_to(value, decimals(value).max(2))
}

/// A price with exactly `digits` decimals: `1.08500`, `2648.20`.
pub(crate) fn price_to(value: f64, digits: usize) -> String {
    format!("{value:.digits$}")
}

/// [`price_to`] where `0` means "not set" (stop loss, take profit): a dash for it.
pub(crate) fn level_to(value: f64, digits: usize) -> String {
    if value > 0.0 { price_to(value, digits) } else { "—".to_owned() }
}

/// The profit factor from which [`ratio`] stops giving digits: one tiny loss among many wins makes it
/// huge and meaningless (`1240.00`), and the figure must fit its column.
const RATIO_CAP: f64 = 50.0;

/// A profit factor: `1.86`; `50+` from 50 up; a dash when nothing was lost yet.
pub(crate) fn ratio(value: Option<f64>) -> String {
    value
        .map_or_else(|| "—".to_owned(), |v| if v >= RATIO_CAP { format!("{RATIO_CAP:.0}+") } else { format!("{v:.2}") })
}

/// Day and month with the time: `20/11 22:44`.
pub(crate) fn short_stamp(at: NaiveDateTime) -> String {
    at.format("%d/%m %H:%M").to_string()
}

/// Day and month only, for a trade whose time of day is unknown: `20/11`.
pub(crate) fn short_day(at: NaiveDateTime) -> String {
    at.format("%d/%m").to_string()
}

/// Day, month, year and time with seconds: `20/11/2026 14:58:12`.
pub(crate) fn full_stamp(at: NaiveDateTime) -> String {
    at.format("%d/%m/%Y %H:%M:%S").to_string()
}

/// Whether a day's list of trades has a column of money beside its column of R.
///
/// A list has one when any of its trades has money; one made of trades that were only kept in R
/// has the R column alone, so the same figure is never printed twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoneyColumn {
    Shown,
    Dropped,
}

impl From<bool> for MoneyColumn {
    /// The column a list needs, from whether any of its trades has money.
    fn from(any_money: bool) -> Self {
        if any_money { Self::Shown } else { Self::Dropped }
    }
}

/// Whether a trade has closed, so that it has a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Position {
    Closed,
    Open,
}

impl From<bool> for Position {
    /// From whether the trade has closed.
    fn from(closed: bool) -> Self {
        if closed { Self::Closed } else { Self::Open }
    }
}

/// What a trade of a day's list says in its result columns: its R, and its money when the list has
/// a money column ([`MoneyColumn`]). A trade without money shows a dash where its money would be,
/// never its R.
///
/// `money` is the formatted money of the trade, `None` when it has none.
pub(crate) fn result_cells(
    position: Position,
    money: Option<String>,
    r: Option<f64>,
    column: MoneyColumn,
) -> (String, Option<String>) {
    if position == Position::Open {
        // An open position has nothing to give in R, so the word stands in the last column.
        let open = t!("format.open");
        return match column {
            MoneyColumn::Shown => ("—".to_owned(), Some(open)),
            MoneyColumn::Dropped => (open, None),
        };
    }
    let money = (column == MoneyColumn::Shown).then(|| money.unwrap_or_else(|| "—".to_owned()));
    (r_text(r), money)
}

/// The name of a weekday: `Thứ 7`, `Chủ nhật` (`Saturday`, `Sunday`).
pub(crate) fn weekday_name(day: Weekday) -> String {
    match day {
        Weekday::Mon => t!("format.weekday.mon"),
        Weekday::Tue => t!("format.weekday.tue"),
        Weekday::Wed => t!("format.weekday.wed"),
        Weekday::Thu => t!("format.weekday.thu"),
        Weekday::Fri => t!("format.weekday.fri"),
        Weekday::Sat => t!("format.weekday.sat"),
        Weekday::Sun => t!("format.weekday.sun"),
    }
}

/// A weekday as a column head asks for it: `T7`, `CN` (`Sat`, `Sun`).
pub(crate) fn weekday_short(day: Weekday) -> String {
    match day {
        Weekday::Mon => t!("format.weekday_short.mon"),
        Weekday::Tue => t!("format.weekday_short.tue"),
        Weekday::Wed => t!("format.weekday_short.wed"),
        Weekday::Thu => t!("format.weekday_short.thu"),
        Weekday::Fri => t!("format.weekday_short.fri"),
        Weekday::Sat => t!("format.weekday_short.sat"),
        Weekday::Sun => t!("format.weekday_short.sun"),
    }
}

/// Keys of the months' names, January first.
const MONTH_KEYS: [&str; 12] = [
    "format.month.1",
    "format.month.2",
    "format.month.3",
    "format.month.4",
    "format.month.5",
    "format.month.6",
    "format.month.7",
    "format.month.8",
    "format.month.9",
    "format.month.10",
    "format.month.11",
    "format.month.12",
];

/// Keys of the months' short names, January first.
const MONTH_SHORT_KEYS: [&str; 12] = [
    "format.month_short.1",
    "format.month_short.2",
    "format.month_short.3",
    "format.month_short.4",
    "format.month_short.5",
    "format.month_short.6",
    "format.month_short.7",
    "format.month_short.8",
    "format.month_short.9",
    "format.month_short.10",
    "format.month_short.11",
    "format.month_short.12",
];

/// The name of a month (1 to 12): `Tháng 11` (`November`); empty for a number that is no month.
pub(crate) fn month_name(month: u32) -> String {
    month_key(&MONTH_KEYS, month).map_or_else(String::new, |key| t!(key))
}

/// The short name of a month (1 to 12): `Th 11` (`Nov`); empty for a number that is no month.
pub(crate) fn month_short(month: u32) -> String {
    month_key(&MONTH_SHORT_KEYS, month).map_or_else(String::new, |key| t!(key))
}

/// The key of month `month` (1 to 12) in `keys`.
fn month_key(keys: &[&'static str; 12], month: u32) -> Option<&'static str> {
    let index = usize::try_from(month.checked_sub(1)?).ok()?;
    keys.get(index).copied()
}

/// A number of trades: `1 trade`, `12 trades` (in Vietnamese the count is the same either way).
pub(crate) fn trade_count(count: usize) -> String {
    if count == 1 { t!("format.trade_count.one", n = count) } else { t!("format.trade_count.other", n = count) }
}

/// Whether a displayed amount is a loss (starts with a minus sign).
pub(crate) fn is_loss(text: &str) -> bool {
    text.starts_with(MINUS) || text.starts_with('-')
}

/// Whether a displayed amount is a gain (starts with a plus sign).
pub(crate) fn is_gain(text: &str) -> bool {
    text.starts_with('+')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_groups_thousands_and_signs() {
        assert_eq!(money(1126.4, Sign::Always, "USD"), "+$1,126.40");
        assert_eq!(money(-116.349, Sign::Always, ""), "\u{2212}$116.35");
        assert_eq!(money(1_234_567.0, Sign::OnlyNegative, "USD"), "$1,234,567.00");
        assert_eq!(money(-5.0, Sign::OnlyNegative, "EUR"), "\u{2212}5.00 EUR");
    }

    #[test]
    fn whole_money_drops_the_cents() {
        assert_eq!(money_whole(330.4, Sign::Always, "USD"), "+$330");
        assert_eq!(money_whole(-1249.5, Sign::Always, ""), "\u{2212}$1,250");
        assert_eq!(money_whole(0.4, Sign::Always, "USD"), "$0");
        assert_eq!(money_whole(-12.0, Sign::OnlyNegative, "EUR"), "\u{2212}12 EUR");
    }

    #[test]
    fn money_that_rounds_to_zero_has_no_sign() {
        assert_eq!(money(0.001, Sign::Always, "USD"), "$0.00");
        assert_eq!(money(-0.001, Sign::Always, "USD"), "$0.00");
        assert_eq!(money(0.0, Sign::Always, "USD"), "$0.00");
    }

    #[test]
    fn r_shows_sign_and_one_decimal() {
        assert_eq!(r_text(Some(1.84)), "+1.8R");
        assert_eq!(r_text(Some(-1.2)), "\u{2212}1.2R");
        assert_eq!(r_text(Some(0.0)), "0.0R");
        assert_eq!(r_text(Some(-0.01)), "0.0R");
        assert_eq!(r_text(None), "—");
    }

    #[test]
    fn average_r_has_two_decimals() {
        assert_eq!(r_average(Some(0.424)), "+0.42R");
        assert_eq!(r_average(Some(-0.5)), "\u{2212}0.50R");
        assert_eq!(r_average(None), "—");
    }

    #[test]
    fn prices_keep_their_decimals() {
        assert_eq!(price(2650.25), "2650.25");
        assert_eq!(price(1.08123), "1.08123");
        assert_eq!(price(150.1), "150.10");
        assert_eq!(price(2650.0), "2650.00");
        assert_eq!(level_to(0.0, 2), "—");
        assert_eq!(level_to(2644.3, 2), "2644.30");
    }

    #[test]
    fn a_symbols_prices_all_have_the_decimals_it_has() {
        // 5-digit FX: a price that ends in 0 keeps its zero, so that 1.08500 and 1.08505 line up.
        assert_eq!(price_to(1.0850, 5), "1.08500");
        assert_eq!(price_to(1.08505, 5), "1.08505");
        assert_eq!(price_to(2648.2, 2), "2648.20");
        assert_eq!(decimals(1.0850), 3);
        assert_eq!(decimals(1.08505), 5);
        assert_eq!(decimals(2648.2), 1);
        assert_eq!(decimals(2650.0), 0);
    }

    #[test]
    fn lots_percent_and_ratio() {
        assert_eq!(lots(0.2), "0.20");
        assert_eq!(percent(0.5833), "58.3%");
        assert_eq!(ratio(Some(1.857)), "1.86");
        assert_eq!(ratio(None), "—");
        assert_eq!(ratio(Some(49.994)), "49.99");
        assert_eq!(ratio(Some(55.0)), "50+", "a profit factor made by one tiny loss is capped");
        assert_eq!(ratio(Some(1240.0)), "50+");
    }

    #[test]
    fn a_day_list_never_prints_r_as_money() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, day_list_never_prints_r_as_money);
    }

    fn day_list_never_prints_r_as_money() {
        use {
            MoneyColumn::{Dropped, Shown},
            Position::{Closed, Open},
        };
        let usd = Some("+$40.00".to_owned());
        // Both kinds in one list: the trade without money has a dash in the money column.
        assert_eq!(result_cells(Closed, usd.clone(), Some(2.0), Shown), ("+2.0R".to_owned(), usd.clone()));
        assert_eq!(result_cells(Closed, None, Some(2.0), Shown), ("+2.0R".to_owned(), Some("—".to_owned())));
        // Only trades in R: the money column is dropped.
        assert_eq!(result_cells(Closed, None, Some(2.0), Dropped), ("+2.0R".to_owned(), None));
        assert_eq!(result_cells(Closed, None, None, Dropped), ("—".to_owned(), None));
        // An open position.
        assert_eq!(result_cells(Open, usd, None, Shown), ("—".to_owned(), Some("đang mở".to_owned())));
        assert_eq!(result_cells(Open, None, None, Dropped), ("đang mở".to_owned(), None));
        assert_eq!(MoneyColumn::from(true), Shown);
        assert_eq!(Position::from(false), Open);
    }

    #[test]
    fn an_open_position_and_a_count_of_trades_are_said_in_the_chosen_language() {
        use crate::i18n::{Lang, with_text};
        assert_eq!(with_text(Lang::En, || result_cells(Position::Open, None, None, MoneyColumn::Dropped).0), "open");
        assert_eq!(with_text(Lang::Vi, || trade_count(3)), "3 lệnh");
        assert_eq!(with_text(Lang::En, || trade_count(1)), "1 trade");
        assert_eq!(with_text(Lang::En, || trade_count(3)), "3 trades");
        assert_eq!(with_text(Lang::Vi, || weekday_name(Weekday::Sun)), "Chủ nhật");
        assert_eq!(with_text(Lang::En, || weekday_name(Weekday::Sat)), "Saturday");
        assert_eq!(with_text(Lang::Vi, || weekday_short(Weekday::Sun)), "CN");
        assert_eq!(with_text(Lang::Vi, || month_name(11)), "Tháng 11");
        assert_eq!(with_text(Lang::En, || month_name(11)), "November");
        assert_eq!(with_text(Lang::En, || month_short(12)), "Dec");
        assert_eq!(with_text(Lang::En, || month_name(0)), "");
    }

    #[test]
    fn signs_are_read_back_from_text() {
        assert!(is_gain(&money(10.0, Sign::Always, "USD")));
        assert!(is_loss(&money(-10.0, Sign::Always, "USD")));
        assert!(!is_gain("$0.00") && !is_loss("$0.00"));
    }
}
