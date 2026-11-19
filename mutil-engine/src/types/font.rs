//! Font identity, style, feature and metric types shared by the shaping bridge
//! and the frontend (spec §2.3, §2.5, §4.1).

use smallvec::SmallVec;
use std::sync::Arc;

/// Opaque, backend-assigned font identifier.
///
/// Meaningful only within the id space of the
/// [`TextShaper`](crate::shaping::TextShaper) that issued it; a renderer resolves
/// it through the [`FontSource`](crate::shaping::FontSource) that covers that
/// space, never through its own font lookup (spec FNT-3, HST-2). The engine
/// never interprets the value.
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

/// CSS-scale font weight: 1 to 1000, 400 normal, 700 bold.
///
/// Parsed, not validated (spec §1.5, AX-3): the field is private and
/// [`try_new`](Self::try_new) refuses a value outside the scale, so a weight that
/// exists is in range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontWeight(u16);

impl FontWeight {
    /// 400.
    pub const NORMAL: Self = Self(400);
    /// 700.
    pub const BOLD: Self = Self(700);

    /// The weight `value`, or `None` outside `1..=1000`.
    #[must_use]
    pub const fn try_new(value: u16) -> Option<Self> {
        if 1 <= value && value <= 1000 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// The weight on the CSS scale.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
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

/// One variable-font axis setting, for example `("wght", 650.0)` (spec §4.2
/// req. 5: the cache key includes "variation settings").
///
/// Parsed, not validated (spec §1.5, AX-3): [`try_new`](Self::try_new) refuses a
/// coordinate that is not finite and maps `-0.0` to `0.0`, so equality and
/// hashing by bit pattern are sound and agree with numeric equality.
#[derive(Clone, Copy, Debug)]
pub struct FontVariation {
    tag: [u8; 4],
    value: f32,
}

impl FontVariation {
    /// The setting of axis `tag` to `value`, with `-0.0` canonicalised to
    /// `0.0`; `None` unless `value` is finite.
    #[must_use]
    pub const fn try_new(tag: [u8; 4], value: f32) -> Option<Self> {
        let value = if value == 0.0 { 0.0 } else { value };
        if value.is_finite() {
            Some(Self { tag, value })
        } else {
            None
        }
    }

    /// Four-byte axis tag.
    #[must_use]
    pub const fn tag(&self) -> [u8; 4] {
        self.tag
    }

    /// Axis coordinate.
    #[must_use]
    pub const fn value(&self) -> f32 {
        self.value
    }
}

impl PartialEq for FontVariation {
    fn eq(&self, other: &Self) -> bool {
        self.tag == other.tag && self.value.to_bits() == other.value.to_bits()
    }
}

impl Eq for FontVariation {}

impl core::hash::Hash for FontVariation {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.tag.hash(state);
        self.value.to_bits().hash(state);
    }
}

/// Feature and variation settings handed to `shape_run` and `reshape_window`.
/// Cache-relevant (spec §4.1, §4.2 req. 5).
///
/// Parsed, not validated (spec §1.5, AX-3): the fields are private and both
/// `push` methods keep each list strictly ascending by tag with the **last**
/// setting for a tag winning (as in CSS cascade order), so `Eq` and `Hash`
/// agree with effective equality and the value is a valid cache key as built.
///
/// **[Design decision]** Variation axes live here rather than in a new
/// parameter because the §4.1 signature carries only `FontFeatures`. Inline
/// capacity (4 features, 2 axes) covers the usual `kern`/`liga`/`chws`/`locl`
/// overrides and `wght`/`wdth` without a heap allocation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontFeatures {
    features: SmallVec<[FontFeature; 4]>,
    variations: SmallVec<[FontVariation; 2]>,
}

/// Inserts `item` into `list` (ascending by `tag`, one entry per tag),
/// replacing an entry with the same tag.
fn set_by_tag<T, A: smallvec::Array<Item = T>>(
    list: &mut SmallVec<A>,
    item: T,
    tag: impl Fn(&T) -> [u8; 4],
) {
    match list.binary_search_by_key(&tag(&item), tag) {
        Ok(at) => list[at] = item,
        Err(at) => list.insert(at, item),
    }
}

impl FontFeatures {
    /// Sets feature `tag` to `value`, replacing an earlier setting of `tag`.
    pub fn push(&mut self, tag: [u8; 4], value: u32) {
        set_by_tag(&mut self.features, FontFeature { tag, value }, |f| f.tag);
    }

    /// Sets a variation axis, replacing an earlier setting of the same axis.
    pub fn push_variation(&mut self, variation: FontVariation) {
        set_by_tag(&mut self.variations, variation, FontVariation::tag);
    }

    /// The feature settings, ascending by tag.
    #[must_use]
    pub fn as_slice(&self) -> &[FontFeature] {
        &self.features
    }

    /// The variation settings, ascending by tag.
    #[must_use]
    pub fn variations(&self) -> &[FontVariation] {
        &self.variations
    }
}

/// Borrowed, complete physical font instance used by measurement queries.
#[derive(Clone, Copy, Debug)]
pub struct FontInstanceRef<'a> {
    /// Face and fallback scale; FontId identifies a face, not variations.
    pub font: FontRef,
    /// Requested size in layout units.
    pub font_size: f32,
    /// Variable font coordinates.
    pub variations: &'a [FontVariation],
}

