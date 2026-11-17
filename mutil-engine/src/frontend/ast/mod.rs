//! Paragraph AST and the box-and-glue sequence (spec §2.4, §2.5).

mod nodes;

pub use nodes::{
    BoxNode, BoxOverflowPolicy, BreakAction, DiscretionaryNode, GlueKind, GlueNode, GlueTier,
    RepeatedOperator, SpanEdge, WordNode,
};

use super::{AnnotationSpan, AtomicEditSpan, ExplicitBidiSpan, InlineDecorationSpan, ShapingRun};
use crate::types::{
    AtomFlags, BidiDirection, ByteIndex, FontMetrics, GraphemeAtom, HyphenOpportunity,
    RawShapedGlyph, ShapedCluster, TextRange, WritingMode,
};
use core::iter::successors;
use core::ops::Range;
use smallvec::SmallVec;

/// A [`BoxNode::Text`] with its slices resolved. `clusters[i].atom_range` is
/// run-local: it indexes [`ParagraphAst::run_atoms`], not `atoms`.
#[derive(Clone, Copy, Debug)]
pub struct TextBoxView<'a> {
    /// Index into [`ParagraphAst::runs`].
    pub run_index: usize,
    /// First atom's start to last atom's end.
    pub byte_range: TextRange,
    /// Paragraph-level index of `atoms[0]`.
    pub atom_start: usize,
    /// The box's atoms.
    pub atoms: &'a [GraphemeAtom],
    /// The box's clusters.
    pub clusters: &'a [ShapedCluster],
    /// The box's cached width.
    pub natural_width: f32,
}

/// A stretch of one shaping run to re-shape as a unit with
/// `TextShaper::reshape_window`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReshapeWindow {
    /// Index into [`ParagraphAst::runs`].
    pub run_index: usize,
    /// Paragraph-level atom indices, covering whole text boxes.
    pub atom_range: Range<usize>,
    /// The window's bytes, on atom boundaries.
    pub byte_range: TextRange,
}

impl ReshapeWindow {
    /// From the start of `first` to the end of `last`, in one hyphen chain.
    fn spanning(first: &TextBoxView<'_>, last: &TextBoxView<'_>) -> Self {
        Self {
            run_index: first.run_index,
            atom_range: first.atom_start..last.atom_start + last.atoms.len(),
            byte_range: first.byte_range.cover(last.byte_range),
        }
    }
}

/// Frontend output: everything the middle-end needs, independent of column
/// width (spec §2.5, §1.3). Owns all its data; the text itself is passed to
/// shapers per call. Shaped text lives once in the flat `atoms`, `clusters`
/// and `glyphs` arrays, which runs, text boxes and clusters slice by range.
#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphAst {
    /// Canonical-to-physical writing mode.
    pub writing_mode: WritingMode,
    /// Explicit directional controls in source coordinates.
    pub bidi_spans: Vec<ExplicitBidiSpan>,
    /// Independent line-over and line-under annotations.
    pub annotations: Vec<AnnotationSpan>,
    /// Resolved paragraph base direction.
    pub base_direction: BidiDirection,
    /// The raw primary-font metrics from `TextShaper::resolve_primary_font_metrics`;
    /// the middle-end resolves the strut with `LineHeight::resolve_strut`
    /// (§5 Stage 4.1).
    pub strut_metrics: FontMetrics,
    /// The font size the strut metrics were requested at.
    pub strut_font_size: f32,
    /// Shaping runs in logical order.
    pub runs: Vec<ShapingRun>,
    /// Every body atom; run `i` owns `atoms[runs[i].atom_range]`.
    pub atoms: Vec<GraphemeAtom>,
    /// Every body cluster; run `i` owns `clusters[runs[i].cluster_range]`.
    pub clusters: Vec<ShapedCluster>,
    /// Every body glyph; cluster `c` owns `glyphs[c.glyph_range]`.
    pub glyphs: Vec<RawShapedGlyph>,
    /// Box-and-glue sequence in logical order.
    pub items: Vec<WordNode>,
    /// Rich-text decoration spans, ascending by start then descending by end
    /// (`InvalidDecorationSpan` otherwise).
    pub decoration_spans: Vec<InlineDecorationSpan>,
    /// Spans the caret and deletion treat as one unit, ascending and disjoint
    /// (`InvalidAtomicSpan` otherwise).
    pub atomic_spans: Vec<AtomicEditSpan>,
}

