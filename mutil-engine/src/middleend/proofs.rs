//! The proof kernel (ADR-0004 rule 5): views that carry a fact a gate proved,
//! so code behind the gate reads it without indexing or falling back.
//!
//! A child of `validated`, so only `ValidatedLayout` can build the views
//! that carry a G4 fact. Every view wraps a borrow and costs nothing at run time. Each slice access
//! names the fact it relies on in a `PROOF(...)` comment, which
//! `scripts/check_pipeline.py` requires.

use super::ValidatedInputs;
use crate::frontend::bidi::visual_order_into;
use crate::frontend::{
    AtomicEditSpan, DiscretionaryNode, InlineDecorationSpan, TextBox, TextBoxView, WordNode,
};
use crate::middleend::output::{CaretStop, HangSide};
use crate::middleend::tolerance::approx_eq;
use crate::types::{BidiLevel, GraphemeAtom, RawShapedGlyph, ShapedCluster, ShapingRun, TextRange};
use core::ops::Range;

/// A slice that is not empty.
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct NonEmptySlice<'a, T>(&'a [T]);

impl<T> Clone for NonEmptySlice<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for NonEmptySlice<'_, T> {}

impl<'a, T> NonEmptySlice<'a, T> {
    /// `slice`, unless it is empty.
    pub(crate) fn try_from_slice(slice: &'a [T]) -> Option<Self> {
        (!slice.is_empty()).then_some(Self(slice))
    }

    pub(crate) fn first(&self) -> &'a T {
        // PROOF(NonEmptySlice::try_from_slice): the slice is not empty.
        &self.0[0]
    }

    pub(crate) fn last(&self) -> &'a T {
        // PROOF(NonEmptySlice::try_from_slice): the slice is not empty.
        &self.0[self.last_index()]
    }

    pub(crate) fn last_index(&self) -> usize {
        self.0.len() - 1
    }

    pub(crate) fn iter(&self) -> core::slice::Iter<'a, T> {
        self.0.iter()
    }

    pub(crate) fn get(&self, index: usize) -> Option<&'a T> {
        self.0.get(index)
    }

    /// The element at `index`, or the last one past the end.
    pub(crate) fn get_clamped(&self, index: usize) -> &'a T {
        // PROOF(NonEmptySlice::try_from_slice): `last_index` is an index.
        &self.0[index.min(self.last_index())]
    }

    pub(crate) fn partition_point(&self, pred: impl FnMut(&T) -> bool) -> usize {
        self.0.partition_point(pred)
    }

    /// The index of the first element for which `pred` fails, else the last.
    pub(crate) fn partition_point_clamped(&self, pred: impl FnMut(&T) -> bool) -> usize {
        self.partition_point(pred).min(self.last_index())
    }

    /// The element with the least key, the first of ties, with its index.
    pub(crate) fn min_by_key_indexed<K: Ord>(
        &self,
        mut key: impl FnMut(&T) -> K,
    ) -> (usize, &'a T) {
        let mut best = (0, self.first());
        for item in self.iter().enumerate().skip(1) {
            if key(item.1) < key(best.1) {
                best = item;
            }
        }
        best
    }
}

/// A string that is not empty.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub(crate) struct NonEmptyStr<'a>(&'a str);

impl<'a> NonEmptyStr<'a> {
    /// `s`, unless it is empty.
    pub(crate) fn try_new(s: &'a str) -> Option<Self> {
        (!s.is_empty()).then_some(Self(s))
    }

    pub(crate) fn first_char(&self) -> char {
        let mut chars = self.0.chars();
        // PROOF(NonEmptyStr::try_new): a non-empty `str` has a first char.
        chars.next().expect("non-empty str")
    }

    pub(crate) fn last_char(&self) -> char {
        let mut chars = self.0.chars();
        // PROOF(NonEmptyStr::try_new): a non-empty `str` has a last char.
        chars.next_back().expect("non-empty str")
    }
}

/// The index of a line of a [`ValidatedLayout`](super::ValidatedLayout).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineIdx(pub(super) usize);

impl LineIdx {
    pub(crate) fn get(&self) -> usize {
        self.0
    }
}

/// The index of a stop in its line's [`SortedCaretStops`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct StopIdx(usize);

