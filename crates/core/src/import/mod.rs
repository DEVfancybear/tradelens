//! Importers: MT5 history reports (HTML, XLSX) and JSON batches, all turned into a
//! [`DealBatch`] that [`crate::Db::upsert_batch`] stores.

use std::path::Path;

use crate::model::DealBatch;

pub mod json;
pub mod mt5_html;
pub mod mt5_xlsx;
mod table;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("không đọc được file: {0}")]
    Io(#[from] std::io::Error),
    #[error("định dạng không hỗ trợ: {0}")]
    Unsupported(String),
    #[error("không tìm thấy bảng Deals trong báo cáo")]
    NoDeals,
    #[error("{0}")]
    Invalid(String),
}

/// The formats an import can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mt5Html,
    Mt5Xlsx,
    Json,
}

impl Format {
    pub fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "html" | "htm" => Some(Self::Mt5Html),
            "xlsx" => Some(Self::Mt5Xlsx),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// The `source` recorded on the imported deals.
    pub fn source(self) -> &'static str {
        match self {
            Self::Mt5Html => "html",
            Self::Mt5Xlsx => "xlsx",
            Self::Json => "json",
        }
    }
}

/// Reads any supported file.
pub fn read_file(path: &Path) -> Result<(Format, DealBatch), ImportError> {
    let format = Format::of(path).ok_or_else(|| ImportError::Unsupported(path.display().to_string()))?;
    let batch = match format {
        Format::Mt5Html => mt5_html::parse(&std::fs::read(path)?)?,
        Format::Mt5Xlsx => mt5_xlsx::read(path)?,
        Format::Json => json::parse(&std::fs::read(path)?)?,
    };
    Ok((format, batch))
}
