//! The share card as a PNG: an SVG laid out from a [`ShareCard`], rasterized by `resvg` with the
//! bundled fonts. The card is always the dark look of the design (`share-card`), whatever theme
//! the app is in, so it reads the same wherever it is posted.

use std::{fmt::Write as _, path::Path};

use base64::{Engine, engine::general_purpose::STANDARD};
use resvg::{tiny_skia, usvg};

use super::{Outcome, ReportError, ShareCard, fonts::FontFace};

/// The card's size in pixels: the 1200 × 630 most sites show as a link preview.
const WIDTH: u32 = 1200;
const HEIGHT: u32 = 630;

// The design's dark tokens (`docs/design/README.md`, `app/theme.json`), as literals: the card is
// an exported image, so it cannot follow the theme the app happens to show.
const BACKGROUND: &str = "#0b0e14";
const GLOW: &str = "#13302d";
const EDGE: &str = "#232b3a";
const FOREGROUND: &str = "#e6eaf2";
const MUTED: &str = "#8a94a7";
const PRIMARY: &str = "#2dd4bf";
const PRIMARY_FOREGROUND: &str = "#04201c";
const PROFIT: &str = "#34d399";
const LOSS: &str = "#f87171";

/// Space around the content, and where the right-hand panel (curve or chart) starts.
const MARGIN: f32 = 60.0;
const PANEL_X: f32 = 640.0;
const PANEL_WIDTH: f32 = 500.0;
/// The widest a figure may be set beside a panel, and without one.
const NARROW_RESULT: f32 = 560.0;
const WIDE_RESULT: f32 = 1080.0;
/// The most characters of one footer value.
const VALUE_CHARS: usize = 16;

/// Draws `card` as a 1200 × 630 PNG. The chart image, when the card has one and the file reads,
/// is drawn at the right; a file that cannot be read is left out rather than failing the card.
///
/// # Errors
///
/// [`ReportError::Render`] when the picture cannot be built or encoded.
pub fn render_png(card: &ShareCard) -> Result<Vec<u8>, ReportError> {
    let svg = card_svg(card);
    let mut options = usvg::Options { font_family: "Inter".to_owned(), ..usvg::Options::default() };
    let fonts = options.fontdb_mut();
    for face in FontFace::ALL {
        fonts.load_font_data(face.bytes().to_vec());
    }
    let tree = usvg::Tree::from_str(&svg, &options).map_err(|e| ReportError::Render(e.to_string()))?;
    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, HEIGHT)
        .ok_or_else(|| ReportError::Render(crate::tr!("core.report.bad_image_size")))?;
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| ReportError::Render(e.to_string()))
}

fn escape(text: &str) -> String {
    text.chars().fold(String::with_capacity(text.len()), |mut out, c| {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
        out
    })
}

/// `text` cut to `limit` characters with an ellipsis.
fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

fn tone(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Win => PROFIT,
        Outcome::Loss => LOSS,
        Outcome::Flat => MUTED,
    }
}

/// An SVG `<text>` element at `(x, y)` set in `face`.
fn text(x: f32, y: f32, size: f32, face: FontFace, fill: &str, content: &str) -> String {
    let (family, weight) = face.css();
    format!(
        "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"{family}\" font-weight=\"{weight}\" font-size=\"{size:.1}\" \
         fill=\"{fill}\">{}</text>",
        escape(content)
    )
}

/// The largest size that sets `chars` characters, `em` ems wide each, within `limit`, at most `max`.
fn fit(chars: usize, em: f32, limit: f32, max: f32) -> f32 {
    let count = f32::from(u16::try_from(chars).unwrap_or(u16::MAX));
    max.min(limit / (count.max(1.0) * em))
}

