//! "Kế hoạch lúc vào vs thực tế": the plan the trader wrote when the position opened set
//! against what the trade did, row by row (entry, stop, target, reward to risk, risk), with
//! the gap and whether the plan was written before the result was known.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    Trade,
    journal::{self, EntryPlan},
};

use super::{
    chips::{Tone, chip, figure_text},
    controls::{Variant, action},
    parts::empty_state,
};
use crate::format::{self, Sign};

/// The plan of a trade with the trade it is measured against.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlanView {
    pub(crate) plan: EntryPlan,
    pub(crate) trade: Trade,
    /// The account's currency (`USD`), for the risk row.
    pub(crate) currency: String,
}

/// One line of the comparison, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanRow {
    pub(crate) label: &'static str,
    pub(crate) planned: String,
    pub(crate) actual: String,
    pub(crate) gap: String,
    /// How the gap reads: worse for the trader is a loss tone, better a profit tone.
    pub(crate) tone: Tone,
}

const NONE: &str = "—";

/// A planned price, or a dash when the plan did not say.
fn planned_price(value: Option<f64>) -> String {
    value.filter(|v| *v > 0.0).map_or_else(|| NONE.to_owned(), format::price)
}

/// A price gap with its sign: `+0.20`, `−0.10`.
fn price_gap(gap: f64) -> String {
    let text = format::price(gap.abs());
    if gap > f64::EPSILON {
        format!("+{text}")
    } else if gap < -f64::EPSILON {
        format!("\u{2212}{text}")
    } else {
        "0".to_owned()
    }
}

/// The rows of the comparison of `plan` with `trade`.
pub(crate) fn plan_rows(plan: &EntryPlan, trade: &Trade, currency: &str) -> Vec<PlanRow> {
    let review = journal::review(plan, trade);
    let row = |label, planned: String, actual: String, gap: String, tone| PlanRow { label, planned, actual, gap, tone };

    let (entry_gap, entry_tone) = match review.entry_slippage {
        Some(slip) if slip.abs() > f64::EPSILON => {
            (format!("{} (trượt giá)", price_gap(slip)), if slip > 0.0 { Tone::Loss } else { Tone::Profit })
        }
        Some(_) => ("đúng giá".to_owned(), Tone::Neutral),
        None => (NONE.to_owned(), Tone::Neutral),
    };
    let (stop_gap, stop_tone) = match (plan.stop_loss.filter(|v| *v > 0.0), review.stop_moved, review.stop_widened) {
        (Some(_), true, true) => ("dời xa, tăng rủi ro".to_owned(), Tone::Loss),
        (Some(_), true, false) => ("dời gần, giảm rủi ro".to_owned(), Tone::Neutral),
        (Some(_), false, _) if trade.sl > 0.0 => ("đúng kế hoạch".to_owned(), Tone::Profit),
        _ => (NONE.to_owned(), Tone::Neutral),
    };
    let target_gap = match (plan.take_profit.filter(|v| *v > 0.0), trade.tp > 0.0) {
        (Some(planned), true) if (trade.tp - planned).abs() > f64::EPSILON => price_gap(trade.tp - planned),
        (Some(_), true) => "đúng kế hoạch".to_owned(),
        _ => NONE.to_owned(),
    };
    let planned_rr = review.planned_reward_risk.map_or_else(|| NONE.to_owned(), |rr| format!("1 : {rr:.1}"));
    let risk_gap = match (plan.risk, trade.risk) {
        (Some(planned), Some(actual)) if planned > 0.0 => {
            let over = actual - planned;
            (price_gap(over), if over > f64::EPSILON { Tone::Loss } else { Tone::Neutral })
        }
        _ => (NONE.to_owned(), Tone::Neutral),
    };
    vec![
        row("Giá vào", planned_price(plan.entry), format::price(trade.open_price), entry_gap, entry_tone),
        row("Stop loss", planned_price(plan.stop_loss), format::level(trade.sl), stop_gap, stop_tone),
        row("Take profit", planned_price(plan.take_profit), format::level(trade.tp), target_gap, Tone::Neutral),
        row("R:R", planned_rr, format::r_text(review.realised_r), NONE.to_owned(), Tone::Neutral),
        row(
            "Risk",
            plan.risk.map_or_else(|| NONE.to_owned(), |r| format::money(r, Sign::OnlyNegative, currency)),
            trade.risk.map_or_else(|| NONE.to_owned(), |r| format::money(r, Sign::OnlyNegative, currency)),
            risk_gap.0,
            risk_gap.1,
        ),
    ]
}

