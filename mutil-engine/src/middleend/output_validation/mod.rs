//! Output validation: the invariants of [`PositionedLine`] and
//! [`PositionedParagraph`] (spec §2.6).

use super::tolerance::tolerance;
use super::{HangSide, LineContext, PositionedGlyph, PositionedLine, PositionedSlot};
use crate::types::{BidiLevel, TextRange};

mod line_checks;
mod paragraph;
mod slot_checks;
#[cfg(test)]
mod tests;
/// A violated [`PositionedLine`] invariant.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LineInvariantError {
    /// The line's slots are malformed or disagree with its content.
    InvalidSlots {
        /// What is wrong.
        reason: SlotErrorReason,
    },
    /// Run orientation does not match the writing mode or level parity.
    OrientationMismatch,
    /// A visual run is a tate-chu-yoko group, whose glyph placement the
    /// output model does not define yet (spec ORI-7).
    UnplacedTateChuYoko {
        /// Index into `visual_runs`.
        run: usize,
    },
    /// A glyph's source range is empty, inverted or outside the line, or a
    /// glyph of an ellipsis-only line has a source range.
    InvalidGlyphSource {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
    /// An annotation run is malformed or misplaced.
    InvalidAnnotation {
        /// Index into `annotations`.
        index: usize,
        /// What is wrong.
        reason: AnnotationErrorReason,
    },
    /// A paragraph-level invariant is violated; reported with the index of
    /// the line where it was detected.
    InvalidParagraph {
        /// What is wrong.
        reason: ParagraphErrorReason,
    },
    /// `baseline_y`, `ascent`, `descent` or `line_height` is NaN or infinite.
    NonFiniteLineMetrics,
    /// A line height or the resolved strut height is not positive.
    NonPositiveLineHeight {
        /// The strut's natural line height.
        expected: f32,
        /// The line's `line_height`.
        actual: f32,
    },
    /// `visible_byte_end` lies outside `byte_range`.
    VisibleEndOutsideRange,
    /// `visible_byte_end` is inside `byte_range` but is neither its start or end
    /// nor the start or end of a non-synthesized glyph, inline box or tab of
    /// the line, so it need not be a grapheme boundary.
    VisibleEndNotOnBoundary,
    /// An empty line (`byte_range.start == byte_range.end`) has visual runs.
    EmptyLineHasVisualRuns,
    /// An empty line has inline boxes.
    EmptyLineHasBoxes,
    /// An empty line's `line_height` differs from the strut's.
    EmptyLineHeightMismatch {
        /// The strut's natural line height.
        expected: f32,
        /// The line's `line_height`.
        actual: f32,
    },
    /// An empty line's slots hold a non-zero content width.
    EmptyLineHasExtent,
    /// Under `LineHeight::Fixed`, `line_height` differs from the strut's.
    LineHeightMismatch {
        /// The strut's natural line height.
        expected: f32,
        /// The line's `line_height`.
        actual: f32,
    },
    /// Under `LineHeight::Fixed`, `ascent` or `descent` differs from the
    /// strut's. A fixed line box never grows for annotations; they overflow
    /// into the inter-line space.
    LineMetricsMismatch {
        /// The strut's ascent.
        expected_ascent: f32,
        /// The strut's descent.
        expected_descent: f32,
        /// The line's `ascent`.
        actual_ascent: f32,
        /// The line's `descent`.
        actual_descent: f32,
    },
    /// Under `LineHeight::Normal`, `line_height` is smaller than the strut's.
    LineHeightBelowStrut {
        /// The strut's natural line height.
        minimum: f32,
        /// The line's `line_height`.
        actual: f32,
    },
    /// A visual run has no glyphs, bytes outside the line, or a font size or
    /// fallback scale that is non-finite or non-positive.
    InvalidVisualRun {
        /// Index into `visual_runs`.
        run: usize,
    },
    /// A glyph's pen position lies left of the previous glyph's in its run:
    /// the glyphs of a run are in screen order.
    GlyphsOutOfVisualOrder {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs` of the glyph that steps back.
        glyph: usize,
    },
    /// An atomic fragment's geometry is invalid; its bytes are
    /// empty, outside the line or outside its span; or the fragments of its
    /// span on this line disagree about the span's range, overlap, or do not
    /// cover the part of the span on the line.
    InvalidAtomicFragment {
        /// Index into `atomic_fragments`.
        index: usize,
    },
    /// A glyph's `x`, `x_offset`, `advance` or `y_offset` is NaN or infinite.
    NonFiniteGlyph {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
    /// A trailing-whitespace glyph has width although the mode is `Collapsible`.
    TrailingWhitespaceNotCollapsed {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
    /// A glyph flagged `is_hanging` overlaps the text block, i.e. hanging
    /// punctuation is being counted inside `visual_width`.
    HangingGlyphInsideBlock {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
    /// A body glyph (not `is_hanging`, not trailing whitespace) lies outside
    /// the content of every slot.
    BodyGlyphOutsideBlock {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
    /// An inline box has a non-finite coordinate, a negative size, an invalid
    /// baseline alignment, or bytes outside the line.
    InvalidInlineBox {
        /// Index into `visual_boxes`.
        index: usize,
    },
    /// An inline box lies outside the content of every slot.
    InlineBoxOutsideBlock {
        /// Index into `visual_boxes`.
        index: usize,
    },
    /// An empty line has tabs.
    EmptyLineHasTabs,
    /// An empty line has span fragments.
    EmptyLineHasSpanFragments,
    /// An empty line has atomic fragments.
    EmptyLineHasAtomicFragments,
    /// A span fragment has a non-finite coordinate or a negative size.
    InvalidSpanFragment {
        /// Index into `span_fragments`.
        index: usize,
    },
    /// A span fragment lies outside the content of every slot.
    SpanFragmentOutsideBlock {
        /// Index into `span_fragments`.
        index: usize,
    },
    /// A span fragment's `byte_range` lies outside the line's.
    SpanFragmentOutsideLine {
        /// Index into `span_fragments`.
        index: usize,
    },
    /// A span fragment's `byte_range` lies outside its `span_range`.
    SpanFragmentOutsideSpan {
        /// Index into `span_fragments`.
        index: usize,
    },
    /// A glyph belonging to a span lies outside every fragment of that span.
    GlyphOutsideSpanFragment {
        /// The span.
        span_id: u64,
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
    /// An inline box belonging to a span lies outside every fragment of it.
    InlineBoxOutsideSpanFragment {
        /// The span.
        span_id: u64,
        /// Index into `visual_boxes`.
        index: usize,
    },
    /// A `Clip` or `ScaleToFit` inline box ends beyond `ctx.wrap_width`.
    InlineBoxWiderThanWrapWidth {
        /// Index into `visual_boxes`.
        index: usize,
    },
    /// The line's total `visual_width` exceeds `ctx.wrap_width` although it is
    /// not a single overflowing entity.
    LineWiderThanWrapWidth {
        /// The line's `visual_width`.
        visual_width: f32,
        /// `LineContext::wrap_width`.
        wrap_width: f32,
    },
    /// A slot's content ends beyond the slot's own constraint although it is
    /// not a single overflowing entity, and the line as a whole still fits
    /// `ctx.wrap_width`.
    SlotOverflow {
        /// Index into `slots`.
        slot: usize,
        /// Distance from the constraint's start to the content's end.
        extent: f32,
        /// The constraint's width.
        capacity: f32,
    },
    /// A tab has a non-finite coordinate, a negative width, or empty bytes or
    /// bytes outside the line.
    InvalidTab {
        /// Index into `tabs`.
        index: usize,
    },
    /// A body tab lies outside the content of every slot.
    TabOutsideBlock {
        /// Index into `tabs`.
        index: usize,
    },
    /// A trailing tab has width although the mode is `Collapsible`.
    TrailingTabNotCollapsed {
        /// Index into `tabs`.
        index: usize,
    },
    /// Preserved trailing whitespace is not on the far edge of the base
    /// direction (UAX #9 L1): right of the body for LTR, left for RTL.
    TrailingWhitespaceOnWrongSide {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
    /// A preserved trailing tab is not on the far edge of the base direction.
    TrailingTabOnWrongSide {
        /// Index into `tabs`.
        index: usize,
    },
    /// A caret stop has a non-finite `x`, is out of visual order, lies outside
    /// the line's bytes or belongs to another line, or has a bidi level that
    /// no glyph, box or tab owning its offset has. At an offset no entity
    /// touches (glyph-less glue, an empty line) the level is the paragraph
    /// level or that of the nearest entity on either side (spec CRT-2).
    InvalidCaretStop {
        /// Index into `caret_stops`.
        index: usize,
    },
    /// A decoration has a non-finite coordinate, a negative width or
    /// thickness, or bytes outside the line.
    InvalidDecoration {
        /// Index into `decorations`.
        index: usize,
    },
    /// A trailing-whitespace glyph is flagged `is_hanging`; the flag is
    /// reserved for hanging punctuation.
    TrailingWhitespaceMarkedHanging {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
    /// Punctuation hangs at an edge the active `HangMode` does not allow.
    HangingNotAllowed {
        /// Edge concerned.
        side: HangSide,
    },
    /// Punctuation hangs at the line end under `HangEndMode::Allow` although
    /// it would have fitted the slot: `Allow` hangs a mark only when it would
    /// otherwise have to wrap.
    HangingWithoutOverflow {
        /// Edge concerned.
        side: HangSide,
    },
    /// The first pen position of a visual run lies left of the last one of
    /// the run before it: runs are in screen order (spec OUT-3).
    VisualRunsOutOfOrder {
        /// Index into `visual_runs` of the later run.
        run: usize,
    },
    /// More than one character hangs on one side of the line (spec HNG-4).
    TooManyHangingGlyphs {
        /// Side concerned.
        side: HangSide,
    },
    /// A body glyph reaches further towards the side a glyph hangs on, so
    /// the hanging glyph is not the outermost one (spec HNG-4).
    HangingGlyphNotOutermost {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
}

/// Why a line's slots are invalid ([`LineInvariantError::InvalidSlots`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SlotErrorReason {
    /// `LineContext::wrap_width` is NaN, zero or negative.
    InvalidWrapWidth,
    /// The line has no slot.
    NoSlots,
    /// A slot's constraint lies outside the wrap envelope, or overlaps or
    /// precedes the previous slot's.
    InvalidConstraint,
    /// A slot's `content_start` or `visual_width` is not finite, the width is
    /// negative, or the content starts outside the slot's constraint.
    InvalidContent,
    /// A slot's content block is not the span of the content placed in it.
    WidthNotContentSpan,
    /// The line has content but no slot holds any of it.
    ContentOutsideSlots,
}

/// Why an annotation is invalid ([`LineInvariantError::InvalidAnnotation`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AnnotationErrorReason {
    /// The line has no source bytes to annotate.
    OnEmptyLine,
    /// A size, scale, offset or extent is not finite, or one that must be
    /// positive or non-negative is not.
    InvalidMetrics,
    /// The orientation does not match the writing mode.
    OrientationMismatch,
    /// The base range is empty or outside the visible part of the line.
    BaseOutsideLine,
    /// The annotation has no glyphs.
    NoGlyphs,
    /// A glyph is not synthesized, carries a body byte range, or has a
    /// non-finite coordinate.
    InvalidGlyph,
    /// Another annotation of the line has the same id.
    DuplicateId,
    /// Another annotation on the same side shares base bytes.
    OverlapsOnSide,
    /// A body glyph intersects the base without lying wholly inside it.
    GlyphStraddlesBase,
    /// No body glyph lies in the base.
    NoBaseGlyphs,
    /// The base glyphs do not lie in one slot.
    BaseSpansSlots,
    /// A glyph lies beyond the overhang allowance or the slot.
    OutsideOverhang,
    /// The annotation sits closer to the baseline than its base.
    InsideBase,
    /// Under `LineHeight::Normal`, the line box is too small for the base and
    /// the annotation.
    DoesNotFitLineBox,
}

/// Why a paragraph is invalid ([`LineInvariantError::InvalidParagraph`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParagraphErrorReason {
    /// The recorded policy is not one layout could have resolved.
    InvalidPolicy,
    /// The paragraph has no line.
    NoLines,
    /// `total_block_size` is not finite, is negative, or ends above the last
    /// line's bottom.
    InvalidBlockSize,
    /// A line box is not finite, is inverted, or starts above the previous
    /// line's bottom.
    LineBoxesOverlap,
    /// A line's `ascent + descent` exceeds its `line_height`.
    LineBoxExceedsLineHeight,
    /// Under [`SlotSource::Standard`](super::SlotSource::Standard), a line's
    /// slots are not exactly the one standard slot `[indent, wrap_width]`.
    NotStandardSlots,
    /// The lines' byte ranges do not tile the rendered text from offset 0 on
    /// character boundaries.
    SourceNotCovered,
    /// The truncation state is inconsistent: an ellipsized line that is not
    /// the last, a `truncated_range` without a clamp policy or an ellipsized
    /// last line, or a tail that is empty or does not end the text.
    InvalidTruncation,
    /// Two lines carry an annotation with the same id.
    DuplicateAnnotation,
    /// The fragments of one decoration span disagree about its range, or do
    /// not hold exactly one first and one last fragment.
    InconsistentSpanFragments,
    /// The fragments of one atomic span disagree about its range or do not
    /// cover the rendered part of it.
    InconsistentAtomicFragments,
}

/// Whether `[lo, hi]` lies in the content block of `slot`. The one rule that
/// decides both whether content belongs to a slot and whether the slot counts
/// as occupied.
pub(super) fn slot_holds(slot: &PositionedSlot, lo: f32, hi: f32) -> bool {
    let eps = tolerance(slot.visual_width, 0.0);
    lo >= slot.content_start - eps && hi <= slot.content_end() + eps
}

/// Source ranges with a payload, sorted for intersection queries.
///
/// Glyph ranges are whole clusters, so they are pairwise disjoint or equal and
/// a query costs `O(log n)` plus its matches; `reach` keeps queries correct,
/// if slower, for ranges that overlap.
pub(super) struct RangeIndex<T> {
    /// Sorted by `range.start`.
    entries: Vec<(TextRange, T)>,
    /// `reach[i]` is the largest `range.end` among `entries[..=i]`.
    reach: Vec<usize>,
    /// `reach_at[i]` is the index of an entry in `entries[..=i]` that ends at
    /// `reach[i]`.
    reach_at: Vec<usize>,
}

impl<T> RangeIndex<T> {
    pub(super) fn new(mut entries: Vec<(TextRange, T)>) -> Self {
        entries.sort_by_key(|(range, _)| range.start());
        let mut reach = Vec::with_capacity(entries.len());
        let mut reach_at = Vec::with_capacity(entries.len());
        let (mut far, mut far_at) = (0, 0);
        for (index, (range, _)) in entries.iter().enumerate() {
            if index == 0 || range.end() > far {
                (far, far_at) = (range.end(), index);
            }
            reach.push(far);
            reach_at.push(far_at);
        }
        Self {
            entries,
            reach,
            reach_at,
        }
    }

    /// For an `offset` that no entry touches: the entry ending last before it
    /// and the entry starting first after it, if any.
    pub(super) fn neighbours(&self, offset: usize) -> [Option<&(TextRange, T)>; 2] {
        let split = self
            .entries
            .partition_point(|(range, _)| range.start() <= offset);
        let before = split
            .checked_sub(1)
            .map(|last| &self.entries[self.reach_at[last]]);
        [before, self.entries.get(split)]
    }

    /// The entries that can have `end >= min_end` and `start <= max_start`.
    fn candidates(&self, min_end: usize, max_start: usize) -> &[(TextRange, T)] {
        let from = self.reach.partition_point(|&reach| reach < min_end);
        let to = self
            .entries
            .partition_point(|(range, _)| range.start() <= max_start);
        self.entries.get(from..to).unwrap_or_default()
    }

    /// The entries sharing at least one byte with the non-empty `range`.
    pub(super) fn overlapping(&self, range: TextRange) -> impl Iterator<Item = &(TextRange, T)> {
        self.candidates(range.start() + 1, range.end() - 1)
            .iter()
            .filter(move |(r, _)| r.start() < range.end() && r.end() > range.start())
    }

    /// The entries that contain `offset` or start or end at it.
    pub(super) fn touching(&self, offset: usize) -> impl Iterator<Item = &(TextRange, T)> {
        self.candidates(offset, offset)
            .iter()
            .filter(move |(r, _)| r.start() <= offset && offset <= r.end())
    }
}

/// The glyphs of a line that render source text, by source range, each with
/// the bidi level of its run.
pub(super) type SourcedGlyphs<'a> = RangeIndex<(BidiLevel, &'a PositionedGlyph)>;

impl PositionedLine {
    /// Validates the line against `ctx`:
    ///
    /// 1. every number is finite and no extent or size is negative;
    /// 2. the envelope: byte range, empty-line state, line box;
    /// 3. source ownership: every run, glyph, box, tab, decoration, caret
    ///    stop and annotation refers to bytes of this line, runs are in screen
    ///    order and caret stops carry the level of the text they belong to;
    /// 4. the slots: valid constraints inside the wrap envelope, content
    ///    inside its constraint, conserved width;
    /// 5. containment: every entity lies in the content of one slot, each
    ///    slot's content block is exactly the span of what it holds, and
    ///    hanging punctuation, preserved whitespace, span fragments and
    ///    annotations sit where their rules put them.
    ///
    /// A slot's content may end beyond its constraint only when the line has a
    /// single slot that holds, from its start, one indivisible entity wider
    /// than it (see `is_single_entity_overflow`), optionally with a synthesized clamp
    /// suffix. Geometry describes pen advances, not glyph ink bounds:
    /// zero-advance combining marks are exempt from containment.
    ///
    /// # Errors
    /// The first violated invariant.
    pub fn validate(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        self.check_finite()?;
        self.check_envelope(ctx)?;
        self.check_sources(ctx)?;
        let glyphs = self.sourced_glyphs();
        self.check_caret_stops(ctx, &glyphs)?;
        self.check_annotation_records(ctx)?;
        self.check_slots(ctx)?;
        if self.byte_range.is_empty() && !self.is_ellipsized {
            return Ok(());
        }
        self.check_containment_and_hanging(ctx, &glyphs)
    }

    /// `true` when [`PositionedLine::validate`] succeeds.
    #[must_use]
    pub fn is_valid(&self, ctx: &LineContext) -> bool {
        self.validate(ctx).is_ok()
    }

    /// The body width: the sum of the slots' content advances, hanging
    /// punctuation, preserved trailing whitespace and obstacles excluded.
    #[must_use]
    pub fn visual_width(&self) -> f32 {
        self.slots.iter().map(|slot| slot.visual_width).sum()
    }

    /// Overflow of hanging punctuation beyond the left edge of the first
    /// occupied slot's content.
    #[must_use]
    pub fn hanging_left(&self) -> f32 {
        self.occupied_slots()
            .map_or(0.0, |o| self.hanging_extents(&o).0)
    }

    /// Overflow of hanging punctuation beyond the right edge of the last
    /// occupied slot's content.
    #[must_use]
    pub fn hanging_right(&self) -> f32 {
        self.occupied_slots()
            .map_or(0.0, |o| self.hanging_extents(&o).1)
    }

    /// `(left, right)` hanging overflow: the largest overflow of an
    /// `is_hanging` glyph's pen interval beyond the body edges, from the left
    /// edge of the first occupied slot's content to the right edge of the
    /// last one's.
    pub(super) fn hanging_extents(&self, &(first, last): &(usize, usize)) -> (f32, f32) {
        let (left, right) = (
            self.slots[first].content_start,
            self.slots[last].content_end(),
        );
        self.visual_runs
            .iter()
            .flat_map(|r| &r.glyphs)
            .filter(|g| g.is_hanging && g.advance != 0.0 && !self.is_trailing_whitespace(g))
            .fold((0.0_f32, 0.0_f32), |(l, r), g| {
                let (lo, hi) = g.pen_interval();
                (l.max(left - lo), r.max(hi - right))
            })
    }

    /// The first and last slot that hold content: a non-zero block, or a
    /// visible body glyph, an inline box or a visible tab inside its block.
    pub(super) fn occupied_slots(&self) -> Option<(usize, usize)> {
        let is_occupied = |slot: &PositionedSlot| {
            slot.visual_width > 0.0
                || self.visual_runs.iter().flat_map(|r| &r.glyphs).any(|g| {
                    let (lo, hi) = g.pen_interval();
                    !g.is_hanging
                        && g.byte_range
                            .is_some_and(|r| r.start() < self.visible_byte_end)
                        && slot_holds(slot, lo, hi)
                })
                || self
                    .visual_boxes
                    .iter()
                    .any(|b| slot_holds(slot, b.x, b.x + b.width))
                || self.tabs.iter().any(|t| {
                    t.byte_range.start() < self.visible_byte_end
                        && slot_holds(slot, t.x, t.x + t.width)
                })
        };
        let first = self.slots.iter().position(is_occupied)?;
        let last = self.slots.iter().rposition(is_occupied)?;
        Some((first, last))
    }

    /// `true` for a glyph of the trailing whitespace: source text at or after
    /// `visible_byte_end`.
    pub(super) fn is_trailing_whitespace(&self, glyph: &PositionedGlyph) -> bool {
        glyph
            .byte_range
            .is_some_and(|range| range.start() >= self.visible_byte_end)
    }

    /// `true` when `range` is well formed and inside this line's bytes.
    pub(super) fn owns_bytes(&self, range: TextRange) -> bool {
        range.start() <= range.end() && self.byte_range.contains_range(range)
    }

    fn sourced_glyphs(&self) -> SourcedGlyphs<'_> {
        RangeIndex::new(
            self.visual_runs
                .iter()
                .flat_map(|run| run.glyphs.iter().map(move |glyph| (run.bidi_level, glyph)))
                .filter_map(|(level, glyph)| Some((glyph.byte_range?, (level, glyph))))
                .collect(),
        )
    }
}
