//! Playbook: quy tắc, checklist, ảnh mẫu và số liệu từng setup.
//! Design: `docs/design/screens/playbook.html`: the list of setups at the left, the chosen one
//! at the right (rules, checklist, example pictures, statistics by grade), edited in place.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    Db, DbError,
    catalog::{CatalogItem, CatalogKind},
    metrics::{self, Group, Sample},
    playbook::{self, Playbook},
};

use super::catalog_tags::catalog_tags;
use crate::{
    format::{self, Sign},
    nav::{self, Route},
    snapshot::Snapshot,
    store::{Reading, query, store, writer},
    trade_list::Filter,
    ui::{
        chips::{Tone, chip, pnl_text},
        controls::{
            Variant, action, action_icon, action_icon_when, confirm_then, link, root_owner, text_field, textarea,
        },
        forms::{NumberRange, number_field},
        gallery::gallery,
        icons::{Icon, icon},
        lines::{filled, line_list},
        list_item::{Column, list_item, mini_table},
        parts::{card, empty_state, error_line, error_note, page_with, stat_tile},
    },
};

/// The setup shown at the right (`None`: the first one that is not archived). The search sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Selected(Option<i64>);

/// Shows the setup `id` the next time the page is on screen (or at once, when it is).
pub(super) fn select(id: i64) {
    global::<Selected>().set(Selected(Some(id)));
}

/// What the form of a setup shows when it opens: the setup, or a blank one.
type Draft = Playbook;

pub(crate) fn playbook() -> impl IntoElement {
    let store = store();
    let selected = global::<Selected>();
    let editing = signal(None::<Draft>);
    let timeframes = query(|db, _| db.catalog(CatalogKind::Timeframe));
    let grades = query(|db, _| db.catalog(CatalogKind::SetupGrade));
    let current = computed(move || {
        let snapshot = store.snapshot();
        let wanted = selected.get().0;
        wanted
            .and_then(|id| snapshot.playbooks.iter().find(|p| p.id == id))
            .or_else(|| snapshot.playbooks.iter().find(|p| !p.archived))
            .cloned()
    });
    let groups = computed(move || setup_groups(&store.snapshot()));
    let is_editing = computed(move || editing.with(Option::is_some));
    page_with(
        t!("playbook.title"),
        t!("playbook.summary"),
        dynamic(move || {
            (!is_editing.get()).then(|| {
                untracked(|| {
                    action_icon(Icon::Plus, t!("playbook.add"), Variant::Primary, move || {
                        editing.set(Some(Draft::default()));
                    })
                })
            })
        }),
        view().flex_row().flex_wrap().items_start().gap_6().child(list_card(current, groups, editing)).child(
            view().flex_1().min_w(px(520.0)).child(dynamic(move || {
                if let Some(draft) = editing.get() {
                    untracked(|| edit_form(draft, editing, timeframes).into_element())
                } else {
                    untracked(|| detail(current, groups, editing, timeframes, grades).into_element())
                }
            })),
        ),
    )
}

/// The palette of the setups' colour squares: the chart tokens in turn.
fn swatch(index: usize) -> AdaptiveColor {
    let theme = theme();
    [theme.chart_1, theme.chart_2, theme.chart_3, theme.chart_4, theme.chart_5][index % 5]
}

fn square(index: usize) -> impl IntoElement {
    view().size(px(10.0)).rounded(px(3.0)).bg(move || swatch(index))
}

// --- statistics ----------------------------------------------------------------------

/// The closed trades of a snapshot, as the statistics see them.
fn samples(snapshot: &Snapshot) -> Vec<Sample<'_>> {
    snapshot.rows.iter().filter(|row| row.trade.is_closed()).map(|row| Sample { trade: &row.trade, r: row.r }).collect()
}

/// The setup a closed trade's journal links to.
fn linked(snapshot: &Snapshot, sample: &Sample<'_>) -> Option<i64> {
    snapshot.journals.get(&sample.trade.key).and_then(|journal| journal.playbook_id)
}