/// Whether the plan was written before the trade closed, as the chip says it.
pub(crate) fn written_before_close(plan: &EntryPlan, trade: &Trade) -> bool {
    journal::review(plan, trade).written_before_close
}

/// Column widths of the table, in points.
const LABEL_WIDTH: f32 = 96.0;
const CELL_WIDTH: f32 = 120.0;

/// The comparison card. `plan` is read where it is bound: `None` shows the empty state with
/// the "write the plan" action, which calls `on_write`.
pub(crate) fn plan_vs_actual(
    plan: impl Fn() -> Option<PlanView> + 'static,
    on_write: impl Fn() + Clone + 'static,
) -> impl IntoElement {
    view()
        .p_5()
        .gap_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(dynamic(move || {
            let shown = plan();
            let on_write = on_write.clone();
            untracked(|| match shown {
                Some(shown) => filled(&shown, on_write).into_element(),
                None => blank(on_write).into_element(),
            })
        }))
}

fn title() -> impl IntoElement {
    text("Kế hoạch lúc vào vs thực tế")
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().card_foreground)
}

fn blank(on_write: impl Fn() + 'static) -> impl IntoElement {
    view()
        .gap_2()
        .child(title())
        .child(empty_state("Chưa có kế hoạch cho lệnh này", "Ghi lý do vào lệnh, SL, TP để so với kết quả."))
        .child(view().flex_row().justify_center().child(action("Viết kế hoạch", Variant::Secondary, on_write)))
}

fn filled(shown: &PlanView, on_write: impl Fn() + 'static) -> impl IntoElement {
    let before = written_before_close(&shown.plan, &shown.trade);
    let rows = plan_rows(&shown.plan, &shown.trade, &shown.currency);
    let thesis = shown.plan.thesis.trim().to_owned();
    let timeframe = shown.plan.timeframe.trim().to_owned();
    view()
        .gap_3()
        .child(
            view().flex_row().flex_wrap().items_center().justify_between().gap_2().child(title()).child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(chip(
                        if before { "Viết trước khi đóng" } else { "Viết sau khi đóng" },
                        if before { Tone::Primary } else { Tone::Neutral },
                    ))
                    .child(action("Sửa kế hoạch", Variant::Ghost, on_write)),
            ),
        )
        .when(!thesis.is_empty(), |v| {
            v.child(
                view()
                    .p_3()
                    .gap_1()
                    .rounded(move || theme().radius)
                    .border_1()
                    .border_color(move || theme().border)
                    .bg(move || theme().background)
                    .child(
                        text(if timeframe.is_empty() {
                            "Lý do vào".to_owned()
                        } else {
                            format!("Lý do vào · {}", timeframe.to_uppercase())
                        })
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                    )
                    .child(text(thesis).text_sm()),
            )
        })
        .child(header_row())
        .children(rows.into_iter().map(table_row))
}

fn header_row() -> impl IntoElement {
    let head = |label: &'static str| {
        text(label).w(px(CELL_WIDTH)).text_xs().text_color(move || theme().muted_foreground).text_right()
    };
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .child(view().w(px(LABEL_WIDTH)))
        .child(head("Kế hoạch"))
        .child(head("Thực tế"))
        .child(text("Chênh").flex_1().min_w(px(0.0)).text_xs().text_color(move || theme().muted_foreground))
}

