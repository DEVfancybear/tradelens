//! Nhập lệnh & nhật ký thủ công (manual trade and journal entry): a trade the terminal never saw (another broker, a journal kept by
//! hand) typed in with its journal. The trade becomes an entry and an exit deal
//! (`Db::add_manual_trade`) and the journal goes with it. The same form edits the facts of a trade
//! the trader owns (`Db::update_manual_trade`): pre-filled, saved over the same trade.
//! Design: `docs/design/screens/manual-entry.html`.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    Account, AccountInfo, Db, DbError, JournalEntry, ManualTrade, Side, TradeKey,
    attachments::Attachment,
    time::{Clock, format_duration},
};

use super::trade_detail::{JournalFields, journal_form};
use crate::{
    format::{self, Sign},
    journal_draft::Draft,
    nav,
    snapshot::Snapshot,
    store::{AccountFilter, Store, store, writer},
    ui::{
        controls::{Choice, Variant, action, action_icon, action_when, link, segmented, select_value, text_field},
        date_picker::{Moment, MomentValue, moment_field},
        forms::{NumberRange, number_field_fluid},
        gallery::gallery,
        icons::Icon,
        parts::{card, error_line, error_note, page_with, warning_line},
    },
};

/// The value of the account choice that stands for the app's own account of hand-entered trades.
const MANUAL_ACCOUNT: &str = "manual";

const SIDES: &[Side] = &[Side::Long, Side::Short];

/// What the form asks first, and what it insists on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lead {
    /// Money: the lot, the prices and the profit are needed; the result in R is optional.
    Money,
    /// R: a journal kept in R (no money), or a trade of which little is known. The result in R leads
    /// and the lot, the prices and the money are optional; the R can come from the prices and the stop.
    R,
}

/// The page: a form that starts blank again after "Lưu & nhập tiếp" (Save & add another).
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
    opened: MomentValue,
    closed: MomentValue,
    volume: Option<f64>,
    open_price: Option<f64>,
    close_price: Option<f64>,
    sl: Option<f64>,
    tp: Option<f64>,
    profit: Option<f64>,
    commission: Option<f64>,
    swap: Option<f64>,
    result_r: Option<f64>,
}

/// How the time typed on the trader's clock becomes the journal's (the server's).
#[derive(Debug, Clone, Copy)]
struct Zone {
    clock: Clock,
    lead: Lead,
    /// A trade whose time of day was never known may stay so.
    date_only_ok: bool,
}

/// The clock times are typed on: the one the trader chose to see times on (their own zone, or
/// the server's), so a time typed is a time read back the same.
fn typed_clock(snapshot: &Snapshot, account: Option<&Account>) -> Clock {
    let zone = snapshot.zone;
    zone.display.clock(account.and_then(|a| a.server_offset_min), zone.local_offset_min)
}

/// The parts of the trade form that can be told wrong, in the order the form shows them (a trade kept
/// in R shows its result before the rest: see [`Part::rank`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Symbol,
    Opened,
    Closed,
    Volume,
    OpenPrice,
    ClosePrice,
    Profit,
    ResultR,
}

impl Part {
    const COUNT: usize = 8;

    /// Which of the form's element refs it has.
    fn index(self) -> usize {
        match self {
            Part::Symbol => 0,
            Part::Opened => 1,
            Part::Closed => 2,
            Part::Volume => 3,
            Part::OpenPrice => 4,
            Part::ClosePrice => 5,
            Part::Profit => 6,
            Part::ResultR => 7,
        }
    }

    /// Where the form shows it, first to last.
    fn rank(self, lead: Lead) -> usize {
        match (self, lead) {
            (Part::ResultR, Lead::R) => 0,
            (part, _) => 1 + part.index(),
        }
    }
}

/// What is wrong with a field of the form, in words: it is said under that field.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Problem {
    part: Part,
    message: String,
}

impl Problem {
    fn new(part: Part, message: impl Into<String>) -> Self {
        Self { part, message: message.into() }
    }
}

/// A number that must be above zero when the form asks for money (`lead`) or when it is typed: it is
/// noted in `problems` when it is not, and `None` is what an empty one that may be gives.
fn positive(problems: &mut Vec<Problem>, lead: Lead, value: Option<f64>, part: Part, what: &str) -> Option<f64> {
    match value {
        Some(v) if v.is_finite() && v > 0.0 => Some(v),
        Some(_) => {
            problems.push(Problem::new(part, t!("manual_entry.problem.positive", what = what)));
            None
        }
        None => {
            if lead == Lead::Money {
                problems.push(Problem::new(part, t!("manual_entry.problem.positive", what = what)));
            }
            None
        }
    }
}

