//! Chia sẻ kết quả: a dialog over any screen that draws the result of a trade, a week or a month
//! as a picture to save or copy, with the trader choosing how much of the money it shows. Opened
//! from a trade's page and from the review screen. Also the one place a monthly report is saved as
//! PDF ([`export_pdf`]). Design: `docs/design/screens/share-card.html`.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{Datelike, NaiveDate};
use nsh_base::{dialog, theme};
use nsh_core::{
    file_dialog::{FileFilter, SaveDialog},
    prelude::*,
};
use tradelens_core::{
    TradeKey,
    attachments::Source,
    report::{self, MonthlyReport, Privacy, ShareCard},
    review::Period,
};

use crate::{
    insight, month_report,
    snapshot::Snapshot,
    store::{query_with, store},
    ui::{
        controls::{Variant, action, action_icon_when, icon_action, segmented},
        files::file_url,
        forms::switch_row,
        icons::Icon,
        theme as tokens,
    },
};

/// Width of the dialog's panel, in points.
const PANEL_WIDTH: f32 = 680.0;
/// The padding inside the panel.
const PANEL_PADDING: f32 = 22.0;
/// How far the scrim darkens the screen behind the dialog.
const SCRIM_OPACITY: f32 = 0.72;
/// The preview's size: the card's 1200 × 630 scaled to the panel's inside width.
const PREVIEW_WIDTH: f32 = PANEL_WIDTH - 2.0 * PANEL_PADDING;
const PREVIEW_HEIGHT: f32 = PREVIEW_WIDTH * 630.0 / 1200.0;

/// What a card is of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShareTarget {
    /// A closed trade.
    Trade(TradeKey),
    /// The week or month starting on a day.
    Period(Period, NaiveDate),
}

/// The target being shared, while the dialog is open.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Open(Option<ShareTarget>);

/// Opens the dialog on `target`.
pub(crate) fn open(target: ShareTarget) {
    global::<Open>().set(Open(Some(target)));
}

fn close() {
    global::<Open>().set(Open(None));
}

/// The dialog, mounted once over the whole window.
pub(crate) fn host() -> impl IntoElement {
    let open = global::<Open>();
    dynamic(move || open.get().0.map(|target| dialog().on_cancel(close).flex_1().child(untracked(|| scrim(&target)))))
}

/// The darkened screen with the panel in its middle.
fn scrim(target: &ShareTarget) -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .p_6()
        .bg(move || theme().background.opacity(SCRIM_OPACITY))
        .child(panel(target))
}

pub(super) const PRIVACY_CHOICES: [Privacy; 3] = [Privacy::ShowMoney, Privacy::OnlyR, Privacy::NoFigures];

pub(super) fn privacy_label(privacy: Privacy) -> &'static str {
    match privacy {
        Privacy::ShowMoney => "Hiện tiền",
        Privacy::OnlyR => "Chỉ R",
        Privacy::NoFigures => "Ẩn số",
    }
}

/// What the trader chose to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Options {
    privacy: Privacy,
    account: bool,
    setup: bool,
    chart: bool,
}

/// Where a card's content comes from.
struct Context<'a> {
    snapshot: &'a Snapshot,
    /// The data folder's `attachments/`, where chart images are.
    attachments: Option<&'a Path>,
    /// The account in view, for the name a period card may show.
    account: Option<i64>,
}

/// The week's title with its year, `Tuần 47 · 16–22/11/2026`, or the month's, `Tháng 11/2026`.
fn period_title(period: Period, start: NaiveDate) -> String {
    let title = period.title(start);
    match period {
        Period::Week => format!("{title}/{}", period.bounds(start).1.year()),
        Period::Month => title,
    }
}

/// The picture of a trade's chart to show: the exit screenshot, else the first one.
fn chart_file(snapshot: &Snapshot, key: &TradeKey, folder: &Path) -> Option<String> {
    let shots = &snapshot.journals.get(key)?.screenshots;
    let shot = shots.iter().find(|s| s.source == Source::EaExit).or_else(|| shots.first())?;
    Some(folder.join(&shot.path).display().to_string())
}

