//! Physical shaping output: glyphs grouped into clusters (spec §2.3).

use super::{AtomFlags, BidiLevel, FontRef, TextRange};
use core::ops::Range;
use smallvec::SmallVec;

/// One glyph as produced by a shaping backend, before line layout.
///
/// **[Design decision]** The spec names `RawShapedGlyph` (§2.3) without
/// defining it. The field set mirrors what HarfBuzz, CoreText and DirectWrite
/// all provide for horizontal text. Vertical text (a `y_advance`) is out of
/// scope for the spec so far.
///
/// Layout: five 4-byte fields plus a `bool`, 24 bytes with 3 bytes of tail
/// padding, alignment 4.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawShapedGlyph {
    /// Backend glyph index within the cluster's [`FontRef`].
    pub glyph_id: u32,
    /// HarfBuzz-style cluster value: the start index of the group this glyph
    /// belongs to (spec §4.2 req. 4). Same coordinate space as the text handed
    /// to the shaper.
    pub cluster: u32,
    /// Horizontal advance.
    pub x_advance: f32,
    /// Horizontal offset from the pen position (marks, kerning).
    pub x_offset: f32,
    /// Vertical offset from the baseline.
    pub y_offset: f32,
    /// `true` for a glyph the engine generated rather than the source text
    /// (the hyphen appended by `TrailingEdit`). Carried through to
    /// `PositionedGlyph::is_synthesized`.
    pub is_synthesized: bool,
}

/// A shaper-produced unit: one or more whole atoms and the glyphs that render
/// them (spec §2.3). The smallest unit a line can be broken between.
///
/// ## Index spaces
///
/// `byte_range` is in paragraph text coordinates. **`atom_range` is relative to
/// the atom slice stored beside the cluster**: the `atoms` argument of
/// `TextShaper::shape_run` for freshly shaped clusters, and `atoms` of the
/// owning `BoxNode::Text` once the frontend has split a run
/// into words (which re-bases the indices). Each node is therefore
/// self-contained: `atoms[cluster.atom_range.clone()]` always works.
/// **[Design decision]**; the spec leaves the base unspecified.
///
/// Two invariants make this safe (checked by `WordNode::is_self_contained`):
///
/// 1. **A cluster belongs to exactly one `WordNode`.** Splitting a
///    `ShapingRun` into nodes must never split a `ShapedCluster`. Split points
///    (UAX #14 opportunities, spaces, punctuation boundaries) are therefore
///    chosen *before* clusters are assigned to nodes. If a split point falls
///    inside a multi-atom cluster, the frontend must re-shape that window with
///    the split as a boundary (`TextShaper::reshape_window` with
///    `LeadingEdit::None` and `TrailingEdit::None`) instead of cutting the cluster.
/// 2. **Within a node, clusters partition the atoms.** In logical order the
///    `atom_range`s are contiguous, non-empty, start at 0 and end at
///    `atoms.len()`, and each `byte_range` runs from its first atom's start to
///    its last atom's end.
///
/// 3. **Flags aggregate per flag.** `flags == AtomFlags::for_cluster(atoms[atom_range])`
///    (also checked by `WordNode::is_self_contained`).
///
/// `TextShaper::reshape_window` output uses the same space as `shape_run`
/// output (indices into the run's atom slice, not re-based to the window).
///
/// ## Break-safety polarity (HarfBuzz `[B1]`)
///
/// Both flags describe the **start** of this cluster, hence "before". The
/// draft's `is_safe_to_break_after` had the polarity inverted: "break after
/// cluster *i*" is "break before cluster *i + 1*", and the end of text is always
/// safe. `true` means the corresponding HarfBuzz unsafe flag is absent.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedCluster {
    /// Source bytes covered by the cluster.
    pub byte_range: TextRange,
    /// Atoms covered: `K >= 1`, and a cluster never splits an atom. `K > 1`
    /// means a ligature (§6.2 gives it K caret sub-slots). Explicitly a
    /// [`Range<usize>`]; see "Index spaces" above.
    pub atom_range: Range<usize>,
    /// Font used for this cluster, with the fallback scale (was `font_id`).
    pub font: FontRef,
    /// Bidi level; a cluster never straddles two levels (§3 Step 2.3).
    pub bidi_level: BidiLevel,
    /// Per-flag aggregate of the covered atoms' flags, as computed by
    /// `AtomFlags::for_cluster`: open-punctuation from the first atom,
    /// `ALLOW_CJK_JUSTIFY_AFTER` and close-punctuation from the last, all other
    /// flags ORed.
    pub flags: AtomFlags,
    /// Glyphs in **logical** order, including the order within the cluster.
    /// Backends that return visual order for RTL must reverse (§4.2 req. 2).
    /// Two inline slots: most clusters are one or two glyphs.
    pub glyphs: SmallVec<[RawShapedGlyph; 2]>,
    /// Advance width as shaped, with any `chws` compression applied.
    pub natural_width: f32,
    /// Width with `chws` off. Costs a second shaping pass (§4.2 req. 6).
    pub uncompressed_width: f32,
    /// `true` iff HarfBuzz's `UNSAFE_TO_BREAK` is absent here: a line may begin
    /// at this cluster without re-shaping either side.
    pub safe_to_break_before: bool,
    /// `true` iff `UNSAFE_TO_CONCAT` is absent here (needs
    /// `HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT`, HarfBuzz 4.0.0). Splicing two
    /// separately shaped pieces is only sound if **both** sides of the seam are
    /// clear. `UNSAFE_TO_BREAK` implies `UNSAFE_TO_CONCAT`, so a cluster with
    /// `safe_to_break_before == false` must have this false too.
    pub safe_to_concat_before: bool,
}