/// The caret stops of a line that passed the output gate, when it has any:
/// G4 (`InvalidCaretStop`) proved them finite and in non-decreasing `x`, and
/// rejected two stops of one offset at one position (`DuplicateCaretStop`).
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub(crate) struct SortedCaretStops<'a>(NonEmptySlice<'a, CaretStop>);

impl<'a> SortedCaretStops<'a> {
    /// The stops of a validated line; `None` for a line without stops
    /// (CRT-1). Only `ValidatedLayout` calls it, on lines G4 accepted.
    pub(super) fn try_from_validated(stops: &'a [CaretStop]) -> Option<Self> {
        NonEmptySlice::try_from_slice(stops).map(Self)
    }

    pub(crate) fn first(&self) -> &'a CaretStop {
        self.0.first()
    }

    pub(crate) fn last(&self) -> &'a CaretStop {
        self.0.last()
    }

    pub(crate) fn iter(&self) -> core::slice::Iter<'a, CaretStop> {
        self.0.iter()
    }

    /// The stops in visual order, with their indices.
    pub(crate) fn indexed(&self) -> impl Iterator<Item = (StopIdx, &'a CaretStop)> + use<'a> {
        self.iter().enumerate().map(|(i, s)| (StopIdx(i), s))
    }

    /// The stop with the least offset, the first of ties.
    pub(crate) fn min_by_offset(&self) -> (StopIdx, &'a CaretStop) {
        let (index, stop) = self.0.min_by_key_indexed(|s| s.offset);
        (StopIdx(index), stop)
    }

    /// The extreme stop towards `side` (spec §6.3 rule 3), as
    /// [`PositionedLine::extreme_caret_stop`](crate::middleend::PositionedLine::extreme_caret_stop)
    /// finds it: sorted stops put the first of the leftmost ties first and
    /// the last of the rightmost ties last.
    pub(crate) fn edge(&self, side: HangSide) -> (StopIdx, &'a CaretStop) {
        match side {
            HangSide::Left => (StopIdx(0), self.first()),
            HangSide::Right => (StopIdx(self.0.last_index()), self.last()),
        }
    }

    /// The stop at `index`, clamped to the last one.
    pub(crate) fn stop(&self, index: StopIdx) -> &'a CaretStop {
        self.0.get_clamped(index.0)
    }

    /// The nearest stop at another position than the one at `from`, one
    /// visual step right or left: the first at it moving right, the last
    /// moving left (spec INT-13).
    pub(crate) fn step_visual(&self, from: StopIdx, rightwards: bool) -> Option<&'a CaretStop> {
        let x = self.stop(from).x;
        let elsewhere = |s: &&CaretStop| !approx_eq(s.x, x);
        if rightwards {
            self.iter().skip(from.0 + 1).find(elsewhere)
        } else {
            self.iter().take(from.0).rev().find(elsewhere)
        }
    }

    /// The stop nearest `x` for `first.x < x < last.x` (spec INT-8, INT-20):
    /// of the stops either side of `x`, the nearer, the right one at the
    /// midpoint. The right one is the first of its coincident group and the
    /// left one the last of its own. Outside the bounds it is still total.
    pub(crate) fn nearest_in_bounds(&self, x: f32) -> &'a CaretStop {
        let right = self.0.partition_point(|s| s.x <= x);
        let r = self.0.get_clamped(right);
        let l = self.0.get_clamped(right.saturating_sub(1));
        if r.x - x <= x - l.x { r } else { l }
    }
}

/// Validated index into `ParagraphAst::runs`: G1 (`check_storage`) proves
/// `runs.len() <= u32::MAX` and every run index is in bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct RunId(pub(super) u32);

impl RunId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Validated index into `ParagraphAst::clusters`: G1 (`check_storage`) proves
/// `clusters.len() <= u32::MAX` and every cluster index is in bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ClusterId(pub(super) u32);

impl ClusterId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Validated index into `ParagraphAst::decoration_spans`: G1
/// (`check_decoration_spans`) proves `decoration_spans.len() <= u32::MAX` and
/// every span index is in bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SpanId(pub(super) u32);

impl SpanId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Index into a [`SentinelTokens`] sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TokenIdx(usize);

impl TokenIdx {
    pub(crate) const START: Self = Self(0);

    pub(crate) fn next(self) -> Self {
        Self(self.0 + 1)
    }

