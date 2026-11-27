//! A row of tabs with an underline that follows the selected one (analytics, the form editor's forms).
//! Built on `nsh_base`'s `tabs` and `tab`, which draw nothing until styled here.

use nsh_base::{tab, tabs, theme};
use nsh_core::prelude::*;

/// A tab list over `options`: exactly one is selected, `value` says which and `on_change` hears
/// the one pressed (or moved to with the arrow keys). `group` names the list for screen
/// readers; each tab reads "group: label".
pub(crate) fn tab_bar<T: Copy + PartialEq + 'static, L: Into<SharedString>>(
    group: impl Into<SharedString>,
    options: &'static [T],
    label: fn(T) -> L,
    value: impl Fn() -> T + Copy + 'static,
    on_change: impl Fn(T) + Copy + 'static,
) -> impl IntoElement {
    let group: SharedString = group.into();
    tabs()
        .accessibility_label(group.clone())
        .flex_row()
        .flex_wrap()
        .gap_1()
        .border_b_1()
        .border_color(move || theme().border)
        .indicator_style(|s| s.bottom(px(0.0)).h(px(2.0)).bg(move || theme().primary))
        .children(options.iter().map(move |&option| {
            let on = move || value() == option;
            tab()
                .cursor_pointer()
                .selected(on)
                .on_press(move || on_change(option))
                .accessibility_label(format!("{group}: {}", label(option).into()))
                .px_3()
                .py_2()
                .rounded_t(px(6.0))
                .hover(|s| s.bg(move || theme().accent))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .child(
                    text(label(option).into())
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .style_when(on, |s| s.font_weight(FontWeight::SEMIBOLD))
                        .text_color(move || if on() { theme().foreground } else { theme().muted_foreground }),
                )
        }))
}
