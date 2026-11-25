//! Helpers the headless screen tests share.

use std::time::{Duration, Instant};

use nsh_core::Checked;
use nsh_testing::{Headless, Screen};

/// How long one press is given to show before it is taken as lost.
const PRESS_SHOWS_WITHIN: Duration = Duration::from_secs(2);

/// How many times a lost press is tried again.
const PRESS_TRIES: usize = 5;

/// How long a toggle may take to appear: its options can come from a database read that a slow
/// CI runner finishes long after the page is drawn.
const TOGGLE_APPEARS_WITHIN: Duration = Duration::from_secs(30);

/// Turns the toggle `selector` names on (a tag, a segmented option), pressing again when a press
/// is lost: a page draws its controls again once its data is read, and a press that lands then
/// reaches a control that is being replaced. Never presses a toggle that is already on, so an
/// option the page shows as chosen before its data is read must be waited past by the caller.
///
/// # Panics
///
/// When the toggle is not on screen, or does not turn on after a few presses.
pub(crate) fn toggle_on(app: &mut Headless, selector: &str) {
    let on = |screen: &Screen| screen.accessibility_state(selector).is_ok_and(|s| s.pressed == Some(Checked::True));
    app.wait_for_within(selector, TOGGLE_APPEARS_WITHIN).expect("the toggle shows");
    for _ in 0..PRESS_TRIES {
        if on(app.screen()) {
            return;
        }
        app.press(selector).expect("the toggle is a button");
        if app.wait_until("the toggle being on", PRESS_SHOWS_WITHIN, on).is_ok() {
            return;
        }
    }
    panic!("`{selector}` did not turn on after {PRESS_TRIES} presses");
}

/// How long `press_until` keeps trying before it gives up.
const PRESS_UNTIL_WITHIN: Duration = Duration::from_secs(30);

/// How long one press of `press_until` is given to have its effect before it is pressed again.
const EFFECT_SHOWS_WITHIN: Duration = Duration::from_millis(500);

/// Presses `selector` until `done` holds on the screen. A page draws its controls again when its
/// data is read or a write lands, so a control may not exist yet, or a press may reach one that
/// is being replaced; either way the press is repeated. Only for a control whose press is safe to
/// repeat and makes `done` true (a navigation, a button that opens a form), never for one that
/// toggles or submits.
///
/// # Panics
///
/// When `done` does not hold within [`PRESS_UNTIL_WITHIN`].
pub(crate) fn press_until(app: &mut Headless, selector: &str, done: impl Fn(&Screen) -> bool) {
    let started = Instant::now();
    let mut tried = false;
    while started.elapsed() < PRESS_UNTIL_WITHIN {
        // The first round presses whatever the screen shows: `done` may hold on the page the press
        // leaves. Later rounds stop once it holds, e.g. when the control is gone because it worked.
        if tried && done(app.screen()) {
            return;
        }
        tried = true;
        // A control that is not there yet is no failure: the next round looks again.
        if app.press(selector).is_ok() && app.wait_until("the press taking effect", EFFECT_SHOWS_WITHIN, &done).is_ok()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("pressing `{selector}` did not have its effect within {PRESS_UNTIL_WITHIN:?}");
}