/// A `Hyphenate` discretionary and the two text boxes it joins.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HyphenSeam<'a> {
    pub(crate) opportunity: &'a HyphenOpportunity,
    pub(crate) before: TextBoxView<'a>,
    pub(crate) after: TextBoxView<'a>,
}

impl ParagraphAst {
    /// The atomic span with `start <= byte < end`, if any (binary search).
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

    /// The atoms of run `run`. Raw access, empty when out of range; behind
    /// the gate use `ValidatedInputs::run_atoms` (spec AX-3).
    #[must_use]
    pub fn run_atoms(&self, run: usize) -> &[GraphemeAtom] {
        self.runs
            .get(run)
            .and_then(|r| self.atoms.get(r.atom_range.clone()))
            .unwrap_or_default()
    }

    /// The clusters of run `run`. Raw access, empty when out of range; behind
    /// the gate use `ValidatedInputs::run_clusters`.
    #[must_use]
    pub fn run_clusters(&self, run: usize) -> &[ShapedCluster] {
        self.runs
            .get(run)
            .and_then(|r| self.clusters.get(r.cluster_range.clone()))
            .unwrap_or_default()
    }

    /// The glyphs of `cluster`. Raw access, empty when out of range; behind
    /// the gate use `ValidatedInputs::cluster_glyphs`.
    #[must_use]
    pub fn cluster_glyphs(&self, cluster: &ShapedCluster) -> &[RawShapedGlyph] {
        self.glyphs
            .get(cluster.glyph_range.clone())
            .unwrap_or_default()
    }

    /// The text box at `items[item]`, or `None` when it is not text or a
    /// range is out of bounds. Raw access; behind the gate use
    /// `ValidatedInputs::text_box` (spec AX-3).
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

    /// Checks the [`ShapedCluster`] invariants for the text box at
    /// `items[item]`; any other node is self-contained, and an `item` past
    /// the end or a box whose ranges do not resolve inside its run is not.
    #[must_use]
    pub fn is_self_contained(&self, item: usize) -> bool {
        let Some(node) = self.items.get(item) else {
            return false;
        };
        let WordNode::Box(BoxNode::Text {
            run_index,
            atom_range,
            cluster_range,
            ..
        }) = node
        else {
            return true;
        };
        let within = |inner: &Range<usize>, outer: &Range<usize>| {
            outer.start <= inner.start && inner.end <= outer.end
        };
        self.runs.get(*run_index).is_some_and(|run| {
            within(atom_range, &run.atom_range)
                && within(cluster_range, &run.cluster_range)
                && self.text_box(item).is_some_and(|view| {
                    let base = view.atom_start - run.atom_range.start;
                    text_is_self_contained(view.byte_range, view.atoms, view.clusters, base)
                })
        })
    }

