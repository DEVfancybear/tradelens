//! The "Kết nối MT5" card of Nguồn dữ liệu: what the machine has for the connection, the sign-in
//! form, the accounts signed in to with their open positions, and the plan prompt switch.
//! The design's EA card is replaced by this one (`docs/design/README.md`, deviations).

use std::path::PathBuf;

use chrono::{DateTime, Local};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_mt5::AccountRef;

use super::position;
use crate::{
    format::{self, Sign},
    mt5::{Live, Mt5, PlanPrompt, State, Status, mt5},
    ui::{
        chips::{Tone, chip, pnl_text},
        controls::{
            Choice, Variant, action, action_when, confirm_then, root_owner, secret_field, select_value, text_field,
        },
        files::{Accept, Pick, pick_files},
        forms::switch_row,
        parts::{card, error_line, warning_banner},
        theme as tokens,
    },
};

/// What the terminal picker takes.
const TERMINAL_FILE: Accept = Accept { name: "terminal64.exe", extensions: &["exe"] };

/// The sign-in form's fields, kept while the card lives.
#[derive(Clone, Copy)]
struct Form {
    login: Signal<String>,
    password: Signal<String>,
    server: Signal<String>,
    /// The installed terminal chosen (its `terminal64.exe`), empty for the first one found.
    terminal: Signal<String>,
    /// A terminal picked with the file dialog, offered beside those found.
    picked: Signal<Option<PathBuf>>,
}

pub(super) fn mt5_card() -> impl IntoElement {
    let mt5 = mt5();
    // The machine is looked at (Python, terminals) once the card shows and the settings are read.
    let loaded = computed(move || mt5.state.with(|state| state.loaded));
    effect(move || {
        if loaded.get() {
            untracked(|| mt5.look_once());
        }
    });
    let form = Form {
        login: signal(String::new()),
        password: signal(String::new()),
        server: signal(String::new()),
        terminal: signal(String::new()),
        picked: signal(None),
    };
    card(
        "Kết nối MT5",
        view()
            .gap_4()
            .child(
                text("Đăng nhập tài khoản MT5 ngay tại đây: TradeLens chạy một bản MetaTrader 5 riêng, ẩn cho mỗi tài khoản và tự đưa lệnh vào nhật ký trong vài giây, không cần mở MT5 hay cài EA. Mật khẩu được giữ trong Windows Credential Manager.")
                    .text_sm()
                    .text_color(move || theme().muted_foreground),
            )
            .child(setup_view(mt5))
            .child(sign_in_form(mt5, form))
            .child(accounts_view(mt5))
            .child(switch_row(
                "Hỏi kế hoạch khi có lệnh mới",
                Some("Mở popup ghi kế hoạch khi một lệnh vừa vào trên tài khoản đang kết nối"),
                move || mt5.state.with(|state| state.prompt_plans),
                move |on| mt5.set_prompt_plans(if on { PlanPrompt::Ask } else { PlanPrompt::Skip }),
            )),
    )
}

/// Why the connection cannot be used yet, with what fixes it; nothing when it can.
fn setup_view(mt5: Mt5) -> impl IntoElement {
    let shown =
        computed(move || mt5.state.with(|state| (state.setup.missing(), state.busy.clone(), state.setup.clone())));
    dynamic(move || {
        let (missing, busy, setup) = shown.get();
        untracked(|| {
            let idle = busy.is_none();
            let line = match (&setup.python, setup.demo) {
                (_, true) => Some("Chế độ thử: tài khoản và lệnh là dữ liệu mẫu trong bộ nhớ.".to_owned()),
                (Some(python), false) if python.has_package => {
                    Some(format!("Python {} · gói MetaTrader5 sẵn sàng ({})", python.version, python.display()))
                }
                _ => None,
            };
            view()
                .gap_2()
                .when_some(busy, |v, busy| v.child(text(busy).text_xs().text_color(move || theme().muted_foreground)))
                .when_some(missing.filter(|_| idle), |v, missing| v.child(warning_banner(missing)))
                .when_some(line, |v, line| v.child(text(line).text_xs().text_color(move || theme().muted_foreground)))
                .when(!setup.demo && setup.checked, |v| {
                    v.child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .gap_2()
                            .child(action_when(
                                "Kiểm tra lại",
                                Variant::Secondary,
                                move || idle,
                                move || mt5.check_again(),
                            ))
                            .when(setup.needs_python(), |v| {
                                let label = if setup.python.is_some() {
                                    "Cài gói MetaTrader5"
                                } else {
                                    "Cài Python tự động"
                                };
                                v.child(action_when(
                                    label,
                                    Variant::Primary,
                                    move || idle,
                                    move || mt5.install_python(),
                                ))
                            }),
                    )
                })
        })
    })
}