/// Complete primary-font metrics (spec §2.5).
///
/// The field set is a **[Design decision]** (the draft never defined it).
/// `resolve_primary_font_metrics` must return every field (§4.5); fonts lacking
/// a table value fall back to the backend's best estimate. All values are in
/// layout units at the requested size. Baseline offsets are positive toward
/// line-over; the backend normalizes table signs and supplies missing metrics.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FontMetrics {
    /// Vertical line-over half extent.
    pub vertical_ascent: f32,
    /// Vertical line-under half extent.
    pub vertical_descent: f32,
    /// Vertical inter-line gap.
    pub vertical_line_gap: f32,
    /// Baseline offset toward line-over, in scaled layout units.
    pub ideographic_baseline_offset: f32,
    /// Baseline offset toward line-over, in scaled layout units.
    pub central_baseline_offset: f32,
    /// Baseline offset toward line-over, in scaled layout units.
    pub hanging_baseline_offset: f32,
    /// Baseline offset toward line-over, in scaled layout units.
    pub mathematical_baseline_offset: f32,
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

impl FontMetrics {
    /// The font's own horizontal line height: `ascent + descent + line_gap`.
    #[must_use]
    pub fn natural_line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }

    /// Checks raw font metrics: every field a measurement
    /// ([`is_measurement`](super::is_measurement), spec MSR-1), the depths
    /// non-negative (ascent, descent, line gap and their vertical counterparts,
    /// underline thickness, x-height, cap height) and both `ascent + descent`
    /// sums positive. A resolved strut has other rules; see `StrutMetrics`.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let nonnegative = [
            self.ascent,
            self.descent,
            self.line_gap,
            self.vertical_ascent,
            self.vertical_descent,
            self.vertical_line_gap,
            self.underline_thickness,
            self.x_height,
            self.cap_height,
        ];
        nonnegative.into_iter().all(super::is_extent)
            && [
                self.underline_position,
                self.strikeout_position,
                self.ideographic_baseline_offset,
                self.central_baseline_offset,
                self.hanging_baseline_offset,
                self.mathematical_baseline_offset,
            ]
            .into_iter()
            .all(super::is_measurement)
            && self.ascent + self.descent > 0.0
            && self.vertical_ascent + self.vertical_descent > 0.0
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn font_weight_is_parsed_onto_the_css_scale() {
        assert_eq!(FontWeight::try_new(400), Some(FontWeight::NORMAL));
        assert_eq!(FontWeight::try_new(1).map(FontWeight::get), Some(1));
        assert_eq!(FontWeight::try_new(1000).map(FontWeight::get), Some(1000));
        assert_eq!(FontWeight::try_new(0), None);
        assert_eq!(FontWeight::try_new(1001), None);
        assert_eq!(FontWeight::default(), FontWeight::NORMAL);
    }

    use super::*;

    #[test]
    fn features_collect_inline() {
        let mut f = FontFeatures::default();
        f.push(*b"chws", 1);
        f.push(*b"kern", 0);
        assert_eq!(f.as_slice().len(), 2);
        assert!(!f.features.spilled());
    }

    #[test]
    fn push_keeps_ascending_tags_and_the_last_setting_per_tag() {
        let mut f = FontFeatures::default();
        f.push(*b"kern", 1);
        f.push(*b"chws", 1);
        f.push(*b"kern", 0); // later wins
        f.push_variation(FontVariation::try_new(*b"wght", 400.0).unwrap());
        f.push_variation(FontVariation::try_new(*b"wdth", 100.0).unwrap());
        f.push_variation(FontVariation::try_new(*b"wght", 650.0).unwrap());
        assert_eq!(
            f.as_slice(),
            &[
                FontFeature {
                    tag: *b"chws",
                    value: 1
                },
                FontFeature {
                    tag: *b"kern",
                    value: 0
                }
            ]
        );
        assert_eq!(f.variations().len(), 2);
        assert_eq!(f.variations()[0].tag(), *b"wdth");
        assert_eq!(f.variations()[1].value(), 650.0);
    }

    #[test]
    fn a_variation_is_finite_by_construction() {
        const WEIGHT: Option<FontVariation> = FontVariation::try_new(*b"wght", 400.0);
        assert!(WEIGHT.is_some());
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(FontVariation::try_new(*b"wght", bad), None);
        }
        assert_eq!(
            FontVariation::try_new(*b"wght", 650.0).map(|v| v.value()),
            Some(650.0)
        );
    }

    #[test]
    fn variation_negative_zero_hashes_like_zero() {
        use std::hash::{BuildHasher, RandomState};
        let s = RandomState::new();
        let a = FontVariation::try_new(*b"slnt", -0.0).unwrap();
        let b = FontVariation::try_new(*b"slnt", 0.0).unwrap();
        assert_eq!(a, b);
        assert_eq!(s.hash_one(a), s.hash_one(b));
    }

    #[test]
    fn unscaled_font_ref_has_unit_scale() {
        assert_eq!(FontRef::unscaled(FontId(3)).scale, 1.0);
    }
}
