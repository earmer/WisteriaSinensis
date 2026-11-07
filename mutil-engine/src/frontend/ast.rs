//! Paragraph AST and the box-and-glue sequence (spec §2.4, §2.5).

use super::ShapingRun;
use crate::types::{
    AtomFlags, BidiDirection, ByteIndex, FontMetrics, GraphemeAtom, ShapedCluster, TextRange,
};

/// Replacement text for spelling-change hyphenation.
///
/// The spec uses `CompactString`; replacements are rare (`Option`, usually
/// `None`), so `Box<str>` costs 16 bytes inline (niche-optimised) and one
/// allocation only when present, with no extra dependency.
pub type ReplacementText = Box<str>;

/// Hyphenation opportunity mirroring libhyphen's `(rep, pos, cut)` `[H3]`
/// (spec §2.4). Example: Dutch "omaatje" becomes "oma-tje".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HyphenOpportunity {
    /// The break position in the **original** text: the prefix ends and the
    /// suffix begins here, before any replacement is applied.
    pub byte_offset: ByteIndex,
    /// Text replacing the bytes cut from the end of the prefix, if the
    /// spelling changes.
    pub prefix_replacement: Option<ReplacementText>,
    /// Text replacing the bytes cut from the start of the suffix, if the
    /// spelling changes.
    pub suffix_replacement: Option<ReplacementText>,
    /// Original bytes removed immediately **before** `byte_offset`. Must end on
    /// an atom boundary. A cut that straddles the break is split between the
    /// two counts.
    pub prefix_cut_bytes: u32,
    /// Original bytes removed immediately **after** `byte_offset`. Must end on
    /// an atom boundary.
    pub suffix_cut_bytes: u32,
    /// Break penalty (feeds Strategy B, §5 Stage 1).
    pub penalty: i32,
}

/// How a [`WordNode::PunctuationRun`] binds to the preceding cluster.
///
/// `PunctuationRun` is **an unbreakable suffix of the preceding word**: UAX #14
/// already forbids breaks before CL, CP, EX, SY, IS and IN, including between
/// consecutive marks `[U7]` (§2.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PunctBind {
    /// The whole run binds to the preceding cluster. The default.
    #[default]
    WholeRun,
    /// A tailored profile binds only the first `bound_count` marks.
    /// **[Design decision]** placeholder for "except where a tailored profile
    /// says otherwise" (§2.4); semantics arrive with the profiles.
    Partial {
        /// Number of leading marks that are bound.
        bound_count: u8,
    },
}

/// Kind of stretchable space (spec §2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlueKind {
    /// An ordinary word space.
    WordSpace,
    /// Automatic CJK / non-CJK spacing; sizes come from the active profile
    /// (`Jlreq` or `CssCompat`, §2.4).
    CjkAutospace,
}

/// One box, glue or penalty item of the line-breaking sequence (spec §2.4).
///
/// Collections are `Vec`, not `SmallVec`: inline storage would inflate every
/// variant to the size of the largest, and `Vec` does not allocate while empty
/// (as `hyphen_breaks` usually is). `ShapedCluster::atom_range` indexes the
/// node's own `atoms` (see [`ShapedCluster`]).
#[derive(Clone, Debug, PartialEq)]
pub enum WordNode {
    /// A breakable word.
    Word {
        /// Index into [`ParagraphAst::runs`].
        run_index: usize,
        /// The word's atoms.
        atoms: Vec<GraphemeAtom>,
        /// Shaped clusters; `atom_range` is relative to `atoms`.
        clusters: Vec<ShapedCluster>,
        /// Hyphenation opportunities inside the word.
        hyphen_breaks: Vec<HyphenOpportunity>,
        /// Sum of cluster widths.
        total_width: f32,
    },
    /// Punctuation bound to the preceding word (replaces `mandatory_bind_count`).
    PunctuationRun {
        /// Index into [`ParagraphAst::runs`].
        run_index: usize,
        /// The marks' atoms.
        atoms: Vec<GraphemeAtom>,
        /// Shaped clusters; `atom_range` is relative to `atoms`.
        clusters: Vec<ShapedCluster>,
        /// Binding rule.
        bind: PunctBind,
    },
    /// Stretchable, shrinkable space.
    Glue {
        /// Source bytes (empty for `CjkAutospace`, which has `start == end`).
        byte_range: TextRange,
        /// Space kind.
        kind: GlueKind,
        /// Natural width.
        natural_width: f32,
        /// Maximum added width.
        max_stretch: f32,
        /// Maximum removed width.
        max_shrink: f32,
    },
    /// A tab; width is `next_tab_stop(x) - x`, evaluated during layout.
    Tab {
        /// Source bytes.
        byte_range: TextRange,
    },
    /// `\n`, `\r\n` or U+2028: ends the line and disables justification.
    HardBreak {
        /// Source bytes.
        byte_range: TextRange,
    },
    /// An embedded object with fixed extents.
    InlineBox {
        /// Source bytes.
        byte_range: TextRange,
        /// Caller-assigned identifier.
        box_id: u64,
        /// Width.
        width: f32,
        /// Extent above the baseline.
        ascent: f32,
        /// Extent below the baseline.
        descent: f32,
    },
}

