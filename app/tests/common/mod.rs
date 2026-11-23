//! Helpers the headless screen tests share.

use std::time::Duration;

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
