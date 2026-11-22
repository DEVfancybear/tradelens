//! A row of tabs with an underline under the open one (`nsh_base::toggle_group`), for the
//! pages that split a subject in views (Phân tích).

use nsh_base::{theme, toggle, toggle_group};
use nsh_core::prelude::*;

/// Tabs of which exactly one is open. `group` names the set for screen readers: each tab reads
/// "group: label".
pub(crate) fn tab_bar<T: Copy + PartialEq + 'static>(
    group: &'static str,
    options: &'static [T],
    label: fn(T) -> &'static str,
    value: impl Fn() -> T + Copy + 'static,
    on_change: impl Fn(T) + Copy + 'static,
) -> impl IntoElement {
    toggle_group().flex_row().flex_wrap().gap_1().border_b_1().border_color(move || theme().border).children(
        options.iter().map(move |&option| {
            let open = move || value() == option;
            toggle()
                .pressed(open)
                .on_change(move |_: &bool| on_change(option))
                .accessibility_label(format!("{group}: {}", label(option)))
                .h(px(42.0))
                .px_4()
                .items_center()
                .justify_center()
                .hover(|s| s.opacity(0.85))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .child(
                    text(label(option))
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .style_when(open, |s| s.font_weight(FontWeight::SEMIBOLD))
                        .text_color(move || if open() { theme().foreground } else { theme().muted_foreground }),
                )
                .child(
                    view()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(2.0))
                        .bg(move || theme().primary)
                        .opacity(move || if open() { 1.0_f32 } else { 0.0_f32 }),
                )
        }),
    )
}
