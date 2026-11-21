//! The trades table: `nsh_base::data_table` with the app's columns, cells and look.

use std::rc::Rc;

use nsh_base::{
    DataTableCell, DataTableClasses, DataTableColumn, DataTablePress, DataTableRows, DataTableSort,
    DataTableSortDirection, button, data_table, data_table::DataTableAlign, theme,
};
use nsh_core::prelude::*;

use super::{
    chips::{Tone, chip, pnl_text},
    parts::empty_state,
    theme as tokens,
};

/// The texts of one table row, as shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TradeCells {
    pub(crate) time: String,
    pub(crate) symbol: String,
    pub(crate) side: String,
    pub(crate) lot: String,
    pub(crate) hold: String,
    pub(crate) net: String,
    pub(crate) r: String,
    pub(crate) setup: String,
    pub(crate) journal: String,
}

/// What the table is ordered by: a column's key and whether it runs upward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableSort {
    pub(crate) key: SharedString,
    pub(crate) ascending: bool,
}

/// What the journal column says of a trade the trader wrote about.
pub(crate) const JOURNALED: &str = "Đã ghi";
/// …and of one they did not.
pub(crate) const NOT_JOURNALED: &str = "Chưa ghi";

const ROW_HEIGHT: f32 = 44.0;
const HEADER_HEIGHT: f32 = 40.0;

/// Whether a column's title has a sort button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sortable {
    Yes,
    No,
}

/// One column: key, title, width, whether it sorts, and how it aligns.
fn column(key: &str, title: &str, width: f32, sortable: Sortable, align: DataTableAlign) -> DataTableColumn {
    let mut column = DataTableColumn::default();
    column.key = key.to_owned().into();
    column.title = Some(title.to_owned().into());
    column.width = Some(px(width));
    column.sortable = sortable == Sortable::Yes;
    column.align = align;
    column.movable = Some(false);
    column
}

fn columns(sorting: Sortable) -> Vec<DataTableColumn> {
    use DataTableAlign::{End, Start};
    use Sortable::No;
    vec![
        column("time", "Đóng lúc", 116.0, sorting, Start),
        column("symbol", "Symbol", 100.0, sorting, Start),
        column("side", "Chiều", 72.0, No, Start),
        column("lot", "Lot", 68.0, sorting, End),
        column("hold", "Thời gian giữ", 120.0, sorting, End),
        column("net", "Net", 112.0, sorting, End),
        column("r", "R", 72.0, sorting, End),
        column("setup", "Setup", 132.0, No, Start),
        column("journal", "Nhật ký", 96.0, No, Start),
    ]
}

fn rows_of(cells: &[TradeCells]) -> DataTableRows {
    let texts = |pick: fn(&TradeCells) -> &str| cells.iter().map(pick).collect::<Vec<_>>();
    DataTableRows::new(cells.len())
        .texts("time", texts(|c| &c.time))
        .texts("symbol", texts(|c| &c.symbol))
        .texts("side", texts(|c| &c.side))
        .texts("lot", texts(|c| &c.lot))
        .texts("hold", texts(|c| &c.hold))
        .texts("net", texts(|c| &c.net))
        .texts("r", texts(|c| &c.r))
        .texts("setup", texts(|c| &c.setup))
        .texts("journal", texts(|c| &c.journal))
}

