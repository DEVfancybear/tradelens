//! Nguồn dữ liệu: kết nối MT5 trực tiếp, import báo cáo MT5, file JSON của script Python và CSV Notion.
//! Design: `docs/design/screens/data-sources.html`. The import of MT5 reports and of a Notion
//! export (automatic: no column mapping), how to export the history with the Python script, and
//! the log of what was imported.

use std::{path::PathBuf, rc::Rc};

use chrono::{DateTime, Local};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_cloud::notion::{NotionDatabase, TOKEN_SECRET};
use tradelens_core::{
    Db, DbError,
    db::ImportLogEntry,
    import::{self, notion},
    time::naive,
};

use crate::{
    cloud as app_cloud,
    format::{self, Sign},
    store::{Writer, query, store, writer},
    ui::{
        chips::{Tone, chip, figure_text},
        controls::{Choice, Variant, action_icon_when, action_when, select_field, text_field},
        files::{Accept, Pick, dropzone},
        parts::{card, empty_state, error_line, page},
        properties::kind_name,
    },
};

/// What the report importer takes: MT5's HTML and XLSX, and the JSON the Python script writes.
const REPORT_FILES: Accept =
    Accept { name: || t!("data_sources.accept.reports"), extensions: &["html", "htm", "xlsx", "json"] };

/// What the Notion importer takes: the CSV, or the zip Notion writes with the images beside it.
const NOTION_FILES: Accept = Accept { name: || t!("data_sources.accept.notion"), extensions: &["csv", "zip"] };

/// What the trader runs, in `python/`, to write the history of the open terminal to a file.
const PYTHON_COMMAND: &str = "python tradelens_sync.py --days 365 --out deals.json";

/// How many skipped rows a Notion summary spells out; the rest are counted.
const ISSUES_SHOWN: usize = 10;

/// How many lines of the import log the page keeps.
const LOG_LINES: u32 = 12;

/// The choice of the account Notion trades go to when the trader does not pick one.
const DEFAULT_ACCOUNT: &str = "";

pub(crate) fn data_sources() -> impl IntoElement {
    page(
        t!("nav.data_sources"),
        t!("data_sources.summary"),
        view()
            .flex_row()
            .flex_wrap()
            .items_start()
            .gap_6()
            .child(
                view()
                    .flex_1()
                    .min_w(px(560.0))
                    .gap_6()
                    .child(super::mt5_connect::mt5_card())
                    .child(report_import_card())
                    .child(notion_import_card()),
            )
            .child(view().w(px(420.0)).gap_6().child(python_card()).child(import_log_card())),
    )
}

/// How to export the history of the open terminal with the Python script.
fn python_card() -> impl IntoElement {
    card(
        t!("data_sources.python.title"),
        view()
            .gap_3()
            .child(text(t!("data_sources.python.body")).text_sm().text_color(move || theme().muted_foreground))
            .child(
                view()
                    .gap_1()
                    .p_3()
                    .rounded(move || theme().radius)
                    .bg(move || theme().background)
                    .border_1()
                    .border_color(move || theme().border)
                    .child(figure_text(|| "pip install -r requirements.txt".to_owned()).text_xs())
                    .child(figure_text(|| PYTHON_COMMAND.to_owned()).text_xs()),
            ),
    )
}

// --- importing an MT5 report -------------------------------------------------------------------------

/// What importing one file did, for the line under the drop zone.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileOutcome {
    name: String,
    /// What the file added, or why it could not be imported (in words).
    result: Result<Imported, String>,
}

/// What importing one report added, as numbers: the import runs on a blocking thread, which cannot
/// make the app's text, so [`outcome_line`] says it where it is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Imported {
    inserted: usize,
    updated: usize,
    existing: usize,
    login: String,
}

/// Imports each file into the database, one by one: a file that cannot be read does not stop the
/// others.
fn import_reports(db: &mut Db, paths: &[PathBuf]) -> Vec<FileOutcome> {
    paths
        .iter()
        .map(|path| FileOutcome {
            name: path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
            result: import_report(db, path),
        })
        .collect()
}

