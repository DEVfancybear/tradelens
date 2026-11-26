//! The icon set of the design: 24 × 24 line glyphs, stroke 1.75, round caps and joins
//! (`docs/design/screens/*.html`). The paths are copied from the design's SVG; an icon is
//! drawn in the text colour of where it sits, so set `.text_color(..)` on it or its parent.

use std::fmt::Write;

use nsh_core::{element::ImageSrc, prelude::*};

/// A glyph of the set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    // Sidebar.
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
    /// Not in the design's SVG: the sidebar mark of the red news.
    Bell,
    // Actions and marks.
    Search,
    Plus,
    /// A horizontal bar, for "zoom out".
    Minus,
    Check,
    Close,
    Warning,
    Eye,
    EyeOff,
    Trash,
    Edit,
    Download,
    Upload,
    Share,
    /// Two overlapping pages, for "Sao chép".
    Copy,
    Refresh,
    History,
    Filter,
    ToTop,
    ArrowDown,
    ChevronLeft,
    /// Not in the design's SVG; the mirror of [`Icon::ChevronLeft`], for "next" buttons.
    ChevronRight,
    /// Not in the design's SVG; opens a drop-down (the month of the calendar).
    ChevronDown,
    // Things.
    Document,
    Folder,
    Image,
    Monitor,
    Clock,
}

impl Icon {
    /// Every glyph, in declaration order.
    pub const ALL: [Self; 40] = [
        Self::Dashboard,
        Self::Trades,
        Self::Calendar,
        Self::Analytics,
        Self::Journal,
        Self::Review,
        Self::Missed,
        Self::Playbook,
        Self::Accounts,
        Self::DataSources,
        Self::Cloud,
        Self::Settings,
        Self::Bell,
        Self::Search,
        Self::Plus,
        Self::Minus,
        Self::Check,
        Self::Close,
        Self::Warning,
        Self::Eye,
        Self::EyeOff,
        Self::Trash,
        Self::Edit,
        Self::Download,
        Self::Upload,
        Self::Share,
        Self::Copy,
        Self::Refresh,
        Self::History,
        Self::Filter,
        Self::ToTop,
        Self::ArrowDown,
        Self::ChevronLeft,
        Self::ChevronRight,
        Self::ChevronDown,
        Self::Document,
        Self::Folder,
        Self::Image,
        Self::Monitor,
        Self::Clock,
    ];

