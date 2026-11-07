//! The abstract shaping bridge (spec §4.1).

use crate::frontend::{HyphenOpportunity, ShapingRun};
use crate::types::{
    Direction, FontFamily, FontFeatures, FontId, FontMetrics, FontRef, FontStyle, GraphemeAtom,
    LocaleTag, ScriptTag, ShapedCluster, TextRange,
};
use smallvec::SmallVec;

/// Edit applied at a line boundary before re-shaping (spec §4.1, §5 Stage 1.4).
///
/// Which edge of the `reshape_window` window an edit acts on:
///
/// | variant | edge |
/// |---|---|
/// | `None` | both, symmetrically (no text change) |
/// | `AppendHyphen`, `ReplacePrefix` | trailing (end) edge only |
/// | `ReplaceSuffix` | leading (start) edge only |
///
/// A one-sided edit leaves the other edge exactly as `None` would. One call
/// carries one edit, so a window that needs a trailing **and** a leading edit
/// (the middle fragment of a word split twice) cannot be expressed (Open
/// question 31).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundaryEdit {
    /// Symmetric, text-preserving re-shape of the window: no text is added or
    /// removed. It re-evaluates the contextual forms of the window's atoms after
    /// a line break or a split ligature, and acts on both edges of the window
    /// alike.
    None,
    /// **Trailing edge only.** Append a (hyphen) character to the end of the
    /// window; the glyph is synthesised.
    AppendHyphen {
        /// U+002D or the locale's hyphenation character (CSS `hyphenate-character`).
        hyphen: char,
    },
    /// **Trailing edge only.** Spelling change on the **prefix** side: drop
    /// `cut_bytes` from the end of the window (the bytes immediately *before*
    /// the break point, i.e. towards the start of the text), append
    /// `replacement`, then the hyphen. Built from
    /// `HyphenOpportunity::{prefix_replacement, prefix_cut_bytes}`.
    ReplacePrefix {
        /// Replacement text.
        replacement: Box<str>,
        /// Original bytes removed.
        cut_bytes: u32,
        /// Hyphen appended after the replacement.
        hyphen: char,
    },
    /// **Leading edge only.** Spelling change on the **suffix** side, at the
    /// start of the following line: drop `cut_bytes` from the start of the window
    /// (the bytes immediately *after* the break point, i.e. towards the end of
    /// the text), then prepend `replacement`. Built from
    /// `HyphenOpportunity::{suffix_replacement, suffix_cut_bytes}`. It adds no
    /// hyphen (that belongs to the previous line). Ranges still describe the
    /// original text, as for every edit.
    ReplaceSuffix {
        /// Replacement text.
        replacement: Box<str>,
        /// Original bytes removed from the start of the window.
        cut_bytes: u32,
    },
}

impl BoundaryEdit {
    /// The edit for the line **ending** at `opportunity`: `ReplacePrefix` if the
    /// opportunity changes the prefix spelling (a replacement text or a cut),
    /// otherwise `AppendHyphen`. A cut without replacement text is a pure
    /// deletion (empty `replacement`).
    #[must_use]
    pub fn prefix_for(opportunity: &HyphenOpportunity, hyphen: char) -> Self {
        if opportunity.prefix_replacement.is_some() || opportunity.prefix_cut_bytes > 0 {
            Self::ReplacePrefix {
                replacement: opportunity.prefix_replacement.clone().unwrap_or_default(),
                cut_bytes: opportunity.prefix_cut_bytes,
                hyphen,
            }
        } else {
            Self::AppendHyphen { hyphen }
        }
    }

    /// The edit for the line **starting** at `opportunity`: `ReplaceSuffix` if
    /// the suffix spelling changes, otherwise `None`. Symmetric with
    /// [`BoundaryEdit::prefix_for`]: each side reads only its own replacement
    /// and cut count.
    #[must_use]
    pub fn suffix_for(opportunity: &HyphenOpportunity) -> Self {
        if opportunity.suffix_replacement.is_some() || opportunity.suffix_cut_bytes > 0 {
            Self::ReplaceSuffix {
                replacement: opportunity.suffix_replacement.clone().unwrap_or_default(),
                cut_bytes: opportunity.suffix_cut_bytes,
            }
        } else {
            Self::None
        }
    }
}

