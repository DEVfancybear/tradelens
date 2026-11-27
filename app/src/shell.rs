//! The window: the sidebar of routes, and the current route's page beside it.

use nsh_base::theme;
use nsh_core::prelude::*;
use tradelens_cloud::Provider;

use crate::{
    cloud::{self, Phase, State},
    nav::{self, Route, SECTIONS},
    pages::{self, search},
    ui::{
        controls::sidebar_row,
        icons::{Icon, icon},
        modal,
        palette::search_trigger,
        viewer,
    },
};

const SIDEBAR_WIDTH: f32 = 236.0;

pub(crate) fn shell() -> impl IntoElement {
    // Ctrl+K (Cmd+K on a Mac) opens the search from any page, and closes it again.
    on_key("mod+k", |_| search::toggle());
    view()
        .flex_1()
        .flex_row()
        .child(sidebar())
        .child(view().flex_1().min_w(px(0.0)).child(dynamic(move || {
            let route = nav::current();
            // "Tuỳ biến form" covers the page of the route; closing it shows the page again. Each
            // page is a keyed element of its own: a page made in the place of another one starts
            // at the top, instead of inheriting the scroll position of the page it replaced.
            match nav::form_editor() {
                Some(scope) => untracked(|| {
                    view()
                        .flex_1()
                        .min_h(px(0.0))
                        .key(format!("form-editor:{scope:?}"))
                        .child(pages::form_editor::form_editor(scope))
                }),
                None => untracked(|| {
                    view().flex_1().min_h(px(0.0)).key(format!("page:{route:?}")).child(pages::page(route))
                }),
            }
        })))
        // The modal that is on screen, one at a time: the plan prompt of a position the EA just
        // reported, and the share dialog opened from a trade's page and from the review.
        .child(modal::host())
        // The full-size picture viewer opened from a gallery; the search stays above it.
        .child(viewer::overlay())
        .child(search::overlay())
}

fn sidebar() -> impl IntoElement {
    view()
        .w(px(SIDEBAR_WIDTH))
        .px_3()
        .py_4()
        .gap_5()
        .bg(move || theme().sidebar)
        .border_r_1()
        .border_color(move || theme().sidebar_border)
        .child(brand())
        .child(search_trigger(search::open))
        .children(SECTIONS.iter().map(|section| {
            view()
                .gap_1()
                .child(
                    text(t!(section.title))
                        .px_3()
                        .pb_1()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
                .children(section.routes.iter().map(|&route| route_row(route)))
        }))
        .child(view().flex_1())
        .child(status())
}

fn brand() -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .child(
            view()
                .size(px(26.0))
                .rounded(px(7.0))
                .items_center()
                .justify_center()
                .bg(move || theme().primary)
                .child(icon(Icon::Search, 16.0).text_color(move || theme().primary_foreground)),
        )
        .child(text("TradeLens").text_base().font_weight(FontWeight::SEMIBOLD))
}

fn route_row(route: Route) -> impl IntoElement {
    sidebar_row(route.icon(), route.label(), move || nav::current() == route, move || nav::go(route))
}

/// The MT5 connection's and the sync's state, at the sidebar's foot.
fn status() -> impl IntoElement {
    let state = cloud::cloud().state;
    let connection = crate::mt5::mt5().state;
    let connected = computed(move || connection.with(mt5_line));
    view()
        .px_3()
        .gap_1()
        .child(text(move || connected.get()).text_xs().number_of_lines(1).text_color(move || theme().muted_foreground))
        .child(
            text(move || state.with(status_line))
                .text_xs()
                .number_of_lines(1)
                .text_color(move || theme().muted_foreground),
        )
}

/// `MT5: chưa kết nối`, or how many of the accounts signed in to are connected.
fn mt5_line(state: &crate::mt5::State) -> String {
    match (state.accounts.len(), state.connected()) {
        (0, _) => t!("shell.mt5.not_connected"),
        (1, 1) => t!("shell.mt5.connected"),
        (_, 0) => t!("shell.mt5.lost"),
        (total, connected) => t!("shell.mt5.accounts", connected = connected, total = total),
    }
}

/// `Cloud: chưa đăng nhập`, or the account's drive and how the last sync went.
fn status_line(state: &State) -> String {
    let Some(provider) = state.active() else { return t!("shell.cloud.signed_out") };
    let drive = match provider {
        Provider::Google => "Google Drive",
        Provider::Microsoft => "OneDrive",
    };
    let last = state.last.as_ref().map(|outcome| outcome.at).or(state.last_sync);
    match (state.phase, &state.last, last) {
        (Phase::Syncing, ..) => t!("shell.cloud.syncing", drive = drive),
        (Phase::Restoring, ..) => t!("shell.cloud.restoring", drive = drive),
        (_, Some(cloud::Outcome { result: Err(_), .. }), _) => t!("shell.cloud.failed", drive = drive),
        (_, _, Some(at)) => {
            let at = chrono::DateTime::from_timestamp_millis(at).map(|t| t.with_timezone(&chrono::Local));
            at.map_or_else(|| drive.to_owned(), |at| t!("shell.cloud.synced", drive = drive, at = at.format("%H:%M")))
        }
        (_, _, None) => t!("shell.cloud.never", drive = drive),
    }
}

#[cfg(test)]
mod tests {
    use tradelens_cloud::auth::Identity;

    use super::*;

    #[test]
    fn the_sidebar_says_who_is_signed_in_and_how_the_sync_went() {
        crate::i18n::with_text(crate::i18n::Lang::Vi, sidebar_lines);
    }

    fn sidebar_lines() {
        let mut state = State::default();
        assert_eq!(status_line(&state), "Cloud: chưa đăng nhập");
        state.accounts = vec![Identity { provider: Provider::Google, email: "a@b.c".into(), name: String::new() }];
        assert_eq!(status_line(&state), "Google Drive · chưa sync");
        state.phase = Phase::Syncing;
        assert_eq!(status_line(&state), "Google Drive · đang sync…");
        state.phase = Phase::Idle;
        state.last = Some(cloud::Outcome { at: 0, result: Err("x".into()), restored: false });
        assert_eq!(status_line(&state), "Google Drive · sync lỗi");
    }
}
