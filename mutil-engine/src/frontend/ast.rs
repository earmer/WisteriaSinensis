//! Paragraph AST and the box-and-glue sequence (spec §2.4, §2.5).

use super::{AtomicEditSpan, InlineDecorationSpan, ShapingRun};
use crate::types::{
    AtomFlags, BidiDirection, BidiLevel, ByteIndex, FontMetrics, GraphemeAtom, ShapedCluster,
    TextRange,
};
use core::ops::Range;
use smallvec::SmallVec;

/// Replacement text for spelling-change hyphenation.
///
/// Replacements are rare (`Option`, usually `None`), so `Box<str>` costs 16
/// bytes inline (niche-optimised) and one allocation only when present.
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
    /// Original bytes removed immediately **before** `byte_offset`. A cut that
    /// straddles the break is split between the two counts. Each cut ends on an
    /// atom boundary within the word's contiguous same-run hyphen chain, so it
    /// may cross an unchosen hyphenation point of the same word but never a run
    /// or span boundary.
    pub prefix_cut_bytes: u32,
    /// Original bytes removed immediately **after** `byte_offset`; the same
    /// rules apply.
    pub suffix_cut_bytes: u32,
}

/// Kind of stretchable space (spec §2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlueKind {
    /// An ordinary word space.
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
    /// The tier the frontend assigns: `WordSpace` and `CjkPunctCompression` to
    /// [`GlueTier::Tier0`], `CjkAutospace` to [`GlueTier::Tier1`]. **No kind
    /// maps to [`GlueTier::Tier2`]**: inter-ideographic stretch is derived from
    /// `ALLOW_CJK_JUSTIFY_AFTER` on the clusters, not from glue nodes (spec §2.4).
    #[must_use]
    pub const fn default_tier(self) -> GlueTier {
        match self {
            Self::WordSpace | Self::CjkPunctCompression => GlueTier::Tier0,
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
    /// natural width, stretch only. A tier of the middle-end's prefix-sum
    /// tables: a `GlueNode` never carries it (`check_layout_inputs` rejects
    /// one), because explicit Tier 2 glue would inflate a CJK paragraph's node
    /// count by a third.
    Tier2 = 2,
}

/// Stretchable, shrinkable space (spec §2.4).
///
/// Glue is **never** an implicit break point: a breakable space is followed by
/// a [`DiscretionaryNode`] emitted by the frontend (see [`WordNode`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlueNode {
    /// Original Unicode bidi class for L1.
    pub original_bidi_class: crate::types::BidiClass,
    /// Resolved level before line-local L1.
    pub bidi_level: crate::types::BidiLevel,
    /// Only collapsible trailing whitespace can disappear.
    pub is_collapsible: bool,
    /// Source bytes (empty for `CjkAutospace` and `CjkPunctCompression`, which
    /// have `start == end`).
    pub byte_range: TextRange,
    /// Space kind.
    pub kind: GlueKind,
    /// Priority tier: `Tier0` or `Tier1` (never `Tier2`). A
    /// `CjkPunctCompression` node is `Tier0`.
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
/// in the paragraph-wide [`ParagraphAst::atoms`] and [`ParagraphAst::clusters`]
/// (see [`ParagraphAst::text_box`] for the resolved view).
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
        /// The box's atoms, as **paragraph-level** indices: `ast.atoms[atom_range]`.
        /// Inside the run's own [`ShapingRun::atom_range`].
        atom_range: Range<usize>,
        /// The box's clusters, as **paragraph-level** indices:
        /// `ast.clusters[cluster_range]`. Inside the run's own
        /// [`ShapingRun::cluster_range`]. Each cluster's `atom_range` is
        /// **run-local** (see [`ShapedCluster`]).
        cluster_range: Range<usize>,
        /// Sum of cluster widths.
        natural_width: f32,
    },
    /// An embedded object with fixed extents.
    InlineObject {
        /// Alignment relative to the line baseline.
        baseline_align: crate::types::BaselineAlignment,
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
        /// UAX #9 level of the boundary: a `Leading` edge takes the level of
        /// the first [`GraphemeAtom`] inside the span, a `Trailing` edge that
        /// of the last. Needed to place the padding after bidi reordering.
        bidi_level: BidiLevel,
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
    ///
    /// `is_paragraph_end == false` is an explicit UAX #14 terminator present in
    /// the text (`\n`, `\r\n`, U+2028, ...): its `byte_range` is non-empty and
    /// is exactly those bytes. `is_paragraph_end == true` is the synthetic
    /// end-of-paragraph sentinel: its `byte_range` is the empty range
    /// `text.len()..text.len()` and it is the **last** discretionary of the
    /// paragraph, at most once. Text ending in `\n` therefore ends with a real
    /// hard break followed by the sentinel (the trailing blank line).
    HardBreak {
        /// `true` for the sentinel, `false` for a terminator in the text.
        is_paragraph_end: bool,
    },
    /// Hyphenation: `TextShaper::reshape_window` runs at materialisation. Both
    /// neighbouring text boxes share one `run_index` (a word is split within a
    /// single `ShapingRun`). The widths minus the cloned padding are the net
    /// prefix and suffix shaping deltas; the cuts stay within the contiguous
    /// same-run chain of text boxes joined by hyphenation points
    /// (`check_layout_inputs`). When the break is **not** taken across a seam
    /// that split a cluster, the two boxes are re-shaped together; see
    /// [`ParagraphAst::rejoin_window`].
    Hyphenate(HyphenOpportunity),
    /// Restore or trim `chws`-compressed CJK punctuation at the line edges.
    ///
    /// A unit variant: the data already lives in the node's widths. After the
    /// cloned padding is subtracted, `pre_break_width` is the signed delta added
    /// to the `natural_width` of the last cluster of the immediately preceding
    /// [`BoxNode::Text`] (the end of line `k`); `post_break_width` the delta
    /// added to the first cluster of the immediately following one (the start
    /// of line `k + 1`). At least one delta is non-zero; a non-zero delta needs
    /// that text box, and the cluster's width plus the delta must stay
    /// non-negative.
    CjkPunctAdjust,
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
    /// Bytes this opportunity owns. An **empty** range at the break position
    /// for every soft opportunity (after a space, between ideographs,
    /// hyphenation, a math operator); for a [`BreakAction::HardBreak`] the exact
    /// newline sequence (`\n` 1 byte, `\r\n` 2, U+2028 3), or an empty range
    /// at the end of the paragraph. The next line starts at `byte_range.end`, so
    /// no byte is left unowned.
    pub byte_range: TextRange,
    /// Knuth-Plass penalty; see the associated constants for the canonical scale.
    pub penalty: i32,
    /// `true` for hyphenation breaks (triggers the alpha demerits).
    pub flagged: bool,
    /// Width added to the end of line `k` if broken here: the `end_padding` of
    /// every span in `cloned_spans` plus the action's own delta (see
    /// [`BreakAction`]); for `Direct`, a real `HardBreak` and
    /// `MathOperator { repeat_glyph_id: None }` the padding sum alone.
    pub pre_break_width: f32,
    /// Width added to the start of line `k + 1` if broken here: the
    /// `start_padding` of every span in `cloned_spans` plus the action's own
    /// delta.
    pub post_break_width: f32,
    /// Width contributed to the current line if **not** broken here (for
    /// example the letter-spacing of the skipped opportunity). Finite; zero for
    /// a hard break.
    pub no_break_width: f32,
    /// Work done when the break is chosen.
    pub action: BreakAction,
    /// Ids of the [`BoxDecorationBreak::Clone`](super::BoxDecorationBreak::Clone)
    /// spans this break falls strictly inside. Cloning padding is orthogonal to
    /// `action` (a hyphenation break inside a chip is both): when the line
    /// breaks here, each listed span gets its cloned `end_padding` on line `k`
    /// (inside `pre_break_width`) and its cloned `start_padding` on line `k + 1`
    /// (inside `post_break_width`).
    pub cloned_spans: SmallVec<[u64; 2]>,
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

    /// A zero-cost, zero-width natural break at `byte_offset` (an empty range).
    #[must_use]
    pub fn direct(byte_offset: ByteIndex) -> Self {
        Self {
            byte_range: TextRange::empty_at(byte_offset),
            penalty: Self::NATURAL_PENALTY,
            flagged: false,
            pre_break_width: 0.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::Direct,
            cloned_spans: SmallVec::new(),
        }
    }

    /// A mandatory break. For `is_paragraph_end == false`, `newline` is the
    /// exact non-empty range of a U+2028 line separator within a paragraph;
    /// paragraph separators (`\n`, `\r`, `\r\n`, U+0085, U+2029, U+001C–U+001E)
    /// are not allowed here because a paragraph AST covers one paragraph only.
    /// Split document text with
    /// [`split_paragraphs`](crate::frontend::split_paragraphs) first. For
    /// `is_paragraph_end == true` the range is `text.len()..text.len()`.
    #[must_use]
    pub fn hard_break(newline: TextRange, is_paragraph_end: bool) -> Self {
        Self {
            byte_range: newline,
            penalty: Self::MANDATORY_BREAK_PENALTY,
            flagged: false,
            pre_break_width: 0.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::HardBreak { is_paragraph_end },
            cloned_spans: SmallVec::new(),
        }
    }

    /// The break position: where line `k` ends and, for a soft opportunity,
    /// line `k + 1` begins (`byte_range.start`).
    #[must_use]
    pub const fn byte_offset(&self) -> ByteIndex {
        self.byte_range.start
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
/// cluster break). A breakable space is `[Box, Glue, Discretionary]`: the
/// discretionary comes **after** the glue run, so on a break the spaces stay on
/// the trailing edge of line `k` (spec §2.6: `byte_range` includes trailing
/// whitespace, `visible_byte_end` does not). Several consecutive spaces share
/// one discretionary at the end of the run. A non-breaking space (NBSP, Czech
/// `v `) is `[Box, Glue]`.
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

/// Checks the invariants documented on [`ShapedCluster`] for one text box.
/// `atoms` and `clusters` are the box's resolved slices; `base` is the
/// run-local index of the box's first atom (`box.atom_range.start -
/// run.atom_range.start`), which converts each cluster's run-local
/// `atom_range` into an index of `atoms`.
pub(crate) fn text_is_self_contained(
    byte_range: TextRange,
    atoms: &[GraphemeAtom],
    clusters: &[ShapedCluster],
    base: usize,
) -> bool {
    let span_ok = match (atoms.first(), atoms.last()) {
        (Some(first), Some(last)) => {
            byte_range.start == first.byte_range.start && byte_range.end == last.byte_range.end
        }
        _ => byte_range.is_empty(),
    };
    span_ok
        && atoms
            .windows(2)
            .all(|w| w[0].byte_range.end == w[1].byte_range.start)
        && atoms.iter().all(|a| a.byte_range.start < a.byte_range.end)
        && clusters_partition_atoms(atoms, clusters, base)
}

fn clusters_partition_atoms(
    atoms: &[GraphemeAtom],
    clusters: &[ShapedCluster],
    base: usize,
) -> bool {
    let mut next = base;
    for c in clusters {
        let r = &c.atom_range;
        if r.start != next || r.end <= r.start || r.end > base + atoms.len() {
            return false;
        }
        let local = r.start - base..r.end - base;
        let (first, last) = (&atoms[local.start], &atoms[local.end - 1]);
        if c.byte_range.start != first.byte_range.start || c.byte_range.end != last.byte_range.end {
            return false;
        }
        // Invariant 3: the cluster's flags are the per-flag aggregate of its atoms.
        if c.flags != AtomFlags::for_cluster(&atoms[local]) {
            return false;
        }
        next = r.end;
    }
    next == base + atoms.len()
}

/// A [`BoxNode::Text`] with its atoms and clusters resolved through the
/// paragraph-wide arrays.
///
/// `clusters[i].atom_range` is **run-local**: it indexes the run's atom slice
/// ([`ParagraphAst::run_atoms`]), not `atoms`.
#[derive(Clone, Copy, Debug)]
pub struct TextBoxView<'a> {
    /// Index into [`ParagraphAst::runs`].
    pub run_index: usize,
    /// Source bytes: first atom's start to last atom's end.
    pub byte_range: TextRange,
    /// `ast.atoms[atom_range]` of the box.
    pub atoms: &'a [GraphemeAtom],
    /// `ast.clusters[cluster_range]` of the box.
    pub clusters: &'a [ShapedCluster],
    /// Sum of cluster widths, as cached on the box.
    pub natural_width: f32,
}

/// Frontend output: everything the middle-end needs, independent of column
/// width (spec §2.5, §1.3).
///
/// Owns all its data (no borrows), so it is `'static` and `Send + Sync`. The
/// paragraph text is not stored here, matching the spec: shapers receive it as
/// `&str` per call.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphAst {
    /// Canonical-to-physical writing mode.
    pub writing_mode: crate::types::WritingMode,
    /// Explicit directional controls in source coordinates.
    pub bidi_spans: Vec<super::ExplicitBidiSpan>,
    /// Independent line-over and line-under annotations.
    pub annotations: Vec<super::AnnotationSpan>,
    /// Resolved paragraph base direction (UAX #9 P2/P3 or the caller's override).
    pub base_direction: BidiDirection,
    /// The **raw** primary-font metrics exactly as `TextShaper::
    /// resolve_primary_font_metrics` returned them for the primary run's
    /// variation instance. The frontend neither reads
    /// `LineHeight` nor computes half-leading; the middle-end resolves the
    /// active strut from this value with `LineHeight::resolve_strut`
    /// (§5 Stage 4.1).
    pub strut_metrics: FontMetrics,
    /// The font size (layout units) the strut metrics were requested at. The
    /// middle-end needs it to resolve a `LineHeight::Fixed` strut.
    pub strut_font_size: f32,
    /// Shaping runs in logical order.
    pub runs: Vec<ShapingRun>,
    /// Every atom of every body run, in run order then logical order. Run `i`
    /// owns `atoms[runs[i].atom_range]`; the runs tile this vector.
    pub atoms: Vec<GraphemeAtom>,
    /// Every shaped cluster of every body run, in run order then logical order.
    /// Run `i` owns `clusters[runs[i].cluster_range]`; the runs tile this
    /// vector. Each cluster's `atom_range` is run-local (see [`ShapedCluster`]).
    pub clusters: Vec<ShapedCluster>,
    /// Box-and-glue sequence in logical order.
    pub items: Vec<WordNode>,
    /// Rich-text decoration spans (chips, code spans): padding and
    /// fragmentation behaviour. Their boundaries appear in `items` as
    /// `BoxNode::SpanBoundary`.
    pub decoration_spans: Vec<InlineDecorationSpan>,
    /// Spans the caret and deletion treat as one unit (mentions, chips).
    /// Orthogonal to `decoration_spans`: a span may be either, both or neither.
    /// The atoms inside carry `AtomFlags::IN_ATOMIC_EDIT_SPAN`, and no others do.
    pub atomic_spans: Vec<AtomicEditSpan>,
}

