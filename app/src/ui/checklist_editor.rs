//! The editor of a checklist: rename items, remove them and add new ones. Items keep their ids
//! (journal entries refer to the ids they confirmed); a new item has id 0 until it is saved.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::ChecklistItem;

use super::{
    controls::{Variant, action_when, commit_field, icon_action, text_field},
    icons::Icon,
};

/// The items with the text of the one at `index` replaced; `None` when nothing changes (no
/// such item, the same text, or an empty one).
pub fn renamed(items: &[ChecklistItem], index: usize, text: &str) -> Option<Vec<ChecklistItem>> {
    let text = text.trim();
    let current = items.get(index)?;
    if text.is_empty() || current.text == text {
        return None;
    }
    let mut next = items.to_vec();
    text.clone_into(&mut next.get_mut(index)?.text);
    Some(next)
}

/// The items without the one at `index`; `None` when there is no such item.
pub fn without(items: &[ChecklistItem], index: usize) -> Option<Vec<ChecklistItem>> {
    (index < items.len()).then(|| {
        let mut next = items.to_vec();
        next.remove(index);
        next
    })
}

/// The items with a new one (id 0) at the end; `None` when `text` is empty.
pub fn appended(items: &[ChecklistItem], text: &str) -> Option<Vec<ChecklistItem>> {
    let text = text.trim();
    (!text.is_empty()).then(|| {
        let mut next = items.to_vec();
        next.push(ChecklistItem { id: 0, text: text.to_owned() });
        next
    })
}

/// A checklist to edit. `items` reads the saved items; `on_change` hears the whole list after
/// each edit and saves it. `group` names the list for screen readers: its rows read
/// "group: mục N" and its buttons "Xoá mục …".
pub fn checklist_editor(
    group: &'static str,
    items: impl Fn() -> Vec<ChecklistItem> + 'static,
    on_change: impl Fn(Vec<ChecklistItem>) + Copy + 'static,
) -> impl IntoElement {
    // What is on screen: edits show at once, and what is saved replaces it when it differs.
    let list = signal(Vec::<ChecklistItem>::new());
    // A memo, so that only a change of the saved items replaces what is on screen: a read that
    // is under way (and still holds the old items) must not undo an edit that was just made.
    let saved = computed(items);
    effect(move || list.set(saved.get()));
    let apply = move |next: Option<Vec<ChecklistItem>>| {
        if let Some(next) = next {
            list.set(next.clone());
            on_change(next);
        }
    };
    let draft = signal(String::new());
    // The rows are known by their item and their place (a new item has no id yet). Renaming an item
    // changes neither, so it makes no row again: the row the trader moves on to is still there when
    // the rename of the one they leave has come back.
    let rows = computed(move || list.with(|items| items.iter().cloned().enumerate().collect::<Vec<_>>()));
    let none = computed(move || list.with(Vec::is_empty));
    view()
        .gap_2()
        .child(dynamic(move || {
            none.get()
                .then(|| untracked(|| text("Chưa có mục nào.").text_sm().text_color(move || theme().muted_foreground)))
        }))
        .child(each(
            rows,
            |(at, item): &(usize, ChecklistItem)| (item.id, *at),
            move |row, _| {
                let (index, item) = row.with_untracked(Clone::clone);
                let removal = format!("Xoá mục {}", item.text);
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(view().flex_1().min_w(px(0.0)).child(commit_field(
                        format!("{group}: mục {}", index + 1),
                        "Nội dung mục",
                        move || row.with(|(_, item)| item.text.clone()),
                        move |typed| apply(renamed(&list.get_untracked(), index, &typed)),
                    )))
                    .child(icon_action(Icon::Trash, removal, Variant::Ghost, move || {
                        apply(without(&list.get_untracked(), index));
                    }))
            },
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(view().flex_1().min_w(px(0.0)).child(text_field(
                    format!("{group}: mục mới"),
                    None,
                    "Thêm mục…",
                    move || draft.get(),
                    move |typed| draft.set(typed.to_owned()),
                )))
                .child(action_when(
                    format!("Thêm vào {group}"),
                    Variant::Secondary,
                    move || !draft.get().trim().is_empty(),
                    move || {
                        let typed = draft.get_untracked();
                        if let Some(next) = appended(&list.get_untracked(), &typed) {
                            draft.set(String::new());
                            apply(Some(next));
                        }
                    },
                )),
        )
}

#[cfg(test)]
mod tests {
    use nsh_core::Size;
    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::Headless;

    use super::*;

    fn item(id: i64, text: &str) -> ChecklistItem {
        ChecklistItem { id, text: text.to_owned() }
    }

    #[test]
    fn renaming_keeps_the_id_and_the_order() {
        let items = [item(4, "A"), item(9, "B")];
        let next = renamed(&items, 1, "  B2 ").expect("a change");
        assert_eq!(next, [item(4, "A"), item(9, "B2")]);
        assert_eq!(renamed(&items, 1, "B"), None, "the same text changes nothing");
        assert_eq!(renamed(&items, 1, "   "), None, "an empty text changes nothing");
        assert_eq!(renamed(&items, 5, "x"), None, "no such item");
    }

    #[test]
    fn removing_and_adding_keep_the_other_ids() {
        let items = [item(4, "A"), item(9, "B")];
        assert_eq!(without(&items, 0).expect("removed"), [item(9, "B")]);
        assert_eq!(without(&items, 2), None);
        assert_eq!(appended(&items, " C ").expect("added"), [item(4, "A"), item(9, "B"), item(0, "C")]);
        assert_eq!(appended(&items, " "), None);
    }

    #[component]
    fn Harness() -> impl IntoElement {
        super::super::theme::install();
        let saved = signal(vec![item(4, "Cùng chiều trend H4"), item(9, "Có vùng cản rõ")]);
        view().gap_2().child(checklist_editor("Checklist", move || saved.get(), move |next| saved.set(next))).child(
            text(move || {
                let all = saved.get();
                format!("saved {}", all.iter().map(|i| format!("{}={}", i.id, i.text)).collect::<Vec<_>>().join("|"))
            }),
        )
    }

    fn start() -> Headless {
        let config =
            EngineConfig { viewport: Size::new(px(900.0), px(600.0)), ..EngineConfig::new(Root::of::<Harness>()) };
        let mut app = Headless::start(config).expect("the engine starts");
        app.wait_for("saved 4=Cùng chiều trend H4|9=Có vùng cản rõ").expect("the harness shows");
        app
    }

    #[test]
    fn editing_the_list_reports_it_with_the_ids_kept() {
        let mut app = start();
        app.type_text("label:Checklist: mục mới", "Risk 1.5%").expect("types");
        app.press("Thêm vào Checklist").expect("adds");
        app.wait_for("saved 4=Cùng chiều trend H4|9=Có vùng cản rõ|0=Risk 1.5%").expect("the new item has id 0");
        app.press("label:Xoá mục Cùng chiều trend H4").expect("removes the first");
        app.wait_for("saved 9=Có vùng cản rõ|0=Risk 1.5%").expect("the others keep their ids");
        app.type_text("label:Checklist: mục 1", " lắm").expect("renames");
        app.submit("label:Checklist: mục 1").expect("Enter");
        app.wait_for("saved 9=Có vùng cản rõ lắm|0=Risk 1.5%").expect("the id stays with the renamed item");
    }
}
