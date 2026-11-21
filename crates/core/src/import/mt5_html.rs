//! MT5 "Report → HTML" (Trade History Report). The terminal writes it in UTF-16 LE
//! with a BOM; older builds and re-saved files are UTF-8.

use crate::model::DealBatch;

use super::{ImportError, table};

/// Decodes the bytes and parses the report.
///
/// # Errors
///
/// [`ImportError`] when the content is not a readable report.
pub fn parse(bytes: &[u8]) -> Result<DealBatch, ImportError> {
    let html = decode(bytes);
    table::batch_from_rows(&rows(&html)?)
}

/// UTF-16 LE/BE with a BOM, else UTF-8 (lossy).
pub fn decode(bytes: &[u8]) -> String {
    let utf16 = |be: bool| {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, ..] => utf16(false),
        [0xFE, 0xFF, ..] => utf16(true),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// The text of every `<td>`/`<th>` of every `<tr>`.
///
/// # Errors
///
/// [`ImportError`] when the content is not a readable report.
fn rows(_html: &str) -> Result<table::Rows, ImportError> {
    Err(ImportError::NotImplemented("v0.1: tokenize <tr>/<td>, strip tags, decode entities, honour colspan"))
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_utf16_reports() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "<html>Deals</html>".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        assert_eq!(decode(&bytes), "<html>Deals</html>");
        assert_eq!(decode(b"plain"), "plain");
    }
}
