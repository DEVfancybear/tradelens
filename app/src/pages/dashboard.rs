//! Tổng quan: Kết quả giao dịch và kỷ luật của tài khoản đang chọn.
//! Design: `docs/design/screens/dashboard.html`: the headline figures, the balance and R
//! curves, the latest trades, today's discipline, the month's calendar and the result by symbol.

use std::sync::Arc;

use chrono::Datelike;
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    DbError, Rules,
    metrics::{RuleKind, RuleUsage, Unit},
};

use super::account_picker::account_picker;
use crate::{
    format::{self, Sign},
    insight::{self, Analysis},
    nav::{self, Route},
    snapshot::{Range, Snapshot},
    store::{Load, Reading, Store, query, store},
    trade_list::Filter,
    ui::{
        chart::equity_chart,
        charts::{DivergingItem, diverging_bars},
        chips::{Tone, chip},
        controls::{Variant, action, action_icon, link, segmented},
        forms::{RuleLevel, progress_rule},
        icons::Icon,
        layout::{columns, titled_card},
        month_grid::mini_month,
        parts::{empty_state, page_with, stat_tile, stat_tile_with, warning_banner},
        r_chart::r_curve_chart,
        table::recent_trades,
    },
};

use super::trades::cells_of;

/// How many of the latest trades the overview lists.
const RECENT: usize = 5;

/// How many symbols the overview's bars show.
const SYMBOLS_SHOWN: usize = 5;

/// Height of the latest-trades table: a header and one row per trade.
const RECENT_HEIGHT: f32 = 40.0 + 44.0 * 5.0 + 2.0;

/// What the big chart draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Curve {
    /// The balance after each closed trade, with its drawdown.
    Balance,
    /// The running total of R.
    R,
}

impl Curve {
    const ALL: [Self; 2] = [Self::Balance, Self::R];

    fn label(self) -> &'static str {
        match self {
            Self::Balance => "Balance",
            Self::R => "R",
        }
    }
}

pub(crate) fn dashboard() -> impl IntoElement {
    let store = store();
    let range = store.range;
    let today = computed(move || insight::today_now(&store.snapshot()));
    let analysis = computed(move || Arc::new(insight::analyse(&store.snapshot(), range.get(), today.get())));
    // Ctrl+N adds a trade by hand, as the button does (and as it does on the trades page).
    on_key_with("mod+n", KeyOptions { pass_through: true, ..KeyOptions::default() }, |_| nav::open_manual_entry());
    page_with(
        "Tổng quan",
        move || {
            let accounts = store.snapshot().accounts.len();
            format!("{} · {accounts} tài khoản", range.get().describe())
        },
        view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(account_picker())
            .child(segmented(
                "Khoảng thời gian",
                &Range::ALL,
                Range::label,
                move || range.get(),
                move |next| range.set(next),
            ))
            .child(action_icon(Icon::Plus, "Thêm lệnh", Variant::Primary, nav::open_manual_entry)),
        view().gap_6().child(stat_row()).child(columns(
            view().gap_6().child(curve_card(today)).child(recent_card()),
            view().gap_6().child(discipline_card(today)).child(month_card(today)).child(symbol_card(analysis)),
        )),
    )
}

fn money(snapshot: &Snapshot, value: f64, sign: Sign) -> String {
    format::money(value, sign, &snapshot.currency)
}

/// The deepest fall of the range, in money or (without money) in R, red when there was one.
fn drawdown_figure(s: &Snapshot) -> (String, Tone) {
    let (text, fell) = match s.unit() {
        Unit::Money => (money(s, -s.stats.max_drawdown, Sign::OnlyNegative), s.stats.max_drawdown > 0.0),
        Unit::R => (format::r_text(Some(-s.stats.max_drawdown_r)), s.stats.max_drawdown_r > 0.0),
    };
    (text, if fell { Tone::Loss } else { Tone::Neutral })
}

/// The line under the drawdown: how far from the peak, or what the R figure is.
fn drawdown_note(s: &Snapshot) -> String {
    match s.unit() {
        Unit::R => "sụt giảm sâu nhất của R tích luỹ".to_owned(),
        Unit::Money => s
            .stats
            .max_drawdown_pct
            .map_or_else(|| "chưa có đợt sụt giảm".to_owned(), |p| format!("−{} từ đỉnh", format::percent(p))),
    }
}

/// What a figure with nothing to count shows.
const NONE: &str = "—";

