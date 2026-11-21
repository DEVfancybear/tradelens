//! MT5 "Report → HTML" (Trade History Report). The terminal writes it in UTF-16 LE
//! with a BOM; older builds and re-saved files are UTF-8.
//!
//! The report is a flat soup of `<tr>`/`<td>`/`<th>` with attributes in any case, cells
//! wrapped in `<div>`/`<b>`, `&nbsp;` for blanks and sometimes no closing tags. A small
//! tokenizer reads exactly that and nothing else: it does not build a DOM.

use std::borrow::Cow;

use crate::model::DealBatch;

use super::{ImportError, table};

/// Decodes the bytes and parses the report.
///
/// # Errors
///
/// [`ImportError`] when the content is not a readable report.
pub fn parse(bytes: &[u8]) -> Result<DealBatch, ImportError> {
    let html = decode(bytes);
    table::batch_from_rows(&rows(&html))
}

/// UTF-16 LE/BE with a BOM (or UTF-16 LE markup without one), else UTF-8 (lossy).
pub fn decode(bytes: &[u8]) -> String {
    let utf16 = |payload: &[u8], big_endian: bool| {
        let units: Vec<u16> = payload
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| if big_endian { u16::from_be_bytes(pair) } else { u16::from_le_bytes(pair) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        // A UTF-16 LE file that lost its BOM still starts with `<` and a zero byte.
        [b'<', 0, ..] => utf16(bytes, false),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// The longest entity body (`&#x1F600;` has 7) worth looking for a `;` in.
const ENTITY_SCAN: usize = 12;

/// An opening or closing tag: its lower-case name and the raw attribute text.
struct Tag<'a> {
    name: String,
    closing: bool,
    attrs: &'a str,
}

/// What starts at a `<`.
enum Markup<'a> {
    /// A tag and the byte length of its source.
    Tag(Tag<'a>, usize),
    /// A comment, doctype or processing instruction: skip this many bytes.
    Skip(usize),
    /// A lone `<` that is part of the text.
    Text,
}

/// A cell being filled: its text so far and the columns it spans.
struct Cell {
    text: String,
    span: usize,
}

#[derive(Default)]
struct Builder {
    rows: table::Rows,
    row: Option<Vec<String>>,
    cell: Option<Cell>,
}

impl Builder {
    fn text(&mut self, raw: &str) {
        if let Some(cell) = &mut self.cell {
            cell.text.push_str(&decode_entities(raw));
        }
    }

    fn tag(&mut self, tag: &Tag<'_>) {
        match (tag.name.as_str(), tag.closing) {
            ("tr", false) => {
                self.end_row();
                self.row = Some(Vec::new());
            }
            ("tr", true) | ("table", _) => self.end_row(),
            ("td" | "th", false) => {
                self.end_cell();
                self.row.get_or_insert_with(Vec::new);
                self.cell = Some(Cell { text: String::new(), span: colspan(tag.attrs) });
            }
            ("td" | "th", true) => self.end_cell(),
            ("br" | "p" | "div" | "li", _) => {
                if let Some(cell) = &mut self.cell {
                    cell.text.push(' ');
                }
            }
            _ => {}
        }
    }

    /// The cell's text goes in its first column; the columns it spans stay empty so the
    /// following cells keep their position.
    fn end_cell(&mut self) {
        let (Some(cell), Some(row)) = (self.cell.take(), self.row.as_mut()) else { return };
        row.push(cell.text.split_whitespace().collect::<Vec<_>>().join(" "));
        row.resize(row.len() + cell.span - 1, String::new());
    }

    fn end_row(&mut self) {
        self.end_cell();
        if let Some(row) = self.row.take()
            && !row.is_empty()
        {
            self.rows.push(row);
        }
    }

    fn finish(mut self) -> table::Rows {
        self.end_row();
        self.rows
    }
}

/// The text of every `<td>`/`<th>` of every `<tr>`, with `colspan` expanded so a cell's
/// index is its column. Text outside cells, comments, scripts and styles are dropped.
fn rows(html: &str) -> table::Rows {
    let mut builder = Builder::default();
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        builder.text(&rest[..lt]);
        rest = &rest[lt..];
        match markup(rest) {
            Markup::Tag(tag, len) => {
                builder.tag(&tag);
                rest = &rest[len..];
                if !tag.closing && (tag.name == "script" || tag.name == "style") {
                    let close = format!("</{}", tag.name);
                    rest = find_ignore_case(rest, &close).map_or("", |at| &rest[at..]);
                }
            }
            Markup::Skip(len) => rest = &rest[len..],
            Markup::Text => {
                builder.text("<");
                rest = &rest[1..];
            }
        }
    }
    builder.text(rest);
    builder.finish()
}

/// Reads what starts at the `<` at the head of `src`.
fn markup(src: &str) -> Markup<'_> {
    if src.starts_with("<!--") {
        return Markup::Skip(src.find("-->").map_or(src.len(), |end| end + 3));
    }
    if src.starts_with("<!") || src.starts_with("<?") {
        return Markup::Skip(src.find('>').map_or(src.len(), |end| end + 1));
    }
    let body = &src[1..];
    let (closing, body) = body.strip_prefix('/').map_or((false, body), |b| (true, b));
    let name_len = body.bytes().take_while(u8::is_ascii_alphanumeric).count();
    if name_len == 0 || !body.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Markup::Text;
    }
    let name = body[..name_len].to_ascii_lowercase();
    let after_name = &body[name_len..];
    let end = tag_end(after_name);
    let attrs = after_name[..end.min(after_name.len())].trim_end_matches('/');
    // `<` + optional `/` + name + attributes + `>` (when there is one).
    let consumed = src.len() - after_name.len() + (end + 1).min(after_name.len());
    Markup::Tag(Tag { name, closing, attrs }, consumed)
}