/// The curve as an area and a line inside the panel; a flat or single-point curve is a straight line.
fn curve(points: &[f64], outcome: Outcome) -> String {
    // Drawing positions, in f64 so that no cast is needed.
    const TOP: f64 = 190.0;
    const BOTTOM: f64 = 410.0;
    const LEFT: f64 = 640.0;
    const SPAN: f64 = 500.0;
    if points.is_empty() {
        return String::new();
    }
    let (low, high) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &p| (lo.min(p), hi.max(p)));
    let flat = (high - low).abs() < f64::EPSILON;
    let last = f64::from(u32::try_from(points.len().saturating_sub(1).max(1)).unwrap_or(u32::MAX));
    let place = |i: usize, value: f64| {
        let step = f64::from(u32::try_from(i).unwrap_or(u32::MAX)) / last;
        let fraction = if flat { 0.5 } else { (value - low) / (high - low) };
        (LEFT + step * SPAN, BOTTOM - fraction * (BOTTOM - TOP))
    };
    let line: Vec<(f64, f64)> = points.iter().enumerate().map(|(i, &p)| place(i, p)).collect();
    let path = |close: bool| {
        let mut d = String::new();
        for (i, (x, y)) in line.iter().enumerate() {
            let _ = write!(d, "{}{x:.1} {y:.1} ", if i == 0 { "M" } else { "L" });
        }
        if close {
            let (first, _) = line[0];
            let (last_x, _) = line[line.len() - 1];
            let _ = write!(d, "L{last_x:.1} {BOTTOM:.1} L{first:.1} {BOTTOM:.1} Z");
        }
        d
    };
    let stroke = if outcome == Outcome::Loss { LOSS } else { PRIMARY };
    format!(
        "<path d=\"{}\" fill=\"url(#area)\" stroke=\"none\"/>\
         <path d=\"{}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"5\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>",
        path(true),
        path(false)
    )
}

/// The chart image, clipped to a rounded panel, or nothing when the file is missing or not an image.
fn chart(path: &str) -> String {
    let mime = match Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => return String::new(),
    };
    let Ok(bytes) = std::fs::read(path) else {
        log::warn!("share card: the chart image {path} cannot be read");
        return String::new();
    };
    format!(
        "<clipPath id=\"shot\"><rect x=\"{PANEL_X}\" y=\"150\" width=\"{PANEL_WIDTH}\" height=\"290\" rx=\"16\"/></clipPath>\
         <image x=\"{PANEL_X}\" y=\"150\" width=\"{PANEL_WIDTH}\" height=\"290\" clip-path=\"url(#shot)\" \
         preserveAspectRatio=\"xMidYMid slice\" href=\"data:{mime};base64,{}\"/>\
         <rect x=\"{PANEL_X}\" y=\"150\" width=\"{PANEL_WIDTH}\" height=\"290\" rx=\"16\" fill=\"none\" stroke=\"{EDGE}\" stroke-width=\"2\"/>",
        STANDARD.encode(bytes)
    )
}

/// The foot of the card: each figure's label over its value, left to right while there is room.
fn footer(stats: &[(String, String)]) -> String {
    const LABEL_Y: f32 = 508.0;
    const VALUE_Y: f32 = 562.0;
    const GAP: f32 = 56.0;
    let mut out = String::new();
    let mut x = MARGIN;
    for (label, value) in stats {
        let value = clip(value, VALUE_CHARS);
        let chars = |s: &str| f32::from(u16::try_from(s.chars().count()).unwrap_or(u16::MAX));
        let width = (chars(label) * 14.0).max(chars(&value) * 24.0);
        if x + width > WIDE_RESULT + MARGIN {
            break;
        }
        out.push_str(&text(x, LABEL_Y, 24.0, FontFace::Regular, MUTED, label));
        out.push_str(&text(x, VALUE_Y, 40.0, FontFace::MonoSemiBold, FOREGROUND, &value));
        x += width + GAP;
    }
    out
}