/// The line under the first tile when no trade closed in the range.
const NONE_HINT: &str = "Chưa có lệnh đóng trong khoảng này";

/// The line under the net result: how much of the balance it is, or what the trades are.
fn net_note(s: &Snapshot) -> String {
    let st = &s.stats;
    if let Some(mixed) = s.basis.money_note() {
        mixed
    } else if s.unit() == Unit::R {
        format!("{} lệnh đã đóng, tính theo R", st.trades)
    } else if st.start_balance > 0.0 {
        format!("{} so với số dư đầu kỳ", format::percent(st.net / st.start_balance))
    } else {
        format!("{} lệnh đã đóng", st.trades)
    }
}

/// The headline numbers of the chosen accounts and range: in money, or in R when none of the
/// trades in view has money (a journal kept in R), so that no tile ever says `$0.00`. With a mix,
/// the money figures cover the trades that have money and say so.
fn stat_row() -> impl IntoElement {
    let store = store();
    let unit = move || store.snapshot().unit();
    // Nothing closed in the range: every tile is a dash, never an amount of zero, and the first
    // one says why.
    let figure = move |pick: fn(&Snapshot) -> (String, Tone)| {
        move || {
            let snapshot = store.snapshot();
            if snapshot.stats.trades == 0 { (NONE.to_owned(), Tone::Neutral) } else { pick(&snapshot) }
        }
    };
    let note = move |pick: fn(&Snapshot) -> String| {
        move || {
            let snapshot = store.snapshot();
            if snapshot.stats.trades == 0 { String::new() } else { pick(&snapshot) }
        }
    };
    view()
        .flex_row()
        .flex_wrap()
        .gap_4()
        .child(stat_tile_with(
            move || if unit() == Unit::R { "Net R" } else { "Net P/L" },
            figure(|s| {
                let text = match s.unit() {
                    Unit::Money => money(s, s.stats.net, Sign::Always),
                    Unit::R => format::r_text(Some(s.stats.total_r)),
                };
                let tone = Tone::of_amount(&text);
                (text, tone)
            }),
            move || {
                let snapshot = store.snapshot();
                if snapshot.stats.trades == 0 { NONE_HINT.to_owned() } else { net_note(&snapshot) }
            },
        ))
        .child(stat_tile(
            "Win rate",
            figure(|s| {
                (if s.stats.trades == 0 { "—".to_owned() } else { format::percent(s.stats.win_rate) }, Tone::Neutral)
            }),
            note(|s| format!("{} thắng · {} thua", s.stats.wins, s.stats.losses)),
        ))
        .child(stat_tile(
            "Profit Factor",
            figure(|s| {
                let factor = match s.unit() {
                    Unit::Money => s.stats.profit_factor,
                    Unit::R => s.stats.profit_factor_r,
                };
                (format::ratio(factor), Tone::Neutral)
            }),
            note(|s| match s.unit() {
                Unit::Money => format!(
                    "Lãi gộp {} / lỗ {}",
                    money(s, s.stats.gross_profit, Sign::OnlyNegative),
                    money(s, -s.stats.gross_loss, Sign::OnlyNegative)
                ),
                Unit::R => "tổng R thắng / tổng R thua".to_owned(),
            }),
        ))
        .child(stat_tile(
            "Expectancy",
            figure(|s| {
                let text = match s.unit() {
                    Unit::Money => money(s, s.stats.expectancy, Sign::Always),
                    Unit::R => format::r_average(s.stats.avg_r),
                };
                let tone = Tone::of_amount(&text);
                (text, tone)
            }),
            note(|s| match s.unit() {
                Unit::Money => "trung bình mỗi lệnh".to_owned(),
                Unit::R => "R trung bình mỗi lệnh".to_owned(),
            }),
        ))
        .child(stat_tile("Max drawdown", figure(drawdown_figure), note(drawdown_note)))
        .child(stat_tile_with(
            move || if unit() == Unit::R { "Lệnh đã đóng" } else { "R trung bình" },
            figure(|s| match s.unit() {
                Unit::R => (s.stats.trades.to_string(), Tone::Neutral),
                Unit::Money => {
                    let text = format::r_average(s.stats.avg_r);
                    let tone = Tone::of_amount(&text);
                    (text, tone)
                }
            }),
            note(|s| match s.unit() {
                Unit::R => format!("{} lệnh có R", s.stats.r_trades),
                Unit::Money => format!("{} lệnh có SL", s.stats.r_trades),
            }),
        ))
}