    pub(crate) fn prev(self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

/// A non-empty token sequence ending with the paragraph-end sentinel break
/// (proved by G1 `check_paragraph_end` and enforced at construction).
pub(crate) struct SentinelTokens<T>(Vec<T>);

impl<T> SentinelTokens<T> {
    /// Appends the paragraph-end `sentinel` to `body`, proving the sequence is
    /// non-empty and ends with that sentinel.
    pub(crate) fn from_body_and_sentinel(mut body: Vec<T>, sentinel: T) -> Self {
        body.push(sentinel);
        Self(body)
    }

    pub(crate) fn is_exhausted(&self, idx: TokenIdx) -> bool {
        idx.0 >= self.0.len()
    }

    /// The token at `idx`, or the trailing sentinel if `idx` reaches the end.
    pub(crate) fn at_or_sentinel(&self, idx: TokenIdx) -> &T {
        let last = self.0.len() - 1;
        // PROOF(SentinelTokens::from_body_and_sentinel): `self.0` is non-empty.
        &self.0[idx.0.min(last)]
    }

    /// The token at `idx`, if in bounds.
    pub(crate) fn get(&self, idx: TokenIdx) -> Option<&T> {
        self.0.get(idx.0)
    }

    /// Tokens from `start` onwards, each paired with its [`TokenIdx`].
    pub(crate) fn iter_from(&self, start: TokenIdx) -> impl Iterator<Item = (TokenIdx, &T)> {
        let start = start.0.min(self.0.len());
        // PROOF(SentinelTokens::iter_from): `start <= self.0.len()`.
        self.0[start..]
            .iter()
            .enumerate()
            .map(move |(off, t)| (TokenIdx(start + off), t))
    }

    /// Tokens in `[start, end)`, each paired with its [`TokenIdx`].
    pub(crate) fn iter_range(
        &self,
        start: TokenIdx,
        end: TokenIdx,
    ) -> impl Iterator<Item = (TokenIdx, &T)> {
        let end = end.0.min(self.0.len());
        let start = start.0.min(end);
        // PROOF(SentinelTokens::iter_range): `start <= end <= self.0.len()`.
        self.0[start..end]
            .iter()
            .enumerate()
            .map(move |(off, t)| (TokenIdx(start + off), t))
    }

    /// The slice from `start` to the end.
    pub(crate) fn slice_from(&self, start: TokenIdx) -> &[T] {
        let start = start.0.min(self.0.len());
        // PROOF(SentinelTokens::slice_from): `start <= self.0.len()`.
        &self.0[start..]
    }

    /// The slice `[start, end)`.
    pub(crate) fn slice(&self, start: TokenIdx, end: TokenIdx) -> &[T] {
        let end = end.0.min(self.0.len());
        let start = start.0.min(end);
        // PROOF(SentinelTokens::slice): `start <= end <= self.0.len()`.
        &self.0[start..end]
    }
}

/// An index `< order.len()` into a line's parallel body columns, produced only
/// by [`VisualOrder`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct BodyIdx(usize);

/// The visual order of a line's body pieces (UAX #9 L2): a permutation of
/// `0..self.len()`, rebuilt in place from the body's resolved bidi levels.
#[derive(Default)]
pub(crate) struct VisualOrder(Vec<usize>);

impl VisualOrder {
    /// Rebuilds the permutation from `levels` (`0..levels.len()`).
    pub(crate) fn rebuild(&mut self, levels: &[BidiLevel]) {
        visual_order_into(levels, &mut self.0);
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// Body piece indices in visual order.
    pub(crate) fn indices(&self) -> impl Iterator<Item = BodyIdx> + '_ {
        self.0.iter().copied().map(BodyIdx)
    }

    /// Proves `slice` covers every index in this visual order.
    pub(crate) fn table<'s, T>(&self, slice: &'s [T]) -> PieceTable<'s, T> {
        assert!(
            slice.len() >= self.0.len(),
            "VisualOrder column shorter than body"
        );
        PieceTable(slice)
    }