/// The statistics of every setup that has trades, in playbook order, and of the trades with none.
fn setup_groups(snapshot: &Snapshot) -> Vec<Group> {
    playbook::by_playbook(&samples(snapshot), |x| linked(snapshot, x), &snapshot.playbooks)
}

/// The group of a setup in `groups`, which are named by playbook.
fn group_of<'g>(groups: &'g [Group], setup: &Playbook) -> Option<&'g Group> {
    groups.iter().find(|group| group.label == setup.name)
}

/// The trades of setup `id` by their setup grade, in the order of the grade catalog; trades
/// with no grade come last as "Not graded".
fn grade_groups(snapshot: &Snapshot, id: i64, catalog: &[CatalogItem]) -> Vec<Group> {
    let mine: Vec<Sample<'_>> = samples(snapshot).into_iter().filter(|x| linked(snapshot, x) == Some(id)).collect();
    let mut groups = metrics::breakdown(
        &mine,
        |x| vec![snapshot.journals.get(&x.trade.key).and_then(|j| j.setup_grade.clone()).unwrap_or_default()],
        |key: &String| {
            catalog.iter().find(|item| item.key == *key).map_or_else(
                || if key.is_empty() { t!("playbook.ungraded") } else { key.clone() },
                |item| item.label.clone(),
            )
        },
    );
    let place = |group: &Group| catalog.iter().position(|item| item.label == group.label).unwrap_or(usize::MAX);
    groups.sort_by_key(place);
    groups
}

// --- the list ------------------------------------------------------------------------

fn list_card(
    current: Memo<Option<Playbook>>,
    groups: Memo<Vec<Group>>,
    editing: Signal<Option<Draft>>,
) -> impl IntoElement {
    let store = store();
    view().w(px(320.0)).child(card(
        t!("playbook.list.title"),
        view()
            .gap_3()
            .child(dynamic(move || {
                let snapshot = store.snapshot();
                let groups = groups.get();
                let chosen = current.get().map(|p| p.id);
                let currency = snapshot.currency.clone();
                untracked(|| {
                    if snapshot.playbooks.is_empty() {
                        return empty_state(t!("playbook.list.empty"), t!("playbook.list.empty_hint")).into_element();
                    }
                    let no_setup = playbook::no_setup_label();
                    let unlinked = groups.iter().find(|g| g.label == no_setup).cloned();
                    view()
                        .gap_3()
                        .children(snapshot.playbooks.iter().enumerate().map(|(index, setup)| {
                            setup_item(index, setup, group_of(&groups, setup), &currency, chosen, editing)
                        }))
                        .children(unlinked.map(|group| unlinked_row(&group, &currency)))
                        .into_element()
                })
            }))
            .child(text(t!("playbook.list.unlinked_hint")).text_xs().text_color(move || theme().muted_foreground)),
    ))
}

/// One setup of the list: its name and result, then its counts.
fn setup_item(
    index: usize,
    setup: &Playbook,
    group: Option<&Group>,
    currency: &str,
    chosen: Option<i64>,
    editing: Signal<Option<Draft>>,
) -> impl IntoElement {
    let id = setup.id;
    let net = group.map_or_else(|| "—".to_owned(), |g| format::money(g.net, Sign::Always, currency));
    let counts = counts_line(group);
    list_item(
        t!("playbook.item.label", name = setup.name),
        move || chosen == Some(id),
        move || {
            batch(|| {
                global::<Selected>().set(Selected(Some(id)));
                editing.set(None);
            });
        },
        view()
            .gap_1()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(square(index))
                    .child(
                        text(setup.name.clone())
                            .flex_1()
                            .number_of_lines(1)
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .when(setup.archived, |row| row.child(chip(t!("playbook.archived"), Tone::Neutral)))
                    .child(pnl_text(move || net.clone())),
            )
            .child(text(counts).text_xs().text_color(move || theme().muted_foreground)),
    )
}