/// The big chart: the balance with its drawdown, or the running total of R.
fn curve_card(today: Memo<chrono::NaiveDate>) -> impl IntoElement {
    let store = store();
    let curve = signal(Curve::Balance);
    // The chart opens on the R tab when the trades in view have no money, until the trader picks a tab.
    let chosen = stored(false);
    effect(move || {
        if store.load_state() != Load::Ready {
            return;
        }
        let unit = store.snapshot().unit();
        untracked(|| {
            if !chosen.get() {
                curve.set(if unit == Unit::R { Curve::R } else { Curve::Balance });
            }
        });
    });
    titled_card(
        move || match curve.get() {
            Curve::Balance => "Balance & drawdown".to_owned(),
            Curve::R => "R tích luỹ".to_owned(),
        },
        segmented(
            "Biểu đồ",
            &Curve::ALL,
            Curve::label,
            move || curve.get(),
            move |next| {
                chosen.replace(true);
                curve.set(next);
            },
        ),
        view().child(dynamic(move || match curve.get() {
            Curve::Balance => untracked(|| balance_body().into_element()),
            Curve::R => untracked(|| r_body(today).into_element()),
        })),
    )
}

fn balance_body() -> impl IntoElement {
    let store = store();
    let range = store.range;
    // A memo, so the chart is made once and follows the data instead of being drawn in again
    // at every reload.
    let drawable = computed(move || {
        let snapshot = store.snapshot();
        snapshot.unit() == Unit::Money && snapshot.curve.len() > 1
    });
    let in_r = computed(move || store.snapshot().unit() == Unit::R);
    view().child(dynamic(move || {
        if in_r.get() {
            // A balance line of zeros says nothing: the journal is kept in R.
            empty_state("Nhật ký này ghi kết quả theo R — xem tab R", "Chưa có số tiền để vẽ đường số dư.")
                .into_element()
        } else if drawable.get() {
            equity_chart(
                move || store.snapshot().curve.clone(),
                move || store.snapshot().currency.clone(),
                move || u32::try_from(Range::ALL.iter().position(|r| *r == range.get()).unwrap_or(0)).unwrap_or(0),
            )
            .into_element()
        } else {
            let (title, hint) = match store.load_state() {
                Load::Loading => ("Đang tải dữ liệu…".to_owned(), String::new()),
                _ => (
                    "Chưa có đủ lệnh đóng để vẽ đường số dư".to_owned(),
                    "Chọn khoảng thời gian dài hơn hoặc import thêm lệnh ở Nguồn dữ liệu.".to_owned(),
                ),
            };
            empty_state(title, hint).into_element()
        }
    }))
}

fn r_body(today: Memo<chrono::NaiveDate>) -> impl IntoElement {
    let store = store();
    let range = store.range;
    let points = computed(move || insight::r_curve(&store.snapshot(), range.get(), today.get()));
    let drawable = computed(move || points.with(|p| p.len() > 1));
    view().child(dynamic(move || {
        if drawable.get() {
            r_curve_chart(
                move || points.with(|p| p.iter().map(|&(_, total_r)| total_r).collect()),
                move || u32::try_from(Range::ALL.iter().position(|r| *r == range.get()).unwrap_or(0)).unwrap_or(0),
            )
            .into_element()
        } else {
            empty_state("Chưa có lệnh nào đo được R", "Đặt SL hoặc ghi rủi ro dự định ở nhật ký lệnh để tính R.")
                .into_element()
        }
    }))
}

fn recent_card() -> impl IntoElement {
    let store = store();
    let rows = move || {
        let snapshot = store.snapshot();
        snapshot.rows_in_range().take(RECENT).map(cells_of).collect()
    };
    let open = move |at: usize| {
        if let Some(row) = store.snapshot_now().rows_in_range().nth(at) {
            nav::show_trade(row.key().clone());
        }
    };
    let empty =
        || ("Chưa có lệnh nào trong khoảng này".to_owned(), "Chọn khoảng thời gian dài hơn để thấy thêm.".to_owned());
    titled_card(
        "Lệnh gần đây",
        link("Xem tất cả", || nav::go(Route::Trades)),
        view().h(px(RECENT_HEIGHT)).child(recent_trades(rows, open, empty)),
    )
}

