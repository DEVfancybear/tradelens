//! Layout parts the pages share: the page frame, cards, stat tiles, placeholders.

use nsh_base::theme;
use nsh_core::prelude::*;

/// A page: its title and summary over a scrolling body.
pub(crate) fn page(title: &'static str, summary: &'static str, body: impl IntoElement) -> impl IntoElement {
    view()
        .flex_1()
        .child(
            view()
                .px_8()
                .pt_6()
                .pb_4()
                .gap_1()
                .border_b_1()
                .border_color(move || theme().border)
                .child(text(title).text_2xl().font_weight(FontWeight::SEMIBOLD))
                .child(text(summary).text_sm().text_color(move || theme().muted_foreground)),
        )
        .child(scroll().flex_1().child(view().px_8().py_6().gap_6().child(body)))
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
        .child(text(title).text_sm().font_weight(FontWeight::SEMIBOLD).text_color(move || theme().card_foreground))
        .child(body)
}

/// A headline number: its label over its value.
pub(crate) fn stat_tile(label: &'static str, value: &'static str) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(160.0))
        .p_4()
        .gap_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(text(label).text_xs().text_color(move || theme().muted_foreground))
        .child(text(value).text_xl().font_weight(FontWeight::SEMIBOLD).tabular_nums())
}

/// What a page will hold, until it is built: one line per block.
pub(crate) fn planned(blocks: &'static [&'static str]) -> impl IntoElement {
    card(
        "Sẽ có ở bản tới",
        view().gap_2().children(blocks.iter().map(|line| {
            view()
                .flex_row()
                .gap_2()
                .child(text("•").text_color(move || theme().primary))
                .child(text(*line).text_sm().flex_1().text_color(move || theme().muted_foreground))
        })),
    )
}
