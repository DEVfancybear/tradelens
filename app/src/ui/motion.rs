//! Motion tokens: every transition in the app takes its timing from here
//! (`DESIGN.md`, "Motion").

#![expect(dead_code, reason = "the skeleton's pages read these from v0.1")]

use std::time::Duration;

/// Hover washes, press feedback.
pub(crate) const FAST: Duration = Duration::from_millis(120);
/// Selection moving in the sidebar, tabs, segmented controls.
pub(crate) const MOVE: Duration = Duration::from_millis(200);
/// Pages and panels appearing.
pub(crate) const ENTER: Duration = Duration::from_millis(240);
/// The scale of a pressed control.
pub(crate) const PRESS_SCALE: f32 = 0.97;
