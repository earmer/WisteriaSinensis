//! The box-and-glue node algebra: glue, boxes and the [`WordNode`] item
//! (spec §2.4).

use super::DiscretionaryNode;
use crate::types::{BaselineAlignment, BidiClass, BidiLevel, TextRange};
use core::ops::Range;

/// Kind of stretchable space (spec §2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlueKind {
    /// Space characters of the source text: a non-empty run of
    /// [`is_word_space`](crate::frontend::is_word_space) characters. The only kind that
    /// owns source bytes.
    WordSpace,
    /// Automatic CJK / non-CJK spacing; sizes come from the active profile
    /// (`Jlreq` or `CssCompat`, §2.4).
    CjkAutospace,
    /// The compressible margin of CJK full-width punctuation that `chws` has
    /// not already squeezed, or that JLReq / CLREQ mid-line compression frees
    /// (spec §2.4, "Tier 2 is derived, not emitted"). It is the **punctuation compression pool** of
    /// [`GlueTier::Tier0`]: `natural_width == 0`, `max_stretch == 0`, and
    /// `max_shrink` up to `0.5 * ic`.
    CjkPunctCompression,
}

impl GlueKind {
    /// The tier of this kind of glue: `WordSpace` and `CjkPunctCompression`
    /// are [`GlueTier::Tier0`], `CjkAutospace` is [`GlueTier::Tier1`]. **No
    /// kind maps to [`GlueTier::Tier2`]**: inter-ideographic stretch is derived
    /// from `ALLOW_CJK_JUSTIFY_AFTER` on the clusters, not from glue nodes
    /// (spec §2.4).
    #[must_use]
    pub const fn tier(self) -> GlueTier {
        match self {
            Self::WordSpace | Self::CjkPunctCompression => GlueTier::Tier0,
            Self::CjkAutospace => GlueTier::Tier1,
        }
    }

    /// `true` for the kinds the engine inserts (`CjkAutospace`,
    /// `CjkPunctCompression`): they own no source bytes.
    #[must_use]
    pub const fn is_synthetic(self) -> bool {
        !matches!(self, Self::WordSpace)
    }
}

/// Priority level of a [`GlueNode`] in the JLReq / CLREQ ladder (spec §2.4,
/// §5 Stage 1.6.3). A lower tier absorbs slack (and, for Tier 0 and 1, gives
/// up width) before a higher tier is touched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum GlueTier {
    /// Punctuation compression pool and Western word spaces.
    #[default]
    Tier0 = 0,
    /// CJK / non-CJK autospace (`GlueKind::CjkAutospace`).
    Tier1 = 1,
    /// Inter-ideographic expansion (`ALLOW_CJK_JUSTIFY_AFTER` gaps); zero
    /// natural width, stretch only. A tier of the middle-end's prefix-sum
    /// tables only: no [`GlueKind`] has it, because explicit Tier 2 glue would
    /// inflate a CJK paragraph's node count by a third.
    Tier2 = 2,
}

/// Stretchable, shrinkable space (spec §2.4).
///
/// Glue is **never** an implicit break point: a breakable space is followed by
/// a [`DiscretionaryNode`] emitted by the frontend (see [`WordNode`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlueNode {
    /// Original Unicode bidi class for L1.
    pub original_bidi_class: BidiClass,
    /// Resolved level before line-local L1.
    pub bidi_level: BidiLevel,
    /// Only collapsible trailing whitespace can disappear. Always `true` for
    /// a `WordSpace` (spec WN-12): a no-break space
    /// ([`is_no_break_space`](crate::frontend::is_no_break_space)) is text, not glue.
    pub is_collapsible: bool,
    /// Source bytes: the space characters of a `WordSpace`, an empty range for
    /// the synthetic kinds.
    pub byte_range: TextRange,
    /// Space kind, which also fixes the priority tier ([`GlueKind::tier`]).
    pub kind: GlueKind,
    /// Natural width.
    pub natural_width: f32,
    /// Maximum added width.
    pub max_stretch: f32,
    /// Maximum removed width.
    pub max_shrink: f32,
}

/// Which end of an [`InlineDecorationSpan`](crate::frontend::InlineDecorationSpan) a
/// [`BoxNode::SpanBoundary`] stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpanEdge {
    /// The logical start of the span (left border plus padding).
    Leading,
    /// The logical end of the span (right border plus padding).
    Trailing,
}

/// What happens to a rigid inline object that is wider than the line (spec
/// §2.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BoxOverflowPolicy {
    /// Clamp the visual rendering to `wrap_width`.
    #[default]
    Clip,
    /// Permit `visual_width > wrap_width` when the object sits alone on an
    /// empty line; the line's `visual_width` then equals the box width.
    AllowOverflow,
    /// Scale width, ascent and descent uniformly by `wrap_width / width`.
    ScaleToFit,
}