/// `42 trades · 62% won · Expectancy +0.58R`.
fn counts_line(group: Option<&Group>) -> String {
    match group {
        None => t!("playbook.counts.none"),
        Some(g) => t!(
            "playbook.counts.some",
            trades = format::trade_count(g.trades),
            win = format!("{:.0}", g.win_rate() * 100.0),
            expectancy = format::r_average(g.avg_r())
        ),
    }
}

/// The trades with no setup: counted, never picked.
fn unlinked_row(group: &Group, currency: &str) -> impl IntoElement + use<> {
    let net = format::money(group.net, Sign::Always, currency);
    view()
        .gap_1()
        .p_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(text(playbook::no_setup_label()).flex_1().text_sm().font_weight(FontWeight::SEMIBOLD))
                .child(pnl_text(move || net.clone())),
        )
        .child(text(counts_line(Some(group))).text_xs().text_color(move || theme().muted_foreground))
}

// --- the chosen setup -----------------------------------------------------------------

fn detail(
    current: Memo<Option<Playbook>>,
    groups: Memo<Vec<Group>>,
    editing: Signal<Option<Draft>>,
    timeframes: Reading<Vec<CatalogItem>>,
    grades: Reading<Vec<CatalogItem>>,
) -> impl IntoElement {
    let store = store();
    let write = writer();
    let notice = signal(None::<String>);
    let grade_rows = computed(move || {
        let catalog = grades.value().map_or_default(|c| c.to_vec());
        current.get().map_or_else(Vec::new, |setup| grade_groups(&store.snapshot(), setup.id, &catalog))
    });
    // The detail is built again whenever the stored setup changes (an edit, an archive, a read
    // after a sync), so it never shows an old copy. The pictures are left out of what is compared:
    // the gallery follows `current` on its own, and building it again would close its viewer and
    // its undo notice each time a picture is added or removed.
    let shown = computed(move || current.get().map(|setup| Playbook { examples: Vec::new(), ..setup }));
    dynamic(move || {
        let setup = shown.get();
        untracked(|| {
            let Some(setup) = setup else { return view().into_element() };
            let id = setup.id;
            view()
                .p_5()
                .gap_5()
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().border)
                .bg(move || theme().card)
                .child(title_row(&setup, current, timeframes, editing, write, notice))
                .child(text(description_line(&setup)).text_sm().text_color(move || theme().muted_foreground))
                .child(stat_row(id, groups))
                .child(rules_columns(&setup))
                .child(
                    view()
                        .gap_3()
                        .child(
                            text(move || {
                                t!(
                                    "playbook.examples.count",
                                    n = current.with(|p| p.as_ref().map_or(0, |p| p.examples.len()))
                                )
                            })
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                        )
                        .child(gallery(
                            t!("playbook.examples"),
                            move || current.with(|p| p.as_ref().map_or_else(Vec::new, |p| p.examples.clone())),
                            move |list| {
                                write.run(move |db| set_examples(db, id, list), move |result| notice.set(result.err()));
                            },
                        )),
                )
                .child(grade_table(grade_rows, &setup.name, groups))
                .child(dynamic(move || notice.get().map(error_note)))
                .into_element()
        })
    })
}

/// Replaces the example pictures of setup `id`.
fn set_examples(db: &mut Db, id: i64, examples: Vec<tradelens_core::attachments::Attachment>) -> Result<(), DbError> {
    let all = db.playbooks()?;
    let Some(at) = all.iter().position(|p| p.id == id) else { return Ok(()) };
    let mut setup = all[at].clone();
    setup.examples = examples;
    save_indexed(db, &setup, at)?;
    Ok(())
}

/// Stores a setup at `position`; the search index is refreshed by the same write.
pub(super) fn save_indexed(db: &mut Db, setup: &Playbook, position: usize) -> Result<i64, DbError> {
    db.save_playbook(setup, i32::try_from(position).unwrap_or(i32::MAX))
}

