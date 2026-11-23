//! The faces the share card and the PDF are drawn with: the Inter and `JetBrains Mono` files the
//! app already bundles for its UI.
//!
//! The bytes are `include_bytes!`-ed from `app/assets/fonts/` so the app's fonts stay the single
//! copy on disk; this is the one place `core` reaches outside its own folder, and it does so at
//! build time only (nothing is read from the user's machine). Rendering must not depend on the
//! fonts installed on the system, or a card would look different from one machine to the next.

use ttf_parser::Face;

/// One face of the bundled families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FontFace {
    Regular,
    Medium,
    SemiBold,
    Bold,
    Mono,
    MonoSemiBold,
}

impl FontFace {
    pub(crate) const ALL: [Self; 6] =
        [Self::Regular, Self::Medium, Self::SemiBold, Self::Bold, Self::Mono, Self::MonoSemiBold];

    /// The font file's bytes.
    pub(crate) fn bytes(self) -> &'static [u8] {
        match self {
            Self::Regular => include_bytes!("../../../../app/assets/fonts/Inter-Regular.ttf"),
            Self::Medium => include_bytes!("../../../../app/assets/fonts/Inter-Medium.ttf"),
            Self::SemiBold => include_bytes!("../../../../app/assets/fonts/Inter-SemiBold.ttf"),
            Self::Bold => include_bytes!("../../../../app/assets/fonts/Inter-Bold.ttf"),
            Self::Mono => include_bytes!("../../../../app/assets/fonts/JetBrainsMono-Regular.ttf"),
            Self::MonoSemiBold => include_bytes!("../../../../app/assets/fonts/JetBrainsMono-SemiBold.ttf"),
        }
    }

    /// The CSS family and weight of the face, as an SVG `font-family` / `font-weight` pair.
    pub(crate) fn css(self) -> (&'static str, u16) {
        match self {
            Self::Regular => ("Inter", 400),
            Self::Medium => ("Inter", 500),
            Self::SemiBold => ("Inter", 600),
            Self::Bold => ("Inter", 700),
            Self::Mono => ("JetBrains Mono", 400),
            Self::MonoSemiBold => ("JetBrains Mono", 600),
        }
    }
}

/// How wide text is set, to wrap and align it: the advance widths of the face's glyphs (no
/// kerning, which only makes the measure a hair wide).
#[derive(Debug)]
pub(crate) struct Metrics {
    faces: Vec<(FontFace, Face<'static>)>,
}

/// The width of a glyph the face lacks, in ems.
const MISSING_GLYPH_EM: f32 = 0.6;

impl Metrics {
    /// Reads the bundled faces.
    ///
    /// # Panics
    ///
    /// Never in practice: the files are compiled in and a test parses each of them.
    pub(crate) fn new() -> Self {
        let faces = FontFace::ALL
            .into_iter()
            .map(|face| (face, Face::parse(face.bytes(), 0).expect("the bundled fonts are valid TrueType files")))
            .collect();
        Self { faces }
    }

    fn face(&self, face: FontFace) -> &Face<'static> {
        self.faces
            .iter()
            .find(|(candidate, _)| *candidate == face)
            .map(|(_, parsed)| parsed)
            .expect("every face is loaded by new")
    }

    /// The width of `text` set in `face` at `size` points.
    pub(crate) fn width(&self, face: FontFace, size: f32, text: &str) -> f32 {
        let parsed = self.face(face);
        let units = f32::from(parsed.units_per_em());
        text.chars()
            .map(|c| {
                parsed
                    .glyph_index(c)
                    .and_then(|glyph| parsed.glyph_hor_advance(glyph))
                    .map_or(MISSING_GLYPH_EM, |advance| f32::from(advance) / units)
                    * size
            })
            .sum()
    }

    /// Whether the face has a glyph for every character of `text`.
    #[cfg(test)]
    pub(crate) fn covers(&self, face: FontFace, text: &str) -> bool {
        let parsed = self.face(face);
        text.chars().all(|c| parsed.glyph_index(c).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_faces_set_vietnamese_and_the_minus_sign() {
        let metrics = Metrics::new();
        for face in FontFace::ALL {
            assert!(metrics.covers(face, "Báo cáo tháng ƯờẶ −+0123456789%$"), "{face:?} lacks a glyph");
        }
    }

    #[test]
    fn wider_text_measures_wider_and_mono_digits_are_even() {
        let metrics = Metrics::new();
        let short = metrics.width(FontFace::Regular, 12.0, "Lệnh");
        let long = metrics.width(FontFace::Regular, 12.0, "Lệnh thắng");
        assert!(long > short && short > 0.0, "{short} < {long}");
        let one = metrics.width(FontFace::Mono, 10.0, "1");
        let eight = metrics.width(FontFace::Mono, 10.0, "8");
        assert!((one - eight).abs() < 1e-3, "a monospaced face: {one} vs {eight}");
    }
}
