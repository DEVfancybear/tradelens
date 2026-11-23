//! Đăng nhập & sync: sign in with Google or Microsoft, sync the journal through the account's
//! app folder, and restore it on a new machine. Design: `docs/design/screens/cloud-sync.html`.

use chrono::{DateTime, Local};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_cloud::{Provider, auth::Identity, sync::DeviceInfo, sync::SyncReport};

use crate::{
    cloud::{self as app_cloud, AutoSync, Cloud, Outcome, Phase, State, Why},
    ui::{
        chips::{Tone, chip},
        controls::{Variant, action, action_icon_when, confirm},
        forms::switch_row,
        icons::{Icon, icon},
        layout::columns,
        parts::{card, warning_banner},
        theme::loss,
    },
};

/// What a provider is called and how the card describes it.
fn describe(provider: Provider) -> (&'static str, &'static str, &'static str) {
    match provider {
        Provider::Google => ("Google Drive", "Tài khoản Google · thư mục riêng của app", "Đăng nhập với Google"),
        Provider::Microsoft => ("OneDrive", "Tài khoản Microsoft · thư mục riêng của app", "Đăng nhập với Microsoft"),
    }
}

pub(crate) fn cloud() -> impl IntoElement {
    let cloud = app_cloud::cloud();
    crate::ui::parts::page(
        "Đăng nhập & sync",
        "Dùng TradeLens trên nhiều máy và không lo mất nhật ký",
        view()
            .gap_6()
            .child(dynamic(move || {
                // Nothing here reads the state: the banner follows it by itself.
                untracked(|| notice(cloud).into_element())
            }))
            .child(view().flex_row().flex_wrap().gap_6().children(Provider::ALL.map(|p| provider_card(cloud, p))))
            .child(columns(
                view().gap_6().child(sync_card(cloud)).child(restore_card(cloud)),
                view().gap_6().child(devices_card(cloud)).child(access_card()),
            )),
    )
}

/// What went wrong when signing in or out, or why syncing cannot start.
fn notice(cloud: Cloud) -> impl IntoElement {
    let state = cloud.state;
    dynamic(move || {
        let line = state.with(|s| s.notice.clone());
        untracked(|| line.map(|line| warning_banner(line).into_element()))
    })
}

fn provider_card(cloud: Cloud, provider: Provider) -> impl IntoElement {
    let state = cloud.state;
    let (name, subtitle, _) = describe(provider);
    let in_use = move || state.with(|s| s.active() == Some(provider));
    view()
        .flex_1()
        .min_w(px(320.0))
        .p_5()
        .gap_3()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || if in_use() { theme().primary } else { theme().border })
        .bg(move || theme().card)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(icon(Icon::Cloud, 22.0).text_color(move || theme().primary))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(text(name).text_base().font_weight(FontWeight::SEMIBOLD))
                        .child(text(subtitle).text_xs().text_color(move || theme().muted_foreground)),
                )
                .child(dynamic(move || in_use().then(|| chip("Đang dùng", Tone::Primary)))),
        )
        .child(dynamic(move || {
            let (signed, phase, missing) = state.with(|s| {
                (s.identity_of(provider).cloned(), s.phase, s.ready.then(|| s.env.missing_client(provider)).flatten())
            });
            untracked(|| provider_body(cloud, provider, signed, phase, missing).into_element())
        }))
}

/// The part of a provider's card that depends on whether somebody is signed in.
fn provider_body(
    cloud: Cloud,
    provider: Provider,
    signed: Option<Identity>,
    phase: Phase,
    missing: Option<String>,
) -> impl IntoElement {
    let (name, _, sign_in_label) = describe(provider);
    if let Some(me) = signed {
        let sign_out = move || {
            spawn(async move {
                let sure = confirm(
                    format!("Đăng xuất khỏi {name}?"),
                    "Dữ liệu trên Drive vẫn còn; TradeLens chỉ ngừng sync trên máy này cho tới khi bạn đăng nhập lại.",
                    "Đăng xuất",
                )
                .await;
                if sure {
                    cloud.sign_out(provider);
                }
            });
        };
        return view()
            .gap_3()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(view().size(px(8.0)).rounded_full().bg(move || theme().primary))
                    .child(text(me.email).text_sm()),
            )
            .child(view().flex_row().child(action("Đăng xuất", Variant::Ghost, sign_out)))
            .into_element();
    }
    if phase == Phase::SigningIn(provider) {
        return view()
            .gap_3()
            .child(
                text("Đang chờ bạn đồng ý trong trình duyệt…").text_sm().text_color(move || theme().muted_foreground),
            )
            .child(view().flex_row().child(action("Huỷ", Variant::Secondary, move || cloud.cancel_sign_in())))
            .into_element();
    }
    view()
        .gap_3()
        .child(text("Chưa đăng nhập").text_sm().text_color(move || theme().muted_foreground))
        .children(missing.map(|why| text(why).text_xs().text_color(crate::ui::theme::warning())))
        .child(view().flex_row().child(action_icon_when(
            Icon::Cloud,
            sign_in_label,
            Variant::Secondary,
            move || cloud.state.with(|s| !s.busy()),
            move || cloud.sign_in(provider),
        )))
        .into_element()
}

