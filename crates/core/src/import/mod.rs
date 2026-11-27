//! Importers: MT5 history reports (HTML, XLSX), Notion journal exports (CSV) and JSON
//! batches, all turned into a
//! [`DealBatch`] that [`crate::Db::upsert_batch`] stores.

use std::path::Path;

use crate::model::DealBatch;

pub mod json;
mod link;
pub mod mt5_html;
pub mod mt5_xlsx;
pub mod notion;
mod table;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{}", crate::tr!("core.import.unreadable_file", error = .0))]
    Io(#[from] std::io::Error),
    #[error("{}", crate::tr!("core.import.unsupported_format", format = .0))]
    Unsupported(String),
    #[error("{}", crate::tr!("core.import.no_deals"))]
    NoDeals,
    #[error("{0}")]
    Invalid(String),
    #[error("{}", crate::tr!("db.not_implemented", what = .0))]
    NotImplemented(&'static str),
}

/// The formats an import can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mt5Html,
    Mt5Xlsx,
    /// A Notion database exported as CSV; read it with [`notion::infer`], which keeps the journal.
    NotionCsv,
    Json,
}

impl Format {
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "html" | "htm" => Some(Self::Mt5Html),
            "xlsx" => Some(Self::Mt5Xlsx),
            "csv" => Some(Self::NotionCsv),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// The `source` recorded on the imported deals.
    pub fn source(self) -> &'static str {
        match self {
            Self::Mt5Html => "html",
            Self::Mt5Xlsx => "xlsx",
            Self::NotionCsv => "notion",
            Self::Json => "json",
        }
    }
}

/// Reads any supported MT5 report or JSON batch. A Notion CSV carries a journal a batch cannot hold:
/// it is refused here and read with [`notion::infer`].
///
/// # Errors
///
/// [`ImportError`] when the file cannot be read or holds no deals, or is a Notion CSV.
pub fn read_file(path: &Path) -> Result<(Format, DealBatch), ImportError> {
    let format = Format::of(path).ok_or_else(|| ImportError::Unsupported(path.display().to_string()))?;
    let batch = match format {
        Format::Mt5Html => mt5_html::parse(&std::fs::read(path)?)?,
        Format::Mt5Xlsx => mt5_xlsx::read(path)?,
        Format::NotionCsv => {
            return Err(ImportError::Invalid(crate::tr!("core.import.notion_csv")));
        }
        Format::Json => json::parse(&std::fs::read(path)?)?,
    };
    Ok((format, batch))
}
