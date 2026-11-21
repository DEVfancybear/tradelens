//! Motion tokens: every transition in the app takes its timing from here
//! (`DESIGN.md`, "Motion").

use std::time::Duration;

/// Hover washes, press feedback.
#[expect(dead_code, reason = "hover and press use the engine's own state timing until the shell animates")]
pub(crate) const FAST: Duration = Duration::from_millis(120);
/// Selection moving in the sidebar, tabs, segmented controls.
#[expect(dead_code, reason = "the sliding selection arrives with LayoutAnimation")]
pub(crate) const MOVE: Duration = Duration::from_millis(200);
/// Pages and panels appearing.
pub(crate) const ENTER: Duration = Duration::from_millis(240);
/// The scale of a pressed control.
pub(crate) const PRESS_SCALE: f32 = 0.97;
