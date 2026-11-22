//! Nguồn dữ liệu: import báo cáo MT5, file JSON của script Python và CSV Notion.
//! Design: `docs/design/screens/data-sources.html`. The import of MT5 reports and of a Notion
//! export (automatic: no column mapping), how to export the history with the Python script, and
//! the log of what was imported.

use std::{path::PathBuf, rc::Rc};

use chrono::{DateTime, Local};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{
    Db,
    db::ImportLogEntry,
    import::{self, notion},
    time::naive,
};

use crate::{
    format::{self, Sign},
    store::{query, store, writer},
    ui::{
        chips::{Tone, chip, figure_text},
        controls::{Choice, Variant, action_icon_when, select_field},
        files::{Accept, Pick, dropzone},
        parts::{card, empty_state, page},
        properties::kind_name,
    },
};

/// What the report importer takes: MT5's HTML and XLSX, and the JSON the Python script writes.
const REPORT_FILES: Accept = Accept { name: "Báo cáo MT5 hoặc JSON", extensions: &["html", "htm", "xlsx", "json"] };

/// What the Notion importer takes.
const NOTION_FILES: Accept = Accept { name: "CSV của Notion", extensions: &["csv"] };

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
        "Nguồn dữ liệu",
        "Import báo cáo MT5, file JSON của script Python và CSV Notion.",
        view()
            .flex_row()
            .flex_wrap()
            .items_start()
            .gap_6()
            .child(view().flex_1().min_w(px(560.0)).gap_6().child(report_import_card()).child(notion_import_card()))
            .child(view().w(px(420.0)).gap_6().child(python_card()).child(import_log_card())),
    )
}

