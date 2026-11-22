//! Nhập lệnh & nhật ký thủ công: a trade the EA never saw (another broker, an account that
//! does not run the EA) typed in with its journal. The trade becomes an entry and an exit deal
//! (`Db::add_manual_trade`) and the journal goes with it.
//! Design: `docs/design/screens/manual-entry.html`.

use chrono::NaiveDateTime;
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    AccountInfo, Db, DbError, JournalEntry, ManualTrade, Side, TradeKey,
    attachments::Attachment,
    time::{format_duration, parse_mt5_time},
};

use super::trade_detail::{JournalFields, journal_form};
use crate::{
    format::{self, Sign},
    journal_draft::Draft,
    nav,
    store::{AccountFilter, Store, store, writer},
    ui::{
        controls::{Choice, Variant, action, action_icon, action_when, segmented, select_value, text_field},
        forms::{NumberRange, number_field},
        gallery::gallery,
        icons::Icon,
        parts::{card, page_with},
        theme as tokens,
    },
};

/// The value of the account choice that stands for the app's own account of hand-entered trades.
const MANUAL_ACCOUNT: &str = "manual";

const SIDES: &[Side] = &[Side::Long, Side::Short];

/// The page: a form that starts blank again after "Lưu & nhập tiếp".
pub(crate) fn manual_entry() -> impl IntoElement {
    let round = signal(0_u32);
    let notice = signal(None::<String>);
    dynamic(move || {
        let _ = round.get();
        untracked(|| entry_page(round, notice))
    })
}

/// What the trader typed about the trade itself, as plain data.
#[derive(Debug, Clone, PartialEq)]
struct TradeInput {
    symbol: String,
    side: Side,
    opened: String,
    closed: String,
    volume: Option<f64>,
    open_price: Option<f64>,
    close_price: Option<f64>,
    sl: Option<f64>,
    tp: Option<f64>,
    profit: Option<f64>,
    commission: Option<f64>,
    swap: Option<f64>,
}

/// The wall-clock formats a time may be typed in.
const TIME_FORMATS: [&str; 4] = ["%Y-%m-%d %H:%M", "%d/%m/%Y %H:%M", "%Y-%m-%d %H:%M:%S", "%d/%m/%Y %H:%M:%S"];

/// A time typed on the trader's clock as the milliseconds the journal keeps (the server's
/// clock), `None` when it is not a date and time.
fn parse_typed_time(typed: &str, server_offset_min: Option<i32>, local_offset_min: i32) -> Option<i64> {
    let typed = typed.trim();
    let local = TIME_FORMATS
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(typed, format).ok())
        .map(tradelens_core::time::to_ms)
        .or_else(|| parse_mt5_time(typed))?;
    // The typed time is local; the journal's are the server's. Without a known server offset
    // they are kept as typed, as an MT5 report without a zone is.
    Some(match server_offset_min {
        Some(server) => local - i64::from(local_offset_min - server) * 60_000,
        None => local,
    })
}

/// The trade to store from what was typed, or what to fix, in Vietnamese.
fn build_trade(
    input: &TradeInput,
    server_offset_min: Option<i32>,
    local_offset_min: i32,
) -> Result<ManualTrade, String> {
    let positive = |value: Option<f64>, what: &str| match value {
        Some(v) if v.is_finite() && v > 0.0 => Ok(v),
        _ => Err(format!("{what} phải là số lớn hơn 0")),
    };
    if input.symbol.trim().is_empty() {
        return Err("Nhập mã giao dịch (symbol).".to_owned());
    }
    let volume = positive(input.volume, "Khối lượng")?;
    let open_price = positive(input.open_price, "Giá vào")?;
    let close_price = positive(input.close_price, "Giá ra")?;
    let time = |typed: &str, what: &str| {
        parse_typed_time(typed, server_offset_min, local_offset_min)
            .ok_or_else(|| format!("{what} nhập dạng 2026-11-20 16:05."))
    };
    let open_time = time(&input.opened, "Thời gian vào")?;
    let close_time = time(&input.closed, "Thời gian ra")?;
    if close_time < open_time {
        return Err("Thời gian ra phải sau thời gian vào.".to_owned());
    }
    let profit = input.profit.ok_or_else(|| "Nhập lãi/lỗ của lệnh (chưa gồm commission và swap).".to_owned())?;
    Ok(ManualTrade {
        symbol: input.symbol.trim().to_owned(),
        side: input.side,
        volume,
        open_time,
        open_price,
        close_time,
        close_price,
        sl: input.sl.unwrap_or(0.0),
        tp: input.tp.unwrap_or(0.0),
        profit,
        commission: input.commission.unwrap_or(0.0),
        swap: input.swap.unwrap_or(0.0),
    })
}

