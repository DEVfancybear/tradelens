//! The command palette: a search field over results the app answers itself (the full-text
//! search), grouped under headings, with the words that matched marked in each snippet, driven
//! by the keyboard (arrows, Enter, Escape). It draws in a frame the caller places.
//!
//! The palette owns no data: `rows` are the results, `query` is the text typed (controlled),
//! and a chosen row is reported by its id.

use nsh_base::{CommandEntry, CommandItem, button, command, theme};
use nsh_core::prelude::*;
use tradelens_core::search::marked;

use super::icons::{Icon, icon};

/// Width of the palette's frame, in points.
pub const PALETTE_WIDTH: f32 = 640.0;

/// Height of one result: a title line over a snippet line.
const ROW_HEIGHT: &str = "h-14";

/// One result of the palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteRow {
    /// What `on_select` hears when the row is chosen; unique among the rows.
    pub id: String,
    /// The heading of the section the row sits under; adjacent rows of one group share a section.
    pub group: String,
    pub title: String,
    /// The snippet, its matching words between `[` and `]`.
    pub snippet: String,
    /// What the row says at its right (a result in R, the kind of note).
    pub meta: String,
}

impl PaletteRow {
    fn item(&self) -> CommandItem {
        // The row's drawing gets the item's fields only: the snippet and the meta ride as keywords.
        CommandItem::new(self.id.clone(), self.title.clone())
            .group(self.group.clone())
            .keywords([self.snippet.clone(), self.meta.clone()])
    }
}

/// The palette: `rows` follow what the caller read for `query`. `chips` sit above the field (a
/// filter by kind), `hint` is what shows while there are no rows, and `busy` marks a search under
/// way. `on_select` hears a chosen row's id; `on_cancel` is Escape on an empty field.
pub fn command_palette(
    rows: impl Fn() -> Vec<PaletteRow> + 'static,
    query: Signal<String>,
    chips: impl IntoElement,
    hint: impl Fn() -> String + 'static,
    busy: impl Fn() -> bool + 'static,
    on_select: impl Fn(&str) + 'static,
    on_cancel: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .w(px(PALETTE_WIDTH))
        .rounded(px(12.0))
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().popover)
        .overflow_hidden()
        .shadow_lg()
        .child(
            command()
                .items(move || rows().iter().map(PaletteRow::item).collect::<Vec<_>>())
                .query(move || Some(SharedString::from(query.get())))
                .filterable(false)
                .loading(busy)
                .placeholder("Tìm lệnh, nhật ký, tổng kết, playbook…")
                .accessibility_label("Kết quả tìm kiếm")
                .search_accessibility_label("Tìm trong nhật ký")
                .auto_focus(true)
                .on_query_change(move |typed: &SharedString| query.set(typed.to_string()))
                .on_select(move |id: &SharedString| on_select(id))
                .on_cancel(on_cancel)
                .separator(false)
                .header(view().px_4().pt_4().pb_1().child(chips))
                .search_prefix(icon(Icon::Search, 18.0).text_color(move || theme().muted_foreground))
                .search_bar_class("flex-row items-center gap-3 mx-4 mb-2 px-3 rounded-lg border border-primary")
                .search_class("h-11 flex-1 text-sm text-foreground placeholder:text-muted-foreground")
                .list_class("max-h-96 px-2")
                .group_class("px-3 pt-3 pb-1 text-xs font-medium text-muted-foreground")
                .item_class(format!("group {ROW_HEIGHT} rounded-md px-3 py-2 focus:bg-accent"))
                .render_item(|entry: &CommandEntry| render_row(entry))
                .empty(text(hint).text_sm().text_color(move || theme().muted_foreground).px_4().py_6())
                .footer(
                    view()
                        .flex_row()
                        .justify_between()
                        .gap_3()
                        .px_4()
                        .py_3()
                        .child(
                            text("↑↓ chọn   Enter mở   Esc đóng")
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            text("Tìm cả ghi chú, tag, lỗi, câu trả lời tổng kết")
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
}

/// `content` (the palette) over the whole window: a scrim that closes it when pressed, and the
/// content near the top, centred. Place it as the last child of the window's root.
pub fn palette_layer(content: impl IntoElement, on_close: impl Fn() + 'static) -> impl IntoElement {
    view()
        .absolute()
        .inset_0()
        .items_center()
        .child(
            button()
                .on_press(on_close)
                .accessibility_label("Đóng tìm kiếm")
                .absolute()
                .inset_0()
                .bg(move || theme().background)
                .opacity(0.72_f32),
        )
        .child(view().pt(px(96.0)).child(content))
}

/// The search box of the sidebar: it looks like a field, shows the key that opens the palette, and
/// opens it when pressed.
pub fn search_trigger(on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .on_press(on_press)
        .accessibility_label("Mở tìm kiếm")
        .h(px(34.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .hover(|s| s.bg(move || theme().accent))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(Icon::Search, 16.0).text_color(move || theme().muted_foreground))
        .child(text("Tìm trong nhật ký").flex_1().text_sm().text_color(move || theme().muted_foreground))
        .child(
            view().px_1().rounded(px(4.0)).border_1().border_color(move || theme().border).child(
                text(key_label("mod+k"))
                    .text_xs()
                    .font_family(super::theme::mono)
                    .text_color(move || theme().muted_foreground),
            ),
        )
}

/// A result's content: the title and the meta on one line, the snippet under it.
fn render_row(entry: &CommandEntry) -> View {
    let snippet = entry.keywords.first().map_or_default(ToString::to_string);
    let meta = entry.keywords.get(1).map_or_default(ToString::to_string);
    view()
        .flex_1()
        .gap_1()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(text(entry.label.clone()).flex_1().number_of_lines(1).text_sm().font_weight(FontWeight::MEDIUM))
                .child(text(meta).text_xs().text_color(move || theme().muted_foreground)),
        )
        .child(view().flex_row().items_center().overflow_hidden().children(marked(&snippet).into_iter().map(
            |(part, matched)| {
                let piece = text(part).text_xs().number_of_lines(1);
                if matched {
                    piece
                        .px_1()
                        .rounded(px(3.0))
                        .bg(move || theme().accent)
                        .text_color(move || theme().primary)
                        .font_weight(FontWeight::MEDIUM)
                } else {
                    piece.text_color(move || theme().muted_foreground)
                }
            },
        )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_carries_its_snippet_and_meta_as_keywords() {
        let row = PaletteRow {
            id: "trade:1".into(),
            group: "Lệnh".into(),
            title: "XAUUSD Buy 20/11".into(),
            snippet: "…[Vào sớm] trước nến".into(),
            meta: "−1.0R".into(),
        };
        let item = row.item();
        assert_eq!(item.id, "trade:1");
        assert_eq!(item.group.as_deref(), Some("Lệnh"));
        assert_eq!(item.keywords.len(), 2);
        assert_eq!(item.keywords[0], "…[Vào sớm] trước nến");
    }
}