fn import_report(db: &mut Db, path: &std::path::Path) -> Result<Imported, String> {
    let (format, batch) = import::read_file(path).map_err(|error| error.to_string())?;
    let report = db.upsert_batch(&batch, format.source()).map_err(|error| error.to_string())?;
    Ok(Imported {
        inserted: report.inserted,
        updated: report.updated,
        existing: batch.deals.len().saturating_sub(report.inserted + report.updated),
        login: batch.account.login,
    })
}

fn report_import_card() -> impl IntoElement {
    let write = writer();
    let results = signal(Vec::<FileOutcome>::new());
    let busy = signal(false);
    card(
        t!("data_sources.report.title"),
        view()
            .gap_3()
            .child(text(t!("data_sources.report.body")).text_sm().text_color(move || theme().muted_foreground))
            .child(dropzone(
                t!("data_sources.report.drop"),
                t!("data_sources.report.drop_hint"),
                REPORT_FILES,
                Pick::Many,
                move |paths: Vec<PathBuf>| {
                    busy.set(true);
                    write.run(
                        move |db| Ok(import_reports(db, &paths)),
                        move |outcome| {
                            busy.set(false);
                            results.set(outcome.unwrap_or_else(|message| {
                                vec![FileOutcome { name: String::new(), result: Err(message) }]
                            }));
                        },
                    );
                },
            ))
            .child(dynamic(move || {
                busy.get().then(|| {
                    text(t!("data_sources.report.importing")).text_sm().text_color(move || theme().muted_foreground)
                })
            }))
            .child(dynamic(move || {
                let lines = results.get();
                untracked(|| view().gap_1().children(lines.iter().map(outcome_line)))
            })),
    )
}

/// What a report added, in a sentence.
fn imported_text(imported: &Imported) -> String {
    t!(
        "data_sources.report.result",
        inserted = imported.inserted,
        updated = imported.updated,
        existing = imported.existing,
        login = imported.login
    )
}

/// A line under the drop zone, the failure in the loss colour.
fn outcome_line(outcome: &FileOutcome) -> impl IntoElement {
    let (line, failed) = match &outcome.result {
        Ok(imported) => (format!("{}: {}", outcome.name, imported_text(imported)), false),
        Err(reason) if outcome.name.is_empty() => (reason.clone(), true),
        Err(reason) => (format!("{}: {reason}", outcome.name), true),
    };
    text(line).text_sm().text_color(move || if failed { crate::ui::theme::loss() } else { theme().foreground })
}

// --- importing a Notion export --------------------------------------------------------------------------

/// The Notion data that was read, waiting for the trader to press "Import".
#[derive(Debug, Clone)]
struct Preview {
    file: String,
    plan: notion::NotionPlan,
    /// The zip the images are read from, when the plan came from one.
    archive: Option<notion::NotionArchive>,
}

/// The state both ways in (a file, the API) share: what was read, what was stored, what went wrong.
#[derive(Clone, Copy)]
struct Flow {
    preview: Signal<Option<Rc<Preview>>>,
    report: Signal<Option<Rc<notion::NotionImportReport>>>,
    failure: Signal<Option<String>>,
    busy: Signal<bool>,
    /// The account the trades go to, as the choice's key.
    account: Signal<String>,
}

impl Flow {
    fn new() -> Self {
        Self {
            preview: signal_with(None, SignalEquals::Never),
            report: signal_with(None, SignalEquals::Never),
            failure: signal(None),
            busy: signal(false),
            account: signal(DEFAULT_ACCOUNT.to_owned()),
        }
    }

    /// Shows what was read as the preview, or why it could not be.
    fn read(self, what: Result<Preview, String>) {
        batch(|| {
            self.busy.set(false);
            self.report.set(None);
            match what {
                Ok(preview) => {
                    self.failure.set(None);
                    self.account.set(DEFAULT_ACCOUNT.to_owned());
                    self.preview.set(Some(Rc::new(preview)));
                }
                Err(message) => {
                    self.preview.set(None);
                    self.failure.set(Some(message));
                }
            }
        });
    }