/// The byte index of the `>` that ends a tag, ignoring any inside quoted values.
fn tag_end(after_name: &str) -> usize {
    let mut quote: Option<u8> = None;
    for (i, b) in after_name.bytes().enumerate() {
        match (quote, b) {
            (Some(q), b) if b == q => quote = None,
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'>') => return i,
            _ => {}
        }
    }
    after_name.len()
}

/// The `colspan` of a cell, at least 1 and capped so a corrupt value cannot blow up memory.
fn colspan(attrs: &str) -> usize {
    const MAX_SPAN: usize = 256;
    attribute(attrs, "colspan").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1).clamp(1, MAX_SPAN)
}

/// The value of attribute `name` (any case, quoted or not).
fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let lower = attrs.to_ascii_lowercase();
    for (at, _) in lower.match_indices(name) {
        let boundary = lower[..at].chars().next_back().is_none_or(|c| c.is_whitespace() || c == '"' || c == '\'');
        let after = lower[at + name.len()..].trim_start();
        let Some(value_start) = after.strip_prefix('=') else { continue };
        if !boundary {
            continue;
        }
        // `lower` and `attrs` have the same byte layout (ASCII lower-casing).
        let offset = attrs.len() - value_start.len();
        let value = attrs[offset..].trim_start();
        return Some(match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or_default(),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or_default(),
        });
    }
    None
}

fn find_ignore_case(haystack: &str, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    haystack.as_bytes().windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle))
}

