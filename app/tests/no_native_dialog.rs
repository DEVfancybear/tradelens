//! The app draws its own modals inside the window (`ui::modal`); the native dialog layer of NSH
//! (`nsh_base::dialog()`) does not paint on Windows, so no screen may use it. A source check, since
//! a headless run cannot see the real window.

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

#[test]
fn no_screen_presents_the_native_dialog_layer() {
    let mut files = Vec::new();
    sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    assert!(files.len() > 20, "the source files were found");
    for file in files {
        let code = fs::read_to_string(&file).expect("the file is readable");
        for (number, line) in code.lines().enumerate() {
            let line = line.trim_start();
            if line.starts_with("//") {
                continue;
            }
            // `alert_dialog()` (the system's alert, for destructive questions) is allowed.
            let native = line
                .match_indices("dialog()")
                .any(|(at, _)| !line[..at].ends_with("alert_") && !line[..at].ends_with("pending_"));
            assert!(!native, "{}:{} uses nsh_base::dialog(): use ui::modal", file.display(), number + 1);
        }
    }
}