    /// Reads the file dropped on the zone (a CSV, or the zip with its images).
    fn read_file(self, paths: Vec<PathBuf>) {
        let Some(path) = paths.into_iter().next() else { return };
        let file = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        self.busy.set(true);
        spawn(async move {
            let read = blocking(move || notion::read_source(&path)).await;
            self.read(match read {
                Ok(Ok((plan, archive))) => Ok(Preview { file, plan, archive }),
                Ok(Err(error)) => Err(format!("{file}: {error}")),
                Err(error) => Err(t!("data_sources.notion.read_stopped", error = error)),
            });
        });
    }

    /// Stores what the preview shows: the images out of the zip first (one copy of each picture, in the
    /// data folder), then the trades and their journals, all in one write.
    fn import(self, write: Writer, attachments: Option<PathBuf>) {
        let Some(shown) = self.preview.get_untracked() else { return };
        let mut plan = shown.plan.clone();
        let archive = shown.archive.clone();
        let chosen = self.account.get_untracked().parse::<i64>().ok();
        self.busy.set(true);
        self.failure.set(None);
        write.run(
            move |db| {
                if let Some(id) = chosen
                    && let Some(target) = db.accounts()?.into_iter().find(|a| a.id == id)
                {
                    plan.use_account(&target);
                }
                let images = match (&archive, &attachments) {
                    (Some(archive), Some(folder)) => Some(
                        archive
                            .attach_images(&mut plan, folder, chrono::Utc::now().timestamp_millis())
                            .map_err(|error| DbError::Invalid(error.to_string()))?,
                    ),
                    _ => None,
                };
                let mut done = db.import_notion(&plan)?;
                if let Some(counts) = images {
                    done.images_attached = counts.attached;
                    done.images_missing = counts.missing;
                }
                Ok(done)
            },
            move |result| {
                self.busy.set(false);
                match result {
                    Ok(done) => batch(|| {
                        self.preview.set(None);
                        self.report.set(Some(Rc::new(done)));
                    }),
                    Err(message) => self.failure.set(Some(message)),
                }
            },
        );
    }
}

fn notion_import_card() -> impl IntoElement {
    let store = store();
    let write = writer();
    let attachments = data_dir().map(|dir| dir.join("attachments"));
    let flow = Flow::new();
    let import_now: Rc<dyn Fn()> = Rc::new(move || flow.import(write, attachments.clone()));
    card(
        t!("data_sources.notion.title"),
        view()
            .gap_3()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(chip(t!("data_sources.notion.auto"), Tone::Primary))
                    .child(
                        text(t!("data_sources.notion.body"))
                            .text_sm()
                            .flex_1()
                            .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(dropzone(
                t!("data_sources.notion.drop"),
                t!("data_sources.notion.drop_hint"),
                NOTION_FILES,
                Pick::One,
                move |paths| flow.read_file(paths),
            ))
            .child(notion_api_section(flow))
            .child(dynamic(move || {
                flow.busy.get().then(|| {
                    text(t!("data_sources.notion.processing")).text_sm().text_color(move || theme().muted_foreground)
                })
            }))
            .child(dynamic(move || flow.failure.get().map(error_line)))
            .child(dynamic(move || {
                let shown = flow.preview.get()?;
                let import_now = Rc::clone(&import_now);
                Some(untracked(|| preview_view(&shown, store, flow.account, flow.busy, import_now).into_element()))
            }))
            .child(dynamic(move || {
                let done = flow.report.get()?;
                Some(untracked(|| summary_view(&done).into_element()))
            })),
    )
}

/// Why a Notion call failed. It is made on a blocking thread, which cannot make the app's text;
/// [`NotionFailure::text`] says it on the UI thread.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NotionFailure {
    /// The call needs a token and none is kept.
    NoToken,
    /// Words that were already made (the credential store's, the API's).
    Words(String),
    /// The working thread stopped before it answered.
    Stopped(String),
}