fn title_row(
    setup: &Playbook,
    current: Memo<Option<Playbook>>,
    timeframes: Reading<Vec<CatalogItem>>,
    editing: Signal<Option<Draft>>,
    write: crate::store::Writer,
    notice: Signal<Option<String>>,
) -> impl IntoElement {
    let archived = setup.archived;
    let frames = setup.timeframes.clone();
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(text(setup.name.clone()).text_xl().font_weight(FontWeight::SEMIBOLD))
                .children(setup.symbols.iter().map(|symbol| chip(symbol.clone(), Tone::Neutral)))
                .child(dynamic(move || {
                    let catalog = timeframes.value();
                    let frames = frames.clone();
                    untracked(|| {
                        view().flex_row().gap_2().children(frames.into_iter().map(|key| {
                            let label = catalog
                                .as_ref()
                                .and_then(|c| c.iter().find(|item| item.key == key))
                                .map_or_else(|| key.to_uppercase(), |item| item.label.clone());
                            chip(label, Tone::Neutral)
                        }))
                    })
                })),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(action_icon(Icon::Edit, t!("common.edit"), Variant::Secondary, move || {
                    // The form opens from the setup as stored now, not from the copy this row was
                    // built with.
                    if let Some(stored) = current.get_untracked() {
                        editing.set(Some(stored));
                    }
                }))
                .child(action_icon(
                    Icon::Download,
                    if archived { t!("playbook.unarchive") } else { t!("playbook.archive") },
                    Variant::Ghost,
                    move || {
                        let Some(mut next) = current.get_untracked() else { return };
                        next.archived = !next.archived;
                        write.run(
                            move |db| {
                                let at = db.playbooks()?.iter().position(|p| p.id == next.id).unwrap_or(0);
                                save_indexed(db, &next, at).map(|_| ())
                            },
                            move |result| notice.set(result.err()),
                        );
                    },
                )),
        )
}

/// The setup's sentence and its usual risk.
fn description_line(setup: &Playbook) -> String {
    let risk = setup.risk_pct.map_or_default(|r| t!("playbook.risk_line", risk = r));
    [setup.description.trim(), risk.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

/// Trades, win rate, expectancy and net of setup `id`.
fn stat_row(id: i64, groups: Memo<Vec<Group>>) -> impl IntoElement {
    let store = store();
    let group = move || {
        let snapshot_playbooks = store.snapshot().playbooks.clone();
        let setup = snapshot_playbooks.into_iter().find(|p| p.id == id);
        setup.and_then(|setup| groups.with(|g| group_of(g, &setup).cloned()))
    };
    let money = move |value: f64| format::money(value, Sign::Always, &store.snapshot().currency);
    view()
        .flex_row()
        .flex_wrap()
        .gap_3()
        .child(stat_tile(
            t!("playbook.stat.trades"),
            move || (group().map_or(0, |g| g.trades).to_string(), Tone::Neutral),
            || t!("playbook.stat.trades_note"),
        ))
        .child(stat_tile(
            t!("playbook.stat.win"),
            move || (group().map_or("—".to_owned(), |g| format!("{:.0}%", g.win_rate() * 100.0)), Tone::Neutral),
            move || group().map_or_else(String::new, |g| format!("{} / {}", g.wins, g.trades)),
        ))
        .child(stat_tile(
            t!("playbook.stat.expectancy"),
            move || {
                let r = group().and_then(|g| g.avg_r());
                (format::r_average(r), r.map_or(Tone::Neutral, |r| if r >= 0.0 { Tone::Profit } else { Tone::Loss }))
            },
            || t!("playbook.stat.expectancy_note"),
        ))
        .child(stat_tile(
            t!("playbook.stat.net"),
            move || {
                let net = group().map_or(0.0, |g| g.net);
                (money(net), if net >= 0.0 { Tone::Profit } else { Tone::Loss })
            },
            || t!("playbook.stat.net_note"),
        ))
}

/// The rules at the left, the checklist at the right.
fn rules_columns(setup: &Playbook) -> impl IntoElement {
    let invalidation: Vec<String> =
        setup.invalidation.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
    view()
        .flex_row()
        .flex_wrap()
        .items_start()
        .gap_6()
        .child(
            view()
                .flex_1()
                .min_w(px(240.0))
                .gap_4()
                .child(numbered(t!("playbook.rules.entry"), &setup.entry_rules))
                .child(numbered(t!("playbook.rules.exit"), &setup.exit_rules))
                .child(numbered(t!("playbook.rules.invalidation"), &invalidation)),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(240.0))
                .gap_2()
                .child(text(t!("playbook.checklist.hint")).text_xs().text_color(move || theme().muted_foreground))
                .children(setup.checklist.iter().map(|item| {
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            view()
                                .size(px(18.0))
                                .items_center()
                                .justify_center()
                                .rounded(px(5.0))
                                .bg(move || theme().primary)
                                .child(icon(Icon::Check, 14.0).text_color(move || theme().primary_foreground)),
                        )
                        .child(text(item.clone()).text_sm().flex_1())
                }))
                .when(setup.checklist.is_empty(), |v| {
                    v.child(text(t!("playbook.checklist.empty")).text_sm().text_color(move || theme().muted_foreground))
                }),
        )
}

