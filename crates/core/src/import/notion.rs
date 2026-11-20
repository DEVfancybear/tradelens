//! Notion trading journals: a database exported as "Markdown & CSV" (one row per
//! trade, columns named by the user), mapped onto trades by [`NotionMapping`].
//! The same mapping serves rows read through the Notion API (`tradelens-cloud`).

use crate::model::DealBatch;

use super::ImportError;

/// The trade fields a Notion column can fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Field {
    OpenTime,
    CloseTime,
    Symbol,
    Side,
    Volume,
    OpenPrice,
    ClosePrice,
    StopLoss,
    TakeProfit,
    Profit,
    Commission,
    Setup,
    Tags,
    Mistakes,
    Emotion,
    Rating,
    Notes,
    Screenshot,
}

impl Field {
    /// Every field, in the order the mapping screen lists them.
    pub const ALL: [Self; 18] = [
        Self::OpenTime,
        Self::CloseTime,
        Self::Symbol,
        Self::Side,
        Self::Volume,
        Self::OpenPrice,
        Self::ClosePrice,
        Self::StopLoss,
        Self::TakeProfit,
        Self::Profit,
        Self::Commission,
        Self::Setup,
        Self::Tags,
        Self::Mistakes,
        Self::Emotion,
        Self::Rating,
        Self::Notes,
        Self::Screenshot,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenTime => "Thời gian vào",
            Self::CloseTime => "Thời gian ra",
            Self::Symbol => "Symbol",
            Self::Side => "Buy/Sell",
            Self::Volume => "Khối lượng",
            Self::OpenPrice => "Giá vào",
            Self::ClosePrice => "Giá ra",
            Self::StopLoss => "Stop loss",
            Self::TakeProfit => "Take profit",
            Self::Profit => "Lãi/lỗ",
            Self::Commission => "Phí",
            Self::Setup => "Setup",
            Self::Tags => "Tag",
            Self::Mistakes => "Lỗi",
            Self::Emotion => "Cảm xúc",
            Self::Rating => "Đánh giá",
            Self::Notes => "Ghi chú",
            Self::Screenshot => "Ảnh",
        }
    }

    /// Column names (lowercase, English and Vietnamese) that usually hold the field.
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::OpenTime => &["date", "open time", "entry time", "ngày", "thời gian vào", "time"],
            Self::CloseTime => &["close time", "exit time", "thời gian ra", "ngày đóng"],
            Self::Symbol => &["symbol", "pair", "instrument", "ticker", "cặp", "mã"],
            Self::Side => &["side", "direction", "type", "position", "long/short", "buy/sell", "chiều"],
            Self::Volume => &["volume", "lot", "lots", "size", "quantity", "khối lượng"],
            Self::OpenPrice => &["entry", "entry price", "open price", "giá vào"],
            Self::ClosePrice => &["exit", "exit price", "close price", "giá ra"],
            Self::StopLoss => &["sl", "stop", "stop loss", "stoploss"],
            Self::TakeProfit => &["tp", "target", "take profit"],
            Self::Profit => &["p&l", "pnl", "p/l", "profit", "result", "lãi/lỗ", "lợi nhuận"],
            Self::Commission => &["commission", "fee", "fees", "phí"],
            Self::Setup => &["setup", "strategy", "chiến lược"],
            Self::Tags => &["tags", "tag"],
            Self::Mistakes => &["mistakes", "mistake", "errors", "lỗi"],
            Self::Emotion => &["emotion", "emotions", "feeling", "mood", "cảm xúc"],
            Self::Rating => &["rating", "grade", "score", "đánh giá"],
            Self::Notes => &["notes", "note", "comment", "review", "ghi chú"],
            Self::Screenshot => &["screenshot", "chart", "image", "ảnh"],
        }
    }
}

/// Which column fills which field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotionMapping {
    /// `(field, column name as in the export)`.
    pub columns: Vec<(Field, String)>,
    /// The account the trades go to (Notion has no MT5 login).
    pub account_login: String,
}

impl NotionMapping {
    /// A first guess from the export's header row; the user corrects it on screen.
    pub fn guess(headers: &[&str]) -> Self {
        let columns = Field::ALL
            .iter()
            .filter_map(|&field| {
                headers
                    .iter()
                    .find(|h| field.aliases().contains(&h.trim().to_lowercase().as_str()))
                    .map(|h| (field, (*h).to_owned()))
            })
            .collect();
        Self { columns, account_login: String::new() }
    }

    pub fn column(&self, field: Field) -> Option<&str> {
        self.columns.iter().find(|(f, _)| *f == field).map(|(_, c)| c.as_str())
    }
}

/// Reads a Notion CSV export with `mapping`. Each row becomes an entry and an exit
/// deal (negative tickets, as manual trades) and, when it has journal columns, a
/// journal entry.
///
/// # Errors
///
/// [`ImportError::Invalid`] when a mapped column is missing or a value does not parse;
/// [`ImportError::NotImplemented`] until v0.2.
pub fn parse_csv(_bytes: &[u8], _mapping: &NotionMapping) -> Result<DealBatch, ImportError> {
    Err(ImportError::NotImplemented("Notion CSV"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_a_typical_notion_journal() {
        let headers = ["Name", "Date", "Pair", "Direction", "Lot", "Entry", "Exit", "P&L", "Setup", "Notes"];
        let mapping = NotionMapping::guess(&headers);
        assert_eq!(mapping.column(Field::OpenTime), Some("Date"));
        assert_eq!(mapping.column(Field::Symbol), Some("Pair"));
        assert_eq!(mapping.column(Field::Side), Some("Direction"));
        assert_eq!(mapping.column(Field::Profit), Some("P&L"));
        assert_eq!(mapping.column(Field::StopLoss), None);
    }
}
