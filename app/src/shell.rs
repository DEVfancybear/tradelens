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
        palette::search_trigger,
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
            // "Tuỳ biến form" covers the page of the route; closing it shows the page again.
            match nav::form_editor() {
                Some(scope) => untracked(|| pages::form_editor::form_editor(scope).into_element()),
                None => untracked(|| pages::page(route)),
            }
        })))
        // The plan prompt of a position the EA just reported: over every screen.
        .child(pages::entry_plan::host())
        // The share dialog, opened from a trade's page and from the review.
        .child(pages::share::host())
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
                    text(section.title)
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

/// The sync's state, at the sidebar's foot.
fn status() -> impl IntoElement {
    let state = cloud::cloud().state;
    view()
        .px_3()
        .gap_1()
        .child(text(move || state.with(status_line)).text_xs().text_color(move || theme().muted_foreground))
}

/// `Cloud: chưa đăng nhập`, or the account's drive and how the last sync went.
fn status_line(state: &State) -> String {
    let Some(provider) = state.active() else { return "Cloud: chưa đăng nhập".to_owned() };
    let drive = match provider {
        Provider::Google => "Google Drive",
        Provider::Microsoft => "OneDrive",
    };
    let last = state.last.as_ref().map(|outcome| outcome.at).or(state.last_sync);
    match (state.phase, &state.last, last) {
        (Phase::Syncing, ..) => format!("{drive} · đang sync…"),
        (Phase::Restoring, ..) => format!("{drive} · đang khôi phục…"),
        (_, Some(cloud::Outcome { result: Err(_), .. }), _) => format!("{drive} · sync lỗi"),
        (_, _, Some(at)) => {
            let at = chrono::DateTime::from_timestamp_millis(at).map(|t| t.with_timezone(&chrono::Local));
            at.map_or_else(|| drive.to_owned(), |t| format!("{drive} · đã sync {}", t.format("%H:%M")))
        }
        (_, _, None) => format!("{drive} · chưa sync"),
    }
}

#[cfg(test)]
mod tests {
    use tradelens_cloud::auth::Identity;

    use super::*;

    #[test]
    fn the_sidebar_says_who_is_signed_in_and_how_the_sync_went() {
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