/// The trade's result after costs and how long it was held, as the "tự tính" line says them.
fn computed_line(trade: &ManualTrade, currency: &str) -> String {
    let net = trade.profit + trade.commission + trade.swap;
    let held = (trade.close_time - trade.open_time) / 1000;
    format!("Tự tính: kết quả ròng {} · giữ {}", format::money(net, Sign::Always, currency), format_duration(held))
}

/// The signals of the trade's own fields.
#[derive(Clone, Copy)]
struct Inputs {
    account: Signal<String>,
    symbol: Signal<String>,
    side: Signal<Side>,
    opened: Signal<String>,
    closed: Signal<String>,
    volume: Signal<Option<f64>>,
    open_price: Signal<Option<f64>>,
    close_price: Signal<Option<f64>>,
    sl: Signal<Option<f64>>,
    tp: Signal<Option<f64>>,
    profit: Signal<Option<f64>>,
    commission: Signal<Option<f64>>,
    swap: Signal<Option<f64>>,
}

impl Inputs {
    fn new(account: String) -> Self {
        Self {
            account: signal(account),
            symbol: signal(String::new()),
            side: signal(Side::Long),
            opened: signal(String::new()),
            closed: signal(String::new()),
            volume: signal(None),
            open_price: signal(None),
            close_price: signal(None),
            sl: signal(None),
            tp: signal(None),
            profit: signal(None),
            commission: signal(None),
            swap: signal(None),
        }
    }

    fn input(&self) -> TradeInput {
        TradeInput {
            symbol: self.symbol.get_untracked(),
            side: self.side.get_untracked(),
            opened: self.opened.get_untracked(),
            closed: self.closed.get_untracked(),
            volume: self.volume.get_untracked(),
            open_price: self.open_price.get_untracked(),
            close_price: self.close_price.get_untracked(),
            sl: self.sl.get_untracked(),
            tp: self.tp.get_untracked(),
            profit: self.profit.get_untracked(),
            commission: self.commission.get_untracked(),
            swap: self.swap.get_untracked(),
        }
    }
}

/// How the saved form goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// Open the trade that was just added.
    Open,
    /// Start a blank form for the next one.
    Continue,
}

/// Stores the trade and its journal; creates the account of hand-entered trades when asked.
fn store_trade(db: &mut Db, account: &str, trade: &ManualTrade, entry: &JournalEntry) -> Result<TradeKey, DbError> {
    let account_id = match account.parse::<i64>() {
        Ok(id) => id,
        Err(_) => db.upsert_account(&AccountInfo {
            login: MANUAL_ACCOUNT.to_owned(),
            server: "Nhập tay".to_owned(),
            name: "Nhập tay".to_owned(),
            ..AccountInfo::default()
        })?,
    };
    let key = db.add_manual_trade(account_id, trade)?;
    if !entry.is_empty() {
        db.save_journal(&key, entry)?;
    }
    Ok(key)
}