/// The rules of the chosen accounts, read from the database.
fn rules_reading() -> Reading<Vec<(i64, Rules)>> {
    query(|db, account| -> Result<Vec<(i64, Rules)>, DbError> {
        let ids: Vec<i64> = match account {
            Some(id) => vec![id],
            None => db.accounts()?.iter().map(|a| a.id).collect(),
        };
        ids.into_iter().map(|id| Ok((id, db.rules(id)?))).collect()
    })
}

/// A rule's name, and what is used of its limit as text.
fn usage_line(usage: &RuleUsage, snapshot: &Snapshot) -> (&'static str, String) {
    let whole = |value: f64| format::money_whole(value, Sign::OnlyNegative, &snapshot.currency);
    let count = |value: f64| format!("{value:.0}");
    match usage.kind {
        RuleKind::DailyLoss => ("Lỗ trong ngày", format!("{} / {}", whole(usage.used), whole(usage.limit))),
        RuleKind::DailyTrades => ("Số lệnh", format!("{} / {}", count(usage.used), count(usage.limit))),
        RuleKind::RiskPerTrade => ("Risk lệnh lớn nhất", format!("{:.1}% / {:.1}%", usage.used, usage.limit)),
        RuleKind::LosingStreak => ("Thua liên tiếp", format!("{} / {}", count(usage.used), count(usage.limit))),
    }
}

/// The first rule that is close to or past its limit, in words.
fn warning_of(usage: &[RuleUsage], snapshot: &Snapshot) -> Option<String> {
    usage.iter().find(|u| RuleLevel::of(u.ratio()) != RuleLevel::Within).map(|u| {
        let (name, summary) = usage_line(u, snapshot);
        match RuleLevel::of(u.ratio()) {
            RuleLevel::Over => format!("{name} đã chạm giới hạn ({summary})"),
            _ => format!("{name} gần chạm giới hạn ({summary})"),
        }
    })
}

/// What today has used of the account's rules.
fn discipline_card(today: Memo<chrono::NaiveDate>) -> impl IntoElement {
    let store = store();
    let rules = rules_reading();
    // The violations of today; a closure of its own so the chip and the bars agree.
    let violations_today = move || {
        let Some(read) = rules.value() else { return 0 };
        let snapshot = store.snapshot();
        let day = today.get();
        insight::violations(&snapshot, &read).iter().filter(|v| v.day == day).count()
    };
    titled_card(
        "Kỷ luật hôm nay",
        dynamic(move || {
            let count = violations_today();
            let tone = if count == 0 { Tone::Profit } else { Tone::Loss };
            chip(format!("{count} vi phạm"), tone)
        }),
        view().child(dynamic(move || {
            let read = rules.value();
            let snapshot = store.snapshot();
            let day = today.get();
            untracked(|| {
                let closest = read.as_deref().and_then(|r| insight::closest_to_limits(&snapshot, r, day));
                let Some((account, usage)) = closest else {
                    return view()
                        .gap_3()
                        .child(empty_state(
                            "Chưa đặt quy tắc kỷ luật",
                            "Đặt giới hạn lỗ và số lệnh mỗi ngày ở Tài khoản & kỷ luật.",
                        ))
                        .child(action("Đặt quy tắc", Variant::Secondary, || nav::go(Route::Accounts)))
                        .into_element();
                };
                let warn = warning_of(&usage, &snapshot);
                // With several accounts under rules, say whose day the bars measure.
                let several =
                    read.as_deref().is_some_and(|r| r.iter().filter(|(_, s)| *s != Rules::default()).count() > 1);
                let whose = several.then(|| insight::account_title(&snapshot, account)).flatten();
                view()
                    .gap_3()
                    .child(whose.map(|name| text(name).text_xs().text_color(move || theme().muted_foreground)))
                    .children(usage.into_iter().map(|u| {
                        let (name, summary) = usage_line(&u, &snapshot);
                        progress_rule(name, move || summary.clone(), move || u.ratio())
                    }))
                    .children(warn.map(warning_banner))
                    .into_element()
            })
        })),
    )
}

/// The title of the month's calendar: `Tháng 11/2026`.
fn month_title(day: chrono::NaiveDate) -> String {
    format!("Tháng {}/{}", day.month(), day.year())
}