/// The card of `target`; `None` when the target is not a closed trade of the snapshot.
fn build_card(context: &Context<'_>, target: &ShareTarget, options: Options) -> Option<ShareCard> {
    let snapshot = context.snapshot;
    match target {
        ShareTarget::Trade(key) => {
            let row = snapshot.row(key).filter(|row| row.trade.is_closed())?;
            let setup = (options.setup && !row.setup.is_empty()).then(|| row.setup.clone());
            let chart =
                if options.chart { context.attachments.and_then(|dir| chart_file(snapshot, key, dir)) } else { None };
            let card = ShareCard::new(
                &row.trade,
                row.r,
                setup,
                chart,
                &row.currency,
                options.privacy,
                insight::summary_clock(snapshot),
            );
            Some(card.with_account(options.account.then(|| row.account.clone())))
        }
        ShareTarget::Period(period, start) => {
            let mut figures = month_report::period_figures(snapshot, *period, *start);
            if !options.setup {
                figures.top_setup = None;
            }
            let card = ShareCard::period(period_title(*period, *start), &figures, options.privacy);
            Some(card.with_account(options.account.then(|| month_report::account_name(snapshot, context.account))))
        }
    }
}

/// A suggested file name (no extension) for the picture or report of `target`.
fn file_stem(snapshot: &Snapshot, target: &ShareTarget) -> String {
    match target {
        ShareTarget::Trade(key) => {
            let symbol = snapshot.row(key).map_or_else(String::new, |row| row.trade.symbol.to_lowercase());
            format!("tradelens-{symbol}-{}", key.position_id)
        }
        ShareTarget::Period(Period::Week, start) => format!("tradelens-tuan-{}", start.format("%G-%V")),
        ShareTarget::Period(Period::Month, start) => format!("tradelens-thang-{}", start.format("%Y-%m")),
    }
}

/// A card drawn, and where its preview file is. The file goes when the card is replaced.
#[derive(Debug)]
struct Rendered {
    bytes: Vec<u8>,
    file: PathBuf,
}

impl Drop for Rendered {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.file) {
            log::debug!("share preview {}: {error}", self.file.display());
        }
    }
}

/// Draws `card` and writes it where the preview can show it. Each preview has its own file name,
/// as an image is cached by its address.
fn render(card: &ShareCard) -> Result<Rendered, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let bytes = report::render_png(card).map_err(|error| format!("Không dựng được ảnh: {error}"))?;
    let name = format!("tradelens-share-{}-{}.png", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let file = std::env::temp_dir().join(name);
    std::fs::write(&file, &bytes).map_err(|error| format!("Không ghi được ảnh xem trước: {error}"))?;
    Ok(Rendered { bytes, file })
}

/// The `data:` URI the clipboard takes an image as.
fn data_uri(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", STANDARD.encode(png))
}

/// Asks where to save `report` and writes it as a PDF showing what `privacy` allows. `done` hears
/// the line to tell the trader (a failure or the saved path); nothing when the dialog is cancelled.
pub(crate) fn export_pdf(report: MonthlyReport, privacy: Privacy, done: impl Fn(Result<String, String>) + 'static) {
    let name = format!("tradelens-bao-cao-{}.pdf", report.month.format("%Y-%m"));
    spawn(async move {
        let dialog = SaveDialog::new()
            .file_name(name)
            .filter(FileFilter::new("Báo cáo PDF", ["pdf"]))
            .title("Lưu báo cáo tháng")
            .button_label("Lưu");
        let path = match pick_save_path(dialog).await {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => return done(Err(format!("Không mở được hộp thoại lưu file: {error}"))),
        };
        let shown = path.display().to_string();
        match blocking(move || report::write_pdf(&report, privacy, &path)).await {
            Ok(Ok(())) => done(Ok(format!("Đã lưu báo cáo: {shown}"))),
            Ok(Err(error)) => done(Err(format!("Không lưu được báo cáo: {error}"))),
            Err(error) => done(Err(format!("Luồng ghi báo cáo bị dừng: {error}"))),
        }
    });
}