/// The card as an SVG document of [`WIDTH`] × [`HEIGHT`] units.
pub(super) fn card_svg(card: &ShareCard) -> String {
    let color = tone(card.outcome);
    let headline = card.headline();
    let chart_svg = card.chart.as_deref().map_or_default(chart);
    let beside = !chart_svg.is_empty() || !card.curve.is_empty();
    let limit = if beside { NARROW_RESULT } else { WIDE_RESULT };
    let is_word = headline.main.chars().all(|c| c.is_alphabetic() || c.is_whitespace());
    let (face, em) = if is_word { (FontFace::Bold, 0.58) } else { (FontFace::MonoSemiBold, 0.6) };
    let main_size = fit(headline.main.chars().count(), em, limit, 96.0);

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{HEIGHT}\" viewBox=\"0 0 {WIDTH} {HEIGHT}\">\
         <defs>\
         <radialGradient id=\"glow\" cx=\"1\" cy=\"0\" r=\"1.2\"><stop offset=\"0\" stop-color=\"{GLOW}\"/>\
         <stop offset=\"0.6\" stop-color=\"{BACKGROUND}\"/></radialGradient>\
         <linearGradient id=\"area\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\
         <stop offset=\"0\" stop-color=\"{PRIMARY}\" stop-opacity=\"0.28\"/>\
         <stop offset=\"1\" stop-color=\"{PRIMARY}\" stop-opacity=\"0\"/></linearGradient>\
         </defs>\
         <rect x=\"1\" y=\"1\" width=\"{}\" height=\"{}\" rx=\"24\" fill=\"{BACKGROUND}\"/>\
         <rect x=\"1\" y=\"1\" width=\"{}\" height=\"{}\" rx=\"24\" fill=\"url(#glow)\" stroke=\"{EDGE}\" stroke-width=\"2\"/>",
        WIDTH - 2,
        HEIGHT - 2,
        WIDTH - 2,
        HEIGHT - 2
    );

    // Heading: what the card is about, its day and account; the brand at the right.
    svg.push_str(&text(MARGIN, 96.0, 34.0, FontFace::SemiBold, FOREGROUND, &clip(&card.title, 40)));
    let mut line_y = 134.0;
    for note in [Some(card.date.as_str()), card.account.as_deref()].into_iter().flatten().filter(|n| !n.is_empty()) {
        svg.push_str(&text(MARGIN, line_y, 24.0, FontFace::Regular, MUTED, &clip(note, 48)));
        line_y += 32.0;
    }
    let _ = write!(
        svg,
        "<rect x=\"964\" y=\"68\" width=\"36\" height=\"36\" rx=\"10\" fill=\"{PRIMARY}\"/>\
         <circle cx=\"980\" cy=\"84\" r=\"8\" fill=\"none\" stroke=\"{PRIMARY_FOREGROUND}\" stroke-width=\"3.5\"/>\
         <path d=\"M986 90 L992 96\" stroke=\"{PRIMARY_FOREGROUND}\" stroke-width=\"3.5\" stroke-linecap=\"round\"/>"
    );
    svg.push_str(&text(1012.0, 96.0, 28.0, FontFace::SemiBold, FOREGROUND, "TradeLens"));

    // The result, big, with the R under the money and a line saying what is hidden.
    svg.push_str(&text(MARGIN, 330.0, main_size, face, color, &headline.main));
    let mut caption_y = 376.0;
    if let Some(secondary) = &headline.secondary {
        svg.push_str(&text(MARGIN, 392.0, 44.0, FontFace::MonoSemiBold, color, secondary));
        caption_y = 440.0;
    }
    svg.push_str(&text(MARGIN, caption_y, 24.0, FontFace::Regular, MUTED, &headline.caption));

    if chart_svg.is_empty() {
        svg.push_str(&curve(&card.curve, card.outcome));
    } else {
        svg.push_str(&chart_svg);
    }
    svg.push_str(&footer(&card.stats));
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        report::{Privacy, tests::trade},
        time::Clock,
    };

    fn card(privacy: Privacy) -> ShareCard {
        ShareCard::new(
            &trade(412.5),
            Some(2.1),
            Some("Breakout phiên Á".into()),
            None,
            "USD",
            privacy,
            Clock::default(),
        )
    }

    /// The pixels of a PNG as RGBA, with its size.
    fn decode(png: &[u8]) -> tiny_skia::Pixmap {
        tiny_skia::Pixmap::decode_png(png).expect("the card is a valid PNG")
    }

    /// How many pixels of the area are close to `rgb`.
    fn near(pixmap: &tiny_skia::Pixmap, rgb: (u8, u8, u8), area: (u32, u32, u32, u32)) -> usize {
        let (x0, y0, x1, y1) = area;
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter_map(|(x, y)| pixmap.pixel(x, y))
            .filter(|p| {
                let close = |a: u8, b: u8| a.abs_diff(b) < 40;
                close(p.red(), rgb.0) && close(p.green(), rgb.1) && close(p.blue(), rgb.2)
            })
            .count()
    }

    #[test]
    fn the_card_is_a_1200_by_630_png_with_the_result_in_profit_green() {
        let png = render_png(&card(Privacy::OnlyR)).expect("the card renders");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let pixmap = decode(&png);
        assert_eq!((pixmap.width(), pixmap.height()), (WIDTH, HEIGHT));
        // The big "+2.10R" sits left of the panel, between y 250 and 340: its glyphs are green.
        let green = near(&pixmap, (0x34, 0xd3, 0x99), (60, 240, 640, 340));
        assert!(green > 800, "the result is drawn with the bundled font, found {green} green pixels");
        // The footer's values are set in the foreground colour: the fonts also reach the last row.
        let light = near(&pixmap, (0xe6, 0xea, 0xf2), (60, 520, 1140, 580));
        assert!(light > 200, "the footer figures are drawn, found {light} pixels");
    }

    #[test]
    fn a_losing_trade_is_red_and_hidden_figures_show_only_the_outcome() {
        let lost = ShareCard::new(&trade(-300.0), Some(-1.4), None, None, "USD", Privacy::OnlyR, Clock::default());
        let pixmap = decode(&render_png(&lost).expect("renders"));
        assert!(near(&pixmap, (0xf8, 0x71, 0x71), (60, 240, 640, 340)) > 800, "a loss is red");
        let hidden = card(Privacy::NoFigures);
        let svg = card_svg(&hidden);
        assert!(svg.contains(">Thắng<"), "the outcome word is shown");
        assert!(!svg.contains("2.10") && !svg.contains('$') && !svg.contains("409"), "no figures: {svg}");
        assert_ne!(
            render_png(&hidden).expect("renders"),
            render_png(&card(Privacy::OnlyR)).expect("renders"),
            "hiding the figures changes the picture"
        );
    }

    #[test]
    fn money_is_drawn_only_when_the_trader_shows_it() {
        let money = card_svg(&card(Privacy::ShowMoney));
        assert!(money.contains("+$409.00") && money.contains("+2.10R"));
        let only_r = card_svg(&card(Privacy::OnlyR));
        assert!(only_r.contains("+2.10R") && !only_r.contains('$') && !only_r.contains("409"), "{only_r}");
    }

    #[test]
    fn a_period_card_draws_its_curve_and_text_is_escaped() {
        let figures = crate::report::PeriodFigures {
            trades: 3,
            wins: 2,
            net: 90.0,
            unit: crate::metrics::Unit::Money,
            total_r: Some(1.5),
            profit_factor: None,
            top_setup: Some("A & <B>".into()),
            r_curve: vec![0.0, 1.0, 0.5, 1.5],
            currency: "USD".into(),
        };
        let svg = card_svg(&ShareCard::period("Tuần 47".into(), &figures, Privacy::OnlyR));
        assert!(svg.contains("fill=\"url(#area)\""), "the curve is drawn");
        assert!(svg.contains("A &amp; &lt;B&gt;"), "the setup name is escaped");
        render_png(&ShareCard::period("Tuần 47".into(), &figures, Privacy::OnlyR)).expect("a period card renders");
        let flat = crate::report::PeriodFigures { r_curve: vec![0.0; 3], ..figures };
        render_png(&ShareCard::period("Tuần 47".into(), &flat, Privacy::OnlyR)).expect("a flat curve renders");
    }

    #[test]
    fn a_chart_image_is_embedded_and_a_missing_one_is_left_out() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let shot = dir.path().join("exit.png");
        let mut tiny = tiny_skia::Pixmap::new(8, 4).expect("a pixmap");
        tiny.fill(tiny_skia::Color::from_rgba8(200, 30, 30, 255));
        std::fs::write(&shot, tiny.encode_png().expect("encodes")).expect("writes");
        let mut with = card(Privacy::OnlyR);
        with.chart = Some(shot.display().to_string());
        let pixmap = decode(&render_png(&with).expect("renders with the image"));
        assert!(near(&pixmap, (200, 30, 30), (660, 180, 1120, 420)) > 10_000, "the screenshot fills its panel");
        with.chart = Some(dir.path().join("gone.png").display().to_string());
        render_png(&with).expect("a chart that cannot be read does not fail the card");
    }

    #[test]
    fn long_names_are_clipped_and_wide_results_shrink() {
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
        assert!(fit(15, 0.6, 560.0, 96.0) < 96.0);
        assert!((fit(3, 0.6, 560.0, 96.0) - 96.0).abs() < f32::EPSILON);
    }
}