/// A heading over lines numbered from 1; nothing at all when there are none.
fn numbered(heading: String, lines: &[String]) -> impl IntoElement {
    view().gap_1().when(!lines.is_empty(), |v| {
        v.child(text(heading).text_xs().text_color(move || theme().muted_foreground)).children(
            lines.iter().enumerate().map(|(at, line)| {
                view()
                    .flex_row()
                    .gap_2()
                    .child(text(format!("{}.", at + 1)).text_sm().text_color(move || theme().muted_foreground))
                    .child(text(line.clone()).text_sm().flex_1())
            }),
        )
    })
}

/// "By setup grade": the setup's trades by the grade they were given.
fn grade_table(rows: Memo<Vec<Group>>, name: &str, groups: Memo<Vec<Group>>) -> impl IntoElement {
    let store = store();
    let name = name.to_owned();
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(text(t!("playbook.grades.title")).text_xs().text_color(move || theme().muted_foreground))
                .child(dynamic({
                    move || {
                        let trades = groups.with(|g| g.iter().find(|x| x.label == name).map_or(0, |x| x.trades));
                        let name = name.clone();
                        untracked(|| {
                            link(t!("playbook.grades.view", trades = format::trade_count(trades)), move || {
                                store.trade_filter.set(Filter { setup: Some(name.clone()), ..Filter::default() });
                                nav::go(Route::Trades);
                            })
                        })
                    }
                })),
        )
        .child(dynamic(move || {
            let groups = rows.get();
            let currency = store.snapshot().currency.clone();
            untracked(|| {
                if groups.is_empty() {
                    return text(t!("playbook.grades.empty"))
                        .text_sm()
                        .text_color(move || theme().muted_foreground)
                        .into_element();
                }
                mini_table(
                    vec![
                        Column::start(t!("playbook.grades.grade")),
                        Column::end(t!("playbook.grades.trades")),
                        Column::end(t!("playbook.grades.win")),
                        Column::end(t!("playbook.grades.expectancy")),
                        Column::end(t!("playbook.grades.net")),
                    ],
                    groups
                        .iter()
                        .map(|g| {
                            let net = format::money(g.net, Sign::Always, &currency);
                            let r = format::r_average(g.avg_r());
                            vec![
                                chip(g.label.clone(), Tone::Primary).into_element(),
                                text(g.trades.to_string()).text_sm().into_element(),
                                text(format!("{:.0}%", g.win_rate() * 100.0)).text_sm().into_element(),
                                pnl_text(move || r.clone()).into_element(),
                                pnl_text(move || net.clone()).into_element(),
                            ]
                        })
                        .collect(),
                )
                .into_element()
            })
        }))
}

