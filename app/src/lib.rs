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

mod cloud;
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
use nsh_core::prelude::*;

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

/// The root: the main window's content, [`Workspace`]. The platform makes the window itself, sized by
/// the `window` of `nsh.json` ([`root`]); a `window()` element here would open a second window of
/// its own, at the default size of a dialog.
#[component]
pub fn App() -> impl IntoElement {
    workspace()
}

/// The text of `nsh.json`: the app's id and the main window's size come from it, whoever starts the app.
const MANIFEST: &str = include_str!("../nsh.json");

/// The bundled font files (`assets/fonts/`): Inter for text, `JetBrains Mono` for figures.
///
/// `cargo run` starts the platform without the app's UI set, so the `registry.assets(..)` fonts
/// of [`native`] are never read and every family would fall back to the system's. The root
/// registers the bytes itself, before the first render, so `font-sans` / `font-mono` of
/// `theme.json` resolve to these faces however the app is started.
const FONTS: [&[u8]; 7] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-SemiBold.ttf"),
];

/// The app's root as the runner needs it: [`App`] with what `nsh.json` declares. NSH's own tooling
/// (`nsh run`, `nsh package`) generates this from the file; `cargo run` has to do it here, because
/// without the id the data folder is the one every app without an id shares (`default`), and
/// without the window options the window opens at a default size.
///
/// # Panics
///
/// When `nsh.json` is not valid JSON or has no `id`; a test checks it.
pub fn root() -> nsh_core::element::Root {
    let manifest: serde_json::Value = serde_json::from_str(MANIFEST).expect("nsh.json is valid JSON");
    let id = manifest["id"].as_str().expect("nsh.json has an id");
    let root = FONTS.iter().fold(nsh_core::element::Root::of::<App>().app_id(id), |root, font| root.font(*font));
    // The theme's tokens are there from the first frame, with the family names of its fonts.
    let root = match ui::theme::tokens() {
        Some(tokens) => root.tokens(tokens),
        None => root,
    };
    match manifest.get("window") {
        Some(window) => root.main_window_json(&window.to_string()),
        None => root,
    }
}

/// The folder name of the app's data inside the system's folder for apps (`%APPDATA%` on Windows):
/// the id of `nsh.json`.
pub const APP_ID: &str = "vn.dat.tradelens";

/// The main window's content: theme, store and shell. A component of its own so
/// headless tests mount it without a desktop window.
#[component]
pub fn Workspace() -> impl IntoElement {
    ui::theme::install();
    store::provide();
    ui::controls::provide_root_owner();
    cloud::provide();
    pages::follow_appearance();
    view().flex_1().bg(move || theme().background).text_color(move || theme().foreground).child(shell::shell())
}

#[cfg(test)]
mod tests {
    use nsh_core::{
        element::AssetKind,
        prelude::{assets, px},
    };

    #[test]
    fn the_root_has_the_apps_own_id_so_its_data_is_not_in_the_folder_shared_by_apps_without_one() {
        let root = super::root();
        assert_eq!(root.app_id_ref(), Some(super::APP_ID));
        assert_eq!(nsh_engine::app_dir_name(root.app_id_ref()), "vn.dat.tradelens");
        assert_ne!(nsh_engine::app_dir_name(None), "vn.dat.tradelens", "`default` is what an app without an id gets");
    }

    #[test]
    fn the_main_window_opens_at_1280_by_800_points_and_cannot_shrink_below_the_design_minimum() {
        let root = super::root();
        let window = root.main_window_ref().expect("nsh.json declares the window");
        assert_eq!(window.default_size, Some(nsh_core::geometry::size(px(1280.0), px(800.0))));
        assert_eq!(window.attributes.min_size, Some(Some(nsh_core::geometry::size(px(1024.0), px(680.0)))));
        assert!(!window.restore_bounds, "a window left tiny once must not come back tiny");
    }

    #[test]
    fn the_root_registers_the_fonts_under_the_families_theme_json_names() {
        let root = super::root();
        let families: Vec<String> = root
            .fonts()
            .iter()
            .map(|data| nsh_core::font_family(data).expect("every bundled font has a family name"))
            .collect();
        let tokens = nsh_base::tokens_of(Some(include_str!("../theme.json"))).expect("theme.json parses");
        for token in [nsh_core::element::FONT_SANS, nsh_core::element::FONT_MONO] {
            let named = tokens.font(token).unwrap_or_else(|| panic!("theme.json has `{token}`"));
            assert!(
                families.iter().any(|family| family == &**named),
                "`{token}` names {named:?}, but the registered families are {families:?}"
            );
        }
        let mono = families.iter().filter(|family| family.as_str() == "JetBrains Mono").count();
        assert_eq!(mono, 3, "Regular, Medium and SemiBold: {families:?}");
    }

    #[test]
    fn the_bundled_fonts_are_registered_with_the_app() {
        let fonts: Vec<&str> =
            assets!().iter().filter(|file| file.kind == AssetKind::Font).map(|file| file.uri).collect();
        assert_eq!(fonts.len(), 7, "Inter 400 to 700 and JetBrains Mono 400 to 600: {fonts:?}");
        assert!(fonts.iter().any(|uri| uri.ends_with("Inter-Regular.ttf")));
        assert!(fonts.iter().any(|uri| uri.ends_with("JetBrainsMono-Regular.ttf")));
    }
}
