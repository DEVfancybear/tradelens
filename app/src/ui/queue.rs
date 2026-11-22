//! A row of the review queue: what needs a look, which trades, and the button that goes there.

use nsh_base::theme;
use nsh_core::prelude::*;

use super::controls::{Variant, action};

/// A queue row: `title` (`3 lệnh chưa ghi nhật ký`), a muted `detail` under it (the trades) and
/// the `action_label` button at the right, which runs `on_press`.
pub(crate) fn queue_row(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    action_label: &'static str,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .py_3()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_1()
                .child(text(title.into()).text_sm())
                .child(text(detail.into()).text_xs().text_color(move || theme().muted_foreground)),
        )
        .child(action(action_label, Variant::Secondary, on_press))
}