// --- the form ---------------------------------------------------------------------------

/// The symbols typed as `XAUUSD, eurusd` as a list: upper case, once each, in the order typed.
fn split_symbols(typed: &str) -> Vec<String> {
    let mut symbols: Vec<String> = Vec::new();
    for symbol in typed.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|s| !s.is_empty()) {
        let symbol = symbol.to_uppercase();
        if !symbols.contains(&symbol) {
            symbols.push(symbol);
        }
    }
    symbols
}

/// Whether `name` is the name of another setup (ignoring case and the spaces around it).
fn name_taken(playbooks: &[Playbook], name: &str, own_id: i64) -> bool {
    let wanted = name.trim().to_lowercase();
    playbooks.iter().any(|p| p.id != own_id && p.name.trim().to_lowercase() == wanted)
}

/// Why a setup cannot take `name`: another setup has it. Made on the UI thread.
fn name_taken_message(name: &str) -> String {
    t!("playbook.name_taken", name = name.trim())
}

/// The name a write found in use by another setup; the words are made where the failure is shown
/// (the write runs on a blocking thread, which cannot read the app's text).
#[derive(Debug)]
struct NameTaken(String);

/// A label over a control.
fn labelled(label: String, control: impl IntoElement) -> impl IntoElement {
    view().gap_1().child(text(label).text_xs().text_color(move || theme().muted_foreground)).child(control)
}

/// What the form edits, one signal per field: typing in one redraws no other.
#[derive(Clone, Copy)]
struct Fields {
    name: Signal<String>,
    description: Signal<String>,
    symbols: Signal<String>,
    frames: Signal<Vec<String>>,
    entry: Signal<Vec<String>>,
    exit: Signal<Vec<String>>,
    invalidation: Signal<String>,
    risk: Signal<Option<f64>>,
    checklist: Signal<Vec<String>>,
}

impl Fields {
    fn of(base: &Playbook) -> Self {
        Self {
            name: signal(base.name.clone()),
            description: signal(base.description.clone()),
            symbols: signal(base.symbols.join(", ")),
            frames: signal(base.timeframes.clone()),
            entry: signal(base.entry_rules.clone()),
            exit: signal(base.exit_rules.clone()),
            invalidation: signal(base.invalidation.clone()),
            risk: signal(base.risk_pct),
            checklist: signal(base.checklist.clone()),
        }
    }

    /// The fields laid over `base`; why not (in words) when the name is empty or another
    /// setup has it.
    fn collect(&self, base: &Playbook, others: &[Playbook]) -> Result<Playbook, String> {
        let name = self.name.get_untracked();
        if name.trim().is_empty() {
            return Err(t!("playbook.name_empty"));
        }
        if name_taken(others, &name, base.id) {
            return Err(name_taken_message(&name));
        }
        Ok(Playbook {
            name: name.trim().to_owned(),
            description: self.description.get_untracked().trim().to_owned(),
            symbols: split_symbols(&self.symbols.get_untracked()),
            timeframes: self.frames.get_untracked(),
            entry_rules: filled(&self.entry.get_untracked()),
            exit_rules: filled(&self.exit.get_untracked()),
            invalidation: self.invalidation.get_untracked().trim().to_owned(),
            risk_pct: self.risk.get_untracked(),
            checklist: filled(&self.checklist.get_untracked()),
            ..base.clone()
        })
    }
}

