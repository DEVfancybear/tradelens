//! Quản lý lệnh: a modal over any screen for a position open on a connected MT5 account. It moves
//! the stop and the target, or closes the position, all of it or part. Closing asks first; what
//! the terminal answers shows under the buttons, and the trade's page follows at the next poll.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_mt5::{AccountRef, Levels, Position};

use crate::{
    format::{self, Sign},
    mt5::mt5,
    ui::{
        chips::pnl_text,
        controls::{Variant, action_when, confirm_then, root_owner},
        forms::{NumberRange, number_field},
        modal::{self, Modal},
        parts::{error_line, save_status},
    },
};

/// Width of the dialog's panel, in points.
const PANEL_WIDTH: f32 = 480.0;

/// The smallest lot a part close takes (brokers' usual step).
const LOT_STEP: f64 = 0.01;

fn modal_id(account: &AccountRef, ticket: i64) -> String {
    format!("position:{account}:{ticket}")
}

/// Opens the dialog for position `ticket` of `account`.
pub(crate) fn open(account: AccountRef, ticket: i64) {
    let id = modal_id(&account, ticket);
    let closing = id.clone();
    modal::open(
        Modal::new(
            id,
            format!("Quản lý lệnh {ticket}"),
            move || panel(account.clone(), ticket),
            move || modal::close(&closing),
        )
        .width(PANEL_WIDTH),
    );
}

/// What the last order did: its result for the line under the buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Sending,
    Done(String),
    Failed(String),
}

fn panel(account: AccountRef, ticket: i64) -> impl IntoElement {
    let mt5 = mt5();
    let owner = root_owner();
    let wanted = account.clone();
    let position = computed(move || {
        mt5.state.with(|state| {
            state.live(&wanted).and_then(|live| {
                let currency = live.info.as_ref().map_or_default(|info| info.currency.clone());
                live.positions.iter().find(|p| p.ticket == ticket).cloned().map(|p| (p, currency))
            })
        })
    });
    let start = position.get_untracked().map(|(p, _)| p);
    let level = |v: f64| (v > 0.0).then_some(v);
    let sl = signal(start.as_ref().and_then(|p| level(p.sl)));
    let tp = signal(start.as_ref().and_then(|p| level(p.tp)));
    let volume = signal(start.as_ref().map(|p| p.volume));
    let outcome = signal(None::<Outcome>);
    let still_open = move || position.get().is_some();
    let idle = move || still_open() && outcome.get() != Some(Outcome::Sending);
    let done = move |what: &'static str| {
        move |result: Result<(), String>| {
            outcome.set(Some(match result {
                Ok(()) => Outcome::Done(what.to_owned()),
                Err(error) => Outcome::Failed(error),
            }));
        }
    };
    let save_levels = {
        let account = account.clone();
        move || {
            outcome.set(Some(Outcome::Sending));
            mt5.modify(
                &account,
                ticket,
                Levels { sl: sl.get_untracked(), tp: tp.get_untracked() },
                done("Đã đặt SL/TP."),
            );
        }
    };
    let close = move || {
        let account = account.clone();
        let Some((open, _)) = position.get_untracked() else { return };
        let (description, what, lots) = close_plan(&open, volume.get_untracked());
        confirm_then(owner, "Đóng lệnh trên MT5?", description, "Đóng lệnh", move || {
            outcome.set(Some(Outcome::Sending));
            mt5.close(&account, ticket, lots, done(what));
        });
    };
    view()
        .gap_4()
        .child(dynamic(move || position.get().map(|(p, currency)| untracked(move || summary(&p, &currency)))))
        .child(dynamic(move || {
            (!still_open()).then(|| {
                text("Lệnh này không còn mở trên tài khoản đang kết nối.")
                    .text_sm()
                    .text_color(move || theme().muted_foreground)
            })
        }))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap_3()
                .child(field(
                    "Stop loss",
                    number_field("Stop loss", None, NumberRange::any(), move || sl.get(), move |v| sl.set(v)),
                ))
                .child(field(
                    "Take profit",
                    number_field("Take profit", None, NumberRange::any(), move || tp.get(), move |v| tp.set(v)),
                ))
                .child(action_when("Lưu SL/TP", Variant::Secondary, idle, save_levels)),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap_3()
                .child(field(
                    "Khối lượng đóng",
                    number_field(
                        "Khối lượng đóng",
                        Some("lot"),
                        NumberRange::between(LOT_STEP, 1_000.0).stepping(LOT_STEP),
                        move || volume.get(),
                        move |v| volume.set(v),
                    ),
                ))
                .child(action_when("Đóng lệnh", Variant::Destructive, idle, close)),
        )
        .child(dynamic(move || outcome.get().map(outcome_view)))
}

