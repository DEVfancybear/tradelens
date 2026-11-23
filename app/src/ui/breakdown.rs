//! A breakdown as a table: a name and what the trades under it made.

use nsh_base::theme;
use nsh_core::prelude::*;

use super::{chips::pnl_text, parts::empty_state, theme as tokens};

/// One line of a [`breakdown_table`], every figure already formatted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BreakdownRow {
    pub(crate) name: String,
    pub(crate) trades: String,
    pub(crate) win_rate: String,
    /// Signed result (money, or R when the trades have no money): its sign colours it.
    pub(crate) net: String,
    pub(crate) profit_factor: String,
    pub(crate) avg_r: String,
}

const TRADES_WIDTH: f32 = 32.0;
const RATE_WIDTH: f32 = 48.0;
const NET_WIDTH: f32 = 74.0;
const FACTOR_WIDTH: f32 = 36.0;
const R_WIDTH: f32 = 52.0;
/// Between the figures.
const FIGURE_GAP: f32 = 4.0;
/// What the figures of a row take together: the narrowest card a side column gives (300 less its
/// padding) holds them, and the name wraps above them when the card is narrower than name plus figures.
const FIGURES_WIDTH: f32 = TRADES_WIDTH + RATE_WIDTH + NET_WIDTH + FACTOR_WIDTH + R_WIDTH + 4.0 * FIGURE_GAP;
/// The least room a name keeps beside the figures before it moves above them.
const NAME_MIN: f32 = 96.0;

fn cell(text_of: String, width: f32) -> Text {
    text(text_of)
        .w(px(width))
        .flex_shrink_0()
        .text_sm()
        .text_right()
        .font_family(tokens::mono)
        .tabular_nums()
        .number_of_lines(1)
}

/// The name column: it takes what the figures leave, never less than [`NAME_MIN`] (or it moves above
/// them), and one line.
fn name_of(content: Text) -> Text {
    content.flex_grow(1.0).min_w(px(NAME_MIN)).number_of_lines(1)
}

/// The table of a breakdown; `name_title` heads the first column and `net_title` the column of
/// results. `rows` is read again when it changes, and an empty one shows `empty` instead.
pub(crate) fn breakdown_table(
    name_title: &'static str,
    net_title: impl Fn() -> &'static str + 'static,
    rows: impl Fn() -> Vec<BreakdownRow> + 'static,
    empty: &'static str,
) -> impl IntoElement {
    let heading = |title: &'static str, width: f32| {
        text(title)
            .w(px(width))
            .flex_shrink_0()
            .text_xs()
            .text_right()
            .number_of_lines(1)
            .text_color(move || theme().muted_foreground)
    };
    let header = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x(px(FIGURE_GAP))
        .pb_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(name_of(text(name_title).text_xs().text_color(move || theme().muted_foreground)))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_x(px(FIGURE_GAP))
                .w(px(FIGURES_WIDTH))
                .child(heading("Lệnh", TRADES_WIDTH))
                .child(heading("Win", RATE_WIDTH))
                .child(
                    text(net_title)
                        .w(px(NET_WIDTH))
                        .flex_shrink_0()
                        .text_xs()
                        .text_right()
                        .number_of_lines(1)
                        .text_color(move || theme().muted_foreground),
                )
                .child(heading("PF", FACTOR_WIDTH))
                .child(heading("R TB", R_WIDTH)),
        );
    view().child(header).child(dynamic(move || {
        let rows = rows();
        untracked(|| {
            if rows.is_empty() {
                return empty_state(empty, "").into_element();
            }
            view()
                .children(rows.into_iter().map(|row| {
                    let net = row.net;
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(FIGURE_GAP))
                        .min_h(px(40.0))
                        .border_b_1()
                        .border_color(move || theme().border.opacity(0.5))
                        .child(name_of(text(row.name).text_sm().font_weight(FontWeight::MEDIUM)))
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_x(px(FIGURE_GAP))
                                .w(px(FIGURES_WIDTH))
                                .child(cell(row.trades, TRADES_WIDTH))
                                .child(cell(row.win_rate, RATE_WIDTH))
                                .child(
                                    view()
                                        .w(px(NET_WIDTH))
                                        .flex_shrink_0()
                                        .items_end()
                                        .child(pnl_text(move || net.clone()).text_sm().number_of_lines(1)),
                                )
                                .child(cell(row.profit_factor, FACTOR_WIDTH))
                                .child(cell(row.avg_r, R_WIDTH)),
                        )
                }))
                .into_element()
        })
    }))
}
