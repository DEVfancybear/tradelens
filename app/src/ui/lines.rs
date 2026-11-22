//! A list of one-line texts the trader writes and reorders by hand: the entry rules of a setup,
//! its checklist, the questions of a review template. Each line is a field with a button that
//! removes it, and the list ends in a button that adds one.
//!
//! The list edits a `Vec<String>` it does not own: every change is reported whole through
//! `on_change`, and keeping it is the caller's job.

use nsh_core::prelude::*;

use super::{
    controls::{Variant, action_icon, icon_action, text_field},
    icons::Icon,
};

/// The lines of `lines`, named `label` (`"Điều kiện vào"`) for screen readers: the fields read
/// "label 1", "label 2", the buttons "Xoá label 1" and `add_label`. Rows are made again only
/// when the number of lines changes, never when a line is typed in, so the caret stays put.
pub fn line_list(
    label: &'static str,
    add_label: &'static str,
    lines: impl Fn() -> Vec<String> + Copy + 'static,
    on_change: impl Fn(Vec<String>) + Copy + 'static,
) -> impl IntoElement {
    let count = computed(move || lines().len());
    view()
        .gap_2()
        .child(dynamic(move || {
            let count = count.get();
            untracked(|| view().gap_2().children((0..count).map(|index| line_row(label, index, lines, on_change))))
        }))
        .child(view().flex_row().child(action_icon(Icon::Plus, add_label, Variant::Secondary, move || {
            let mut next = lines();
            next.push(String::new());
            on_change(next);
        })))
}

/// One line: its field and the button that removes it.
fn line_row(
    label: &'static str,
    index: usize,
    lines: impl Fn() -> Vec<String> + Copy + 'static,
    on_change: impl Fn(Vec<String>) + Copy + 'static,
) -> impl IntoElement {
    let name = format!("{label} {}", index + 1);
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(view().flex_1().min_w(px(0.0)).child(text_field(
            name.clone(),
            None,
            "",
            move || lines().get(index).cloned().unwrap_or_default(),
            move |typed| on_change(replaced(&lines(), index, typed)),
        )))
        .child(icon_action(Icon::Close, format!("Xoá {name}"), Variant::Ghost, move || {
            on_change(without(&lines(), index));
        }))
}

/// `lines` with the line at `index` set to `typed`; unchanged when there is no such line.
fn replaced(lines: &[String], index: usize, typed: &str) -> Vec<String> {
    let mut next = lines.to_vec();
    if let Some(line) = next.get_mut(index) {
        typed.clone_into(line);
    }
    next
}

/// `lines` without the line at `index`.
fn without(lines: &[String], index: usize) -> Vec<String> {
    lines.iter().enumerate().filter(|(at, _)| *at != index).map(|(_, line)| line.clone()).collect()
}

/// The lines that hold something, trimmed: what is kept when a form is saved.
pub fn filled(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| line.trim().to_owned()).filter(|line| !line.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn a_line_is_replaced_or_removed_by_its_place() {
        let list = lines(&["a", "b", "c"]);
        assert_eq!(replaced(&list, 1, "bb"), lines(&["a", "bb", "c"]));
        assert_eq!(replaced(&list, 9, "x"), list, "no such line");
        assert_eq!(without(&list, 0), lines(&["b", "c"]));
        assert_eq!(without(&list, 7), list);
    }

    #[test]
    fn blank_lines_are_dropped_and_the_rest_trimmed() {
        assert_eq!(filled(&lines(&["  a ", "", "   ", "b"])), lines(&["a", "b"]));
    }
}
