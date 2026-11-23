//! A card that is pressed to pick it: an account among the accounts, a day among the week's.

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use super::motion;

/// A card holding `body`; the one `selected` is outlined in the primary colour. `label` is what
/// screen readers say and what tests press.
pub fn selectable_card(
    label: impl Into<SharedString>,
    selected: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
    body: impl IntoElement,
) -> impl IntoElement {
    button()
        .cursor_pointer()
        .on_press(on_press)
        .accessibility_label(label.into())
        .p_4()
        .gap_2()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || if selected() { theme().primary } else { theme().border })
        .bg(move || theme().card)
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.scale(motion::PRESS_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(body)
}