/// The terminals the trader can choose: those found, then one picked with the file dialog.
fn terminal_choices(state: &State, picked: Option<&PathBuf>) -> Vec<Choice> {
    let mut choices: Vec<Choice> = state
        .setup
        .installs
        .iter()
        .map(|install| Choice::new(install.terminal.display().to_string(), install.name.clone()))
        .collect();
    if let Some(picked) = picked.filter(|p| !state.setup.installs.iter().any(|install| &install.terminal == *p)) {
        choices.push(Choice::new(picked.display().to_string(), picked.display().to_string()));
    }
    choices
}

/// The terminal the form signs in with: the one chosen, else the first offered.
fn chosen_terminal(state: &State, form: Form) -> Option<PathBuf> {
    let wanted = form.terminal.get_untracked();
    let choices = terminal_choices(state, form.picked.get_untracked().as_ref());
    choices
        .iter()
        .find(|choice| choice.value.as_ref() == wanted.as_str())
        .or(choices.first())
        .map(|choice| PathBuf::from(choice.value.as_ref()))
}

fn sign_in_form(mt5: Mt5, form: Form) -> impl IntoElement {
    let demo = computed(move || mt5.state.with(|state| state.setup.demo));
    // No sign-in while Python is being installed or the machine looked at: it would start a second install.
    let idle = computed(move || mt5.state.with(|state| state.busy.is_none()));
    let sign_in = move || {
        let terminal = mt5.state.with_untracked(|state| {
            if state.setup.demo { Some(PathBuf::from("terminal64.exe")) } else { chosen_terminal(state, form) }
        });
        let Some(terminal) = terminal else {
            mt5.state.update(|state| {
                state.error = Some("Chưa có terminal MetaTrader 5 nào: cài MT5 hoặc chọn terminal64.exe.".to_owned());
            });
            return;
        };
        let account = AccountRef::new(&form.login.get_untracked(), &form.server.get_untracked());
        mt5.sign_in(account, terminal, &form.password.get_untracked());
        form.password.set(String::new());
    };
    view()
        .gap_3()
        .p_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .child(text("Đăng nhập tài khoản").text_sm().font_weight(FontWeight::MEDIUM))
        .child(fields_row(form))
        .child(dynamic(move || (!demo.get()).then(|| untracked(|| terminal_row(mt5, form)))))
        .child(
            view()
                .flex_row()
                .gap_3()
                .items_center()
                .child(action_when("Đăng nhập", Variant::Primary, move || idle.get(), sign_in))
                .child(
                    text("Lần đầu app chép terminal MT5 cho tài khoản này (vài trăm MB), mất khoảng một phút.")
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child({
            let error = computed(move || mt5.state.with(|state| state.error.clone()));
            dynamic(move || error.get().map(error_line))
        })
}

/// The login, the password and the server.
fn fields_row(form: Form) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap_3()
        .child(labelled(
            "Số tài khoản",
            text_field(
                "Số tài khoản",
                None,
                "52012345",
                move || form.login.get(),
                move |t| form.login.set(t.to_owned()),
            ),
        ))
        .child(labelled(
            "Mật khẩu",
            secret_field(
                "Mật khẩu",
                "Mật khẩu giao dịch",
                move || form.password.get(),
                move |t| {
                    form.password.set(t.to_owned());
                },
            ),
        ))
        .child(labelled(
            "Server",
            text_field(
                "Server",
                None,
                "ICMarketsSC-Demo",
                move || form.server.get(),
                move |t| {
                    form.server.set(t.to_owned());
                },
            ),
        ))
}

/// The installed terminal to copy, chosen among those found or picked with the file dialog.
fn terminal_row(mt5: Mt5, form: Form) -> impl IntoElement {
    let owner = root_owner();
    let pick = move || {
        owner.run(move || {
            spawn(async move {
                match pick_files(TERMINAL_FILE, Pick::One).await {
                    Ok(paths) => {
                        if let Some(path) = paths.into_iter().next() {
                            form.terminal.set(path.display().to_string());
                            form.picked.set(Some(path));
                        }
                    }
                    Err(error) => mt5.state.update(|state| state.error = Some(error)),
                }
            });
        });
    };
    view()
        .flex_row()
        .flex_wrap()
        .items_end()
        .gap_3()
        .child(labelled(
            "Terminal MT5",
            dynamic(move || {
                let none = mt5.state.with(|state| terminal_choices(state, form.picked.get().as_ref()).is_empty());
                untracked(|| {
                    if none {
                        return text("Chưa thấy MetaTrader 5 trên máy: cài MT5 của broker rồi bấm \"Kiểm tra lại\", hoặc chọn terminal64.exe.")
                            .text_sm()
                            .text_color(tokens::warning)
                            .into_element();
                    }
                    select_value(
                        "Terminal MT5",
                        move || mt5.state.with(|state| terminal_choices(state, form.picked.get().as_ref())),
                        move || SharedString::from(form.terminal.get()),
                        move |next| form.terminal.set(next.to_string()),
                    )
                    .into_element()
                })
            }),
        ))
        .child(action("Chọn terminal64.exe…", Variant::Ghost, pick))
}

/// A field with its label above it.
fn labelled(label: &'static str, field: impl IntoElement) -> impl IntoElement {
    view()
        .gap_1()
        .min_w(px(180.0))
        .flex_1()
        .child(text(label).text_xs().font_weight(FontWeight::MEDIUM).text_color(move || theme().muted_foreground))
        .child(field)
}

/// What a status chip says.
fn status_chip(status: &Status) -> (String, Tone) {
    match status {
        Status::Preparing => ("Đang chuẩn bị terminal".to_owned(), Tone::Neutral),
        Status::Connecting => ("Đang kết nối".to_owned(), Tone::Neutral),
        Status::Connected => ("Đã kết nối".to_owned(), Tone::Profit),
        Status::Retrying(_) => ("Đang thử lại".to_owned(), Tone::Loss),
        Status::Refused(_) => ("Đăng nhập lỗi".to_owned(), Tone::Loss),
        Status::Stopped => ("Đã ngắt".to_owned(), Tone::Neutral),
    }
}

/// The line under an account's title: its server, balance and when it was last read.
fn account_line(live: &Live) -> String {
    let mut parts = vec![live.account.server.clone()];
    if let Some(info) = &live.info {
        if let Some(balance) = info.balance {
            parts.push(format!("Balance {}", format::money(balance, Sign::OnlyNegative, &info.currency)));
        }
        if let Some(equity) = info.equity {
            parts.push(format!("Equity {}", format::money(equity, Sign::OnlyNegative, &info.currency)));
        }
    }
    if let Some(at) = live.polled_at.and_then(DateTime::from_timestamp_millis) {
        parts.push(format!("cập nhật {}", at.with_timezone(&Local).format("%H:%M:%S")));
    }
    parts.join(" · ")
}

/// The accounts, one block each. The list is rebuilt only when an account comes or goes: each
/// block follows its account's figures in place, so that a press on its buttons is never lost to a
/// poll (every 2 s).
fn accounts_view(mt5: Mt5) -> impl IntoElement {
    let accounts = computed(move || {
        mt5.state.with(|state| state.accounts.iter().map(|live| live.account.clone()).collect::<Vec<_>>())
    });
    dynamic(move || {
        let accounts = accounts.get();
        untracked(|| {
            if accounts.is_empty() {
                return view().child(
                    text("Chưa có tài khoản MT5 nào kết nối.").text_sm().text_color(move || theme().muted_foreground),
                );
            }
            view().gap_3().children(accounts.into_iter().map(move |account| account_row(mt5, account)))
        })
    })
}

fn account_row(mt5: Mt5, account: AccountRef) -> impl IntoElement {
    let owner = root_owner();
    let live = {
        let account = account.clone();
        computed(move || mt5.state.with(|state| state.live(&account).cloned()))
    };
    let status = computed(move || live.with(|live| live.as_ref().map(|live| live.status.clone())));
    let tickets = computed(move || {
        live.with(|live| {
            live.as_ref().map_or_default(|live| live.positions.iter().map(|p| p.ticket).collect::<Vec<_>>())
        })
    });
    let forget = {
        let account = account.clone();
        move || {
            let account = account.clone();
            let title = live.with_untracked(|live| live.as_ref().map_or_default(Live::title));
            confirm_then(
                owner,
                "Xoá tài khoản MT5?",
                format!(
                    "Ngắt kết nối {title}, xoá mật khẩu đã lưu và bản terminal riêng của tài khoản. Lệnh đã vào nhật ký vẫn giữ nguyên."
                ),
                "Xoá",
                move || mt5.forget(&account),
            );
        }
    };
    let reconnect = {
        let account = account.clone();
        move || mt5.reconnect(&account)
    };
    view()
        .gap_2()
        .p_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_1()
                        .child(
                            text(move || live.with(|live| live.as_ref().map_or_default(Live::title)))
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .number_of_lines(1),
                        )
                        .child(
                            text(move || live.with(|live| live.as_ref().map_or_default(account_line)))
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(dynamic(move || {
                    status.get().map(|status| {
                        let (label, tone) = status_chip(&status);
                        untracked(|| chip(label, tone))
                    })
                })),
        )
        .child(dynamic(move || {
            status.get().and_then(|status| match status {
                Status::Retrying(why) | Status::Refused(why) => Some(error_line(why)),
                _ => None,
            })
        }))
        .child(dynamic(move || {
            let tickets = tickets.get();
            let account = account.clone();
            (!tickets.is_empty()).then(|| {
                untracked(|| {
                    view()
                        .gap_1()
                        .pt_1()
                        .child(
                            text(format!("Lệnh đang mở ({})", tickets.len()))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().muted_foreground),
                        )
                        .children(tickets.into_iter().map(|ticket| position_row(live, account.clone(), ticket)))
                })
            })
        }))
        .child(
            view().flex_row().gap_2().pt_1().child(action("Kết nối lại", Variant::Secondary, reconnect)).child(action(
                "Xoá",
                Variant::Destructive,
                forget,
            )),
        )
}

/// One open position: its figures follow each poll in place.
fn position_row(live: Memo<Option<Live>>, account: AccountRef, ticket: i64) -> impl IntoElement {
    let position = move || {
        live.with(|live| {
            live.as_ref().and_then(|live| {
                let currency = live.info.as_ref().map_or_default(|info| info.currency.clone());
                live.positions.iter().find(|p| p.ticket == ticket).map(|p| (p.clone(), currency))
            })
        })
    };
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .py_1()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            text(move || {
                position().map_or_default(|(p, _)| {
                    format!(
                        "{} · {} {} · {} → {}",
                        p.symbol,
                        p.side.side().label(),
                        format::lots(p.volume),
                        format::price(p.open_price),
                        format::price(p.current_price)
                    )
                })
            })
            .flex_1()
            .text_sm()
            .number_of_lines(1),
        )
        .child(
            pnl_text(move || {
                position().map_or_default(|(p, currency)| format::money(p.profit, Sign::Always, &currency))
            })
            .text_sm(),
        )
        .child(action("Quản lý", Variant::Secondary, move || position::open(account.clone(), ticket)))
}