/// The trade to store from what was typed, or everything to fix (each said under its field), first
/// the one the form shows first.
fn build_trade(input: &TradeInput, zone: Zone) -> Result<ManualTrade, Vec<Problem>> {
    let needed = zone.lead == Lead::Money;
    let mut problems = Vec::new();
    if input.symbol.trim().is_empty() {
        problems.push(Problem::new(Part::Symbol, t!("manual_entry.problem.symbol")));
    }
    let volume = positive(&mut problems, zone.lead, input.volume, Part::Volume, &t!("manual_entry.field.volume"));
    let open_price =
        positive(&mut problems, zone.lead, input.open_price, Part::OpenPrice, &t!("manual_entry.field.open_price"));
    let close_price =
        positive(&mut problems, zone.lead, input.close_price, Part::ClosePrice, &t!("manual_entry.field.close_price"));
    let mut day = |moment: MomentValue, part: Part, what: &str| {
        let day = moment.day;
        if day.is_none() {
            problems.push(Problem::new(part, t!("manual_entry.problem.no_day", what = what)));
        }
        day
    };
    let (open_day, close_day) = (
        day(input.opened, Part::Opened, &t!("manual_entry.field.opened")),
        day(input.closed, Part::Closed, &t!("manual_entry.field.closed")),
    );
    // Without a time of day on either end the trade is kept by its days; one typed makes both known.
    let date_only =
        (zone.lead == Lead::R || zone.date_only_ok) && input.opened.time.is_none() && input.closed.time.is_none();
    let time = |day: chrono::NaiveDate, time: Option<chrono::NaiveTime>| {
        zone.clock.server_ms(day.and_time(time.unwrap_or(chrono::NaiveTime::MIN)))
    };
    let times =
        open_day.zip(close_day).map(|(open, close)| (time(open, input.opened.time), time(close, input.closed.time)));
    if let Some((open, close)) = times
        && close < open
    {
        problems.push(Problem::new(Part::Closed, t!("manual_entry.problem.closed_before_opened")));
    }
    let mut finite = |value: Option<f64>, part: Part, what: &str| match value {
        Some(v) if !v.is_finite() => {
            problems.push(Problem::new(part, t!("manual_entry.problem.number", what = what)));
            None
        }
        other => other,
    };
    let profit = finite(input.profit, Part::Profit, &t!("manual_entry.field.profit"));
    let result_r = finite(input.result_r, Part::ResultR, &t!("manual_entry.what.result_r"));
    if needed && input.profit.is_none() {
        problems.push(Problem::new(Part::Profit, t!("manual_entry.problem.profit")));
    }
    let (Some((open_time, close_time)), true) = (times, problems.is_empty()) else {
        problems.sort_by_key(|problem| problem.part.rank(zone.lead));
        return Err(problems);
    };
    let mut trade = ManualTrade {
        symbol: input.symbol.trim().to_owned(),
        side: input.side,
        volume,
        open_time,
        open_price,
        close_time,
        close_price,
        date_only,
        sl: input.sl.unwrap_or(0.0),
        tp: input.tp.unwrap_or(0.0),
        profit,
        commission: input.commission.unwrap_or(0.0),
        swap: input.swap.unwrap_or(0.0),
        result_r,
    };
    if trade.profit.is_none() && trade.result_r.is_none() {
        // A trade without money is kept by its R: the one the prices and the stop say.
        trade.result_r = Some(
            trade.computed_r().ok_or_else(|| vec![Problem::new(Part::ResultR, t!("manual_entry.problem.result_r"))])?,
        );
    }
    Ok(trade)
}

/// What the "tự tính" line says of the form so far: the result after costs, how long the trade was
/// held and its R (the one typed, else the one the prices and the stop say), as far as the form
/// has what they need.
fn computed_line(input: &TradeInput, zone: Zone, currency: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(profit) = input.profit {
        let net = profit + input.commission.unwrap_or(0.0) + input.swap.unwrap_or(0.0);
        parts.push(t!("manual_entry.line.net", amount = format::money(net, Sign::Always, currency)));
    }
    // As `build_trade` reads them: a day with no time is midnight, unless the trade is kept by its days.
    let by_days =
        (zone.lead == Lead::R || zone.date_only_ok) && input.opened.time.is_none() && input.closed.time.is_none();
    let known = |moment: MomentValue| {
        moment
            .day
            .filter(|_| !by_days)
            .map(|day| zone.clock.server_ms(day.and_time(moment.time.unwrap_or(chrono::NaiveTime::MIN))))
    };
    if let (Some(open), Some(close)) = (known(input.opened), known(input.closed))
        && close >= open
    {
        parts.push(t!("manual_entry.line.held", duration = format_duration((close - open) / 1000)));
    }
    let measured = ManualTrade {
        symbol: String::new(),
        side: input.side,
        volume: None,
        open_time: 0,
        open_price: input.open_price.filter(|p| *p > 0.0),
        close_time: 0,
        close_price: input.close_price.filter(|p| *p > 0.0),
        date_only: false,
        sl: input.sl.unwrap_or(0.0),
        tp: 0.0,
        profit: None,
        commission: 0.0,
        swap: 0.0,
        result_r: None,
    };
    match (input.result_r, measured.computed_r()) {
        (Some(typed), _) => parts.push(t!("manual_entry.line.typed_r", r = format::r_text(Some(typed)))),
        (None, Some(computed)) => parts.push(t!("manual_entry.line.stop_r", r = format::r_text(Some(computed)))),
        (None, None) => {}
    }
    if parts.is_empty() {
        return match zone.lead {
            Lead::Money => t!("manual_entry.line.empty_money"),
            Lead::R => t!("manual_entry.line.empty_r"),
        };
    }
    t!("manual_entry.line.computed", parts = parts.join(" · "))
}

