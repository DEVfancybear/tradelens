//! Tài khoản & kỷ luật: the accounts, the rules of each, today's status against them, the
//! violations of the last 30 days and the checklist before an entry.
//! Design: `docs/design/screens/accounts-discipline.html`.

use chrono::{Days, NaiveDate};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    Account, ChecklistItem, Db, DbError, Rules,
    rules::{self, DayStatus},
};

use super::account_trades::{self, AccountTrades};
use crate::{
    format::{self, Sign},
    store::{Reading, Writer, query, writer},
    ui::{
        checklist_editor::checklist_editor,
        chips::{Tone, chip, figure_text},
        controls::{Choice, Variant, action, select_value, text_field},
        forms::{NumberRange, check_box, number_field, progress_rule},
        parts::{card, empty_state, page_with, property_row},
        rule_row::rule_row,
        selectable_card::selectable_card,
        theme::loss,
    },
};

/// How many days back the violations list looks.
const WINDOW_DAYS: u64 = 30;

/// How many violations the list shows before it says how many more there are.
const LISTED: usize = 30;

/// The limits a rule that was never switched on starts with.
const DEFAULT_LOSS: f64 = 300.0;
const DEFAULT_TRADES: f64 = 5.0;
const DEFAULT_RISK_PCT: f64 = 1.5;
const DEFAULT_STREAK: f64 = 3.0;
const DEFAULT_TARGET: f64 = 150.0;

/// One violation as the list shows it.
#[derive(Debug, Clone, PartialEq)]
struct ViolationLine {
    day: NaiveDate,
    text: String,
    place: String,
}

/// One account with its rules, today's status and recent violations.
#[derive(Debug, Clone)]
struct AccountEntry {
    account: Account,
    rules: Rules,
    /// The items of this account's own checklist (not the shared ones).
    own_checklist: Vec<ChecklistItem>,
    today: DayStatus,
    /// The violations of the last [`WINDOW_DAYS`] days, newest first.
    violations: Vec<ViolationLine>,
    /// The share of days traded in the window without a violation.
    compliance: Option<f64>,
}

/// What the page reads.
#[derive(Debug, Clone, Default)]
struct AccountsData {
    shared_checklist: Vec<ChecklistItem>,
    accounts: Vec<AccountEntry>,
}

impl AccountsData {
    fn entry(&self, id: i64) -> Option<&AccountEntry> {
        self.accounts.iter().find(|a| a.account.id == id)
    }
}

/// Reads the accounts and checks their rules against their trades.
fn load(db: &Db, today: NaiveDate) -> Result<AccountsData, DbError> {
    let shared = db.checklist(None)?;
    let from = today.checked_sub_days(Days::new(WINDOW_DAYS - 1)).unwrap_or(today);
    let accounts = account_trades::load(db, None)?
        .iter()
        .map(|own| entry_of(db, own, &shared, today, from))
        .collect::<Result<Vec<_>, DbError>>()?;
    Ok(AccountsData { shared_checklist: shared, accounts })
}

fn entry_of(
    db: &Db,
    own: &AccountTrades,
    shared: &[ChecklistItem],
    today: NaiveDate,
    from: NaiveDate,
) -> Result<AccountEntry, DbError> {
    let rules = db.rules(own.account.id)?;
    let own_checklist = db
        .checklist(Some(own.account.id))?
        .into_iter()
        .filter(|item| !shared.iter().any(|s| s.id == item.id))
        .collect();
    let samples = own.samples();
    let curve = own.curve();
    let found = rules::evaluate(&rules, &samples, |ms| rules::balance_before(&curve, ms), own.clock);
    let in_window: Vec<_> = found.into_iter().filter(|v| v.day >= from && v.day <= today).collect();
    let mut trading_days: Vec<NaiveDate> =
        samples.iter().map(|s| own.clock.day(s.trade.open_time)).filter(|d| *d >= from && *d <= today).collect();
    trading_days.sort_unstable();
    trading_days.dedup();
    let violations = in_window
        .iter()
        .rev()
        .map(|v| {
            let place = if let Some(key) = &v.trade {
                own.trades.iter().find(|t| t.key == *key).map_or_else(String::new, |t| {
                    format!("{} · {}", t.symbol, own.clock.local(t.open_time).format("%H:%M"))
                })
            } else {
                format!("{} lệnh", rules::day_status(&samples, own.clock, v.day).trades)
            };
            ViolationLine { day: v.day, text: v.kind.label(), place }
        })
        .collect();
    Ok(AccountEntry {
        today: rules::day_status(&samples, own.clock, today),
        compliance: rules::compliance_rate(&in_window, trading_days.len()),
        account: own.account.clone(),
        rules,
        own_checklist,
        violations,
    })
}