    /// The seam at `items[index]`: a `Hyphenate` discretionary whose
    /// neighbours are text boxes of one run, the three meeting end to end.
    /// Seams link boxes into **hyphen chains**.
    pub(crate) fn hyphen_seam(&self, index: usize) -> Option<HyphenSeam<'_>> {
        let WordNode::Discretionary(DiscretionaryNode {
            action: BreakAction::Hyphenate(opportunity),
            byte_range,
            ..
        }) = self.items.get(index)?
        else {
            return None;
        };
        let before = self.text_box(index.checked_sub(1)?)?;
        let after = self.text_box(index + 1)?;
        (before.run_index == after.run_index
            && before.byte_range.end() == byte_range.start()
            && byte_range.end() == after.byte_range.start())
        .then_some(HyphenSeam {
            opportunity,
            before,
            after,
        })
    }

    /// The window to re-shape when the `Hyphenate` discretionary at
    /// `items[index]` is taken (spec §5 Stage 1.6.5): the boxes on each side,
    /// extended along the hyphen chain as far as the cuts reach. `None` when
    /// `items[index]` is not a seam or a cut leaves its chain.
    #[must_use]
    pub fn break_window(&self, index: usize) -> Option<ReshapeWindow> {
        let HyphenSeam {
            opportunity,
            before,
            after,
        } = self.hyphen_seam(index)?;
        let offset = before.byte_range.end();
        // An edit without a cut still touches the neighbouring box.
        let first = offset.checked_sub((opportunity.prefix_cut_bytes as usize).max(1))?;
        let last = offset.checked_add((opportunity.suffix_cut_bytes as usize).max(1))? - 1;
        // `hyphen_seam(item - 1)` succeeds only when `item - 1 >= 1`.
        let (_, start) = successors(Some((index - 1, before)), |&(item, _)| {
            let seam = self.hyphen_seam(item.checked_sub(1)?)?;
            Some((item - 2, seam.before))
        })
        .find(|(_, view)| view.byte_range.start() <= first)?;
        let (_, end) = successors(Some((index + 1, after)), |&(item, _)| {
            Some((item + 2, self.hyphen_seam(item + 1)?.after))
        })
        .find(|(_, view)| last < view.byte_range.end())?;
        Some(ReshapeWindow::spanning(&start, &end))
    }

    /// The windows to re-shape whole because the `Hyphenate` discretionaries
    /// at `untaken` were not taken (spec §5 Stage 1.6.5, rejoin rule; §2.3
    /// note 3). Each seam whose following cluster is not
    /// `safe_to_concat_before` contributes its two boxes; overlapping windows
    /// merge, so the result is in text order and pairwise disjoint.
    #[must_use]
    pub fn rejoin_windows(
        &self,
        untaken: impl IntoIterator<Item = usize>,
    ) -> SmallVec<[ReshapeWindow; 4]> {
        let mut windows: SmallVec<[ReshapeWindow; 4]> = untaken
            .into_iter()
            .filter_map(|index| {
                let HyphenSeam { before, after, .. } = self.hyphen_seam(index)?;
                let safe = after.clusters.first()?.safe_to_concat_before;
                (!safe).then(|| ReshapeWindow::spanning(&before, &after))
            })
            .collect();
        windows.sort_by_key(|window| window.byte_range.start());
        windows.dedup_by(|next, last| {
            let overlaps = next.byte_range.start() < last.byte_range.end();
            if overlaps {
                last.byte_range = last.byte_range.cover(next.byte_range);
                last.atom_range.end = last.atom_range.end.max(next.atom_range.end);
            }
            overlaps
        });
        windows
    }
}

/// Checks the [`ShapedCluster`] invariants for one text payload (a text box
/// or an annotation's text): non-empty contiguous atoms tiling `byte_range`,
/// clusters tiling the atoms in order with matching bytes and aggregated
/// flags. `base` is the run-local index of `atoms[0]`.
pub(crate) fn text_is_self_contained(
    byte_range: TextRange,
    atoms: &[GraphemeAtom],
    clusters: &[ShapedCluster],
    base: usize,
) -> bool {
    let tiled = clusters.iter().try_fold(base, |next, cluster| {
        let range = &cluster.atom_range;
        let own = atoms.get(range.start.checked_sub(base)?..range.end.checked_sub(base)?)?;
        (range.start == next
            && spans_atoms(cluster.byte_range, own)
            && cluster.flags == AtomFlags::for_cluster(own))
        .then_some(range.end)
    });
    spans_atoms(byte_range, atoms)
        && atoms
            .windows(2)
            .all(|w| w[0].byte_range.end() == w[1].byte_range.start())
        && atoms.iter().all(|a| !a.byte_range.is_empty())
        && tiled == Some(base + atoms.len())
}

/// `atoms` is non-empty and `byte_range` runs from its first start to its
/// last end.
fn spans_atoms(byte_range: TextRange, atoms: &[GraphemeAtom]) -> bool {
    atoms
        .first()
        .zip(atoms.last())
        .is_some_and(|(first, last)| {
            byte_range.start == first.byte_range.start && byte_range.end == last.byte_range.end
        })
}
