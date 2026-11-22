//! Page layouts the screens share.

use nsh_base::theme;
use nsh_core::prelude::*;

/// The narrowest the main column gets before the side column drops below it.
const MAIN_MIN: f32 = 520.0;
const SIDE_MIN: f32 = 300.0;
const SIDE_MAX: f32 = 440.0;

/// A card whose title has something at its right (a switch, a link) on the same line.
pub(crate) fn titled_card(
    title: impl IntoReactive<SharedString>,
    trailing: impl IntoElement,
    body: impl IntoElement,
) -> impl IntoElement {
    view()
        .p_5()
        .gap_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    text(title).text_sm().font_weight(FontWeight::SEMIBOLD).text_color(move || theme().card_foreground),
                )
                .child(trailing),
        )
        .child(body)
}

/// A main column and a side column, side by side; the side one wraps under the main one when
/// the window is narrow. The columns stack their children with the page's card spacing.
pub(crate) fn columns(main: impl IntoElement, side: impl IntoElement) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .items_start()
        .gap_6()
        .child(view().flex_grow(3.0).min_w(px(MAIN_MIN)).gap_6().child(main))
        .child(view().flex_grow(1.0).min_w(px(SIDE_MIN)).max_w(px(SIDE_MAX)).gap_6().child(side))
}
