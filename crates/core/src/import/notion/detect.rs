//! Which column holds which trade field. Columns are recognised by what they contain and by
//! how they relate to each other (a result column agrees in sign with a status column); a
//! column's name is only a tie-breaker between columns that fit equally well, so a file whose
//! columns are all renamed, or reordered, is read the same way.

use crate::import::ImportError;

use super::cells::{
    fold, header_key, is_image_name, parse_bool, parse_datetime, parse_number, parse_rating, parse_side, split_files,
};
use super::sheet::Sheet;
use super::{Field, NotionMapping, OutcomeColumn, Verdict};

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

/// A number this small counts as zero.
const FLAT: f64 = 1e-9;

/// Results in R stay within this many R (a trader rarely books more than a dozen).
const MAX_R: f64 = 20.0;

/// How much of a result column's values must be multiples of 0.1 for it to read as R rather
/// than money: `ROUND_SHARE_NUM` in `ROUND_SHARE_DEN`.
const ROUND_SHARE_NUM: usize = 7;
const ROUND_SHARE_DEN: usize = 10;

/// A status column has at least this many different values and at most `MAX_STATUS_VALUES`.
const MIN_STATUS_VALUES: usize = 2;
const MAX_STATUS_VALUES: usize = 6;

/// Of the rows, this share must have a status that another row has too: `REPEAT_NUM` in `REPEAT_DEN`.
const REPEAT_NUM: usize = 3;
const REPEAT_DEN: usize = 5;

/// How much of the rows a status column must sort into the sign of the result: `PURITY_NUM` in
/// `PURITY_DEN`.
const PURITY_NUM: usize = 85;
const PURITY_DEN: usize = 100;

/// An instrument column repeats a few codes: at most this share of its cells are distinct
/// (when it has enough rows to tell): `DISTINCT_NUM` in `DISTINCT_DEN`.
const DISTINCT_NUM: usize = 3;
const DISTINCT_DEN: usize = 5;
const ROWS_TO_TELL_REPEATS: usize = 6;

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
    /// The status column (which stays a property too) and what its values mean.
    pub(super) outcome: Option<(usize, Vec<(String, Verdict)>)>,
}

impl Layout {
    pub(super) fn column(&self, field: Field) -> Option<usize> {
        self.fields.iter().find(|(f, _)| *f == field).map(|&(_, c)| c)
    }

    /// The status column as the plan reports it.
    pub(super) fn outcome_column(&self, sheet: &Sheet) -> Option<OutcomeColumn> {
        self.outcome
            .as_ref()
            .map(|(column, values)| OutcomeColumn { column: sheet.headers[*column].clone(), values: values.clone() })
    }
}

/// What the cells of a column say about it, found once.
struct Profile<'a> {
    /// The non-empty trimmed cells.
    values: Vec<&'a str>,
    /// The name, folded without units.
    key: String,
}