impl NotionFailure {
    fn text(&self) -> String {
        match self {
            Self::NoToken => t!("data_sources.api.no_token"),
            Self::Words(words) => words.clone(),
            Self::Stopped(error) => t!("data_sources.api.stopped", error = error),
        }
    }
}

/// What the API section of the card holds: the token, the databases shared with the integration and
/// the one chosen.
#[derive(Clone, Copy)]
struct Api {
    flow: Flow,
    state: Signal<app_cloud::State>,
    /// The token as typed, until it is kept.
    token: Signal<String>,
    has_token: Signal<bool>,
    databases: Signal<Vec<NotionDatabase>>,
    chosen: Signal<String>,
    /// The list was asked for, so that "no database" can be told from "not asked yet".
    listed: Signal<bool>,
    /// What a button asked for; it runs once the settings are read (at once when they are).
    asked: Signal<Option<Ask>>,
    /// The section's owner: what a button asked for runs under it, not under the effect that
    /// hears the ask, whose tasks end when it runs again.
    owner: Owner,
}

/// What a button of the API section asks of the Notion API.
#[derive(Debug, Clone, PartialEq)]
enum Ask {
    /// List the databases, keeping the given token first.
    List(Option<String>),
    Preview,
    Forget,
}

impl Api {
    fn env(self) -> app_cloud::Env {
        self.state.with(|s| s.env.clone())
    }

    /// Records what a button asked for. It is not run before the settings are read: until then the
    /// environment is the default one, which keeps a token in the operating system's credential
    /// store and talks to the real Notion instead of the demo or the server a setting names.
    fn ask(self, what: Ask) {
        self.asked.set(Some(what));
    }

    fn run(self, what: Ask) {
        match what {
            Ask::List(token) => self.list(token),
            Ask::Preview => self.preview_chosen(),
            Ask::Forget => self.forget(),
        }
    }

    /// Asks whether a token is kept already (the demo keeps it in memory).
    fn look_for_token(self) {
        let env = self.env();
        spawn(async move {
            let found = blocking(move || env.load_secret(TOKEN_SECRET)).await;
            self.has_token.set(matches!(found, Ok(Ok(Some(_)))));
        });
    }

    /// Lists the databases shared with the integration: with `new_token` after keeping it, else with
    /// the kept one.
    fn list(self, new_token: Option<String>) {
        let env = self.env();
        self.flow.busy.set(true);
        self.flow.failure.set(None);
        spawn(async move {
            let result = blocking(move || -> Result<Vec<NotionDatabase>, NotionFailure> {
                let token = match new_token {
                    Some(token) => {
                        env.save_secret(TOKEN_SECRET, &token).map_err(NotionFailure::Words)?;
                        token
                    }
                    None => {
                        env.load_secret(TOKEN_SECRET).map_err(NotionFailure::Words)?.ok_or(NotionFailure::NoToken)?
                    }
                };
                env.notion_client(&token).databases().map_err(|error| NotionFailure::Words(error.to_string()))
            })
            .await
            .unwrap_or_else(|error| Err(NotionFailure::Stopped(error.to_string())));
            batch(|| {
                self.flow.busy.set(false);
                match result {
                    Ok(found) => {
                        self.has_token.set(true);
                        self.token.set(String::new());
                        self.chosen.set(found.first().map_or_default(|d| d.id.clone()));
                        self.listed.set(true);
                        self.databases.set(found);
                    }
                    Err(failure) => self.flow.failure.set(Some(failure.text())),
                }
            });
        });
    }

    /// Reads the chosen database into the preview.
    fn preview_chosen(self) {
        let id = self.chosen.get_untracked();
        let title = self.databases.with_untracked(|all| all.iter().find(|d| d.id == id).map(|d| d.title.clone()));
        let env = self.env();
        self.flow.busy.set(true);
        self.flow.failure.set(None);
        spawn(async move {
            let result = blocking(move || -> Result<notion::NotionPlan, NotionFailure> {
                let token =
                    env.load_secret(TOKEN_SECRET).map_err(NotionFailure::Words)?.ok_or(NotionFailure::NoToken)?;
                env.notion_client(&token).plan(&id).map_err(|error| NotionFailure::Words(error.to_string()))
            })
            .await
            .unwrap_or_else(|error| Err(NotionFailure::Stopped(error.to_string())));
            self.flow.read(result.map_err(|failure| failure.text()).map(|plan| Preview {
                file: t!("data_sources.api.file_name", title = title.unwrap_or_default()),
                plan,
                archive: None,
            }));
        });
    }

