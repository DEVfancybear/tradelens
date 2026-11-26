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

/// What a glyph of a figure takes in the mono font (`JetBrains Mono` at 14 pt is 8.4 wide), with room.
const FIGURE_GLYPH: f32 = 8.8;
/// What a glyph of a column title takes (12 pt, medium), with room.
const TITLE_GLYPH: f32 = 7.0;
/// Between the figure columns when the card has room.
const COLUMN_GAP: f32 = 20.0;
/// The least between them before the columns of a row wrap onto a second line.
const COLUMN_GAP_MIN: f32 = 6.0;
/// The least room a name keeps beside the figures before it moves above them.
const NAME_MIN: f32 = 96.0;

/// The width of every figure column: what its widest figure or its title needs, so that nothing is cut.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Widths {
    trades: f32,
    win_rate: f32,
    net: f32,
    profit_factor: f32,
    avg_r: f32,
}

impl Widths {
    /// The widths the `rows` and the titles ask for; `net_title` heads the column of results.
    fn of(rows: &[BreakdownRow], net_title: &str) -> Self {
        let glyphs = |count: usize, glyph: f32| f32::from(u16::try_from(count).unwrap_or(u16::MAX)) * glyph;
        let column = |title: &str, figure: fn(&BreakdownRow) -> &str| {
            let widest = rows.iter().map(|row| figure(row).chars().count()).max().unwrap_or(0);
            glyphs(widest, FIGURE_GLYPH).max(glyphs(title.chars().count(), TITLE_GLYPH) + 2.0).ceil()
        };
        Self {
            trades: column("Lệnh", |row| &row.trades),
            win_rate: column("Win", |row| &row.win_rate),
            net: column(net_title, |row| &row.net),
            profit_factor: column("PF", |row| &row.profit_factor),
            avg_r: column("R TB", |row| &row.avg_r),
        }
    }

    fn total(self) -> f32 {
        self.trades + self.win_rate + self.net + self.profit_factor + self.avg_r
    }

    /// The width of the figures of a row with the closest gaps they keep before they wrap.
    fn least(self) -> f32 {
        self.total() + 4.0 * COLUMN_GAP_MIN
    }

    /// The width of the figures of a row when the card has room for the full gaps.
    fn preferred(self) -> f32 {
        self.total() + 4.0 * COLUMN_GAP
    }
}

/// The figures of a row (or of the header): the columns at their widths, a full gap apart when the card
/// has room, closer when it has less, and wrapped onto a second line when it has less than that. The
/// group asks for the room of the closest gaps only, so that the card beside it keeps its place in the
/// page's columns, and grows into the full gaps with what the card has to spare.
fn figures(widths: Widths) -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_x(px(COLUMN_GAP_MIN))
        .basis(px(widths.least()))
        .flex_grow(1.0)
        .flex_shrink(1.0)
        .min_w(px(0.0))
        .max_w(px(widths.preferred()))
}

/// A line of the table: the name takes what the figures leave, never less than [`NAME_MIN`] (or it
/// moves above them), and the figures stand at the right. The wrapped lines sit in the middle of the
/// row's height: a wrapping row packs them at its top otherwise, against the rule above it.
fn line(name: Text, figures: View) -> View {
    view().flex_row().flex_wrap().items_center().content_center().gap_x(px(COLUMN_GAP_MIN)).child(name).child(figures)
}

/// The name column: one line, cut with an ellipsis only when the name is longer than the room.
fn name_of(content: Text) -> Text {
    content.flex_1().min_w(px(NAME_MIN)).truncate()
}

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

fn heading(title: &'static str, width: f32) -> Text {
    text(title)
        .w(px(width))
        .flex_shrink_0()
        .text_xs()
        .text_right()
        .number_of_lines(1)
        .text_color(move || theme().muted_foreground)
}

/// The table of a breakdown; `name_title` heads the first column and `net_title` the column of
/// results. `rows` is read again when it changes, and an empty one shows `empty` instead. The figure
/// columns are as wide as their widest figure or title, so none is cut, and the name takes the rest.
pub(crate) fn breakdown_table(
    name_title: &'static str,
    net_title: impl Fn() -> &'static str + 'static,
    rows: impl Fn() -> Vec<BreakdownRow> + 'static,
    empty: &'static str,
) -> impl IntoElement {
    dynamic(move || {
        let rows = rows();
        let net_title = net_title();
        untracked(|| {
            let widths = Widths::of(&rows, net_title);
            let header = line(
                name_of(text(name_title).text_xs().text_color(move || theme().muted_foreground)),
                figures(widths)
                    .child(heading("Lệnh", widths.trades))
                    .child(heading("Win", widths.win_rate))
                    .child(heading(net_title, widths.net))
                    .child(heading("PF", widths.profit_factor))
                    .child(heading("R TB", widths.avg_r)),
            )
            .pb_2()
            .border_b_1()
            .border_color(move || theme().border);
            let body = if rows.is_empty() {
                empty_state(empty, "").into_element()
            } else {
                view()
                    .children(rows.into_iter().map(|row| {
                        let net = row.net;
                        line(
                            name_of(text(row.name).text_sm().font_weight(FontWeight::MEDIUM)),
                            figures(widths)
                                .child(cell(row.trades, widths.trades))
                                .child(cell(row.win_rate, widths.win_rate))
                                .child(
                                    view()
                                        .w(px(widths.net))
                                        .flex_shrink_0()
                                        .items_end()
                                        .child(pnl_text(move || net.clone()).text_sm().number_of_lines(1)),
                                )
                                .child(cell(row.profit_factor, widths.profit_factor))
                                .child(cell(row.avg_r, widths.avg_r)),
                        )
                        .min_h(px(40.0))
                        .border_b_1()
                        .border_color(move || theme().border.opacity(0.5))
                    }))
                    .into_element()
            };
            view().child(header).child(body).into_element()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(trades: &str, win: &str, net: &str, pf: &str, avg: &str) -> BreakdownRow {
        BreakdownRow {
            name: "EURUSD".into(),
            trades: trades.into(),
            win_rate: win.into(),
            net: net.into(),
            profit_factor: pf.into(),
            avg_r: avg.into(),
        }
    }

    #[test]
    fn a_column_is_as_wide_as_its_widest_figure_or_its_title() {
        let rows = [row("5", "100.0%", "+10.8R", "50+", "+1.54R"), row("12", "50.0%", "\u{2212}1.2R", "1.86", "-0.5R")];
        let widths = Widths::of(&rows, "Tổng R");
        let glyphs = |n: f32| (n * FIGURE_GLYPH).ceil();
        assert!(widths.win_rate >= glyphs(6.0), "100.0% fits: {widths:?}");
        assert!(widths.net >= glyphs(6.0), "+10.8R fits: {widths:?}");
        assert!(widths.avg_r >= glyphs(6.0), "+1.54R fits: {widths:?}");
        assert!(widths.net >= (6.0_f32 * TITLE_GLYPH).ceil(), "the title Tổng R fits: {widths:?}");
        assert!(widths.profit_factor >= glyphs(4.0), "1.86 fits: {widths:?}");
        let empty = Widths::of(&[], "Net");
        assert!(empty.trades >= (4.0_f32 * TITLE_GLYPH).ceil(), "a table with no rows still fits its titles");
    }
}
