//! Font identity, style, feature and metric types shared by the shaping bridge
//! and the frontend (spec §2.3, §2.5, §4.1).

use smallvec::SmallVec;
use std::sync::Arc;

/// Opaque, backend-assigned font identifier.
///
/// Meaningful only to the [`TextShaper`](crate::shaping::TextShaper) that issued
/// it; the engine never interprets the value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(pub u32);

/// A font plus the scale at which it must be applied (spec §2.3).
///
/// `scale` exists because DirectWrite's `MapCharacters` returns a scale factor
/// alongside the mapped font `[B5]`. `1.0` means "as requested".
/// Layout: 8 bytes (`u32` + `f32`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontRef {
    /// Backend font identifier.
    pub font_id: FontId,
    /// Multiplicative size correction supplied by font fallback.
    pub scale: f32,
}

impl FontRef {
    /// A reference with `scale == 1.0`.
    #[inline]
    #[must_use]
    pub const fn unscaled(font_id: FontId) -> Self {
        Self {
            font_id,
            scale: 1.0,
        }
    }
}

/// CSS-style generic family keyword.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenericFamily {
    /// `serif`.
    Serif,
    /// `sans-serif`.
    SansSerif,
    /// `monospace`.
    Monospace,
    /// `cursive`.
    Cursive,
    /// `fantasy`.
    Fantasy,
    /// `system-ui`.
    SystemUi,
    /// `emoji`: routes to colour emoji fonts (cf. the `Zsye` pseudo-script, §2.1).
    Emoji,
}

/// One entry of a family stack (spec §2.5: "same family stack" per run).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FontFamily {
    /// A concrete family name, shared cheaply between runs.
    Named(Arc<str>),
    /// A generic family keyword.
    Generic(GenericFamily),
}

/// An ordered family stack shared by every run that uses it.
///
/// `Arc<[_]>` makes cloning a reference-count bump and lets caches key on
/// pointer identity or a precomputed hash (spec §4.2 req. 5 keys on the stack).
pub type FamilyStack = Arc<[FontFamily]>;

/// CSS-scale font weight (1 to 1000; 400 normal, 700 bold).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontWeight(pub u16);

impl FontWeight {
    /// 400.
    pub const NORMAL: Self = Self(400);
    /// 700.
    pub const BOLD: Self = Self(700);
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Slant of the requested face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontSlant {
    /// Upright.
    #[default]
    Normal,
    /// True italic.
    Italic,
    /// Synthesisable oblique.
    Oblique,
}

/// Face selection style: weight and slant (spec §3, Step 5: "family, size,
/// weight, slant"; family is a separate [`FamilyStack`], size a separate `f32`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontStyle {
    /// Requested weight.
    pub weight: FontWeight,
    /// Requested slant.
    pub slant: FontSlant,
}

/// One OpenType feature setting, for example `("chws", 1)` (spec §4.2 req. 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFeature {
    /// Four-byte OpenType feature tag.
    pub tag: [u8; 4],
    /// Feature value (0 = off, 1 = on, higher for alternates).
    pub value: u32,
}

/// Feature settings handed to `shape_run`. Cache-relevant (spec §4.1, §4.2 req. 5).
///
/// Four inline entries cover the usual `kern`/`liga`/`chws`/`locl` overrides
/// without a heap allocation.
///
/// **Deferred:** variation-axis settings, which the §4.2 req. 5 cache key also
/// needs, are not modelled here because the §4.1 signature carries only
/// `FontFeatures`. They are a Phase 1 decision tied to the cache.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontFeatures(pub SmallVec<[FontFeature; 4]>);

impl FontFeatures {
    /// An empty feature set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a setting.
    pub fn push(&mut self, tag: [u8; 4], value: u32) {
        self.0.push(FontFeature { tag, value });
    }

    /// The settings as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[FontFeature] {
        &self.0
    }
}

/// Complete primary-font metrics (spec §2.5).
///
/// The field set is a **[Design decision]** (the draft never defined it).
/// `resolve_primary_font_metrics` must return every field (§4.5); fonts lacking
/// a table value fall back to the backend's best estimate. All values are in
/// layout units at the requested size; positions follow the OpenType sign
/// convention chosen by the backend and normalised by the middle-end later.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FontMetrics {
    /// Distance from baseline to the top of the line box.
    pub ascent: f32,
    /// Distance from baseline to the bottom of the line box (positive downwards).
    pub descent: f32,
    /// Extra inter-line gap, used only for `line-height: normal` (§5, Stage 4).
    pub line_gap: f32,
    /// Underline offset from the baseline.
    pub underline_position: f32,
    /// Underline stroke thickness.
    pub underline_thickness: f32,
    /// Strike-through offset from the baseline.
    pub strikeout_position: f32,
    /// x-height.
    pub x_height: f32,
    /// Cap height.
    pub cap_height: f32,
    /// OS/2 `fsSelection` bit 7 (`USE_TYPO_METRICS`) `[B7]`.
    pub use_typo_metrics: bool,
}

/// Strut metrics for a paragraph (spec §2.5, §5 Stage 4): what an empty or
/// fixed-height line is measured with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VerticalMetrics {
    /// Baseline to line-box top.
    pub ascent: f32,
    /// Baseline to line-box bottom.
    pub descent: f32,
    /// Extra inter-line gap.
    pub line_gap: f32,
}

impl VerticalMetrics {
    /// Takes the vertical subset of a font's metrics. No policy is applied here:
    /// `USE_TYPO_METRICS` selection is the backend's job (§4.5).
    #[must_use]
    pub const fn from_font_metrics(m: &FontMetrics) -> Self {
        Self {
            ascent: m.ascent,
            descent: m.descent,
            line_gap: m.line_gap,
        }
    }

    /// `ascent + descent + line_gap`, the CSS `line-height: normal` height.
    #[must_use]
    pub fn natural_line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_collect_inline() {
        let mut f = FontFeatures::new();
        f.push(*b"chws", 1);
        f.push(*b"kern", 0);
        assert_eq!(f.as_slice().len(), 2);
        assert!(!f.0.spilled());
    }

    #[test]
    fn vertical_metrics_follow_font_metrics() {
        let fm = FontMetrics {
            ascent: 8.0,
            descent: 2.0,
            line_gap: 1.0,
            ..Default::default()
        };
        let v = VerticalMetrics::from_font_metrics(&fm);
        assert_eq!(v.natural_line_height(), 11.0);
    }

    #[test]
    fn unscaled_font_ref_has_unit_scale() {
        assert_eq!(FontRef::unscaled(FontId(3)).scale, 1.0);
    }
}
