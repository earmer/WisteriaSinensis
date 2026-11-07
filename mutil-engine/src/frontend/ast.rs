//! Paragraph AST and the box-and-glue sequence (spec §2.4, §2.5).

use super::{AtomicEditSpan, InlineDecorationSpan, ShapingRun};
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
    /// Break penalty (feeds the Knuth-Plass demerits, §5 Stage 1.6).
    pub penalty: i32,
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

impl GlueKind {
    /// The tier the frontend assigns by default: `WordSpace` to
    /// [`GlueTier::Tier0`], `CjkAutospace` to [`GlueTier::Tier1`]. Tier 2 has no
    /// kind of its own yet (Open question 33).
    #[must_use]
    pub const fn default_tier(self) -> GlueTier {
        match self {
            Self::WordSpace => GlueTier::Tier0,
            Self::CjkAutospace => GlueTier::Tier1,
        }
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
    /// natural width, stretch only.
    Tier2 = 2,
}

/// Stretchable, shrinkable space (spec §2.4).
///
/// Glue is **never** an implicit break point: a breakable space is preceded by
/// a [`DiscretionaryNode`] emitted by the frontend (see [`WordNode`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlueNode {
    /// Source bytes (empty for `CjkAutospace`, which has `start == end`).
    pub byte_range: TextRange,
    /// Space kind.
    pub kind: GlueKind,
    /// Priority tier.
    pub tier: GlueTier,
    /// Natural width.
    pub natural_width: f32,
    /// Maximum added width.
    pub max_stretch: f32,
    /// Maximum removed width.
    pub max_shrink: f32,
}

/// Which end of an [`InlineDecorationSpan`](super::InlineDecorationSpan) a
/// [`BoxNode::SpanBoundary`] stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpanEdge {
    /// The logical start of the span (left border plus padding).
    Leading,
    /// The logical end of the span (right border plus padding).
    Trailing,
}

/// What happens to a rigid inline object that is wider than the line (spec
/// §2.4, row 76).
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
/// Collections are `Vec`, not `SmallVec`: inline storage would inflate every
/// variant to the size of the largest. `ShapedCluster::atom_range` indexes the
/// node's own `atoms` (see [`ShapedCluster`]).
#[derive(Clone, Debug, PartialEq)]
pub enum BoxNode {
    /// Shaped text: a word, a fragment of one, or a punctuation mark. Closing
    /// and inseparable punctuation (UAX #14 CL, CP, EX, IS, SY, IN) is fused to
    /// its neighbour by the **absence** of a [`DiscretionaryNode`], not by a
    /// node of its own.
    Text {
        /// Index into [`ParagraphAst::runs`].
        run_index: usize,
        /// Source bytes: first atom's start to last atom's end.
        byte_range: TextRange,
        /// The box's atoms.
        atoms: Vec<GraphemeAtom>,
        /// Shaped clusters; `atom_range` is relative to `atoms`.
        clusters: Vec<ShapedCluster>,
        /// Sum of cluster widths.
        natural_width: f32,
    },
    /// An embedded object with fixed extents.
    InlineObject {
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
        /// Behaviour when wider than the line.
        overflow_policy: BoxOverflowPolicy,
    },
    /// The padding-plus-border advance at one end of an inline decoration span.
    SpanBoundary {
        /// Empty range anchored at the span's start or end.
        byte_range: TextRange,
        /// Identifier of the [`InlineDecorationSpan`](super::InlineDecorationSpan).
        span_id: u64,
        /// Which end of the span.
        edge: SpanEdge,
        /// `start_padding` or `end_padding` of the span.
        width: f32,
    },
}

