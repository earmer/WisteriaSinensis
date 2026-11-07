//! The abstract shaping bridge (spec §4.1).

use crate::frontend::{HyphenOpportunity, ShapingRun};
use crate::types::{
    Direction, FontFamily, FontFeatures, FontId, FontMetrics, FontRef, FontStyle, GraphemeAtom,
    LocaleTag, ScriptTag, ShapedCluster, TextRange,
};
use smallvec::SmallVec;

/// Edit at the **leading** (start) edge of a `reshape_window` window: the break
/// point *before* this window (spec §4.1, §5 Stage 1.4).
///
/// Orthogonal to [`TrailingEdit`]: a window takes one of each, so a middle
/// fragment of a word split twice can carry a leading and a trailing edit at
/// the same time. Ranges always describe the original text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeadingEdit {
    /// No change at the leading edge.
    None,
    /// Spelling change on the **suffix** side of a break: drop `cut_bytes` from
    /// the start of the window (the bytes immediately *after* the break point,
    /// towards the end of the text), then prepend `replacement`. Built from
    /// `HyphenOpportunity::{suffix_replacement, suffix_cut_bytes}`. Adds no
    /// hyphen (that belongs to the previous line).
    ReplaceSuffix {
        /// Replacement text.
        replacement: Box<str>,
        /// Original bytes removed from the start of the window.
        cut_bytes: u32,
    },
}

/// Edit at the **trailing** (end) edge of a `reshape_window` window: the break
/// point *after* this window. See [`LeadingEdit`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrailingEdit {
    /// No change at the trailing edge.
    None,
    /// Append a (hyphen) character to the end of the window; the glyph is
    /// synthesised. U+002D or the locale's hyphenation character (CSS
    /// `hyphenate-character`).
    AppendHyphen {
        /// The hyphen character.
        hyphen: char,
    },
    /// Spelling change on the **prefix** side of a break: drop `cut_bytes` from
    /// the end of the window (the bytes immediately *before* the break point,
    /// towards the start of the text), append `replacement`, then the hyphen.
    /// Built from `HyphenOpportunity::{prefix_replacement, prefix_cut_bytes}`.
    ReplacePrefix {
        /// Replacement text.
        replacement: Box<str>,
        /// Original bytes removed.
        cut_bytes: u32,
        /// Hyphen appended after the replacement.
        hyphen: char,
    },
}

