//! Tìm kiếm: the palette that opens with Ctrl+K or from the sidebar's search box.
//! Design: `docs/design/screens/search.html`: results grouped by kind with the words that
//! matched marked, the kind as a filter with its count, Enter opens the result.

use nsh_core::prelude::*;
use tradelens_core::search::{DocKind, Hit, Target};

use super::{missed, playbook, review};
use crate::{
    format,
    nav::{self, Route},
    store::{query_with, store},
    ui::{
        controls::tag_picker,
        palette::{PaletteRow, command_palette, palette_layer},
    },
};

/// How many results one search asks for.
const LIMIT: u32 = 40;

/// The key of the filter that shows every kind.
const ALL: &str = "all";

/// Whether the palette is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Open(bool);

/// Opens the palette.
pub(crate) fn open() {
    global::<Open>().set(Open(true));
}

/// Closes the palette.
pub(crate) fn close() {
    global::<Open>().set(Open(false));
}

/// Opens the palette, or closes it when it is open: what Ctrl+K does.
pub(crate) fn toggle() {
    let open = global::<Open>();
    open.set(Open(!open.get_untracked().0));
}

/// The palette over the window while it is open; nothing while it is not. The last child of the
/// window's root.
pub(crate) fn overlay() -> impl IntoElement {
    let open = global::<Open>();
    when(move || open.get().0, || untracked(layer))
}

/// A kind's key in the filter.
fn kind_key(kind: DocKind) -> &'static str {
    match kind {
        DocKind::Trade => "trade",
        DocKind::Day => "day",
        DocKind::Review => "review",
        DocKind::Missed => "missed",
        DocKind::Playbook => "playbook",
    }
}

/// The id of a hit among the palette's rows.
fn row_id(hit: &Hit) -> String {
    format!("{}:{}", kind_key(hit.kind), hit.reference)
}

/// The hits ordered by kind (the order of the groups), the search's own order kept within a kind,
/// and only `kind` when one is chosen.
fn arranged(hits: &[Hit], kind: Option<DocKind>) -> Vec<Hit> {
    let mut shown: Vec<Hit> = hits.iter().filter(|h| kind.is_none_or(|k| k == h.kind)).cloned().collect();
    shown.sort_by_key(|h| DocKind::ALL.iter().position(|k| *k == h.kind));
    shown
}

/// The filter's options: all results and each kind that has some, with how many.
fn filter_options(hits: &[Hit]) -> Vec<(String, String)> {
    if hits.is_empty() {
        return Vec::new();
    }
    let mut options = vec![(ALL.to_owned(), format!("Tất cả · {}", hits.len()))];
    for kind in DocKind::ALL {
        let count = hits.iter().filter(|h| h.kind == kind).count();
        if count > 0 {
            options.push((kind_key(kind).to_owned(), format!("{} · {count}", kind.label())));
        }
    }
    options
}

/// What a result says at its right.
fn meta_of(hit: &Hit, result: Option<f64>) -> String {
    match hit.kind {
        DocKind::Trade => format::r_text(result),
        DocKind::Day => "Nhật ký ngày".to_owned(),
        DocKind::Review => match hit.target() {
            Some(Target::Review { period, .. }) => format!("Tổng kết {}", period_word(period)),
            _ => "Tổng kết".to_owned(),
        },
        DocKind::Missed => "Lệnh bỏ lỡ".to_owned(),
        DocKind::Playbook => "Playbook".to_owned(),
    }
}

fn period_word(period: tradelens_core::review::Period) -> &'static str {
    match period {
        tradelens_core::review::Period::Week => "tuần",
        tradelens_core::review::Period::Month => "tháng",
    }
}

/// Shows what a result points at.
fn show(target: Target) {
    match target {
        Target::Trade(key) => nav::show_trade(key),
        Target::Day { day, account_id } => nav::show_journal_day(day, account_id),
        Target::Review { period, start, .. } => {
            review::jump(period, start);
            nav::go(Route::Review);
        }
        Target::Missed(id) => {
            missed::select(id);
            nav::go(Route::Missed);
        }
        Target::Playbook(id) => {
            playbook::select(id);
            nav::go(Route::Playbook);
        }
    }
}