/// Opportunity-specific work done when a [`DiscretionaryNode`] is chosen as a
/// line end (spec §2.4). The payload is applied lazily, at commit time
/// (§5 Stage 1.6.5): candidate evaluation uses the three widths only.
#[derive(Clone, Debug, PartialEq)]
pub enum BreakAction {
    /// Zero-cost break between words, Han ideographs or Thai words.
    Direct,
    /// Mandatory line termination (`penalty == MANDATORY_BREAK_PENALTY`).
    HardBreak {
        /// `true` for the end of the paragraph, `false` for `\n`, `\r\n`
        /// and U+2028 inside it.
        is_paragraph_end: bool,
    },
    /// Hyphenation: `TextShaper::reshape_window` runs at materialisation.
    Hyphenate(HyphenOpportunity),
    /// Restore or trim `chws`-compressed CJK punctuation at the line edges.
    CjkPunctAdjust {
        /// Cluster at the end of line `k`.
        trailing_cluster_index: usize,
        /// Cluster at the start of line `k + 1`.
        leading_cluster_index: usize,
    },
    /// Inject cloned padding when breaking inside a
    /// `BoxDecorationBreak::Clone` span.
    CloneSpanPadding {
        /// The span whose padding is cloned.
        span_id: u64,
    },
    /// Break after a top-level math relation (`=`) or binary operator (`+`).
    MathOperator {
        /// Glyph repeated at the start of the next line, if any.
        repeat_glyph_id: Option<u32>,
    },
}

/// Generalised three-state penalty node after Omega / LuaTeX (spec §2.4): the
/// **only** place a line may end (except for an emergency cluster break on an
/// overflowing empty line).
///
/// If the line breaks here, line `k` gains `pre_break_width` and line `k + 1`
/// gains `post_break_width`; if not, the current line gains `no_break_width`.
/// Widths are signed deltas (a trim or restore may be negative) but finite.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscretionaryNode {
    /// Break position in the original text.
    pub byte_offset: ByteIndex,
    /// Knuth-Plass penalty; see the associated constants for the canonical scale.
    pub penalty: i32,
    /// `true` for hyphenation breaks (triggers the alpha demerits).
    pub flagged: bool,
    /// Width added to the end of line `k` if broken here.
    pub pre_break_width: f32,
    /// Width added to the start of line `k + 1` if broken here.
    pub post_break_width: f32,
    /// Width contributed to the current line if **not** broken here.
    pub no_break_width: f32,
    /// Work done when the break is chosen.
    pub action: BreakAction,
}

impl DiscretionaryNode {
    /// Mandatory break (`\n`, `\r\n`, U+2028, end of paragraph): `-INF_PENALTY`.
    pub const MANDATORY_BREAK_PENALTY: i32 = -10_000;
    /// An unpenalised natural break.
    pub const NATURAL_PENALTY: i32 = 0;
    /// Lowest URL interior break penalty (slash and punctuation breaks).
    pub const URL_PENALTY_MIN: i32 = 25;
    /// Highest URL interior break penalty.
    pub const URL_PENALTY_MAX: i32 = 50;
    /// Standard Liang-pattern hyphenation (`hypher`); flagged.
    pub const HYPHEN_PENALTY: i32 = 135;
    /// Spelling-change hyphenation (libhyphen Dutch, Catalan, Hungarian); flagged.
    pub const SPELLING_CHANGE_HYPHEN_PENALTY: i32 = 250;
    /// Added to every break inside a `BoxDecorationBreak::Clone` span.
    pub const CLONED_SPAN_PENALTY: i32 = 300;
    /// Break after a top-level inline math relation (`\relpenalty`).
    pub const MATH_RELATION_PENALTY: i32 = 500;
    /// Break after a top-level inline math binary operator (`\binoppenalty`).
    pub const MATH_BINARY_OPERATOR_PENALTY: i32 = 700;
    /// A forbidden break is represented by **omitting** the node; a node with
    /// this penalty or more is rejected by `check_layout_inputs`.
    pub const FORBIDDEN_BREAK_PENALTY: i32 = 10_000;

    /// A zero-cost, zero-width natural break at `byte_offset`.
    #[must_use]
    pub const fn direct(byte_offset: ByteIndex) -> Self {
        Self {
            byte_offset,
            penalty: Self::NATURAL_PENALTY,
            flagged: false,
            pre_break_width: 0.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::Direct,
        }
    }

