//! Every text the app shows is a key of `locales/`, read with `t!`, so it shows in the language the
//! trader chose. A source check: a Vietnamese string literal left in the program (outside its tests)
//! is a text that stays Vietnamese in English.

use std::{fs, path::Path};

/// Every `.rs` file under `dir`.
fn sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).expect("the source folder is readable") {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Whether `c` is a Latin letter with a mark (`ầ`, `ơ`, `đ`…), as Vietnamese words have and the app's
/// symbols (`—`, `·`, `…`, `▾`) do not. A word without marks (`Thua`) escapes this check.
fn vietnamese(c: char) -> bool {
    matches!(c, 'À'..='ỹ') && !matches!(c, '×' | '÷')
}

/// The string literals of `line` that hold a Vietnamese letter.
fn vietnamese_literals(line: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else { break };
        let literal = &after[..close];
        if literal.chars().any(vietnamese) {
            found.push(literal);
        }
        rest = &after[close + 1..];
    }
    found
}

/// The mark of a line whose Vietnamese is data the app stores, not a text it shows as its own.
const STORED: &str = "// i18n: stored data";

#[test]
fn no_screen_shows_a_vietnamese_text_written_in_its_code() {
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    assert!(files.len() > 20, "the source files were found");
    let mut left = Vec::new();
    for file in files {
        let code = fs::read_to_string(&file).expect("the file is readable");
        // What follows the first test module is not the program's.
        let program = code.split("#[cfg(test)]").next().unwrap_or_default();
        for (number, line) in program.lines().enumerate() {
            let line = line.trim_start();
            // Comments; logs, which are for the developer; and data the app stores as written (a
            // default account name), marked so on the line.
            if line.starts_with("//") || line.contains("log::") || line.contains(STORED) {
                continue;
            }
            for literal in vietnamese_literals(line) {
                left.push(format!("{}:{}: \"{literal}\"", file.display(), number + 1));
            }
        }
    }
    assert!(left.is_empty(), "texts written in the code instead of `locales/` (use t!):\n{}", left.join("\n"));
}

#[test]
fn the_check_finds_vietnamese_and_lets_symbols_through() {
    assert_eq!(vietnamese_literals(r#"text("Cài đặt").child("—")"#), ["Cài đặt"]);
    assert_eq!(vietnamese_literals(r#"format!("{} · {}…", a, b)"#), Vec::<&str>::new());
    assert_eq!(vietnamese_literals(r#"t!("settings.title")"#), Vec::<&str>::new());
}