/// What closing `wanted` lots of `open` asks, says once done, and sends (`None`: all of it). No lots,
/// or as many as are open, close it all.
fn close_plan(open: &Position, wanted: Option<f64>) -> (String, &'static str, Option<f64>) {
    let lots = wanted.filter(|lots| *lots > 0.0).unwrap_or(open.volume);
    if lots >= open.volume - 1e-9 {
        let asked =
            format!("Đóng toàn bộ {} lot {} theo giá thị trường trên MT5.", format::lots(open.volume), open.symbol);
        return (asked, "Đã đóng lệnh.", None);
    }
    let asked = format!(
        "Đóng {} trên {} lot {} theo giá thị trường trên MT5.",
        format::lots(lots),
        format::lots(open.volume),
        open.symbol
    );
    (asked, "Đã đóng một phần lệnh.", Some(lots))
}

/// The line under the buttons.
fn outcome_view(outcome: Outcome) -> AnyElement {
    match outcome {
        Outcome::Sending => save_status("Đang gửi lệnh tới MT5…").text_sm().into_element(),
        Outcome::Done(what) => save_status(what).text_sm().into_element(),
        Outcome::Failed(error) => error_line(error).into_element(),
    }
}

/// A field with its label above it.
fn field(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    view()
        .gap_1()
        .child(text(label).text_xs().font_weight(FontWeight::MEDIUM).text_color(move || theme().muted_foreground))
        .child(control)
}

fn summary(p: &Position, currency: &str) -> impl IntoElement + use<> {
    let profit = format::money(p.profit, Sign::Always, currency);
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .p_3()
        .rounded(move || theme().radius)
        .bg(move || theme().muted)
        .child(
            view()
                .gap_1()
                .child(
                    text(format!("{} · {} {} lot", p.symbol, p.side.side().label(), format::lots(p.volume)))
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(
                    text(format!(
                        "Giá vào {} · giá hiện tại {}",
                        format::price(p.open_price),
                        format::price(p.current_price)
                    ))
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                ),
        )
        .child(pnl_text(move || profit.clone()).text_sm())
}

#[cfg(test)]
mod tests {
    use tradelens_mt5::Direction;

    use super::*;

    #[test]
    fn closing_asks_for_all_of_it_unless_fewer_lots_are_given() {
        let open = Position {
            ticket: 7,
            time_msc: 0,
            symbol: "XAUUSD".into(),
            side: Direction::Buy,
            volume: 0.3,
            open_price: 0.0,
            current_price: 0.0,
            sl: 0.0,
            tp: 0.0,
            profit: 0.0,
            swap: 0.0,
            magic: 0,
            comment: String::new(),
        };
        assert_eq!(close_plan(&open, None).2, None, "no lots: all of it");
        assert_eq!(close_plan(&open, Some(0.3)).2, None, "as many as are open: all of it");
        let (asked, done, lots) = close_plan(&open, Some(0.1));
        assert_eq!(lots, Some(0.1));
        assert_eq!(done, "Đã đóng một phần lệnh.");
        assert!(asked.contains("0.10") && asked.contains("0.30"), "{asked}");
    }
}