    /// Forgets the kept token and the list that came with it.
    fn forget(self) {
        let env = self.env();
        spawn(async move {
            match blocking(move || env.clear_secret(TOKEN_SECRET)).await {
                Ok(Ok(())) => batch(|| {
                    self.has_token.set(false);
                    self.listed.set(false);
                    self.databases.set(Vec::new());
                    self.chosen.set(String::new());
                }),
                Ok(Err(message)) => self.flow.failure.set(Some(message)),
                Err(error) => self.flow.failure.set(Some(NotionFailure::Stopped(error.to_string()).text())),
            }
        });
    }
}

/// Importing from Notion without exporting a file: the integration's token (kept in the operating
/// system's credential store), the databases shared with it, and the one chosen, read into the same
/// preview as a file.
fn notion_api_section(flow: Flow) -> impl IntoElement {
    let api = Api {
        flow,
        state: app_cloud::cloud().state,
        token: signal(String::new()),
        has_token: signal(false),
        databases: signal(Vec::new()),
        chosen: signal(String::new()),
        listed: signal(false),
        asked: signal(None),
        owner: Owner::current(),
    };
    // A token may be kept already: asked once the settings are read.
    effect(move || {
        if api.state.with(|s| s.ready) {
            untracked(|| api.look_for_token());
        }
    });
    effect(move || {
        if !api.state.with(|s| s.ready) {
            return;
        }
        if let Some(what) = api.asked.get() {
            untracked(|| {
                api.asked.set(None);
                api.owner.run(move || api.run(what));
            });
        }
    });
    view()
        .gap_3()
        .p_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .child(text(t!("data_sources.api.title")).text_sm().font_weight(FontWeight::MEDIUM))
        .child(text(t!("data_sources.api.body")).text_xs().text_color(move || theme().muted_foreground))
        .child(text_field(
            t!("data_sources.api.token"),
            None,
            t!("data_sources.api.token_placeholder"),
            move || api.token.get(),
            move |value| api.token.set(value.to_owned()),
        ))
        .child(api_buttons(api))
        .child(databases_list(api))
}

fn api_buttons(api: Api) -> impl IntoElement {
    let idle = move || !api.flow.busy.get();
    view()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .child(action_when(
            t!("data_sources.api.save"),
            Variant::Secondary,
            move || !api.token.get().trim().is_empty() && idle(),
            move || api.ask(Ask::List(Some(api.token.get_untracked().trim().to_owned()))),
        ))
        .child(action_when(
            t!("data_sources.api.reload"),
            Variant::Ghost,
            move || api.has_token.get() && idle(),
            move || api.ask(Ask::List(None)),
        ))
        .child(action_when(
            t!("data_sources.api.forget"),
            Variant::Ghost,
            move || api.has_token.get() && idle(),
            move || api.ask(Ask::Forget),
        ))
}

/// The databases the integration can see, to choose one from.
fn databases_list(api: Api) -> impl IntoElement {
    dynamic(move || {
        let found = api.databases.get();
        let asked = api.listed.get();
        untracked(|| {
            if found.is_empty() {
                return view()
                    .children(asked.then(|| {
                        text(t!("data_sources.api.no_database")).text_xs().text_color(crate::ui::theme::warning)
                    }))
                    .into_element();
            }
            let choices: Vec<Choice> = found.iter().map(|d| Choice::new(d.id.clone(), d.title.clone())).collect();
            view()
                .gap_2()
                .child(select_field(
                    t!("data_sources.api.database"),
                    move || choices.clone(),
                    move || SharedString::from(api.chosen.get()),
                    move |value| api.chosen.set(value.to_string()),
                ))
                .child(view().flex_row().child(action_when(
                    t!("data_sources.api.preview"),
                    Variant::Primary,
                    move || !api.flow.busy.get() && !api.chosen.get().is_empty(),
                    move || api.ask(Ask::Preview),
                )))
                .into_element()
        })
    })
}