impl LeadingEdit {
    /// The edit for the line **starting** at `opportunity`: `ReplaceSuffix` if
    /// the suffix spelling changes (a replacement text or a cut; a cut without
    /// replacement is a pure deletion), otherwise `None`. Reads only the suffix
    /// side.
    #[must_use]
    pub fn for_opportunity(opportunity: &HyphenOpportunity) -> Self {
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

impl TrailingEdit {
    /// The edit for the line **ending** at `opportunity`: `ReplacePrefix` if the
    /// prefix spelling changes, otherwise `AppendHyphen`. Reads only the prefix
    /// side.
    #[must_use]
    pub fn for_opportunity(opportunity: &HyphenOpportunity, hyphen: char) -> Self {
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
    /// * A window that carries a hyphen (`AppendHyphen`, `ReplacePrefix`)
    ///   must contain at least one atom, since the hyphen's glyphs attach to
    ///   the window's last cluster.
    /// * The two edits are independent and act on their own edge only: a
    ///   window may carry both (the middle fragment of a word split twice) or
    ///   neither (both `None`: a plain, symmetric, text-preserving re-shape).
    ///   Their cuts must not overlap: `leading cut + trailing cut` must not
    ///   exceed the window length, and each cut ends on an atom boundary.
    /// * Edits never change ranges: they always describe the **original** text
    ///   (§6.6). A synthesised hyphen is appended as glyphs of the last
    ///   cluster, each flagged `RawShapedGlyph::is_synthesized`. For
    ///   `ReplacePrefix` and `ReplaceSuffix`, the replacement glyphs are
    ///   attributed to the cluster(s) covering the removed bytes.
    ///   With `cut_bytes == 0` and a non-empty replacement no cluster covers
    ///   removed bytes: the glyphs belong to the window's last cluster for
    ///   `ReplacePrefix` and to its first cluster for `ReplaceSuffix`.
    ///   **[Design decision]**; to be refined with the HarfRust backend in
    ///   Phase 1.
    #[allow(clippy::too_many_arguments)] // signature derived from spec §4.1
    fn reshape_window(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        window: TextRange,
        features: &FontFeatures,
        leading_edit: LeadingEdit,
        trailing_edit: TrailingEdit,
        output_clusters: &mut Vec<ShapedCluster>,
    );

    /// GDEF ligature caret positions: `n - 1` for `n` components, empty if the
    /// font has none (§6.2). Positions are unshaped and may need kerning
    /// adjustment.
    ///
    /// **Units.** The return value is in layout coordinates, never font design
    /// units: the backend has already applied the font size and the fallback
    /// scale. Positions are x offsets **relative to the glyph origin**, in the
    /// unit of `font_size`, measured along +x and increasing. `font_size` is the run's
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
    fn edits_are_compact_and_aligned() {
        // Largest payload: Box<str> (16) + u32 + char on 64-bit.
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(core::mem::align_of::<TrailingEdit>(), 8);
            assert!(core::mem::size_of::<TrailingEdit>() <= 32);
            assert!(core::mem::size_of::<LeadingEdit>() <= 32);
        }
    }

    #[test]
    fn replace_suffix_constructs_clones_and_compares() {
        let e = LeadingEdit::ReplaceSuffix {
            replacement: "tje".into(),
            cut_bytes: 1,
        };
        assert_eq!(e.clone(), e);
        assert_ne!(
            e,
            LeadingEdit::ReplaceSuffix {
                replacement: "tje".into(),
                cut_bytes: 2
            }
        );
        assert_ne!(e, LeadingEdit::None);
        let LeadingEdit::ReplaceSuffix {
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
            TrailingEdit::for_opportunity(&o, '-'),
            TrailingEdit::ReplacePrefix {
                replacement: "a".into(),
                cut_bytes: 2,
                hyphen: '-'
            }
        );
        assert_eq!(
            LeadingEdit::for_opportunity(&o),
            LeadingEdit::ReplaceSuffix {
                replacement: "tje".into(),
                cut_bytes: 1
            }
        );
        let p = opportunity(Some("a"), 1, None, 0);
        assert_eq!(LeadingEdit::for_opportunity(&p), LeadingEdit::None);
        let s = opportunity(None, 0, Some("k"), 1);
        assert_eq!(
            TrailingEdit::for_opportunity(&s, '-'),
            TrailingEdit::AppendHyphen { hyphen: '-' }
        );
        // A cut with no replacement text is a pure deletion on that side.
        let d = opportunity(None, 1, None, 1);
        assert_eq!(
            TrailingEdit::for_opportunity(&d, '-'),
            TrailingEdit::ReplacePrefix {
                replacement: "".into(),
                cut_bytes: 1,
                hyphen: '-'
            }
        );
        assert_eq!(
            LeadingEdit::for_opportunity(&d),
            LeadingEdit::ReplaceSuffix {
                replacement: "".into(),
                cut_bytes: 1
            }
        );
    }

    #[test]
    fn middle_fragment_takes_both_edits_from_two_consecutive_breaks() {
        // "pro-gram-ming": the middle line "gram" starts after break 1 and ends at
        // break 2, so it needs a leading and a trailing edit at once.
        let first = opportunity(None, 0, Some("g"), 1);
        let second = opportunity(Some("m"), 1, None, 0);
        let leading = LeadingEdit::for_opportunity(&first);
        let trailing = TrailingEdit::for_opportunity(&second, '-');
        assert!(matches!(leading, LeadingEdit::ReplaceSuffix { .. }));
        assert!(matches!(trailing, TrailingEdit::ReplacePrefix { .. }));
    }

    #[test]
    fn shaper_trait_objects_are_send_and_sync() {
        fn check<T: Send + Sync + ?Sized>() {}
        check::<dyn TextShaper>();
        check::<Box<dyn TextShaper>>();
    }
}