/// Replaces `&nbsp;`, the common named entities and `&#N;` / `&#xH;` by their characters.
/// An unknown or unterminated entity stays as written.
fn decode_entities(raw: &str) -> Cow<'_, str> {
    if !raw.contains('&') {
        return Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        if let Some((ch, len)) = entity(rest) {
            out.push(ch);
            rest = &rest[len..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The character of the entity at the head of `src` and the bytes it takes.
fn entity(src: &str) -> Option<(char, usize)> {
    let end = src.bytes().take(ENTITY_SCAN).position(|b| b == b';')?;
    let body = &src[1..end];
    let ch = match body.strip_prefix('#') {
        Some(number) => {
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)?
        }
        None => named_entity(body)?,
    };
    Some((ch, end + 1))
}

fn named_entity(name: &str) -> Option<char> {
    Some(match name {
        "nbsp" => '\u{a0}',
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "thinsp" => '\u{2009}',
        "ndash" => '–',
        "mdash" => '—',
        "minus" => '−',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "hellip" => '…',
        "middot" => '·',
        "times" => '×',
        "plusmn" => '±',
        "deg" => '°',
        "copy" => '©',
        "reg" => '®',
        "euro" => '€',
        "pound" => '£',
        "yen" => '¥',
        "cent" => '¢',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{decode, decode_entities, rows};

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|c| (*c).to_owned()).collect()
    }

    #[test]
    fn decodes_utf16_reports() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "<html>Deals</html>".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        assert_eq!(decode(&bytes), "<html>Deals</html>");
        assert_eq!(decode(b"plain"), "plain");
    }

    #[test]
    fn decodes_utf16_without_a_bom_and_utf8_with_one() {
        let bytes: Vec<u8> = "<b>Đ</b>".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&bytes), "<b>Đ</b>");
        let mut utf8 = vec![0xEF, 0xBB, 0xBF];
        utf8.extend("Tổng".as_bytes());
        assert_eq!(decode(&utf8), "Tổng");
    }

    #[test]
    fn decodes_named_and_numeric_entities() {
        assert_eq!(decode_entities("a&nbsp;b"), "a\u{a0}b");
        assert_eq!(decode_entities("&lt;sl&gt; &amp; &quot;x&quot;"), "<sl> & \"x\"");
        assert_eq!(decode_entities("&#65;&#x42;&#X43;"), "ABC");
        assert_eq!(decode_entities("1&#160;234.56"), "1\u{a0}234.56");
        // A decoded `&` does not start a second entity.
        assert_eq!(decode_entities("&amp;lt;"), "&lt;");
        // Unknown, unterminated and out-of-range entities are kept.
        assert_eq!(decode_entities("&bogus; AT&T &#xFFFFFFFF; &amp"), "&bogus; AT&T &#xFFFFFFFF; &amp");
    }

    #[test]
    fn expands_colspan_so_columns_stay_aligned() {
        let html = "<table><tr><td>a</td><td colspan=3></td><td>b</td></tr>\
                    <tr><TD COLSPAN=\"2\">c</TD><td colspan='0'>d</td><td colspan=x>e</td></tr></table>";
        assert_eq!(rows(html), vec![row(&["a", "", "", "", "b"]), row(&["c", "", "d", "e"])]);
    }

    #[test]
    fn strips_nested_tags_and_collapses_whitespace() {
        let html = "<tr align=right><td nowrap><div style=\"font: 8pt\"><b>2026.11.18</b>&nbsp;09:00:00</div></td>\
                    <th>Line<br>break\n  and   spaces</th><td>&nbsp;</td></tr>";
        assert_eq!(rows(html), vec![row(&["2026.11.18 09:00:00", "Line break and spaces", ""])]);
    }

    #[test]
    fn survives_missing_closing_tags_and_loose_markup() {
        let html = "<TABLE><TR><TD>a<TD>b<TR><TH>c<TH colspan=2>d\n<TR><TD>e</TABLE> trailing";
        assert_eq!(rows(html), vec![row(&["a", "b"]), row(&["c", "d", ""]), row(&["e"])]);
        // Cells without a row still form one.
        assert_eq!(rows("<td>x</td><td>y</td>"), vec![row(&["x", "y"])]);
    }

    #[test]
    fn ignores_comments_scripts_styles_and_text_outside_cells() {
        let html = "<!DOCTYPE html><html><head><style>td { x: '<td>nope</td>' }</style>\
                    <script>if (a < b) { document.write('<tr><td>no</td></tr>') }</script></head>\
                    <body>title<!-- <tr><td>hidden</td></tr> --><table><tr><td>1 &lt; 2</td></tr></table></body>";
        assert_eq!(rows(html), vec![row(&["1 < 2"])]);
    }

    #[test]
    fn a_greater_than_inside_a_quoted_attribute_does_not_end_the_tag() {
        let html = "<tr><td title=\"a>b\" colspan=\"2\">x</td><td>y</td></tr>";
        assert_eq!(rows(html), vec![row(&["x", "", "y"])]);
    }

    #[test]
    fn keeps_a_bare_less_than_in_text() {
        assert_eq!(rows("<tr><td>a < b</td><td>5<6</td></tr>"), vec![row(&["a < b", "5<6"])]);
    }
}