/// `value` as a count of at least zero, or `None` when it is not one.
fn whole(value: f64) -> Option<u32> {
    // Through text, so that a value outside `u32` is refused instead of cut.
    value.is_finite().then(|| format!("{:.0}", value.round()).parse().ok()).flatten()
}

/// The limit of a rule as the number field shows it.
fn real(count: Option<u32>) -> Option<f64> {
    count.map(f64::from)
}

/// The currency as a unit after a number, for the ones traders hold accounts in.
fn unit_of(currency: &str) -> Option<&'static str> {
    ["USD", "EUR", "GBP", "JPY", "AUD", "CAD", "CHF", "NZD", "SGD", "VND", "USC"]
        .into_iter()
        .find(|code| code.eq_ignore_ascii_case(currency.trim()))
}

/// How long ago `then_ms` was, as the card says it.
fn ago(now_ms: i64, then_ms: i64) -> String {
    let minutes = (now_ms - then_ms).max(0) / 60_000;
    match minutes {
        0 => "vừa xong".to_owned(),
        1..=59 => format!("{minutes} phút trước"),
        60..=1_439 => format!("{} giờ trước", minutes / 60),
        _ => format!("{} ngày trước", minutes / 1_440),
    }
}

/// The server's offset as a choice: whole hours as `UTC+2`, anything else in minutes.
fn offset_title(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let (hours, rest) = (minutes.abs() / 60, minutes.abs() % 60);
    if rest == 0 { format!("UTC{sign}{hours}") } else { format!("UTC{sign}{hours}:{rest:02}") }
}

/// The offsets the server can be at, plus the one the account has when it is none of them.
fn offset_choices(current: Option<i32>) -> Vec<Choice> {
    let mut choices = vec![Choice::new("", "Chưa biết (giữ giờ server)")];
    let mut minutes: Vec<i32> = (-12..=14).map(|h| h * 60).collect();
    if let Some(now) = current.filter(|m| !minutes.contains(m)) {
        minutes.push(now);
        minutes.sort_unstable();
    }
    choices.extend(minutes.into_iter().map(|m| Choice::new(m.to_string(), offset_title(m))));
    choices
}

pub(crate) fn accounts() -> impl IntoElement {
    let data = query(|db, _| load(db, account_trades::today()));
    let write = writer();
    let chosen = signal(None::<i64>);
    // The account shown: the one pressed, else the first. Equal ids do not wake the sections.
    let current = computed(move || {
        let wanted = chosen.get();
        data.value().and_then(|d| {
            d.accounts.iter().find(|a| Some(a.account.id) == wanted).or(d.accounts.first()).map(|a| a.account.id)
        })
    });
    let notice = signal(None::<String>);
    let summary = move || {
        data.value().map_or_else(
            || SharedString::from("Các tài khoản MT5, quy tắc rủi ro và checklist trước lệnh"),
            |d| SharedString::from(format!("{} tài khoản · quy tắc rủi ro và checklist trước lệnh", d.accounts.len())),
        )
    };
    page_with(
        "Tài khoản & kỷ luật",
        summary,
        view(),
        view()
            .gap_6()
            .child(dynamic(move || data.error().map(|error| text(error).text_sm().text_color(loss))))
            .child(dynamic(move || notice.get().map(|m| text(m).text_sm().text_color(loss))))
            .child(dynamic(move || {
                let none = data.value().is_some_and(|d| d.accounts.is_empty());
                untracked(|| {
                    if none {
                        empty_state(
                            "Chưa có tài khoản nào",
                            "Tài khoản xuất hiện khi EA gửi deal đầu tiên hoặc khi bạn import báo cáo MT5.",
                        )
                        .into_element()
                    } else {
                        account_cards(data, chosen, current).into_element()
                    }
                })
            }))
            .child(dynamic(move || {
                let id = current.get();
                untracked(|| id.map(|id| detail(data, write, id, notice)))
            })),
    )
}