fn table_row(row: PlanRow) -> impl IntoElement {
    let PlanRow { label, planned, actual, gap, tone } = row;
    let ink = move || match tone {
        Tone::Profit => super::theme::profit(),
        Tone::Loss => super::theme::loss(),
        Tone::Neutral | Tone::Primary => theme().muted_foreground,
    };
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .py_1()
        .border_t_1()
        .border_color(move || theme().border)
        .child(text(label).w(px(LABEL_WIDTH)).text_sm().text_color(move || theme().muted_foreground))
        .child(figure_text(move || planned.clone()).w(px(CELL_WIDTH)).text_sm().text_right())
        .child(figure_text(move || actual.clone()).w(px(CELL_WIDTH)).text_sm().text_right())
        .child(text(gap).flex_1().min_w(px(0.0)).text_sm().text_color(ink))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradelens_core::{Side, TradeKey};

    fn trade(side: Side, open: f64, sl: f64, tp: f64) -> Trade {
        Trade {
            key: TradeKey { account_id: 1, position_id: 1, segment: 0 },
            symbol: "XAUUSD".into(),
            side,
            open_time: 1_000,
            close_time: Some(5_000),
            volume: 0.5,
            open_price: open,
            close_price: Some(2656.2),
            sl,
            tp,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit: 0.0,
            risk: Some(195.0),
            magic: 0,
            comment: String::new(),
            manual: false,
        }
    }

    fn plan() -> EntryPlan {
        EntryPlan {
            entry: Some(2648.0),
            stop_loss: Some(2644.0),
            take_profit: Some(2660.0),
            risk: Some(150.0),
            written_at: 900,
            ..EntryPlan::default()
        }
    }

    fn by_label<'a>(rows: &'a [PlanRow], label: &str) -> &'a PlanRow {
        rows.iter().find(|r| r.label == label).expect("the row exists")
    }

    #[test]
    fn slippage_and_a_widened_stop_read_as_worse_for_the_trader() {
        let rows = plan_rows(&plan(), &trade(Side::Long, 2648.2, 2640.0, 2660.0), "USD");
        let entry = by_label(&rows, "Giá vào");
        assert_eq!((entry.planned.as_str(), entry.actual.as_str()), ("2648.00", "2648.20"));
        assert_eq!(entry.gap, "+0.20 (trượt giá)");
        assert_eq!(entry.tone, Tone::Loss);
        let stop = by_label(&rows, "Stop loss");
        assert_eq!(stop.gap, "dời xa, tăng rủi ro");
        assert_eq!(stop.tone, Tone::Loss);
        assert_eq!(by_label(&rows, "Take profit").gap, "đúng kế hoạch");
        assert_eq!(by_label(&rows, "R:R").planned, "1 : 3.0");
        let risk = by_label(&rows, "Risk");
        assert_eq!((risk.planned.as_str(), risk.actual.as_str()), ("$150.00", "$195.00"));
        assert_eq!(risk.tone, Tone::Loss, "more risk than planned");
    }

    #[test]
    fn a_better_fill_and_the_planned_stop_read_as_good() {
        let rows = plan_rows(&plan(), &trade(Side::Long, 2647.9, 2644.0, 2660.0), "USD");
        assert_eq!(by_label(&rows, "Giá vào").tone, Tone::Profit);
        assert_eq!(by_label(&rows, "Giá vào").gap, "\u{2212}0.10 (trượt giá)");
        assert_eq!(by_label(&rows, "Stop loss").gap, "đúng kế hoạch");
    }

    #[test]
    fn what_the_plan_does_not_say_is_a_dash() {
        let rows = plan_rows(&EntryPlan::default(), &trade(Side::Short, 1.2684, 0.0, 0.0), "USD");
        for label in ["Giá vào", "Stop loss", "Take profit", "R:R", "Risk"] {
            assert_eq!(by_label(&rows, label).planned, "—", "{label}");
            assert_eq!(by_label(&rows, label).gap, "—", "{label}");
        }
    }

    #[test]
    fn a_plan_written_after_the_close_says_so() {
        let late = EntryPlan { written_at: 9_000, ..plan() };
        assert!(!written_before_close(&late, &trade(Side::Long, 2648.0, 2644.0, 2660.0)));
        assert!(written_before_close(&plan(), &trade(Side::Long, 2648.0, 2644.0, 2660.0)));
    }
}
