//! Styled controls: the app's buttons and the sidebar's rows.

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use super::motion;

/// How much a button stands out.
#[expect(dead_code, reason = "the skeleton's pages use buttons from v0.1")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variant {
    /// The page's main action.
    Primary,
    /// Other actions.
    Secondary,
    /// Actions in toolbars and rows.
    Ghost,
}

/// The app's button.
#[expect(dead_code, reason = "the skeleton's pages use buttons from v0.1")]
pub(crate) fn action(label: &'static str, variant: Variant, on_press: impl Fn() + 'static) -> impl IntoElement {
    let (fill, ink) = match variant {
        Variant::Primary => (theme().primary, theme().primary_foreground),
        Variant::Secondary => (theme().secondary, theme().secondary_foreground),
        Variant::Ghost => (AdaptiveColor::TRANSPARENT, theme().foreground),
    };
    button()
        .on_press(on_press)
        .h(px(34.0))
        .px_4()
        .flex_row()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(fill)
        .hover(|s| s.opacity(0.9))
        .active(|s| s.scale(motion::PRESS_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.5))
        .child(text(label).text_sm().font_weight(FontWeight::MEDIUM).text_color(ink))
}

/// A row of the sidebar: a mark, a label, and the selection's fill and bar.
pub(crate) fn sidebar_row(
    glyph: &'static str,
    label: &'static str,
    selected: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .accessibility_label(label)
        .h(px(34.0))
        .px_3()
        .flex_row()
        .items_center()
        .gap_3()
        .rounded(move || theme().radius)
        .hover(|s| s.bg(move || theme().sidebar_accent))
        .active(|s| s.scale(motion::PRESS_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().sidebar_ring))
        .style_when(selected, |s| s.bg(move || theme().sidebar_accent))
        .child(
            view()
                .absolute()
                .left_0()
                .top(px(9.0))
                .w(px(3.0))
                .h(px(16.0))
                .rounded(px(1.5))
                .bg(move || theme().sidebar_primary)
                .opacity(move || if selected() { 1.0_f32 } else { 0.0 }),
        )
        .child(
            text(glyph)
                .w(px(18.0))
                .text_color(move || if selected() { theme().sidebar_primary } else { theme().muted_foreground }),
        )
        .child(
            text(label)
                .flex_1()
                .number_of_lines(1)
                .text_sm()
                .style_when(selected, |s| s.font_weight(FontWeight::SEMIBOLD))
                .text_color(move || theme().sidebar_foreground),
        )
}