// --- sync ---------------------------------------------------------------------------------------------------

/// A moment as the screen says it: `14:02 hôm nay`, or `18/11 22:10` for another day.
fn when(ms: i64) -> String {
    let Some(at) = DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&Local)) else { return String::new() };
    if at.date_naive() == Local::now().date_naive() {
        format!("{} hôm nay", at.format("%H:%M"))
    } else {
        at.format("%d/%m %H:%M").to_string()
    }
}

/// What a sync did, in one line.
fn counts(report: &SyncReport) -> String {
    let mut parts = vec![format!("đẩy {} thay đổi, nhận {}", report.pushed, report.pulled)];
    if report.conflicts_resolved > 0 {
        parts.push(format!("{} chỗ hai máy sửa khác nhau đã gộp theo lần sửa mới nhất", report.conflicts_resolved));
    }
    if report.attachments_uploaded + report.attachments_downloaded > 0 {
        parts.push(format!("ảnh: {} lên, {} xuống", report.attachments_uploaded, report.attachments_downloaded));
    }
    if report.snapshot_taken {
        parts.push("đã chụp bản sao tuần".to_owned());
    }
    parts.join(" · ")
}

/// The headline and the detail line of the sync card for the state.
fn headline(state: &State) -> (&'static str, String, bool) {
    match (state.phase, &state.last) {
        (Phase::Syncing, _) => {
            ("Đang đồng bộ…", "Đẩy thay đổi của máy này và nhận thay đổi của máy khác".to_owned(), false)
        }
        (Phase::Restoring, _) => {
            ("Đang khôi phục…", "Tải bản sao lưu gần nhất rồi nhận các thay đổi sau đó".to_owned(), false)
        }
        (_, Some(Outcome { at, result: Err(message), .. })) => {
            ("Sync lỗi", format!("Lần thử {} · {message}", when(*at)), true)
        }
        (_, Some(Outcome { at, result: Ok(report), restored })) => (
            if *restored { "Đã khôi phục" } else { "Đã đồng bộ" },
            format!("Lần cuối {} · {}", when(*at), counts(report)),
            false,
        ),
        (_, None) => match state.last_sync {
            Some(at) => ("Đã đồng bộ", format!("Lần cuối {}", when(at)), false),
            None => ("Chưa đồng bộ", "Bấm Sync ngay sau khi đăng nhập".to_owned(), false),
        },
    }
}

fn sync_card(cloud: Cloud) -> impl IntoElement {
    let state = cloud.state;
    card(
        "Đồng bộ",
        view()
            .gap_3()
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_4()
                    .child(view().flex_1().min_w(px(220.0)).gap_1().child(dynamic(move || {
                        let (title, detail, failed) = state.with(headline);
                        untracked(|| {
                            view()
                                .gap_1()
                                .child(
                                    text(title)
                                        .text_base()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || if failed { loss() } else { theme().foreground }),
                                )
                                .child(text(detail).text_xs().text_color(move || theme().muted_foreground))
                                .into_element()
                        })
                    })))
                    .child(action_icon_when(
                        Icon::Refresh,
                        "Sync ngay",
                        Variant::Primary,
                        move || state.with(|s| s.ready && s.active().is_some() && !s.busy()),
                        move || cloud.sync(Why::Pressed),
                    )),
            )
            .child(dynamic(move || {
                state
                    .with(|s| s.env.is_demo())
                    .then(|| chip("Chế độ thử: ổ đĩa nằm trong bộ nhớ, không cần tài khoản", Tone::Neutral))
            }))
            .child(switch_row(
                "Tự động sync",
                Some("Mỗi 15 phút và khi đóng app; chỉ áp dụng cho máy này"),
                move || state.with(|s| s.env.auto_sync()),
                move |on| cloud.set_auto_sync(if on { AutoSync::On } else { AutoSync::Off }),
            )),
    )
}