fn entry_page(round: Signal<u32>, notice: Signal<Option<String>>) -> impl IntoElement {
    let store = store();
    let write = writer();
    let default_account = match store.accounts.get_untracked() {
        AccountFilter::One(id) => id.to_string(),
        AccountFilter::All => {
            store.snapshot_now().accounts.first().map_or_else(|| MANUAL_ACCOUNT.to_owned(), |a| a.id.to_string())
        }
    };
    let inputs = Inputs::new(default_account);
    let fields = JournalFields::of(Draft::default());
    let shots = signal(Vec::<Attachment>::new());
    let busy = signal(false);
    let failure = signal(None::<String>);
    let shot_count = computed(move || shots.with(Vec::len));
    let save = move |after: After| {
        if busy.get_untracked() {
            return;
        }
        let snapshot = store.snapshot_now();
        let account = inputs.account.get_untracked();
        let account_row = account.parse::<i64>().ok().and_then(|id| snapshot.accounts.iter().find(|a| a.id == id));
        let offset = account_row.and_then(|a| a.server_offset_min);
        let local_offset = chrono::Local::now().offset().local_minus_utc() / 60;
        let trade = match build_trade(&inputs.input(), offset, local_offset) {
            Ok(trade) => trade,
            Err(message) => return failure.set(Some(message)),
        };
        let mut entry = JournalEntry::default();
        fields.draft().apply(&mut entry);
        entry.screenshots = shots.get_untracked();
        let symbol = trade.symbol.clone();
        busy.set(true);
        failure.set(None);
        write.run(
            move |db| store_trade(db, &account, &trade, &entry),
            move |result| {
                busy.set(false);
                match result {
                    Ok(key) => match after {
                        After::Open => {
                            nav::close_manual_entry();
                            nav::show_trade(key);
                        }
                        After::Continue => {
                            notice.set(Some(format!("Đã lưu lệnh {symbol}. Nhập lệnh tiếp theo.")));
                            round.update(|n| *n += 1);
                        }
                    },
                    Err(error) => failure.set(Some(error)),
                }
            },
        );
    };
    on_key_with("mod+enter", KeyOptions { pass_through: true, ..KeyOptions::default() }, move |_| save(After::Open));
    let actions = view()
        .flex_row()
        .items_center()
        .gap_3()
        .child(action("Huỷ", Variant::Ghost, nav::close_manual_entry))
        .child(action_when("Lưu & nhập tiếp", Variant::Secondary, move || !busy.get(), move || save(After::Continue)))
        .child(view().child(action_icon(Icon::Check, "Lưu lệnh", Variant::Primary, move || save(After::Open))));
    page_with(
        "Nhập lệnh & nhật ký thủ công",
        "Cho lệnh ngoài MT5, tài khoản không chạy EA, hoặc bổ sung nhật ký cho lệnh có sẵn",
        actions,
        view()
            .flex_row()
            .flex_wrap()
            .items_start()
            .gap_6()
            .child(
                view()
                    .flex_1()
                    .min_w(px(520.0))
                    .gap_6()
                    .child(dynamic(move || notice.get().map(|line| text(line).text_sm().text_color(tokens::warning()))))
                    .child(trade_card(inputs))
                    .child(card("2 · Nhật ký", journal_form(fields, shot_count, None, failure))),
            )
            .child(
                view()
                    .w(px(380.0))
                    .gap_6()
                    .child(card(
                        "Ảnh & đính kèm",
                        view()
                            .gap_2()
                            .child(gallery("Ảnh chart", move || shots.get(), move |list| shots.set(list)))
                            .child(
                                text("Ảnh lưu cùng dữ liệu trên máy, theo lệnh sau khi bấm Lưu lệnh.")
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                            ),
                    ))
                    .child(dynamic(move || {
                        failure.get().map(|error| text(error).text_sm().text_color(tokens::loss()))
                    })),
            ),
    )
}

/// A caption over a control, in the form's grid.
fn field(label: &'static str, control: impl IntoElement) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(150.0))
        .gap_1()
        .child(text(label).text_xs().text_color(move || theme().muted_foreground))
        .child(control)
}