/// The accounts as cards to pick from.
fn account_cards(
    data: Reading<AccountsData>,
    chosen: Signal<Option<i64>>,
    current: Memo<Option<i64>>,
) -> impl IntoElement {
    let now = chrono::Utc::now().timestamp_millis();
    let list = data.value().map_or_default(|d| d.accounts.clone());
    view().flex_row().flex_wrap().gap_4().children(list.into_iter().map(move |entry| {
        let AccountEntry { account, rules, today, .. } = entry;
        let id = account.id;
        let money = |value: Option<f64>| {
            value.map_or_else(|| "—".to_owned(), |v| format::money(v, Sign::OnlyNegative, &account.currency))
        };
        let name = if account.name.trim().is_empty() { account.login.clone() } else { account.name.clone() };
        let identity = format!("{} · {}", account.login, account.server);
        let synced =
            account.last_sync.map_or_else(|| "Chưa đồng bộ".to_owned(), |t| format!("Đồng bộ {}", ago(now, t)));
        let loss_line = rules.max_daily_loss.map(|limit| {
            format!(
                "Lỗ hôm nay {} / giới hạn {}",
                format::money(today.loss(), Sign::OnlyNegative, &account.currency),
                format::money(limit, Sign::OnlyNegative, &account.currency)
            )
        });
        let (balance, equity) = (money(account.last_balance), money(account.last_equity));
        let currency = if account.currency.is_empty() { "—".to_owned() } else { account.currency.clone() };
        let figure = |label: &'static str, value: String| {
            view()
                .gap_1()
                .child(text(label).text_xs().text_color(move || theme().muted_foreground))
                .child(figure_text(move || value.clone()).text_sm().font_weight(FontWeight::SEMIBOLD))
        };
        let body = view()
            .gap_2()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(text(name.clone()).text_sm().font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
                    .child(chip(currency, Tone::Neutral)),
            )
            .child(figure_text(move || identity.clone()).text_xs().text_color(move || theme().muted_foreground))
            .child(view().flex_row().gap_6().child(figure("Balance", balance)).child(figure("Equity", equity)))
            .child(text(synced).text_xs().text_color(move || theme().muted_foreground))
            .child(loss_line.map(|line| text(line).text_xs().text_color(move || theme().muted_foreground)));
        view().w(px(264.0)).child(selectable_card(
            format!("Tài khoản {name}"),
            move || current.get() == Some(id),
            move || chosen.set(Some(id)),
            body,
        ))
    }))
}

/// The two columns under the cards, for the account `id`.
fn detail(data: Reading<AccountsData>, write: Writer, id: i64, notice: Signal<Option<String>>) -> impl IntoElement {
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
                .child(rules_card(data, write, id, notice))
                .child(info_card(data, write, id, notice)),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(320.0))
                .max_w(px(420.0))
                .gap_6()
                .child(checklist_card(data, write, id, notice))
                .child(today_card(data, id))
                .child(violations_card(data, id)),
        )
}

/// What `pick` reads of the account `id`, or its default while the account is not read yet.
fn read<T: Default>(data: Reading<AccountsData>, id: i64, pick: impl FnOnce(&AccountEntry) -> T) -> T {
    data.value().and_then(|d| d.entry(id).map(pick)).unwrap_or_default()
}

/// The five limits being edited. A limit is `None` while its rule is off.
#[derive(Clone, Copy)]
struct RuleDraft {
    loss: Signal<Option<f64>>,
    trades: Signal<Option<f64>>,
    risk: Signal<Option<f64>>,
    streak: Signal<Option<f64>>,
    target: Signal<Option<f64>>,
}

impl RuleDraft {
    fn of(rules: &Rules) -> Self {
        Self {
            loss: signal(rules.max_daily_loss),
            trades: signal(real(rules.max_daily_trades)),
            risk: signal(rules.max_risk_pct),
            streak: signal(real(rules.max_consecutive_losses)),
            target: signal(rules.daily_target),
        }
    }

    fn rules(&self) -> Rules {
        Rules {
            max_daily_loss: self.loss.get_untracked(),
            max_daily_trades: self.trades.get_untracked().and_then(whole),
            max_risk_pct: self.risk.get_untracked(),
            max_consecutive_losses: self.streak.get_untracked().and_then(whole),
            daily_target: self.target.get_untracked(),
        }
    }
}

