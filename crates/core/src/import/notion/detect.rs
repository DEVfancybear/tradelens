//! Which column holds which trade field: by the column's name first, then, for the three
//! fields a trade cannot do without, by what the column contains.

use crate::import::ImportError;

use super::cells::{fold, header_key, parse_bool, parse_datetime, parse_number, parse_rating, parse_side};
use super::sheet::Sheet;
use super::{Field, NotionMapping};

/// How much of a column's filled cells must look like a field for the column to be it:
/// `MIN_SHARE_NUM` in `MIN_SHARE_DEN`.
const MIN_SHARE_NUM: usize = 4;
const MIN_SHARE_DEN: usize = 5;

/// Columns Notion adds by itself; they say when and who, not what happened.
const SYSTEM_COLUMNS: [&str; 6] =
    ["created time", "last edited time", "created by", "last edited by", "ngay tao", "chinh sua lan cuoi"];

/// Names of the title column, which names the row and keys the trade.
const TITLE_NAMES: [&str; 5] = ["name", "title", "ten", "tieu de", "trade"];

/// The fields without which a row is not a trade.
const REQUIRED: [Field; 3] = [Field::OpenTime, Field::Symbol, Field::Side];

/// The columns of a file, sorted into what they are.
#[derive(Debug)]
pub(super) struct Layout {
    /// The trade fields and the column index of each.
    pub(super) fields: Vec<(Field, usize)>,
    /// The title column when one is named like it (`Name`).
    pub(super) title: Option<usize>,
    /// The column whose text and open time identify a row: the title, else the first.
    pub(super) key_column: usize,
    /// Empty and Notion-made columns.
    pub(super) ignored: Vec<usize>,
    /// Everything else, which becomes properties.
    pub(super) properties: Vec<usize>,
}

impl Layout {
    pub(super) fn column(&self, field: Field) -> Option<usize> {
        self.fields.iter().find(|(f, _)| *f == field).map(|&(_, c)| c)
    }
}

/// # Errors
///
/// [`ImportError::Invalid`] when an override names a missing column or a required field has
/// no column.
pub(super) fn detect(sheet: &Sheet, overrides: &NotionMapping) -> Result<Layout, ImportError> {
    let width = sheet.headers.len();
    let keys: Vec<String> = sheet.headers.iter().map(|h| header_key(h)).collect();
    let values: Vec<Vec<&str>> = (0..width).map(|c| sheet.values(c)).collect();
    let ignored: Vec<usize> =
        (0..width).filter(|&c| values[c].is_empty() || SYSTEM_COLUMNS.contains(&keys[c].as_str())).collect();
    let mut fields: Vec<(Field, usize)> = Vec::new();
    let free =
        |fields: &[(Field, usize)], c: usize| !ignored.contains(&c) && fields.iter().all(|&(_, taken)| taken != c);

    for (field, name) in &overrides.columns {
        let wanted = name.trim().to_lowercase();
        let column = sheet
            .headers
            .iter()
            .position(|h| h.trim().to_lowercase() == wanted)
            .or_else(|| keys.iter().position(|k| *k == header_key(name)))
            .ok_or_else(|| ImportError::Invalid(format!("không có cột «{name}» trong file")))?;
        fields.retain(|(f, c)| f != field && *c != column);
        fields.push((*field, column));
    }

    for field in Field::ALL {
        if fields.iter().any(|(f, _)| *f == field) {
            continue;
        }
        let aliases: Vec<String> = field.aliases().iter().map(|a| fold(a)).collect();
        let found =
            (0..width).find(|&c| free(&fields, c) && aliases.contains(&keys[c]) && accepts(field, &values[c], sheet));
        if let Some(column) = found {
            fields.push((field, column));
        }
    }

    for field in REQUIRED {
        if fields.iter().any(|(f, _)| *f == field) {
            continue;
        }
        let found = (0..width).find(|&c| free(&fields, c) && looks_like(field, &values[c], sheet));
        if let Some(column) = found {
            fields.push((field, column));
        }
    }

    let missing: Vec<&str> =
        REQUIRED.iter().filter(|field| fields.iter().all(|(f, _)| f != *field)).map(|field| field.label()).collect();
    if !missing.is_empty() {
        return Err(ImportError::Invalid(format!(
            "không nhận ra cột {} trong file (các cột có: {})",
            missing.join(", "),
            sheet.headers.join(", ")
        )));
    }

    let title = (0..width).find(|&c| free(&fields, c) && TITLE_NAMES.contains(&keys[c].as_str()));
    let key_column = title.unwrap_or(0);
    let properties = (0..width)
        .filter(|&c| !ignored.contains(&c) && Some(c) != title && fields.iter().all(|&(_, taken)| taken != c))
        .collect();
    fields.sort_by_key(|&(field, _)| field);
    Ok(Layout { fields, title, key_column, ignored, properties })
}