/// How to export the history of the open terminal with the Python script.
fn python_card() -> impl IntoElement {
    card(
        "Script Python (MetaTrader5)",
        view()
            .gap_3()
            .child(
                text("Ghi lịch sử của terminal MT5 đang mở ra file JSON (chỉ Windows), rồi import file đó ở thẻ bên trái.")
                    .text_sm()
                    .text_color(move || theme().muted_foreground),
            )
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
    /// The summary, or why the file could not be imported (Vietnamese).
    result: Result<String, String>,
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

fn import_report(db: &mut Db, path: &std::path::Path) -> Result<String, String> {
    let (format, batch) = import::read_file(path).map_err(|error| error.to_string())?;
    let report = db.upsert_batch(&batch, format.source()).map_err(|error| error.to_string())?;
    Ok(format!(
        "{} deal mới, {} deal cập nhật, {} deal đã có · tài khoản {}",
        report.inserted,
        report.updated,
        batch.deals.len().saturating_sub(report.inserted + report.updated),
        batch.account.login
    ))
}

fn report_import_card() -> impl IntoElement {
    let write = writer();
    let results = signal(Vec::<FileOutcome>::new());
    let busy = signal(false);
    card(
        "Import báo cáo MT5",
        view()
            .gap_3()
            .child(
                text("Báo cáo MT5 (History → Report → HTML hoặc Open XML), hoặc file JSON từ script Python. Import lại cùng file không tạo trùng.")
                    .text_sm()
                    .text_color(move || theme().muted_foreground),
            )
            .child(dropzone(
                "Kéo thả báo cáo MT5 vào đây",
                "HTML, XLSX hoặc JSON · chọn được nhiều file",
                REPORT_FILES,
                Pick::Many,
                move |paths: Vec<PathBuf>| {
                    busy.set(true);
                    write.run(
                        move |db| Ok(import_reports(db, &paths)),
                        move |outcome| {
                            busy.set(false);
                            results.set(outcome.unwrap_or_else(|message| vec![FileOutcome { name: String::new(), result: Err(message) }]));
                        },
                    );
                },
            ))
            .child(dynamic(move || busy.get().then(|| text("Đang nhập…").text_sm().text_color(move || theme().muted_foreground))))
            .child(dynamic(move || {
                let lines = results.get();
                untracked(|| view().gap_1().children(lines.iter().map(outcome_line)))
            })),
    )
}

/// A line under the drop zone, the failure in the loss colour.
fn outcome_line(outcome: &FileOutcome) -> impl IntoElement {
    let (line, failed) = match &outcome.result {
        Ok(summary) => (format!("{}: {summary}", outcome.name), false),
        Err(reason) if outcome.name.is_empty() => (reason.clone(), true),
        Err(reason) => (format!("{}: {reason}", outcome.name), true),
    };
    text(line).text_sm().text_color(move || if failed { crate::ui::theme::loss() } else { theme().foreground })
}

// --- importing a Notion export --------------------------------------------------------------------------

/// The Notion file that was read, waiting for the trader to press "Nhập".
#[derive(Debug, Clone, PartialEq)]
struct Preview {
    file: String,
    plan: notion::NotionPlan,
}

fn notion_import_card() -> impl IntoElement {
    let store = store();
    let write = writer();
    let preview = signal_with(None::<Rc<Preview>>, SignalEquals::Never);
    let report = signal_with(None::<Rc<notion::NotionImportReport>>, SignalEquals::Never);
    let failure = signal(None::<String>);
    let busy = signal(false);
    let account = signal(DEFAULT_ACCOUNT.to_owned());
    let read = move |paths: Vec<PathBuf>| {
        let Some(path) = paths.into_iter().next() else { return };
        let file = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        busy.set(true);
        spawn(async move {
            let read = blocking(move || notion::read_file(&path)).await;
            batch(|| {
                busy.set(false);
                report.set(None);
                match read {
                    Ok(Ok(plan)) => {
                        failure.set(None);
                        account.set(DEFAULT_ACCOUNT.to_owned());
                        preview.set(Some(Rc::new(Preview { file, plan })));
                    }
                    Ok(Err(error)) => {
                        preview.set(None);
                        failure.set(Some(format!("{file}: {error}")));
                    }
                    Err(error) => {
                        preview.set(None);
                        failure.set(Some(format!("Luồng đọc file bị dừng: {error}")));
                    }
                }
            });
        });
    };
    let import_now = move || {
        let Some(shown) = preview.get_untracked() else { return };
        let mut plan = shown.plan.clone();
        let chosen = account.get_untracked().parse::<i64>().ok();
        busy.set(true);
        failure.set(None);
        write.run(
            move |db| {
                if let Some(id) = chosen
                    && let Some(target) = db.accounts()?.into_iter().find(|a| a.id == id)
                {
                    plan.use_account(&target);
                }
                db.import_notion(&plan)
            },
            move |result| {
                busy.set(false);
                match result {
                    Ok(done) => batch(|| {
                        preview.set(None);
                        report.set(Some(Rc::new(done)));
                    }),
                    Err(message) => failure.set(Some(message)),
                }
            },
        );
    };
    let import_now = Rc::new(import_now);
    card(
        "Import từ Notion",
        view()
            .gap_3()
            .child(
                view().flex_row().items_center().gap_2().child(chip("Tự động", Tone::Primary)).child(
                    text("Database → ••• → Export → Markdown & CSV, giải nén rồi thả file .csv vào đây. Không cần ghép cột.")
                        .text_sm()
                        .flex_1()
                        .text_color(move || theme().muted_foreground),
                ),
            )
            .child(dropzone(
                "Kéo thả file CSV của Notion vào đây",
                "Chỉ nhận file .csv",
                NOTION_FILES,
                Pick::One,
                read,
            ))
            .child(dynamic(move || busy.get().then(|| text("Đang xử lý…").text_sm().text_color(move || theme().muted_foreground))))
            .child(dynamic(move || failure.get().map(|line| text(line).text_sm().text_color(crate::ui::theme::loss()))))
            .child(dynamic(move || {
                let shown = preview.get()?;
                let import_now = Rc::clone(&import_now);
                Some(untracked(|| preview_view(&shown, store, account, busy, import_now).into_element()))
            }))
            .child(dynamic(move || {
                let done = report.get()?;
                Some(untracked(|| summary_view(&done).into_element()))
            })),
    )
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
    let currency = plan.account.currency.clone();
    let accounts_of_plan = plan.account.login.clone();
    let count = plan.rows.len();
    let found = format!(
        "{} · {} dòng: {} lệnh đọc được, {} dòng bị bỏ qua",
        shown.file,
        plan.total_rows,
        count,
        plan.issues.len()
    );
    let period = plan.period().map(|(first, last)| {
        format!(
            "Từ {} đến {} · tổng lãi/lỗ trong file {}",
            naive(first).format("%d/%m/%Y"),
            naive(last).format("%d/%m/%Y"),
            format::money(plan.net_total(), Sign::Always, &currency)
        )
    });
    let fields: Vec<String> =
        plan.mapping.columns.iter().map(|(field, column)| format!("{} ← {column}", field.label())).collect();
    let properties: Vec<String> = plan
        .properties
        .iter()
        .map(|p| format!("{} · {} · {} lệnh có giá trị", p.label, kind_name(p.kind), p.filled))
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
            "Trường tự nhận ra",
            if fields.is_empty() { vec!["Không có.".to_owned()] } else { vec![fields.join("  ·  ")] },
        ))
        .when(!properties.is_empty(), |v| {
            v.child(section("Thuộc tính sẽ tạo trong form lệnh (dùng lại nếu đã có cùng tên)", properties))
        })
        .when(!plan.ignored_columns.is_empty(), |v| {
            v.child(muted(format!("Bỏ qua {} cột: {}", plan.ignored_columns.len(), plan.ignored_columns.join(", "))))
        })
        .when(!plan.warnings.is_empty(), |v| {
            v.child(view().gap_1().children(
                plan.warnings.iter().map(|w| text(w.clone()).text_xs().text_color(crate::ui::theme::warning())),
            ))
        })
        .when(!plan.issues.is_empty(), |v| {
            let mut lines: Vec<String> = plan.issues.iter().take(ISSUES_SHOWN).map(ToString::to_string).collect();
            if plan.issues.len() > ISSUES_SHOWN {
                lines.push(format!("… và {} dòng nữa", plan.issues.len() - ISSUES_SHOWN));
            }
            v.child(section("Dòng sẽ bị bỏ qua", lines))
        })
        .child(select_field(
            "Nhập vào tài khoản",
            move || {
                let mut choices = vec![Choice::new(DEFAULT_ACCOUNT, "Nhật ký Notion (mặc định)")];
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
            format!("Nhập {count} lệnh"),
            Variant::Primary,
            move || count > 0 && !busy.get(),
            move || import_now(),
        )))
}