fn edit_form(base: Draft, editing: Signal<Option<Draft>>, timeframes: Reading<Vec<CatalogItem>>) -> impl IntoElement {
    let store = store();
    let write = writer();
    let id = base.id;
    let fields = Fields::of(&base);
    let error = signal(None::<String>);
    let busy = signal(false);
    let save = move || {
        let setup = match fields.collect(&base, &store.snapshot_now().playbooks) {
            Ok(setup) => setup,
            Err(message) => return error.set(Some(message)),
        };
        error.set(None);
        busy.set(true);
        write.run(
            move |db| {
                let all = db.playbooks()?;
                // The list on screen may not be read yet when Save is pressed: the stored setups
                // decide whether the name is free.
                if name_taken(&all, &setup.name, setup.id) {
                    return Ok(Err(NameTaken(setup.name.clone())));
                }
                let at = all.iter().position(|p| p.id == setup.id).unwrap_or(all.len());
                save_indexed(db, &setup, at).map(Ok::<_, NameTaken>)
            },
            move |result| {
                busy.set(false);
                match result {
                    Ok(Ok(saved)) => batch(|| {
                        global::<Selected>().set(Selected(Some(saved)));
                        editing.set(None);
                    }),
                    Ok(Err(NameTaken(name))) => error.set(Some(name_taken_message(&name))),
                    Err(message) => error.set(Some(message)),
                }
            },
        );
    };
    // The question is the root's task: this form is built again when the data is read again.
    let asker = root_owner();
    let remove = move || {
        confirm_then(
            asker,
            t!("playbook.delete.title"),
            t!("playbook.delete.body"),
            t!("playbook.delete.confirm"),
            move || {
                write.run(
                    move |db| db.delete_playbook(id),
                    move |result| match result {
                        Ok(()) => batch(|| {
                            global::<Selected>().set(Selected(None));
                            editing.set(None);
                        }),
                        Err(message) => error.set(Some(message)),
                    },
                );
            },
        );
    };
    view()
        .p_5()
        .gap_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .child(
            text(if id == 0 { t!("playbook.form.new") } else { t!("playbook.form.edit") })
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD),
        )
        .child(form_fields(fields, timeframes))
        .child(dynamic(move || error.get().map(error_line)))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    view().flex_row().when(id != 0, |v| v.child(action(t!("playbook.delete"), Variant::Ghost, remove))),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(action(t!("common.cancel"), Variant::Ghost, move || editing.set(None)))
                        .child(action_icon_when(
                            Icon::Check,
                            t!("common.save"),
                            Variant::Primary,
                            move || !busy.get(),
                            save,
                        )),
                ),
        )
}

/// The timeframes the setup is traded on, picked from the trader's list, which is kept here too.
fn timeframe_field(frames: Signal<Vec<String>>, timeframes: Reading<Vec<CatalogItem>>) -> impl IntoElement {
    catalog_tags(
        CatalogKind::Timeframe,
        t!("playbook.form.timeframe"),
        t!("playbook.form.timeframe_placeholder"),
        move || timeframes.value().map_or_default(|all| all.to_vec()),
        move || frames.get(),
        move |key, on| {
            frames.update(|keys| {
                keys.retain(|k| k != key);
                if on {
                    keys.push(key.to_owned());
                }
            });
        },
    )
}