impl ParagraphAst {
    /// The atoms of body run `run`: `atoms[runs[run].atom_range]`, which is
    /// exactly the slice `TextShaper::shape_run` and `reshape_window` take.
    /// Empty for an out-of-range run or range (validation reports the error).
    #[must_use]
    pub fn run_atoms(&self, run: usize) -> &[GraphemeAtom] {
        self.runs
            .get(run)
            .and_then(|r| self.atoms.get(r.atom_range.clone()))
            .unwrap_or_default()
    }

    /// The clusters of body run `run`: `clusters[runs[run].cluster_range]`;
    /// their `atom_range`s index [`run_atoms`](Self::run_atoms). Empty for an
    /// out-of-range run or range.
    #[must_use]
    pub fn run_clusters(&self, run: usize) -> &[ShapedCluster] {
        self.runs
            .get(run)
            .and_then(|r| self.clusters.get(r.cluster_range.clone()))
            .unwrap_or_default()
    }

    /// The text box at `items[item]` with its slices resolved, or `None` when
    /// that item is not a [`BoxNode::Text`] or one of its ranges is out of
    /// bounds.
    #[must_use]
    pub fn text_box(&self, item: usize) -> Option<TextBoxView<'_>> {
        let WordNode::Box(BoxNode::Text {
            run_index,
            byte_range,
            atom_range,
            cluster_range,
            natural_width,
        }) = self.items.get(item)?
        else {
            return None;
        };
        Some(TextBoxView {
            run_index: *run_index,
            byte_range: *byte_range,
            atoms: self.atoms.get(atom_range.clone())?,
            clusters: self.clusters.get(cluster_range.clone())?,
            natural_width: *natural_width,
        })
    }

    /// Checks the invariants documented on [`ShapedCluster`] for the text box at
    /// `items[item]`: its atoms are contiguous and tile its `byte_range` (an
    /// empty box has no atoms), its clusters partition its atoms in order with
    /// matching byte ranges, and each cluster's flags aggregate its atoms'.
    /// Every other node is trivially self-contained; an `item` past the end,
    /// or a text box whose ranges do not resolve or leave its run's ranges,
    /// is not.
    #[must_use]
    pub fn is_self_contained(&self, item: usize) -> bool {
        match self.items.get(item) {
            Some(WordNode::Box(BoxNode::Text {
                atom_range,
                cluster_range,
                ..
            })) => {
                let Some(view) = self.text_box(item) else {
                    return false;
                };
                let Some(run) = self.runs.get(view.run_index) else {
                    return false;
                };
                let inside_run = run.atom_range.start <= atom_range.start
                    && atom_range.end <= run.atom_range.end
                    && run.cluster_range.start <= cluster_range.start
                    && cluster_range.end <= run.cluster_range.end;
                inside_run
                    && text_is_self_contained(
                        view.byte_range,
                        view.atoms,
                        view.clusters,
                        atom_range.start - run.atom_range.start,
                    )
            }
            Some(_) => true,
            None => false,
        }
    }

    /// The window to re-shape as **one** piece when the `Hyphenate`
    /// discretionary at `items[index]` is *not* taken on a committed line
    /// (spec §5 Stage 1.6.5, rejoin rule).
    ///
    /// A hyphenation opportunity inside a multi-atom cluster (the `of-fice`
    /// split of an `ffi` ligature) is resolved early by re-shaping the two
    /// sides separately (§2.3 note 3). The frontend marks such a seam by giving
    /// the first cluster after it `safe_to_concat_before == false`. If the
    /// break is then not taken, concatenating the two separately shaped boxes
    /// would lose the ligature and any kerning across the seam, so the
    /// middle-end calls `TextShaper::reshape_window` once over the returned
    /// range with `LeadingEdit::None` and `TrailingEdit::None`.
    ///
    /// Returns the byte range from the start of the preceding text box to the
    /// end of the following one, or `None` when `items[index]` is not a
    /// `Hyphenate` discretionary between two text boxes or its seam is safe to
    /// concatenate. Windows of consecutive untaken seams overlap and should be
    /// merged so that each atom is shaped once.
    #[must_use]
    pub fn rejoin_window(&self, index: usize) -> Option<TextRange> {
        let WordNode::Discretionary(DiscretionaryNode {
            action: BreakAction::Hyphenate(_),
            ..
        }) = self.items.get(index)?
        else {
            return None;
        };
        let before = self.text_box(index.checked_sub(1)?)?.byte_range;
        let after = self.text_box(index + 1)?;
        let seam_is_unsafe = after
            .clusters
            .first()
            .is_some_and(|c| !c.safe_to_concat_before);
        seam_is_unsafe.then(|| TextRange::new(before.start, after.byte_range.end))
    }
}
