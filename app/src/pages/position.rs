//! Quản lý lệnh: a modal over any screen for a position open on a connected MT5 account. It moves
//! the stop and the target, or closes the position, all of it or part. Closing asks first; what
//! the terminal answers shows under the buttons, and the trade's page follows at the next poll.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_mt5::{AccountRef, Levels, Position};

use crate::{
    format::{self, Sign},
    mt5::{Status, mt5},
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

/// What the trader typed in a level field: `None` while untouched (the field follows the position),
/// `Some(None)` for an emptied field (remove the level), `Some(Some(v))` for a new level.
type Typed = Option<Option<f64>>;

/// The position's level, `None` for 0 (no level).
fn level(value: f64) -> Option<f64> {
    (value > 0.0).then_some(value)
}

/// The levels to send: what the trader typed, and the position's current level for a field left
/// alone (a stop moved in MT5 meanwhile is not undone). Also the names of the levels it removes.
fn levels_to_send(sl: Typed, tp: Typed, current: &Position) -> (Levels, Vec<&'static str>) {
    let levels = Levels { sl: sl.unwrap_or(level(current.sl)), tp: tp.unwrap_or(level(current.tp)) };
    let mut removed = Vec::new();
    if levels.sl.is_none() && level(current.sl).is_some() {
        removed.push("stop loss");
    }
    if levels.tp.is_none() && level(current.tp).is_some() {
        removed.push("take profit");
    }
    (levels, removed)
}

fn panel(account: AccountRef, ticket: i64) -> impl IntoElement {
    let mt5 = mt5();
    let owner = root_owner();
    let wanted = account.clone();
    // The position, its account's currency, and whether the account is connected now: orders are
    // sent only then (an order made while it logs in again would be dropped).
    let position = computed(move || {
        mt5.state.with(|state| {
            state.live(&wanted).and_then(|live| {
                let currency = live.info.as_ref().map_or_default(|info| info.currency.clone());
                let connected = live.status == Status::Connected;
                live.positions.iter().find(|p| p.ticket == ticket).cloned().map(|p| (p, currency, connected))
            })
        })
    });
    let sl = signal(None::<Option<f64>>);
    let tp = signal(None::<Option<f64>>);
    let volume = signal(position.get_untracked().map(|(p, ..)| p.volume));
    let outcome = signal(None::<Outcome>);
    let current = move |pick: fn(&Position) -> f64| position.with(|p| p.as_ref().and_then(|(p, ..)| level(pick(p))));
    let idle = move || {
        position.with(|p| p.as_ref().is_some_and(|(_, _, connected)| *connected))
            && outcome.get() != Some(Outcome::Sending)
    };
    let done = move |what: &'static str| {
        move |result: Result<(), String>| {
            if result.is_ok() {
                batch(|| {
                    sl.set(None);
                    tp.set(None);
                });
            }
            outcome.set(Some(match result {
                Ok(()) => Outcome::Done(what.to_owned()),
                Err(error) => Outcome::Failed(error),
            }));
        }
    };
    let save_levels = {
        let account = account.clone();
        move || {
            let Some((open, ..)) = position.get_untracked() else { return };
            let (levels, removed) = levels_to_send(sl.get_untracked(), tp.get_untracked(), &open);
            let account = account.clone();
            let send = move || {
                outcome.set(Some(Outcome::Sending));
                mt5.modify(&account, ticket, levels, done("Đã đặt SL/TP."));
            };
            if removed.is_empty() {
                send();
            } else {
                let said = format!("Ô để trống sẽ bỏ {} của lệnh {ticket} trên MT5.", removed.join(" và "));
                confirm_then(owner, "Bỏ mức dừng của lệnh?", said, "Bỏ và lưu", send);
            }
        }
    };
    let close = move || {
        let account = account.clone();
        let Some((open, ..)) = position.get_untracked() else { return };
        let (description, what, lots) = close_plan(&open, volume.get_untracked());
        confirm_then(owner, "Đóng lệnh trên MT5?", description, "Đóng lệnh", move || {
            outcome.set(Some(Outcome::Sending));
            mt5.close(&account, ticket, lots, done(what));
        });
    };
    view()
        .gap_4()
        .child(dynamic(move || position.get().map(|(p, currency, _)| untracked(move || summary(&p, &currency)))))
        .child(connection_note(position))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap_3()
                .child(level_field("Stop loss", sl, move || current(|p| p.sl)))
                .child(level_field("Take profit", tp, move || current(|p| p.tp)))
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
                        Some("lot".into()),
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

/// Why the dialog cannot send orders now, if it cannot.
fn connection_note(position: Memo<Option<(Position, String, bool)>>) -> impl IntoElement {
    dynamic(move || {
        let said = match position.get() {
            None => Some("Lệnh này không còn mở trên tài khoản đang kết nối."),
            Some((_, _, false)) => Some("Tài khoản đang kết nối lại: chờ kết nối xong rồi gửi lệnh."),
            Some(_) => None,
        };
        said.map(|said| text(said).text_sm().text_color(move || theme().muted_foreground))
    })
}

/// A stop or a target: what the trader typed, else the position's level as it is now.
fn level_field(
    label: &'static str,
    typed: Signal<Typed>,
    current: impl Fn() -> Option<f64> + 'static,
) -> impl IntoElement {
    field(
        label,
        number_field(
            label,
            None,
            NumberRange::any(),
            move || typed.get().unwrap_or_else(&current),
            move |v| typed.set(Some(v)),
        ),
    )
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

    fn open_position() -> Position {
        Position {
            ticket: 7,
            time_msc: 0,
            symbol: "XAUUSD".into(),
            side: Direction::Buy,
            volume: 0.3,
            open_price: 0.0,
            current_price: 0.0,
            sl: 2640.0,
            tp: 2680.0,
            profit: 0.0,
            swap: 0.0,
            magic: 0,
            comment: String::new(),
        }
    }

    #[test]
    fn a_level_left_alone_is_sent_as_it_is_now_and_an_emptied_one_is_named_as_removed() {
        let mut now = open_position();
        now.sl = 2645.0; // moved in MT5 after the dialog opened
        let (levels, removed) = levels_to_send(None, Some(Some(2700.0)), &now);
        assert_eq!(levels, Levels { sl: Some(2645.0), tp: Some(2700.0) }, "only the target changes");
        assert_eq!(removed, Vec::<&str>::new(), "nothing is removed");
        let (levels, removed) = levels_to_send(Some(None), None, &now);
        assert_eq!(levels, Levels { sl: None, tp: Some(2680.0) });
        assert_eq!(removed, vec!["stop loss"]);
    }

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