    /// Proves mutable `slice` covers every index in this visual order.
    pub(crate) fn table_mut<'s, T>(&self, slice: &'s mut [T]) -> PieceTableMut<'s, T> {
        assert!(
            slice.len() >= self.0.len(),
            "VisualOrder column shorter than body"
        );
        PieceTableMut(slice)
    }

    /// Borrows the elements of `slice` in visual order.
    pub(crate) fn pick<'s, T>(&'s self, slice: &'s [T]) -> impl Iterator<Item = &'s T> + 's {
        let table = self.table(slice);
        self.indices().map(move |k| table.get(k))
    }
}

/// Read-only view over a line column proved at least as long as its
/// [`VisualOrder`].
pub(crate) struct PieceTable<'a, T>(&'a [T]);

impl<T> Clone for PieceTable<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for PieceTable<'_, T> {}

impl<'a, T> PieceTable<'a, T> {
    pub(crate) fn get(self, idx: BodyIdx) -> &'a T {
        // PROOF(VisualOrder::table): `idx` came from `VisualOrder::rebuild`
        // (`idx.0 < order.len() <= self.0.len()`).
        &self.0[idx.0]
    }
}

/// Mutable view over a line column proved at least as long as its
/// [`VisualOrder`].
pub(crate) struct PieceTableMut<'a, T>(&'a mut [T]);

impl<T> PieceTableMut<'_, T> {
    pub(crate) fn set(&mut self, idx: BodyIdx, value: T) {
        // PROOF(VisualOrder::table_mut): `idx` came from `VisualOrder::rebuild`
        // (`idx.0 < order.len() <= self.0.len()`).
        self.0[idx.0] = value;
    }
}

impl<'a> ValidatedInputs<'a> {
    /// The items before the first break opportunity, and that break (at latest
    /// the paragraph-end sentinel).
    pub(crate) fn before_first_break(&self) -> (&'a [WordNode], &'a DiscretionaryNode) {
        let at = self.summary.first_break_item;
        // PROOF(G1::check_paragraph_end): `first_break_item < ast.items.len()`
        // and names a `WordNode::Discretionary`.
        let WordNode::Discretionary(brk) = &self.ast.items[at] else {
            // PROOF(G1::check_paragraph_end): `first_break_item` is a break.
            panic!("G1 proved first_break_item is a Discretionary");
        };
        // PROOF(G1::check_paragraph_end): `at < ast.items.len()`.
        (&self.ast.items[..at], brk)
    }

    /// Every item before the paragraph-end sentinel, and the sentinel itself.
    pub(crate) fn split_paragraph_end(&self) -> (&'a [WordNode], &'a DiscretionaryNode) {
        let split = self.ast.items.split_last();
        // PROOF(G1::check_paragraph_end): `ast.items` ends with the sentinel.
        let (last, body) = split.expect("G1 proved non-empty items");
        let WordNode::Discretionary(sentinel) = last else {
            // PROOF(G1::check_paragraph_end): the last item is `Discretionary`.
            panic!("G1 proved last item is the paragraph-end Discretionary");
        };
        (body, sentinel)
    }