/// A number the trader types, empty until they do.
fn number(label: &'static str, unit: Option<&'static str>, value: Signal<Option<f64>>, step: f64) -> impl IntoElement {
    field(
        label,
        number_field(label, unit, NumberRange::any().stepping(step), move || value.get(), move |n| value.set(n)),
    )
}

/// The "tự tính" line: the net result and how long the trade was held, once the form has the
/// times and the profit to say so.
fn preview_line(store: Store, inputs: Inputs) -> String {
    let snapshot = store.snapshot();
    let account = inputs.account.get().parse::<i64>().ok().and_then(|id| snapshot.accounts.iter().find(|a| a.id == id));
    let local_offset = chrono::Local::now().offset().local_minus_utc() / 60;
    // Only the times, the profit and the costs matter here: the rest stands in for a valid trade.
    let stand_in = TradeInput {
        profit: inputs.profit.get().or(Some(0.0)),
        symbol: "x".to_owned(),
        volume: Some(1.0),
        open_price: Some(1.0),
        close_price: Some(1.0),
        opened: inputs.opened.get(),
        closed: inputs.closed.get(),
        commission: inputs.commission.get(),
        swap: inputs.swap.get(),
        side: Side::Long,
        sl: None,
        tp: None,
    };
    match build_trade(&stand_in, account.and_then(|a| a.server_offset_min), local_offset) {
        Ok(trade) => computed_line(&trade, account.map_or("", |a| a.currency.as_str())),
        Err(_) => "Tự tính: nhập thời gian vào, ra và lãi/lỗ để xem kết quả ròng và thời gian giữ.".to_owned(),
    }
}

/// The account, the symbol and the side.
fn who_row(inputs: Inputs) -> impl IntoElement {
    let store = store();
    let accounts = move || {
        let mut all: Vec<Choice> =
            store.snapshot().accounts.iter().map(|a| Choice::new(a.id.to_string(), a.title())).collect();
        all.push(Choice::new(MANUAL_ACCOUNT, "Tài khoản nhập tay"));
        all
    };
    let (symbol, side) = (inputs.symbol, inputs.side);
    view()
        .flex_row()
        .flex_wrap()
        .items_end()
        .gap_3()
        .child(field(
            "Tài khoản",
            select_value(
                "Tài khoản",
                accounts,
                move || SharedString::from(inputs.account.get()),
                move |value| inputs.account.set(value.to_string()),
            ),
        ))
        .child(field(
            "Symbol",
            text_field("Symbol", None, "XAUUSD", move || symbol.get(), move |t| symbol.set(t.to_owned())),
        ))
        .child(field("Chiều", segmented("Chiều", SIDES, Side::label, move || side.get(), move |s| side.set(s))))
}

/// When the trade opened and closed, and how big it was.
fn when_row(inputs: Inputs) -> impl IntoElement {
    let (opened, closed) = (inputs.opened, inputs.closed);
    view()
        .flex_row()
        .flex_wrap()
        .gap_3()
        .child(field(
            "Thời gian vào",
            text_field(
                "Thời gian vào",
                None,
                "2026-11-20 16:05",
                move || opened.get(),
                move |t| opened.set(t.to_owned()),
            ),
        ))
        .child(field(
            "Thời gian ra",
            text_field(
                "Thời gian ra",
                None,
                "2026-11-20 17:40",
                move || closed.get(),
                move |t| closed.set(t.to_owned()),
            ),
        ))
        .child(number("Khối lượng", Some("lot"), inputs.volume, 0.01))
}

