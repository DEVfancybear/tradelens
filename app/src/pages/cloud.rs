//! Đăng nhập & sync: sign in with Google or Microsoft, sync the journal through the account's
//! app folder, and restore it on a new machine. Design: `docs/design/screens/cloud-sync.html`.

use chrono::{DateTime, Local};
use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_cloud::{Provider, auth::Identity, sync::DeviceInfo, sync::SyncReport};

use crate::{
    cloud::{self as app_cloud, AutoSync, Cloud, Fault, Outcome, Phase, State, Why},
    ui::{
        chips::{Tone, chip},
        controls::{Variant, action, action_icon_when},
        forms::switch_row,
        icons::{Icon, icon},
        layout::columns,
        parts::{card, warning_banner},
        theme::loss,
    },
};

/// What a provider is called and how the card describes it: its name, its subtitle and the sign-in button.
fn describe(provider: Provider) -> (&'static str, String, String) {
    match provider {
        Provider::Google => ("Google Drive", t!("cloud.google.subtitle"), t!("cloud.google.sign_in")),
        Provider::Microsoft => ("OneDrive", t!("cloud.microsoft.subtitle"), t!("cloud.microsoft.sign_in")),
    }
}

pub(crate) fn cloud() -> impl IntoElement {
    let cloud = app_cloud::cloud();
    crate::ui::parts::page(
        t!("nav.cloud"),
        t!("cloud.page.summary"),
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
        untracked(|| line.map(|fault| warning_banner(fault.text()).into_element()))
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
                .child(dynamic(move || in_use().then(|| chip(t!("cloud.in_use"), Tone::Primary)))),
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
    missing: Option<Fault>,
) -> impl IntoElement {
    let (name, _, sign_in_label) = describe(provider);
    if let Some(me) = signed {
        // The question is the cloud's task, not this button's: this card is built again whenever the
        // state changes, and a task of the button would go with it before the alert shows.
        let sign_out = move || cloud.confirm_sign_out(provider, name);
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
            .child(view().flex_row().child(action(t!("cloud.sign_out"), Variant::Ghost, sign_out)))
            .into_element();
    }
    if phase == Phase::SigningIn(provider) {
        return view()
            .gap_3()
            .child(text(t!("cloud.waiting")).text_sm().text_color(move || theme().muted_foreground))
            .child(
                view()
                    .flex_row()
                    .child(action(t!("common.cancel"), Variant::Secondary, move || cloud.cancel_sign_in())),
            )
            .into_element();
    }
    view()
        .gap_3()
        .child(text(t!("cloud.signed_out")).text_sm().text_color(move || theme().muted_foreground))
        .children(missing.map(|why| text(why.text()).text_xs().text_color(crate::ui::theme::warning)))
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

/// A moment as the screen says it: `14:02 hôm nay` (`14:02 today`), or `18/11 22:10` for another day.
fn when(ms: i64) -> String {
    let Some(at) = DateTime::from_timestamp_millis(ms).map(|t| t.with_timezone(&Local)) else { return String::new() };
    if at.date_naive() == Local::now().date_naive() {
        t!("cloud.today", time = at.format("%H:%M"))
    } else {
        at.format("%d/%m %H:%M").to_string()
    }
}

/// What a sync did, in one line.
fn counts(report: &SyncReport) -> String {
    let mut parts = vec![t!("cloud.counts.moved", pushed = report.pushed, pulled = report.pulled)];
    if report.conflicts_resolved > 0 {
        parts.push(t!("cloud.counts.conflicts", n = report.conflicts_resolved));
    }
    if report.attachments_uploaded + report.attachments_downloaded > 0 {
        parts.push(t!("cloud.counts.pictures", up = report.attachments_uploaded, down = report.attachments_downloaded));
    }
    if report.attachments_failed > 0 {
        parts.push(t!("cloud.counts.pictures_failed", n = report.attachments_failed));
    }
    if report.snapshot_taken {
        parts.push(t!("cloud.counts.snapshot"));
    }
    parts.join(" · ")
}

/// The headline and the detail line of the sync card for the state.
fn headline(state: &State) -> (String, String, bool) {
    match (state.phase, &state.last) {
        (Phase::Syncing, _) => (t!("cloud.headline.syncing"), t!("cloud.headline.syncing_detail"), false),
        (Phase::Restoring, _) => (t!("cloud.headline.restoring"), t!("cloud.headline.restoring_detail"), false),
        (_, Some(Outcome { at, result: Err(fault), .. })) => (
            t!("cloud.headline.failed"),
            t!("cloud.headline.failed_detail", when = when(*at), message = fault.text()),
            true,
        ),
        (_, Some(Outcome { at, result: Ok(report), restored })) => (
            if *restored { t!("cloud.headline.restored") } else { t!("cloud.headline.synced") },
            t!("cloud.headline.last_counts", when = when(*at), counts = counts(report)),
            false,
        ),
        (_, None) => match state.last_sync {
            Some(at) => (t!("cloud.headline.synced"), t!("cloud.headline.last", when = when(at)), false),
            None => (t!("cloud.headline.never"), t!("cloud.headline.never_detail"), false),
        },
    }
}

fn sync_card(cloud: Cloud) -> impl IntoElement {
    let state = cloud.state;
    card(
        t!("cloud.sync.title"),
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
                        t!("cloud.sync.now"),
                        Variant::Primary,
                        move || state.with(|s| s.ready && s.active().is_some() && !s.busy()),
                        move || cloud.sync(Why::Pressed),
                    )),
            )
            .child(dynamic(move || {
                // The button is off while nobody is signed in: say what turns it on.
                let signed_out = state.with(|s| s.ready && s.active().is_none());
                signed_out.then(|| {
                    text(t!("cloud.sync.sign_in_first")).text_sm().text_color(move || theme().muted_foreground)
                })
            }))
            .child(dynamic(move || state.with(|s| s.env.is_demo()).then(|| chip(t!("cloud.sync.demo"), Tone::Neutral))))
            .child(switch_row(
                t!("cloud.sync.auto"),
                Some(t!("cloud.sync.auto_hint")),
                move || state.with(|s| s.env.auto_sync()),
                move |on| cloud.set_auto_sync(if on { AutoSync::On } else { AutoSync::Off }),
            )),
    )
}