/// The period the file covers and its total, in a line; `None` when it has no dated trade.
fn period_line(plan: &notion::NotionPlan) -> Option<String> {
    plan.period().map(|(first, last)| {
        let total = if plan.is_in_r() {
            let (wins, losses, even) = plan.verdicts();
            t!(
                "data_sources.preview.total_r",
                r = format::r_text(Some(plan.total_r())),
                wins = wins,
                losses = losses,
                even = even
            )
        } else {
            t!(
                "data_sources.preview.total_money",
                net = format::money(plan.net_total(), Sign::Always, &plan.account.currency)
            )
        };
        t!(
            "data_sources.preview.period",
            from = naive(first).format("%d/%m/%Y"),
            to = naive(last).format("%d/%m/%Y"),
            total = total
        )
    })
}

/// What each column was taken for: found from what the column holds, shown so it can be checked.
fn recognised_columns(plan: &notion::NotionPlan) -> Vec<String> {
    let mut fields: Vec<String> = plan
        .mapping
        .columns
        .iter()
        .map(|(field, column)| t!("data_sources.preview.column", column = column, field = field.label()))
        .collect();
    if let Some(outcome) = &plan.outcome {
        let values: Vec<String> =
            outcome.values.iter().map(|(value, verdict)| format!("{value} = {}", verdict.label())).collect();
        fields.push(t!("data_sources.preview.outcome", column = outcome.column, values = values.join(", ")));
    }
    fields
}

/// The short preview of a Notion file: what was found, what will be created, what is odd, and
/// the one button that stores it.
fn preview_view(
    shown: &Preview,
    store: crate::store::Store,
    account: Signal<String>,
    busy: Signal<bool>,
    import_now: Rc<dyn Fn()>,
) -> impl IntoElement {
    let plan = &shown.plan;
    let accounts_of_plan = plan.account.login.clone();
    let count = plan.rows.len();
    let found = t!(
        "data_sources.preview.found",
        file = shown.file,
        rows = plan.total_rows,
        count = format::trade_count(count),
        skipped = plan.issues.len()
    );
    let period = period_line(plan);
    let images = shown.archive.as_ref().map(|archive| {
        let counts = archive.count(plan);
        t!("data_sources.preview.images", found = counts.attached, missing = counts.missing)
    });
    let fields = recognised_columns(plan);
    let properties: Vec<String> = plan
        .properties
        .iter()
        .map(|p| {
            t!(
                "data_sources.preview.property",
                label = p.label,
                kind = kind_name(p.kind),
                trades = format::trade_count(p.filled)
            )
        })
        .collect();
    let muted = |line: String| text(line).text_xs().text_color(move || theme().muted_foreground);
    view()
        .gap_3()
        .p_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().background)
        .child(text(found).text_sm().font_weight(FontWeight::MEDIUM))
        .children(period.map(&muted))
        .child(section(
            t!("data_sources.preview.columns_title"),
            if fields.is_empty() { vec![t!("data_sources.preview.none")] } else { fields },
        ))
        .children(images.map(&muted))
        .when(!properties.is_empty(), |v| v.child(section(t!("data_sources.preview.properties_title"), properties)))
        .when(!plan.ignored_columns.is_empty(), |v| {
            v.child(muted(t!(
                "data_sources.preview.ignored",
                n = plan.ignored_columns.len(),
                columns = plan.ignored_columns.join(", ")
            )))
        })
        .when(!plan.warnings.is_empty(), |v| {
            v.child(view().gap_1().children(
                plan.warnings.iter().map(|w| text(w.clone()).text_xs().text_color(crate::ui::theme::warning)),
            ))
        })
        .when(!plan.issues.is_empty(), |v| {
            let mut lines: Vec<String> = plan.issues.iter().take(ISSUES_SHOWN).map(ToString::to_string).collect();
            if plan.issues.len() > ISSUES_SHOWN {
                lines.push(t!("data_sources.preview.more_rows", n = plan.issues.len() - ISSUES_SHOWN));
            }
            v.child(section(t!("data_sources.preview.skipped_title"), lines))
        })
        .child(select_field(
            t!("data_sources.preview.account"),
            move || {
                let mut choices = vec![Choice::new(DEFAULT_ACCOUNT, t!("data_sources.preview.default_account"))];
                choices.extend(
                    store
                        .snapshot()
                        .accounts
                        .iter()
                        .filter(|a| a.login != accounts_of_plan)
                        .map(|a| Choice::new(a.id.to_string(), a.title())),
                );
                choices
            },
            move || SharedString::from(account.get()),
            move |value| account.set(value.to_string()),
        ))
        .child(view().flex_row().justify_end().child(action_icon_when(
            crate::ui::icons::Icon::Download,
            t!("data_sources.preview.import", trades = format::trade_count(count)),
            Variant::Primary,
            move || count > 0 && !busy.get(),
            move || import_now(),
        )))
}

