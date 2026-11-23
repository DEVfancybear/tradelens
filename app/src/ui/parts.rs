//! Layout parts the pages share: the page frame, cards, stat tiles, property rows, empty
//! states and placeholders.

use nsh_base::theme;
use nsh_core::prelude::*;

use super::{
    chips::Tone,
    icons::{Icon, icon},
    theme::warning,
};

/// The page's header: title, summary and the actions at the right.
fn header(
    title: &'static str,
    summary: impl IntoReactive<SharedString>,
    actions: impl IntoElement,
) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_4()
        .px_8()
        .pt_6()
        .pb_4()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .gap_1()
                .min_w(px(0.0))
                .child(text(title).text_2xl().number_of_lines(1).font_weight(FontWeight::SEMIBOLD))
                .child(text(summary).text_sm().number_of_lines(1).text_color(move || theme().muted_foreground)),
        )
        .child(view().flex_row().flex_wrap().items_center().gap_3().child(actions))
}

/// A page: its title and summary over a scrolling body.
pub(crate) fn page(title: &'static str, summary: &'static str, body: impl IntoElement) -> impl IntoElement {
    page_with(title, summary, view(), body)
}

/// A page with actions beside its title and a summary that may change.
pub(crate) fn page_with(
    title: &'static str,
    summary: impl IntoReactive<SharedString>,
    actions: impl IntoElement,
    body: impl IntoElement,
) -> impl IntoElement {
    view()
        .flex_1()
        .child(header(title, summary, actions))
        .child(scroll().flex_1().child(view().px_8().py_6().gap_6().child(body)))
}

/// A page whose body fills what is left under the header and scrolls itself (a long table).
pub(crate) fn page_fill(
    title: &'static str,
    summary: impl IntoReactive<SharedString>,
    actions: impl IntoElement,
    body: impl IntoElement,
) -> impl IntoElement {
    view()
        .flex_1()
        .child(header(title, summary, actions))
        .child(view().flex_1().min_h(px(0.0)).px_8().pt_5().pb_6().gap_4().child(body))
}

/// A card with a title.
pub(crate) fn card(title: &'static str, body: impl IntoElement) -> impl IntoElement {
    view()
        .p_5()
        .gap_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(
            text(title)
                .text_sm()
                .number_of_lines(1)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().card_foreground),
        )
        .child(body)
}

/// A headline number: its label over its value and a note. The value's colour follows its tone.
pub(crate) fn stat_tile(
    label: &'static str,
    value: impl Fn() -> (String, Tone) + Clone + 'static,
    note: impl Fn() -> String + 'static,
) -> impl IntoElement {
    stat_tile_with(move || label, value, note)
}

/// [`stat_tile`] whose label follows the data: a figure that is in money for one set of trades and in
/// R for another.
pub(crate) fn stat_tile_with(
    label: impl Fn() -> &'static str + 'static,
    value: impl Fn() -> (String, Tone) + Clone + 'static,
    note: impl Fn() -> String + 'static,
) -> impl IntoElement {
    let shown = value.clone();
    view()
        .flex_1()
        .min_w(px(136.0))
        .p_4()
        .gap_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(text(label).text_xs().number_of_lines(1).text_color(move || theme().muted_foreground))
        .child(
            text(move || shown().0)
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .font_family(super::theme::mono)
                .tabular_nums()
                .number_of_lines(1)
                .text_color(move || match value().1 {
                    Tone::Profit => super::theme::profit(),
                    Tone::Loss => super::theme::loss(),
                    Tone::Neutral | Tone::Primary => theme().foreground,
                }),
        )
        .child(text(note).text_xs().number_of_lines(1).text_color(move || theme().muted_foreground))
}

/// A row of a property list: the property's name, then its control.
pub(crate) fn property_row(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    view()
        .flex_row()
        .items_start()
        .gap_3()
        .min_h(px(34.0))
        .child(text(label).w(px(124.0)).pt_2().text_sm().text_color(move || theme().muted_foreground))
        .child(view().flex_1().min_w(px(0.0)).child(control))
}

/// Nothing to show yet: one line saying why and what to do about it.
pub(crate) fn empty_state(
    title: impl IntoReactive<SharedString>,
    hint: impl IntoReactive<SharedString>,
) -> impl IntoElement {
    view()
        .items_center()
        .justify_center()
        .gap_1()
        .py_10()
        .child(text(title).text_sm().font_weight(FontWeight::MEDIUM))
        .child(text(hint).text_sm().text_color(move || theme().muted_foreground))
}

/// A line of warning on a tinted ground: a rule about to be, or already, broken.
pub(crate) fn warning_banner(message: impl IntoReactive<SharedString>) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Alert)
        .flex_row()
        .items_center()
        .gap_2()
        .p_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || warning().opacity(0.4))
        .bg(move || warning().opacity(0.12))
        .child(icon(Icon::Warning, 16.0).text_color(warning))
        .child(text(message).flex_1().text_xs().text_color(warning))
}
