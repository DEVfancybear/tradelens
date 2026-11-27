//! Reads the app's translations (`locales/`, rust-i18n's formats) as `nsh run` and `nsh package` do,
//! for the builds that are not theirs: `cargo run` and the headless tests. `lib.rs` includes the
//! JSON this writes (`$OUT_DIR/translations.json`).

use std::{env, fs, path::PathBuf};

fn main() {
    let app_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-changed=locales");
    for file in nsh_i18n::locale_files(&app_dir) {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    let translations = nsh_i18n::read_translations(&app_dir).unwrap_or_else(|error| panic!("{error}"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("translations.json");
    fs::write(&out, nsh_i18n::translations_json(&translations))
        .unwrap_or_else(|error| panic!("cannot write {}: {error}", out.display()));
}
