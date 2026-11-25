//! The trades table: `nsh_base::data_table` with the app's columns, cells and look.

use std::rc::Rc;

use nsh_base::{
    DataTableCell, DataTableClasses, DataTableColumn, DataTablePress, DataTableRows, DataTableSort,
    DataTableSortDirection, button, data_table, data_table::DataTableAlign, theme,
};
use nsh_core::{Checked, prelude::*};

use super::{
    chips::{Tone, chip, pnl_text},
    icons::{Icon, icon},
    parts::empty_state,
    theme as tokens,
};

/// The texts of one table row, as shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TradeCells {
    /// The selection box: `1` or `0`, a bar, and what screen readers call it (`0|Chọn lệnh EURUSD 20/11 22:44`).
    pub(crate) select: String,
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

/// The width of a sort button at the end of a title (`nsh_base`'s), which a right-aligned title gives
/// up and a right-aligned cell must give up too, so that a figure stands under its title.
const SORT_BUTTON: f32 = 20.0;

/// The width of the column of selection boxes.
const SELECT_WIDTH: f32 = 44.0;

/// The least the setup column gets; it grows into what the others leave.
const SETUP_MIN: f32 = 96.0;

/// Which columns a table has: the trades page has them all, the overview's list the design's seven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Full,
    Recent,
}

/// The side padding of a cell (`px-3`), both sides.
const CELL_PADDING: f32 = 24.0;
/// What a glyph of a figure takes in the table's mono font (`JetBrains Mono` at 14 pt is 8.4 wide), with room.
const FIGURE_GLYPH: f32 = 8.8;
/// What a glyph of a title takes (12 pt, medium), with room.
const TITLE_GLYPH: f32 = 7.0;

/// The width a right-aligned figure column needs: its padding, the room of the sort button its title
/// has (a figure stands under its title's text, so it gives up the same), and `figure` glyphs of the
/// longest figure a trade has, or its `title`, whichever is wider. A column is never narrower than what
/// it shows: `Lot` holds `12.50`, `R` holds `+12.5R`, the hold time `123g 30p`.
fn figure_column(title: &str, figure: usize, sorting: Sortable) -> f32 {
    let sort = if sorting == Sortable::Yes { SORT_BUTTON } else { 0.0 };
    let glyphs = |count: usize, glyph: f32| f32::from(u16::try_from(count).unwrap_or(u16::MAX)) * glyph;
    let widest = glyphs(figure, FIGURE_GLYPH).max(glyphs(title.chars().count(), TITLE_GLYPH) + 4.0);
    (CELL_PADDING + sort + widest).ceil()
}

/// The columns of a table. The figure columns start from the width their figures and titles need and
/// are fitted to what they show once the rows come (`auto_fit`), never narrower than that start; the
/// setup column grows into what is left, so that the rows reach the right edge of the card (its chip
/// is cut with an ellipsis when it is narrow).
fn columns(sorting: Sortable, shape: Shape) -> Vec<DataTableColumn> {
    use DataTableAlign::{End, Start};
    use Sortable::No;
    let lot = figure_column("Lot", 5, sorting);
    let net = figure_column("Net", 11, sorting);
    let r = figure_column("R", 6, sorting);
    let mut list = match shape {
        Shape::Full => vec![
            column("select", "", SELECT_WIDTH, No, Start),
            column("time", "Đóng lúc", 108.0, sorting, Start),
            column("symbol", "Symbol", 104.0, sorting, Start),
            column("side", "Chiều", 68.0, No, Start),
            column("lot", "Lot", lot, sorting, End),
            column("hold", "Thời gian giữ", figure_column("Thời gian giữ", 8, sorting), sorting, End),
            column("net", "Net", net, sorting, End),
            column("r", "R", r, sorting, End),
            column("setup", "Setup", SETUP_MIN, No, Start),
            column("journal", "Nhật ký", 88.0, No, Start),
        ],
        Shape::Recent => vec![
            column("time", "Đóng lúc", 108.0, sorting, Start),
            column("symbol", "Symbol", 104.0, sorting, Start),
            column("side", "Chiều", 68.0, No, Start),
            column("lot", "Lot", lot, sorting, End),
            column("net", "Net", net, sorting, End),
            column("r", "R", r, sorting, End),
            column("setup", "Setup", SETUP_MIN, No, Start),
        ],
    };
    for column in &mut list {
        match column.key.as_ref() {
            "setup" => {
                column.min_width = column.width;
                column.grow = Some(1.0);
            }
            "symbol" | "lot" | "hold" | "net" | "r" => fit_to_content(column, sorting),
            _ => {}
        }
    }
    list
}

/// Lets the column fit its widest text and title once the rows come, from the width it starts with up.
/// A cell is padded and a sortable title ends in its sort button, which the fit does not measure.
fn fit_to_content(column: &mut DataTableColumn, sorting: Sortable) {
    let sort = if sorting == Sortable::Yes { SORT_BUTTON } else { 0.0 };
    column.min_width = column.width;
    column.fit_padding = Some(px(CELL_PADDING + sort));
    column.auto_fit = true;
}

