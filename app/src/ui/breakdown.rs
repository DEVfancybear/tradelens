//! A breakdown as a table: a name and what the trades under it made.

use std::rc::Rc;

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

/// The titles over the figure columns (the net title follows the data).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Titles {
    trades: SharedString,
    net: SharedString,
    avg_r: SharedString,
}

impl Titles {
    fn new(net: SharedString) -> Self {
        Self { trades: t!("ui.breakdown.trades").into(), net, avg_r: t!("ui.breakdown.avg_r").into() }
    }
}

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
    /// The widths the `rows` and the `titles` ask for.
    fn of(rows: &[BreakdownRow], titles: &Titles) -> Self {
        let glyphs = |count: usize, glyph: f32| f32::from(u16::try_from(count).unwrap_or(u16::MAX)) * glyph;
        let column = |title: &str, figure: fn(&BreakdownRow) -> &str| {
            let widest = rows.iter().map(|row| figure(row).chars().count()).max().unwrap_or(0);
            glyphs(widest, FIGURE_GLYPH).max(glyphs(title.chars().count(), TITLE_GLYPH) + 2.0).ceil()
        };
        Self {
            trades: column(&titles.trades, |row| &row.trades),
            win_rate: column("Win", |row| &row.win_rate),
            net: column(&titles.net, |row| &row.net),
            profit_factor: column("PF", |row| &row.profit_factor),
            avg_r: column(&titles.avg_r, |row| &row.avg_r),
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

/// The name column: a name longer than the room (a session with its hours, in English) wraps onto a
/// second line, which the row's height holds, and is cut with an ellipsis only beyond it.
fn name_of(content: Text) -> Text {
    content.flex_1().min_w(px(NAME_MIN)).number_of_lines(2u32).text_overflow(nsh_core::EllipsizeMode::Tail)
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

fn heading(title: impl IntoReactive<SharedString>, width: f32) -> Text {
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
pub(crate) fn breakdown_table<N: Into<SharedString>>(
    name_title: impl IntoReactive<SharedString>,
    net_title: impl Fn() -> N + 'static,
    rows: impl Fn() -> Vec<BreakdownRow> + 'static,
    empty: impl IntoReactive<SharedString>,
) -> impl IntoElement {
    table(name_title, net_title, rows, empty, None)
}

/// A [`breakdown_table`] whose lines can be pressed: `on_pick` gets the name of the line pressed
/// (a symbol, a setup, a tag), to open the trades under it.
pub(crate) fn breakdown_table_pickable<N: Into<SharedString>>(
    name_title: impl IntoReactive<SharedString>,
    net_title: impl Fn() -> N + 'static,
    rows: impl Fn() -> Vec<BreakdownRow> + 'static,
    empty: impl IntoReactive<SharedString>,
    on_pick: impl Fn(&str) + 'static,
) -> impl IntoElement {
    table(name_title, net_title, rows, empty, Some(Rc::new(on_pick)))
}

/// What a pressed line of a table does with its name.
type Pick = Rc<dyn Fn(&str)>;

fn table<N: Into<SharedString>>(
    name_title: impl IntoReactive<SharedString>,
    net_title: impl Fn() -> N + 'static,
    rows: impl Fn() -> Vec<BreakdownRow> + 'static,
    empty: impl IntoReactive<SharedString>,
    on_pick: Option<Pick>,
) -> impl IntoElement {
    let name_title = name_title.into_reactive();
    let empty = empty.into_reactive();
    dynamic(move || {
        let rows = rows();
        let titles = Titles::new(net_title().into());
        untracked(|| {
            let widths = Widths::of(&rows, &titles);
            let header = line(
                name_of(text(name_title.clone()).text_xs().text_color(move || theme().muted_foreground)),
                figures(widths)
                    .child(heading(titles.trades.clone(), widths.trades))
                    .child(heading("Win", widths.win_rate))
                    .child(heading(titles.net.clone(), widths.net))
                    .child(heading("PF", widths.profit_factor))
                    .child(heading(titles.avg_r.clone(), widths.avg_r)),
            )
            .pb_2()
            .border_b_1()
            .border_color(move || theme().border);
            let body = if rows.is_empty() {
                empty_state(empty.clone(), "").into_element()
            } else {
                view()
                    .children(rows.into_iter().map(|row| {
                        let net = row.net;
                        let name = row.name.clone();
                        let pick = on_pick.clone();
                        let shown = line(
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
                        .border_color(move || theme().border.opacity(0.5));
                        match pick {
                            Some(pick) => shown.cursor_pointer().on_press(move || pick(&name)).into_element(),
                            None => shown.into_element(),
                        }
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
        let titles = Titles { trades: "Lệnh".into(), net: "Tổng R".into(), avg_r: "R TB".into() };
        let widths = Widths::of(&rows, &titles);
        let glyphs = |n: f32| (n * FIGURE_GLYPH).ceil();
        assert!(widths.win_rate >= glyphs(6.0), "100.0% fits: {widths:?}");
        assert!(widths.net >= glyphs(6.0), "+10.8R fits: {widths:?}");
        assert!(widths.avg_r >= glyphs(6.0), "+1.54R fits: {widths:?}");
        assert!(widths.net >= (6.0_f32 * TITLE_GLYPH).ceil(), "the title Tổng R fits: {widths:?}");
        assert!(widths.profit_factor >= glyphs(4.0), "1.86 fits: {widths:?}");
        let empty = Widths::of(&[], &Titles { net: "Net".into(), ..titles });
        assert!(empty.trades >= (4.0_f32 * TITLE_GLYPH).ceil(), "a table with no rows still fits its titles");
    }
}
