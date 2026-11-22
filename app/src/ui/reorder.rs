//! Putting things in another order: the `⋮⋮` handle that is dragged (or moved with the arrow
//! keys when it has the focus) and the arithmetic that turns a drag into a new position.

use nsh_base::theme;
use nsh_core::prelude::*;

/// One step of a handle moved from the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// One place towards the start.
    Earlier,
    /// One place towards the end.
    Later,
}

/// The grip of an item that can be moved. Dragging it reports how far it went, in points
/// (`on_drag(dx, dy)`, when the finger or mouse is released); with the keyboard focus on it the
/// arrow keys report a [`Step`]. `label` is what screen readers say and what tests press.
pub fn drag_handle(
    label: impl IntoReactive<SharedString>,
    on_drag: impl Fn(f32, f32) + 'static,
    on_step: impl Fn(Step) + 'static,
) -> impl IntoElement {
    view()
        .role(AccessibilityRole::Button)
        .label(label)
        .focusable(true)
        .on_press(|| {})
        .on_pan_end(move |event: &PanEvent| {
            if !event.cancelled {
                on_drag(event.translation.x.0, event.translation.y.0);
            }
        })
        .on_key_down(move |event: &KeyEvent| match &*event.key {
            "ArrowLeft" | "ArrowUp" => on_step(Step::Earlier),
            "ArrowRight" | "ArrowDown" => on_step(Step::Later),
            _ => {}
        })
        .w(px(22.0))
        .h(px(26.0))
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(text("\u{22ee}\u{22ee}").text_xs().text_color(move || theme().muted_foreground))
}

/// The most cells a drag is taken to cross, so a wild translation cannot overflow.
const MAX_CELLS: f32 = 10_000.0;

/// How many whole cells of `cell` points a drag of `distance` crosses (rounded; negative
/// towards the start).
fn whole_cells(distance: f32, cell: f32) -> isize {
    if cell <= 0.0 || !distance.is_finite() {
        return 0;
    }
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "bounded by MAX_CELLS first")]
    let cells = (distance / cell).round().clamp(-MAX_CELLS, MAX_CELLS) as isize;
    cells
}

/// How many tiles of `tile` points with `gap` points between them fit a row `width` points
/// wide (at least one).
pub fn grid_columns(width: f32, tile: f32, gap: f32) -> usize {
    let pitch = tile + gap;
    if pitch <= 0.0 || !width.is_finite() {
        return 1;
    }
    let fitting = ((width + gap) / pitch).floor().clamp(1.0, MAX_CELLS);
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "held to 1..=MAX_CELLS first"
    )]
    let columns = fitting as usize;
    columns
}

/// Where the item at `from` lands when dragged by `(dx, dy)` points in a wrapping grid: cells
/// are `cell.0 × cell.1` points (tile and gap), `columns` to a row, `len` items in all.
pub fn grid_drop_index(from: usize, drag: (f32, f32), cell: (f32, f32), columns: usize, len: usize) -> usize {
    let columns = columns.max(1);
    let (row, column) = (from / columns, from % columns);
    let column = column.saturating_add_signed(whole_cells(drag.0, cell.0)).min(columns - 1);
    let row = row.saturating_add_signed(whole_cells(drag.1, cell.1));
    (row * columns + column).min(len.saturating_sub(1))
}

/// Where the item at `from` lands when dragged `dy` points down a list whose rows are
/// `heights` points tall: the row its centre ends up over.
pub fn list_drop_index(from: usize, dy: f32, heights: &[f32]) -> usize {
    let Some(&own) = heights.get(from) else { return from };
    if !dy.is_finite() {
        return from;
    }
    let top: f32 = heights[..from].iter().sum();
    let centre = top + own / 2.0 + dy;
    let mut edge = 0.0;
    for (index, height) in heights.iter().enumerate() {
        edge += height;
        if centre < edge {
            return index;
        }
    }
    heights.len() - 1
}

/// The position after one keyboard `step` from `from`, kept within `len` items.
pub fn stepped(from: usize, step: Step, len: usize) -> usize {
    match step {
        Step::Earlier => from.saturating_sub(1),
        Step::Later => (from + 1).min(len.saturating_sub(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: (f32, f32) = (208.0, 180.0);

    #[test]
    fn a_grid_drag_moves_by_whole_cells() {
        // 3 columns, 7 items; item 1 is in the first row, middle column.
        assert_eq!(grid_drop_index(1, (210.0, 0.0), CELL, 3, 7), 2, "one cell right");
        assert_eq!(grid_drop_index(1, (-210.0, 0.0), CELL, 3, 7), 0, "one cell left");
        assert_eq!(grid_drop_index(1, (0.0, 190.0), CELL, 3, 7), 4, "one row down");
        assert_eq!(grid_drop_index(1, (40.0, 20.0), CELL, 3, 7), 1, "a small drag stays");
    }

    #[test]
    fn a_grid_drag_stays_inside_the_grid() {
        assert_eq!(grid_drop_index(1, (9_000.0, 0.0), CELL, 3, 7), 2, "no wrapping to the next row");
        assert_eq!(grid_drop_index(1, (0.0, 9_000.0), CELL, 3, 7), 6, "no further than the last item");
        assert_eq!(grid_drop_index(4, (0.0, -9_000.0), CELL, 3, 7), 1, "no earlier than the first row");
        assert_eq!(grid_drop_index(0, (f32::NAN, f32::INFINITY), CELL, 3, 7), 0, "nonsense changes nothing");
        assert_eq!(grid_drop_index(0, (500.0, 0.0), CELL, 0, 5), 0, "zero columns count as one");
    }

    #[test]
    fn a_list_drag_lands_on_the_row_under_the_centre() {
        let heights = [40.0, 40.0, 80.0, 40.0];
        assert_eq!(list_drop_index(0, 10.0, &heights), 0, "a nudge stays");
        assert_eq!(list_drop_index(0, 45.0, &heights), 1);
        assert_eq!(list_drop_index(0, 100.0, &heights), 2, "tall rows take more distance");
        assert_eq!(list_drop_index(0, 5_000.0, &heights), 3);
        assert_eq!(list_drop_index(3, -5_000.0, &heights), 0);
        assert_eq!(list_drop_index(9, 10.0, &heights), 9, "an unknown row stays where it is");
    }

    #[test]
    fn a_row_fits_as_many_tiles_as_its_width_allows() {
        assert_eq!(grid_columns(196.0, 196.0, 12.0), 1);
        assert_eq!(grid_columns(611.0, 196.0, 12.0), 2, "one point short of three tiles and two gaps");
        assert_eq!(grid_columns(612.0, 196.0, 12.0), 3);
        assert_eq!(grid_columns(50.0, 196.0, 12.0), 1, "never fewer than one");
        assert_eq!(grid_columns(0.0, 196.0, 12.0), 1, "before the first layout");
        assert_eq!(grid_columns(f32::NAN, 196.0, 12.0), 1);
    }

    #[test]
    fn keyboard_steps_stop_at_the_ends() {
        assert_eq!(stepped(0, Step::Earlier, 3), 0);
        assert_eq!(stepped(1, Step::Earlier, 3), 0);
        assert_eq!(stepped(1, Step::Later, 3), 2);
        assert_eq!(stepped(2, Step::Later, 3), 2);
        assert_eq!(stepped(0, Step::Later, 0), 0);
    }
}
