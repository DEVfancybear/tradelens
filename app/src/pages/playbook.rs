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
        "Playbook",
        "Mỗi setup một bộ quy tắc, checklist và ảnh mẫu, kèm số liệu thật.",
        dynamic(move || {
            (!is_editing.get()).then(|| {
                untracked(|| {
                    action_icon(Icon::Plus, "Thêm playbook", Variant::Primary, move || {
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
/// with no grade come last as "Chưa chấm".
fn grade_groups(snapshot: &Snapshot, id: i64, catalog: &[CatalogItem]) -> Vec<Group> {
    const UNGRADED: &str = "Chưa chấm";
    let mine: Vec<Sample<'_>> = samples(snapshot).into_iter().filter(|x| linked(snapshot, x) == Some(id)).collect();
    let mut groups = metrics::breakdown(
        &mine,
        |x| vec![snapshot.journals.get(&x.trade.key).and_then(|j| j.setup_grade.clone()).unwrap_or_default()],
        |key: &String| {
            catalog.iter().find(|item| item.key == *key).map_or_else(
                || if key.is_empty() { UNGRADED.to_owned() } else { key.clone() },
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
        "Playbook của bạn",
        view()
            .gap_3()
            .child(dynamic(move || {
                let snapshot = store.snapshot();
                let groups = groups.get();
                let chosen = current.get().map(|p| p.id);
                let currency = snapshot.currency.clone();
                untracked(|| {
                    if snapshot.playbooks.is_empty() {
                        return empty_state("Chưa có playbook nào", "Bấm “Thêm playbook” để ghi setup đầu tiên.")
                            .into_element();
                    }
                    let unlinked = groups.iter().find(|g| g.label == "Chưa gắn setup").cloned();
                    view()
                        .gap_3()
                        .children(snapshot.playbooks.iter().enumerate().map(|(index, setup)| {
                            setup_item(index, setup, group_of(&groups, setup), &currency, chosen, editing)
                        }))
                        .children(unlinked.map(|group| unlinked_row(&group, &currency)))
                        .into_element()
                })
            }))
            .child(
                text("“Chưa gắn setup” là các lệnh chưa có playbook — gắn trong nhật ký của từng lệnh.")
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            ),
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
        format!("Playbook {}", setup.name),
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
                    .when(setup.archived, |row| row.child(chip("Đã lưu trữ", Tone::Neutral)))
                    .child(pnl_text(move || net.clone())),
            )
            .child(text(counts).text_xs().text_color(move || theme().muted_foreground)),
    )
}

/// `42 lệnh · 62% thắng · Kỳ vọng +0.58R`.
fn counts_line(group: Option<&Group>) -> String {
    match group {
        None => "Chưa có lệnh".to_owned(),
        Some(g) => {
            format!("{} lệnh · {:.0}% thắng · Kỳ vọng {}", g.trades, g.win_rate() * 100.0, format::r_average(g.avg_r()))
        }
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
                .child(text("Chưa gắn setup").flex_1().text_sm().font_weight(FontWeight::SEMIBOLD))
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
    when(
        move || current.with(Option::is_some),
        move || {
            untracked(|| {
                let Some(setup) = current.get() else { return view().into_element() };
                let id = setup.id;
                let name = setup.name.clone();
                view()
                    .p_5()
                    .gap_5()
                    .rounded(move || theme().radius)
                    .border_1()
                    .border_color(move || theme().border)
                    .bg(move || theme().card)
                    .child(title_row(&setup, timeframes, editing, write, notice))
                    .child(dynamic({
                        let setup = setup.clone();
                        move || {
                            let line = description_line(&setup);
                            untracked(|| text(line).text_sm().text_color(move || theme().muted_foreground))
                        }
                    }))
                    .child(stat_row(id, groups))
                    .child(rules_columns(&setup))
                    .child(
                        view()
                            .gap_3()
                            .child(
                                text(format!("Ảnh mẫu · {}", setup.examples.len()))
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(gallery(
                                "Ảnh mẫu",
                                move || current.with(|p| p.as_ref().map_or_else(Vec::new, |p| p.examples.clone())),
                                move |list| {
                                    write.run(
                                        move |db| set_examples(db, id, list),
                                        move |result| notice.set(result.err()),
                                    );
                                },
                            )),
                    )
                    .child(grade_table(grade_rows, &name, groups))
                    .child(dynamic(move || notice.get().map(error_note)))
                    .into_element()
            })
        },
    )
    .otherwise(|| view().into_element())
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
    timeframes: Reading<Vec<CatalogItem>>,
    editing: Signal<Option<Draft>>,
    write: crate::store::Writer,
    notice: Signal<Option<String>>,
) -> impl IntoElement {
    let for_edit = setup.clone();
    let archived = setup.archived;
    let archive_target = setup.clone();
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
                .child(action_icon(Icon::Edit, "Sửa", Variant::Secondary, move || {
                    editing.set(Some(for_edit.clone()));
                }))
                .child(action_icon(
                    Icon::Download,
                    if archived { "Bỏ lưu trữ" } else { "Lưu trữ" },
                    Variant::Ghost,
                    move || {
                        let mut next = archive_target.clone();
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
    let risk = setup.risk_pct.map_or_default(|r| format!("Risk mặc định {r}%."));
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
            "Lệnh",
            move || (group().map_or(0, |g| g.trades).to_string(), Tone::Neutral),
            || "tất cả thời gian".to_owned(),
        ))
        .child(stat_tile(
            "Thắng",
            move || (group().map_or("—".to_owned(), |g| format!("{:.0}%", g.win_rate() * 100.0)), Tone::Neutral),
            move || group().map_or_else(String::new, |g| format!("{} / {}", g.wins, g.trades)),
        ))
        .child(stat_tile(
            "Kỳ vọng",
            move || {
                let r = group().and_then(|g| g.avg_r());
                (format::r_average(r), r.map_or(Tone::Neutral, |r| if r >= 0.0 { Tone::Profit } else { Tone::Loss }))
            },
            || "trung bình mỗi lệnh".to_owned(),
        ))
        .child(stat_tile(
            "Net",
            move || {
                let net = group().map_or(0.0, |g| g.net);
                (money(net), if net >= 0.0 { Tone::Profit } else { Tone::Loss })
            },
            || "sau phí".to_owned(),
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
                .child(numbered("Điều kiện vào", &setup.entry_rules))
                .child(numbered("Thoát lệnh", &setup.exit_rules))
                .child(numbered("Huỷ setup khi", &invalidation)),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(240.0))
                .gap_2()
                .child(
                    text("Checklist — hiện khi chọn playbook này lúc vào lệnh")
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
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
                    v.child(text("Chưa có mục nào.").text_sm().text_color(move || theme().muted_foreground))
                }),
        )
}

/// A heading over lines numbered from 1; nothing at all when there are none.
fn numbered(heading: &'static str, lines: &[String]) -> impl IntoElement {
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

/// "Theo hạng setup": the setup's trades by the grade they were given.
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
                .child(text("Theo hạng setup").text_xs().text_color(move || theme().muted_foreground))
                .child(dynamic({
                    move || {
                        let trades = groups.with(|g| g.iter().find(|x| x.label == name).map_or(0, |x| x.trades));
                        let name = name.clone();
                        untracked(|| {
                            link(format!("Xem {trades} lệnh"), move || {
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
                    return text("Chưa có lệnh nào gắn setup này.")
                        .text_sm()
                        .text_color(move || theme().muted_foreground)
                        .into_element();
                }
                mini_table(
                    vec![
                        Column::start("Hạng"),
                        Column::end("Lệnh"),
                        Column::end("Thắng"),
                        Column::end("Kỳ vọng"),
                        Column::end("Net"),
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

/// Why a setup cannot take `name` (in Vietnamese): another setup has it.
fn name_taken_message(name: &str) -> String {
    format!("Đã có playbook tên “{}”.", name.trim())
}

/// A label over a control.
fn labelled(label: &'static str, control: impl IntoElement) -> impl IntoElement {
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

    /// The fields laid over `base`; why not (in Vietnamese) when the name is empty or another
    /// setup has it.
    fn collect(&self, base: &Playbook, others: &[Playbook]) -> Result<Playbook, String> {
        let name = self.name.get_untracked();
        if name.trim().is_empty() {
            return Err("Tên playbook không được để trống.".to_owned());
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
                    return Ok(Err(name_taken_message(&setup.name)));
                }
                let at = all.iter().position(|p| p.id == setup.id).unwrap_or(all.len());
                save_indexed(db, &setup, at).map(Ok)
            },
            move |result| {
                busy.set(false);
                match result.flatten() {
                    Ok(saved) => batch(|| {
                        global::<Selected>().set(Selected(Some(saved)));
                        editing.set(None);
                    }),
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
            "Xoá playbook?",
            "Các lệnh đã gắn setup này sẽ thành “Setup đã xoá” trong thống kê. Không hoàn tác được.",
            "Xoá hẳn",
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
        .child(text(if id == 0 { "Playbook mới" } else { "Sửa playbook" }).text_xl().font_weight(FontWeight::SEMIBOLD))
        .child(form_fields(fields, timeframes))
        .child(dynamic(move || error.get().map(error_line)))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(view().flex_row().when(id != 0, |v| v.child(action("Xoá playbook", Variant::Ghost, remove))))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(action("Huỷ", Variant::Ghost, move || editing.set(None)))
                        .child(action_icon_when(Icon::Check, "Lưu", Variant::Primary, move || !busy.get(), save)),
                ),
        )
}

/// The timeframes the setup is traded on, picked from the trader's list, which is kept here too.
fn timeframe_field(frames: Signal<Vec<String>>, options: Memo<Vec<(String, String)>>) -> impl IntoElement {
    catalog_tags(
        CatalogKind::Timeframe,
        "Khung thời gian",
        "+ Thêm khung (vd. M30), rồi Enter",
        move || options.get(),
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
    let options = super::missed::catalog_options(timeframes);
    view()
        .gap_4()
        .child(labelled(
            "Tên",
            text_field(
                "Tên playbook",
                None,
                "Breakout phiên Á",
                move || name.get(),
                move |typed| {
                    name.set(typed.to_owned());
                },
            ),
        ))
        .child(labelled(
            "Mô tả",
            textarea(
                "Mô tả",
                "Setup này là gì, trong một vài câu",
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
                    "Symbol áp dụng (bỏ trống: mọi symbol)",
                    text_field(
                        "Symbol áp dụng",
                        None,
                        "XAUUSD, EURUSD",
                        move || symbols.get(),
                        move |typed| {
                            symbols.set(typed.to_owned());
                        },
                    ),
                )))
                .child(labelled(
                    "Risk mặc định",
                    number_field(
                        "Risk mặc định",
                        Some("%"),
                        NumberRange::between(0.0, 100.0).stepping(0.25),
                        move || risk.get(),
                        move |next| risk.set(next),
                    ),
                )),
        )
        .child(labelled("Khung thời gian", timeframe_field(frames, options)))
        .child(labelled(
            "Điều kiện vào",
            line_list("Điều kiện vào", "Thêm điều kiện vào", move || entry.get(), move |next| entry.set(next)),
        ))
        .child(labelled(
            "Thoát lệnh",
            line_list("Thoát lệnh", "Thêm quy tắc thoát", move || exit.get(), move |next| exit.set(next)),
        ))
        .child(labelled(
            "Huỷ setup khi (mỗi dòng một ý)",
            textarea(
                "Huỷ setup khi",
                "Có tin đỏ trong 30 phút",
                3,
                move || invalidation.get(),
                move |typed| {
                    invalidation.set(typed.to_owned());
                },
            ),
        ))
        .child(labelled(
            "Checklist trước khi vào lệnh",
            line_list("Mục checklist", "Thêm mục checklist", move || checklist.get(), move |next| checklist.set(next)),
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
        assert_eq!(description_line(&setup), "Giá tích luỹ. Risk mặc định 1.5%.");
        assert_eq!(description_line(&Playbook::default()), "");
    }

    #[test]
    fn counts_read_as_one_line() {
        assert_eq!(counts_line(None), "Chưa có lệnh");
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
        assert_eq!(counts_line(Some(&group)), "4 lệnh · 75% thắng · Kỳ vọng +0.50R");
    }
}