fn rules_card(data: Reading<AccountsData>, write: Writer, id: i64, notice: Signal<Option<String>>) -> impl IntoElement {
    let saved = read(data, id, |a| a.rules.clone());
    let currency = read(data, id, |a| a.account.currency.clone());
    let unit = unit_of(&currency);
    let draft = RuleDraft::of(&saved);
    let everywhere = signal(false);
    let status = signal(String::new());
    let save = move || {
        let rules = draft.rules();
        let all = everywhere.get_untracked();
        status.set("Đang lưu…".to_owned());
        write.run(
            move |db: &mut Db| {
                if all {
                    for account in db.accounts()? {
                        db.save_rules(account.id, &rules)?;
                    }
                    Ok(())
                } else {
                    db.save_rules(id, &rules)
                }
            },
            move |result| match result {
                Ok(()) => {
                    notice.set(None);
                    status.set("Đã lưu quy tắc".to_owned());
                }
                Err(error) => status.set(error),
            },
        );
    };
    let money_range = NumberRange::between(0.0, 100_000_000.0).stepping(10.0);
    let count_range = NumberRange::between(1.0, 1_000.0);
    card(
        "Quy tắc kỷ luật",
        view()
            .child(rule_row(
                "Lỗ tối đa trong ngày",
                "Chạm ngưỡng: thanh trạng thái chuyển đỏ và các lệnh vi phạm được ghi lại",
                unit,
                money_range,
                DEFAULT_LOSS,
                move || draft.loss.get(),
                move |v| draft.loss.set(v),
            ))
            .child(rule_row(
                "Số lệnh tối đa mỗi ngày",
                "Tính lệnh mở trong ngày, theo giờ chọn ở Cài đặt",
                Some("lệnh"),
                count_range,
                DEFAULT_TRADES,
                move || draft.trades.get(),
                move |v| draft.trades.set(v),
            ))
            .child(rule_row(
                "Risk tối đa mỗi lệnh",
                "Theo balance trước lệnh; cần lệnh có SL để EA tính risk",
                Some("%"),
                NumberRange::between(0.1, 100.0).stepping(0.1),
                DEFAULT_RISK_PCT,
                move || draft.risk.get(),
                move |v| draft.risk.set(v),
            ))
            .child(rule_row(
                "Thua liên tiếp thì nghỉ",
                "Lệnh vào sau chuỗi thua trong cùng ngày bị đánh dấu vi phạm",
                Some("lệnh"),
                count_range,
                DEFAULT_STREAK,
                move || draft.streak.get(),
                move |v| draft.streak.set(v),
            ))
            .child(rule_row(
                "Mục tiêu ngày",
                "Đạt mục tiêu: gợi ý dừng giao dịch",
                unit,
                money_range,
                DEFAULT_TARGET,
                move || draft.target.get(),
                move |v| draft.target.set(v),
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .justify_end()
                    .gap_4()
                    .pt_4()
                    .child(text(move || status.get()).text_sm().text_color(move || theme().muted_foreground))
                    .child(
                        check_box("Áp dụng cho mọi tài khoản", move || everywhere.get(), move |v| everywhere.set(v))
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(text("Áp dụng cho mọi tài khoản").text_sm()),
                    )
                    .child(action("Lưu quy tắc", Variant::Primary, save)),
            ),
    )
}

/// The account's own details: name, broker, currency, starting balance and the server's zone.
fn info_card(data: Reading<AccountsData>, write: Writer, id: i64, notice: Signal<Option<String>>) -> impl IntoElement {
    let saved = read(data, id, |a| a.account.clone());
    let title = saved.title();
    let name = signal(saved.name.clone());
    let broker = signal(saved.broker.clone());
    let currency = signal(saved.currency.clone());
    let initial = signal(saved.initial_balance);
    let offset = signal(saved.server_offset_min.map_or_default(|m| m.to_string()));
    let offsets = saved.server_offset_min;
    let status = signal(String::new());
    let save = move || {
        let mut account = saved.clone();
        name.get_untracked().trim().clone_into(&mut account.name);
        broker.get_untracked().trim().clone_into(&mut account.broker);
        account.currency = currency.get_untracked().trim().to_uppercase();
        account.initial_balance = initial.get_untracked();
        account.server_offset_min = offset.get_untracked().parse().ok();
        status.set("Đang lưu…".to_owned());
        write.run(
            move |db: &mut Db| db.update_account(&account),
            move |result| match result {
                Ok(()) => {
                    notice.set(None);
                    status.set("Đã lưu thông tin".to_owned());
                }
                Err(error) => status.set(error),
            },
        );
    };
    card(
        "Thông tin tài khoản",
        view()
            .gap_2()
            .child(text(title).text_xs().text_color(move || theme().muted_foreground))
            .child(property_row(
                "Tên hiển thị",
                text_field("Tên hiển thị", None, "Tên gợi nhớ", move || name.get(), move |t| name.set(t.to_owned())),
            ))
            .child(property_row(
                "Broker",
                text_field("Broker", None, "Broker", move || broker.get(), move |t| broker.set(t.to_owned())),
            ))
            .child(property_row(
                "Tiền tệ",
                text_field("Tiền tệ", None, "USD", move || currency.get(), move |t| currency.set(t.to_owned())),
            ))
            .child(property_row(
                "Balance ban đầu",
                number_field(
                    "Balance ban đầu",
                    unit_of(&currency.get_untracked()),
                    NumberRange::between(0.0, 1_000_000_000.0).stepping(100.0),
                    move || initial.get(),
                    move |v| initial.set(v),
                ),
            ))
            .child(property_row(
                "Giờ server MT5",
                select_value(
                    "Giờ server MT5",
                    move || offset_choices(offsets),
                    move || SharedString::from(offset.get()),
                    move |v| offset.set(v.to_string()),
                ),
            ))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .justify_end()
                    .gap_4()
                    .pt_2()
                    .child(text(move || status.get()).text_sm().text_color(move || theme().muted_foreground))
                    .child(action("Lưu thông tin", Variant::Secondary, save)),
            ),
    )
}