    /// The validated shaping run for `id` (spec AST-4).
    pub(crate) fn run(&self, id: RunId) -> &'a ShapingRun {
        // PROOF(G1::check_storage): `RunId` is minted in `0..ast.runs.len()`.
        &self.ast.runs[id.index()]
    }

    /// The validated shaped cluster for `id` (spec AST-4).
    pub(crate) fn cluster(&self, id: ClusterId) -> &'a ShapedCluster {
        // PROOF(G1::check_storage): `ClusterId` is minted in `0..ast.clusters.len()`.
        &self.ast.clusters[id.index()]
    }

    /// The validated decoration span for `id` (spec AST-7).
    pub(crate) fn decoration_span(&self, id: SpanId) -> &'a InlineDecorationSpan {
        // PROOF(G1::check_decoration_spans): `SpanId` is minted in `0..ast.decoration_spans.len()`.
        &self.ast.decoration_spans[id.index()]
    }

    /// The atoms owned by `cluster` within `run` (spec AST-4).
    pub(crate) fn cluster_atoms(&self, run: RunId, cluster: ClusterId) -> &'a [GraphemeAtom] {
        let atoms = self.run_atoms_id(run);
        // PROOF(G1::check_each_item): `cluster.atom_range` indexes its run's atoms.
        &atoms[self.cluster(cluster).atom_range.clone()]
    }

    /// The glyphs owned by `cluster` (spec AST-4).
    pub(crate) fn cluster_glyphs_id(&self, cluster: ClusterId) -> &'a [RawShapedGlyph] {
        self.cluster_glyphs(self.cluster(cluster))
    }

    /// The atoms of `run` (spec AST-4).
    pub(crate) fn run_atoms_id(&self, run: RunId) -> &'a [GraphemeAtom] {
        self.run_atoms(run.index())
    }

    /// Resolves `text`, a box of this paragraph, to its validated [`RunId`],
    /// [`ShapingRun`], and [`TextBoxView`] (spec AST-4).
    pub(crate) fn resolve_text(
        &self,
        text: &'a TextBox,
    ) -> (RunId, &'a ShapingRun, TextBoxView<'a>) {
        // PROOF(G1::check_storage): `text.run_index < ast.runs.len() <= u32::MAX`.
        let run_id = RunId(text.run_index as u32);
        (run_id, self.run(run_id), self.ast.resolve(text))
    }

    /// Yields each validated [`ClusterId`] and cluster of `text` in order.
    pub(crate) fn text_clusters(
        &self,
        text: &'a TextBox,
    ) -> impl Iterator<Item = (ClusterId, &'a ShapedCluster)> + 'a {
        let view = self.ast.resolve(text);
        // PROOF(G1::check_storage): `text.cluster_range.end <= ast.clusters.len() <= u32::MAX`.
        let start = text.cluster_range.start as u32;
        view.clusters
            .iter()
            .enumerate()
            .map(move |(offset, info)| (ClusterId(start + offset as u32), info))
    }

    /// Cluster ids of `run` strictly after `after`, in ascending order.
    pub(crate) fn run_clusters_after(
        &self,
        run: RunId,
        after: ClusterId,
    ) -> impl Iterator<Item = ClusterId> {
        let end = self.run(run).cluster_range.end;
        // PROOF(G1::check_storage): `end <= ast.clusters.len() <= u32::MAX`.
        (after.index() + 1..end).map(|c| ClusterId(c as u32))
    }

    /// Cluster ids of `run` in `[max(run.start, floor), after)` in reverse order.
    pub(crate) fn run_clusters_before(
        &self,
        run: RunId,
        floor: ClusterId,
        after: ClusterId,
    ) -> impl Iterator<Item = ClusterId> {
        let start = self.run(run).cluster_range.start.max(floor.index());
        // PROOF(G1::check_storage): `after.index() <= ast.clusters.len() <= u32::MAX`.
        (start..after.index()).rev().map(|c| ClusterId(c as u32))
    }

    /// The validated [`SpanId`] of a `SpanBoundary`'s `span_id` (spec AST-8).
    pub(crate) fn boundary_span_id(&self, span_id: u64) -> SpanId {
        let found = self.summary.decoration_ids.index_of(span_id);
        // PROOF(G1::check_each_item): every boundary's `span_id` is in `decoration_ids`,
        // whose indices are `< decoration_spans.len() <= u32::MAX`.
        let index = found.expect("G1 proved boundary span_id exists");
        SpanId(index as u32)
    }

    /// Validated [`SpanId`]s for indices in `range`, bounded to `decoration_spans`.
    pub(crate) fn span_ids_in(&self, range: Range<usize>) -> impl Iterator<Item = SpanId> {
        let end = range.end.min(self.ast.decoration_spans.len());
        let start = range.start.min(end);
        // PROOF(G1::check_decoration_spans): `end <= decoration_spans.len() <= u32::MAX`.
        (start..end).map(|i| SpanId(i as u32))
    }

    /// Atomic spans meeting `range`: G1 proves `atomic_spans` strictly disjoint
    /// and ascending (spec AST-11).
    pub(crate) fn atomic_spans_meeting(&self, range: TextRange) -> &'a [AtomicEditSpan] {
        let spans = &self.ast.atomic_spans;
        let lo = spans.partition_point(|s| s.byte_range.end() <= range.start());
        let hi = spans.partition_point(|s| s.byte_range.start() < range.end());
        // PROOF(G1::check_atomic_spans): `lo <= hi.max(lo) <= spans.len()`.
        &spans[lo..hi.max(lo)]
    }
}

#[cfg(test)]
impl VisualOrder {
    pub(crate) fn as_raw(&self) -> &[usize] {
        &self.0
    }
}
