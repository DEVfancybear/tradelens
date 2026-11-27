//! The export as a table of text: encoding, delimiter, quotes and line numbers settled,
//! nothing interpreted yet.

use crate::import::{ImportError, mt5_html};

use super::cells::{CommaRole, Locale, detect_date_order};

/// One data row and the line of the file it starts on (the header is line 1).
#[derive(Debug)]
pub(super) struct Row {
    pub(super) line: usize,
    pub(super) cells: Vec<String>,
}

#[derive(Debug)]
pub(super) struct Sheet {
    pub(super) headers: Vec<String>,
    pub(super) rows: Vec<Row>,
    pub(super) locale: Locale,
}

impl Row {
    /// The trimmed text of a cell; empty when the row is shorter than the header.
    pub(super) fn cell(&self, column: usize) -> &str {
        self.cells.get(column).map_or("", |cell| cell.trim())
    }
}

impl Sheet {
    /// The non-empty trimmed cells of a column.
    pub(super) fn values(&self, column: usize) -> Vec<&str> {
        self.rows.iter().map(|row| row.cell(column)).filter(|cell| !cell.is_empty()).collect()
    }
}

/// Reads CSV text: UTF-8 (BOM or not) or UTF-16, `,` `;` or tab delimited, quoted fields
/// with embedded newlines.
///
/// # Errors
///
/// [`ImportError::Invalid`] when the file is empty, has no data rows or is not CSV.
pub(super) fn read(bytes: &[u8]) -> Result<Sheet, ImportError> {
    let text = mt5_html::decode(bytes);
    let text = text.trim_start_matches('\u{feff}');
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter_of(text))
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut headers: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    let (mut cursor, mut line) = (0_usize, 1_usize);
    for record in reader.records() {
        let record = record.map_err(|e| ImportError::Invalid(crate::tr!("core.notion.csv_error", error = e)))?;
        let cells: Vec<String> = record.iter().map(str::to_owned).collect();
        match &headers {
            None => {
                headers = Some(
                    cells
                        .iter()
                        .enumerate()
                        .map(|(i, h)| if h.trim().is_empty() { format!("Cột {}", i + 1) } else { h.trim().to_owned() })
                        .collect(),
                );
            }
            Some(_) if cells.iter().all(|c| c.trim().is_empty()) => {}
            Some(_) => {
                // The reader's own line count ignores the blank lines it skips, so count from the offset.
                let offset = record.position().and_then(|p| usize::try_from(p.byte()).ok()).unwrap_or(cursor);
                let offset = offset.max(cursor);
                let skipped = text[offset..].len() - text[offset..].trim_start_matches(['\r', '\n']).len();
                line += text[cursor..offset + skipped].matches('\n').count();
                cursor = offset + skipped;
                rows.push(Row { line, cells });
            }
        }
    }
    let headers = headers.ok_or_else(|| ImportError::Invalid(crate::tr!("core.notion.csv_empty")))?;
    if rows.is_empty() {
        return Err(ImportError::Invalid(crate::tr!("core.notion.csv_only_header")));
    }
    let delimiter = delimiter_of(text);
    let dates = detect_date_order(rows.iter().flat_map(|row| row.cells.iter().map(String::as_str)));
    let comma = if delimiter == b';' { CommaRole::Decimal } else { CommaRole::Thousands };
    Ok(Sheet { headers, rows, locale: Locale { dates, comma } })
}

/// A sheet from a table already in memory, as the Notion API returns it: the header row and
/// the data rows, cells flattened to text with `.` as the decimal point. The line of a row is
/// its place in the table (the header is line 1).
///
/// # Errors
///
/// [`ImportError::Invalid`] when there is no header or no data row.
pub(super) fn from_table(headers: &[String], rows: &[Vec<String>]) -> Result<Sheet, ImportError> {
    if headers.is_empty() {
        return Err(ImportError::Invalid(crate::tr!("core.notion.table_no_columns")));
    }
    let headers: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| if h.trim().is_empty() { format!("Cột {}", i + 1) } else { h.trim().to_owned() })
        .collect();
    let rows: Vec<Row> = rows
        .iter()
        .enumerate()
        .filter(|(_, cells)| cells.iter().any(|c| !c.trim().is_empty()))
        .map(|(i, cells)| Row { line: i + 2, cells: cells.clone() })
        .collect();
    if rows.is_empty() {
        return Err(ImportError::Invalid(crate::tr!("core.notion.table_no_rows")));
    }
    let dates = detect_date_order(rows.iter().flat_map(|row| row.cells.iter().map(String::as_str)));
    Ok(Sheet { headers, rows, locale: Locale { dates, comma: CommaRole::Thousands } })
}

/// The delimiter that splits the header line (outside quotes) into the most cells.
fn delimiter_of(text: &str) -> u8 {
    let (mut commas, mut semicolons, mut tabs) = (0_usize, 0_usize, 0_usize);
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            '\n' if !quoted => break,
            ',' if !quoted => commas += 1,
            ';' if !quoted => semicolons += 1,
            '\t' if !quoted => tabs += 1,
            _ => {}
        }
    }
    if semicolons > commas && semicolons >= tabs {
        b';'
    } else if tabs > commas {
        b'\t'
    } else {
        b','
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_quotes_newlines_bom_and_line_numbers() {
        let csv = "\u{feff}Name,Notes\n\"a, b\",\"line 1\nline 2\"\n\n\"say \"\"hi\"\"\",x\n";
        let sheet = read(csv.as_bytes()).unwrap();
        assert_eq!(sheet.headers, ["Name", "Notes"]);
        assert_eq!(sheet.rows.len(), 2);
        assert_eq!(sheet.rows[0].cells, ["a, b", "line 1\nline 2"]);
        assert_eq!(sheet.rows[0].line, 2);
        assert_eq!(sheet.rows[1].line, 5, "a quoted newline and a blank line push the next row down");
        assert_eq!(sheet.rows[1].cells[0], "say \"hi\"");
    }

    #[test]
    fn picks_the_delimiter_of_the_header() {
        assert_eq!(read(b"a;b;\"c,d\"\n1,5;2;3\n").unwrap().locale.comma, CommaRole::Decimal);
        assert_eq!(read(b"a\tb\n1\t2\n").unwrap().rows[0].cells, ["1", "2"]);
        assert_eq!(read(b"a,b\n1,2\n").unwrap().locale.comma, CommaRole::Thousands);
    }

    #[test]
    fn reads_utf16_and_refuses_empty_files() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "A,B\n1,2\n".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        assert_eq!(read(&bytes).unwrap().rows[0].cells, ["1", "2"]);
        assert!(read(b"").is_err());
        assert!(read(b"A,B\n").is_err());
    }
}