fn restore_card(cloud: Cloud) -> impl IntoElement {
    let state = cloud.state;
    let restore = move || cloud.confirm_restore();
    card(
        t!("cloud.restore.title"),
        view()
            .gap_3()
            .child(text(t!("cloud.restore.body")).text_sm().text_color(move || theme().muted_foreground))
            .child(view().flex_row().child(action_icon_when(
                Icon::Upload,
                t!("cloud.restore.button"),
                Variant::Secondary,
                move || state.with(|s| s.ready && s.active().is_some() && !s.busy()),
                restore,
            ))),
    )
}

fn devices_card(cloud: Cloud) -> impl IntoElement {
    let state = cloud.state;
    card(
        t!("cloud.devices.title"),
        dynamic(move || {
            let (devices, mine) = state.with(|s| (s.devices.clone(), s.device_id.clone()));
            untracked(|| {
                if devices.is_empty() {
                    return text(t!("cloud.devices.empty"))
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
                .when(here, |v| {
                    v.child(text(t!("cloud.devices.this_one")).text_xs().text_color(move || theme().muted_foreground))
                }),
        )
        .child(text(when(device.last_sync)).text_xs().text_color(move || theme().muted_foreground))
}

fn access_card() -> impl IntoElement {
    card(
        t!("cloud.access.title"),
        view().gap_2().children(
            [t!("cloud.access.folder"), t!("cloud.access.no_server"), t!("cloud.access.token")].map(|line| {
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
        let said = |report: &SyncReport, lang| {
            let report = *report;
            crate::i18n::with_text(lang, move || counts(&report))
        };
        assert_eq!(said(&report, crate::i18n::Lang::Vi), "đẩy 12 thay đổi, nhận 3");
        assert_eq!(said(&report, crate::i18n::Lang::En), "pushed 12 changes, received 3");
        report.conflicts_resolved = 2;
        report.attachments_uploaded = 1;
        report.attachments_failed = 3;
        report.snapshot_taken = true;
        let line = said(&report, crate::i18n::Lang::Vi);
        assert!(line.contains("2 chỗ hai máy sửa khác nhau") && line.contains("ảnh: 1 lên, 0 xuống"), "{line}");
        assert!(line.contains("3 ảnh chưa chuyển được"), "{line}");
        assert!(line.ends_with("đã chụp bản sao tuần"));
    }

    #[test]
    fn the_headline_follows_the_phase_the_last_outcome_and_the_database() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, || {
            let idle = State::default();
            assert_eq!(headline(&idle).0, "Chưa đồng bộ");
            let known = State { last_sync: Some(0), ..State::default() };
            assert_eq!(headline(&known).0, "Đã đồng bộ");
            let failed = State {
                last: Some(Outcome { at: 0, result: Err(Fault::Words("lỗi mạng: x".into())), restored: false }),
                ..State::default()
            };
            let (title, detail, bad) = headline(&failed);
            assert_eq!((title.as_str(), bad), ("Sync lỗi", true));
            assert!(detail.contains("lỗi mạng: x"));
            let busy = State { phase: Phase::Syncing, ..State::default() };
            assert_eq!(headline(&busy).0, "Đang đồng bộ…");
        });
    }
}