/// The elements of the form's fields, to bring one into view.
#[derive(Clone, Copy)]
struct Marks([ElementRef; Part::COUNT]);

impl Marks {
    fn new() -> Self {
        Self(std::array::from_fn(|_| element_ref()))
    }

    fn of(self, part: Part) -> ElementRef {
        self.0[part.index()]
    }
}

/// The signals of the trade's own fields.
#[derive(Clone, Copy)]
struct Inputs {
    /// What is wrong with the fields after a save; each is said under its field and goes when it is edited.
    problems: Signal<Vec<Problem>>,
    marks: Marks,
    account: Signal<String>,
    symbol: Signal<String>,
    side: Signal<Side>,
    opened: Moment,
    closed: Moment,
    volume: Signal<Option<f64>>,
    open_price: Signal<Option<f64>>,
    close_price: Signal<Option<f64>>,
    sl: Signal<Option<f64>>,
    tp: Signal<Option<f64>>,
    profit: Signal<Option<f64>>,
    commission: Signal<Option<f64>>,
    swap: Signal<Option<f64>>,
    result_r: Signal<Option<f64>>,
}

impl Inputs {
    /// A blank form for `account`.
    fn new(account: String) -> Self {
        Self {
            problems: signal(Vec::new()),
            marks: Marks::new(),
            account: signal(account),
            symbol: signal(String::new()),
            side: signal(Side::Long),
            opened: Moment::new(MomentValue::default()),
            closed: Moment::new(MomentValue::default()),
            volume: signal(None),
            open_price: signal(None),
            close_price: signal(None),
            sl: signal(None),
            tp: signal(None),
            profit: signal(None),
            commission: signal(None),
            swap: signal(None),
            result_r: signal(None),
        }
    }

    /// A form filled with the trade as it is stored, its times on the trader's clock.
    fn of(account: String, trade: &ManualTrade, clock: Clock) -> Self {
        let moment = |server_ms: i64| {
            let local = clock.local(server_ms);
            if trade.date_only { MomentValue::of_day(local.date()) } else { MomentValue::of(local) }
        };
        let some = |value: f64| (value != 0.0).then_some(value);
        Self {
            problems: signal(Vec::new()),
            marks: Marks::new(),
            account: signal(account),
            symbol: signal(trade.symbol.clone()),
            side: signal(trade.side),
            opened: Moment::new(moment(trade.open_time)),
            closed: Moment::new(moment(trade.close_time)),
            volume: signal(trade.volume),
            open_price: signal(trade.open_price),
            close_price: signal(trade.close_price),
            sl: signal(some(trade.sl)),
            tp: signal(some(trade.tp)),
            profit: signal(trade.profit),
            commission: signal(trade.profit.and(some(trade.commission))),
            swap: signal(trade.profit.and(some(trade.swap))),
            result_r: signal(trade.result_r),
        }
    }

    fn input(&self) -> TradeInput {
        TradeInput {
            symbol: self.symbol.get_untracked(),
            side: self.side.get_untracked(),
            opened: self.opened.value(),
            closed: self.closed.value(),
            volume: self.volume.get_untracked(),
            open_price: self.open_price.get_untracked(),
            close_price: self.close_price.get_untracked(),
            sl: self.sl.get_untracked(),
            tp: self.tp.get_untracked(),
            profit: self.profit.get_untracked(),
            commission: self.commission.get_untracked(),
            swap: self.swap.get_untracked(),
            result_r: self.result_r.get_untracked(),
        }
    }

    /// The same, read where a change of any field must show (the "tự tính" line).
    fn tracked(&self) -> TradeInput {
        TradeInput {
            symbol: self.symbol.get(),
            side: self.side.get(),
            opened: self.opened.tracked(),
            closed: self.closed.tracked(),
            volume: self.volume.get(),
            open_price: self.open_price.get(),
            close_price: self.close_price.get(),
            sl: self.sl.get(),
            tp: self.tp.get(),
            profit: self.profit.get(),
            commission: self.commission.get(),
            swap: self.swap.get(),
            result_r: self.result_r.get(),
        }
    }

    /// The trade the form holds. When it holds none, says under each field what is wrong (a time that
    /// could not be read is said by its own field already) and brings the first such field into view.
    fn checked(&self, zone: Zone) -> Option<ManualTrade> {
        let stuck: Vec<Part> = [(Part::Opened, &self.opened), (Part::Closed, &self.closed)]
            .into_iter()
            .filter(|(_, moment)| moment.problem().is_some())
            .map(|(part, _)| part)
            .collect();
        let (trade, mut problems) = match build_trade(&self.input(), zone) {
            Ok(trade) => (Some(trade), Vec::new()),
            Err(problems) => (None, problems),
        };
        problems.retain(|problem| !stuck.contains(&problem.part));
        let first = problems.iter().map(|problem| problem.part).chain(stuck).min_by_key(|part| part.rank(zone.lead));
        self.problems.set(problems);
        match first {
            Some(part) => {
                self.marks.of(part).reveal(RevealOptions::default().vertical(ScrollAlignment::Center));
                None
            }
            None => trade,
        }
    }

