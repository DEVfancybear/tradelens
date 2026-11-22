//! The columns that are not trade fields become properties of the trade form: their kind
//! is read off the values, the way Notion types a column.

use std::collections::BTreeSet;

use crate::catalog::{FieldKind, FieldValue};

use super::PlannedProperty;
use super::cells::{Locale, is_url, parse_bool, parse_datetime, parse_number, slug, split_list};
use super::detect::Layout;
use super::sheet::Sheet;

/// The most distinct values a column may have to be a single select.
const MAX_SELECT_OPTIONS: usize = 15;
/// The most distinct items a column may have to be a multi-select.
const MAX_MULTI_OPTIONS: usize = 30;
/// A select option or multi-select item longer than this is prose, not a label.
const MAX_OPTION_CHARS: usize = 40;
/// A cell longer than this wants a text area.
const LONG_TEXT_CHARS: usize = 120;

/// The properties of the layout's property columns, with the column each reads.
pub(super) fn infer(sheet: &Sheet, layout: &Layout) -> Vec<(usize, PlannedProperty)> {
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut planned = Vec::new();
    for &column in &layout.properties {
        let values = sheet.values(column);
        let label = sheet.headers[column].clone();
        let mut key = slug(&label);
        let base = key.clone();
        for n in 2.. {
            if taken.insert(key.clone()) {
                break;
            }
            key = format!("{base}_{n}");
        }
        let kind = kind_of(&values, sheet.locale);
        planned.push((column, PlannedProperty { key, label, kind, options: Vec::new(), filled: 0 }));
    }
    planned
}

/// The kind of a column from its filled cells (there is at least one).
pub(super) fn kind_of(values: &[&str], locale: Locale) -> FieldKind {
    let nine_in_ten = |test: &dyn Fn(&str) -> bool| values.iter().filter(|v| test(v)).count() * 10 >= values.len() * 9;
    if nine_in_ten(&|v| parse_bool(v).is_some()) {
        return FieldKind::Checkbox;
    }
    if nine_in_ten(&|v| parse_datetime(v, locale.dates).is_some()) {
        return FieldKind::Date;
    }
    if nine_in_ten(&|v| parse_number(v, locale.comma).is_some()) {
        return FieldKind::Number;
    }
    if values.iter().filter(|v| is_url(v)).count() * 5 >= values.len() * 4 {
        return FieldKind::Url;
    }
    let distinct: BTreeSet<&str> = values.iter().copied().collect();
    let short = values.iter().all(|v| !v.contains('\n') && v.chars().count() <= MAX_OPTION_CHARS);
    if short && is_multi_select(values) {
        return FieldKind::MultiSelect;
    }
    if short && distinct.len() <= MAX_SELECT_OPTIONS && distinct.len() * 2 <= values.len() {
        return FieldKind::Select;
    }
    if values.iter().any(|v| v.contains('\n') || v.chars().count() > LONG_TEXT_CHARS) {
        FieldKind::LongText
    } else {
        FieldKind::Text
    }
}

/// Comma-separated items that come back: some cells hold several, and the items repeat.
fn is_multi_select(values: &[&str]) -> bool {
    if !values.iter().any(|v| v.contains(',')) {
        return false;
    }
    let items: Vec<String> = values.iter().flat_map(|v| split_list(v)).collect();
    let distinct: BTreeSet<&String> = items.iter().collect();
    items.iter().all(|item| item.chars().count() <= MAX_OPTION_CHARS)
        && distinct.len() <= MAX_MULTI_OPTIONS
        && items.len() > distinct.len()
}

/// The value a cell holds for a property of `kind`; `None` when it holds nothing the
/// kind can keep (an outlier of a numeric column, an empty cell).
pub(super) fn value_of(kind: FieldKind, cell: &str, locale: Locale) -> Option<FieldValue> {
    let cell = cell.trim();
    if cell.is_empty() {
        return None;
    }
    match kind {
        FieldKind::Checkbox => parse_bool(cell).map(FieldValue::Flag),
        FieldKind::Number => parse_number(cell, locale.comma).map(FieldValue::Number),
        FieldKind::Date => {
            parse_datetime(cell, locale.dates).map(|moment| FieldValue::Date(moment.format("%Y-%m-%d").to_string()))
        }
        FieldKind::Select => Some(FieldValue::Choice(cell.to_owned())),
        FieldKind::MultiSelect => {
            let items = split_list(cell);
            (!items.is_empty()).then_some(FieldValue::Choices(items))
        }
        FieldKind::Text | FieldKind::LongText | FieldKind::Url | FieldKind::Scale => {
            Some(FieldValue::Text(cell.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::notion::cells::{CommaRole, DateOrder};

    const LOCALE: Locale = Locale { dates: DateOrder::DayFirst, comma: CommaRole::Thousands };

    fn kind(values: &[&str]) -> FieldKind {
        kind_of(values, LOCALE)
    }

    #[test]
    fn reads_the_kind_off_the_values() {
        assert_eq!(kind(&["Yes", "No", "No", "Yes"]), FieldKind::Checkbox);
        assert_eq!(kind(&["✓", "✗"]), FieldKind::Checkbox);
        assert_eq!(kind(&["7", "8.5", "9"]), FieldKind::Number);
        assert_eq!(kind(&["$1,200", "-$300"]), FieldKind::Number);
        assert_eq!(kind(&["November 20, 2026", "November 21, 2026"]), FieldKind::Date);
        assert_eq!(kind(&["20/11/2026 09:30", "21/11/2026"]), FieldKind::Date);
        assert_eq!(kind(&["https://www.tradingview.com/x/a/", "https://www.tradingview.com/x/b/"]), FieldKind::Url);
        assert_eq!(kind(&["Calm", "Anxious", "Calm", "Anxious", "Calm"]), FieldKind::Select);
        assert_eq!(kind(&["a, b", "b", "a, c", "c"]), FieldKind::MultiSelect);
        assert_eq!(kind(&["one", "two", "three"]), FieldKind::Text, "all different: free text");
        assert_eq!(kind(&["good entry, late exit", "chased the move, got stopped"]), FieldKind::Text);
        assert_eq!(kind(&["line one\nline two", "x"]), FieldKind::LongText);
    }

    #[test]
    fn a_cell_becomes_the_value_of_its_kind() {
        assert_eq!(value_of(FieldKind::Checkbox, "Yes", LOCALE), Some(FieldValue::Flag(true)));
        assert_eq!(value_of(FieldKind::Number, "1,5", LOCALE), Some(FieldValue::Number(1.5)));
        assert_eq!(value_of(FieldKind::Number, "n/a", LOCALE), None);
        assert_eq!(
            value_of(FieldKind::Date, "November 20, 2026 9:30 AM", LOCALE),
            Some(FieldValue::Date("2026-11-20".into()))
        );
        assert_eq!(
            value_of(FieldKind::MultiSelect, "a, b", LOCALE),
            Some(FieldValue::Choices(vec!["a".into(), "b".into()]))
        );
        assert_eq!(value_of(FieldKind::Select, "  ", LOCALE), None);
    }
}
