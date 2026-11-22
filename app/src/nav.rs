//! The routes: one page shows at a time, chosen in the sidebar.

use nsh_core::prelude::*;
use tradelens_core::{TradeKey, catalog::FieldScope};

use crate::ui::icons::Icon;

/// A screen of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) enum Route {
    #[default]
    Dashboard,
    Trades,
    Calendar,
    Analytics,
    Journal,
    Review,
    Missed,
    Playbook,
    Accounts,
    DataSources,
    Cloud,
    Settings,
}

/// A group of routes in the sidebar.
pub(crate) struct Section {
    pub(crate) title: &'static str,
    pub(crate) routes: &'static [Route],
}

pub(crate) const SECTIONS: &[Section] = &[
    Section {
        title: "Giao dịch",
        routes: &[
            Route::Dashboard,
            Route::Trades,
            Route::Calendar,
            Route::Analytics,
            Route::Journal,
            Route::Review,
            Route::Missed,
        ],
    },
    Section { title: "Chiến lược", routes: &[Route::Playbook] },
    Section { title: "Quản lý", routes: &[Route::Accounts, Route::DataSources, Route::Cloud] },
    Section { title: "Hệ thống", routes: &[Route::Settings] },
];

impl Route {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Tổng quan",
            Self::Trades => "Lệnh",
            Self::Calendar => "Lịch P/L",
            Self::Analytics => "Phân tích",
            Self::Journal => "Nhật ký ngày",
            Self::Review => "Tổng kết & xem lại",
            Self::Missed => "Lệnh bỏ lỡ",
            Self::Playbook => "Playbook",
            Self::Accounts => "Tài khoản & kỷ luật",
            Self::DataSources => "Nguồn dữ liệu",
            Self::Cloud => "Đăng nhập & sync",
            Self::Settings => "Cài đặt",
        }
    }

    /// The sidebar's icon for the route.
    pub(crate) fn icon(self) -> Icon {
        match self {
            Self::Dashboard => Icon::Dashboard,
            Self::Trades => Icon::Trades,
            Self::Calendar => Icon::Calendar,
            Self::Analytics => Icon::Analytics,
            Self::Journal => Icon::Journal,
            Self::Review => Icon::Review,
            Self::Missed => Icon::Missed,
            Self::Playbook => Icon::Playbook,
            Self::Accounts => Icon::Accounts,
            Self::DataSources => Icon::DataSources,
            Self::Cloud => Icon::Cloud,
            Self::Settings => Icon::Settings,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Current(Route);

/// The route shown (tracked where it is read).
pub(crate) fn current() -> Route {
    global::<Current>().get().0
}

/// Shows a route; a trade's page open over the trades list closes.
pub(crate) fn go(route: Route) {
    batch(|| {
        global::<OpenTrade>().set(OpenTrade(None));
        global::<FormEditor>().set(FormEditor(None));
        global::<ManualEntry>().set(ManualEntry(false));
        global::<Current>().set(Current(route));
    });
}

/// The trade whose page is open over the trades list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct OpenTrade(Option<TradeKey>);

/// The trade whose page is open (tracked where it is read).
pub(crate) fn open_trade() -> Option<TradeKey> {
    global::<OpenTrade>().get().0
}

/// Opens a trade's page over the trades list, from any route.
pub(crate) fn show_trade(key: TradeKey) {
    batch(|| {
        global::<Current>().set(Current(Route::Trades));
        global::<OpenTrade>().set(OpenTrade(Some(key)));
    });
}

/// Closes the trade's page: back to the trades list.
pub(crate) fn close_trade() {
    global::<OpenTrade>().set(OpenTrade(None));
}

/// The form editor open over the current page, with the form it starts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct FormEditor(Option<FieldScope>);

/// The form the editor is open on (tracked where it is read); `None` while it is closed.
pub(crate) fn form_editor() -> Option<FieldScope> {
    global::<FormEditor>().get().0
}

/// Opens "Tuỳ biến form" over whatever page is showing, on the tab of `scope`. Closing it
/// shows that page again.
pub(crate) fn open_form_editor(scope: FieldScope) {
    global::<FormEditor>().set(FormEditor(Some(scope)));
}

/// Closes the form editor.
pub(crate) fn close_form_editor() {
    global::<FormEditor>().set(FormEditor(None));
}

/// Whether the manual entry form is open over the trades list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ManualEntry(bool);

/// Whether the manual entry form is open (tracked where it is read).
pub(crate) fn manual_entry_open() -> bool {
    global::<ManualEntry>().get().0
}

/// Opens the manual entry form over the trades list, from any route.
pub(crate) fn open_manual_entry() {
    batch(|| {
        global::<Current>().set(Current(Route::Trades));
        global::<OpenTrade>().set(OpenTrade(None));
        global::<ManualEntry>().set(ManualEntry(true));
    });
}

/// Closes the manual entry form: back to the trades list.
pub(crate) fn close_manual_entry() {
    global::<ManualEntry>().set(ManualEntry(false));
}