/// What the dialog's controls and results hold: one signal each, so the pieces below share them.
#[derive(Clone, Copy)]
struct State {
    privacy: Signal<Privacy>,
    show_account: Signal<bool>,
    show_setup: Signal<bool>,
    show_chart: Signal<bool>,
    /// The newest card drawn.
    rendered: Signal<Option<Arc<Rendered>>>,
    failure: Signal<Option<String>>,
    note: Signal<Option<String>>,
}

impl State {
    fn new() -> Self {
        Self {
            privacy: signal(Privacy::OnlyR),
            show_account: signal(false),
            show_setup: signal(true),
            show_chart: signal(true),
            rendered: signal_with(None::<Arc<Rendered>>, SignalEquals::Never),
            failure: signal(None),
            note: signal(None),
        }
    }

    /// Tells the trader something went well, and clears any earlier trouble.
    fn succeed(self, line: String) {
        batch(|| {
            self.failure.set(None);
            self.note.set(Some(line));
        });
    }

    fn fail(self, line: String) {
        self.failure.set(Some(line));
    }
}

/// Draws the card again whenever the choices or the data change, off the UI thread; only the
/// newest drawing is shown.
fn keep_drawn(target: ShareTarget, state: State) {
    let store = store();
    let folder = data_dir().map(|dir| dir.join("attachments"));
    let card = computed(move || {
        let options = Options {
            privacy: state.privacy.get(),
            account: state.show_account.get(),
            setup: state.show_setup.get(),
            chart: state.show_chart.get(),
        };
        let snapshot = store.snapshot();
        let context =
            Context { snapshot: &snapshot, attachments: folder.as_deref(), account: store.accounts.get().id() };
        build_card(&context, &target, options)
    });
    let generation = stored(0_u64);
    effect(move || {
        let card = card.get();
        untracked(|| {
            let this_drawing = generation.with(|g| {
                *g += 1;
                *g
            });
            let Some(card) = card else {
                state.rendered.set(None);
                return state.fail("Chỉ chia sẻ được lệnh đã đóng.".to_owned());
            };
            spawn(async move {
                let drawn = blocking(move || render(&card)).await;
                if generation.get() != this_drawing {
                    return;
                }
                match drawn {
                    Ok(Ok(picture)) => batch(|| {
                        state.rendered.set(Some(Arc::new(picture)));
                        state.failure.set(None);
                    }),
                    Ok(Err(message)) => state.fail(message),
                    Err(error) => state.fail(format!("Luồng dựng ảnh bị dừng: {error}")),
                }
            });
        });
    });
}

/// "Lưu ảnh": asks where, then writes the picture shown.
fn save_picture(target: ShareTarget, state: State) -> impl Fn() + Clone + 'static {
    let store = store();
    move || {
        let Some(picture) = state.rendered.get_untracked() else { return };
        let stem = file_stem(&store.snapshot_now(), &target);
        state.note.set(None);
        spawn(async move {
            let dialog = SaveDialog::new()
                .file_name(format!("{stem}.png"))
                .filter(FileFilter::new("Ảnh PNG", ["png"]))
                .title("Lưu ảnh chia sẻ")
                .button_label("Lưu");
            let path = match pick_save_path(dialog).await {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(error) => return state.fail(format!("Không mở được hộp thoại lưu file: {error}")),
            };
            let shown = path.display().to_string();
            let bytes = Arc::clone(&picture);
            match blocking(move || std::fs::write(&path, &bytes.bytes)).await {
                Ok(Ok(())) => state.succeed(format!("Đã lưu ảnh: {shown}")),
                Ok(Err(error)) => state.fail(format!("Không lưu được ảnh: {error}")),
                Err(error) => state.fail(format!("Luồng ghi ảnh bị dừng: {error}")),
            }
        });
    }
}

