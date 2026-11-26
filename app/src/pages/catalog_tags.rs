//! A pick from one of the trader's catalogs (tags, mistakes, timeframes, reasons for a missed trade)
//! that keeps the catalog too: a new option is typed and Enter adds it and picks it; the ✕ after an
//! option removes it from the list after asking. Removing archives the option, so what already
//! stored it (a trade's journal, a playbook, a plan) still reads its label.

use nsh_core::prelude::*;
use tradelens_core::catalog::CatalogKind;

use crate::{
    store::writer,
    ui::{
        controls::{add_field, confirm_then, removable_tags, root_owner},
        parts::error_line,
    },
};

/// The chips of `kind`'s options (`options`, as `(key, label)`), on while `selected` holds their key;
/// `on_toggle` hears a pick, and also the option just added (on) and the one just removed (off).
/// `group` names the list ("Tag", "Lỗi", "Khung thời gian") and `placeholder` fills the add field.
pub(super) fn catalog_tags(
    kind: CatalogKind,
    group: &'static str,
    placeholder: &'static str,
    options: impl Fn() -> Vec<(String, String)> + 'static,
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
    let list = named.clone();
    let remove = move |key: &str, label: &str| {
        let key = key.to_owned();
        confirm_then(
            asker,
            format!("Xoá «{label}» khỏi danh sách {list}?"),
            "Không còn để chọn nữa; lệnh, playbook và kế hoạch đã ghi vẫn giữ nó.",
            "Xoá",
            move || {
                let archived = key.clone();
                write.run(
                    move |db| db.archive_catalog_item(kind, &archived),
                    move |result| match result {
                        Ok(()) if selected().contains(&key) => on_toggle(&key, false),
                        Ok(()) => {}
                        Err(message) => error.set(Some(message)),
                    },
                );
            },
        );
    };
    view()
        .gap_2()
        .child(removable_tags(group, options, selected, on_toggle, remove))
        .child(add_field(format!("Thêm {named}"), placeholder, add))
        .child(dynamic(move || error.get().map(error_line)))
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

    #[test]
    fn a_typed_option_is_trimmed_and_a_timeframe_is_in_capitals() {
        assert_eq!(label_of(CatalogKind::Timeframe, " m30 "), "M30");
        assert_eq!(label_of(CatalogKind::Tag, " London open "), "London open", "a tag keeps its case");
        assert_eq!(label_of(CatalogKind::Mistake, "   "), "");
    }
}
