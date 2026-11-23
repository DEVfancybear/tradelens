//! Rows that are picked from a list and a small table of figures: the playbook's list and its
//! breakdown by grade, the missed trades and their breakdown by reason.

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use super::motion;

/// A pickable card in a list: `content` inside a frame that is outlined while `selected`.
/// `label` is what screen readers say and what tests press.
pub(crate) fn list_item(
    label: impl Into<SharedString>,
    selected: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
    content: impl IntoElement,
) -> impl IntoElement {
    button()
        .cursor_pointer()
        .on_press(on_press)
        .accessibility_label(label.into())
        .p_3()
        .gap_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.scale(motion::PRESS_SCALE))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .style_when(selected, |s| s.bg(move || theme().accent).border_color(move || theme().primary))
        .child(content)
}

/// Which side of its column a table cell's text sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Start,
    End,
}

/// A column of a [`mini_table`]: its heading and where its cells sit. The first column takes the
/// room that is left; the others are of a fixed width.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Column {
    pub(crate) title: &'static str,
    pub(crate) side: Side,
}

impl Column {
    pub(crate) const fn start(title: &'static str) -> Self {
        Self { title, side: Side::Start }
    }

    pub(crate) const fn end(title: &'static str) -> Self {
        Self { title, side: Side::End }
    }
}

/// Width of every column but the first, in points.
const CELL_WIDTH: f32 = 96.0;

/// A small table of `rows` of cells under the `columns`' headings: for a handful of rows that
/// need no scrolling or sorting. A row with fewer cells than there are columns leaves the rest empty.
pub(crate) fn mini_table(columns: Vec<Column>, rows: Vec<Vec<AnyElement>>) -> impl IntoElement {
    let head = view().flex_row().items_center().gap_3().px_2().pb_2().children(columns.iter().enumerate().map(
        |(at, column)| cell(at, column.side, text(column.title).text_xs().text_color(move || theme().muted_foreground)),
    ));
    view().child(head).children(rows.into_iter().map(move |row| {
        view()
            .flex_row()
            .items_center()
            .gap_3()
            .px_2()
            .min_h(px(40.0))
            .border_t_1()
            .border_color(move || theme().border)
            .children(
                row.into_iter()
                    .enumerate()
                    .map(|(at, content)| cell(at, columns.get(at).map_or(Side::Start, |c| c.side), content)),
            )
    }))
}

/// One cell: the first column grows, the others have a width; `side` places the content.
fn cell(at: usize, side: Side, content: impl IntoElement) -> View {
    let cell = view().flex_row().items_center().child(content);
    let cell = if at == 0 { cell.flex_1().min_w(px(0.0)) } else { cell.w(px(CELL_WIDTH)) };
    match side {
        Side::Start => cell.justify_start(),
        Side::End => cell.justify_end(),
    }
}
