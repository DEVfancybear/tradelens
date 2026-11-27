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
    News,
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
    /// The key of its title in `locales/`.
    pub(crate) title: &'static str,
    pub(crate) routes: &'static [Route],
}

pub(crate) const SECTIONS: &[Section] = &[
    Section {
        title: "shell.section.trading",
        routes: &[
            Route::Dashboard,
            Route::Trades,
            Route::Calendar,
            Route::News,
            Route::Analytics,
            Route::Journal,
            Route::Review,
            Route::Missed,
        ],
    },
    Section { title: "shell.section.strategy", routes: &[Route::Playbook] },
    Section { title: "shell.section.manage", routes: &[Route::Accounts, Route::DataSources, Route::Cloud] },
    Section { title: "shell.section.system", routes: &[Route::Settings] },
];

impl Route {
    /// The route's name, as the sidebar shows it.
    pub(crate) fn label(self) -> String {
        match self {
            Self::Dashboard => t!("nav.dashboard"),
            Self::Trades => t!("nav.trades"),
            Self::Calendar => t!("nav.calendar"),
            Self::News => t!("nav.news"),
            Self::Analytics => t!("nav.analytics"),
            Self::Journal => t!("nav.journal"),
            Self::Review => t!("nav.review"),
            Self::Missed => t!("nav.missed"),
            Self::Playbook => t!("nav.playbook"),
            Self::Accounts => t!("nav.accounts"),
            Self::DataSources => t!("nav.data_sources"),
            Self::Cloud => t!("nav.cloud"),
            Self::Settings => t!("nav.settings"),
        }
    }

    /// The sidebar's icon for the route.
    pub(crate) fn icon(self) -> Icon {
        match self {
            Self::Dashboard => Icon::Dashboard,
            Self::Trades => Icon::Trades,
            Self::Calendar => Icon::Calendar,
            Self::News => Icon::Bell,
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
        global::<EditTrade>().set(EditTrade(None));
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
        global::<EditTrade>().set(EditTrade(None));
        global::<OpenTrade>().set(OpenTrade(Some(key)));
    });
}

/// Closes the trade's page: back to the trades list.
pub(crate) fn close_trade() {
    batch(|| {
        global::<EditTrade>().set(EditTrade(None));
        global::<OpenTrade>().set(OpenTrade(None));
    });
}

/// The trade whose facts are being edited over its page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct EditTrade(Option<TradeKey>);

/// The trade being edited (tracked where it is read); its page is open under the form.
pub(crate) fn edit_trade() -> Option<TradeKey> {
    global::<EditTrade>().get().0
}

/// Opens the form that edits the facts of a trade over the trade's page.
pub(crate) fn open_edit_trade(key: TradeKey) {
    batch(|| {
        global::<Current>().set(Current(Route::Trades));
        global::<OpenTrade>().set(OpenTrade(Some(key.clone())));
        global::<EditTrade>().set(EditTrade(Some(key)));
    });
}

/// Closes the edit form: the trade's page shows again.
pub(crate) fn close_edit_trade() {
    global::<EditTrade>().set(EditTrade(None));
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
        global::<EditTrade>().set(EditTrade(None));
        global::<ManualEntry>().set(ManualEntry(true));
    });
}

/// Closes the manual entry form: back to the trades list.
pub(crate) fn close_manual_entry() {
    global::<ManualEntry>().set(ManualEntry(false));
}

/// A day's note the search or the calendar asked the journal to show: the day and the account whose note
/// it is (`None`: the note of all accounts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct JournalDay(Option<(chrono::NaiveDate, Option<i64>)>);

/// Shows the day journal on `day` (of `account_id`, `None` for all accounts).
pub(crate) fn show_journal_day(day: chrono::NaiveDate, account_id: Option<i64>) {
    batch(|| {
        global::<JournalDay>().set(JournalDay(Some((day, account_id))));
        go(Route::Journal);
    });
}

/// The day the search or the calendar asked the journal to show, until [`take_journal_day`] clears it
/// (tracked where it is read).
pub(crate) fn requested_journal_day() -> Option<(chrono::NaiveDate, Option<i64>)> {
    global::<JournalDay>().get().0
}

/// Clears the request once the journal has shown the day.
pub(crate) fn take_journal_day() {
    global::<JournalDay>().set(JournalDay(None));
}