fn layer() -> impl IntoElement {
    let store = store();
    let typed = signal(String::new());
    let kind = signal(None::<DocKind>);
    let found = query_with(move || typed.get(), |db, _, text| db.search(text, LIMIT));
    // Results of an earlier query stay in the reading while the next one is made: none show
    // for an empty field.
    let hits = computed(move || {
        if typed.with(|t| t.trim().is_empty()) {
            return Vec::new();
        }
        found.value().map_or_else(Vec::new, |h| h.to_vec())
    });
    let shown = computed(move || arranged(&hits.get(), kind.get()));
    let rows = move || {
        let snapshot = store.snapshot();
        shown
            .get()
            .iter()
            .map(|hit| {
                let result = match hit.target() {
                    Some(Target::Trade(key)) => snapshot.row(&key).and_then(|row| row.r),
                    _ => None,
                };
                PaletteRow {
                    id: row_id(hit),
                    group: hit.kind.label().to_owned(),
                    title: hit.title.clone(),
                    snippet: hit.snippet.clone(),
                    meta: meta_of(hit, result),
                }
            })
            .collect()
    };
    let chips = tag_picker(
        "Loại kết quả",
        move || filter_options(&hits.get()),
        move || vec![kind.get().map_or(ALL, kind_key).to_owned()],
        move |key, on| kind.set(if on { DocKind::ALL.into_iter().find(|k| kind_key(*k) == key) } else { None }),
    );
    let hint = move || {
        if typed.with(|t| t.trim().is_empty()) {
            "Gõ để tìm trong lệnh, nhật ký ngày, tổng kết, lệnh bỏ lỡ và playbook — không cần gõ dấu.".to_owned()
        } else if found.loading() {
            "Đang tìm…".to_owned()
        } else {
            format!("Không có kết quả cho “{}”.", typed.get().trim())
        }
    };
    palette_layer(
        command_palette(
            rows,
            typed,
            chips,
            hint,
            move || found.loading(),
            move |id| {
                let target = shown.with(|hits| hits.iter().find(|h| row_id(h) == id).and_then(Hit::target));
                close();
                if let Some(target) = target {
                    show(target);
                }
            },
            close,
        ),
        close,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(kind: DocKind, reference: &str) -> Hit {
        Hit { kind, reference: reference.into(), title: String::new(), snippet: String::new() }
    }

    #[test]
    fn results_are_grouped_by_kind_and_filtered() {
        let hits =
            [hit(DocKind::Playbook, "1"), hit(DocKind::Trade, "a"), hit(DocKind::Day, "d"), hit(DocKind::Trade, "b")];
        let all = arranged(&hits, None);
        let order: Vec<&str> = all.iter().map(|h| h.reference.as_str()).collect();
        assert_eq!(order, ["a", "b", "d", "1"], "trades first, then days, then setups; a kind keeps its order");
        let trades = arranged(&hits, Some(DocKind::Trade));
        assert_eq!(trades.len(), 2);
    }

    #[test]
    fn the_filter_counts_each_kind_that_has_hits() {
        assert_eq!(filter_options(&[]).len(), 0, "no hits, no filter");
        let hits = [hit(DocKind::Trade, "a"), hit(DocKind::Trade, "b"), hit(DocKind::Review, "week:2026-11-16:0")];
        assert_eq!(
            filter_options(&hits),
            [
                ("all".to_owned(), "Tất cả · 3".to_owned()),
                ("trade".to_owned(), "Lệnh · 2".to_owned()),
                ("review".to_owned(), "Tổng kết · 1".to_owned()),
            ]
        );
    }

    #[test]
    fn a_trade_says_its_result_and_other_kinds_name_themselves() {
        assert_eq!(meta_of(&hit(DocKind::Trade, "1:1:0"), Some(-1.0)), "\u{2212}1.0R");
        assert_eq!(meta_of(&hit(DocKind::Day, "2026-11-20"), None), "Nhật ký ngày");
        assert_eq!(meta_of(&hit(DocKind::Review, "month:2026-11-01:0"), None), "Tổng kết tháng");
    }
}
