//! The language of the app: chosen in the settings, shown at once on every screen, kept for the
//! next run.
//!
//! The language is the process's, so every check that changes it lives in this one test, in a test
//! binary of its own: the screen tests of the other binaries read Vietnamese.

#[expect(dead_code, reason = "this file turns no toggle on")]
mod common;
mod support;

use nsh_testing::Headless;
use support::{WAIT, wait_until};

/// Chooses `language` (its own name, "English" or "Tiếng Việt") in the settings' drop-down.
fn choose(app: &mut Headless, drop_down: &str, language: &str) {
    let trigger = format!("role:combobox[{drop_down}]");
    app.wait_for_within(&trigger, WAIT).expect("the language drop-down shows");
    // The trigger shows the chosen language: the list is open once one more shows.
    let shown = |screen: &nsh_testing::Screen| screen.count(language).unwrap_or(0);
    let closed = shown(app.screen());
    common::press_until(app, &trigger, |screen| shown(screen) > closed);
    let choice = if closed == 0 { language.to_owned() } else { format!("{language} >> nth={closed}") };
    app.press(&choice).expect("the language is a choice of the list");
}

#[test]
fn the_language_chosen_in_the_settings_turns_every_screen_and_comes_back_on_the_next_run() {
    let fixture = support::Fixture::new();
    let mut app = fixture.start(1280.0, 1000.0, |_| {});
    common::press_until(&mut app, "role:button[Cài đặt]", |screen| screen.shows("Chế độ màu"));

    choose(&mut app, "Ngôn ngữ", "English");
    app.wait_for_within("Color mode", WAIT).expect("the settings are in English at once");
    for sidebar in ["Overview", "Trades", "P/L calendar", "Daily journal", "Settings"] {
        app.wait_for_within(sidebar, WAIT).unwrap_or_else(|_| panic!("the sidebar reads {sidebar:?}"));
    }
    assert!(!app.screen().shows("Chế độ màu"), "no Vietnamese is left on the settings");
    let db = fixture.db();
    wait_until("the language being stored", || db.setting("language").expect("reads").as_deref() == Some("en"));

    common::press_until(&mut app, "role:button[Trades]", |screen| !screen.shows("Color mode"));
    common::press_until(&mut app, "role:button[Settings]", |screen| screen.shows("Color mode"));
    drop(app);

    // The next run opens in English.
    let mut app = fixture.start(1280.0, 1000.0, |_| {});
    app.wait_for_within("Overview", WAIT).expect("the next run is in English");
    common::press_until(&mut app, "role:button[Settings]", |screen| screen.shows("Color mode"));

    choose(&mut app, "Language", "Tiếng Việt");
    app.wait_for_within("Chế độ màu", WAIT).expect("back in Vietnamese at once");
    app.wait_for_within("Tổng quan", WAIT).expect("the sidebar is in Vietnamese again");
    wait_until("Vietnamese being stored", || db.setting("language").expect("reads").as_deref() == Some("vi"));
}