/// A heading over a few lines of detail.
fn section(heading: &'static str, lines: Vec<String>) -> impl IntoElement {
    view()
        .gap_1()
        .child(text(heading).text_xs().text_color(move || theme().muted_foreground))
        .children(lines.into_iter().map(|line| text(line).text_sm()))
}

/// What an import stored, in lines.
fn summary_lines(report: &notion::NotionImportReport) -> Vec<String> {
    let mut lines = vec![format!(
        "Đã nhập {} lệnh mới · {} cập nhật · {} không đổi",
        report.created, report.updated, report.unchanged
    )];
    if report.matched_mt5 > 0 {
        lines.push(format!(
            "{} lệnh trùng lệnh MT5: chỉ gắn nhật ký vào lệnh MT5, không tạo thêm lệnh",
            report.matched_mt5
        ));
    }
    if !report.properties_created.is_empty() {
        let created: Vec<String> =
            report.properties_created.iter().map(|p| format!("{} ({})", p.label, kind_name(p.kind))).collect();
        lines.push(format!("Thuộc tính đã tạo: {}", created.join(", ")));
    }
    if !report.properties_reused.is_empty() {
        lines.push(format!("Thuộc tính dùng lại: {}", report.properties_reused.join(", ")));
    }
    if report.options_added > 0 {
        lines.push(format!("{} lựa chọn mới đã thêm vào danh sách tag, lỗi, cảm xúc…", report.options_added));
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
                "Dòng bị bỏ qua",
                skipped.iter().cloned().chain((more > 0).then(|| format!("… và {more} dòng nữa"))).collect(),
            ))
        })
        .children(report.warnings.iter().map(|w| text(w.clone()).text_xs().text_color(crate::ui::theme::warning())))
}

// --- the import log ---------------------------------------------------------------------------------------

/// Where an import log line says the data came from.
fn source_label(source: &str) -> String {
    match source {
        "ea" => "EA".to_owned(),
        "python" => "Python".to_owned(),
        "html" => "Import HTML".to_owned(),
        "xlsx" => "Import Excel".to_owned(),
        "json" => "Import JSON".to_owned(),
        "notion" => "Import Notion".to_owned(),
        other => other.to_owned(),
    }
}

/// When a line was logged, on the trader's clock.
fn logged_at(ms: i64) -> String {
    DateTime::from_timestamp_millis(ms)
        .map_or_else(String::new, |t| t.with_timezone(&Local).format("%d/%m %H:%M:%S").to_string())
}

fn log_row(entry: &ImportLogEntry) -> impl IntoElement + use<> {
    let result = if entry.message.is_empty() {
        format!("+{} · {} cập nhật", entry.inserted, entry.updated)
    } else {
        entry.message.clone()
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
        "Nhật ký import",
        dynamic(move || {
            let lines = log.value().map_or_else(Vec::new, |l| l.to_vec());
            untracked(|| {
                if lines.is_empty() {
                    return empty_state("Chưa import gì", "Kết quả mỗi lần import hiện ở đây.").into_element();
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
        assert_eq!(source_label("html"), "Import HTML");
        assert_eq!(source_label("ea"), "EA");
        assert_eq!(source_label("sync"), "sync");
        assert_eq!(logged_at(1_763_716_920_000).len(), "21/11 09:42:00".len());
    }

    #[test]
    fn a_summary_lists_what_changed_and_nothing_else() {
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