/// Contract between the engine and a shaping backend (HarfRust, CoreText,
/// DirectWrite, a test double). Object-safe, so a backend can be chosen at run
/// time as `&dyn TextShaper`. `Send + Sync`: methods take `&self`, so one
/// backend can serve several layout threads; implementations needing interior
/// mutability (caches) must synchronise it themselves.
///
/// Output vectors are **appended to, never cleared**: callers own the buffer
/// and may reuse its capacity.
pub trait TextShaper: Send + Sync {
    /// Resolves the primary font for a style and returns its identifier and
    /// complete metrics (§4.5). Used for the paragraph strut.
    fn resolve_primary_font_metrics(
        &self,
        families: &[FontFamily],
        style: FontStyle,
        script: ScriptTag,
        locale: &LocaleTag,
        font_size: f32,
    ) -> (FontId, FontMetrics);

    /// Shapes one complete run. Output is in **logical** order (§4.2 req. 2);
    /// `atoms` is the run's atom slice, and `ShapedCluster::atom_range` indexes it.
    ///
    /// `text` is the **whole paragraph text**, with `run.byte_range` selecting
    /// the run, so that cluster byte offsets are absolute paragraph indices and
    /// need no translation between layers. The backend may use the text outside
    /// the run as shaping context (e.g. HarfBuzz pre- and post-context).
    ///
    /// **Cross-font limit.** OpenType contextual lookups, kerning and `chws`
    /// squeezing act only inside one physical font. When two adjacent clusters
    /// are routed to different fonts, no passing of the full text makes the
    /// backend kern or squeeze across the seam. Full-width punctuation
    /// squeezing across fonts is therefore a middle-end metrics-stage fallback
    /// (spec §4.2, §5).
    #[allow(clippy::too_many_arguments)] // signature fixed by spec §4.1
    fn shape_run(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        families: &[FontFamily],
        font_size: f32,
        features: &FontFeatures,
        output_clusters: &mut Vec<ShapedCluster>,
    );

    /// Re-shapes a window around a break, hyphenation point or word split
    /// (§5 Stage 1.4).
    ///
    /// **Deviation from the §4.1 signature** (two parameters added, both
    /// needed for a sound splice):
    /// * `atoms`: the *same* run-local atom slice that was given to
    ///   `shape_run` for this run;
    /// * `features`: the *same* settings, since re-shaping with different
    ///   features (`chws`, `locl`, variations) would not splice.
    ///
    /// Contract:
    /// * `window` is in paragraph text coordinates, lies within
    ///   `run.byte_range`, and starts and ends on atom boundaries of `atoms`.
    /// * Output `atom_range`s index `atoms` **directly** (they are *not*
    ///   re-based to the window), i.e. in the same coordinate space as the
    ///   clusters `shape_run` produced for the run. The caller splices by
    ///   replacing the clusters whose `atom_range` lies inside the window.
    /// * The output clusters partition exactly the atoms inside the window, in
    ///   logical order, and never split an atom.
    /// * `edit` never changes ranges: they always describe the **original**
    ///   text (§6.6). A synthesised hyphen is appended as glyphs of the last
    ///   cluster, each flagged `RawShapedGlyph::is_synthesized`. For
    ///   `ReplacePrefix`, the replacement glyphs are attributed to the
    ///   cluster(s) covering the removed bytes. **[Design decision]**; to be
    ///   refined with the HarfRust backend in Phase 1.
    #[allow(clippy::too_many_arguments)] // signature derived from spec §4.1
    fn reshape_window(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        window: TextRange,
        features: &FontFeatures,
        edit: BoundaryEdit,
        output_clusters: &mut Vec<ShapedCluster>,
    );

