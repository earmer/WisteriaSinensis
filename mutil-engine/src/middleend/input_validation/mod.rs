//! Input validation: the invariants of a [`ParagraphAst`] and of the layout
//! inputs around it (spec §2.4, §2.5).
//!
//! The AST is untrusted: nothing downstream repairs an invalid one, so
//! [`check_layout_inputs`] rejects it here, in time linear in its size.

mod items;
mod spans;
mod storage;
#[cfg(test)]
mod tests;

use super::{LayoutConstraints, LayoutOptions, LayoutPolicySnapshot};
use crate::frontend::{ParagraphAst, is_paragraph_separator};
use crate::types::TextRange;

/// An input that [`check_layout_inputs`] rejects.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutInputError {
    /// A run's orientation does not suit the writing mode or its level parity,
    /// or a tate-chu-yoko run is not one cluster in one text box spanning the
    /// whole run.
    OrientationMismatch,
    /// The AST's raw strut metrics are not valid `FontMetrics` although the
    /// strut's own ascent, descent and line gap pass (`InvalidStrut` covers
    /// those); for example an invalid vertical metric or baseline offset.
    InvalidFontMetrics,
    /// A `TextShaper` answered metrics that are not valid `FontMetrics` at
    /// layout time, or that resolve under the line height to an invalid line
    /// box (the shaper gate, G2). The AST is not at fault.
    InvalidShaperMetrics,
    /// Nonfinite or forbidden explicit spacing.
    InvalidSpacing,
    /// Invalid explicit directional span.
    InvalidBidiSpan {
        /// Index into `ParagraphAst::bidi_spans`.
        span: usize,
    },
    /// Invalid annotation base or locally shaped payload.
    InvalidAnnotationSpan {
        /// Index into `ParagraphAst::annotations`.
        annotation: usize,
    },
    /// Invalid clamp or other layout policy.
    InvalidLayoutOptions,
    /// A provider answer outside the wrap envelope or out of order, or an
    /// obstructed answer without progress ahead.
    InvalidLineGeometry,
    /// More than the allowed number of obstructed provider answers in a row
    /// for one line (spec GEO-4).
    ClearanceExhausted,
    /// One line still outgrew its envelope after the allowed refits
    /// (spec GEO-6).
    RefitExhausted,
    /// Expanding a block envelope introduced available inline space.
    NonMonotonicGeometry,
    /// Invalid width or indentation; positive-infinite width is allowed. Without a geometry provider each indent must lie in
    /// `[0, wrap_width)`.
    InvalidConstraints,
    /// `text` contains a paragraph separator; split it with
    /// [`frontend::split_paragraphs`](crate::frontend::split_paragraphs) first.
    ParagraphSeparatorInText,
    /// The raw strut metrics, the strut font size or a `LineHeight::Fixed`
    /// multiplier is not finite (or the size is negative).
    InvalidStrut,
    /// A run's `byte_range` exceeds `text` or splits a UTF-8 sequence.
    RunOutsideText {
        /// Index into `ParagraphAst::runs`.
        run: usize,
    },
    /// A run's font size is not positive and finite, its script is unresolved
    /// (`Zyyy` / `Zinh`), or its `byte_range` starts before the previous body run's `byte_range` ends (the body runs
    /// must be non-decreasing and non-overlapping in run order).
    InvalidShapingRun {
        /// Index into `ParagraphAst::runs`.
        run: usize,
    },
    /// A run's `atom_range` / `cluster_range` do not tile `ParagraphAst::atoms` /
    /// `ParagraphAst::clusters` (a gap, an overlap or an out-of-bounds range),
    /// or the run's atoms or clusters are not fully covered by its text boxes.
    InvalidRunStorage {
        /// Index into `ParagraphAst::runs`.
        run: usize,
    },
    /// The clusters' `glyph_range`s do not tile `ParagraphAst::glyphs` (a gap,
    /// an overlap or an out-of-bounds range).
    InvalidGlyphStorage {
        /// Index into `ParagraphAst::clusters` of the first cluster whose
        /// range is out of place, or of the last cluster when glyphs are left
        /// over after it.
        cluster: usize,
    },
    /// An item's `run_index` is not a valid index into `ParagraphAst::runs`.
    RunIndexOutOfRange {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An item's bytes exceed `text` or split a UTF-8 sequence.
    ItemOutsideText {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// The items leave source text unowned: the non-empty item ranges must
    /// concatenate to exactly `0..text.len()`.
    UncoveredSource {
        /// The first byte of `text` that no item owns.
        byte: usize,
    },
    /// An atom of a text box has a script different from its run's.
    ScriptMismatch {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A text box (or one of its atoms) is not contained in the byte range of
    /// the shaping run it names.
    ItemOutsideRun {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A text box's cached `natural_width` is not finite, differs from the sum
    /// of its cluster widths, or one of its clusters has a non-finite width,
    /// font scale or raw glyph measurement.
    InvalidTextWidth {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A `BoxNode::Text` violates `ParagraphAst::is_self_contained` (which
    /// includes owning at least one atom), has a cluster whose break-safety
    /// flags are inconsistent, or its `atom_range` / `cluster_range` lies
    /// outside its run's ranges or out of order among the run's boxes.
    ItemNotSelfContained {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An inline object has a non-finite or negative width, ascent or descent,
    /// an invalid baseline alignment, or owns no source bytes.
    InvalidInlineBox {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A tab owns no source bytes (its `byte_range` is empty). A tab's geometric
    /// width is resolved during layout and is not constrained here.
    InvalidTab {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A glue node has a non-finite or negative width, stretch or shrink; is a `CjkPunctCompression` node with a
    /// natural width or a stretch; does not match its source (a `WordSpace`
    /// is collapsible and owns a non-empty run of word-space characters,
    /// which excludes the no-break spaces; a synthetic kind owns no bytes).
    InvalidGlue {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A discretionary node is malformed:
    /// * a non-finite width, a forbidden penalty (`>= FORBIDDEN_BREAK_PENALTY`;
    ///   a forbidden break is an omitted node), or a `HardBreak` that disagrees
    ///   with a mandatory penalty;
    /// * a `byte_range` that breaks the ownership rule (soft: empty; real hard
    ///   break: exactly a line terminator; sentinel: `text.len()..text.len()`,
    ///   the last discretionary, at most once);
    /// * a repeated or incomplete `cloned_spans`;
    /// * break widths that do not match the cloned padding: `Direct` and
    ///   `HardBreak` must equal the clone sums, `MathOperator` must equal them
    ///   plus the repeated operator's advance on the `post` side (and that
    ///   advance must be finite and non-negative), and `CjkPunctAdjust` must
    ///   carry a real net delta whose neighbouring cluster stays non-negative.
    InvalidDiscretionary {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A hyphenation discretionary is not `flagged`, does not sit between two
    /// text boxes of the same `run_index` that meet at its offset, or has a
    /// cut that leaves its same-run hyphen chain or ends off an atom boundary.
    InvalidHyphenOpportunity {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A span boundary is not an empty range, has a non-finite or negative
    /// width, not at its span's `byte_range.start` (`Leading`) or `.end`
    /// (`Trailing`), a `bidi_level` that differs from the level of the first (for
    /// a `Leading` edge) or last (`Trailing`) atom inside its span, or an order
    /// that breaks LIFO nesting (a `Trailing` edge closing a span that is not the
    /// innermost open one, a repeated `Leading`, or, for two spans with equal
    /// ranges, the higher-index span not nested inside the lower-index one).
    InvalidSpanBoundary {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An item names a span (`SpanBoundary`, or an entry of
    /// `DiscretionaryNode::cloned_spans`, which must also be
    /// `BoxDecorationBreak::Clone` and contain the break strictly inside) that
    /// `decoration_spans` lacks.
    UnknownSpan {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A decoration span has an invalid padding or range, does not fit the text,
    /// repeats a `span_id`, is out of order (`decoration_spans` must ascend by
    /// start and, for equal starts, descend by end), partially overlaps another
    /// decoration span (nesting must be proper containment; equal ranges are
    /// allowed, the lower index being the outer span) or nests deeper than
    /// `MAX_DECORATION_NESTING_DEPTH`.
    InvalidDecorationSpan {
        /// Index into `ParagraphAst::decoration_spans`.
        span: usize,
    },
    /// `ParagraphAst::items` does not end with the paragraph-end sentinel, the
    /// `HardBreak` owning the empty range at the end of the text (an empty
    /// paragraph included).
    MissingParagraphEnd,
    /// A decoration span lacks its `Leading` boundary at `byte_range.start` or
    /// its `Trailing` boundary at `byte_range.end` in `items` (both are
    /// mandatory, even with zero padding).
    MissingSpanBoundary {
        /// Index into `ParagraphAst::decoration_spans`.
        span: usize,
    },
    /// An item starts before the previous item ends: `items` must progress
    /// monotonically through the text (`prev.end <= next.start`).
    ItemsOutOfOrder {
        /// Index into `ParagraphAst::items` of the later item.
        item: usize,
    },
    /// An item is on the wrong side of a decoration span's boundaries: an item
    /// inside `byte_range` is not strictly between the `Leading` and `Trailing`
    /// nodes, an item outside it is between them, a break opportunity at the
    /// span's start is not before the `Leading` node (or one at its end not
    /// after the `Trailing` node), or an item straddles a span edge.
    SpanEnclosure {
        /// Index into `ParagraphAst::decoration_spans`.
        span: usize,
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An atomic span is empty, does not fit the text, repeats a `span_id`,
    /// is out of order (`atomic_spans` must ascend by start), overlaps or nests
    /// with another atomic span, or has an edge that is not
    /// the start or end of a text atom or of an inline object (an edge inside
    /// glue, a tab or a line terminator, or on an unowned offset, is rejected),
    /// or wraps no atom or object at all.
    InvalidAtomicSpan {
        /// Index into `ParagraphAst::atomic_spans`.
        span: usize,
    },
    /// A text box's run, an inline object, a glue or a span boundary has a
    /// bidi level below the paragraph's base level (spec AST-13).
    InvalidBidiLevel {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
}

/// `true` when `range` is well formed, inside `text` and on character
/// boundaries.
pub(super) fn fits(text: &str, range: TextRange) -> bool {
    range.slice(text).is_some()
}

/// Whether `ranges` are non-overlapping and in byte order, which the binary
/// searches below require. Validation computes this before searching: an
/// unordered sequence always belongs to an AST that a later item check
/// (`ItemOutsideText`, `ItemsOutOfOrder`, `ItemNotSelfContained`) rejects, so
/// a search over it is skipped rather than allowed to report a misleading
/// error first.
pub(super) fn in_byte_order(ranges: impl Iterator<Item = TextRange>) -> bool {
    ranges.flat_map(|r| [r.start(), r.end()]).is_sorted()
}

/// Whether `items` (byte-ordered by `range`) has one starting or ending at
/// `offset`.
pub(super) fn is_range_edge<T>(
    items: &[T],
    range: impl Fn(&T) -> TextRange,
    offset: usize,
) -> bool {
    let at = items.partition_point(|item| range(item).end() < offset);
    items
        .get(at)
        .map(range)
        .is_some_and(|r| r.start() == offset || r.end() == offset)
}

/// Checks the inputs of [`layout_paragraph`](super::layout_paragraph).
///
/// The checks, in order (each failure has its own [`LayoutInputError`] variant):
/// 1. the policy: constraints, clamp and strut
///    ([`LayoutPolicySnapshot::resolve`]), and the single-paragraph rule;
/// 2. storage, writing modes, bidi spans, annotations and runs;
/// 3. the decoration and atomic span streams;
/// 4. the items: inside the text and in order; then each on its own (text
///    boxes, inline objects, span boundaries, glue, discretionaries, tabs);
/// 5. the sentinel last, `Leading` / `Trailing` boundary order, span
///    enclosure including seam spacing, and source coverage.
///
/// # Errors
/// The first problem found.
pub fn check_layout_inputs(
    ast: &ParagraphAst,
    text: &str,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> Result<(), LayoutInputError> {
    super::ValidatedInputs::try_from_ast(ast, text, constraints, options).map(drop)
}

/// A paragraph AST covers one paragraph: the document text is split first.
fn check_no_paragraph_separator(text: &str) -> Result<(), LayoutInputError> {
    if text.chars().any(is_paragraph_separator) {
        return Err(LayoutInputError::ParagraphSeparatorInText);
    }
    Ok(())
}

/// [`check_layout_inputs`], returning the policy it resolved on the way so
/// that layout resolves the strut once.
pub(super) fn validate_layout_inputs(
    ast: &ParagraphAst,
    text: &str,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> Result<LayoutPolicySnapshot, LayoutInputError> {
    let policy = LayoutPolicySnapshot::resolve(ast, constraints, options)?;
    check_no_paragraph_separator(text)?;
    // Phase order is error precedence. `check_storage` and `check_spans`
    // return the proofs that the later phases take as arguments.
    let boxes = storage::check_storage(ast)?;
    storage::check_presentation(ast, &boxes)?;
    spans::check_bidi_and_annotations(ast, text, &boxes)?;
    storage::check_runs(ast, text)?;
    let ids = spans::check_spans(ast, text, &boxes)?;
    items::check_items_fit(ast, text)?;
    items::check_item_order(ast)?;
    let spans = spans::SpanIndex::new(ast, &boxes);
    items::check_each_item(ast, text, &boxes, &ids, &spans)?;
    items::check_paragraph_end(ast)?;
    spans::check_boundary_order(ast, &ids)?;
    spans::check_span_enclosure(ast, &spans, &ids)?;
    storage::check_coverage(ast, text)?;
    Ok(policy)
}
