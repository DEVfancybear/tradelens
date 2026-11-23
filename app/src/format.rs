//! Text for numbers and times, the way MT5 shows them: a dot for decimals, a comma
//! between thousands, a real minus sign. Pure functions, so the pages stay declarative.

use chrono::NaiveDateTime;

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

/// A price with the decimals it has, at least two: `2650.25`, `1.08123`.
pub(crate) fn price(value: f64) -> String {
    let text = format!("{value:.5}");
    let trimmed = text.trim_end_matches('0');
    let decimals = trimmed.split_once('.').map_or(0, |(_, fraction)| fraction.len());
    if decimals >= 2 { trimmed.to_owned() } else { format!("{value:.2}") }
}

/// A price level where `0` means "not set" (stop loss, take profit).
pub(crate) fn level(value: f64) -> String {
    if value > 0.0 { price(value) } else { "—".to_owned() }
}

/// A profit factor: `1.86`, or a dash when nothing was lost yet.
pub(crate) fn ratio(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_owned(), |v| format!("{v:.2}"))
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
        assert_eq!(level(0.0), "—");
        assert_eq!(level(2644.3), "2644.30");
    }

    #[test]
    fn lots_percent_and_ratio() {
        assert_eq!(lots(0.2), "0.20");
        assert_eq!(percent(0.5833), "58.3%");
        assert_eq!(ratio(Some(1.857)), "1.86");
        assert_eq!(ratio(None), "—");
    }

    #[test]
    fn signs_are_read_back_from_text() {
        assert!(is_gain(&money(10.0, Sign::Always, "USD")));
        assert!(is_loss(&money(-10.0, Sign::Always, "USD")));
        assert!(!is_gain("$0.00") && !is_loss("$0.00"));
    }
}