/// A heading over a few lines of detail.
fn section(heading: String, lines: Vec<String>) -> impl IntoElement {
    view()
        .gap_1()
        .child(text(heading).text_xs().text_color(move || theme().muted_foreground))
        .children(lines.into_iter().map(|line| text(line).text_sm()))
}

/// What an import stored, in lines.
fn summary_lines(report: &notion::NotionImportReport) -> Vec<String> {
    let mut lines = vec![t!(
        "data_sources.summary.imported",
        created = report.created,
        updated = report.updated,
        unchanged = report.unchanged
    )];
    if report.matched_mt5 > 0 {
        lines.push(t!("data_sources.summary.matched", n = report.matched_mt5));
    }
    if !report.properties_created.is_empty() {
        let created: Vec<String> =
            report.properties_created.iter().map(|p| format!("{} ({})", p.label, kind_name(p.kind))).collect();
        lines.push(t!("data_sources.summary.properties_created", list = created.join(", ")));
    }
    if !report.properties_reused.is_empty() {
        lines.push(t!("data_sources.summary.properties_reused", list = report.properties_reused.join(", ")));
    }
    if report.options_added > 0 {
        lines.push(t!("data_sources.summary.options", n = report.options_added));
    }
    if report.images_attached + report.images_missing > 0 {
        lines.push(t!(
            "data_sources.summary.images",
            attached = report.images_attached,
            missing = report.images_missing
        ));
    }
    lines
}

fn summary_view(report: &notion::NotionImportReport) -> impl IntoElement {
    let skipped: Vec<String> = report.skipped.iter().take(ISSUES_SHOWN).map(ToString::to_string).collect();
    let more = report.skipped.len().saturating_sub(ISSUES_SHOWN);
    view()
        .gap_2()
        .p_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().primary)
        .bg(move || theme().background)
        .children(
            summary_lines(report)
                .into_iter()
                .enumerate()
                .map(|(at, line)| text(line).text_sm().when(at == 0, |t| t.font_weight(FontWeight::MEDIUM))),
        )
        .when(!report.skipped.is_empty(), |v| {
            v.child(section(
                t!("data_sources.summary.skipped_title"),
                skipped
                    .iter()
                    .cloned()
                    .chain((more > 0).then(|| t!("data_sources.preview.more_rows", n = more)))
                    .collect(),
            ))
        })
        .children(report.warnings.iter().map(|w| text(w.clone()).text_xs().text_color(crate::ui::theme::warning)))
}

// --- the import log ---------------------------------------------------------------------------------------

/// Where an import log line says the data came from.
fn source_label(source: &str) -> String {
    match source {
        "ea" => "EA".to_owned(),
        "mt5" => t!("data_sources.source.mt5"),
        "python" => "Python".to_owned(),
        "html" => t!("data_sources.source.html"),
        "xlsx" => t!("data_sources.source.xlsx"),
        "json" => t!("data_sources.source.json"),
        "notion" => t!("data_sources.source.notion"),
        other => other.to_owned(),
    }
}

