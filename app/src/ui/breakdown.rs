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
    /// Signed money: its sign colours it.
    pub(crate) net: String,
    pub(crate) profit_factor: String,
    pub(crate) avg_r: String,
}

const TRADES_WIDTH: f32 = 56.0;
const RATE_WIDTH: f32 = 72.0;
const NET_WIDTH: f32 = 112.0;
const FACTOR_WIDTH: f32 = 56.0;
const R_WIDTH: f32 = 72.0;

fn cell(text_of: String, width: f32) -> Text {
    text(text_of).w(px(width)).text_sm().text_right().font_family(tokens::mono).tabular_nums().number_of_lines(1)
}

/// The table of a breakdown; `name_title` heads the first column. `rows` is read again when it
/// changes, and an empty one shows `empty` instead.
pub(crate) fn breakdown_table(
    name_title: &'static str,
    rows: impl Fn() -> Vec<BreakdownRow> + 'static,
    empty: &'static str,
) -> impl IntoElement {
    let heading = |title: &'static str, width: f32| {
        text(title).w(px(width)).text_xs().text_right().text_color(move || theme().muted_foreground)
    };
    let header = view()
        .flex_row()
        .items_center()
        .gap_3()
        .pb_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(text(name_title).flex_1().text_xs().text_color(move || theme().muted_foreground))
        .child(heading("Lệnh", TRADES_WIDTH))
        .child(heading("Win rate", RATE_WIDTH))
        .child(heading("Net", NET_WIDTH))
        .child(heading("PF", FACTOR_WIDTH))
        .child(heading("R TB", R_WIDTH));
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
                        .items_center()
                        .gap_3()
                        .min_h(px(40.0))
                        .border_b_1()
                        .border_color(move || theme().border.opacity(0.5))
                        .child(
                            text(row.name)
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .number_of_lines(1),
                        )
                        .child(cell(row.trades, TRADES_WIDTH))
                        .child(cell(row.win_rate, RATE_WIDTH))
                        .child(view().w(px(NET_WIDTH)).items_end().child(pnl_text(move || net.clone())))
                        .child(cell(row.profit_factor, FACTOR_WIDTH))
                        .child(cell(row.avg_r, R_WIDTH))
                }))
                .into_element()
        })
    }))
}