/// "Sao chép ảnh": puts the picture shown on the clipboard.
fn copy_picture(state: State) -> impl Fn() + Clone + 'static {
    move || {
        let Some(picture) = state.rendered.get_untracked() else { return };
        set_clipboard_image(data_uri(&picture.bytes));
        state.succeed("Đã sao chép ảnh vào clipboard.".to_owned());
    }
}

/// "Xuất báo cáo PDF tháng": the PDF of the month shown, at the privacy level chosen here. The
/// month's extra rows are read once the dialog is open; pressing before they arrive says so.
fn export_month(month: Option<NaiveDate>, state: State) -> impl Fn() + Clone + 'static {
    let store = store();
    let extras = query_with(
        move || month,
        |db, account, month| match month {
            Some(start) => month_report::load_extras(db, account, *start).map(Some),
            None => Ok(None),
        },
    );
    move || {
        let Some(start) = month else { return };
        let Some(Some(extras)) = extras.value().map(|read| read.as_ref().clone()) else {
            return state.fail("Đang đọc dữ liệu của tháng, thử lại sau giây lát.".to_owned());
        };
        let report = month_report::build(&store.snapshot_now(), store.accounts.get_untracked().id(), start, &extras);
        export_pdf(report, state.privacy.get_untracked(), move |result| match result {
            Ok(line) => state.succeed(line),
            Err(line) => state.fail(line),
        });
    }
}

fn panel(target: &ShareTarget) -> impl IntoElement {
    let state = State::new();
    keep_drawn(target.clone(), state);
    let month = match target {
        ShareTarget::Period(Period::Month, start) => Some(*start),
        _ => None,
    };
    let (save, copy, export) = (save_picture(target.clone(), state), copy_picture(state), export_month(month, state));
    view()
        .w(px(PANEL_WIDTH))
        .p(px(PANEL_PADDING))
        .gap_4()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || theme().border)
        .bg(move || theme().card)
        .shadow_lg()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(text("Chia sẻ kết quả").text_base().font_weight(FontWeight::SEMIBOLD))
                .child(icon_action(Icon::Close, "Đóng", Variant::Ghost, close)),
        )
        .child(segmented(
            "Quyền riêng tư",
            &PRIVACY_CHOICES,
            privacy_label,
            move || state.privacy.get(),
            move |next| state.privacy.set(next),
        ))
        .child(preview(state.rendered))
        .child(toggles(state, target))
        .child(dynamic(move || state.failure.get().map(|line| text(line).text_sm().text_color(tokens::loss()))))
        .child(dynamic(move || {
            state.note.get().map(|line| text(line).text_sm().text_color(move || theme().muted_foreground))
        }))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(view().flex_row().when_some(month, |slot, start| {
                    slot.child(action(
                        format!("Xuất báo cáo PDF tháng {} →", start.format("%m")),
                        Variant::Ghost,
                        export,
                    ))
                }))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(action_icon_when(
                            Icon::Download,
                            "Lưu ảnh",
                            Variant::Secondary,
                            move || state.rendered.get().is_some(),
                            save,
                        ))
                        .child(action_icon_when(
                            Icon::Copy,
                            "Sao chép ảnh",
                            Variant::Primary,
                            move || state.rendered.get().is_some(),
                            copy,
                        )),
                ),
        )
}

/// What the card shows besides the figures: the account, the setup and, for a trade, the chart.
fn toggles(state: State, target: &ShareTarget) -> impl IntoElement {
    view()
        .child(switch_row(
            "Hiện tên tài khoản / broker",
            None,
            move || state.show_account.get(),
            move |on| state.show_account.set(on),
        ))
        .child(switch_row("Hiện tên setup", None, move || state.show_setup.get(), move |on| state.show_setup.set(on)))
        .when(matches!(target, ShareTarget::Trade(_)), |rows| {
            rows.child(switch_row(
                "Kèm ảnh chart",
                Some("Ảnh chụp chart có thể hiện số tiền: xem kỹ ảnh trước khi đăng."),
                move || state.show_chart.get(),
                move |on| state.show_chart.set(on),
            ))
        })
}

