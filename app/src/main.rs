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
        platform::AppSource::Rust(platform::Root::of::<nsh_app_tradelens::App>()),
        platform::AppOptions {
            title: "TradeLens".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: "vn.dat.tradelens".to_owned(),
            size: (1280, 800),
            ..Default::default()
        },
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("TradeLens is a desktop app: build it on Windows, macOS or Linux.");
