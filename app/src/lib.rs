//! TradeLens's UI, in NSH's Rust mode: `nsh.json` `"root"` names [`App`].
//!
//! - `shell`      the window: sidebar and the page of the current route
//! - `nav`        the routes, and the trade whose page is open
//! - `store`      the app's state over `tradelens-core`; reads the database on blocking threads
//! - `snapshot`   what the pages show, read in one go (pure over a `Db`)
//! - `trade_list` the trades page's filters and sort order
//! - `journal_draft` the journal fields the trade page edits
//! - `zone`       the clock times are shown on (the `time_display` setting) and "today"
//! - `format`     numbers and times as text
//! - `month_report` the week or month a share card or the monthly PDF is made from
//! - `insight`   the calendar, analytics and discipline figures read out of a snapshot
//! - `ui`         the app's styled components and tokens; pages use only these
//! - `pages`      one module per screen

mod format;
mod insight;
mod journal_draft;
mod month_report;
mod nav;
mod pages;
mod shell;
mod snapshot;
mod store;
mod trade_list;
pub mod ui;
mod zone;

use nsh_base::theme;
use nsh_core::{element::desktop::window as app_window, prelude::*};

pub use store::{Reading, Writer, data_changed, query, query_with, writer};

/// The app's UI set (`nsh.json` `"rust"`).
#[native_module("tradelens/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
    registry.register::<Workspace>();
    // Inter and JetBrains Mono (`assets/fonts/`): the desktop does not download web fonts, so
    // the families `theme.json` names resolve to these files.
    registry.assets(assets!());
}

/// The root: the main window, showing [`Workspace`].
#[component]
pub fn App() -> impl IntoElement {
    app_window().id("main").content(workspace)
}

/// The main window's content: theme, store and shell. A component of its own so
/// headless tests mount it without a desktop window.
#[component]
pub fn Workspace() -> impl IntoElement {
    ui::theme::install();
    store::provide();
    pages::follow_appearance();
    view().flex_1().bg(move || theme().background).text_color(move || theme().foreground).child(shell::shell())
}

#[cfg(test)]
mod tests {
    use nsh_core::{element::AssetKind, prelude::assets};

    #[test]
    fn the_bundled_fonts_are_registered_with_the_app() {
        let fonts: Vec<&str> =
            assets!().iter().filter(|file| file.kind == AssetKind::Font).map(|file| file.uri).collect();
        assert_eq!(fonts.len(), 7, "Inter 400 to 700 and JetBrains Mono 400 to 600: {fonts:?}");
        assert!(fonts.iter().any(|uri| uri.ends_with("Inter-Regular.ttf")));
        assert!(fonts.iter().any(|uri| uri.ends_with("JetBrainsMono-Regular.ttf")));
    }
}
