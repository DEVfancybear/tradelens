//! MT5 "Report → Open XML" (.xlsx): the same sections as the HTML report, one sheet.

use std::path::Path;

use crate::model::DealBatch;

use super::{ImportError, table};

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
fn rows(_path: &Path) -> Result<table::Rows, ImportError> {
    Err(ImportError::NotImplemented("v0.1: calamine::open_workbook_auto, first sheet, Data → String"))
}