/// The month's calendar in small, with a link to the full one. It shows the current month, or the
/// latest one that has trades when the current month has none (as the calendar page opens), and its
/// title says which.
fn month_card(today: Memo<chrono::NaiveDate>) -> impl IntoElement {
    let store = store();
    let days = computed(move || insight::daily_results(&store.snapshot()));
    let shown = computed(move || days.with(|d| insight::opening_day(d, today.get())));
    let weeks = move || {
        let day = shown.get();
        days.with(|d| tradelens_core::metrics::month_weeks(day.year(), day.month(), d))
    };
    titled_card(
        move || month_title(shown.get()),
        link("Mở lịch", || nav::go(Route::Calendar)),
        mini_month(weeks, today, super::calendar::show_day),
    )
}

/// The symbols that made or lost the most, best first.
fn symbol_items(analysis: &Analysis, currency: &str) -> Vec<DivergingItem> {
    // In R when none of the trades has money, so the bars are never a row of $0.00.
    let unit = analysis.unit();
    let result = |g: &&tradelens_core::metrics::Group| analysis.result_of(g);
    let mut groups: Vec<&tradelens_core::metrics::Group> = analysis.by_symbol.iter().collect();
    groups.sort_by(|a, b| result(b).abs().total_cmp(&result(a).abs()));
    groups.truncate(SYMBOLS_SHOWN);
    groups.sort_by(|a, b| result(b).total_cmp(&result(a)));
    groups
        .into_iter()
        .map(|g| DivergingItem {
            label: g.label.clone(),
            shown: match unit {
                Unit::Money => format::money(g.net, Sign::Always, currency),
                Unit::R => format::r_text(Some(g.total_r)),
            },
            value: analysis.result_of(g),
        })
        .collect()
}

fn symbol_card(analysis: Memo<Arc<Analysis>>) -> impl IntoElement {
    let store: Store = store();
    titled_card(
        "Theo symbol",
        link("Xem phân tích", || nav::go(Route::Analytics)),
        diverging_bars(move || analysis.with(|a| symbol_items(a, &store.snapshot().currency)), show_symbol),
    )
}

/// Opens the trades list on the trades of `symbol` alone.
fn show_symbol(symbol: &str) {
    let store = store();
    store.trade_filter.set(Filter { symbol: Some(symbol.to_owned()), ..Filter::default() });
    nav::go(Route::Trades);
}

#[cfg(test)]
mod tests {
    use tradelens_core::metrics::Group;

    use super::*;

    fn group(label: &str, net: f64) -> Group {
        Group {
            label: label.into(),
            trades: 1,
            wins: usize::from(net > 0.0),
            net,
            gross_profit: net.max(0.0),
            gross_loss: net.min(0.0),
            total_r: 0.0,
            r_trades: 0,
            r_profit: 0.0,
            r_loss: 0.0,
            keys: Vec::new(),
        }
    }

    #[test]
    fn the_symbol_bars_keep_the_biggest_movers_best_first() {
        let analysis = Analysis {
            by_symbol: vec![
                group("A", 10.0),
                group("B", -300.0),
                group("C", 500.0),
                group("D", 1.0),
                group("E", -20.0),
                group("F", 40.0),
                group("G", -5.0),
            ],
            ..Analysis::default()
        };
        let items = symbol_items(&analysis, "USD");
        let names: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(names, ["C", "F", "A", "E", "B"], "the five largest by size, ordered by result");
        assert_eq!(items[0].shown, "+$500.00");
    }

    #[test]
    fn a_rule_close_to_its_limit_is_worded_as_a_warning() {
        let snapshot = Snapshot::default();
        let near = [RuleUsage { kind: RuleKind::RiskPerTrade, used: 1.3, limit: 1.5 }];
        assert_eq!(warning_of(&near, &snapshot).as_deref(), Some("Risk lệnh lớn nhất gần chạm giới hạn (1.3% / 1.5%)"));
        let over = [RuleUsage { kind: RuleKind::DailyTrades, used: 5.0, limit: 5.0 }];
        assert_eq!(warning_of(&over, &snapshot).as_deref(), Some("Số lệnh đã chạm giới hạn (5 / 5)"));
        let fine = [RuleUsage { kind: RuleKind::DailyTrades, used: 1.0, limit: 5.0 }];
        assert_eq!(warning_of(&fine, &snapshot), None);
    }

    #[test]
    fn the_loss_limit_reads_in_money() {
        let snapshot = Snapshot::default();
        let usage = RuleUsage { kind: RuleKind::DailyLoss, used: 120.0, limit: 300.0 };
        assert_eq!(usage_line(&usage, &snapshot), ("Lỗ trong ngày", "$120 / $300".to_owned()));
    }
}
