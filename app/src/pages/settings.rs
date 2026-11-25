//! Cài đặt: appearance, the clock times are shown on, where the data lives and the keyboard shortcuts. Design: `docs/design/screens/settings.html`.
//!
//! Every choice is a setting of the database (`Db::set_setting`), so it survives restarts and
//! travels with the data file.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_core::{Db, DbError, catalog::FieldScope, time::TimeDisplay};

use crate::{
    nav,
    store::{Reading, query, writer},
    ui::{
        controls::{Variant, action, action_when, segmented},
        forms::setting_row,
        parts::{card, error_line, page},
    },
    zone::TIME_DISPLAY_KEY,
};

/// The settings key of the look the trader chose.
const APPEARANCE_KEY: &str = "appearance";

/// The keys of the app, as the design lists them: what they do and the chord.
const SHORTCUTS: &[(&str, &str)] = &[
    ("Tìm lệnh, symbol, ngày", "Ctrl+K"),
    ("Nhập lệnh thủ công", "Ctrl+N"),
    ("Lệnh sau / trước trong chi tiết", "J / K"),
    ("Lưu form", "Ctrl+Enter"),
    ("Chuyển trang trong nhóm Giao dịch", "Ctrl+1…5"),
    ("Sync ngay", "Ctrl+Shift+S"),
];

/// The three looks the trader picks from.
const APPEARANCES: &[Appearance] = &[Appearance::Dark, Appearance::Light, Appearance::System];

/// The two clocks the trader picks from.
const CLOCKS: &[TimeDisplay] = &[TimeDisplay::Local, TimeDisplay::Server];

fn appearance_label(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::Dark => "Tối",
        Appearance::Light => "Sáng",
        Appearance::System => "Theo hệ thống",
    }
}

fn appearance_key(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::Dark => "dark",
        Appearance::Light => "light",
        Appearance::System => "system",
    }
}

/// The look stored as `text`; anything else follows the system.
fn parse_appearance(text: &str) -> Appearance {
    match text.trim() {
        "dark" => Appearance::Dark,
        "light" => Appearance::Light,
        _ => Appearance::System,
    }
}

fn clock_label(clock: TimeDisplay) -> &'static str {
    match clock {
        TimeDisplay::Local => "Giờ của bạn",
        TimeDisplay::Server => "Giờ server",
    }
}

/// A file size as the data card says it: `18.4 MB`, `120 KB`.
fn size_text(bytes: u64) -> String {
    const KB: u64 = 1_024;
    const MB: u64 = KB * KB;
    let whole = |unit: u64| u32::try_from(bytes / (unit / 10)).map_or(f64::from(u32::MAX), f64::from) / 10.0;
    if bytes >= MB { format!("{:.1} MB", whole(MB)) } else { format!("{} KB", bytes.div_ceil(KB)) }
}

/// What the page shows of the stored settings.
#[derive(Debug, Clone, PartialEq, Default)]
struct Stored {
    appearance: String,
    time_display: String,
    /// The data file and its size, when the data is in a file.
    file: Option<(String, u64)>,
}

fn read_settings(db: &Db) -> Result<Stored, DbError> {
    Ok(Stored {
        appearance: db.setting(APPEARANCE_KEY)?.unwrap_or_default(),
        time_display: db.setting(TIME_DISPLAY_KEY)?.unwrap_or_default(),
        file: db.path().map(|path| {
            let size = std::fs::metadata(path).map_or(0, |m| m.len());
            (path.display().to_string(), size)
        }),
    })
}

/// Keeps the window in the look the trader chose: call it once, where the app starts. A change
/// made in the settings is shown at once and the saved choice brings it back on the next run.
pub(crate) fn follow_appearance() {
    let saved = query(|db, _| db.setting(APPEARANCE_KEY));
    effect(move || {
        if let Some(value) = saved.value() {
            set_color_scheme(parse_appearance(value.as_deref().unwrap_or_default()));
        }
    });
}

pub(crate) fn settings() -> impl IntoElement {
    let stored = query(|db, _| read_settings(db));
    let write = writer();
    let error = signal(None::<String>);
    let looks = signal(Appearance::System);
    let clock = signal(TimeDisplay::Local);
    // A memo: a read of the database that brings the same settings must not put back the choice the
    // trader has just made, before its own write is read.
    let latest = computed(move || stored.value());
    effect(move || {
        if let Some(read) = latest.get() {
            batch(|| {
                looks.set(parse_appearance(&read.appearance));
                clock.set(TimeDisplay::parse(&read.time_display));
            });
        }
    });
    let remember = move |key: &'static str, value: String| {
        write.run(move |db: &mut Db| db.set_setting(key, &value), move |result| error.set(result.err()));
    };
    page(
        "Cài đặt",
        "Giao diện, giờ hiển thị và dữ liệu",
        view().gap_6().child(dynamic(move || error.get().or_else(|| stored.error()).map(error_line))).child(
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
                        .child(appearance_card(looks, remember))
                        .child(time_card(clock, remember)),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w(px(320.0))
                        .max_w(px(420.0))
                        .gap_6()
                        .child(data_card(stored))
                        .child(shortcuts_card())
                        .child(about_card()),
                ),
        ),
    )
}