/// The card as drawn, or a line saying it is being drawn.
fn preview(rendered: Signal<Option<Arc<Rendered>>>) -> impl IntoElement {
    view()
        .w(px(PREVIEW_WIDTH))
        .h(px(PREVIEW_HEIGHT))
        .rounded(move || theme().radius)
        .overflow_hidden()
        .items_center()
        .justify_center()
        .bg(move || theme().secondary)
        .child(dynamic(move || match rendered.get() {
            Some(picture) => file_url(&picture.file).map(|url| {
                image(url)
                    .w(px(PREVIEW_WIDTH))
                    .h(px(PREVIEW_HEIGHT))
                    .object_fit(ResizeMode::Contain)
                    .label("Xem trước thẻ chia sẻ")
                    .into_element()
            }),
            None => Some(text("Đang dựng ảnh…").text_sm().text_color(move || theme().muted_foreground).into_element()),
        }))
}

#[cfg(test)]
mod tests {
    use tradelens_core::{
        Db, Deal, DealBatch, DealEntry, DealKind, JournalEntry, attachments::Attachment, model::AccountInfo,
        time::parse_mt5_time,
    };

    use super::*;
    use crate::snapshot::{Query, Range, load};

    fn deal(ticket: i64, position: i64, at: &str, kind: DealKind, entry: DealEntry, profit: f64) -> Deal {
        Deal {
            ticket,
            order: ticket,
            position_id: position,
            time_msc: parse_mt5_time(at).expect("fixture times parse"),
            symbol: "XAUUSD".into(),
            kind,
            entry,
            volume: 0.1,
            price: 2650.0,
            commission: 0.0,
            swap: 0.0,
            fee: 0.0,
            profit,
            magic: 0,
            comment: String::new(),
            sl: 2645.0,
            tp: 0.0,
            risk: Some(100.0),
            result_r: None,
            gaps: tradelens_core::Gaps::NONE,
            reason: None,
        }
    }

    fn snapshot() -> Snapshot {
        use DealEntry::{In, Out};
        use DealKind::{Buy, Sell};
        let mut db = Db::in_memory().expect("a memory database opens");
        let batch = DealBatch {
            account: AccountInfo {
                login: "42".into(),
                server: "Demo".into(),
                broker: "Exness".into(),
                currency: "USD".into(),
                ..AccountInfo::default()
            },
            deals: vec![
                deal(1, 7, "2026.11.20 09:00:00", Buy, In, 0.0),
                deal(2, 7, "2026.11.20 10:00:00", Sell, Out, 250.0),
                deal(3, 8, "2026.11.21 09:00:00", Buy, In, 0.0),
                deal(4, 8, "2026.11.21 09:30:00", Sell, Out, -100.0),
            ],
            positions: Vec::new(),
        };
        db.upsert_batch(&batch, "test").expect("the batch is stored");
        let shot = |path: &str, source| Attachment {
            path: path.to_owned(),
            caption: String::new(),
            timeframe: None,
            source,
            added_at: 0,
        };
        let entry = JournalEntry {
            setup: "Breakout".into(),
            screenshots: vec![shot("images/a.png", Source::Upload), shot("images/b.png", Source::EaExit)],
            ..JournalEntry::default()
        };
        db.save_journal(&TradeKey { account_id: 1, position_id: 7, segment: 0 }, &entry).expect("stored");
        let query = Query { account: None, range: Range::All, now_utc_ms: 0, local_offset_min: 0 };
        load(&db, &query).expect("the snapshot loads")
    }

    fn trade(position: i64) -> ShareTarget {
        ShareTarget::Trade(TradeKey { account_id: 1, position_id: position, segment: 0 })
    }

    fn options(privacy: Privacy) -> Options {
        Options { privacy, account: false, setup: true, chart: true }
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("a test date")
    }

