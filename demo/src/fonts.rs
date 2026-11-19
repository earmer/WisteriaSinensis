//! The registered font faces: their bytes, the shaper built over them, the
//! family stack every paragraph uses, and glyph outlines drawn from the same
//! bytes the layout shaped.

use std::fmt::Write as _;
use std::sync::Arc;

use mutil_engine::{
    FontBytes, FontFace, FontFamily, FontFeatures, FontRegistry, FontStyle, FontVariation,
    HarfRustShaper,
};
use skrifa::outline::DrawSettings;
use skrifa::prelude::{LocationRef, Size};
use skrifa::{FontRef as SkrifaFont, GlyphId, MetadataProvider};

/// The order of the registered families in every paragraph's stack after the
/// chosen face: Latin, CJK, Indic and Southeast Asian, then the rest.
const FIXED_STACK: [&str; 13] = [
    "Noto Sans",
    "Noto Serif",
    "JetBrains Mono",
    "Noto Sans SC",
    "Noto Sans TC",
    "Noto Sans JP",
    "Noto Sans Devanagari",
    "Noto Sans Thai",
    "Noto Sans Lao",
    "Noto Sans Khmer",
    "Noto Sans Myanmar",
    "Noto Sans Hebrew",
    "Noto Naskh Arabic",
];

/// The harness asks for Regular. Noto Sans JP's variable font defaults to
/// `wght=100`, so the setting is explicit and applied to shaping and drawing
/// alike; an outline drawn at a different instance than the layout would be a
/// renderer bug, not a design choice.
const WEIGHT: f32 = 400.0;

struct FaceEntry {
    family: Arc<str>,
    bytes: FontBytes,
}

/// The registered faces and the shaper built over them. The registry is
/// rebuilt only when a face is added, which the page does before laying out.
#[derive(Default)]
pub struct Fonts {
    faces: Vec<FaceEntry>,
    shaper: Option<HarfRustShaper>,
}

impl Fonts {
    /// Registers `bytes` under `family` and returns the face id and its
    /// units per em, or `None` for a family already registered or bytes
    /// that hold no font (a zero units-per-em included).
    pub fn add(&mut self, family: &str, bytes: Vec<u8>) -> Option<(u32, u16)> {
        if self.faces.iter().any(|face| *face.family == *family) {
            return None;
        }
        let upem = SkrifaFont::new(bytes.as_slice())
            .ok()?
            .metrics(Size::unscaled(), LocationRef::default())
            .units_per_em;
        if upem == 0 {
            return None;
        }
        self.faces.push(FaceEntry {
            family: Arc::from(family),
            bytes: FontBytes::new(bytes),
        });
        self.shaper = None;
        let id = u32::try_from(self.faces.len() - 1).ok()?;
        Some((id, upem))
    }

    /// The shaper over every registered face, built on first use after a
    /// face was added.
    pub fn shaper(&mut self) -> &HarfRustShaper {
        let faces = &self.faces;
        self.shaper.get_or_insert_with(|| {
            let mut registry = FontRegistry::new();
            for face in faces {
                registry.add_face(
                    Arc::clone(&face.family),
                    FontStyle::default(),
                    FontFace::new(face.bytes.clone(), 0),
                );
            }
            HarfRustShaper::new(Arc::new(registry))
        })
    }

    /// The family stack handed to the engine: every registered face, the
    /// chosen one first, then in `FIXED_STACK` order, then the rest in
    /// registration order. It is the same for every locale: the engine
    /// routes CJK runs to the regional face of their locale (spec SHP-12,
    /// SHP-25), so the page never reorders the stack.
    pub fn family_stack(&self, primary: Option<&str>) -> Arc<[FontFamily]> {
        let rank = |family: &str| {
            primary
                .into_iter()
                .chain(FIXED_STACK)
                .position(|f| f == family)
        };
        let mut families: Vec<&Arc<str>> = self.faces.iter().map(|face| &face.family).collect();
        families.sort_by_key(|family| rank(family).unwrap_or(usize::MAX));
        families
            .into_iter()
            .map(|family| FontFamily::Named(Arc::clone(family)))
            .collect()
    }

    /// The outline of `glyph` in face `font`, in font units, y down, as SVG
    /// path data; `None` for an unknown face or glyph.
    pub fn outline(&self, font: u32, glyph: u32) -> Option<String> {
        let face = self.faces.get(usize::try_from(font).ok()?)?;
        let font = SkrifaFont::new(face.bytes.as_bytes()).ok()?;
        let outline = font.outline_glyphs().get(GlyphId::new(glyph))?;
        // Draw the same instance the layout shaped (see `WEIGHT`).
        let location = font.axes().location([("wght", WEIGHT)]);
        // Font units, no hinting (spec HST-5/HST-6).
        let settings = DrawSettings::unhinted(Size::unscaled(), &location);
        let mut pen = PathPen::default();
        outline.draw(settings, &mut pen).ok()?;
        Some(pen.0)
    }
}

/// The font features every style carries: the explicit Regular weight.
pub fn regular_weight() -> FontFeatures {
    let mut features = FontFeatures::default();
    if let Some(weight) = FontVariation::new(*b"wght", WEIGHT) {
        features.push_variation(weight);
    }
    features
}

/// Collects an outline as SVG path data, in font units with y pointing down
/// (the canvas convention), so the page can scale it by `size / upem`.
#[derive(Default)]
struct PathPen(String);

impl PathPen {
    /// Two decimals of a font unit: far below a device pixel at any plausible
    /// size, and short enough to keep the payload small.
    fn segment(&mut self, command: char, points: &[(f32, f32)]) {
        self.0.push(command);
        for (index, (x, y)) in points.iter().enumerate() {
            let separator = if index > 0 { " " } else { "" };
            let _ = write!(self.0, "{separator}{x:.2} {:.2}", -y);
        }
    }
}

impl skrifa::outline::OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.segment('M', &[(x, y)]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.segment('L', &[(x, y)]);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.segment('Q', &[(cx0, cy0), (x, y)]);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.segment('C', &[(cx0, cy0), (cx1, cy1), (x, y)]);
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}