    /// A mandatory break at `byte_offset`.
    #[must_use]
    pub const fn hard_break(byte_offset: ByteIndex, is_paragraph_end: bool) -> Self {
        Self {
            byte_offset,
            penalty: Self::MANDATORY_BREAK_PENALTY,
            flagged: false,
            pre_break_width: 0.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::HardBreak { is_paragraph_end },
        }
    }

    /// `true` when the break is mandatory (`penalty <= MANDATORY_BREAK_PENALTY`).
    #[must_use]
    pub const fn is_mandatory(&self) -> bool {
        self.penalty <= Self::MANDATORY_BREAK_PENALTY
    }
}

/// One item of the line-breaking sequence (spec §2.4): a four-variant
/// algebra after Omega / LuaTeX.
///
/// The frontend is a compiler from linguistic rules (UAX #14, kinsoku,
/// `no_line_end_words`, hyphenation, `chws`) to these primitives; the
/// middle-end is a language-agnostic solver. **Invariant:** the line breaker
/// may end a line only at a [`WordNode::Discretionary`] (bar the emergency
/// cluster break). A breakable space is `[Box, Discretionary, Glue]`; a
/// non-breaking one (NBSP, Czech `v `) is `[Box, Glue]`.
#[derive(Clone, Debug, PartialEq)]
pub enum WordNode {
    /// A rigid item.
    Box(BoxNode),
    /// Stretchable, shrinkable space.
    Glue(GlueNode),
    /// A break opportunity.
    Discretionary(DiscretionaryNode),
    /// A tab; width is `next_tab_stop(x) - x`, evaluated during layout.
    Tab {
        /// Source bytes.
        byte_range: TextRange,
    },
}

impl WordNode {
    /// The source bytes this node covers: the stored range for `Box`, `Glue`
    /// and `Tab`, an empty range at `byte_offset` for a `Discretionary`. Always
    /// `Some`; the `Option` is kept for call-site compatibility with earlier
    /// revisions in which atom-derived spans could be missing.
    #[must_use]
    pub fn byte_span(&self) -> Option<TextRange> {
        Some(match self {
            Self::Box(BoxNode::Text { byte_range, .. })
            | Self::Box(BoxNode::InlineObject { byte_range, .. })
            | Self::Box(BoxNode::SpanBoundary { byte_range, .. })
            | Self::Glue(GlueNode { byte_range, .. })
            | Self::Tab { byte_range } => *byte_range,
            Self::Discretionary(d) => TextRange::empty_at(d.byte_offset),
        })
    }

    /// `true` for the only node a line may end at.
    #[must_use]
    pub const fn is_break_opportunity(&self) -> bool {
        matches!(self, Self::Discretionary(_))
    }

    /// Checks the `atom_range` invariants documented on [`ShapedCluster`] for a
    /// `Box::Text` (the clusters partition its atoms, in order, with matching
    /// byte ranges) and that its `byte_range` runs from the first atom's start
    /// to the last atom's end (an empty one has no atoms). Every other node is
    /// trivially self-contained.
    #[must_use]
    pub fn is_self_contained(&self) -> bool {
        match self {
            Self::Box(BoxNode::Text {
                byte_range,
                atoms,
                clusters,
                ..
            }) => {
                let span_ok = match (atoms.first(), atoms.last()) {
                    (Some(first), Some(last)) => {
                        byte_range.start == first.byte_range.start
                            && byte_range.end == last.byte_range.end
                    }
                    _ => byte_range.is_empty(),
                };
                span_ok && clusters_partition_atoms(atoms, clusters)
            }
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
    /// Rich-text decoration spans (chips, code spans): padding and
    /// fragmentation behaviour. Their boundaries appear in `items` as
    /// `BoxNode::SpanBoundary`.
    pub decoration_spans: Vec<InlineDecorationSpan>,
    /// Spans the caret and deletion treat as one unit (mentions, chips).
    /// Orthogonal to `decoration_spans`: a span may be either, both or neither.
    pub atomic_spans: Vec<AtomicEditSpan>,
}