fn appearance_card(
    looks: Signal<Appearance>,
    remember: impl Fn(&'static str, String) + Copy + 'static,
) -> impl IntoElement {
    card(
        "Giao diện",
        setting_row(
            "Chế độ màu",
            Some("Theo hệ thống tự đổi theo Windows, macOS hoặc Linux"),
            segmented(
                "Chế độ màu",
                APPEARANCES,
                appearance_label,
                move || looks.get(),
                move |next| {
                    looks.set(next);
                    set_color_scheme(next);
                    remember(APPEARANCE_KEY, appearance_key(next).to_owned());
                },
            ),
        ),
    )
}

fn time_card(clock: Signal<TimeDisplay>, remember: impl Fn(&'static str, String) + Copy + 'static) -> impl IntoElement {
    card(
        "Thời gian",
        setting_row(
            "Giờ hiển thị",
            Some("Lịch, nhật ký ngày và quy tắc theo ngày tính theo giờ này; giờ server là giờ MT5 báo"),
            segmented(
                "Giờ hiển thị",
                CLOCKS,
                clock_label,
                move || clock.get(),
                move |next| {
                    clock.set(next);
                    remember(TIME_DISPLAY_KEY, next.key().to_owned());
                },
            ),
        ),
    )
}

/// Where the data file is, and the way to the form editor.
fn data_card(stored: Reading<Stored>) -> impl IntoElement {
    let file = move || stored.value().and_then(|s| s.file.clone());
    card(
        "Dữ liệu",
        view()
            .gap_3()
            .child(
                view()
                    .gap_1()
                    .child(text("File dữ liệu").text_sm().font_weight(FontWeight::MEDIUM))
                    .child(
                        text(move || match file() {
                            Some((path, size)) => format!("{path} · {}", size_text(size)),
                            None => "Dữ liệu đang nằm trong bộ nhớ, sẽ mất khi đóng ứng dụng.".to_owned(),
                        })
                        .text_xs()
                        .font_family(crate::ui::theme::mono)
                        .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(action_when(
                "Mở thư mục",
                Variant::Secondary,
                move || file().is_some(),
                move || {
                    if let Some((path, _)) = file() {
                        let folder = std::path::Path::new(&path).parent().map(std::path::Path::to_path_buf);
                        if let Some(folder) = folder {
                            drop(reveal_path(folder));
                        }
                    }
                },
            ))
            .child(
                view()
                    .gap_1()
                    .pt_3()
                    .child(text("Tuỳ biến form").text_sm().font_weight(FontWeight::MEDIUM))
                    .child(
                        text("Đổi tên, sắp xếp, ẩn, thêm thuộc tính riêng và đổi kiểu cho từng form: nhật ký lệnh, kế hoạch lúc vào, nhật ký ngày, lệnh bỏ lỡ và tổng kết tuần/tháng. Giá trị đã ghi ở các lệnh cũ được giữ lại.")
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    )
                    .child(view().flex_row().child(action("Mở Tuỳ biến form", Variant::Secondary, || nav::open_form_editor(FieldScope::Trade)))),
            ),
    )
}

fn shortcuts_card() -> impl IntoElement {
    card(
        "Phím tắt",
        view().children(SHORTCUTS.iter().map(|(what, keys)| {
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .py_2()
                .border_b_1()
                .border_color(move || theme().border)
                .child(text(*what).text_sm())
                .child(
                    text(*keys)
                        .text_xs()
                        .font_family(crate::ui::theme::mono)
                        .text_color(move || theme().muted_foreground),
                )
        })),
    )
}

fn about_card() -> impl IntoElement {
    card(
        "Về TradeLens",
        view()
            .gap_1()
            .child(text(format!("TradeLens {}", env!("CARGO_PKG_VERSION"))).text_sm().font_weight(FontWeight::MEDIUM))
            .child(
                text("Nhật ký giao dịch, dữ liệu nằm trên máy của bạn.")
                    .text_xs()
                    .text_color(move || theme().muted_foreground),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearance_round_trips_and_unknown_follows_the_system() {
        for look in APPEARANCES {
            assert_eq!(parse_appearance(appearance_key(*look)), *look);
        }
        assert_eq!(parse_appearance(""), Appearance::System);
        assert_eq!(parse_appearance("sepia"), Appearance::System);
    }

    #[test]
    fn sizes_read_in_kilobytes_and_megabytes() {
        assert_eq!(size_text(512), "1 KB");
        assert_eq!(size_text(120 * 1_024), "120 KB");
        assert_eq!(size_text(18 * 1_024 * 1_024 + 450 * 1_024), "18.4 MB");
    }

    #[test]
    fn the_stored_settings_are_read_back() {
        let mut db = Db::in_memory().expect("a memory database opens");
        let empty = read_settings(&db).expect("reads");
        assert_eq!(empty.appearance, "");
        assert_eq!(empty.file, None, "a memory database has no file");
        db.set_setting(APPEARANCE_KEY, "light").expect("stored");
        db.set_setting(TIME_DISPLAY_KEY, "server").expect("stored");
        let read = read_settings(&db).expect("reads");
        assert_eq!(parse_appearance(&read.appearance), Appearance::Light);
        assert_eq!(TimeDisplay::parse(&read.time_display), TimeDisplay::Server);
    }
}
