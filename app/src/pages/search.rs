//! Tìm kiếm (search): the palette that opens with Ctrl+K or from the sidebar's search box.
//! Design: `docs/design/screens/search.html`: results grouped by kind with the words that
//! matched marked, the kind as a filter with its count, Enter opens the result.

use nsh_core::prelude::*;
use tradelens_core::search::{DocKind, Hit, Target};

use super::{missed, playbook, review};
use crate::{
    format,
    nav::{self, Route},
    snapshot::{Snapshot, TradeRow},
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

/// The trades whose symbol starts with, or contains, every word typed (`xag` finds `XAGUSDm`), or
/// whose position number is a word typed, newest first, at most `limit`. The full-text index holds
/// only what the trader wrote, so a trade without a journal (one the MT5 sync just stored) is found
/// here; titles read like the index's (`XAGUSDm Buy 27/11`).
fn trades_by_symbol(snapshot: &Snapshot, typed: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut rows: Vec<&TradeRow> = snapshot
        .rows
        .iter()
        .filter(|row| {
            let symbol = row.trade.symbol.to_lowercase();
            let ticket = row.trade.key.position_id.to_string();
            words.iter().all(|word| symbol.contains(word.as_str()) || ticket == *word)
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.sort_time()));
    rows.into_iter()
        .take(limit)
        .map(|row| {
            let trade = &row.trade;
            let opened = chrono::DateTime::from_timestamp_millis(trade.open_time)
                .map_or_else(String::new, |at| at.format(" %d/%m").to_string());
            Hit {
                kind: DocKind::Trade,
                reference: trade.key.id(),
                title: format!("{} {}{opened}", trade.symbol, trade.side.label()),
                snippet: String::new(),
            }
        })
        .collect()
}

/// The index's hits, then the trades found by symbol that the index did not already give.
fn with_trades(mut hits: Vec<Hit>, by_symbol: Vec<Hit>) -> Vec<Hit> {
    for hit in by_symbol {
        if !hits.iter().any(|h| h.kind == hit.kind && h.reference == hit.reference) {
            hits.push(hit);
        }
    }
    hits
}

/// The filter's options: all results and each kind that has some, with how many.
fn filter_options(hits: &[Hit]) -> Vec<(String, String)> {
    if hits.is_empty() {
        return Vec::new();
    }
    let mut options = vec![(ALL.to_owned(), t!("search.filter_all", n = hits.len()))];
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
        DocKind::Day => t!("search.meta.day"),
        DocKind::Review => match hit.target() {
            Some(Target::Review { period: tradelens_core::review::Period::Week, .. }) => t!("search.meta.review_week"),
            Some(Target::Review { period: tradelens_core::review::Period::Month, .. }) => {
                t!("search.meta.review_month")
            }
            _ => t!("search.meta.review"),
        },
        DocKind::Missed => t!("search.meta.missed"),
        DocKind::Playbook => "Playbook".to_owned(),
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
        let text = typed.get();
        if text.trim().is_empty() {
            return Vec::new();
        }
        let indexed = found.value().map_or_else(Vec::new, |h| h.to_vec());
        let by_symbol = trades_by_symbol(&store.snapshot(), &text, usize::try_from(LIMIT).unwrap_or(usize::MAX));
        with_trades(indexed, by_symbol)
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
                    group: hit.kind.label(),
                    title: hit.title.clone(),
                    snippet: hit.snippet.clone(),
                    meta: meta_of(hit, result),
                }
            })
            .collect()
    };
    let chips = tag_picker(
        t!("search.filter_group"),
        move || filter_options(&hits.get()),
        move || vec![kind.get().map_or(ALL, kind_key).to_owned()],
        move |key, on| kind.set(if on { DocKind::ALL.into_iter().find(|k| kind_key(*k) == key) } else { None }),
    );
    let hint = move || {
        if typed.with(|t| t.trim().is_empty()) {
            t!("search.hint")
        } else if found.loading() {
            t!("search.searching")
        } else {
            t!("search.no_results", query = typed.get().trim())
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

    /// Two trades without a journal: `XAGUSDm` (position 5149465589) and `XAUUSDm`, the latter newer.
    fn unjournaled() -> Snapshot {
        use tradelens_core::{
            Db, DealBatch, DealEntry, DealKind,
            model::{AccountInfo, Deal},
            time::parse_mt5_time,
        };
        let deal = |ticket: i64, position: i64, symbol: &str, at: &str, kind: DealKind, entry: DealEntry| Deal {
            ticket,
            order: ticket,
            position_id: position,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: symbol.into(),
            kind,
            entry,
            volume: 10.0,
            price: 30.0,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit: 0.0,
            magic: 0,
            comment: String::new(),
            sl: 0.0,
            tp: 0.0,
            risk: None,
            result_r: None,
            gaps: tradelens_core::Gaps::NONE,
            reason: None,
        };
        let deals = vec![
            deal(1, 5_149_465_589, "XAGUSDm", "2026.11.27 02:33:03", DealKind::Buy, DealEntry::In),
            deal(2, 5_149_465_589, "XAGUSDm", "2026.11.27 02:34:33", DealKind::Sell, DealEntry::Out),
            deal(3, 5_149_518_021, "XAUUSDm", "2026.11.27 02:35:48", DealKind::Buy, DealEntry::In),
            deal(4, 5_149_518_021, "XAUUSDm", "2026.11.27 02:36:33", DealKind::Sell, DealEntry::Out),
        ];
        let mut db = Db::in_memory().expect("a memory database opens");
        let account = AccountInfo { login: "1".into(), server: "S".into(), ..AccountInfo::default() };
        db.upsert_batch(&DealBatch { account, deals, positions: Vec::new() }, "test").expect("stored");
        let now = parse_mt5_time("2026.11.27 03:00:00").expect("a valid time");
        let query = crate::snapshot::Query {
            account: None,
            range: crate::snapshot::Range::All,
            now_utc_ms: now,
            local_offset_min: 420,
        };
        crate::snapshot::load(&db, &query).expect("loads")
    }

    #[test]
    fn a_trade_without_a_journal_is_found_by_part_of_its_symbol_or_its_ticket() {
        let snapshot = unjournaled();
        let xag = trades_by_symbol(&snapshot, "xag", 40);
        assert_eq!(xag.len(), 1, "{xag:?}");
        assert_eq!(xag[0].title, "XAGUSDm Buy 27/11");
        assert_eq!(xag[0].kind, DocKind::Trade);
        let both: Vec<String> = trades_by_symbol(&snapshot, "USD", 40).into_iter().map(|h| h.title).collect();
        assert_eq!(both, ["XAUUSDm Buy 27/11", "XAGUSDm Buy 27/11"], "newest first");
        assert_eq!(trades_by_symbol(&snapshot, "5149465589", 40).len(), 1, "by ticket");
        assert_eq!(trades_by_symbol(&snapshot, "  ", 40), [], "nothing typed");
        assert_eq!(trades_by_symbol(&snapshot, "eurusd", 40), [], "no such symbol");

        // A trade the index found already is not listed twice.
        let indexed =
            vec![Hit { title: "XAGUSDm Buy 27/11".into(), snippet: "[xag] breakout".into(), ..xag[0].clone() }];
        let merged = with_trades(indexed, trades_by_symbol(&snapshot, "usd", 40));
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].snippet, "[xag] breakout", "the index's hit comes first and is kept");
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
        crate::i18n::with_text(crate::i18n::Lang::Vi, || {
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
        });
    }

    #[test]
    fn a_trade_says_its_result_and_other_kinds_name_themselves() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, || {
            assert_eq!(meta_of(&hit(DocKind::Trade, "1:1:0"), Some(-1.0)), "\u{2212}1.0R");
            assert_eq!(meta_of(&hit(DocKind::Day, "2026-11-20"), None), "Nhật ký ngày");
            assert_eq!(meta_of(&hit(DocKind::Review, "month:2026-11-01:0"), None), "Tổng kết tháng");
        });
        crate::i18n::with_text(crate::i18n::Lang::En, || {
            assert_eq!(meta_of(&hit(DocKind::Review, "month:2026-11-01:0"), None), "Month review");
            assert_eq!(meta_of(&hit(DocKind::Missed, "3"), None), "Missed trades");
        });
    }
}