fn rows_of(cells: &[TradeCells]) -> DataTableRows {
    let texts = |pick: fn(&TradeCells) -> &str| cells.iter().map(pick).collect::<Vec<_>>();
    DataTableRows::new(cells.len())
        .texts("select", texts(|c| &c.select))
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

/// The look of a ticked or empty box, with the name and state screen readers read. It is not a control
/// of its own: a press on it is a press on the row, at the column of the boxes, which the table hears.
fn tick_box(label: String, state: Checked) -> impl IntoElement {
    let on = state == Checked::True;
    view()
        .role(AccessibilityRole::Checkbox)
        .label(label)
        .accessibility_state(AccessibilityState { checked: Some(state), ..AccessibilityState::default() })
        .cursor_pointer()
        .size(px(18.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .border_1()
        .border_color(move || if on { theme().primary } else { theme().input })
        .bg(move || if on { theme().primary } else { theme().card })
        .child(icon(Icon::Check, 14.0).text_color(move || theme().primary_foreground).opacity(if on {
            1.0_f32
        } else {
            0.0_f32
        }))
}

/// Draws one cell from its column and text: the text is everything the cell shows, so a
/// cell is made again exactly when what it shows changes.
fn cell(cell: &DataTableCell, sorting: Sortable, on_open: impl Fn(usize) + Copy + 'static) -> AnyElement {
    // A right-aligned figure ends where its title's text ends: before the title's sort button.
    let end_gap = if sorting == Sortable::Yes { SORT_BUTTON } else { 0.0 };
    let shown = cell.text.clone();
    let key = cell.key.as_str();
    let row = usize::try_from(cell.row).unwrap_or(usize::MAX);
    match key {
        // Only the look of a box: a press on it reaches the table's row press, which toggles the row.
        "select" => {
            let (flag, label) = shown.split_once('|').unwrap_or(("0", ""));
            let on = flag == "1";
            tick_box(label.to_owned(), if on { Checked::True } else { Checked::False }).into_element()
        }
        "time" => text(shown)
            .font_family(tokens::mono)
            .tabular_nums()
            .text_xs()
            .text_color(move || theme().muted_foreground)
            .into_element(),
        "symbol" => button()
            .cursor_pointer()
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
        "lot" | "hold" => text(shown)
            .font_family(tokens::mono)
            .tabular_nums()
            .number_of_lines(1)
            .text_right()
            .pr(px(end_gap))
            .into_element(),
        "net" | "r" => {
            let figure = shown.to_string();
            pnl_text(move || figure.clone()).text_right().pr(px(end_gap)).into_element()
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
/// `on_open` hears the position in `rows` of a row the trader pressed; `on_select` hears the
/// position of a row whose box was pressed, and whether Shift was held; `empty` gives the
/// title and hint shown when there are no rows.
pub(crate) fn trades_table(
    rows: impl Fn() -> Vec<TradeCells> + Copy + 'static,
    sorting: Sorting,
    on_open: impl Fn(usize) + Copy + 'static,
    on_select: impl Fn(usize, bool) + Copy + 'static,
    empty: impl Fn() -> (String, String) + Copy + 'static,
) -> impl IntoElement {
    build(rows, Some(sorting), Shape::Full, (on_open, on_select), empty)
}

/// The same table for a short, fixed list (the latest trades): titles do not sort.
pub(crate) fn recent_trades(
    rows: impl Fn() -> Vec<TradeCells> + Copy + 'static,
    on_open: impl Fn(usize) + Copy + 'static,
    empty: impl Fn() -> (String, String) + Copy + 'static,
) -> impl IntoElement {
    build(rows, None, Shape::Recent, (on_open, |_, _| {}), empty)
}

fn build(
    rows: impl Fn() -> Vec<TradeCells> + Copy + 'static,
    sorting: Option<Sorting>,
    shape: Shape,
    (on_open, on_select): (impl Fn(usize) + Copy + 'static, impl Fn(usize, bool) + Copy + 'static),
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
    view().flex_1().min_w(px(0.0)).child(
        data_table()
            .flex_1()
            .accessibility_label("Danh sách lệnh")
            .columns(columns(sortable, shape))
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
                    sort.direction = if s.ascending {
                        DataTableSortDirection::Ascending
                    } else {
                        DataTableSortDirection::Descending
                    };
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
            // The rows the trader ticked are lit, as the design lights them.
            .row_classes(move || {
                rows()
                    .iter()
                    .map(|cells| SharedString::from(if cells.select.starts_with('1') { "bg-accent" } else { "" }))
                    .collect::<Vec<_>>()
            })
            .on_row_press(move |press: &DataTablePress| {
                if let Ok(row) = usize::try_from(press.row) {
                    // The first column of the full table holds the boxes: a press there selects, a press
                    // anywhere else on the row opens it.
                    if shape == Shape::Full && press.column == Some(0) {
                        on_select(row, press.shift);
                    } else {
                        on_open(row);
                    }
                }
            })
            // Enter on the row the keyboard selected opens it, as a press on it does.
            .on_row_activate(move |row: &u32| {
                if let Ok(row) = usize::try_from(*row) {
                    on_open(row);
                }
            })
            .render_cell(move |c: &DataTableCell| cell(c, sortable, on_open))
            .empty(empty_state(move || empty().0, move || empty().1)),
    )
}
