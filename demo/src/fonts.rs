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

/// The registered families in the order every paragraph's stack lists them
/// after the chosen face.
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
    /// that hold no font.
    pub fn add(&mut self, family: &str, bytes: Vec<u8>) -> Option<(u32, u16)> {
        if self.faces.iter().any(|face| *face.family == *family) {
            return None;
        }
        let upem = SkrifaFont::new(bytes.as_slice())
            .ok()?
            .metrics(Size::unscaled(), LocationRef::default())
            .units_per_em;
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

    /// The family stack handed to the engine: the chosen face, then the
    /// Latin faces, the CJK faces, the Indic and Southeast Asian faces and
    /// anything else registered. It is the same for every locale: the engine
    /// routes CJK runs to the regional face of their locale (spec SHP-12,
    /// SHP-25), so the page never reorders the stack.
    pub fn family_stack(&self, primary: Option<&str>) -> Arc<[FontFamily]> {
        let requested = primary
            .into_iter()
            .chain(FIXED_STACK)
            .chain(self.faces.iter().map(|face| &*face.family));
        let mut names: Vec<Arc<str>> = Vec::new();
        for family in requested {
            if let Some(face) = self.faces.iter().find(|face| *face.family == *family)
                && !names.contains(&face.family)
            {
                names.push(Arc::clone(&face.family));
            }
        }
        names.into_iter().map(FontFamily::Named).collect()
    }

    /// The outline of `glyph` in face `font`, in font units, y down, as SVG
    /// path data; `None` for an unknown face or glyph.
    pub fn outline(&self, font: u32, glyph: u32) -> Option<String> {
        let face = self.faces.get(usize::try_from(font).ok()?)?;
        let font = SkrifaFont::new(face.bytes.as_bytes()).ok()?;
        let outline = font.outline_glyphs().get(GlyphId::new(glyph))?;
        // Draw the same instance the layout shaped (see `WEIGHT`).
        let location = font.axes().location([("wght", WEIGHT)]);
        let mut pen = PathPen::default();
        // Font units, no hinting (spec HST-5/HST-6). The pen flips y into
        // canvas coordinates.
        outline
            .draw(
                DrawSettings::unhinted(Size::unscaled(), &location),
                &mut pen,
            )
            .ok()?;
        Some(pen.0)
    }
}

/// The font features every style carries: the explicit Regular weight.
pub fn regular_weight() -> FontFeatures {
    let mut features = FontFeatures::new();
    if let Some(weight) = FontVariation::new(*b"wght", WEIGHT) {
        features.push_variation(weight);
    }
    features
}

/// Collects an outline as SVG path data, in font units with y pointing down
/// (the canvas convention), so the page can scale it by `size / upem`.
#[derive(Default)]
struct PathPen(String);

impl skrifa::outline::OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push('M');
        self.point(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push('L');
        self.point(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.push('Q');
        self.point(cx0, cy0);
        self.0.push(' ');
        self.point(x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.push('C');
        self.point(cx0, cy0);
        self.0.push(' ');
        self.point(cx1, cy1);
        self.0.push(' ');
        self.point(x, y);
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}

impl PathPen {
    fn point(&mut self, x: f32, y: f32) {
        // Two decimals of a font unit: far below a device pixel at any
        // plausible size, and short enough to keep the payload small.
        let _ = write!(self.0, "{x:.2} {:.2}", -y);
    }
}