impl ShapedCluster {
    /// Number of atoms covered (`K`).
    #[inline]
    #[must_use]
    pub fn atom_count(&self) -> usize {
        self.atom_range.len()
    }

    /// `true` when the cluster covers more than one atom (a ligature).
    #[inline]
    #[must_use]
    pub fn is_ligature(&self) -> bool {
        self.atom_count() > 1
    }

    /// `true` when the break-safety flags honour the HarfBuzz implication
    /// "unsafe to break implies unsafe to concat", i.e. `safe_to_concat_before`
    /// implies `safe_to_break_before`.
    #[inline]
    #[must_use]
    pub fn safety_flags_consistent(&self) -> bool {
        self.safe_to_break_before || !self.safe_to_concat_before
    }
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<RawShapedGlyph>() == 24);
    assert!(core::mem::align_of::<RawShapedGlyph>() == 4);
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FontId;

    fn cluster(atoms: Range<usize>, brk: bool, cat: bool) -> ShapedCluster {
        ShapedCluster {
            byte_range: TextRange::new(0, 2),
            atom_range: atoms,
            font: FontRef::unscaled(FontId(0)),
            bidi_level: 0,
            flags: AtomFlags::EMPTY,
            glyphs: SmallVec::new(),
            natural_width: 0.0,
            uncompressed_width: 0.0,
            safe_to_break_before: brk,
            safe_to_concat_before: cat,
        }
    }

    #[test]
    fn ligature_detection() {
        assert!(!cluster(3..4, true, true).is_ligature());
        assert!(cluster(3..5, true, true).is_ligature());
        assert_eq!(cluster(3..5, true, true).atom_count(), 2);
    }

    #[test]
    fn unsafe_to_break_implies_unsafe_to_concat() {
        assert!(cluster(0..1, true, true).safety_flags_consistent());
        assert!(cluster(0..1, false, false).safety_flags_consistent());
        // Only concat unsafe: legal (UNSAFE_TO_CONCAT without UNSAFE_TO_BREAK).
        assert!(cluster(0..1, true, false).safety_flags_consistent());
        // Unsafe to break but safe to concat contradicts HarfBuzz: UNSAFE_TO_BREAK
        // always implies UNSAFE_TO_CONCAT.
        assert!(!cluster(0..1, false, true).safety_flags_consistent());
    }
}
