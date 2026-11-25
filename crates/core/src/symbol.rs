//! Instrument codes as MetaTrader writes them: upper case, without the separators a hand or a
//! journal puts between the two currencies.

/// The instrument as MetaTrader writes it: `eurusd`, `BTC/USD` and `xau-usd` become `EURUSD`,
/// `BTCUSD` and `XAUUSD`. Text that is not a code (it has spaces, or other punctuation) is only
/// trimmed. A `.` stays, since it starts a broker's suffix (`XAUUSD.m`).
///
/// Every place that stores a symbol, the importers and the trader's own entry, goes through here,
/// so the same instrument is one symbol everywhere it is counted.
#[must_use]
pub fn normalise(text: &str) -> String {
    let text = text.trim();
    let separator = |c: char| matches!(c, '/' | '\\' | '-' | '_');
    let code =
        !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || separator(c) || matches!(c, '.' | '#' | '+'));
    if code { text.chars().filter(|&c| !separator(c)).flat_map(char::to_uppercase).collect() } else { text.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_upper_case_without_separators() {
        assert_eq!(normalise("eurusd"), "EURUSD");
        assert_eq!(normalise(" btc/usd "), "BTCUSD");
        assert_eq!(normalise("xau-usd"), "XAUUSD");
        assert_eq!(normalise("us_30"), "US30");
        assert_eq!(normalise("eurusd.m"), "EURUSD.M", "a broker's suffix keeps its dot");
    }

    #[test]
    fn text_that_is_not_a_code_is_left_alone() {
        assert_eq!(normalise("Gold spot"), "Gold spot");
        assert_eq!(normalise("  "), "");
    }
}
