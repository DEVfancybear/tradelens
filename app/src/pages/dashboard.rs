//! Tổng quan: Kết quả giao dịch và kỷ luật của tài khoản đang chọn.
//! Design: `docs/design/screens/dashboard.html` (v0.1: the headline figures, the balance
//! curve and the latest trades; the other panels arrive in v0.2).

use nsh_core::prelude::*;

use super::account_picker::account_picker;
use crate::{
    format::{self, Sign},
    nav,
    snapshot::{Range, Snapshot},
    store::{Load, store},
    ui::{
        chart::equity_chart,
        chips::Tone,
        controls::segmented,
        parts::{card, empty_state, page_with, planned, stat_tile},
        table::recent_trades,
    },
};

use super::trades::cells_of;

const PLANNED: &[&str] = &[
    "Kỷ luật hôm nay: lỗ trong ngày, số lệnh, vi phạm quy tắc",
    "Lịch P/L thu nhỏ của tháng này",
    "Kết quả theo symbol",
];

/// How many of the latest trades the overview lists.
const RECENT: usize = 5;

/// Height of the latest-trades table: a header and one row per trade.
const RECENT_HEIGHT: f32 = 40.0 + 44.0 * 5.0 + 2.0;

pub(crate) fn dashboard() -> impl IntoElement {
    let store = store();
    let range = store.range;
    page_with(
        "Tổng quan",
        move || {
            let accounts = store.snapshot().accounts.len();
            format!("{} · {accounts} tài khoản MT5", range.get().describe())
        },
        view().flex_row().flex_wrap().items_center().gap_3().child(account_picker()).child(segmented(
            "Khoảng thời gian",
            &Range::ALL,
            Range::label,
            move || range.get(),
            move |next| range.set(next),
        )),
        view().gap_6().child(stat_row()).child(balance_card()).child(recent_card()).child(planned(PLANNED)),
    )
}

fn money(snapshot: &Snapshot, value: f64, sign: Sign) -> String {
    format::money(value, sign, &snapshot.currency)
}

/// The headline numbers of the chosen accounts and range.
fn stat_row() -> impl IntoElement {
    let store = store();
    let figure = move |pick: fn(&Snapshot) -> (String, Tone)| move || pick(&store.snapshot());
    let note = move |pick: fn(&Snapshot) -> String| move || pick(&store.snapshot());
    view()
        .flex_row()
        .flex_wrap()
        .gap_4()
        .child(stat_tile(
            "Net P/L",
            figure(|s| {
                let text = money(s, s.stats.net, Sign::Always);
                let tone = Tone::of_amount(&text);
                (text, tone)
            }),
            note(|s| {
                let st = &s.stats;
                if st.start_balance > 0.0 {
                    format!("{} so với số dư đầu kỳ", format::percent(st.net / st.start_balance))
                } else {
                    format!("{} lệnh đã đóng", st.trades)
                }
            }),
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
            figure(|s| (format::ratio(s.stats.profit_factor), Tone::Neutral)),
            note(|s| {
                format!(
                    "Lãi gộp {} / lỗ {}",
                    money(s, s.stats.gross_profit, Sign::OnlyNegative),
                    money(s, -s.stats.gross_loss, Sign::OnlyNegative)
                )
            }),
        ))
        .child(stat_tile(
            "Expectancy",
            figure(|s| {
                let text = money(s, s.stats.expectancy, Sign::Always);
                let tone = Tone::of_amount(&text);
                (text, tone)
            }),
            note(|_| "trung bình mỗi lệnh".to_owned()),
        ))
        .child(stat_tile(
            "Max drawdown",
            figure(|s| {
                let text = money(s, -s.stats.max_drawdown, Sign::OnlyNegative);
                let tone = if s.stats.max_drawdown > 0.0 { Tone::Loss } else { Tone::Neutral };
                (text, tone)
            }),
            note(|s| {
                s.stats
                    .max_drawdown_pct
                    .map_or_else(|| "chưa có đợt sụt giảm".to_owned(), |p| format!("−{} từ đỉnh", format::percent(p)))
            }),
        ))
        .child(stat_tile(
            "R trung bình",
            figure(|s| {
                let text = format::r_average(s.stats.avg_r);
                let tone = Tone::of_amount(&text);
                (text, tone)
            }),
            note(|s| format!("{} lệnh có SL", s.stats.r_trades)),
        ))
}

fn balance_card() -> impl IntoElement {
    let store = store();
    let range = store.range;
    // A memo, so the chart is made once and follows the data instead of being drawn in again
    // at every reload.
    let drawable = computed(move || store.snapshot().curve.len() > 1);
    card(
        "Balance & drawdown",
        view().child(dynamic(move || {
            if drawable.get() {
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
                        "Chọn khoảng thời gian dài hơn hoặc kết nối EA ở Nguồn dữ liệu.".to_owned(),
                    ),
                };
                empty_state(title, hint).into_element()
            }
        })),
    )
}

fn recent_card() -> impl IntoElement {
    let store = store();
    let rows = move || {
        let snapshot = store.snapshot();
        snapshot.rows.iter().take(RECENT).map(cells_of).collect()
    };
    let open = move |at: usize| {
        if let Some(row) = store.snapshot_now().rows.get(at) {
            nav::show_trade(row.key().clone());
        }
    };
    let empty = || ("Chưa có lệnh nào".to_owned(), "Lệnh đóng gần đây sẽ hiện ở đây.".to_owned());
    card("Lệnh gần đây", view().h(px(RECENT_HEIGHT)).child(recent_trades(rows, open, empty)))
}