impl WordNode {
    /// The source bytes this node covers: the first atom's start to the last
    /// atom's end for `Word` and `PunctuationRun` (`None` when they hold no
    /// atoms or if the atoms are out of order), the stored range for the other
    /// variants.
    #[must_use]
    pub fn byte_span(&self) -> Option<TextRange> {
        match self {
            Self::Word { atoms, .. } | Self::PunctuationRun { atoms, .. } => {
                let (first, last) = (atoms.first()?, atoms.last()?);
                if first.byte_range.start > last.byte_range.end {
                    return None; // inverted: callers treat a missing span of a non-empty node as invalid
                }
                Some(TextRange::new(first.byte_range.start, last.byte_range.end))
            }
            Self::Glue { byte_range, .. }
            | Self::Tab { byte_range }
            | Self::HardBreak { byte_range }
            | Self::InlineBox { byte_range, .. } => Some(*byte_range),
        }
    }

    /// Checks the `atom_range` invariants documented on [`ShapedCluster`]:
    /// the node's clusters partition its atoms, in order, with matching byte
    /// ranges. Nodes without clusters (glue, tab, hard break, inline box) are
    /// trivially self-contained.
    #[must_use]
    pub fn is_self_contained(&self) -> bool {
        match self {
            Self::Word {
                atoms, clusters, ..
            }
            | Self::PunctuationRun {
                atoms, clusters, ..
            } => clusters_partition_atoms(atoms, clusters),
            _ => true,
        }
    }
}

fn clusters_partition_atoms(atoms: &[GraphemeAtom], clusters: &[ShapedCluster]) -> bool {
    let mut next = 0;
    for c in clusters {
        let r = &c.atom_range;
        if r.start != next || r.end <= r.start || r.end > atoms.len() {
            return false;
        }
        let (first, last) = (&atoms[r.start], &atoms[r.end - 1]);
        if c.byte_range.start != first.byte_range.start || c.byte_range.end != last.byte_range.end {
            return false;
        }
        // Invariant 3: the cluster's flags are the per-flag aggregate of its atoms.
        if c.flags != AtomFlags::for_cluster(&atoms[r.clone()]) {
            return false;
        }
        next = r.end;
    }
    next == atoms.len()
}

/// Frontend output: everything the middle-end needs, independent of column
/// width (spec §2.5, §1.3).
///
/// Owns all its data (no borrows), so it is `'static` and `Send + Sync`. The
/// paragraph text is not stored here, matching the spec: shapers receive it as
/// `&str` per call.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphAst {
    /// Resolved paragraph base direction (UAX #9 P2/P3 or the caller's override).
    pub base_direction: BidiDirection,
    /// The **raw** primary-font metrics exactly as `TextShaper::
    /// resolve_primary_font_metrics` returned them. The frontend neither reads
    /// `LineHeight` nor computes half-leading; the middle-end resolves the
    /// active strut from this value with `LineHeight::resolve_strut`
    /// (§5 Stage 4.1).
    pub strut_metrics: FontMetrics,
    /// The font size (layout units) the strut metrics were requested at. The
    /// middle-end needs it to resolve a `LineHeight::Fixed` strut.
    pub strut_font_size: f32,
    /// Shaping runs in logical order.
    pub runs: Vec<ShapingRun>,
    /// Box-and-glue sequence in logical order.
    pub items: Vec<WordNode>,
}