/// The checklist before an entry: the items for every account, then this account's own.
fn checklist_card(
    data: Reading<AccountsData>,
    write: Writer,
    id: i64,
    notice: Signal<Option<String>>,
) -> impl IntoElement {
    let save = move |scope: Option<i64>, items: Vec<ChecklistItem>| {
        write.run(
            move |db: &mut Db| db.save_checklist(scope, &items),
            move |result| {
                if let Err(error) = result {
                    notice.set(Some(error));
                }
            },
        );
    };
    card(
        "Checklist trước lệnh",
        view()
            .gap_4()
            .child(
                view()
                    .gap_2()
                    .child(text("Dùng chung cho mọi tài khoản").text_xs().text_color(move || theme().muted_foreground))
                    .child(checklist_editor(
                        "Checklist dùng chung",
                        move || data.value().map_or_default(|d| d.shared_checklist.clone()),
                        move |items| save(None, items),
                    )),
            )
            .child(
                view()
                    .gap_2()
                    .child(text("Riêng tài khoản này").text_xs().text_color(move || theme().muted_foreground))
                    .child(checklist_editor(
                        "Checklist của tài khoản",
                        move || read(data, id, |a| a.own_checklist.clone()),
                        move |items| save(Some(id), items),
                    )),
            ),
    )
}