    /// Forgets what was said about `part` as soon as the field is edited: `changed` reads what it holds.
    fn forget_on_edit<T>(self, part: Part, changed: impl Fn() -> T + 'static) {
        effect(move || {
            let _ = changed();
            untracked(|| {
                if self.problems.with(|all| all.iter().any(|problem| problem.part == part)) {
                    self.problems.update(|all| all.retain(|problem| problem.part != part));
                }
            });
        });
    }

    /// Makes every field forget what was said about it when it is edited. Call once, while the page is built.
    fn watching(self) -> Self {
        self.forget_on_edit(Part::Symbol, move || self.symbol.get());
        self.forget_on_edit(Part::Opened, move || self.opened.tracked());
        self.forget_on_edit(Part::Closed, move || (self.opened.tracked(), self.closed.tracked()));
        self.forget_on_edit(Part::Volume, move || self.volume.get());
        self.forget_on_edit(Part::OpenPrice, move || self.open_price.get());
        self.forget_on_edit(Part::ClosePrice, move || self.close_price.get());
        self.forget_on_edit(Part::Profit, move || self.profit.get());
        self.forget_on_edit(Part::ResultR, move || (self.result_r.get(), self.open_price.get(), self.sl.get()));
        self
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

/// Whether the account keeps its trades in R alone: it has trades and none has money.
fn keeps_r_only(snapshot: &Snapshot, account: &str) -> bool {
    let Ok(id) = account.parse::<i64>() else { return false };
    let mut own = snapshot.rows.iter().filter(|row| row.trade.key.account_id == id).peekable();
    own.peek().is_some() && own.all(|row| !row.trade.money_known())
}

/// What the form asks first for the account chosen.
fn lead_of(snapshot: &Snapshot, account: &str) -> Lead {
    if keeps_r_only(snapshot, account) { Lead::R } else { Lead::Money }
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
    let inputs = Inputs::new(default_account).watching();
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
        let zone = Zone {
            clock: typed_clock(&snapshot, account_row),
            lead: lead_of(&snapshot, &account),
            date_only_ok: false,
        };
        let Some(trade) = inputs.checked(zone) else { return };
        let mut entry = JournalEntry::default();
        fields.draft().apply(&mut entry);
        entry.screenshots = shots.get_untracked();
        let symbol = tradelens_core::symbol::normalise(&trade.symbol);
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
                            notice.set(Some(t!("manual_entry.saved_next", symbol = symbol)));
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
        .child(action(t!("common.cancel"), Variant::Ghost, nav::close_manual_entry))
        .child(action_when(
            t!("manual_entry.save_next"),
            Variant::Secondary,
            move || !busy.get(),
            move || save(After::Continue),
        ))
        .child(view().child(action_icon(Icon::Check, t!("manual_entry.save"), Variant::Primary, move || {
            save(After::Open);
        })));
    page_with(
        t!("manual_entry.title"),
        t!("manual_entry.summary"),
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
                    .child(dynamic(move || notice.get().map(warning_line)))
                    .child(failure_line(failure))
                    .child(trade_card(inputs, String::new(), TradeMode::New))
                    .child(card(t!("manual_entry.journal"), journal_form(fields, shot_count, None, failure))),
            )
            .child(attachments_column(shots)),
    )
}

/// What went wrong with a save that no field is to blame for (the database, the form's layout), at the
/// top of the form where the save buttons are; what is wrong with a field is said under that field.
fn failure_line(failure: Signal<Option<String>>) -> impl IntoElement {
    dynamic(move || failure.get().map(error_line))
}

/// The column of chart images beside the form.
fn attachments_column(shots: Signal<Vec<Attachment>>) -> impl IntoElement {
    view().w(px(SIDE_WIDTH)).gap_6().child(card(
        t!("manual_entry.attachments"),
        view()
            .gap_2()
            .child(gallery(t!("manual_entry.shots"), move || shots.get(), move |list| shots.set(list)))
            .child(text(t!("manual_entry.attachments_note")).text_xs().text_color(move || theme().muted_foreground)),
    ))
}

/// The width of the column beside the form (images, or the summary of what a trade being edited keeps).
const SIDE_WIDTH: f32 = 380.0;

/// What the side card of the edit form says of the journal and images the trade keeps: they are not
/// edited here, and the link goes to where they are.
fn kept_card(key: TradeKey) -> impl IntoElement {
    let store = store();
    let summary = move || {
        let entry = store.snapshot().journal(&key);
        let mut kept = Vec::new();
        if !entry.is_empty() {
            kept.push(t!("manual_entry.kept.journal"));
        }
        if entry.rating > 0 {
            kept.push(t!("manual_entry.kept.rating", n = entry.rating));
        }
        match entry.screenshots.len() {
            0 => {}
            1 => kept.push(t!("manual_entry.kept.shots_one")),
            n => kept.push(t!("manual_entry.kept.shots_many", n = n)),
        }
        if kept.is_empty() {
            t!("manual_entry.kept.none")
        } else {
            t!("manual_entry.kept.sentence", kept = kept.join(", "))
        }
    };
    card(
        t!("manual_entry.kept.title"),
        view()
            .gap_2()
            .child(text(summary).text_sm())
            .child(text(t!("manual_entry.kept.note")).text_xs().text_color(move || theme().muted_foreground))
            .child(view().flex_row().child(link(t!("manual_entry.kept.link"), nav::close_edit_trade))),
    )
}

/// The page that edits the facts of the trade `key`: the same form as a new trade's, filled with
/// what is stored and saved over the same deals, so the trade keeps its journal, images and links.
/// A trade the trader does not own (the terminal's) has nothing to edit: the page says so.
pub(crate) fn edit_trade(key: TradeKey) -> impl IntoElement {
    let store = store();
    let loaded = signal(None::<Result<ManualTrade, String>>);
    {
        let read_key = key.clone();
        store.read(
            move |db| db.manual_trade(&read_key),
            move |result| {
                loaded.set(Some(match result {
                    Ok(Some(trade)) => Ok(trade),
                    Ok(None) => Err(t!("manual_entry.edit.not_editable")),
                    Err(error) => Err(error),
                }));
            },
        );
    }
    let back = move || nav::close_edit_trade();
    dynamic(move || {
        let key = key.clone();
        match loaded.get() {
            None => page_with(
                t!("manual_entry.edit.title"),
                t!("manual_entry.edit.reading"),
                action(t!("common.cancel"), Variant::Ghost, back),
                view().child(text(t!("common.loading")).text_sm().text_color(move || theme().muted_foreground)),
            )
            .into_element(),
            Some(Err(problem)) => page_with(
                t!("manual_entry.edit.title"),
                t!("manual_entry.edit.refused"),
                action(t!("manual_entry.edit.back"), Variant::Secondary, back),
                view().child(error_line(problem)),
            )
            .into_element(),
            Some(Ok(trade)) => untracked(|| edit_page(&key, &trade)).into_element(),
        }
    })
}

fn edit_page(key: &TradeKey, stored: &ManualTrade) -> impl IntoElement {
    let store = store();
    let write = writer();
    let snapshot = store.snapshot_now();
    let account_row = snapshot.accounts.iter().find(|a| a.id == key.account_id);
    let clock = typed_clock(&snapshot, account_row);
    let inputs = Inputs::of(key.account_id.to_string(), stored, clock).watching();
    // A trade that lacks something (money, a lot, prices, a time of day) may stay without it; one that
    // has everything is held to it, as when it was typed.
    let complete =
        stored.profit.is_some() && stored.volume.is_some() && stored.open_price.is_some() && !stored.date_only;
    let zone = Zone { clock, lead: if complete { Lead::Money } else { Lead::R }, date_only_ok: stored.date_only };
    let account = account_row.map_or_else(|| t!("manual_entry.account_n", id = key.account_id), Account::title);
    let busy = signal(false);
    let failure = signal(None::<String>);
    let save = {
        let key = key.clone();
        move || {
            if busy.get_untracked() {
                return;
            }
            let Some(trade) = inputs.checked(zone) else { return };
            busy.set(true);
            failure.set(None);
            let key = key.clone();
            write.run(
                move |db| db.update_manual_trade(&key, &trade),
                move |result| {
                    busy.set(false);
                    match result {
                        Ok(()) => nav::close_edit_trade(),
                        Err(error) => failure.set(Some(error)),
                    }
                },
            );
        }
    };
    let save_by_key = save.clone();
    on_key_with("mod+enter", KeyOptions { pass_through: true, ..KeyOptions::default() }, move |_| save_by_key());
    let actions = view()
        .flex_row()
        .items_center()
        .gap_3()
        .child(action(t!("common.cancel"), Variant::Ghost, nav::close_edit_trade))
        .child(view().child(action_icon(Icon::Check, t!("manual_entry.edit.save"), Variant::Primary, save)));
    page_with(
        t!("manual_entry.edit.title"),
        t!("manual_entry.edit.summary"),
        actions,
        view()
            .flex_row()
            .flex_wrap()
            .items_start()
            .gap_6()
            .child(view().flex_1().min_w(px(520.0)).gap_6().child(failure_line(failure)).child(trade_card(
                inputs,
                account,
                TradeMode::Edit(zone.lead),
            )))
            .child(view().w(px(SIDE_WIDTH)).gap_6().child(kept_card(key.clone()))),
    )
}

/// The least a field of the grid is wide: a number field with a unit (`1.08505`, `100.00 lot`) fits it.
const FIELD_MIN: f32 = 184.0;

/// A caption over a control, in the form's grid.
fn field(label: impl Into<SharedString>, control: impl IntoElement) -> View {
    view()
        .flex_1()
        .min_w(px(FIELD_MIN))
        .gap_1()
        .child(text(label.into()).text_xs().text_color(move || theme().muted_foreground))
        .child(control)
}

/// A [`field`] that says what is wrong with it under it, and can be brought into view.
fn checked_field(
    inputs: Inputs,
    part: Part,
    label: impl Into<SharedString>,
    control: impl IntoElement,
) -> impl IntoElement {
    field(label, control).element_ref(&inputs.marks.of(part)).child(dynamic(move || {
        inputs
            .problems
            .with(|all| all.iter().find(|problem| problem.part == part).map(|problem| problem.message.clone()))
            .map(error_note)
    }))
}

/// A number the trader types, empty until they do; it takes the width of its cell.
fn number(
    inputs: Inputs,
    part: Option<Part>,
    label: impl Into<SharedString>,
    unit: Option<&'static str>,
    value: Signal<Option<f64>>,
    step: f64,
) -> impl IntoElement {
    let label: SharedString = label.into();
    let control = number_field_fluid(
        label.clone(),
        unit.map(Into::into),
        NumberRange::any().stepping(step),
        move || value.get(),
        move |n| value.set(n),
    );
    match part {
        Some(part) => checked_field(inputs, part, label, control).into_element(),
        None => field(label, control).into_element(),
    }
}

/// The "tự tính" line: the net result, how long the trade was held and its R, once the form has
/// what they need.
fn preview_line(store: Store, inputs: Inputs, lead: Lead) -> String {
    let snapshot = store.snapshot();
    let account = inputs.account.get().parse::<i64>().ok().and_then(|id| snapshot.accounts.iter().find(|a| a.id == id));
    let zone = Zone { clock: typed_clock(&snapshot, account), lead, date_only_ok: false };
    computed_line(&inputs.tracked(), zone, account.map_or("", |a| a.currency.as_str()))
}

/// Whether the form adds a trade or edits one, and for an edit what it leads with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TradeMode {
    New,
    Edit(Lead),
}