fn restore_card(cloud: Cloud) -> impl IntoElement {
    let state = cloud.state;
    let restore = move || {
        spawn(async move {
            let sure = confirm(
                "Thay toàn bộ dữ liệu trên máy này?",
                "Dữ liệu đang có trên máy này sẽ bị thay bằng bản sao lưu gần nhất trên Drive, cộng các thay đổi sau đó. \
                 Một bản của dữ liệu cũ được giữ cạnh file dữ liệu (tradelens.before-restore.db). \
                 Chỉ làm khi đây là máy mới. Không hoàn tác được.",
                "Thay dữ liệu",
            )
            .await;
            if sure {
                cloud.restore();
            }
        });
    };
    card(
        "Khôi phục & sao lưu",
        view()
            .gap_3()
            .child(
                text(
                    "Máy mới: đăng nhập cùng tài khoản rồi khôi phục từ bản chụp gần nhất của Drive (mỗi tuần một bản) \
                     và các thay đổi sau đó.",
                )
                .text_sm()
                .text_color(move || theme().muted_foreground),
            )
            .child(view().flex_row().child(action_icon_when(
                Icon::Upload,
                "Khôi phục sang máy này",
                Variant::Secondary,
                move || state.with(|s| s.ready && s.active().is_some() && !s.busy()),
                restore,
            ))),
    )
}

fn devices_card(cloud: Cloud) -> impl IntoElement {
    let state = cloud.state;
    card(
        "Thiết bị dùng chung dữ liệu",
        dynamic(move || {
            let (devices, mine) = state.with(|s| (s.devices.clone(), s.device_id.clone()));
            untracked(|| {
                if devices.is_empty() {
                    return text("Danh sách hiện sau lần sync đầu tiên.")
                        .text_sm()
                        .text_color(move || theme().muted_foreground)
                        .into_element();
                }
                view().children(devices.iter().map(|device| device_row(device, &mine))).into_element()
            })
        }),
    )
}

fn device_row(device: &DeviceInfo, mine: &str) -> impl IntoElement + use<> {
    let here = device.id == mine;
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .py_2()
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(text(device.name.clone()).text_sm().font_weight(FontWeight::MEDIUM))
                .when(here, |v| v.child(text("thiết bị này").text_xs().text_color(move || theme().muted_foreground))),
        )
        .child(text(when(device.last_sync)).text_xs().text_color(move || theme().muted_foreground))
}

fn access_card() -> impl IntoElement {
    card(
        "Quyền truy cập",
        view().gap_2().children(
            [
                "Chỉ thư mục ẩn của TradeLens trên Drive (appDataFolder / approot); app không thấy file khác của bạn.",
                "Không có server TradeLens: dữ liệu đi thẳng từ máy bạn lên Drive.",
                "Token đăng nhập nằm trong kho khoá của hệ điều hành (Windows Credential Manager, macOS Keychain), không nằm trong file dữ liệu.",
            ]
            .map(|line| {
                view()
                    .flex_row()
                    .gap_2()
                    .child(text("•").text_color(move || theme().primary))
                    .child(text(line).text_sm().flex_1().text_color(move || theme().muted_foreground))
            }),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sync_is_summed_up_in_one_line() {
        let mut report = SyncReport { pushed: 12, pulled: 3, ..SyncReport::default() };
        assert_eq!(counts(&report), "đẩy 12 thay đổi, nhận 3");
        report.conflicts_resolved = 2;
        report.attachments_uploaded = 1;
        report.snapshot_taken = true;
        let line = counts(&report);
        assert!(line.contains("2 chỗ hai máy sửa khác nhau") && line.contains("ảnh: 1 lên, 0 xuống"), "{line}");
        assert!(line.ends_with("đã chụp bản sao tuần"));
    }

    #[test]
    fn the_headline_follows_the_phase_the_last_outcome_and_the_database() {
        let idle = State::default();
        assert_eq!(headline(&idle).0, "Chưa đồng bộ");
        let known = State { last_sync: Some(0), ..State::default() };
        assert_eq!(headline(&known).0, "Đã đồng bộ");
        let failed = State {
            last: Some(Outcome { at: 0, result: Err("lỗi mạng: x".into()), restored: false }),
            ..State::default()
        };
        let (title, detail, bad) = headline(&failed);
        assert_eq!((title, bad), ("Sync lỗi", true));
        assert!(detail.contains("lỗi mạng: x"));
        let busy = State { phase: Phase::Syncing, ..State::default() };
        assert_eq!(headline(&busy).0, "Đang đồng bộ…");
    }
}
