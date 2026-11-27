//! The "Kết nối MT5" card of Nguồn dữ liệu: what the machine has for the connection, the sign-in
//! form, the accounts signed in to with their open positions, and the plan prompt switch.
//! The design's EA card is replaced by this one (`docs/design/README.md`, deviations).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_mt5::{AccountRef, install};

use super::position;
use crate::{
    format::{self, Sign},
    mt5::{Live, Mt5, Notice, PasswordChoice, PlanPrompt, State, Status, mt5},
    ui::{
        chips::{Tone, chip, pnl_text},
        controls::{
            Choice, Variant, action, action_when, confirm_then, root_owner, secret_field, select_value, text_field,
        },
        files::{Accept, Pick, pick_files},
        forms::{check_box, switch_row},
        parts::{card, error_line, warning_banner},
        theme as tokens,
    },
};

/// What the terminal picker takes.
const TERMINAL_FILE: Accept = Accept { name: || "terminal64.exe".to_owned(), extensions: &["exe"] };

/// The sign-in form's fields, kept while the card lives.
#[derive(Clone, Copy)]
struct Form {
    login: Signal<String>,
    password: Signal<String>,
    /// Whether the password is kept for the next start (ticked unless the trader unticks it).
    remember: Signal<bool>,
    server: Signal<String>,
    /// The installed terminal chosen (its `terminal64.exe`), empty for the first one found.
    terminal: Signal<String>,
    /// A terminal picked with the file dialog, offered beside those found.
    picked: Signal<Option<PathBuf>>,
    /// The login and server last filled in from a chosen terminal: a field still holding them may be
    /// filled again, one the trader typed in is left alone.
    filled: Signal<(String, String)>,
    /// The terminal (or account copy) chosen last, whose account is being read: a read that arrives
    /// after another was chosen is dropped.
    reading: Signal<String>,
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
        remember: signal(true),
        server: signal(String::new()),
        terminal: signal(String::new()),
        picked: signal(None),
        filled: signal((String::new(), String::new())),
        reading: signal(String::new()),
    };
    card(
        t!("mt5_connect.title"),
        view()
            .gap_4()
            .child(text(t!("mt5_connect.intro")).text_sm().text_color(move || theme().muted_foreground))
            .child(setup_view(mt5))
            .child(sign_in_form(mt5, form))
            .child(accounts_view(mt5))
            .child(switch_row(
                t!("mt5_connect.plan_switch"),
                Some(t!("mt5_connect.plan_hint")),
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
                (_, true) => Some(t!("mt5_connect.demo")),
                (Some(python), false) if python.has_package => {
                    Some(t!("mt5_connect.ready", version = python.version, path = python.display()))
                }
                _ => None,
            };
            view()
                .gap_2()
                .when_some(busy, |v, busy| {
                    v.child(text(busy.text()).text_xs().text_color(move || theme().muted_foreground))
                })
                .when_some(missing.filter(|_| idle), |v, missing| v.child(warning_banner(missing.text())))
                .when_some(line, |v, line| v.child(text(line).text_xs().text_color(move || theme().muted_foreground)))
                .when(!setup.demo && setup.checked, |v| {
                    v.child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .gap_2()
                            .child(action_when(
                                t!("mt5_connect.check_again"),
                                Variant::Secondary,
                                move || idle,
                                move || mt5.check_again(),
                            ))
                            .when(setup.needs_python(), |v| {
                                let label = if setup.python.is_some() {
                                    t!("mt5_connect.install_package")
                                } else {
                                    t!("mt5_connect.install_python")
                                };
                                v.child(action_when(
                                    label,
                                    Variant::Primary,
                                    move || idle,
                                    move || mt5.install_python(),
                                ))
                            })
                            .when(setup.needs_terminal(), |v| {
                                v.child(action_when(
                                    t!("mt5_connect.install_terminal"),
                                    Variant::Primary,
                                    move || idle,
                                    move || mt5.install_terminal(),
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
        // None chosen: the first terminal that is no copy the app made for an account.
        .or_else(|| choices.iter().find(|choice| !crate::mt5::is_own_copy(Path::new(choice.value.as_ref()))))
        .or(choices.first())
        .map(|choice| PathBuf::from(choice.value.as_ref()))
}

/// Why the sign-in button is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Blocked {
    /// The demo is getting ready.
    DemoBusy,
    NotWindows,
    /// Something installs or checks.
    Busy,
    /// The machine is not looked at yet.
    Checking,
    NeedPython,
    NeedTerminal,
}

impl Blocked {
    fn text(self) -> String {
        match self {
            Self::DemoBusy => t!("mt5_connect.blocked.demo_busy"),
            Self::NotWindows => t!("mt5_connect.blocked.not_windows"),
            Self::Busy => t!("mt5_connect.blocked.busy"),
            Self::Checking => t!("mt5_connect.blocked.checking"),
            Self::NeedPython => t!("mt5_connect.blocked.need_python"),
            Self::NeedTerminal => t!("mt5_connect.blocked.need_terminal"),
        }
    }
}

/// Why the sign-in button is off, `None` when it is on: in the demo it always is; otherwise the
/// machine must have been looked at, Python must have the `MetaTrader5` package (the card's
/// "Install Python automatically" installs both) and an MT5 terminal must be found or picked, and
/// nothing may be installing.
fn sign_in_blocked(state: &State, picked: Option<&PathBuf>) -> Option<Blocked> {
    if state.setup.demo {
        return state.busy.is_some().then_some(Blocked::DemoBusy);
    }
    if !cfg!(windows) {
        return Some(Blocked::NotWindows);
    }
    if state.busy.is_some() {
        return Some(Blocked::Busy);
    }
    if !state.setup.checked {
        return Some(Blocked::Checking);
    }
    if state.setup.python.as_ref().is_none_or(|python| !python.has_package) {
        return Some(Blocked::NeedPython);
    }
    if terminal_choices(state, picked).is_empty() {
        return Some(Blocked::NeedTerminal);
    }
    None
}

fn sign_in_form(mt5: Mt5, form: Form) -> impl IntoElement {
    let demo = computed(move || mt5.state.with(|state| state.setup.demo));
    // What stops a sign-in, said beside the button: nothing in the demo; otherwise Python with its
    // package and an MT5 terminal must be there first, and nothing may be installing.
    let blocked = computed(move || {
        let picked = form.picked.get();
        mt5.state.with(|state| sign_in_blocked(state, picked.as_ref()))
    });
    let sign_in = move || {
        let terminal = mt5.state.with_untracked(|state| {
            if state.setup.demo { Some(PathBuf::from("terminal64.exe")) } else { chosen_terminal(state, form) }
        });
        let Some(terminal) = terminal else {
            mt5.state.update(|state| {
                state.error = Some(Notice::NoTerminal);
            });
            return;
        };
        let account = AccountRef::new(&form.login.get_untracked(), &form.server.get_untracked());
        let choice = if form.remember.get_untracked() { PasswordChoice::Remember } else { PasswordChoice::ThisRunOnly };
        mt5.sign_in(account, terminal, &form.password.get_untracked(), choice);
        form.password.set(String::new());
    };
    view()
        .gap_3()
        .p_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .child(text(t!("mt5_connect.sign_in_title")).text_sm().font_weight(FontWeight::MEDIUM))
        .child(fields_row(form))
        .child(remember_row(form))
        .child(dynamic(move || (!demo.get()).then(|| untracked(|| terminal_row(mt5, form)))))
        .child(
            view()
                .flex_row()
                .gap_3()
                .items_center()
                .child(action_when(
                    t!("mt5_connect.sign_in"),
                    Variant::Primary,
                    move || blocked.get().is_none(),
                    sign_in,
                ))
                .child(
                    text(move || blocked.get().map_or_else(|| t!("mt5_connect.first_time"), Blocked::text))
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child({
            let error = computed(move || mt5.state.with(|state| state.error.clone()));
            dynamic(move || error.get().map(|notice| error_line(notice.text())))
        })
}

/// The login, the password and the server.
fn fields_row(form: Form) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap_3()
        .child(labelled(
            t!("mt5_connect.login"),
            text_field(
                t!("mt5_connect.login"),
                None,
                "52012345",
                move || form.login.get(),
                move |t| form.login.set(t.to_owned()),
            ),
        ))
        .child(labelled(
            t!("mt5_connect.password"),
            secret_field(
                t!("mt5_connect.password"),
                t!("mt5_connect.password_placeholder"),
                move || form.password.get(),
                move |t| {
                    form.password.set(t.to_owned());
                },
            ),
        ))
        .child(labelled(
            t!("mt5_connect.server"),
            text_field(
                t!("mt5_connect.server"),
                None,
                "ICMarketsSC-Demo",
                move || form.server.get(),
                move |t| {
                    form.server.set(t.to_owned());
                },
            ),
        ))
}

/// "Remember the password", with where a remembered one is kept.
fn remember_row(form: Form) -> impl IntoElement {
    view()
        .gap_1()
        .child(
            check_box(t!("mt5_connect.remember"), move || form.remember.get(), move |on| form.remember.set(on))
                .flex_row()
                .items_center()
                .gap_2()
                .child(text(t!("mt5_connect.remember")).text_sm()),
        )
        .child(text(t!("mt5_connect.remember_hint")).text_xs().text_color(move || theme().muted_foreground))
}

/// The installed terminal to copy, chosen among those found or picked with the file dialog.
fn terminal_row(mt5: Mt5, form: Form) -> impl IntoElement {
    let owner = root_owner();
    let choices = computed(move || mt5.state.with(|state| terminal_choices(state, form.picked.get().as_ref())));
    let pick = move || {
        owner.run(move || {
            spawn(async move {
                match pick_files(TERMINAL_FILE, Pick::One).await {
                    Ok(paths) => {
                        if let Some(path) = paths.into_iter().next() {
                            form.picked.set(Some(path.clone()));
                            choose_terminal(owner, mt5, form, &path.display().to_string());
                        }
                    }
                    Err(error) => mt5.state.update(|state| state.error = Some(Notice::Words(error))),
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
            t!("mt5_connect.terminal"),
            dynamic(move || {
                // Through a memo compared with `==`: the state changes at every poll (balance,
                // positions), and rebuilding the select then closed its open list.
                let none = choices.with(Vec::is_empty);
                untracked(|| {
                    if none {
                        return text(t!("mt5_connect.no_terminal"))
                            .text_sm()
                            .text_color(tokens::warning)
                            .into_element();
                    }
                    select_value(
                        t!("mt5_connect.terminal"),
                        move || choices.get(),
                        move || SharedString::from(form.terminal.get()),
                        move |next| choose_terminal(owner, mt5, form, &next),
                    )
                    .into_element()
                })
            }),
        ))
        .child(action(t!("mt5_connect.pick"), Variant::Ghost, pick))
}

/// Chooses the terminal at `terminal` and fills the login and the server with the account it
/// logged in to last (a terminal of the account the trader means to add). The terminal's settings
/// are read on a blocking thread, under `owner` (the card's root, which a rebuilt select does not
/// cancel); what arrives after another terminal was chosen is dropped.
///
/// A copy the app made for an account only names that account: its login and server are filled in,
/// and the terminal goes back at once to the first one that is no such copy, the MetaTrader the new account is
/// copied from.
fn choose_terminal(owner: Owner, mt5: Mt5, form: Form, terminal: &str) {
    let chosen = terminal.to_owned();
    // At once, so that a sign-in pressed before the account is read never copies the copy.
    let source =
        crate::mt5::is_own_copy(Path::new(terminal)).then(|| mt5.state.with_untracked(source_terminal)).flatten();
    form.terminal.set(source.unwrap_or_else(|| chosen.clone()));
    form.reading.set(chosen.clone());
    owner.run(move || {
        spawn(async move {
            let path = PathBuf::from(&chosen);
            let Ok(last) = blocking(move || install::last_login(&path)).await else { return };
            if form.reading.get_untracked() == chosen {
                fill_from_terminal(form, last.unwrap_or_default());
            }
        });
    });
}

/// The first terminal found that is no copy the app made for an account, as the select names it.
fn source_terminal(state: &State) -> Option<String> {
    state
        .setup
        .installs
        .iter()
        .find(|install| !crate::mt5::is_own_copy(&install.terminal))
        .map(|install| install.terminal.display().to_string())
}

/// Puts `login` and `server` (empty when the terminal tells none) into the fields that are empty or
/// still hold what the previous terminal filled in; a value the trader typed stays.
fn fill_from_terminal(form: Form, (login, server): (String, String)) {
    let (filled_login, filled_server) = form.filled.get_untracked();
    if may_fill(&form.login.get_untracked(), &filled_login) {
        form.login.set(login.clone());
    }
    if may_fill(&form.server.get_untracked(), &filled_server) {
        form.server.set(server.clone());
    }
    form.filled.set((login, server));
}

/// Whether a field holding `now` may take a terminal's value: it is empty, or holds what was filled
/// in from a terminal before (`filled`), not something the trader typed.
fn may_fill(now: &str, filled: &str) -> bool {
    now.trim().is_empty() || now == filled
}

/// A field with its label above it.
fn labelled(label: String, field: impl IntoElement) -> impl IntoElement {
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
        Status::Preparing => (t!("mt5_connect.status.preparing"), Tone::Neutral),
        Status::Connecting => (t!("mt5_connect.status.connecting"), Tone::Neutral),
        Status::Connected => (t!("mt5_connect.status.connected"), Tone::Profit),
        Status::Retrying(_) => (t!("mt5_connect.status.retrying"), Tone::Loss),
        Status::Refused(_) => (t!("mt5_connect.status.refused"), Tone::Loss),
        Status::Stopped => (t!("mt5_connect.status.stopped"), Tone::Neutral),
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
        parts.push(t!("mt5_connect.updated", time = at.with_timezone(&Local).format("%H:%M:%S")));
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
                return view()
                    .child(text(t!("mt5_connect.no_accounts")).text_sm().text_color(move || theme().muted_foreground));
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
                t!("mt5_connect.forget.title"),
                t!("mt5_connect.forget.body", title = title),
                t!("mt5_connect.forget.confirm"),
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
                Status::Retrying(why) | Status::Refused(why) => Some(error_line(why.text())),
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
                            text(t!("mt5_connect.open_trades", n = tickets.len()))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().muted_foreground),
                        )
                        .children(tickets.into_iter().map(|ticket| position_row(live, account.clone(), ticket)))
                })
            })
        }))
        .child(account_buttons(status, reconnect, forget))
}

/// An account's buttons: "Reconnect" while its link is stuck (`offers_reconnect`), and "Sign out".
fn account_buttons(
    status: Memo<Option<Status>>,
    reconnect: impl Fn() + Clone + 'static,
    forget: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .gap_2()
        .pt_1()
        .child(dynamic(move || {
            let stuck = status.with(|status| status.as_ref().is_some_and(offers_reconnect));
            let reconnect = reconnect.clone();
            stuck.then(|| untracked(|| action(t!("mt5_connect.reconnect"), Variant::Secondary, reconnect)))
        }))
        .child(action(t!("mt5_connect.sign_out"), Variant::Destructive, forget))
}

/// Whether an account in `status` shows "Reconnect": only when its link gave up or keeps failing,
/// never while it is connected or still on its way.
fn offers_reconnect(status: &Status) -> bool {
    matches!(status, Status::Retrying(_) | Status::Refused(_) | Status::Stopped)
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
        .child(action(t!("mt5_connect.manage"), Variant::Secondary, move || position::open(account.clone(), ticket)))
}

#[cfg(test)]
mod tests {
    use tradelens_mt5::{install::Install, python::Python};

    use super::*;

    #[test]
    fn sign_in_waits_for_python_and_a_terminal_except_in_the_demo() {
        let mut state = State::default();
        state.setup.demo = true;
        assert_eq!(sign_in_blocked(&state, None), None, "the demo needs nothing");
        state.setup.demo = false;
        assert!(sign_in_blocked(&state, None).is_some(), "before the machine is looked at");
        state.setup.checked = true;
        if !cfg!(windows) {
            assert_eq!(sign_in_blocked(&state, None), Some(Blocked::NotWindows));
            return;
        }
        assert_eq!(sign_in_blocked(&state, None), Some(Blocked::NeedPython));
        let python = Python { program: "python".into(), args: Vec::new(), version: "3.14".into(), has_package: false };
        state.setup.python = Some(python);
        assert_eq!(sign_in_blocked(&state, None), Some(Blocked::NeedPython), "the package is missing");
        state.setup.python.as_mut().expect("set above").has_package = true;
        assert_eq!(sign_in_blocked(&state, None), Some(Blocked::NeedTerminal), "no terminal");
        let picked = PathBuf::from("D:/MT5/terminal64.exe");
        assert_eq!(sign_in_blocked(&state, Some(&picked)), None, "a picked terminal is enough");
        state.setup.installs = vec![Install::at(PathBuf::from("C:/MT5/terminal64.exe"))];
        assert_eq!(sign_in_blocked(&state, None), None, "a terminal found is enough");
        state.busy = Some(crate::mt5::Busy::InstallingPackage);
        assert!(sign_in_blocked(&state, None).is_some(), "not while installing");
    }

    #[test]
    fn a_chosen_terminal_fills_only_what_the_trader_has_not_typed() {
        assert!(may_fill("", ""), "an empty field");
        assert!(may_fill("  ", "123"), "a blank one");
        assert!(may_fill("123", "123"), "what the previous terminal filled in");
        assert!(!may_fill("456", "123"), "a login the trader typed");
        assert!(!may_fill("456", ""), "typed before any terminal was chosen");
    }
}