    /// GDEF ligature caret positions: `n - 1` for `n` components, empty if the
    /// font has none (§6.2). Positions are unshaped and may need kerning
    /// adjustment.
    ///
    /// **Units.** The backend converts from font design units itself: returned
    /// positions are x coordinates **relative to the glyph origin**, in layout
    /// units, i.e. the unit of `font_size` (pixels when `font_size` is in
    /// pixels), measured along +x and increasing. `font_size` is the run's
    /// requested size; the backend multiplies it by `font.scale` (the fallback
    /// size correction) to get the effective size. **[Design decision]**: the
    /// engine is unit-agnostic and does not know the device pixel ratio, so
    /// "physical pixels" here means "the same units as `font_size`".
    fn ligature_carets(
        &self,
        font: FontRef,
        glyph_id: u32,
        direction: Direction,
        font_size: f32,
    ) -> SmallVec<[f32; 4]>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_edit_is_compact_and_aligned() {
        // Largest payload: Box<str> (16) + u32 + char (or u32) on 64-bit.
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(core::mem::align_of::<BoundaryEdit>(), 8);
            assert!(core::mem::size_of::<BoundaryEdit>() <= 32);
        }
    }

    #[test]
    fn replace_suffix_constructs_clones_and_compares() {
        let e = BoundaryEdit::ReplaceSuffix {
            replacement: "tje".into(),
            cut_bytes: 1,
        };
        assert_eq!(e.clone(), e);
        assert_ne!(
            e,
            BoundaryEdit::ReplaceSuffix {
                replacement: "tje".into(),
                cut_bytes: 2
            }
        );
        assert_ne!(e, BoundaryEdit::None);
        let BoundaryEdit::ReplaceSuffix {
            replacement,
            cut_bytes,
        } = e
        else {
            unreachable!()
        };
        assert_eq!((&*replacement, cut_bytes), ("tje", 1));
    }

    fn opportunity(
        prefix: Option<&str>,
        prefix_cut: u32,
        suffix: Option<&str>,
        suffix_cut: u32,
    ) -> HyphenOpportunity {
        HyphenOpportunity {
            byte_offset: 3,
            prefix_replacement: prefix.map(Into::into),
            suffix_replacement: suffix.map(Into::into),
            prefix_cut_bytes: prefix_cut,
            suffix_cut_bytes: suffix_cut,
            penalty: 0,
        }
    }

    #[test]
    fn each_side_reads_only_its_own_replacement_and_cut() {
        let o = opportunity(Some("a"), 2, Some("tje"), 1);
        assert_eq!(
            BoundaryEdit::prefix_for(&o, '-'),
            BoundaryEdit::ReplacePrefix {
                replacement: "a".into(),
                cut_bytes: 2,
                hyphen: '-'
            }
        );
        assert_eq!(
            BoundaryEdit::suffix_for(&o),
            BoundaryEdit::ReplaceSuffix {
                replacement: "tje".into(),
                cut_bytes: 1
            }
        );
        // Prefix-only change leaves the suffix side untouched, and vice versa.
        let p = opportunity(Some("a"), 1, None, 0);
        assert_eq!(BoundaryEdit::suffix_for(&p), BoundaryEdit::None);
        let s = opportunity(None, 0, Some("k"), 1);
        assert_eq!(
            BoundaryEdit::prefix_for(&s, '-'),
            BoundaryEdit::AppendHyphen { hyphen: '-' }
        );
        // A cut with no replacement text is a pure deletion on that side.
        let d = opportunity(None, 1, None, 1);
        assert_eq!(
            BoundaryEdit::prefix_for(&d, '-'),
            BoundaryEdit::ReplacePrefix {
                replacement: "".into(),
                cut_bytes: 1,
                hyphen: '-'
            }
        );
        assert_eq!(
            BoundaryEdit::suffix_for(&d),
            BoundaryEdit::ReplaceSuffix {
                replacement: "".into(),
                cut_bytes: 1
            }
        );
    }

    #[test]
    fn shaper_trait_objects_are_send_and_sync() {
        fn check<T: Send + Sync + ?Sized>() {}
        check::<dyn TextShaper>();
        check::<Box<dyn TextShaper>>();
    }

    #[test]
    fn prefix_and_suffix_edits_are_symmetric_in_shape() {
        let p = BoundaryEdit::ReplacePrefix {
            replacement: "a".into(),
            cut_bytes: 1,
            hyphen: '-',
        };
        let s = BoundaryEdit::ReplaceSuffix {
            replacement: "a".into(),
            cut_bytes: 1,
        };
        // Same payload except that only the prefix side carries the hyphen.
        assert!(matches!(p, BoundaryEdit::ReplacePrefix { hyphen: '-', .. }));
        assert!(matches!(s, BoundaryEdit::ReplaceSuffix { .. }));
    }
}
