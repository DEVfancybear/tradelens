//! Runs TradeLens on the machine's native backend (`cargo run -p nsh-app-tradelens`).

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl std::process::Termination {
    env_logger::init();
    platform::run_app(
        platform::AppSource::Rust(nsh_app_tradelens::root()),
        platform::AppOptions {
            title: "TradeLens".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: nsh_app_tradelens::APP_ID.to_owned(),
            // A packaged runner reads `nsh\icon.ico`; this one is for `cargo run`.
            #[cfg(all(target_os = "windows", debug_assertions))]
            icon: Some(concat!(env!("CARGO_MANIFEST_DIR"), "/icon/icon.ico").into()),
            size: (1280, 800),
            ..Default::default()
        },
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("TradeLens is a desktop app: build it on Windows, macOS or Linux.");