/// The fields of the form: the setup's identity, then its rules and checklist.
fn form_fields(fields: Fields, timeframes: Reading<Vec<CatalogItem>>) -> impl IntoElement {
    let Fields { name, description, symbols, frames, entry, exit, invalidation, risk, checklist } = fields;
    view()
        .gap_4()
        .child(labelled(
            t!("playbook.form.name"),
            text_field(
                t!("playbook.form.name_label"),
                None,
                t!("playbook.form.name_placeholder"),
                move || name.get(),
                move |typed| {
                    name.set(typed.to_owned());
                },
            ),
        ))
        .child(labelled(
            t!("playbook.form.description"),
            textarea(
                t!("playbook.form.description"),
                t!("playbook.form.description_placeholder"),
                3,
                move || description.get(),
                move |typed| {
                    description.set(typed.to_owned());
                },
            ),
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_4()
                .child(view().flex_1().min_w(px(240.0)).child(labelled(
                    t!("playbook.form.symbols"),
                    text_field(
                        t!("playbook.form.symbols_label"),
                        None,
                        "XAUUSD, EURUSD",
                        move || symbols.get(),
                        move |typed| {
                            symbols.set(typed.to_owned());
                        },
                    ),
                )))
                .child(labelled(
                    t!("playbook.form.risk"),
                    number_field(
                        t!("playbook.form.risk"),
                        Some("%".into()),
                        NumberRange::between(0.0, 100.0).stepping(0.25),
                        move || risk.get(),
                        move |next| risk.set(next),
                    ),
                )),
        )
        .child(labelled(t!("playbook.form.timeframe"), timeframe_field(frames, timeframes)))
        .child(labelled(
            t!("playbook.form.entry"),
            line_list(
                t!("playbook.form.entry"),
                t!("playbook.form.entry_add"),
                move || entry.get(),
                move |next| entry.set(next),
            ),
        ))
        .child(labelled(
            t!("playbook.form.exit"),
            line_list(
                t!("playbook.form.exit"),
                t!("playbook.form.exit_add"),
                move || exit.get(),
                move |next| exit.set(next),
            ),
        ))
        .child(labelled(
            t!("playbook.form.invalidation"),
            textarea(
                t!("playbook.form.invalidation_label"),
                t!("playbook.form.invalidation_placeholder"),
                3,
                move || invalidation.get(),
                move |typed| {
                    invalidation.set(typed.to_owned());
                },
            ),
        ))
        .child(labelled(
            t!("playbook.form.checklist"),
            line_list(
                t!("playbook.form.checklist_label"),
                t!("playbook.form.checklist_add"),
                move || checklist.get(),
                move |next| checklist.set(next),
            ),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_are_split_upper_cased_and_deduplicated() {
        assert_eq!(split_symbols("xauusd, EURUSD; xauusd  gbpjpy"), ["XAUUSD", "EURUSD", "GBPJPY"]);
        assert_eq!(split_symbols("  , ").len(), 0, "only separators");
    }

    #[test]
    fn a_name_is_taken_by_another_setup_only() {
        let list = [
            Playbook { id: 1, name: "Pullback EMA20".into(), ..Playbook::default() },
            Playbook { id: 2, name: "Breakout".into(), ..Playbook::default() },
        ];
        assert!(name_taken(&list, "  pullback ema20 ", 0));
        assert!(name_taken(&list, "Breakout", 1));
        assert!(!name_taken(&list, "Breakout", 2), "a setup keeps its own name");
        assert!(!name_taken(&list, "Đảo chiều", 0));
    }

    #[test]
    fn the_description_carries_the_risk() {
        let setup = Playbook { description: " Giá tích luỹ. ".into(), risk_pct: Some(1.5), ..Playbook::default() };
        let (with_risk, none) = crate::i18n::with_text(crate::i18n::Lang::Vi, move || {
            (description_line(&setup), description_line(&Playbook::default()))
        });
        assert_eq!(with_risk, "Giá tích luỹ. Risk mặc định 1.5%.");
        assert_eq!(none, "");
    }

    #[test]
    fn counts_read_as_one_line() {
        assert_eq!(crate::i18n::with_text(crate::i18n::Lang::Vi, || counts_line(None)), "Chưa có lệnh");
        let group = Group {
            label: "x".into(),
            trades: 4,
            wins: 3,
            net: 10.0,
            gross_profit: 12.0,
            gross_loss: -2.0,
            total_r: 2.0,
            r_trades: 4,
            r_profit: 2.0,
            r_loss: 0.0,
        };
        let counts = |lang| {
            crate::i18n::with_text(lang, {
                let group = group.clone();
                move || counts_line(Some(&group))
            })
        };
        assert_eq!(counts(crate::i18n::Lang::Vi), "4 lệnh · 75% thắng · Kỳ vọng +0.50R");
        assert_eq!(counts(crate::i18n::Lang::En), "4 trades · 75% won · Expectancy +0.50R");
    }
}