/// The account, the symbol and the side. A trade being edited stays in its account: it is told, not chosen.
fn who_row(inputs: Inputs, account_title: String, mode: TradeMode) -> impl IntoElement {
    let store = store();
    let accounts = move || {
        let mut all: Vec<Choice> =
            store.snapshot().accounts.iter().map(|a| Choice::new(a.id.to_string(), a.title())).collect();
        all.push(Choice::new(MANUAL_ACCOUNT, t!("manual_entry.manual_account")));
        all
    };
    let (symbol, side) = (inputs.symbol, inputs.side);
    view()
        .flex_row()
        .flex_wrap()
        .items_end()
        .gap_3()
        .child(field(
            t!("manual_entry.field.account"),
            match mode {
                TradeMode::New => select_value(
                    t!("manual_entry.field.account"),
                    accounts,
                    move || SharedString::from(inputs.account.get()),
                    move |value| inputs.account.set(value.to_string()),
                )
                .into_element(),
                TradeMode::Edit(_) => view()
                    .h(px(34.0))
                    .justify_center()
                    .child(text(account_title).text_sm().font_weight(FontWeight::MEDIUM).number_of_lines(1))
                    .into_element(),
            },
        ))
        .child(checked_field(
            inputs,
            Part::Symbol,
            "Symbol",
            text_field("Symbol", None, "XAUUSD", move || symbol.get(), move |t| symbol.set(t.to_owned())),
        ))
        .child(field(
            t!("manual_entry.field.side"),
            segmented(t!("manual_entry.field.side"), SIDES, Side::label, move || side.get(), move |s| side.set(s)),
        ))
}