/// # Errors
///
/// [`ImportError::Invalid`] when an override names a missing column or a required field has
/// no column.
pub(super) fn detect(sheet: &Sheet, overrides: &NotionMapping) -> Result<Layout, ImportError> {
    let width = sheet.headers.len();
    let profiles: Vec<Profile<'_>> =
        (0..width).map(|c| Profile { values: sheet.values(c), key: header_key(&sheet.headers[c]) }).collect();
    let ignored: Vec<usize> = (0..width)
        .filter(|&c| profiles[c].values.is_empty() || SYSTEM_COLUMNS.contains(&profiles[c].key.as_str()))
        .collect();
    let mut fields: Vec<(Field, usize)> = Vec::new();
    let free =
        |fields: &[(Field, usize)], c: usize| !ignored.contains(&c) && fields.iter().all(|&(_, taken)| taken != c);

    for (field, name) in &overrides.columns {
        let wanted = name.trim().to_lowercase();
        let column = sheet
            .headers
            .iter()
            .position(|h| h.trim().to_lowercase() == wanted)
            .or_else(|| profiles.iter().position(|p| p.key == header_key(name)))
            .ok_or_else(|| ImportError::Invalid(format!("không có cột «{name}» trong file")))?;
        fields.retain(|(f, c)| f != field && *c != column);
        fields.push((*field, column));
    }

    // Names that fit a field, and a column that holds what the field needs, go together: the
    // name only settles which of the columns that could be the field is.
    for field in Field::ALL {
        if fields.iter().any(|(f, _)| *f == field) || is_decided_by_contents(field) {
            continue;
        }
        let aliases: Vec<String> = field.aliases().iter().map(|a| fold(a)).collect();
        let found = (0..width).find(|&c| {
            free(&fields, c) && aliases.contains(&profiles[c].key) && accepts(field, &profiles[c].values, sheet)
        });
        if let Some(column) = found {
            fields.push((field, column));
        }
    }

    // The three fields a trade cannot do without, and the images, by contents; a name breaks ties.
    for field in [Field::OpenTime, Field::Side, Field::Symbol, Field::Screenshot] {
        if fields.iter().any(|(f, _)| *f == field) {
            continue;
        }
        let candidates: Vec<usize> =
            (0..width).filter(|&c| free(&fields, c) && looks_like(field, &profiles[c].values, sheet)).collect();
        let aliases: Vec<String> = field.aliases().iter().map(|a| fold(a)).collect();
        let named = |c: usize| aliases.contains(&profiles[c].key);
        let chosen = if field == Field::OpenTime {
            earliest(&candidates, sheet, &named)
        } else {
            candidates.iter().copied().find(|&c| named(c)).or_else(|| candidates.first().copied())
        };
        if let Some(column) = chosen {
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

    let outcome = find_results(sheet, &profiles, &mut fields, &ignored);

    let title = (0..width).find(|&c| free(&fields, c) && TITLE_NAMES.contains(&profiles[c].key.as_str()));
    let key_column = title.unwrap_or(0);
    let properties = (0..width)
        .filter(|&c| !ignored.contains(&c) && Some(c) != title && fields.iter().all(|&(_, taken)| taken != c))
        .collect();
    fields.sort_by_key(|&(field, _)| field);
    Ok(Layout { fields, title, key_column, ignored, properties, outcome })
}

/// Fields whose column is found by [`find_results`] from the numbers, not by name.
fn is_decided_by_contents(field: Field) -> bool {
    matches!(field, Field::OpenTime | Field::Side | Field::Symbol | Field::Screenshot | Field::ResultR)
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
        Field::ResultR => numbers_of(values, sheet).is_some_and(|numbers| has_both_signs(&numbers)),
        Field::Rating => share(values, |v| parse_rating(v).is_some()),
        Field::FollowedPlan | Field::ReviewLater => share(values, |v| parse_bool(v).is_some()),
        Field::Screenshot => share(values, is_files_cell),
        Field::Setup
        | Field::SetupGrade
        | Field::Tags
        | Field::Mistakes
        | Field::Emotion
        | Field::EmotionAfter
        | Field::ExitReason
        | Field::Market
        | Field::Notes => true,
    }
}

/// Whether a column of any name holds the field, judging by its contents alone.
fn looks_like(field: Field, values: &[&str], sheet: &Sheet) -> bool {
    match field {
        Field::Symbol => {
            share(values, |v| is_instrument_code(v, sheet)) && repeats(values) && !share(values, is_files_cell)
        }
        Field::Screenshot => share(values, is_files_cell),
        _ => accepts(field, values, sheet),
    }
}

/// A files cell: every part, split on commas and stripped of a `(link)` suffix, names an image.
fn is_files_cell(cell: &str) -> bool {
    let parts = split_files(cell);
    !parts.is_empty() && parts.iter().all(|part| is_image_name(part))
}

/// Whether a column keeps coming back to the same few values, as the instrument of a journal
/// does (a column of all-different codes is more likely a trade reference).
fn repeats(values: &[&str]) -> bool {
    if values.len() < ROWS_TO_TELL_REPEATS {
        return true;
    }
    let mut distinct: Vec<&str> = values.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    distinct.len() * DISTINCT_DEN <= values.len() * DISTINCT_NUM
}

/// Among date columns, the one a trade's open belongs in: the one that is earlier than the
/// others in most rows (an entry precedes an exit, a trade date precedes the time it was
/// logged); a name that fits, then the most complete column, break ties.
fn earliest(candidates: &[usize], sheet: &Sheet, named: &impl Fn(usize) -> bool) -> Option<usize> {
    let order = sheet.locale.dates;
    let moment = |row: &super::sheet::Row, column: usize| parse_datetime(row.cell(column), order);
    let wins = |column: usize| {
        sheet
            .rows
            .iter()
            .filter(|row| {
                let Some(mine) = moment(row, column) else { return false };
                candidates
                    .iter()
                    .filter(|&&other| other != column)
                    .all(|&other| moment(row, other).is_none_or(|theirs| mine <= theirs))
            })
            .count()
    };
    let filled = |column: usize| sheet.rows.iter().filter(|row| moment(row, column).is_some()).count();
    candidates.iter().copied().max_by_key(|&c| (wins(c), named(c), filled(c), std::cmp::Reverse(c)))
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

// --- the result of a trade ---------------------------------------------------------------------------

/// The numbers of a column when it is numeric (most cells read as numbers); the cells that are
/// not numbers are left out.
fn numbers_of(values: &[&str], sheet: &Sheet) -> Option<Vec<f64>> {
    let numbers: Vec<f64> = values.iter().filter_map(|v| parse_number(v, sheet.locale.comma)).collect();
    (!values.is_empty() && numbers.len() * MIN_SHARE_DEN >= values.len() * MIN_SHARE_NUM).then_some(numbers)
}

fn has_both_signs(numbers: &[f64]) -> bool {
    numbers.iter().any(|n| *n < -FLAT) && numbers.iter().any(|n| *n > FLAT)
}

/// Results in R: small, and mostly multiples of a tenth (`2`, `0.6`, `-1`, `-0.2`).
fn looks_like_r(numbers: &[f64]) -> bool {
    let round = numbers.iter().filter(|n| ((**n * 10.0) - (**n * 10.0).round()).abs() < 1e-6).count();
    numbers.iter().all(|n| n.abs() <= MAX_R) && round * ROUND_SHARE_DEN >= numbers.len() * ROUND_SHARE_NUM
}

fn sign_of(n: f64) -> i8 {
    if n > FLAT {
        1
    } else if n < -FLAT {
        -1
    } else {
        0
    }
}

/// How well the values of a status column sort the rows by the sign of their result: the share
/// of rows whose sign is the usual one of their value, and what each value stands for.
fn purity(results: &[Option<f64>], statuses: &[&str]) -> Option<(f64, Vec<(String, Verdict)>)> {
    let mut groups: Vec<(&str, [usize; 3])> = Vec::new();
    for (result, status) in results.iter().zip(statuses) {
        let (Some(result), false) = (result, status.is_empty()) else { continue };
        let slot = if let Some(slot) = groups.iter().position(|(value, _)| value == status) {
            slot
        } else {
            groups.push((status, [0; 3]));
            groups.len() - 1
        };
        let class = usize::try_from(sign_of(*result) + 1).unwrap_or(1);
        groups[slot].1[class] += 1;
    }
    let total: usize = groups.iter().map(|(_, counts)| counts.iter().sum::<usize>()).sum();
    if total == 0
        || groups.len() < MIN_STATUS_VALUES
        || groups.len() > MAX_STATUS_VALUES
        || !repeats_enough(&groups, total)
    {
        return None;
    }
    let mut agreeing = 0;
    let mut values = Vec::new();
    for (value, counts) in &groups {
        // The class with the most rows; on a tie the one nearest to breakeven.
        let dominant = [1_usize, 0, 2].into_iter().max_by_key(|&class| counts[class]).unwrap_or(1);
        agreeing += counts[dominant];
        let verdict = match dominant {
            0 => Verdict::Loss,
            2 => Verdict::Win,
            _ => Verdict::BreakEven,
        };
        values.push(((*value).to_owned(), verdict));
    }
    let has = |wanted: Verdict| values.iter().any(|(_, verdict)| *verdict == wanted);
    (has(Verdict::Win) && has(Verdict::Loss) && agreeing * PURITY_DEN >= total * PURITY_NUM).then(|| {
        (f64::from(u32::try_from(agreeing).unwrap_or(u32::MAX)) / f64::from(u32::try_from(total).unwrap_or(1)), values)
    })
}

/// A status column repeats its values: most rows share theirs with another row (a column whose
/// values are all different sorts the rows perfectly and means nothing).
fn repeats_enough(groups: &[(&str, [usize; 3])], total: usize) -> bool {
    let shared: usize = groups.iter().map(|(_, counts)| counts.iter().sum::<usize>()).filter(|n| *n >= 2).sum();
    shared * REPEAT_DEN >= total * REPEAT_NUM
}

/// The result of each row in column `column`, when the cell is a number.
fn results_of(sheet: &Sheet, column: usize) -> Vec<Option<f64>> {
    sheet.rows.iter().map(|row| parse_number(row.cell(column), sheet.locale.comma)).collect()
}

fn statuses_of(sheet: &Sheet, column: usize) -> Vec<&str> {
    sheet.rows.iter().map(|row| row.cell(column)).collect()
}

/// Finds the result column (money or R) and the status column that goes with it. The result is
/// a numeric column with both gains and losses; among several, the one that a status column
/// sorts best. It is money when the file has prices or a volume to go with it, R otherwise
/// (small, mostly round numbers). The status column's values are learned from the signs.
fn find_results(
    sheet: &Sheet,
    profiles: &[Profile<'_>],
    fields: &mut Vec<(Field, usize)>,
    ignored: &[usize],
) -> Option<(usize, Vec<(String, Verdict)>)> {
    let width = profiles.len();
    let free =
        |fields: &[(Field, usize)], c: usize| !ignored.contains(&c) && fields.iter().all(|&(_, taken)| taken != c);
    let statuses: Vec<usize> = (0..width)
        .filter(|&c| {
            free(fields, c) && {
                let values = &profiles[c].values;
                let mut distinct: Vec<&str> = values.clone();
                distinct.sort_unstable();
                distinct.dedup();
                (MIN_STATUS_VALUES..=MAX_STATUS_VALUES).contains(&distinct.len())
                    && distinct.iter().all(|d| d.chars().count() <= 40)
            }
        })
        .collect();
    let best_status = |results: &[Option<f64>], taken: &[usize]| {
        statuses
            .iter()
            .copied()
            .filter(|c| !taken.contains(c))
            .filter_map(|c| purity(results, &statuses_of(sheet, c)).map(|(score, values)| (score, c, values)))
            .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)))
    };

    let has_prices =
        [Field::Volume, Field::OpenPrice, Field::ClosePrice].iter().any(|field| fields.iter().any(|(f, _)| f == field));
    let existing = fields.iter().find(|(f, _)| matches!(f, Field::Profit | Field::ResultR)).map(|&(_, c)| c);
    let result_column = existing.or_else(|| {
        let candidates: Vec<(usize, Vec<f64>)> = (0..width)
            .filter(|&c| free(fields, c))
            .filter_map(|c| numbers_of(&profiles[c].values, sheet).map(|numbers| (c, numbers)))
            .filter(|(_, numbers)| has_both_signs(numbers))
            .collect();
        let scored: Vec<(usize, f64)> = candidates
            .iter()
            .map(|(c, _)| (*c, best_status(&results_of(sheet, *c), &[*c]).map_or(0.0, |(score, ..)| score)))
            .collect();
        let best = scored.iter().map(|(_, score)| *score).fold(0.0_f64, f64::max);
        // A name that fits (R, risk/reward, result…) settles a tie between equally good columns.
        let named = |c: usize| {
            let key = &profiles[c].key;
            [Field::ResultR, Field::Profit].iter().any(|f| f.aliases().iter().any(|a| fold(a) == *key))
        };
        let chosen = scored
            .iter()
            .filter(|(_, score)| (*score - best).abs() < 1e-9)
            .max_by_key(|(c, _)| (named(*c), std::cmp::Reverse(*c)))
            .map(|(c, _)| *c);
        if let Some(column) = chosen {
            let numbers = candidates.iter().find(|(c, _)| *c == column).map_or_default(|(_, n)| n.as_slice());
            let field = if looks_like_r(numbers) && !has_prices { Field::ResultR } else { Field::Profit };
            fields.push((field, column));
        }
        chosen
    });
    let results = results_of(sheet, result_column?);
    best_status(&results, &[result_column?]).map(|(_, column, values)| (column, values))
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

    /// Rows of a journal kept in R: the names say nothing, the contents do.
    const IN_R: &str = "c1,c2,c3,c4,c5,c6,c7\n\
        x1,BTC/USD,Buy (Long),\"April 16, 2026\",2,A,image%201.png\n\
        x2,XAU/USD,Sell (Short),\"April 15, 2026\",-1,B,image%202.png\n\
        x3,BTC/USD,Buy (Long),\"April 14, 2026\",0,C,image%203.png\n\
        x4,USD/JPY,Sell (Short),\"April 13, 2026\",0.6,A,image%204.png\n\
        x5,BTC/USD,Buy (Long),\"April 12, 2026\",-0.2,B,image%205.png\n\
        x6,XAU/USD,Buy (Long),\"April 11, 2026\",3,A,image%206.png\n\
        x7,XAU/USD,Sell (Short),\"April 10, 2026\",-1,B,image%207.png\n\
        x8,USD/JPY,Buy (Long),\"April 9, 2026\",0,C,image%208.png\n";

    #[test]
    fn finds_every_role_of_a_journal_in_r_from_the_contents() {
        let found = layout(IN_R);
        assert_eq!(found.column(Field::Symbol), Some(1));
        assert_eq!(found.column(Field::Side), Some(2));
        assert_eq!(found.column(Field::OpenTime), Some(3));
        assert_eq!(found.column(Field::ResultR), Some(4), "small round numbers with losses: R");
        assert_eq!(found.column(Field::Screenshot), Some(6));
        let (status, values) = found.outcome.expect("the status column sorts the rows by the sign of R");
        assert_eq!(status, 5);
        let of = |name: &str| values.iter().find(|(value, _)| value == name).map(|(_, verdict)| *verdict);
        assert_eq!(of("A"), Some(Verdict::Win));
        assert_eq!(of("B"), Some(Verdict::Loss));
        assert_eq!(of("C"), Some(Verdict::BreakEven));
        assert!(found.properties.contains(&5), "the status stays a property");
    }

    #[test]
    fn money_results_with_prices_are_a_profit_not_r() {
        let found = layout(
            "d,s,p,entry,exit,lot,x\n\
             2026-11-20 09:00,EURUSD,Buy,1.0850,1.0900,0.5,12\n\
             2026-11-21 09:00,EURUSD,Sell,1.0900,1.0950,0.5,-5\n\
             2026-11-22 09:00,EURUSD,Buy,1.0950,1.1000,0.5,10\n",
        );
        assert_eq!(found.column(Field::ResultR), None);
        assert_eq!(found.column(Field::Profit), Some(6));
    }

    #[test]
    fn a_ratio_that_is_never_negative_is_not_a_result() {
        let found = layout(
            "Date,Pair,Side,Risk/Reward\n\
             2026-11-20,EURUSD,Buy,2\n2026-11-21,EURUSD,Sell,1.5\n2026-11-22,EURUSD,Buy,3\n",
        );
        assert_eq!(found.column(Field::ResultR), None);
        assert!(found.properties.contains(&3));
    }
}
