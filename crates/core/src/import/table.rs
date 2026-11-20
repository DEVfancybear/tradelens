//! The shared reading of a report's rows, whichever file they came from: find the
//! account header, the Deals table and the Positions table, then parse the cells.

use crate::model::DealBatch;

use super::ImportError;

/// A report as rows of cell texts.
pub(crate) type Rows = Vec<Vec<String>>;

/// Turns a report's rows into a batch.
///
/// Columns are found by their content (a date-time, a ticket, `in`/`out`), not only by
/// their titles, because the terminal writes the titles in its own language.
pub(crate) fn batch_from_rows(_rows: &Rows) -> Result<DealBatch, ImportError> {
    todo!("v0.1: account header, Deals section, Positions section (SL/TP)")
}

/// `1 234.56`, `-12.50`, `0.10 / 0.10` → a number (the first one).
#[allow(dead_code, reason = "read by batch_from_rows in v0.1")]
pub(crate) fn number(cell: &str) -> Option<f64> {
    let first = cell.split('/').next()?.trim();
    let cleaned: String = first.chars().filter(|c| !c.is_whitespace() && *c != '\u{a0}').collect();
    if cleaned.is_empty() { None } else { cleaned.parse().ok() }
}

#[cfg(test)]
mod tests {
    use super::number;

    #[test]
    fn reads_report_numbers() {
        assert_eq!(number("1 234.56"), Some(1234.56));
        assert_eq!(number("-12.50"), Some(-12.5));
        assert_eq!(number("0.10 / 0.10"), Some(0.1));
        assert_eq!(number(""), None);
    }
}