fn trade_card(inputs: Inputs) -> impl IntoElement {
    let store = store();
    card(
        "1 · Thông tin lệnh",
        view()
            .gap_3()
            .child(who_row(inputs))
            .child(when_row(inputs))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_3()
                    .child(number("Giá vào", None, inputs.open_price, 0.01))
                    .child(number("Giá ra", None, inputs.close_price, 0.01))
                    .child(number("Stop loss", None, inputs.sl, 0.01))
                    .child(number("Take profit", None, inputs.tp, 0.01)),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_3()
                    .child(number("Lãi/lỗ", None, inputs.profit, 1.0))
                    .child(number("Commission", None, inputs.commission, 0.1))
                    .child(number("Swap", None, inputs.swap, 0.1)),
            )
            .child(view().p_3().rounded(move || theme().radius).border_1().border_color(move || theme().border).child(
                text(move || preview_line(store, inputs)).text_xs().text_color(move || theme().muted_foreground),
            )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tradelens_core::Side;

    fn input() -> TradeInput {
        TradeInput {
            symbol: " GBPUSD ".into(),
            side: Side::Short,
            opened: "2026-11-20 16:05".into(),
            closed: "20/11/2026 17:40".into(),
            volume: Some(0.8),
            open_price: Some(1.2684),
            close_price: Some(1.2651),
            sl: Some(1.2705),
            tp: None,
            profit: Some(264.0),
            commission: Some(-5.6),
            swap: None,
        }
    }

    #[test]
    fn a_complete_form_becomes_a_trade() {
        let trade = build_trade(&input(), None, 420).expect("a valid trade");
        assert_eq!(trade.symbol, "GBPUSD");
        assert_eq!(trade.side, Side::Short);
        assert_eq!(trade.tp, 0.0, "a level left empty is 0, which means none");
        assert_eq!(trade.swap, 0.0);
        assert_eq!((trade.close_time - trade.open_time) / 60_000, 95);
        assert_eq!(computed_line(&trade, "USD"), "Tự tính: kết quả ròng +$258.40 · giữ 1g 35p");
    }

    #[test]
    fn typed_times_move_to_the_servers_clock() {
        // 16:05 in Vietnam (UTC+7) is 11:05 on a server at UTC+2.
        let server = parse_typed_time("2026-11-20 16:05", Some(120), 420).expect("parses");
        assert_eq!(tradelens_core::time::naive(server).format("%H:%M").to_string(), "11:05");
        let kept = parse_typed_time("2026-11-20 16:05", None, 420).expect("parses");
        assert_eq!(tradelens_core::time::naive(kept).format("%H:%M").to_string(), "16:05");
        assert_eq!(parse_typed_time("hôm qua", None, 420), None);
    }

    #[test]
    fn what_is_wrong_is_said_in_words() {
        let fix = |edit: fn(&mut TradeInput)| {
            let mut input = input();
            edit(&mut input);
            build_trade(&input, None, 420).expect_err("must be refused")
        };
        assert!(fix(|i| i.symbol = " ".into()).contains("symbol"));
        assert!(fix(|i| i.volume = None).starts_with("Khối lượng"));
        assert!(fix(|i| i.close_price = Some(0.0)).starts_with("Giá ra"));
        assert!(fix(|i| i.opened = "sáng nay".into()).starts_with("Thời gian vào"));
        assert!(fix(|i| i.closed = "2026-11-20 15:00".into()).contains("sau thời gian vào"));
        assert!(fix(|i| i.profit = None).starts_with("Nhập lãi/lỗ"));
    }

    #[test]
    fn a_trade_and_its_journal_are_stored_together_on_the_manual_account() {
        let mut db = Db::in_memory().expect("opens");
        let trade = build_trade(&input(), None, 420).expect("a valid trade");
        let entry = JournalEntry { notes: "vào sớm".into(), rating: 3, ..JournalEntry::default() };
        let key = store_trade(&mut db, MANUAL_ACCOUNT, &trade, &entry).expect("stored");
        assert_eq!(db.journal(&key).expect("reads").expect("an entry").notes, "vào sớm");
        let again = store_trade(&mut db, MANUAL_ACCOUNT, &trade, &JournalEntry::default()).expect("stored again");
        assert_eq!(again.account_id, key.account_id, "hand-entered trades share one account");
        assert_ne!(again.position_id, key.position_id);
        assert!(db.journal(&again).expect("reads").is_none(), "no entry is written for an empty journal");
    }
}
