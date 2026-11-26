//! The private Python's install against files served on this machine: a package shaped like
//! python.org's embeddable zip, checked by size and SHA-256 before it is unpacked.

use std::fmt::Write as _;
use std::{
    fs,
    io::{Cursor, Write as _},
    path::Path,
    sync::Arc,
    thread,
};

use sha2::{Digest, Sha256};
use tradelens_mt5::runtime::{self, Asset, Source};

/// A zip of `files` (name, content).
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).expect("an entry");
        zip.write_all(data).expect("its bytes");
    }
    zip.finish().expect("the zip").into_inner()
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().fold(String::new(), |mut hex, byte| {
        write!(hex, "{byte:02x}").expect("writing to a String does not fail");
        hex
    })
}

/// Serves `body` at every path, on a port of its own; the URL of `name` on it.
fn serve(body: Vec<u8>, name: &str) -> String {
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("a local server"));
    let port = server.server_addr().to_ip().expect("an IP address").port();
    thread::spawn(move || {
        for request in server.incoming_requests() {
            drop(request.respond(tiny_http::Response::from_data(body.clone())));
        }
    });
    format!("http://127.0.0.1:{port}/{name}")
}

fn source(package: &[u8], url: String, sha256: Option<String>) -> Source {
    let python = Asset {
        url,
        sha256: sha256.unwrap_or_else(|| hex(package)),
        size: u64::try_from(package.len()).expect("small"),
    };
    Source { python, pip: Asset { url: String::new(), sha256: String::new(), size: 0 }, packages: Vec::new() }
}

const PTH: &[u8] = b"python314.zip\r\n.\r\n\r\n# Uncomment to run site.main() automatically\r\n#import site\r\n";

#[test]
fn the_embeddable_package_is_checked_unpacked_and_its_site_switched_on() {
    let package = zip_of(&[("python.exe", b"MZ"), ("python314._pth", PTH), ("Lib/extra.txt", b"x")]);
    let url = serve(package.clone(), "python-embed.zip");
    let url_again = url.clone();
    let dir = tempfile::tempdir().expect("a temp dir");
    let program = runtime::install_python(dir.path(), &source(&package, url, None)).expect("it installs");
    assert_eq!(program, runtime::executable(dir.path()));
    assert!(program.is_file(), "python.exe is in its folder");
    let pth = fs::read_to_string(runtime::folder(dir.path()).join("python314._pth")).expect("the ._pth");
    assert!(pth.contains("Lib\\site-packages") && pth.lines().any(|line| line == "import site"), "{pth}");
    assert!(runtime::folder(dir.path()).join("Lib").join("extra.txt").is_file(), "folders are unpacked too");
    assert_eq!(names(dir.path()), ["python"], "nothing but the Python is left: {:?}", names(dir.path()));
    let marker = fs::read_to_string(runtime::folder(dir.path()).join(".tradelens-python")).expect("the marker");
    assert_eq!(marker, runtime::VERSION, "the marker is written last, with the version");
    // This python.exe does not run: a second call takes it for broken and installs it again.
    fs::write(runtime::folder(dir.path()).join("stale.txt"), b"x").expect("a file of the broken install");
    runtime::install_python(dir.path(), &source(&package, url_again, None)).expect("a broken Python is replaced");
    assert!(!runtime::folder(dir.path()).join("stale.txt").exists(), "the broken install is gone");
}

#[test]
fn a_package_that_is_not_the_one_expected_is_refused_and_removed() {
    let package = zip_of(&[("python.exe", b"MZ"), ("python314._pth", PTH)]);
    let url = serve(package.clone(), "python-embed.zip");
    let dir = tempfile::tempdir().expect("a temp dir");
    let wrong = source(&package, url, Some("0".repeat(64)));
    let error = runtime::install_python(dir.path(), &wrong).expect_err("a wrong hash is refused");
    assert!(error.message().contains("không đúng"), "{error:?}");
    assert!(!runtime::executable(dir.path()).exists(), "nothing is installed");
    assert!(names(dir.path()).is_empty(), "nothing is left behind: {:?}", names(dir.path()));
}

#[test]
fn an_entry_that_would_leave_the_folder_is_refused() {
    let package = zip_of(&[("../outside.txt", b"x"), ("python314._pth", PTH)]);
    let url = serve(package.clone(), "python-embed.zip");
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = dir.path().join("runtime");
    let error = runtime::install_python(&root, &source(&package, url, None)).expect_err("the entry is refused");
    assert!(error.message().contains("đường dẫn lạ"), "{error:?}");
    assert!(!dir.path().join("outside.txt").exists(), "nothing is written outside");
    assert!(!runtime::executable(&root).exists());
    assert!(names(&root).is_empty(), "nothing is left behind: {:?}", names(&root));
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("the folder")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}