/// When the trade opened and closed, and how big it was.
fn when_row(inputs: Inputs) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap_3()
        .child(checked_field(
            inputs,
            Part::Opened,
            t!("manual_entry.field.opened"),
            moment_field(t!("manual_entry.field.opened"), t!("manual_entry.field.opened_time"), inputs.opened),
        ))
        .child(checked_field(
            inputs,
            Part::Closed,
            t!("manual_entry.field.closed"),
            moment_field(t!("manual_entry.field.closed"), t!("manual_entry.field.closed_time"), inputs.closed),
        ))
        .child(number(inputs, Some(Part::Volume), t!("manual_entry.field.volume"), Some("lot"), inputs.volume, 0.01))
}

/// The widest the field of the result in R grows: it is one number, not a row of them.
const RESULT_MAX: f32 = 280.0;

/// The result in R, which leads the form of a trade kept in R.
fn r_row(inputs: Inputs) -> impl IntoElement {
    view().flex_row().flex_wrap().gap_3().max_w(px(RESULT_MAX)).child(number(
        inputs,
        Some(Part::ResultR),
        t!("manual_entry.field.result_r"),
        Some("R"),
        inputs.result_r,
        0.1,
    ))
}

fn trade_card(inputs: Inputs, account_title: String, mode: TradeMode) -> impl IntoElement {
    let store = store();
    let lead = move || match mode {
        TradeMode::Edit(lead) => lead,
        TradeMode::New => lead_of(&store.snapshot(), &inputs.account.get()),
    };
    // The snapshot is read again (and announces it) whenever anything is stored, here or in another
    // window, so the rows below follow what the lead *is*, not every read of it: a `dynamic` over
    // `lead()` rebuilt the result field in the middle of a keystroke, and the rest of the typing
    // went to the field that was gone.
    let leads_r = computed(move || lead() == Lead::R);
    let leads_money = computed(move || lead() == Lead::Money);
    card(
        match mode {
            TradeMode::New => t!("manual_entry.trade_new"),
            TradeMode::Edit(_) => t!("manual_entry.trade_edit"),
        },
        view()
            .gap_3()
            .child(who_row(inputs, account_title, mode))
            .child(dynamic(move || {
                // A journal kept in R: its result leads, and the rest says it is optional.
                leads_r.get().then(|| {
                    untracked(|| {
                        view().gap_2().child(r_row(inputs)).child(
                            text(t!("manual_entry.r_note")).text_xs().text_color(move || theme().muted_foreground),
                        )
                    })
                })
            }))
            .child(when_row(inputs))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_3()
                    .child(number(
                        inputs,
                        Some(Part::OpenPrice),
                        t!("manual_entry.field.open_price"),
                        None,
                        inputs.open_price,
                        0.01,
                    ))
                    .child(number(
                        inputs,
                        Some(Part::ClosePrice),
                        t!("manual_entry.field.close_price"),
                        None,
                        inputs.close_price,
                        0.01,
                    ))
                    .child(number(inputs, None, "Stop loss", None, inputs.sl, 0.01))
                    .child(number(inputs, None, "Take profit", None, inputs.tp, 0.01)),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_3()
                    .child(number(
                        inputs,
                        Some(Part::Profit),
                        t!("manual_entry.field.profit"),
                        None,
                        inputs.profit,
                        1.0,
                    ))
                    .child(number(inputs, None, "Commission", None, inputs.commission, 0.1))
                    .child(number(inputs, None, "Swap", None, inputs.swap, 0.1)),
            )
            .child(dynamic(move || leads_money.get().then(|| untracked(|| r_row(inputs)))))
            .child(
                view().p_3().rounded(move || theme().radius).border_1().border_color(move || theme().border).child(
                    text(move || preview_line(store, inputs, lead()))
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
            ),
    )
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveTime};
    use tradelens_core::Side;

    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).expect("a date")
    }

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).expect("a time")
    }

    fn moment(d: u32, h: u32, m: u32) -> MomentValue {
        MomentValue { day: Some(day(d)), time: Some(at(h, m)) }
    }

    fn input() -> TradeInput {
        TradeInput {
            symbol: " GBPUSD ".into(),
            side: Side::Short,
            opened: moment(20, 16, 5),
            closed: moment(20, 17, 40),
            volume: Some(0.8),
            open_price: Some(1.2684),
            close_price: Some(1.2651),
            sl: Some(1.2705),
            tp: None,
            profit: Some(264.0),
            commission: Some(-5.6),
            swap: None,
            result_r: None,
        }
    }

    fn zone(lead: Lead) -> Zone {
        Zone { clock: Clock { server_offset_min: None, local_offset_min: 420 }, lead, date_only_ok: false }
    }

    /// Runs `check` where the text is Vietnamese.
    fn in_vietnamese<R: Send + 'static>(check: impl FnOnce() -> R + Send + 'static) -> R {
        crate::i18n::with_text(crate::i18n::Lang::Vi, check)
    }

    #[test]
    fn a_complete_form_becomes_a_trade() {
        in_vietnamese(|| {
            let trade = build_trade(&input(), zone(Lead::Money)).expect("a valid trade");
            assert_eq!(trade.symbol, "GBPUSD");
            assert_eq!(trade.side, Side::Short);
            assert_eq!(trade.tp, 0.0, "a level left empty is 0, which means none");
            assert_eq!(trade.swap, 0.0);
            assert_eq!((trade.close_time - trade.open_time) / 60_000, 95);
            assert_eq!(trade.result_r, None, "the R is read from the stop, not stored, unless the trader typed one");
            assert_eq!(
                computed_line(&input(), zone(Lead::Money), "USD"),
                "Tự tính: kết quả ròng +$258.40 · giữ 1g 35p · R tự tính từ SL +1.6R"
            );
        });
        crate::i18n::with_text(crate::i18n::Lang::En, || {
            let blank = TradeInput {
                profit: None,
                opened: MomentValue::default(),
                closed: MomentValue::default(),
                open_price: None,
                close_price: None,
                ..input()
            };
            assert_eq!(
                computed_line(&blank, zone(Lead::Money), "USD"),
                "Calculated: enter the entry and exit times and the profit/loss to see the net result and the hold time."
            );
            assert!(computed_line(&input(), zone(Lead::Money), "USD").starts_with("Calculated: net result +$258.40"));
        });
    }

    #[test]
    fn the_r_the_trader_types_stands_over_the_one_the_stop_says() {
        in_vietnamese(|| {
            let typed = TradeInput { result_r: Some(2.0), ..input() };
            assert_eq!(build_trade(&typed, zone(Lead::Money)).expect("valid").result_r, Some(2.0));
            assert!(computed_line(&typed, zone(Lead::Money), "USD").ends_with("R nhập tay +2.0R"));
        });
    }

    #[test]
    fn typed_times_move_to_the_servers_clock() {
        in_vietnamese(|| {
            // 16:05 in Vietnam (UTC+7) is 11:05 on a server at UTC+2.
            let on_server =
                Zone { clock: Clock { server_offset_min: Some(120), local_offset_min: 420 }, ..zone(Lead::Money) };
            let trade = build_trade(&input(), on_server).expect("valid");
            assert_eq!(tradelens_core::time::naive(trade.open_time).format("%H:%M").to_string(), "11:05");
            let kept = build_trade(&input(), zone(Lead::Money)).expect("valid");
            assert_eq!(tradelens_core::time::naive(kept.open_time).format("%H:%M").to_string(), "16:05");
        });
    }

    #[test]
    fn what_is_wrong_is_said_in_words_and_under_the_field_it_concerns() {
        in_vietnamese(|| {
            let fix = |edit: fn(&mut TradeInput)| {
                let mut input = input();
                edit(&mut input);
                build_trade(&input, zone(Lead::Money)).expect_err("must be refused").remove(0)
            };
            let said = |problem: Problem, part: Part, text: &str| {
                assert_eq!(problem.part, part, "{}", problem.message);
                assert!(problem.message.contains(text), "{}", problem.message);
            };
            said(fix(|i| i.symbol = " ".into()), Part::Symbol, "symbol");
            said(fix(|i| i.volume = None), Part::Volume, "Khối lượng");
            said(fix(|i| i.close_price = Some(0.0)), Part::ClosePrice, "Giá ra");
            said(fix(|i| i.opened = MomentValue::default()), Part::Opened, "Thời gian vào");
            said(fix(|i| i.closed = moment(20, 15, 0)), Part::Closed, "sau thời gian vào");
            said(fix(|i| i.profit = None), Part::Profit, "Nhập lãi/lỗ");
        });
        crate::i18n::with_text(crate::i18n::Lang::En, || {
            let refused = build_trade(&TradeInput { volume: None, ..input() }, zone(Lead::Money)).expect_err("refused");
            assert_eq!(refused[0].message, "Volume must be a number greater than 0");
        });
    }

    #[test]
    fn every_problem_is_listed_the_one_the_form_shows_first_first() {
        in_vietnamese(|| {
            let empty = TradeInput {
                symbol: String::new(),
                closed: MomentValue::default(),
                volume: None,
                profit: None,
                ..input()
            };
            let parts: Vec<Part> =
                build_trade(&empty, zone(Lead::Money)).expect_err("refused").iter().map(|p| p.part).collect();
            assert_eq!(parts, [Part::Symbol, Part::Closed, Part::Volume, Part::Profit]);
        });
        // A journal kept in R shows its result before the rest.
        assert!(Part::ResultR.rank(Lead::R) < Part::Symbol.rank(Lead::R));
        assert!(Part::ResultR.rank(Lead::Money) > Part::Profit.rank(Lead::Money));
    }

    #[test]
    fn a_trade_kept_in_r_needs_only_its_r_or_what_makes_one() {
        in_vietnamese(trade_kept_in_r_needs_only_its_r_or_what_makes_one);
    }

    fn trade_kept_in_r_needs_only_its_r_or_what_makes_one() {
        let bare = TradeInput {
            volume: None,
            open_price: None,
            close_price: None,
            sl: None,
            profit: None,
            commission: None,
            result_r: None,
            opened: MomentValue::of_day(day(20)),
            closed: MomentValue::of_day(day(21)),
            ..input()
        };
        let refused = build_trade(&bare, zone(Lead::R)).expect_err("nothing says how it went");
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].part, Part::ResultR, "said under the result in R");
        assert!(refused[0].message.contains("kết quả (R)"), "{}", refused[0].message);
        let with_r = build_trade(&TradeInput { result_r: Some(-1.0), ..bare.clone() }, zone(Lead::R)).expect("valid");
        assert_eq!((with_r.profit, with_r.result_r, with_r.volume, with_r.date_only), (None, Some(-1.0), None, true));
        assert_eq!(
            with_r.gaps(),
            tradelens_core::Gaps::VOLUME.with(tradelens_core::Gaps::PRICE).with(tradelens_core::Gaps::TIME)
        );

        // The prices and the stop make the R, and it is stored (the trade has no money to read it from).
        let measured =
            TradeInput { open_price: Some(100.0), close_price: Some(103.0), sl: Some(98.0), side: Side::Long, ..bare };
        assert_eq!(build_trade(&measured, zone(Lead::R)).expect("valid").result_r, Some(1.5));
    }

    #[test]
    fn the_money_form_keeps_asking_for_money_even_when_an_r_is_typed() {
        in_vietnamese(|| {
            let typed = TradeInput { profit: None, result_r: Some(1.0), ..input() };
            assert!(build_trade(&typed, zone(Lead::Money)).expect_err("refused")[0].message.starts_with("Nhập lãi/lỗ"));
            assert!(build_trade(&typed, zone(Lead::R)).is_ok());
        });
    }

    #[test]
    fn a_trade_and_its_journal_are_stored_together_on_the_manual_account() {
        let trade = in_vietnamese(|| build_trade(&input(), zone(Lead::Money)).expect("a valid trade"));
        let mut db = Db::in_memory().expect("opens");
        let entry = JournalEntry { notes: "vào sớm".into(), rating: 3, ..JournalEntry::default() };
        let key = store_trade(&mut db, MANUAL_ACCOUNT, &trade, &entry).expect("stored");
        assert_eq!(db.journal(&key).expect("reads").expect("an entry").notes, "vào sớm");
        let again = store_trade(&mut db, MANUAL_ACCOUNT, &trade, &JournalEntry::default()).expect("stored again");
        assert_eq!(again.account_id, key.account_id, "hand-entered trades share one account");
        assert_ne!(again.position_id, key.position_id);
        assert!(db.journal(&again).expect("reads").is_none(), "no entry is written for an empty journal");
    }
}
