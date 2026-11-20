//! The routes: one page shows at a time, chosen in the sidebar.

use nsh_core::prelude::*;

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

    /// A one-glyph mark until the icon set lands (`ui::icons`, v0.1).
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            Self::Dashboard => "◧",
            Self::Trades => "≡",
            Self::Calendar => "▦",
            Self::Analytics => "◔",
            Self::Journal => "✎",
            Self::Review => "◫",
            Self::Missed => "⊘",
            Self::Playbook => "▤",
            Self::Accounts => "◎",
            Self::DataSources => "⇅",
            Self::Cloud => "☁",
            Self::Settings => "⚙",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Current(Route);

/// The route shown (tracked where it is read).
pub(crate) fn current() -> Route {
    global::<Current>().get().0
}

pub(crate) fn go(route: Route) {
    global::<Current>().set(Current(route));
}