    #[test]
    fn a_trade_card_follows_the_choices() {
        let snap = snapshot();
        let folder = PathBuf::from("data/attachments");
        let context = Context { snapshot: &snap, attachments: Some(&folder), account: None };
        let card = build_card(&context, &trade(7), options(Privacy::OnlyR)).expect("a closed trade");
        assert_eq!(card.title, "XAUUSD · Buy");
        assert_eq!(card.result_r, "+2.50R", "250 on a risk of 100");
        assert!(card.result_money.is_none());
        assert_eq!(card.stats.last().map(|(l, v)| (l.as_str(), v.as_str())), Some(("Setup", "Breakout")));
        let chart = card.chart.expect("the exit screenshot is chosen");
        assert!(chart.ends_with("b.png"), "the EA's exit shot comes before the first upload: {chart}");

        let money = build_card(&context, &trade(7), options(Privacy::ShowMoney)).expect("a card");
        assert_eq!(money.result_money.as_deref(), Some("+$250.00"));
        let bare = Options { setup: false, chart: false, account: true, ..options(Privacy::OnlyR) };
        let card = build_card(&context, &trade(7), bare).expect("a card");
        assert!(card.chart.is_none() && card.stats.iter().all(|(l, _)| l != "Setup"));
        assert!(card.account.is_some_and(|a| a.contains("42")), "the account's name is shown on request");
        let lost = build_card(&context, &trade(8), options(Privacy::NoFigures)).expect("a card");
        assert_eq!((lost.result_r.as_str(), lost.headline().main.as_str()), ("", "Thua"));
        assert!(build_card(&context, &trade(99), options(Privacy::OnlyR)).is_none(), "an unknown trade has no card");
    }

    #[test]
    fn a_period_card_sums_the_period_and_names_it() {
        let snap = snapshot();
        let context = Context { snapshot: &snap, attachments: None, account: None };
        let week = ShareTarget::Period(Period::Week, day("2026-11-16"));
        let card = build_card(&context, &week, options(Privacy::ShowMoney)).expect("a card");
        assert_eq!(card.title, "Tuần 47 · 16–22/11/2026");
        assert_eq!(card.result_money.as_deref(), Some("+$150.00"));
        assert_eq!(card.result_r, "+1.5R");
        assert_eq!(card.curve.len(), 2);
        let month = ShareTarget::Period(Period::Month, day("2026-11-01"));
        let card = build_card(&context, &month, Options { setup: false, ..options(Privacy::OnlyR) }).expect("a card");
        assert_eq!(card.title, "Tháng 11/2026");
        assert!(card.stats.iter().all(|(l, _)| l != "Setup chính"), "the setup name can be left out");
    }

    #[test]
    fn files_are_named_for_what_they_show() {
        let snap = snapshot();
        assert_eq!(file_stem(&snap, &trade(7)), "tradelens-xauusd-7");
        assert_eq!(file_stem(&snap, &ShareTarget::Period(Period::Week, day("2026-11-16"))), "tradelens-tuan-2026-47");
        assert_eq!(file_stem(&snap, &ShareTarget::Period(Period::Month, day("2026-11-01"))), "tradelens-thang-2026-11");
        assert_eq!(period_title(Period::Month, day("2026-11-01")), "Tháng 11/2026");
    }

    #[test]
    fn a_drawn_card_leaves_a_preview_file_that_goes_with_it() {
        let snap = snapshot();
        let context = Context { snapshot: &snap, attachments: None, account: None };
        let card = build_card(&context, &trade(7), options(Privacy::OnlyR)).expect("a card");
        let drawn = render(&card).expect("the card is drawn");
        assert!(drawn.bytes.starts_with(b"\x89PNG"));
        let file = drawn.file.clone();
        assert!(file.exists(), "the preview file is written");
        assert!(data_uri(&drawn.bytes).starts_with("data:image/png;base64,iVBOR"));
        drop(drawn);
        assert!(!file.exists(), "and removed with the picture");
    }
}