/// Today against each rule that is on.
fn today_card(data: Reading<AccountsData>, id: i64) -> impl IntoElement {
    let rules = computed(move || read(data, id, |a| a.rules.clone()));
    let status = move || read(data, id, |a| a.today);
    let currency = move || read(data, id, |a| a.account.currency.clone());
    let money = move |value: f64| format::money(value, Sign::OnlyNegative, &currency());
    let loss_on = computed(move || rules.get().max_daily_loss.is_some());
    let trades_on = computed(move || rules.get().max_daily_trades.is_some());
    let streak_on = computed(move || rules.get().max_consecutive_losses.is_some());
    let target_on = computed(move || rules.get().daily_target.is_some());
    let any = computed(move || loss_on.get() || trades_on.get() || streak_on.get() || target_on.get());
    card(
        "Hôm nay",
        view()
            .gap_4()
            .child(when(
                move || !any.get(),
                || {
                    text("Chưa bật quy tắc nào: bật ở khối Quy tắc kỷ luật.")
                        .text_sm()
                        .text_color(move || theme().muted_foreground)
                },
            ))
            .child(when(loss_on, move || {
                progress_rule(
                    "Lỗ trong ngày",
                    move || {
                        let limit = rules.get().max_daily_loss.unwrap_or_default();
                        let loss = status().loss();
                        format!("{} / {}", money(-loss), money(limit))
                    },
                    move || ratio(status().loss(), rules.get().max_daily_loss),
                )
            }))
            .child(when(trades_on, move || {
                progress_rule(
                    "Số lệnh trong ngày",
                    move || format!("{} / {}", status().trades, rules.get().max_daily_trades.unwrap_or_default()),
                    move || ratio(f64::from(status().trades), real(rules.get().max_daily_trades)),
                )
            }))
            .child(when(streak_on, move || {
                progress_rule(
                    "Thua liên tiếp",
                    move || {
                        format!(
                            "{} / {}",
                            status().losing_streak,
                            rules.get().max_consecutive_losses.unwrap_or_default()
                        )
                    },
                    move || ratio(f64::from(status().losing_streak), real(rules.get().max_consecutive_losses)),
                )
            }))
            .child(when(target_on, move || {
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(text("Mục tiêu ngày").text_sm().font_weight(FontWeight::MEDIUM))
                    .child(
                        figure_text(move || {
                            let limit = rules.get().daily_target.unwrap_or_default();
                            format!("{} / {}", money(status().net), money(limit))
                        })
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                    )
            })),
    )
}

/// `used / limit`; zero when the limit is not set.
fn ratio(used: f64, limit: Option<f64>) -> f64 {
    limit.filter(|l| *l > 0.0).map_or(0.0, |l| used / l)
}

/// The violations of the last 30 days.
fn violations_card(data: Reading<AccountsData>, id: i64) -> impl IntoElement {
    let lines = move || read(data, id, |a| a.violations.clone());
    let compliance = move || read(data, id, |a| a.compliance);
    card(
        "Vi phạm 30 ngày",
        view()
            .gap_3()
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_end()
                    .gap_3()
                    .child(
                        figure_text(move || lines().len().to_string())
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || if lines().is_empty() { theme().foreground } else { loss() }),
                    )
                    .child(
                        text(move || match compliance() {
                            Some(rate) => format!("vi phạm · tuân thủ {} số ngày", format::percent(rate)),
                            None => "vi phạm · chưa có ngày giao dịch nào".to_owned(),
                        })
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(dynamic(move || {
                let all = lines();
                untracked(|| {
                    if all.is_empty() {
                        return text("Không có vi phạm nào trong 30 ngày qua.")
                            .text_sm()
                            .text_color(move || theme().muted_foreground)
                            .into_element();
                    }
                    let more = all.len().saturating_sub(LISTED);
                    view()
                        .child(view().children(all.into_iter().take(LISTED).map(|line| violation_row(&line))))
                        .child((more > 0).then(|| {
                            text(format!("…và {more} vi phạm cũ hơn"))
                                .pt_2()
                                .text_xs()
                                .text_color(move || theme().muted_foreground)
                        }))
                        .into_element()
                })
            })),
    )
}

fn violation_row(line: &ViolationLine) -> impl IntoElement + use<> {
    view()
        .flex_row()
        .items_start()
        .gap_3()
        .py_2()
        .border_t_1()
        .border_color(move || theme().border)
        .child(
            figure_text({
                let day = line.day.format("%d/%m").to_string();
                move || day.clone()
            })
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_1()
                .child(text(line.text.clone()).text_sm().text_color(loss))
                .child(text(line.place.clone()).text_xs().text_color(move || theme().muted_foreground)),
        )
}

#[cfg(test)]
mod tests {
    use tradelens_core::{DealBatch, DealEntry, DealKind, model::AccountInfo, time::parse_mt5_time};

    use super::*;

    #[test]
    fn counts_come_from_whole_non_negative_numbers() {
        assert_eq!(whole(3.0), Some(3));
        assert_eq!(whole(2.6), Some(3));
        assert_eq!(whole(-1.0), None);
        assert_eq!(whole(f64::NAN), None);
        assert_eq!(whole(1e12), None);
    }

