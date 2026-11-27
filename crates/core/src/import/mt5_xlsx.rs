//! MT5 "Report → Open XML" (.xlsx): the same sections as the HTML report, one sheet.

use std::path::Path;

use calamine::{Data, Reader, open_workbook_auto};
use chrono::{NaiveDate, NaiveDateTime, TimeDelta};

use crate::model::DealBatch;

use super::{ImportError, table};

/// The report's time format, as the HTML report writes it.
const REPORT_TIME: &str = "%Y.%m.%d %H:%M:%S";

/// # Errors
///
/// [`ImportError`] when the content is not a readable report.
pub fn read(path: &Path) -> Result<DealBatch, ImportError> {
    table::batch_from_rows(&rows(path)?)
}

/// The first sheet's cells as text (dates as `YYYY.MM.DD HH:MM:SS`).
///
/// # Errors
///
/// [`ImportError`] when the content is not a readable report.
fn rows(path: &Path) -> Result<table::Rows, ImportError> {
    let unreadable =
        |e: &dyn std::fmt::Display| ImportError::Invalid(crate::tr!("core.import.excel_unreadable", error = e));
    let mut workbook = open_workbook_auto(path).map_err(|e| unreadable(&e))?;
    let range = workbook
        .worksheet_range_at(0)
        .ok_or_else(|| ImportError::Invalid(crate::tr!("core.import.excel_no_sheet")))?
        .map_err(|e| unreadable(&e))?;
    Ok(range.rows().map(|row| row.iter().map(cell_text).collect()).collect())
}

/// One cell as the text the HTML report would show.
fn cell_text(cell: &Data) -> String {
    match cell {
        Data::Empty | Data::Error(_) => String::new(),
        Data::DateTime(value) => value.as_datetime().map_or_else(|| value.to_string(), report_time),
        Data::DateTimeIso(value) => iso_time(value),
        Data::Int(_) | Data::Float(_) | Data::String(_) | Data::Bool(_) | Data::DurationIso(_) => {
            cell.to_string().trim().to_owned()
        }
    }
}

/// Excel stores a time as a fraction of a day, so it is rounded to the nearest second.
fn report_time(dt: NaiveDateTime) -> String {
    dt.checked_add_signed(TimeDelta::milliseconds(500)).unwrap_or(dt).format(REPORT_TIME).to_string()
}

/// `2026-11-20T14:05:33` or `2026-11-20` → the report's format; anything else as written.
fn iso_time(value: &str) -> String {
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(value, format) {
            return report_time(dt);
        }
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map_or_else(|| value.to_owned(), report_time)
}

#[cfg(test)]
mod tests {
    use calamine::{Data, ExcelDateTime, ExcelDateTimeType};

    use super::cell_text;

    #[test]
    fn writes_every_cell_kind_as_text() {
        assert_eq!(cell_text(&Data::Empty), "");
        assert_eq!(cell_text(&Data::Int(81_234_567)), "81234567");
        assert_eq!(cell_text(&Data::Float(-12.5)), "-12.5");
        assert_eq!(cell_text(&Data::String("  XAUUSD ".to_owned())), "XAUUSD");
        assert_eq!(cell_text(&Data::DateTimeIso("2026-11-20T14:05:33".to_owned())), "2026.11.20 14:05:33");
        assert_eq!(cell_text(&Data::DateTimeIso("2026-11-20".to_owned())), "2026.11.20 00:00:00");
        assert_eq!(cell_text(&Data::DateTimeIso("garbage".to_owned())), "garbage");
    }

    #[test]
    fn writes_excel_dates_like_the_html_report() {
        // 2026-11-20 14:05:33 is serial 46346 plus 50733 s of the day's 86400.
        let serial = 46_346.0 + 50_733.0 / 86_400.0;
        let cell = Data::DateTime(ExcelDateTime::new(serial, ExcelDateTimeType::DateTime, false));
        assert_eq!(cell_text(&cell), "2026.11.20 14:05:33");
    }
}