/// Draws one cell from its column and text: the text is everything the cell shows, so a
/// cell is made again exactly when what it shows changes.
fn cell(cell: &DataTableCell, on_open: impl Fn(usize) + Copy + 'static) -> AnyElement {
    let shown = cell.text.clone();
    let key = cell.key.as_str();
    let row = usize::try_from(cell.row).unwrap_or(usize::MAX);
    match key {
        "time" => text(shown)
            .font_family(tokens::mono)
            .tabular_nums()
            .text_xs()
            .text_color(move || theme().muted_foreground)
            .into_element(),
        "symbol" => button()
            .on_press(move || on_open(row))
            .accessibility_label(format!("Mở nhật ký lệnh {shown}"))
            .px_1()
            .rounded(px(4.0))
            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
            .child(text(shown).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
            .into_element(),
        "side" => {
            let tone = if shown.as_ref() == "Buy" { Tone::Profit } else { Tone::Loss };
            chip(shown, tone).into_element()
        }
        "lot" | "hold" => text(shown).font_family(tokens::mono).tabular_nums().number_of_lines(1).into_element(),
        "net" | "r" => {
            let figure = shown.to_string();
            pnl_text(move || figure.clone()).into_element()
        }
        "setup" if !shown.is_empty() => chip(shown, Tone::Neutral).into_element(),
        "journal" => {
            let written = shown.as_ref() == JOURNALED;
            text(shown)
                .text_sm()
                .text_color(move || if written { theme().primary } else { theme().muted_foreground })
                .into_element()
        }
        _ => text(shown).into_element(),
    }
}

/// How a table is ordered: what it is ordered by now, and what a click on a title asks for.
pub(crate) struct Sorting {
    current: Rc<dyn Fn() -> Option<TableSort>>,
    change: Rc<dyn Fn(Option<TableSort>)>,
}

impl Sorting {
    pub(crate) fn new(
        current: impl Fn() -> Option<TableSort> + 'static,
        change: impl Fn(Option<TableSort>) + 'static,
    ) -> Self {
        Self { current: Rc::new(current), change: Rc::new(change) }
    }
}

/// The trades table. `rows` are read again when they change, in the order `sorting` says;
/// `on_open` hears the position in `rows` of a row the trader pressed; `empty` gives the
/// title and hint shown when there are no rows.
pub(crate) fn trades_table(
    rows: impl Fn() -> Vec<TradeCells> + Copy + 'static,
    sorting: Sorting,
    on_open: impl Fn(usize) + Copy + 'static,
    empty: impl Fn() -> (String, String) + Copy + 'static,
) -> impl IntoElement {
    build(rows, Some(sorting), on_open, empty)
}

/// The same table for a short, fixed list (the latest trades): titles do not sort.
pub(crate) fn recent_trades(
    rows: impl Fn() -> Vec<TradeCells> + Copy + 'static,
    on_open: impl Fn(usize) + Copy + 'static,
    empty: impl Fn() -> (String, String) + Copy + 'static,
) -> impl IntoElement {
    build(rows, None, on_open, empty)
}

fn build(
    rows: impl Fn() -> Vec<TradeCells> + Copy + 'static,
    sorting: Option<Sorting>,
    on_open: impl Fn(usize) + Copy + 'static,
    empty: impl Fn() -> (String, String) + Copy + 'static,
) -> impl IntoElement {
    let mut classes = DataTableClasses::default();
    classes.root = Some("rounded-lg border border-border bg-card".into());
    classes.header = Some("border-b border-border bg-secondary/50".into());
    classes.header_cell = Some("px-3 text-xs font-medium text-muted-foreground".into());
    classes.header_content = Some("gap-1".into());
    classes.sort_ascending =
        Some("text-muted-foreground opacity-60 selected:text-foreground selected:opacity-100".into());
    classes.sort_descending =
        Some("text-muted-foreground opacity-60 selected:text-foreground selected:opacity-100".into());
    classes.row = Some("border-b border-border".into());
    classes.row_hover = Some("bg-accent/50".into());
    classes.row_selected = Some("bg-accent".into());
    classes.cell = Some("px-3 text-sm".into());
    classes.fixed_edge = Some("bg-border".into());

    let (current, change, sortable) = match sorting {
        Some(Sorting { current, change }) => (Some(current), Some(change), Sortable::Yes),
        None => (None, None, Sortable::No),
    };
    data_table()
        .flex_1()
        .accessibility_label("Danh sách lệnh")
        .columns(columns(sortable))
        .data(move || rows_of(&rows()))
        .row_height(px(ROW_HEIGHT))
        .header_height(px(HEADER_HEIGHT))
        .column_selectable(false)
        .movable_columns(false)
        .classes(classes)
        .sort(move || {
            current.as_ref().and_then(|current| current()).map(|s| {
                let mut sort = DataTableSort::default();
                sort.key = s.key;
                sort.direction =
                    if s.ascending { DataTableSortDirection::Ascending } else { DataTableSortDirection::Descending };
                sort
            })
        })
        .on_sort_change(move |next: &Option<DataTableSort>| {
            if let Some(change) = &change {
                change(next.as_ref().map(|s| TableSort {
                    key: s.key.clone(),
                    ascending: s.direction == DataTableSortDirection::Ascending,
                }));
            }
        })
        .on_row_press(move |press: &DataTablePress| {
            if let Ok(row) = usize::try_from(press.row) {
                on_open(row);
            }
        })
        .render_cell(move |c: &DataTableCell| cell(c, on_open))
        .empty(empty_state(move || empty().0, move || empty().1))
}