fn share(values: &[&str], mut test: impl FnMut(&str) -> bool) -> bool {
    if values.is_empty() {
        return false;
    }
    let hits = values.iter().filter(|v| test(v)).count();
    hits * MIN_SHARE_DEN >= values.len() * MIN_SHARE_NUM
}

/// Whether the contents of a column fit a field whose header matched.
fn accepts(field: Field, values: &[&str], sheet: &Sheet) -> bool {
    let locale = sheet.locale;
    match field {
        Field::OpenTime | Field::CloseTime => share(values, |v| parse_datetime(v, locale.dates).is_some()),
        Field::Side => share(values, |v| parse_side(v).is_some()),
        Field::Symbol => {
            share(values, |v| v.chars().count() <= 24 && v.chars().any(char::is_alphabetic) && parse_side(v).is_none())
        }
        Field::Volume
        | Field::OpenPrice
        | Field::ClosePrice
        | Field::StopLoss
        | Field::TakeProfit
        | Field::Profit
        | Field::Commission
        | Field::Swap => share(values, |v| parse_number(v, locale.comma).is_some()),
        Field::Rating => share(values, |v| parse_rating(v).is_some()),
        Field::FollowedPlan | Field::ReviewLater => share(values, |v| parse_bool(v).is_some()),
        Field::Setup
        | Field::SetupGrade
        | Field::Tags
        | Field::Mistakes
        | Field::Emotion
        | Field::EmotionAfter
        | Field::ExitReason
        | Field::Market
        | Field::Notes
        | Field::Screenshot => true,
    }
}

/// Whether a column of unknown name holds a required field, judging by its contents alone.
fn looks_like(field: Field, values: &[&str], sheet: &Sheet) -> bool {
    match field {
        Field::Symbol => share(values, |v| is_instrument_code(v, sheet)),
        _ => accepts(field, values, sheet),
    }
}

/// `EURUSD`, `XAUUSD.m`, `US30`, `BTC/USD`: short, one word, capitals and digits.
fn is_instrument_code(value: &str, sheet: &Sheet) -> bool {
    let len = value.chars().count();
    (3..=14).contains(&len)
        && value
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '/' | '.' | '_' | '#' | '-' | '+'))
        && value.chars().filter(char::is_ascii_uppercase).count() >= 2
        && parse_side(value).is_none()
        && parse_number(value, sheet.locale.comma).is_none()
}

#[cfg(test)]
mod tests {
    use super::super::sheet;
    use super::*;

    fn layout(csv: &str) -> Layout {
        detect(&sheet::read(csv.as_bytes()).unwrap(), &NotionMapping::default()).unwrap()
    }

    #[test]
    fn finds_required_columns_by_contents_when_names_say_nothing() {
        let found = layout(
            "Name,When,Asset class,Dir,Result\n\
             a,2026-11-20 09:00,EURUSD,Long,Win\n\
             b,2026-11-21 10:00,XAUUSD,Short,Loss\n\
             c,2026-11-22 11:00,EURUSD,Long,Win\n",
        );
        assert_eq!(found.column(Field::OpenTime), Some(1));
        assert_eq!(found.column(Field::Symbol), Some(2));
        assert_eq!(found.column(Field::Side), Some(3));
        assert_eq!(found.column(Field::Profit), None, "a Win/Loss column is not a profit");
        assert_eq!(found.properties, [4], "it becomes a property, and the title is not one");
        assert_eq!(found.title, Some(0));
    }

    #[test]
    fn ignores_empty_and_notion_made_columns() {
        let found = layout(
            "Date,Pair,Side,Created time,Empty\n\
             2026-11-20,EURUSD,Buy,\"November 20, 2026 9:30 AM\",\n",
        );
        assert_eq!(found.ignored, [3, 4]);
        assert_eq!(found.properties, Vec::<usize>::new());
    }

    #[test]
    fn says_what_is_missing() {
        let csv = "Name,Notes\nx,y\n";
        let err = detect(&sheet::read(csv.as_bytes()).unwrap(), &NotionMapping::default()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("Thời gian vào") && message.contains("Symbol") && message.contains("Name, Notes"));
    }

    #[test]
    fn an_override_wins_over_the_detection() {
        let csv = "Date,Pair,Side,Opened\n2026-11-20,EURUSD,Buy,2026-11-21\n";
        let mapping = NotionMapping { columns: vec![(Field::OpenTime, "opened".into())], account_login: String::new() };
        let found = detect(&sheet::read(csv.as_bytes()).unwrap(), &mapping).unwrap();
        assert_eq!(found.column(Field::OpenTime), Some(3));
        let bad = NotionMapping { columns: vec![(Field::OpenTime, "nope".into())], account_login: String::new() };
        assert!(detect(&sheet::read(csv.as_bytes()).unwrap(), &bad).is_err());
    }
}