/// When a line was logged, on the trader's clock.
fn logged_at(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms)
        .map_or_else(String::new, |t| t.with_timezone(&Local).format("%d/%m %H:%M:%S").to_string())
}

fn log_row(entry: &ImportLogEntry) -> impl IntoElement + use<> {
    // The stored message is in Vietnamese whatever the language then: the sentence is made again from
    // the counts, in the language now.
    let result = if entry.message.is_empty() {
        t!("data_sources.log.counts", inserted = entry.inserted, updated = entry.updated)
    } else if entry.source == "notion" {
        t!("data_sources.log.notion", inserted = entry.inserted, updated = entry.updated)
    } else {
        t!("data_sources.log.deals", inserted = entry.inserted, updated = entry.updated)
    };
    view()
        .gap_1()
        .py_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(
                    figure_text({
                        let at = logged_at(entry.at);
                        move || at.clone()
                    })
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
                )
                .child(text(source_label(&entry.source)).text_sm().font_weight(FontWeight::MEDIUM)),
        )
        .child(text(result).text_xs().text_color(move || theme().muted_foreground))
}

fn import_log_card() -> impl IntoElement {
    let log = query(|db, _| db.import_log(LOG_LINES));
    card(
        t!("data_sources.log.title"),
        dynamic(move || {
            let lines = log.value().map_or_else(Vec::new, |l| l.to_vec());
            untracked(|| {
                if lines.is_empty() {
                    return empty_state(t!("data_sources.log.empty"), t!("data_sources.log.empty_hint")).into_element();
                }
                view().children(lines.iter().map(log_row)).into_element()
            })
        }),
    )
}

#[cfg(test)]
mod tests {
    use tradelens_core::{catalog::FieldKind, import::notion::PropertyCreated};

    use super::*;

    #[test]
    fn sources_read_as_the_trader_names_them() {
        let label = |source: &'static str, lang| crate::i18n::with_text(lang, move || source_label(source));
        assert_eq!(label("html", crate::i18n::Lang::Vi), "Import HTML");
        assert_eq!(label("html", crate::i18n::Lang::En), "HTML import");
        assert_eq!(label("ea", crate::i18n::Lang::Vi), "EA");
        assert_eq!(label("sync", crate::i18n::Lang::Vi), "sync");
        assert_eq!(logged_at(1_763_716_920_000).len(), "21/11 09:42:00".len());
    }

    #[test]
    fn a_summary_lists_what_changed_and_nothing_else() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, || {
            let quiet = notion::NotionImportReport { created: 3, ..notion::NotionImportReport::default() };
            assert_eq!(summary_lines(&quiet), ["Đã nhập 3 lệnh mới · 0 cập nhật · 0 không đổi"]);
            let busy = notion::NotionImportReport {
                created: 2,
                updated: 1,
                matched_mt5: 1,
                properties_created: vec![PropertyCreated { label: "Mood".into(), kind: FieldKind::Select }],
                properties_reused: vec!["Setup".into()],
                options_added: 4,
                ..notion::NotionImportReport::default()
            };
            let lines = summary_lines(&busy);
            assert_eq!(lines.len(), 5);
            assert!(lines[2].contains("Mood (Chọn một)"), "{}", lines[2]);
            assert!(lines[4].starts_with("4 lựa chọn mới"));
        });
    }

    #[test]
    fn a_report_that_cannot_be_read_is_reported_and_the_others_still_import() {
        let mut db = Db::in_memory().expect("a memory database opens");
        let dir = tempfile::tempdir().expect("a temp dir");
        let broken = dir.path().join("broken.html");
        std::fs::write(&broken, "<html><body>không có bảng deal</body></html>").expect("writes");
        let missing = dir.path().join("missing.xlsx");
        let outcomes = import_reports(&mut db, &[broken, missing]);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|o| o.result.is_err()), "{outcomes:?}");
        assert_eq!(outcomes[0].name, "broken.html");
        assert_eq!(db.deals(None).expect("deals read").len(), 0, "nothing was stored");
    }
}