    #[test]
    fn card_text_helpers() {
        assert_eq!(ago(10 * 60_000, 10 * 60_000), "vừa xong");
        assert_eq!(ago(10 * 60_000, 4 * 60_000), "6 phút trước");
        assert_eq!(ago(5 * 3_600_000, 0), "5 giờ trước");
        assert_eq!(ago(3 * 86_400_000, 0), "3 ngày trước");
        assert_eq!(offset_title(120), "UTC+2");
        assert_eq!(offset_title(-330), "UTC-5:30");
        assert_eq!(unit_of(" usd "), Some("USD"));
        assert_eq!(unit_of("XYZ"), None);
        assert!((ratio(50.0, Some(100.0)) - 0.5).abs() < 1e-9);
        assert!(ratio(50.0, None).abs() < 1e-9);
    }

    #[test]
    fn an_odd_server_offset_stays_a_choice() {
        let titles = |current| offset_choices(current).into_iter().map(|c| c.title.to_string()).collect::<Vec<_>>();
        assert!(titles(Some(330)).contains(&"UTC+5:30".to_owned()));
        assert!(!titles(Some(120)).contains(&"UTC+2:00".to_owned()));
        assert_eq!(titles(None)[0], "Chưa biết (giữ giờ server)");
    }

    fn deal(
        ticket: i64,
        position: i64,
        at: &str,
        kind: DealKind,
        entry: DealEntry,
        profit: f64,
    ) -> tradelens_core::Deal {
        tradelens_core::Deal {
            ticket,
            order: ticket,
            position_id: position,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: "EURUSD".into(),
            kind,
            entry,
            volume: 0.1,
            price: 1.1,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit,
            magic: 0,
            comment: String::new(),
            sl: 0.0,
            tp: 0.0,
            risk: None,
            reason: None,
        }
    }

    #[test]
    fn the_status_and_violations_follow_the_saved_rules() {
        let mut db = Db::in_memory().expect("a memory database opens");
        let batch = DealBatch {
            account: AccountInfo { login: "1".into(), server: "S".into(), ..AccountInfo::default() },
            deals: vec![
                deal(1, 1, "2026.11.20 09:00:00", DealKind::Buy, DealEntry::In, 0.0),
                deal(2, 1, "2026.11.20 10:00:00", DealKind::Sell, DealEntry::Out, -120.0),
                deal(3, 2, "2026.11.20 11:00:00", DealKind::Buy, DealEntry::In, 0.0),
                deal(4, 2, "2026.11.20 12:00:00", DealKind::Sell, DealEntry::Out, -90.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("stored");
        let today = NaiveDate::from_ymd_opt(2026, 11, 20).expect("a date");
        let before = load(&db, today).expect("loads");
        assert!(before.accounts[0].violations.is_empty(), "no rule is on");
        let account = before.accounts[0].account.id;
        let rules = Rules { max_daily_loss: Some(200.0), max_daily_trades: Some(1), ..Rules::default() };
        db.save_rules(account, &rules).expect("saved");
        let after = load(&db, today).expect("loads again");
        let entry = &after.accounts[0];
        assert_eq!(entry.rules, rules);
        assert_eq!(entry.violations.len(), 2, "the loss and the trade count");
        assert!((entry.today.loss() - 210.0).abs() < 1e-9);
        assert_eq!(entry.today.trades, 2);
        assert_eq!(entry.compliance, Some(0.0), "the only day traded was broken");
        assert!(entry.violations.iter().any(|v| v.place == "2 lệnh"));
    }

    #[test]
    fn a_shared_item_is_not_listed_as_the_accounts_own() {
        let mut db = Db::in_memory().expect("a memory database opens");
        let id = db
            .upsert_account(&AccountInfo { login: "1".into(), server: "S".into(), ..AccountInfo::default() })
            .expect("an account");
        db.save_checklist(None, &[ChecklistItem { id: 0, text: "chung".into() }]).expect("shared");
        db.save_checklist(Some(id), &[ChecklistItem { id: 0, text: "riêng".into() }]).expect("own");
        let data = load(&db, NaiveDate::from_ymd_opt(2026, 11, 20).expect("a date")).expect("loads");
        assert_eq!(data.shared_checklist.len(), 1);
        assert_eq!(data.accounts[0].own_checklist.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["riêng"]);
    }
}
