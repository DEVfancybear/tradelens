//! A pick from one of the trader's catalogs (tags, mistakes, timeframes, reasons for a missed trade)
//! that keeps the catalog too: a new option is typed and Enter adds it and picks it; the ✕ after an
//! option removes it from the list after asking. Removing archives the option, so what already
//! stored it (a trade's journal, a playbook, a plan, the record open here) keeps it and still reads
//! its label; an archived option the open record holds stays as a chip without ✕ until unpicked.

use nsh_core::prelude::*;
use tradelens_core::catalog::{CatalogItem, CatalogKind};

use crate::{
    store::writer,
    ui::{
        controls::{TagOption, add_field, confirm_then, removable_tags, root_owner},
        parts::error_line,
    },
};

/// The chips of `kind`'s options (`items`: the whole catalog, archived options included), on while
/// `selected` holds their key; `on_toggle` hears a pick, and also the option just added (on). `group`
/// names the list ("Tag", "Lỗi", "Khung thời gian") and `placeholder` fills the add field.
pub(super) fn catalog_tags(
    kind: CatalogKind,
    group: &'static str,
    placeholder: &'static str,
    items: impl Fn() -> Vec<CatalogItem> + 'static,
    selected: impl Fn() -> Vec<String> + Copy + 'static,
    on_toggle: impl Fn(&str, bool) + Copy + 'static,
) -> impl IntoElement {
    let write = writer();
    let error = signal(None::<String>);
    let named = group.to_lowercase();
    let add = move |typed: String| {
        let label = label_of(kind, &typed);
        if label.is_empty() {
            return;
        }
        error.set(None);
        write.run(
            move |db| db.add_catalog_label(kind, &label),
            move |result| match result {
                Ok(key) if !selected().contains(&key) => on_toggle(&key, true),
                Ok(_) => {}
                Err(message) => error.set(Some(message)),
            },
        );
    };
    // The question is the root's task: the form holding this pick is built again when data is read.
    let asker = root_owner();
    let list = stored(named.clone());
    let remove = move |key: &str, label: &str| {
        let key = key.to_owned();
        confirm_then(
            asker,
            format!("Xoá «{label}» khỏi danh sách {}?", list.get()),
            "Không còn để chọn nữa; những gì đã ghi vẫn giữ nó.",
            "Xoá",
            move || {
                write.run(
                    move |db| db.archive_catalog_item(kind, &key),
                    move |result| {
                        if let Err(message) = result {
                            error.set(Some(message));
                        }
                    },
                );
            },
        );
    };
    view()
        .gap_2()
        .child(removable_tags(group, move || chips(&items(), &selected()), selected, on_toggle, remove))
        .child(add_field(format!("Thêm {named}"), placeholder, add))
        .child(dynamic(move || error.get().map(error_line)))
}

/// The chips of a catalog: its options in use, then the archived ones the record still holds.
fn chips(items: &[CatalogItem], selected: &[String]) -> Vec<TagOption> {
    let listed = items.iter().filter(|i| !i.archived).map(|i| TagOption::listed(i.key.clone(), i.label.clone()));
    let kept = items
        .iter()
        .filter(|i| i.archived && selected.contains(&i.key))
        .map(|i| TagOption::kept(i.key.clone(), i.label.clone()));
    listed.chain(kept).collect()
}

/// What a typed option is stored as: trimmed, and a timeframe in capitals (`m30` is `M30`).
fn label_of(kind: CatalogKind, typed: &str) -> String {
    match kind {
        CatalogKind::Timeframe => typed.trim().to_uppercase(),
        _ => typed.trim().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(key: &str) -> CatalogItem {
        CatalogItem {
            id: 0,
            kind: CatalogKind::Tag,
            key: key.into(),
            label: key.to_uppercase(),
            color: None,
            position: 0,
            archived: false,
        }
    }

    fn archived(key: &str) -> CatalogItem {
        CatalogItem { archived: true, ..listed(key) }
    }

    #[test]
    fn a_typed_option_is_trimmed_and_a_timeframe_is_in_capitals() {
        assert_eq!(label_of(CatalogKind::Timeframe, " m30 "), "M30");
        assert_eq!(label_of(CatalogKind::Tag, " London open "), "London open", "a tag keeps its case");
        assert_eq!(label_of(CatalogKind::Mistake, "   "), "");
    }

    #[test]
    fn an_archived_option_shows_only_while_the_record_holds_it_and_cannot_be_removed_again() {
        let items = [listed("a"), archived("b"), archived("c")];
        let shown = chips(&items, &["b".to_owned()]);
        assert_eq!(shown, [TagOption::listed("a", "A"), TagOption::kept("b", "B")], "c is archived and not held");
    }
}