    /// The SVG path data of the glyph, in a 24 × 24 box.
    pub fn path(self) -> &'static str {
        match self {
            Self::Dashboard => "M3 3h7v9H3zM14 3h7v5h-7zM14 12h7v9h-7zM3 16h7v5H3z",
            Self::Trades => "M4 6h16M4 12h16M4 18h10",
            Self::Calendar => "M4 5h16v15H4zM4 10h16M9 3v4M15 3v4",
            Self::Analytics => "M4 20V10M10 20V4M16 20v-7M22 20H2",
            Self::Journal => "M5 4h11l3 3v13H5zM9 9h6M9 13h6M9 17h4",
            Self::Review => "M4 4h16v12H8l-4 4zM8 8h8M8 12h5",
            Self::Missed => "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM9 9l6 6M15 9l-6 6",
            Self::Playbook => "M4 5a2 2 0 0 1 2-2h12v16H6a2 2 0 0 0-2 2zM4 5v16M9 7h6",
            Self::Accounts => "M12 3l8 4v5c0 5-3.5 8-8 9-4.5-1-8-4-8-9V7z",
            Self::DataSources => "M7 4v13M7 17l-3-3M7 17l3-3M17 20V7M17 7l-3 3M17 7l3 3",
            Self::Cloud => "M7 18h10a4 4 0 0 0 .5-8 6 6 0 0 0-11.5 1.5A3.5 3.5 0 0 0 7 18z",
            Self::Settings => {
                "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1"
            }
            Self::Bell => "M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10.3 21a1.94 1.94 0 0 0 3.4 0",
            Self::Search => "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM20 20l-4-4",
            Self::Plus => "M12 5v14M5 12h14",
            Self::Minus => "M5 12h14",
            Self::Check => "M5 12l5 5L20 7",
            Self::Close => "M6 6l12 12M18 6L6 18",
            Self::Warning => {
                "M12 9v4M12 17h.01M10.3 3.9L1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z"
            }
            Self::Eye => "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12zM12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z",
            Self::EyeOff => {
                "M3 3l18 18M10.6 10.6a3 3 0 0 0 4 4M9.9 5.1A10 10 0 0 1 22 12s-1 2-3 4M6.6 6.6C3.9 8.4 2 12 2 12s3.5 7 10 7c1.7 0 3.2-.4 4.5-1"
            }
            Self::Trash => "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13",
            Self::Edit => "M4 20h4L19 9l-4-4L4 16z",
            Self::Download => "M12 4v12M7 11l5 5 5-5M5 20h14",
            Self::Upload => "M12 16V4M7 9l5-5 5 5M4 16v4h16v-4",
            Self::Share => "M4 12v7h16v-7M12 3v12M8 7l4-4 4 4",
            Self::Copy => "M8 8h12v12H8zM4 16V4h12",
            Self::Refresh => "M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7",
            Self::History => "M3 12a9 9 0 1 0 3-6.7M3 4v5h5",
            Self::Filter => "M4 6h16M7 12h10M10 18h4",
            Self::ToTop => "M12 20V8M7 13l5-5 5 5M4 4h16",
            Self::ArrowDown => "M12 4v12M7 11l5 5 5-5",
            Self::ChevronLeft => "M15 18l-6-6 6-6",
            Self::ChevronRight => "M9 6l6 6-6 6",
            Self::ChevronDown => "M6 9l6 6 6-6",
            Self::Document => "M5 4h11l3 3v13H5z",
            Self::Folder => "M3 7h6l2 2h10v10H3z",
            Self::Image => "M4 5h16v14H4zM4 15l4-4 5 5 3-3 4 4M15 9h.01",
            Self::Monitor => "M3 5h18v11H3zM8 20h8M12 16v4",
            Self::Clock => "M12 8v4l3 2M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z",
        }
    }

    /// The glyph as an SVG document drawn `size` points wide, stroked in `currentColor`.
    fn markup(self, size: f32) -> String {
        let mut out = String::with_capacity(self.path().len() + 220);
        // Writing to a String cannot fail.
        let _ = write!(
            out,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{size}\" height=\"{size}\" viewBox=\"0 0 24 24\" \
             fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.75\" stroke-linecap=\"round\" \
             stroke-linejoin=\"round\"><path d=\"{}\"/></svg>",
            self.path()
        );
        out
    }
}

/// An icon `size` points square, drawn in the text colour it inherits (set it with
/// `.text_color(..)` on the icon or on a parent). Decorative: a control that holds only an
/// icon names itself with `accessibility_label`, see `controls::icon_action`.
pub fn icon(icon: Icon, size: f32) -> Svg {
    svg(ImageSrc::svg_markup(&icon.markup(size))).size(px(size)).flex_shrink_0()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use nsh_engine::{EngineConfig, Root};
    use nsh_testing::Headless;

    use super::*;

    #[test]
    fn every_icon_has_its_own_path() {
        let paths: HashSet<&str> = Icon::ALL.iter().map(|i| i.path()).collect();
        assert_eq!(paths.len(), Icon::ALL.len(), "two icons share a path");
        assert!(Icon::ALL.iter().all(|i| !i.path().is_empty()));
    }

    #[test]
    fn the_markup_is_the_designs_stroke_in_the_current_colour() {
        let markup = Icon::Plus.markup(18.0);
        assert!(markup.contains("stroke-width=\"1.75\""), "{markup}");
        assert!(markup.contains("stroke=\"currentColor\""), "{markup}");
        assert!(markup.contains("d=\"M12 5v14M5 12h14\""), "{markup}");
        assert!(markup.contains("width=\"18\""), "{markup}");
    }

    #[component]
    fn Harness() -> impl IntoElement {
        view().child(icon(Icon::Search, 18.0).test_id("search-icon")).child(text("beside the icon"))
    }

    #[test]
    fn an_icon_is_placed_in_the_tree() {
        let mut app = Headless::start(EngineConfig::new(Root::of::<Harness>())).expect("the engine starts");
        app.wait_for("beside the icon").expect("the harness shows");
        assert_eq!(app.screen().count("id:search-icon").expect("a valid selector"), 1);
    }
}