/// A rigid item of the line-breaking sequence (spec §2.4).
///
/// A text box owns no atoms or clusters: it names a contiguous slice of each
/// in the paragraph-wide [`ParagraphAst::atoms`](super::ParagraphAst::atoms) and [`ParagraphAst::clusters`](super::ParagraphAst::clusters)
/// (see [`ParagraphAst::text_box`](super::ParagraphAst::text_box) for the resolved view).
#[derive(Clone, Debug, PartialEq)]
pub enum BoxNode {
    /// Shaped text: a word, a fragment of one, or a punctuation mark. Closing
    /// and inseparable punctuation (UAX #14 CL, CP, EX, IS, SY, IN) is fused to
    /// its neighbour by the **absence** of a [`DiscretionaryNode`], not by a
    /// node of its own.
    Text {
        /// Index into [`ParagraphAst::runs`](super::ParagraphAst::runs).
        run_index: usize,
        /// Source bytes: first atom's start to last atom's end.
        byte_range: TextRange,
        /// The box's atoms, as **paragraph-level** indices: `ast.atoms[atom_range]`.
        /// Inside the run's own [`ShapingRun::atom_range`](crate::types::ShapingRun::atom_range). Never empty: a
        /// text box owns at least one atom.
        atom_range: Range<usize>,
        /// The box's clusters, as **paragraph-level** indices:
        /// `ast.clusters[cluster_range]`. Inside the run's own
        /// [`ShapingRun::cluster_range`](crate::types::ShapingRun::cluster_range). Each cluster's `atom_range` is
        /// **run-local** (see [`ShapedCluster`](crate::types::ShapedCluster)).
        cluster_range: Range<usize>,
        /// Sum of the clusters' `natural_width`s: a validated cache (spec §1.5,
        /// AX-2), because the line fitter reads it for every candidate;
        /// `check_layout_inputs` requires exact agreement with the clusters.
        natural_width: f32,
    },
    /// An embedded object with fixed extents.
    InlineObject {
        /// Alignment relative to the line baseline.
        baseline_align: BaselineAlignment,
        /// Source bytes the object stands for, usually one U+FFFC. Never
        /// empty: carets and selections address the object through them.
        byte_range: TextRange,
        /// Caller-assigned identifier.
        box_id: u64,
        /// Width.
        width: f32,
        /// Extent above the baseline.
        ascent: f32,
        /// Extent below the baseline.
        descent: f32,
        /// Behaviour when wider than the line.
        overflow_policy: BoxOverflowPolicy,
        /// Resolved UAX #9 level of the object's source (U+FFFC is a neutral,
        /// so the level comes from its context).
        bidi_level: BidiLevel,
    },
    /// The padding-plus-border advance at one end of an inline decoration span.
    SpanBoundary {
        /// Empty range anchored at the span's start or end.
        byte_range: TextRange,
        /// Identifier of the [`InlineDecorationSpan`](crate::frontend::InlineDecorationSpan).
        span_id: u64,
        /// Which end of the span.
        edge: SpanEdge,
        /// `start_padding` or `end_padding` of the span.
        width: f32,
        /// UAX #9 level of the boundary: a `Leading` edge takes the level of
        /// the first [`GraphemeAtom`](crate::types::GraphemeAtom) inside the span, a `Trailing` edge that
        /// of the last. Needed to place the padding after bidi reordering.
        bidi_level: BidiLevel,
    },
}

/// One item of the line-breaking sequence (spec §2.4): a four-variant
/// algebra after Omega / LuaTeX.
///
/// The frontend is a compiler from linguistic rules (UAX #14, kinsoku,
/// `no_line_end_words`, hyphenation, `chws`) to these primitives; the
/// middle-end is a language-agnostic solver. **Invariant:** the line breaker
/// may end a line only at a [`WordNode::Discretionary`] (bar the emergency
/// cluster break). A breakable space is `[Box, Glue, Discretionary]`: the
/// discretionary comes **after** the glue run, so on a break the spaces stay on
/// the trailing edge of line `k` (spec §2.6: `byte_range` includes trailing
/// whitespace, `visible_byte_end` does not). Several consecutive spaces share
/// one discretionary at the end of the run. A space the frontend binds to the
/// next word (Czech `v `) is `[Box, Glue]`; a no-break space character is
/// text in a box.
#[derive(Clone, Debug, PartialEq)]
pub enum WordNode {
    /// A rigid item.
    Box(BoxNode),
    /// Stretchable, shrinkable space.
    Glue(GlueNode),
    /// A break opportunity.
    Discretionary(DiscretionaryNode),
    /// A tab; width is `TabStops::next_stop(x) - x`, evaluated during layout.
    Tab {
        /// Source bytes.
        ///
        /// A tab stores no bidi level: it is a segment separator, which UAX #9
        /// rule L1 resets to the paragraph level,
        /// `ParagraphAst::base_direction.base_level()` (spec §1.5, AX-2).
        byte_range: TextRange,
    },
}

impl WordNode {
    /// The source bytes this node covers: the stored range for every variant
    /// (a `Discretionary` owns its `byte_range`: empty for a soft opportunity,
    /// the newline for a hard break). Zero-width nodes report an empty
    /// `offset..offset` range, so every node has a position in the source.
    #[must_use]
    pub fn byte_span(&self) -> TextRange {
        match self {
            Self::Box(BoxNode::Text { byte_range, .. })
            | Self::Box(BoxNode::InlineObject { byte_range, .. })
            | Self::Box(BoxNode::SpanBoundary { byte_range, .. })
            | Self::Glue(GlueNode { byte_range, .. })
            | Self::Discretionary(DiscretionaryNode { byte_range, .. })
            | Self::Tab { byte_range } => *byte_range,
        }
    }

    /// `true` for the only node a line may end at.
    #[must_use]
    pub const fn is_break_opportunity(&self) -> bool {
        matches!(self, Self::Discretionary(_))
    }
}
