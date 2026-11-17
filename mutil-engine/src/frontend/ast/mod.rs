//! Paragraph AST and the box-and-glue sequence (spec §2.4, §2.5).
//!
//! The node algebra lives in `nodes` and `discretionary`; the
//! self-containment check of a text payload in `self_contained`; the
//! hyphenation seams and the re-shape windows they imply in `hyphen`.

mod discretionary;
mod hyphen;
mod nodes;
mod self_contained;

pub use discretionary::{BreakAction, DiscretionaryNode, RepeatedOperator};
pub use nodes::{BoxNode, BoxOverflowPolicy, GlueKind, GlueNode, GlueTier, SpanEdge, WordNode};
pub(crate) use self_contained::text_is_self_contained;

use super::{AnnotationSpan, AtomicEditSpan, ExplicitBidiSpan, InlineDecorationSpan, ShapingRun};
use crate::types::{
    BidiDirection, ByteIndex, FontMetrics, GraphemeAtom, RawShapedGlyph, ShapedCluster, TextRange,
    WritingMode,
};
use core::ops::Range;

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
    /// Paragraph-level index of `atoms[0]`: the start of the box's
    /// `atom_range`.
    pub atom_start: usize,
    /// `ast.atoms[atom_range]` of the box.
    pub atoms: &'a [GraphemeAtom],
    /// `ast.clusters[cluster_range]` of the box.
    pub clusters: &'a [ShapedCluster],
    /// Sum of cluster widths, as cached on the box.
    pub natural_width: f32,
}

/// A stretch of one shaping run to re-shape as a unit: the arguments
/// `TextShaper::reshape_window` needs besides the edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReshapeWindow {
    /// Index into [`ParagraphAst::runs`] of the run that owns the window.
    pub run_index: usize,
    /// The window's atoms, as **paragraph-level** indices:
    /// `ast.atoms[atom_range]`. The clusters to replace are those of the text
    /// boxes covering exactly these atoms.
    pub atom_range: Range<usize>,
    /// The window in paragraph text coordinates; starts and ends on atom
    /// boundaries.
    pub byte_range: TextRange,
}

/// Frontend output: everything the middle-end needs, independent of column
/// width (spec §2.5, §1.3).
///
/// Owns all its data (no borrows), so it is `'static` and `Send + Sync`. The
/// paragraph text is not stored here, matching the spec: shapers receive it as
/// `&str` per call.
///
/// Shaped text is stored once, in three flat arrays: `atoms`, the `clusters`
/// that group them, and the `glyphs` that render the clusters. Runs, text
/// boxes and clusters name slices of the next array by range.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphAst {
    /// Canonical-to-physical writing mode.
    pub writing_mode: WritingMode,
    /// Explicit directional controls in source coordinates.
    pub bidi_spans: Vec<ExplicitBidiSpan>,
    /// Independent line-over and line-under annotations.
    pub annotations: Vec<AnnotationSpan>,
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
    /// Every glyph of every body cluster, in cluster order then logical order.
    /// Cluster `i` owns `glyphs[clusters[i].glyph_range]`; the clusters tile
    /// this vector.
    pub glyphs: Vec<RawShapedGlyph>,
    /// Box-and-glue sequence in logical order.
    pub items: Vec<WordNode>,
    /// Rich-text decoration spans (chips, code spans): padding and
    /// fragmentation behaviour. Their boundaries appear in `items` as
    /// `BoxNode::SpanBoundary`.
    ///
    /// **Order:** ascending by `byte_range.start`, then descending by
    /// `byte_range.end` (an outer span before the spans it contains; equal
    /// ranges keep their nesting order). `check_layout_inputs` rejects any
    /// other order with `InvalidDecorationSpan`.
    pub decoration_spans: Vec<InlineDecorationSpan>,
    /// Spans the caret and deletion treat as one unit (mentions, chips).
    /// Orthogonal to `decoration_spans`: a span may be either, both or neither.
    /// Membership is looked up, never stored on atoms
    /// ([`atomic_span_containing`](Self::atomic_span_containing)).
    ///
    /// **Order:** ascending by `byte_range.start` and pairwise disjoint
    /// (`prev.end <= next.start`). `check_layout_inputs` rejects any other
    /// order, an overlap or a nesting with `InvalidAtomicSpan`.
    pub atomic_spans: Vec<AtomicEditSpan>,
}

impl ParagraphAst {
    /// The atomic span that owns `byte` (`start <= byte < end`), if any. The
    /// spans ascend and are disjoint, so this is a binary search; an in-order
    /// sweep over the atoms gets the same answer in amortised O(1) by keeping
    /// a cursor into `atomic_spans`.
    #[must_use]
    pub fn atomic_span_containing(&self, byte: ByteIndex) -> Option<&AtomicEditSpan> {
        let after = self
            .atomic_spans
            .partition_point(|span| span.byte_range.start() <= byte);
        after
            .checked_sub(1)
            .map(|i| &self.atomic_spans[i])
            .filter(|span| byte < span.byte_range.end())
    }

    /// The atoms of body run `run`: `atoms[runs[run].atom_range]`, which is
    /// exactly the slice `TextShaper::shape_run` and `reshape_window` take.
    ///
    /// Raw access for an AST that may not have passed the input gate (the
    /// gate itself uses it): empty for an out-of-range run or range. Code
    /// behind the gate uses `ValidatedInputs::run_atoms` (spec AX-3).
    #[must_use]
    pub fn run_atoms(&self, run: usize) -> &[GraphemeAtom] {
        self.runs
            .get(run)
            .and_then(|r| self.atoms.get(r.atom_range.clone()))
            .unwrap_or_default()
    }

    /// The clusters of body run `run`: `clusters[runs[run].cluster_range]`;
    /// their `atom_range`s index [`run_atoms`](Self::run_atoms). Raw access:
    /// empty for an out-of-range run or range; behind the gate use
    /// `ValidatedInputs::run_clusters`.
    #[must_use]
    pub fn run_clusters(&self, run: usize) -> &[ShapedCluster] {
        self.runs
            .get(run)
            .and_then(|r| self.clusters.get(r.cluster_range.clone()))
            .unwrap_or_default()
    }

    /// The glyphs of a body cluster: `glyphs[cluster.glyph_range]`. Raw
    /// access: empty for an out-of-range range; behind the gate use
    /// `ValidatedInputs::cluster_glyphs`.
    #[must_use]
    pub fn cluster_glyphs(&self, cluster: &ShapedCluster) -> &[RawShapedGlyph] {
        self.glyphs
            .get(cluster.glyph_range.clone())
            .unwrap_or_default()
    }

    /// The text box at `items[item]` with its slices resolved, or `None` when
    /// that item is not a [`BoxNode::Text`] or one of its ranges is out of
    /// bounds. Raw access for an AST that may not have passed the input gate;
    /// behind the gate, `ValidatedInputs::text_box` returns `None` only for an
    /// item that is not text (spec AX-3).
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
            atom_start: atom_range.start,
            atoms: self.atoms.get(atom_range.clone())?,
            clusters: self.clusters.get(cluster_range.clone())?,
            natural_width: *natural_width,
        })
    }
}
