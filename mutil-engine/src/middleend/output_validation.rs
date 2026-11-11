//! Output validation: the invariants of [`PositionedLine`] and
//! [`PositionedParagraph`] (spec §2.6).

use super::options::slots_are_valid;
use super::tolerance::{approx_eq, tolerance};
use super::{
    AtomicFragment, HangEndMode, HangSide, HangingWhitespaceMode, LineContext, LineHeight,
    PositionedGlyph, PositionedLine, PositionedParagraph, PositionedSlot,
};
use crate::frontend::{AnnotationSide, BoxOverflowPolicy};
use crate::types::{BIDI_MAX_DEPTH, BidiDirection, BidiLevel, TextRange};
use std::collections::{HashMap, HashSet};

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
    /// A glyph's synthesized flag and source range disagree, or its source range
    /// is empty, inverted or outside the line.
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
    /// `visual_width`, `hanging_left` or `hanging_right` is negative, NaN or infinite.
    NonFiniteOrNegativeExtent,
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
    /// An empty line has a non-zero `visual_width` or hanging extent.
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
    /// A visual run has no glyphs, bytes outside the line, or a font size,
    /// fallback scale or variation coordinate that is non-finite or
    /// non-positive.
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
    /// An atomic fragment's geometry or bidi level is invalid; its bytes are
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
    /// A hanging extent differs from what the hanging glyphs occupy: positive
    /// without hanging glyphs, zero with them, or simply a different width.
    HangingExtentMismatch {
        /// Edge concerned.
        side: HangSide,
        /// Extent derived from the glyphs.
        expected: f32,
        /// Stored extent.
        actual: f32,
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
    /// A span fragment's `byte_range` is inverted or outside the line's.
    SpanFragmentOutsideLine {
        /// Index into `span_fragments`.
        index: usize,
    },
    /// A span fragment's `byte_range` lies outside its `span_range`, or
    /// `is_first_fragment` / `is_last_fragment` disagrees with whether the
    /// fragment starts / ends where the span does.
    SpanFragmentEdgeMismatch {
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
    /// is unrepresentable or is not that of a run owning the text at its
    /// offset.
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
}

/// Why a line's slots are invalid ([`LineInvariantError::InvalidSlots`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SlotErrorReason {
    /// `LineContext::wrap_width` is NaN, zero or negative.
    InvalidWrapWidth,
    /// The line has no slot.
    NoSlots,
    /// A slot's constraint is malformed, lies outside the wrap envelope, or
    /// overlaps or precedes the previous slot's.
    InvalidConstraint,
    /// A slot's `content_start` or `visual_width` is not finite, the width is
    /// negative, or the content starts outside the slot's constraint.
    InvalidContent,
    /// The slots' `visual_width`s do not sum to the line's.
    WidthNotConserved,
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
    /// A size, scale, offset, extent or variation coordinate is not finite,
    /// or one that must be positive or non-negative is not.
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

/// The pen interval of a glyph, normalised so that `lo <= hi` (an advance may
/// be negative).
fn pen_interval(glyph: &PositionedGlyph) -> (f32, f32) {
    let end = glyph.x + glyph.advance;
    (glyph.x.min(end), glyph.x.max(end))
}

/// Whether `[lo, hi]` lies in the content block of `slot`. The one rule that
/// decides both whether content belongs to a slot and whether the slot counts
/// as occupied.
fn slot_holds(slot: &PositionedSlot, lo: f32, hi: f32) -> bool {
    let eps = tolerance(slot.visual_width, 0.0);
    lo >= slot.content_start - eps && hi <= slot.content_end() + eps
}

/// Source ranges with a payload, sorted for intersection queries.
///
/// Glyph ranges are whole clusters, so they are pairwise disjoint or equal and
/// a query costs `O(log n)` plus its matches; `reach` keeps queries correct,
/// if slower, for ranges that overlap.
struct RangeIndex<T> {
    /// Sorted by `range.start`.
    entries: Vec<(TextRange, T)>,
    /// `reach[i]` is the largest `range.end` among `entries[..=i]`.
    reach: Vec<usize>,
}

impl<T> RangeIndex<T> {
    fn new(mut entries: Vec<(TextRange, T)>) -> Self {
        entries.sort_by_key(|(range, _)| range.start);
        let reach = entries
            .iter()
            .scan(0, |reach, (range, _)| {
                *reach = (*reach).max(range.end);
                Some(*reach)
            })
            .collect();
        Self { entries, reach }
    }

    /// The entries that can have `end >= min_end` and `start <= max_start`.
    fn candidates(&self, min_end: usize, max_start: usize) -> &[(TextRange, T)] {
        let from = self.reach.partition_point(|&reach| reach < min_end);
        let to = self
            .entries
            .partition_point(|(range, _)| range.start <= max_start);
        self.entries.get(from..to).unwrap_or_default()
    }

    /// The entries sharing at least one byte with the non-empty `range`.
    fn overlapping(&self, range: TextRange) -> impl Iterator<Item = &(TextRange, T)> {
        self.candidates(range.start + 1, range.end - 1)
            .iter()
            .filter(move |(r, _)| r.start < range.end && r.end > range.start)
    }

    /// The entries that contain `offset` or start or end at it.
    fn touching(&self, offset: usize) -> impl Iterator<Item = &(TextRange, T)> {
        self.candidates(offset, offset)
            .iter()
            .filter(move |(r, _)| r.start <= offset && offset <= r.end)
    }
}

/// The glyphs of a line that render source text, by source range, each with
/// the bidi level of its run.
type SourcedGlyphs<'a> = RangeIndex<(BidiLevel, &'a PositionedGlyph)>;

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
    /// single slot as wide as the wrap width that holds one indivisible entity
    /// (see `is_single_entity_overflow`), optionally with a synthesized clamp
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

    /// `true` for a glyph of the trailing whitespace: source text at or after
    /// `visible_byte_end`.
    fn is_trailing_whitespace(&self, glyph: &PositionedGlyph) -> bool {
        !glyph.is_synthesized
            && glyph
                .byte_range
                .is_some_and(|range| range.start >= self.visible_byte_end)
    }

    /// `true` when `range` is well formed and inside this line's bytes.
    fn owns_bytes(&self, range: TextRange) -> bool {
        range.start <= range.end && self.byte_range.contains_range(range)
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

    /// Pass 1: all numbers finite, no negative extent or size.
    fn check_finite(&self) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let ok = |v: f32| v.is_finite() && v >= 0.0;
        if !(ok(self.visual_width) && ok(self.hanging_left) && ok(self.hanging_right)) {
            return Err(E::NonFiniteOrNegativeExtent);
        }
        let metrics = [
            self.baseline_y,
            self.ascent,
            self.descent,
            self.line_height,
            self.baseline_y - self.ascent,
            self.baseline_y + self.ascent,
            self.baseline_y - self.descent,
            self.baseline_y + self.descent,
            self.ascent + self.descent,
        ];
        if !metrics.iter().all(|v| v.is_finite()) {
            return Err(E::NonFiniteLineMetrics);
        }
        for (run, r) in self.visual_runs.iter().enumerate() {
            if !(r.font_size.is_finite()
                && r.font_size > 0.0
                && r.font.scale.is_finite()
                && r.font.scale > 0.0
                && r.variations.iter().all(|v| v.value().is_finite()))
            {
                return Err(E::InvalidVisualRun { run });
            }
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let finite = g.x.is_finite()
                    && g.x_offset.is_finite()
                    && g.advance.is_finite()
                    && g.y_offset.is_finite();
                if !(finite && (g.x + g.advance).is_finite() && (g.x + g.x_offset).is_finite()) {
                    return Err(E::NonFiniteGlyph { run, glyph });
                }
            }
        }
        for (index, b) in self.visual_boxes.iter().enumerate() {
            let finite = [b.x, b.y, b.width, b.ascent, b.descent, b.x + b.width]
                .iter()
                .all(|v| v.is_finite());
            if !(finite && b.width >= 0.0 && b.ascent >= 0.0 && b.descent >= 0.0) {
                return Err(E::InvalidInlineBox { index });
            }
        }
        for (index, t) in self.tabs.iter().enumerate() {
            let finite = t.x.is_finite() && t.width.is_finite() && (t.x + t.width).is_finite();
            if !(finite && t.width >= 0.0) {
                return Err(E::InvalidTab { index });
            }
        }
        for (index, f) in self.span_fragments.iter().enumerate() {
            let finite = [f.x, f.width, f.ascent, f.descent, f.x + f.width]
                .iter()
                .all(|v| v.is_finite());
            let level_ok = f.bidi_level <= BIDI_MAX_DEPTH + 1;
            if !(finite && level_ok && f.width >= 0.0 && f.ascent >= 0.0 && f.descent >= 0.0) {
                return Err(E::InvalidSpanFragment { index });
            }
        }
        for (index, f) in self.atomic_fragments.iter().enumerate() {
            let finite = f.x.is_finite() && f.width.is_finite() && (f.x + f.width).is_finite();
            if !(finite && f.width >= 0.0 && f.bidi_level <= BIDI_MAX_DEPTH + 1) {
                return Err(E::InvalidAtomicFragment { index });
            }
        }
        for (index, c) in self.caret_stops.iter().enumerate() {
            if !c.x.is_finite() {
                return Err(E::InvalidCaretStop { index });
            }
        }
        for (index, d) in self.decorations.iter().enumerate() {
            let finite = [d.x, d.width, d.y_offset, d.thickness, d.x + d.width]
                .iter()
                .all(|v| v.is_finite());
            if !(finite && d.width >= 0.0 && d.thickness >= 0.0) {
                return Err(E::InvalidDecoration { index });
            }
        }
        Ok(())
    }

    /// Pass 2: byte range, atomic fragments, empty-line state, line box.
    fn check_envelope(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let range = self.byte_range;
        if !(range.start <= self.visible_byte_end && self.visible_byte_end <= range.end) {
            return Err(E::VisibleEndOutsideRange);
        }
        self.check_atomic_fragment_ranges()?;
        let expected = ctx.strut.line_height();
        if !(self.line_height.is_finite()
            && self.line_height > 0.0
            && expected.is_finite()
            && expected > 0.0)
        {
            return Err(E::NonPositiveLineHeight {
                expected,
                actual: self.line_height,
            });
        }
        if range.is_empty() {
            self.check_empty_line(expected)?;
            if !self.is_ellipsized {
                // A fixed line box is the strut's even when the line is empty.
                return match ctx.line_height {
                    LineHeight::Fixed(_) => self.check_fixed_metrics(ctx),
                    LineHeight::Normal => Ok(()),
                };
            }
        }
        match ctx.line_height {
            LineHeight::Fixed(_) => {
                if !approx_eq(self.line_height, expected) {
                    return Err(E::LineHeightMismatch {
                        expected,
                        actual: self.line_height,
                    });
                }
                self.check_fixed_metrics(ctx)
            }
            LineHeight::Normal => {
                if self.line_height < expected - tolerance(self.line_height, expected) {
                    return Err(E::LineHeightBelowStrut {
                        minimum: expected,
                        actual: self.line_height,
                    });
                }
                Ok(())
            }
        }
    }

    /// A line without source bytes holds nothing that stands for source text.
    /// An ordinary empty line holds nothing at all and is exactly the strut
    /// high; a clamp may remove every base cluster of the last line, which
    /// then keeps only its synthesized suffix.
    fn check_empty_line(&self, strut_height: f32) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        if !self.is_ellipsized && !self.visual_runs.is_empty() {
            return Err(E::EmptyLineHasVisualRuns);
        }
        if !self.visual_boxes.is_empty() {
            return Err(E::EmptyLineHasBoxes);
        }
        if !self.tabs.is_empty() {
            return Err(E::EmptyLineHasTabs);
        }
        if !self.span_fragments.is_empty() {
            return Err(E::EmptyLineHasSpanFragments);
        }
        if !self.atomic_fragments.is_empty() {
            return Err(E::EmptyLineHasAtomicFragments);
        }
        if !self.annotations.is_empty() {
            return Err(E::InvalidAnnotation {
                index: 0,
                reason: AnnotationErrorReason::OnEmptyLine,
            });
        }
        if self.is_ellipsized {
            for (run, r) in self.visual_runs.iter().enumerate() {
                if let Some(glyph) = r.glyphs.iter().position(|g| !g.is_synthesized) {
                    return Err(E::InvalidGlyphSource { run, glyph });
                }
            }
            return Ok(());
        }
        if !approx_eq(self.line_height, strut_height) {
            return Err(E::EmptyLineHeightMismatch {
                expected: strut_height,
                actual: self.line_height,
            });
        }
        if self.visual_width != 0.0 || self.hanging_left != 0.0 || self.hanging_right != 0.0 {
            return Err(E::EmptyLineHasExtent);
        }
        Ok(())
    }

    /// Under `LineHeight::Fixed` the line box's `ascent` and `descent` are the
    /// strut's, whether or not the line has content.
    fn check_fixed_metrics(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        if approx_eq(self.ascent, ctx.strut.ascent) && approx_eq(self.descent, ctx.strut.descent) {
            return Ok(());
        }
        Err(LineInvariantError::LineMetricsMismatch {
            expected_ascent: ctx.strut.ascent,
            expected_descent: ctx.strut.descent,
            actual_ascent: self.ascent,
            actual_descent: self.descent,
        })
    }

    /// The bytes of the atomic fragments: each fragment owns a non-empty range
    /// inside the line and inside its span, and the fragments of one span
    /// agree on the span's range, are disjoint and together cover exactly the
    /// part of the span that lies on this line. (Bidi reordering can split
    /// one span into several visual fragments.)
    fn check_atomic_fragment_ranges(&self) -> Result<(), LineInvariantError> {
        let invalid = |index| Err(LineInvariantError::InvalidAtomicFragment { index });
        let fragments = &self.atomic_fragments;
        for (index, f) in fragments.iter().enumerate() {
            if f.fragment_byte_range.is_empty()
                || !f.full_byte_range.contains_range(f.fragment_byte_range)
                || !self.byte_range.contains_range(f.fragment_byte_range)
            {
                return invalid(index);
            }
        }
        let mut order: Vec<usize> = (0..fragments.len()).collect();
        order.sort_by_key(|&i| {
            (
                fragments[i].atomic_id,
                fragments[i].fragment_byte_range.start,
            )
        });
        let same_span = |a: &usize, b: &usize| fragments[*a].atomic_id == fragments[*b].atomic_id;
        for span in order.chunk_by(same_span) {
            let full = fragments[span[0]].full_byte_range;
            let mut covered = full.start.max(self.byte_range.start);
            for &index in span {
                let f: &AtomicFragment = &fragments[index];
                if f.full_byte_range != full || f.fragment_byte_range.start != covered {
                    return invalid(index);
                }
                covered = f.fragment_byte_range.end;
            }
            if covered != full.end.min(self.byte_range.end) {
                return invalid(span[span.len() - 1]);
            }
        }
        Ok(())
    }

    /// Pass 3: source ownership. Every run, glyph, box, tab and decoration
    /// refers to bytes of this line, the glyphs of a run are in screen order,
    /// and `visible_byte_end` is a boundary of the line's content.
    fn check_sources(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let valid_glyph = |g: &PositionedGlyph| {
            g.is_synthesized == g.byte_range.is_none()
                && g.byte_range
                    .is_none_or(|r| r.start < r.end && self.byte_range.contains_range(r))
        };
        for (run, r) in self.visual_runs.iter().enumerate() {
            if r.bidi_level > BIDI_MAX_DEPTH + 1
                || !r.orientation.is_valid(ctx.writing_mode, r.bidi_level)
            {
                return Err(E::OrientationMismatch);
            }
            if let Some(glyph) = r.glyphs.iter().position(|g| !valid_glyph(g)) {
                return Err(E::InvalidGlyphSource { run, glyph });
            }
            if r.glyphs.is_empty() || !self.owns_bytes(r.byte_range) {
                return Err(E::InvalidVisualRun { run });
            }
            // Screen order: a pen position never lies left of the previous one.
            // Marks share their base's position, so only `x` is compared.
            if let Some(step) = r
                .glyphs
                .windows(2)
                .position(|w| w[1].x < w[0].x - tolerance(w[0].x, w[1].x))
            {
                return Err(E::GlyphsOutOfVisualOrder {
                    run,
                    glyph: step + 1,
                });
            }
        }
        for (index, b) in self.visual_boxes.iter().enumerate() {
            if !b.baseline_align.is_valid()
                || b.byte_range.is_empty()
                || !self.owns_bytes(b.byte_range)
            {
                return Err(E::InvalidInlineBox { index });
            }
        }
        for (index, t) in self.tabs.iter().enumerate() {
            if t.byte_range.is_empty() || !self.owns_bytes(t.byte_range) {
                return Err(E::InvalidTab { index });
            }
        }
        for (index, d) in self.decorations.iter().enumerate() {
            if !self.owns_bytes(d.byte_range) {
                return Err(E::InvalidDecoration { index });
            }
        }
        self.check_visible_end_boundary()
    }

    /// `visible_byte_end` is a position the line's own content ends or starts
    /// at: the line's start or end, or the start or end of a non-synthesized
    /// glyph, an inline box or a tab. Glyph byte ranges are whole clusters and
    /// clusters are whole graphemes, so such an offset is a grapheme boundary;
    /// an offset inside a cluster could split one.
    fn check_visible_end_boundary(&self) -> Result<(), LineInvariantError> {
        let end = self.visible_byte_end;
        let on_edge = |r: TextRange| r.start == end || r.end == end;
        let ok = end == self.byte_range.start
            || end == self.byte_range.end
            || self
                .visual_runs
                .iter()
                .flat_map(|r| &r.glyphs)
                .any(|g| g.byte_range.is_some_and(on_edge))
            || self.visual_boxes.iter().any(|b| on_edge(b.byte_range))
            || self.tabs.iter().any(|t| on_edge(t.byte_range));
        if ok {
            Ok(())
        } else {
            Err(LineInvariantError::VisibleEndNotOnBoundary)
        }
    }

    /// Caret stops are in visual order, lie on this line, and carry the bidi
    /// level of the text they belong to.
    ///
    /// The level of a stop at `offset` is the level of a visual run with a
    /// glyph whose bytes contain or touch `offset`; at a run boundary either
    /// adjacent run qualifies. Inline boxes and tabs carry no level in the
    /// output, so a stop that touches one of them (or touches nothing at all)
    /// is only range-checked.
    fn check_caret_stops(
        &self,
        ctx: &LineContext,
        glyphs: &SourcedGlyphs<'_>,
    ) -> Result<(), LineInvariantError> {
        let unleveled = RangeIndex::new(
            self.visual_boxes
                .iter()
                .map(|b| (b.byte_range, ()))
                .chain(self.tabs.iter().map(|t| (t.byte_range, ())))
                .collect(),
        );
        for (index, stop) in self.caret_stops.iter().enumerate() {
            let in_order = index == 0 || {
                let previous = self.caret_stops[index - 1].x;
                stop.x >= previous - tolerance(previous, stop.x)
            };
            let level_is_owned = || {
                let mut owners = glyphs
                    .touching(stop.offset)
                    .map(|(_, (level, _))| *level)
                    .peekable();
                owners.peek().is_none()
                    || owners.any(|level| level == stop.level)
                    || unleveled.touching(stop.offset).next().is_some()
            };
            if !in_order
                || !(self.byte_range.start..=self.byte_range.end).contains(&stop.offset)
                || stop.line != ctx.line_index
                || stop.level > BIDI_MAX_DEPTH + 1
                || !level_is_owned()
            {
                return Err(LineInvariantError::InvalidCaretStop { index });
            }
        }
        Ok(())
    }

    /// The annotations' own records: metrics, base range, glyphs, unique ids
    /// and disjoint bases per side. Their placement is checked with the
    /// geometry, in `check_annotation_placement`.
    fn check_annotation_records(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use AnnotationErrorReason as R;
        let finite_nonnegative = |v: f32| v.is_finite() && v >= 0.0;
        let finite_positive = |v: f32| v.is_finite() && v > 0.0;
        for (index, ann) in self.annotations.iter().enumerate() {
            let reason = if !(finite_positive(ann.font_size)
                && finite_positive(ann.font.scale)
                && ann.baseline_offset.is_finite()
                && finite_nonnegative(ann.max_overhang)
                && finite_nonnegative(ann.block_extent)
                && finite_nonnegative(ann.base_over)
                && finite_nonnegative(ann.base_under)
                && ann.variations.iter().all(|v| v.value().is_finite()))
            {
                R::InvalidMetrics
            } else if !ann.orientation.is_valid(ctx.writing_mode, 0) {
                R::OrientationMismatch
            } else if ann.base_byte_range.is_empty()
                || ann.base_byte_range.start < self.byte_range.start
                || ann.base_byte_range.end > self.visible_byte_end
            {
                R::BaseOutsideLine
            } else if ann.glyphs.is_empty() {
                R::NoGlyphs
            } else if ann.glyphs.iter().any(|g| {
                let numbers = [
                    g.x,
                    g.x_offset,
                    g.y_offset,
                    g.advance,
                    g.x + g.advance,
                    g.x + g.x_offset,
                ];
                !g.is_synthesized
                    || g.byte_range.is_some()
                    || !numbers.iter().all(|v| v.is_finite())
            }) {
                R::InvalidGlyph
            } else {
                continue;
            };
            return Err(LineInvariantError::InvalidAnnotation { index, reason });
        }
        let mut ids = HashSet::new();
        if let Some(index) = self
            .annotations
            .iter()
            .position(|a| !ids.insert(a.annotation_id))
        {
            return Err(LineInvariantError::InvalidAnnotation {
                index,
                reason: R::DuplicateId,
            });
        }
        for side in [AnnotationSide::Over, AnnotationSide::Under] {
            let mut on_side: Vec<usize> = (0..self.annotations.len())
                .filter(|&i| self.annotations[i].side == side)
                .collect();
            on_side.sort_by_key(|&i| self.annotations[i].base_byte_range.start);
            let mut reach = 0;
            for &index in &on_side {
                let base = self.annotations[index].base_byte_range;
                if base.start < reach {
                    return Err(LineInvariantError::InvalidAnnotation {
                        index,
                        reason: R::OverlapsOnSide,
                    });
                }
                reach = base.end;
            }
        }
        Ok(())
    }

    /// Pass 4: the slots. Each constraint is well formed, inside the wrap
    /// envelope and right of the previous one; each content block is finite
    /// and starts inside its constraint; the content widths sum to the line's.
    /// Whether a block also *ends* inside its constraint is the overflow rule
    /// of pass 5.
    fn check_slots(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use SlotErrorReason as R;
        let invalid = |reason| Err(LineInvariantError::InvalidSlots { reason });
        if ctx.wrap_width.is_nan() || ctx.wrap_width <= 0.0 {
            return invalid(R::InvalidWrapWidth);
        }
        if self.slots.is_empty() {
            return invalid(R::NoSlots);
        }
        let constraints: smallvec::SmallVec<[_; 2]> =
            self.slots.iter().map(|slot| slot.constraint).collect();
        if !slots_are_valid(&constraints, ctx.wrap_width) {
            return invalid(R::InvalidConstraint);
        }
        for slot in &self.slots {
            let eps = tolerance(slot.content_start, slot.constraint.start);
            if !(slot.content_start.is_finite()
                && slot.visual_width.is_finite()
                && slot.visual_width >= 0.0
                && slot.content_end().is_finite()
                && slot.content_start >= slot.constraint.start - eps
                && slot.content_start <= slot.constraint.end + eps)
            {
                return invalid(R::InvalidContent);
            }
        }
        let total: f32 = self.slots.iter().map(|slot| slot.visual_width).sum();
        if !approx_eq(total, self.visual_width) {
            return invalid(R::WidthNotConserved);
        }
        Ok(())
    }

    /// Pass 5: containment of glyphs, boxes, tabs and fragments in the slots'
    /// content; hanging; trailing whitespace; annotations; overflow.
    fn check_containment_and_hanging(
        &self,
        ctx: &LineContext,
        glyphs: &SourcedGlyphs<'_>,
    ) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let slots = &self.slots;
        let is_occupied = |slot: &PositionedSlot| {
            slot.visual_width > 0.0
                || self.visual_runs.iter().flat_map(|r| &r.glyphs).any(|g| {
                    let (lo, hi) = pen_interval(g);
                    !g.is_hanging
                        && g.byte_range
                            .is_some_and(|r| r.start < self.visible_byte_end)
                        && slot_holds(slot, lo, hi)
                })
                || self
                    .visual_boxes
                    .iter()
                    .any(|b| slot_holds(slot, b.x, b.x + b.width))
                || self.tabs.iter().any(|t| {
                    t.byte_range.start < self.visible_byte_end
                        && slot_holds(slot, t.x, t.x + t.width)
                })
        };
        let first_index = slots.iter().position(is_occupied);
        let last_index = slots.iter().rposition(is_occupied);
        let (Some(first_index), Some(last_index)) = (first_index, last_index) else {
            let has_nothing = self.visual_runs.iter().all(|r| r.glyphs.is_empty())
                && self.visual_boxes.is_empty()
                && self.tabs.is_empty()
                && self.span_fragments.is_empty()
                && self.atomic_fragments.is_empty()
                && self.annotations.is_empty()
                && self.hanging_left == 0.0
                && self.hanging_right == 0.0;
            return if has_nothing {
                Ok(())
            } else {
                Err(E::InvalidSlots {
                    reason: SlotErrorReason::ContentOutsideSlots,
                })
            };
        };
        let (first, last) = (&slots[first_index], &slots[last_index]);
        // The body: from the left edge of the first occupied slot's content to
        // the right edge of the last one's.
        let (left, right) = (first.content_start, last.content_end());
        let eps = tolerance(self.visual_width, 0.0);
        let inside = |lo: f32, hi: f32| slots.iter().any(|s| slot_holds(s, lo, hi));
        // Hanging marks and preserved whitespace lie outside the body, and
        // never in an internal obstacle or an unused slot.
        let is_outside_clear = |lo: f32, hi: f32| {
            (lo >= left - eps || first_index == 0)
                && (hi <= right + eps || last_index + 1 == slots.len())
        };
        // UAX #9 L1: trailing whitespace is on the far edge of the base direction.
        let is_on_far_edge = |lo: f32, hi: f32| {
            is_outside_clear(lo, hi)
                && match ctx.base_direction {
                    BidiDirection::Ltr => lo >= right - eps,
                    BidiDirection::Rtl => hi <= left + eps,
                }
        };
        let collapsible = ctx.whitespace == HangingWhitespaceMode::Collapsible;
        let (mut hanging_left, mut hanging_right) = (0.0_f32, 0.0_f32);
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let (lo, hi) = pen_interval(g);
                if self.is_trailing_whitespace(g) {
                    if g.is_hanging {
                        return Err(E::TrailingWhitespaceMarkedHanging { run, glyph });
                    }
                    if collapsible && g.advance != 0.0 {
                        return Err(E::TrailingWhitespaceNotCollapsed { run, glyph });
                    }
                    if !collapsible && g.advance != 0.0 && !is_on_far_edge(lo, hi) {
                        return Err(E::TrailingWhitespaceOnWrongSide { run, glyph });
                    }
                } else if g.advance == 0.0 {
                    continue;
                } else if g.is_hanging {
                    let over_left = (left - lo).max(0.0);
                    let over_right = (hi - right).max(0.0);
                    if over_left <= eps && over_right <= eps {
                        return Err(E::HangingGlyphInsideBlock { run, glyph });
                    }
                    // A mark hangs from the outer slot on its side: whatever
                    // of it is not overflow lies in that slot's content.
                    let hangs_from_first = over_left > eps && hi <= first.content_end() + eps;
                    let hangs_from_last = over_right > eps && lo >= last.content_start - eps;
                    if !is_outside_clear(lo, hi) || !(hangs_from_first || hangs_from_last) {
                        return Err(E::BodyGlyphOutsideBlock { run, glyph });
                    }
                    hanging_left = hanging_left.max(over_left);
                    hanging_right = hanging_right.max(over_right);
                } else if !inside(lo, hi) {
                    return Err(E::BodyGlyphOutsideBlock { run, glyph });
                }
            }
        }
        for (side, expected, actual, slot) in [
            (HangSide::Left, hanging_left, self.hanging_left, first),
            (HangSide::Right, hanging_right, self.hanging_right, last),
        ] {
            if !approx_eq(expected, actual) || (actual > 0.0 && expected == 0.0) {
                return Err(E::HangingExtentMismatch {
                    side,
                    expected,
                    actual,
                });
            }
            if expected == 0.0 {
                continue;
            }
            if !ctx
                .hang_mode
                .allows(side, ctx.base_direction, ctx.line_index == 0)
            {
                return Err(E::HangingNotAllowed { side });
            }
            // `Allow` hangs a mark only if it would otherwise wrap: put back
            // into the body, it would not fit the slot it hangs from.
            let at_line_end = !side.is_line_start(ctx.base_direction);
            let capacity = slot.constraint.width();
            if at_line_end
                && ctx.hang_mode.end == HangEndMode::Allow
                && slot.visual_width + expected <= capacity - tolerance(capacity, 0.0)
            {
                return Err(E::HangingWithoutOverflow { side });
            }
        }
        for (index, b) in self.visual_boxes.iter().enumerate() {
            if !inside(b.x, b.x + b.width) {
                return Err(E::InlineBoxOutsideBlock { index });
            }
            if b.overflow_policy != BoxOverflowPolicy::AllowOverflow
                && b.x + b.width > ctx.wrap_width + eps
            {
                return Err(E::InlineBoxWiderThanWrapWidth { index });
            }
        }
        for (index, t) in self.tabs.iter().enumerate() {
            if t.byte_range.start >= self.visible_byte_end {
                if collapsible && t.width != 0.0 {
                    return Err(E::TrailingTabNotCollapsed { index });
                }
                if !collapsible && t.width != 0.0 && !is_on_far_edge(t.x, t.x + t.width) {
                    return Err(E::TrailingTabOnWrongSide { index });
                }
            } else if !inside(t.x, t.x + t.width) {
                return Err(E::TabOutsideBlock { index });
            }
        }
        for (index, f) in self.span_fragments.iter().enumerate() {
            if !inside(f.x, f.x + f.width) {
                return Err(E::SpanFragmentOutsideBlock { index });
            }
        }
        for (index, f) in self.atomic_fragments.iter().enumerate() {
            if !inside(f.x, f.x + f.width) {
                return Err(E::InvalidAtomicFragment { index });
            }
        }
        self.check_annotation_placement(ctx, glyphs)?;
        self.check_span_fragments()?;
        self.check_slot_overflow(ctx)?;
        self.check_slot_spans(ctx.base_direction)
    }

    /// Each annotation sits over its base, in the base's slot.
    ///
    /// The base is the envelope of the non-synthesized body glyphs that
    /// intersect its source range. Glue and other glyph-less nodes may sit
    /// inside the base, so contiguous byte coverage is deliberately not
    /// required. Every intersecting glyph must belong wholly to the base and
    /// to one slot; a glyph straddling either boundary is a source-ownership
    /// error, not an invitation to clip it.
    fn check_annotation_placement(
        &self,
        ctx: &LineContext,
        glyphs: &SourcedGlyphs<'_>,
    ) -> Result<(), LineInvariantError> {
        use AnnotationErrorReason as R;
        let eps = tolerance(self.visual_width, 0.0);
        for (index, ann) in self.annotations.iter().enumerate() {
            let invalid = |reason| Err(LineInvariantError::InvalidAnnotation { index, reason });
            let (mut base_lo, mut base_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for (range, (_, glyph)) in glyphs.overlapping(ann.base_byte_range) {
                if !ann.base_byte_range.contains_range(*range) {
                    return invalid(R::GlyphStraddlesBase);
                }
                let (lo, hi) = pen_interval(glyph);
                (base_lo, base_hi) = (base_lo.min(lo), base_hi.max(hi));
            }
            if base_lo > base_hi {
                return invalid(R::NoBaseGlyphs);
            }
            let Some(slot) = self.slots.iter().find(|slot| {
                base_lo >= slot.content_start - eps && base_hi <= slot.content_end() + eps
            }) else {
                return invalid(R::BaseSpansSlots);
            };
            let lo = slot.content_start.max(base_lo - ann.max_overhang);
            let hi = slot.content_end().min(base_hi + ann.max_overhang);
            if ann
                .glyphs
                .iter()
                .map(pen_interval)
                .any(|(glyph_lo, glyph_hi)| glyph_lo < lo - eps || glyph_hi > hi + eps)
            {
                return invalid(R::OutsideOverhang);
            }
            let (base_extent, line_extent, strut_extent, clears_base) = match ann.side {
                AnnotationSide::Over => (
                    ann.base_over,
                    self.ascent,
                    ctx.strut.ascent,
                    ann.baseline_offset >= ann.base_over - eps,
                ),
                AnnotationSide::Under => (
                    ann.base_under,
                    self.descent,
                    ctx.strut.descent,
                    ann.baseline_offset <= -ann.base_under + eps,
                ),
            };
            if !clears_base {
                return invalid(R::InsideBase);
            }
            // A fixed line box does not grow; the annotation overflows it.
            if matches!(ctx.line_height, LineHeight::Normal)
                && line_extent + eps < (base_extent + ann.block_extent).max(strut_extent)
            {
                return invalid(R::DoesNotFitLineBox);
            }
        }
        Ok(())
    }

    /// A slot's content ends inside its constraint, unless the line is one
    /// slot as wide as the wrap width holding a single entity that cannot be
    /// divided (`is_single_entity_overflow`).
    fn check_slot_overflow(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        for (index, slot) in self.slots.iter().enumerate() {
            let eps = tolerance(slot.visual_width, 0.0);
            let capacity = slot.constraint.width();
            let overflows = slot.content_end() > slot.constraint.end + eps;
            let is_exempt = self.slots.len() == 1
                && capacity >= ctx.wrap_width - eps
                && self.is_single_entity_overflow(slot.content_start);
            if overflows && !is_exempt {
                // The whole line is wider than the container, or only this
                // slot is wider than its own capacity.
                let line_overflows =
                    self.visual_width > ctx.wrap_width + tolerance(self.visual_width, 0.0);
                return Err(if line_overflows {
                    E::LineWiderThanWrapWidth {
                        visual_width: self.visual_width,
                        wrap_width: ctx.wrap_width,
                    }
                } else {
                    E::SlotOverflow {
                        slot: index,
                        extent: slot.content_end() - slot.constraint.start,
                        capacity,
                    }
                });
            }
        }
        Ok(())
    }

    /// Each slot's content block is exactly the span of the content placed in
    /// it (see [`PositionedSlot`]): every item lies inside the block, the
    /// item nearest the **line end** ends on the block's edge, and only the
    /// line-start edge may stand off, by the width of preserved leading glue.
    /// A slot without content has `visual_width == 0`.
    ///
    /// Content is the body glyphs (pen `x` to `x + advance`), inline boxes,
    /// tabs and span fragments (which carry the padding of a decoration span,
    /// advance that no glyph covers) that fit the slot.
    ///
    /// Not content: trailing whitespace and trailing tabs (they lie outside the
    /// body under `Preserved` and have no width under `Collapsible`),
    /// zero-advance glyphs (a pen position, not an extent) and hanging
    /// punctuation. A hanging glyph counts only through the part of it that lies
    /// inside the slot's content, so a mark that straddles the edge is content of
    /// the slot it hangs from and one entirely outside is not.
    fn check_slot_spans(&self, base_direction: BidiDirection) -> Result<(), LineInvariantError> {
        let slots = &self.slots;
        let mut spans: Vec<Option<(f32, f32)>> = vec![None; slots.len()];
        let mut place = |lo: f32, hi: f32| {
            if let Some(i) = slots.iter().position(|s| slot_holds(s, lo, hi)) {
                let (a, b) = spans[i].get_or_insert((lo, hi));
                *a = a.min(lo);
                *b = b.max(hi);
            }
        };
        for g in self.visual_runs.iter().flat_map(|r| &r.glyphs) {
            let (lo, hi) = pen_interval(g);
            if self.is_trailing_whitespace(g) || g.advance == 0.0 {
                continue;
            }
            if g.is_hanging {
                for s in slots.iter() {
                    let (lo, hi) = (lo.max(s.content_start), hi.min(s.content_end()));
                    if lo < hi {
                        place(lo, hi);
                    }
                }
            } else {
                place(lo, hi);
            }
        }
        for b in &self.visual_boxes {
            place(b.x, b.x + b.width);
        }
        for t in self
            .tabs
            .iter()
            .filter(|t| t.byte_range.start < self.visible_byte_end)
        {
            place(t.x, t.x + t.width);
        }
        // The padding of a decoration span is advance of the body that no glyph
        // or box covers: the fragment, which includes it, is part of the span.
        for f in &self.span_fragments {
            place(f.x, f.x + f.width);
        }
        for (slot, span) in slots.iter().zip(spans) {
            let is_content_span = match span {
                None => approx_eq(slot.visual_width, 0.0),
                // `slot_holds` already keeps the content inside the block.
                Some((lo, hi)) => match base_direction {
                    BidiDirection::Ltr => approx_eq(hi, slot.content_end()),
                    BidiDirection::Rtl => approx_eq(lo, slot.content_start),
                },
            };
            if !is_content_span {
                return Err(LineInvariantError::InvalidSlots {
                    reason: SlotErrorReason::WidthNotContentSpan,
                });
            }
        }
        Ok(())
    }

    /// The single-entity overflow exemptions (spec §2.6, Pass 5 (h)), for a
    /// line whose one slot's content starts at `origin`.
    ///
    /// **A:** nothing but one `AllowOverflow` box (no visual runs, no tabs).
    /// **B:** nothing but the glyphs of one indivisible cluster (a shared
    /// non-empty `byte_range`; no boxes, no tabs). The entity spans
    /// `[entity_start_x, entity_end_x]` and is `entity_width` wide.
    ///
    /// * No span fragments: the entity starts at `origin` and `visual_width`
    ///   equals `entity_width`.
    /// * Span fragments: they are the padding around the one entity. Every
    ///   fragment starts at the entity's start and ends at or after its end;
    ///   the entity ends at or after `visible_byte_end`, so the extra bytes lie
    ///   in the trailing whitespace (`check_span_fragments` keeps fragments
    ///   inside the line). The entity lies inside the body and the fragments
    ///   reach both edges of the body (`min x == origin`, `max end == origin +
    ///   visual_width`). Unpadded entities cannot inflate `visual_width` this
    ///   way.
    fn is_single_entity_overflow(&self, origin: f32) -> bool {
        let vw = self.visual_width;
        if !self.tabs.is_empty() {
            return false;
        }
        // (byte_range, start_x, end_x, entity_width)
        let entity = match self.visual_boxes.as_slice() {
            [b] => {
                if !(self.visual_runs.is_empty()
                    && b.overflow_policy == BoxOverflowPolicy::AllowOverflow)
                {
                    return false;
                }
                (b.byte_range, b.x, b.x + b.width, b.width)
            }
            [] => {
                let glyphs = || {
                    self.visual_runs
                        .iter()
                        .flat_map(|r| r.glyphs.iter())
                        .filter(|g| {
                            !g.is_hanging
                                && (!g.is_synthesized || self.is_ellipsized)
                                && g.byte_range.is_none_or(|b| b.start < self.visible_byte_end)
                        })
                };
                let mut ranges = glyphs().filter(|g| !g.is_synthesized).map(|g| g.byte_range);
                let first = ranges.next().unwrap_or_else(|| {
                    self.is_ellipsized
                        .then_some(TextRange::empty_at(self.visible_byte_end))
                });
                let Some(r) = first else {
                    return false;
                };
                if (r.is_empty() && !self.is_ellipsized) || !ranges.all(|o| o == first) {
                    return false;
                }
                let start = glyphs().map(|g| g.x).fold(f32::INFINITY, f32::min);
                let end = glyphs()
                    .map(|g| g.x + g.advance)
                    .fold(f32::NEG_INFINITY, f32::max);
                let width = glyphs().map(|g| g.advance).sum::<f32>();
                if !approx_eq(width, end - start) {
                    return false;
                }
                (r, start, end, width)
            }
            _ => return false,
        };
        let (range, start, end, width) = entity;
        if self.span_fragments.is_empty() {
            return (start - origin).abs() <= tolerance(width, vw) && approx_eq(width, vw);
        }
        let left = self
            .span_fragments
            .iter()
            .map(|f| f.x)
            .fold(f32::INFINITY, f32::min);
        let right = self
            .span_fragments
            .iter()
            .map(|f| f.x + f.width)
            .fold(f32::NEG_INFINITY, f32::max);
        self.span_fragments
            .iter()
            .all(|f| f.byte_range.start == range.start && f.byte_range.end >= range.end)
            && range.end >= self.visible_byte_end
            && start >= origin - tolerance(width, vw)
            && end <= origin + vw + tolerance(width, vw)
            && (left - origin).abs() <= tolerance(width, vw)
            && approx_eq(right - origin, vw)
    }

    /// Span fragments (spec §2.6, Pass 5 (g)). Each fragment covers a
    /// well-formed part of its span inside the line, and its first / last flags
    /// say exactly whether it starts / ends where the span does. Every glyph
    /// or inline box that belongs to a span lies inside one of its fragments.
    fn check_span_fragments(&self) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        if self.span_fragments.is_empty() {
            return Ok(());
        }
        for (index, f) in self.span_fragments.iter().enumerate() {
            if !self.owns_bytes(f.byte_range) {
                return Err(E::SpanFragmentOutsideLine { index });
            }
            if !f.span_range.contains_range(f.byte_range)
                || f.is_first_fragment != (f.byte_range.start == f.span_range.start)
                || f.is_last_fragment != (f.byte_range.end == f.span_range.end)
            {
                return Err(E::SpanFragmentEdgeMismatch { index });
            }
        }
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let Some(range) = g.byte_range else {
                    continue;
                };
                if g.is_hanging || self.is_trailing_whitespace(g) {
                    continue;
                }
                let (lo, hi) = pen_interval(g);
                if let Some(span_id) = self.uncovered_span(range, lo, hi) {
                    return Err(E::GlyphOutsideSpanFragment {
                        span_id,
                        run,
                        glyph,
                    });
                }
            }
        }
        for (index, b) in self.visual_boxes.iter().enumerate() {
            if let Some(span_id) = self.uncovered_span(b.byte_range, b.x, b.x + b.width) {
                return Err(E::InlineBoxOutsideSpanFragment { span_id, index });
            }
        }
        Ok(())
    }

    /// The first span owning `range` (some fragment of it covers `range` in
    /// bytes) none of whose fragments contains `[lo, hi]` horizontally.
    fn uncovered_span(&self, range: TextRange, lo: f32, hi: f32) -> Option<u64> {
        let vw = self.visual_width;
        self.span_fragments
            .iter()
            .filter(|f| f.byte_range.contains_range(range))
            .map(|f| f.span_id)
            .find(|&id| {
                !self.span_fragments.iter().any(|f| {
                    let tol = tolerance(f.x.abs().max((f.x + f.width).abs()), vw);
                    f.span_id == id && lo >= f.x - tol && hi <= f.x + f.width + tol
                })
            })
    }
}

impl PositionedParagraph {
    /// Validates the geometry of the paragraph against its own recorded
    /// policy: the policy is one layout could have resolved; there is at least
    /// one line; every line is valid under its own `line_index` (so
    /// `HangStartMode::FirstLine` applies to line 0 only); and the line boxes
    /// follow one another inside `total_block_size`.
    ///
    /// Serves geometry-only fixtures; it does not certify source coverage.
    ///
    /// # Errors
    /// The index of the first invalid line and its violation.
    pub fn validate_geometry(&self) -> Result<(), (usize, LineInvariantError)> {
        use ParagraphErrorReason as R;
        let invalid = |line, reason| Err((line, LineInvariantError::InvalidParagraph { reason }));
        if !self.policy.is_valid() {
            return invalid(0, R::InvalidPolicy);
        }
        if self.lines.is_empty() {
            return invalid(0, R::NoLines);
        }
        if !self.total_block_size.is_finite() || self.total_block_size < 0.0 {
            return invalid(0, R::InvalidBlockSize);
        }
        let mut previous_bottom = 0.0;
        for (i, line) in self.lines.iter().enumerate() {
            let ctx = self.policy.line_context(self.base_direction, i);
            line.validate(&ctx).map_err(|e| (i, e))?;
            let (top, bottom) =
                self.policy
                    .writing_mode
                    .line_bounds(line.baseline_y, line.ascent, line.descent);
            let eps = tolerance(line.line_height, 0.0);
            if !top.is_finite()
                || !bottom.is_finite()
                || top < previous_bottom - eps
                || bottom < top
            {
                return invalid(i, R::LineBoxesOverlap);
            }
            if line.ascent + line.descent > line.line_height + eps {
                return invalid(i, R::LineBoxExceedsLineHeight);
            }
            previous_bottom = bottom;
        }
        if self.total_block_size < previous_bottom - tolerance(previous_bottom, 0.0) {
            return invalid(self.lines.len() - 1, R::InvalidBlockSize);
        }
        Ok(())
    }

    /// Validates the whole paragraph against `text`, the paragraph text it was
    /// laid out from. Beyond [`validate_geometry`](Self::validate_geometry):
    ///
    /// * **coverage**: the lines' byte ranges tile the rendered text from
    ///   offset 0, on character boundaries;
    /// * **truncation**: an unclamped paragraph renders all of `text`; a
    ///   clamped one has a clamp policy, a non-empty `truncated_range` from the
    ///   last line's end to the end of `text`, and exactly its last line
    ///   ellipsized;
    /// * **identity across lines**: annotation ids are unique; the fragments
    ///   of one decoration span agree on its range and hold exactly one first
    ///   and one last fragment (the last one may fall in the truncated tail);
    ///   the fragments of one atomic span agree on its range and cover every
    ///   rendered byte of it.
    ///
    /// # Errors
    /// The first invalid line and its violation; a paragraph-level violation
    /// is reported with the line where it was detected.
    pub fn validate(&self, text: &str) -> Result<(), (usize, LineInvariantError)> {
        use ParagraphErrorReason as R;
        let invalid = |line, reason| Err((line, LineInvariantError::InvalidParagraph { reason }));
        self.validate_geometry()?;
        let last = self.lines.len() - 1;
        let mut rendered_end = 0;
        let mut annotation_ids = HashSet::new();
        for (i, line) in self.lines.iter().enumerate() {
            if line.byte_range.start != rendered_end
                || line.byte_range.end > text.len()
                || !text.is_char_boundary(line.byte_range.start)
                || !text.is_char_boundary(line.byte_range.end)
            {
                return invalid(i, R::SourceNotCovered);
            }
            if line.is_ellipsized && i != last {
                return invalid(i, R::InvalidTruncation);
            }
            if !line
                .annotations
                .iter()
                .all(|a| annotation_ids.insert(a.annotation_id))
            {
                return invalid(i, R::DuplicateAnnotation);
            }
            rendered_end = line.byte_range.end;
        }
        match self.truncated_range {
            None if rendered_end != text.len() => return invalid(last, R::SourceNotCovered),
            None if self.lines[last].is_ellipsized => return invalid(last, R::InvalidTruncation),
            None => {}
            Some(tail) => {
                let is_consistent = self.policy.clamp.is_some()
                    && tail.start == rendered_end
                    && tail.end == text.len()
                    && tail.start < tail.end
                    && self.lines[last].is_ellipsized;
                if !is_consistent {
                    return invalid(last, R::InvalidTruncation);
                }
            }
        }
        self.check_span_fragment_identity(rendered_end)?;
        self.check_atomic_fragment_identity(rendered_end)
    }

    /// Across the lines, the fragments of one decoration span carry one
    /// `span_range`, exactly one of them is the first and exactly one the
    /// last. Each line already ties the flags to the fragment's bytes, so the
    /// fragments are in logical order. A span whose end lies in the truncated
    /// tail (beyond `rendered_end`) has no last fragment.
    fn check_span_fragment_identity(
        &self,
        rendered_end: usize,
    ) -> Result<(), (usize, LineInvariantError)> {
        struct Seen {
            span_range: TextRange,
            line: usize,
            has_first: bool,
            has_last: bool,
        }
        let invalid = |line| {
            Err((
                line,
                LineInvariantError::InvalidParagraph {
                    reason: ParagraphErrorReason::InconsistentSpanFragments,
                },
            ))
        };
        let mut spans: HashMap<u64, Seen> = HashMap::new();
        for (line, fragment) in self
            .lines
            .iter()
            .enumerate()
            .flat_map(|(i, line)| line.span_fragments.iter().map(move |f| (i, f)))
        {
            let seen = spans.entry(fragment.span_id).or_insert(Seen {
                span_range: fragment.span_range,
                line,
                has_first: false,
                has_last: false,
            });
            if seen.span_range != fragment.span_range
                || (seen.has_first && fragment.is_first_fragment)
                || (seen.has_last && fragment.is_last_fragment)
            {
                return invalid(line);
            }
            seen.line = line;
            seen.has_first |= fragment.is_first_fragment;
            seen.has_last |= fragment.is_last_fragment;
        }
        // Report the earliest offending line, whatever the map's order.
        match spans
            .values()
            .filter(|seen| {
                let ends_in_tail = seen.span_range.end > rendered_end;
                !seen.has_first || (!seen.has_last && !ends_in_tail)
            })
            .map(|seen| seen.line)
            .min()
        {
            Some(line) => invalid(line),
            None => Ok(()),
        }
    }

    /// Across the lines, the fragments of one atomic span carry one
    /// `full_byte_range` and cover every rendered byte of it. Each line
    /// already checks that its fragments cover the part of the span on that
    /// line without overlap, so the byte counts decide: a line that crosses
    /// the span without a fragment leaves the total short.
    fn check_atomic_fragment_identity(
        &self,
        rendered_end: usize,
    ) -> Result<(), (usize, LineInvariantError)> {
        struct Seen {
            full: TextRange,
            line: usize,
            bytes: usize,
        }
        let invalid = |line| {
            Err((
                line,
                LineInvariantError::InvalidParagraph {
                    reason: ParagraphErrorReason::InconsistentAtomicFragments,
                },
            ))
        };
        let mut spans: HashMap<u64, Seen> = HashMap::new();
        for (line, fragment) in self
            .lines
            .iter()
            .enumerate()
            .flat_map(|(i, line)| line.atomic_fragments.iter().map(move |f| (i, f)))
        {
            let seen = spans.entry(fragment.atomic_id).or_insert(Seen {
                full: fragment.full_byte_range,
                line,
                bytes: 0,
            });
            if seen.full != fragment.full_byte_range {
                return invalid(line);
            }
            seen.line = line;
            seen.bytes += fragment.fragment_byte_range.len();
        }
        match spans
            .values()
            .filter(|seen| {
                seen.bytes
                    != seen
                        .full
                        .end
                        .min(rendered_end)
                        .saturating_sub(seen.full.start)
            })
            .map(|seen| seen.line)
            .min()
        {
            Some(line) => invalid(line),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleend::{
        CaretStop, DecorationKind, DecorationLine, HangMode, HangStartMode, InlineSlotConstraint,
        LayoutPolicySnapshot, PositionedInlineBox, PositionedSpanFragment, PositionedTab,
        StrutMetrics, VisualGlyphRun,
    };
    use crate::types::{
        BaselineAlignment, ByteIndex, FontId, FontRef, GlyphOrientation, WritingMode,
    };
    use smallvec::{SmallVec, smallvec};

    const STRUT: StrutMetrics = StrutMetrics {
        ascent: 12.0,
        descent: 4.0,
        line_gap: 0.0,
    };

    /// An LTR horizontal context with no wrap limit. Permissive on hanging so
    /// the geometry tests stay about geometry.
    fn ctx() -> LineContext {
        LineContext {
            writing_mode: WritingMode::HorizontalTb,
            base_direction: BidiDirection::Ltr,
            strut: STRUT,
            line_height: LineHeight::Normal,
            whitespace: HangingWhitespaceMode::Collapsible,
            hang_mode: HangMode {
                start: HangStartMode::AllLines,
                end: HangEndMode::Force,
            },
            wrap_width: f32::INFINITY,
            line_index: 0,
        }
    }

    fn preserved() -> LineContext {
        LineContext {
            whitespace: HangingWhitespaceMode::Preserved,
            ..ctx()
        }
    }

    /// The policy of [`ctx`], for paragraph fixtures.
    fn policy(ctx: &LineContext) -> LayoutPolicySnapshot {
        LayoutPolicySnapshot {
            writing_mode: ctx.writing_mode,
            strut: ctx.strut,
            line_height: ctx.line_height,
            whitespace: ctx.whitespace,
            hang_mode: ctx.hang_mode,
            wrap_width: ctx.wrap_width,
            clamp: None,
        }
    }

    /// A slot `capacity` wide from `start`, with content from its start.
    fn slot(start: f32, visual_width: f32, capacity: f32) -> PositionedSlot {
        PositionedSlot {
            constraint: InlineSlotConstraint {
                start,
                end: start + capacity,
            },
            content_start: start,
            visual_width,
        }
    }

    fn glyph(x: f32, advance: f32, is_hanging: bool) -> PositionedGlyph {
        PositionedGlyph {
            glyph_id: 1,
            x,
            x_offset: 0.0,
            y_offset: 0.0,
            advance,
            byte_range: Some(TextRange::new(0, 1)),
            is_synthesized: false,
            is_hanging,
        }
    }

    fn one_slot(visual_width: f32, wrap_width: f32) -> SmallVec<[PositionedSlot; 1]> {
        smallvec![slot(0.0, visual_width, wrap_width)]
    }

    fn line(glyphs: Vec<PositionedGlyph>, width: f32, hang_l: f32, hang_r: f32) -> PositionedLine {
        let byte_end = glyphs
            .iter()
            .filter_map(|g| g.byte_range.map(|r| r.end))
            .max()
            .unwrap_or(4)
            .max(4);
        PositionedLine {
            slots: one_slot(width, f32::INFINITY),
            annotations: Vec::new(),
            is_ellipsized: false,
            atomic_fragments: vec![],

            byte_range: TextRange::new(0, byte_end),
            visible_byte_end: byte_end,
            baseline_y: 12.0,
            ascent: 12.0,
            descent: 4.0,
            line_height: 16.0,
            visual_width: width,
            hanging_left: hang_l,
            hanging_right: hang_r,
            is_justified: false,
            visual_runs: vec![VisualGlyphRun {
                orientation: GlyphOrientation::Horizontal,
                font_size: 16.0,
                variations: Default::default(),

                font: FontRef::unscaled(FontId(0)),
                bidi_level: 0,
                byte_range: TextRange::new(0, byte_end),
                glyphs,
            }],
            visual_boxes: vec![],
            tabs: vec![],
            span_fragments: vec![],
            caret_stops: vec![],
            decorations: vec![],
        }
    }

    /// Sets the line's width and keeps its one slot in step.
    fn set_width(l: &mut PositionedLine, width: f32) {
        l.visual_width = width;
        l.slots[0].visual_width = width;
    }

    fn empty_line(height: f32) -> PositionedLine {
        PositionedLine {
            byte_range: TextRange::empty_at(7),
            visible_byte_end: 7,
            line_height: height,
            visual_runs: vec![],
            ..line(vec![], 0.0, 0.0, 0.0)
        }
    }

    fn inline_box(x: f32, width: f32) -> PositionedInlineBox {
        PositionedInlineBox {
            baseline_align: BaselineAlignment::Auto,
            box_id: 9,
            byte_range: TextRange::new(1, 4),
            x,
            y: 0.0,
            width,
            ascent: 10.0,
            descent: 2.0,
            overflow_policy: BoxOverflowPolicy::Clip,
        }
    }

    #[test]
    fn empty_line_contract() {
        assert!(empty_line(16.0).is_valid(&ctx()));
        assert_eq!(
            empty_line(15.0).validate(&ctx()),
            Err(LineInvariantError::EmptyLineHeightMismatch {
                expected: 16.0,
                actual: 15.0
            })
        );
        let mut with_runs = line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        with_runs.byte_range = TextRange::empty_at(0);
        with_runs.visible_byte_end = 0;
        assert_eq!(
            with_runs.validate(&ctx()),
            Err(LineInvariantError::EmptyLineHasVisualRuns)
        );
        let mut boxed = empty_line(16.0);
        boxed.visual_boxes.push(inline_box(0.0, 0.0));
        assert_eq!(
            boxed.validate(&ctx()),
            Err(LineInvariantError::EmptyLineHasBoxes)
        );
        let mut wide = empty_line(16.0);
        wide.visual_width = 1.0;
        assert_eq!(
            wide.validate(&ctx()),
            Err(LineInvariantError::EmptyLineHasExtent)
        );
    }

    #[test]
    fn line_height_follows_the_policy() {
        let mut tall = line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        tall.line_height = 20.0;
        // Normal: taller is fine (fallback glyphs), shorter is not.
        assert!(tall.is_valid(&ctx()));
        let mut short = tall.clone();
        short.line_height = 10.0;
        assert_eq!(
            short.validate(&ctx()),
            Err(LineInvariantError::LineHeightBelowStrut {
                minimum: 16.0,
                actual: 10.0
            })
        );
        // Fixed: must equal the resolved strut exactly (within tolerance).
        let fixed = LineContext {
            line_height: LineHeight::Fixed(1.0),
            ..ctx()
        };
        assert_eq!(
            tall.validate(&fixed),
            Err(LineInvariantError::LineHeightMismatch {
                expected: 16.0,
                actual: 20.0
            })
        );
        assert!(line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0).is_valid(&fixed));
    }

    #[test]
    fn hanging_on_both_edges_is_valid_and_excluded_from_width() {
        // Body [0, 20]; an opening mark hangs left, a full stop hangs right.
        let l = line(
            vec![
                glyph(-6.0, 6.0, true),
                glyph(0.0, 20.0, false),
                glyph(20.0, 5.0, true),
            ],
            20.0,
            6.0,
            5.0,
        );
        assert!(l.is_valid(&ctx()));
    }

    #[test]
    fn counting_hanging_punctuation_in_visual_width_is_rejected() {
        let l = line(
            vec![glyph(0.0, 20.0, false), glyph(20.0, 5.0, true)],
            25.0,
            0.0,
            0.0,
        );
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::HangingGlyphInsideBlock { run: 0, glyph: 1 })
        );
    }

    #[test]
    fn hanging_extent_is_positive_iff_hanging_glyphs_exist() {
        let none = line(vec![glyph(0.0, 20.0, false)], 20.0, 0.0, 5.0);
        assert_eq!(
            none.validate(&ctx()),
            Err(LineInvariantError::HangingExtentMismatch {
                side: HangSide::Right,
                expected: 0.0,
                actual: 5.0
            })
        );
        let missing = line(
            vec![glyph(0.0, 20.0, false), glyph(20.0, 5.0, true)],
            20.0,
            0.0,
            0.0,
        );
        assert_eq!(
            missing.validate(&ctx()),
            Err(LineInvariantError::HangingExtentMismatch {
                side: HangSide::Right,
                expected: 5.0,
                actual: 0.0
            })
        );
        let wrong = line(
            vec![glyph(-6.0, 6.0, true), glyph(0.0, 20.0, false)],
            20.0,
            3.0,
            0.0,
        );
        assert_eq!(
            wrong.validate(&ctx()),
            Err(LineInvariantError::HangingExtentMismatch {
                side: HangSide::Left,
                expected: 6.0,
                actual: 3.0
            })
        );
    }

    fn trailing_space(x: f32, advance: f32) -> PositionedGlyph {
        PositionedGlyph {
            byte_range: Some(TextRange::new(3, 4)),
            ..glyph(x, advance, false)
        }
    }

    #[test]
    fn preserved_trailing_space_may_sit_outside_the_body() {
        // Body "ab" = [0, 20]; the space (bytes 3..4) hangs beyond visual_width.
        let mut l = line(
            vec![glyph(0.0, 20.0, false), trailing_space(20.0, 5.0)],
            20.0,
            0.0,
            0.0,
        );
        l.visible_byte_end = 3;
        assert!(l.is_valid(&preserved()));
        // It is not punctuation: hanging_right stays zero and a non-zero value is wrong.
        let mut claimed = l.clone();
        claimed.hanging_right = 5.0;
        assert_eq!(
            claimed.validate(&preserved()),
            Err(LineInvariantError::HangingExtentMismatch {
                side: HangSide::Right,
                expected: 0.0,
                actual: 5.0
            })
        );
        // Collapsible: the same glyph must have collapsed to zero width.
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::TrailingWhitespaceNotCollapsed { run: 0, glyph: 1 })
        );
        let mut collapsed = l;
        collapsed.visual_runs[0].glyphs[1] = trailing_space(20.0, 0.0);
        assert!(collapsed.is_valid(&ctx()));
    }

    #[test]
    fn mixed_tolerance_absorbs_f32_noise_at_large_coordinates() {
        // At 1e6 an f32 ulp is 0.0625, far above the 1e-3 absolute floor.
        let l = line(vec![glyph(0.0, 1_000_000.1, false)], 1_000_000.0, 0.0, 0.0);
        assert!(l.is_valid(&ctx()));
        // Real overflow is still caught (relative tolerance there is 10).
        let l = line(vec![glyph(0.0, 1_000_100.0, false)], 1_000_000.0, 0.0, 0.0);
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
        );
        // Small coordinates keep the strict absolute floor.
        let l = line(vec![glyph(0.0, 20.01, false)], 20.0, 0.0, 0.0);
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
        );
    }

    #[test]
    fn inline_boxes_assemble_and_are_checked() {
        let mut l = line(vec![glyph(0.0, 5.0, false)], 30.0, 0.0, 0.0);
        l.visual_boxes = vec![inline_box(5.0, 12.0), inline_box(17.0, 13.0)];
        assert!(l.is_valid(&ctx()));
        assert_eq!(l.visual_boxes[1].box_id, 9);
        assert_eq!(l.visual_boxes[0].byte_range, TextRange::new(1, 4));

        let mut outside = l.clone();
        outside.visual_boxes[1].width = 40.0;
        assert_eq!(
            outside.validate(&ctx()),
            Err(LineInvariantError::InlineBoxOutsideBlock { index: 1 })
        );
        let mut negative = l.clone();
        negative.visual_boxes[0].width = -1.0;
        assert_eq!(
            negative.validate(&ctx()),
            Err(LineInvariantError::InvalidInlineBox { index: 0 })
        );
        let mut nan = l;
        nan.visual_boxes[0].y = f32::NAN;
        assert_eq!(
            nan.validate(&ctx()),
            Err(LineInvariantError::InvalidInlineBox { index: 0 })
        );
    }

    #[test]
    fn visible_end_must_lie_inside_the_line() {
        let mut l = line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        l.visible_byte_end = 9;
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::VisibleEndOutsideRange)
        );
    }

    #[test]
    fn non_finite_geometry_is_rejected() {
        for bad in [f32::NAN, f32::INFINITY] {
            let mut g = glyph(0.0, 5.0, false);
            g.x = bad;
            assert_eq!(
                line(vec![g], 5.0, 0.0, 0.0).validate(&ctx()),
                Err(LineInvariantError::NonFiniteGlyph { run: 0, glyph: 0 })
            );
            let mut g = glyph(0.0, 5.0, false);
            g.advance = bad;
            assert_eq!(
                line(vec![g], 5.0, 0.0, 0.0).validate(&ctx()),
                Err(LineInvariantError::NonFiniteGlyph { run: 0, glyph: 0 })
            );
            let mut l = line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
            l.line_height = bad;
            assert_eq!(
                l.validate(&ctx()),
                Err(LineInvariantError::NonFiniteLineMetrics)
            );
        }
    }

    #[test]
    fn zero_advance_marks_with_ink_offsets_do_not_trip_containment() {
        // The mark's pen position is 0; its ink sits 3 units to the left.
        let mut mark = glyph(0.0, 0.0, false);
        mark.x_offset = -3.0;
        let l = line(vec![mark, glyph(0.0, 20.0, false)], 20.0, 0.0, 0.0);
        assert!(l.is_valid(&ctx()));
        // `x_offset` plays no part in containment, even when large.
        let mut far = glyph(5.0, 10.0, false);
        far.x_offset = -50.0;
        let body = || vec![glyph(0.0, 5.0, false), far, glyph(15.0, 5.0, false)];
        assert!(line(body(), 20.0, 0.0, 0.0).is_valid(&ctx()));
        // A zero advance has no horizontal extent, so its pen position is
        // exempt from containment even outside the body (and it is not content
        // of the slot).
        let with_mark = |mark: PositionedGlyph| {
            // A run's glyphs are in screen order.
            let mut glyphs = vec![mark, glyph(0.0, 20.0, false)];
            glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
            line(glyphs, 20.0, 0.0, 0.0)
        };
        assert!(with_mark(glyph(-3.0, 0.0, false)).is_valid(&ctx()));
        assert!(with_mark(glyph(25.0, 0.0, false)).is_valid(&ctx()));
        // A zero-advance mark on a hanging base is flagged hanging and skipped.
        let l = line(
            vec![
                glyph(0.0, 20.0, false),
                glyph(20.0, 5.0, true),
                glyph(25.0, 0.0, true),
            ],
            20.0,
            0.0,
            5.0,
        );
        assert!(l.is_valid(&ctx()));
        // A non-finite ink offset is rejected in pass 1.
        let mut nan = glyph(0.0, 5.0, false);
        nan.x_offset = f32::NAN;
        assert_eq!(
            line(vec![nan], 5.0, 0.0, 0.0).validate(&ctx()),
            Err(LineInvariantError::NonFiniteGlyph { run: 0, glyph: 0 })
        );
    }

    fn fragment(span_id: u64, x: f32, width: f32, bytes: (usize, usize)) -> PositionedSpanFragment {
        PositionedSpanFragment {
            span_id,
            span_range: TextRange::new(bytes.0, bytes.1),
            byte_range: TextRange::new(bytes.0, bytes.1),
            x,
            width,
            ascent: 12.0,
            descent: 4.0,
            is_first_fragment: true,
            is_last_fragment: true,
            bidi_level: 0,
        }
    }

    /// Two glyphs `[0,10]` (bytes 0..2) and `[10,30]` (bytes 2..4); body 30.
    fn spanned_line() -> PositionedLine {
        let mut second = glyph(10.0, 20.0, false);
        second.byte_range = Some(TextRange::new(2, 4));
        let mut first = glyph(0.0, 10.0, false);
        first.byte_range = Some(TextRange::new(0, 2));
        line(vec![first, second], 30.0, 0.0, 0.0)
    }

    #[test]
    fn span_fragments_must_be_sane_and_inside_the_block() {
        let mut l = spanned_line();
        l.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 4))];
        assert!(l.is_valid(&ctx()));
        for (frag, expected) in [
            (
                fragment(1, -2.0, 10.0, (0, 4)),
                LineInvariantError::SpanFragmentOutsideBlock { index: 0 },
            ),
            (
                fragment(1, 0.0, 31.0, (0, 4)),
                LineInvariantError::SpanFragmentOutsideBlock { index: 0 },
            ),
            (
                fragment(1, 0.0, -1.0, (0, 4)),
                LineInvariantError::InvalidSpanFragment { index: 0 },
            ),
            (
                fragment(1, f32::NAN, 5.0, (0, 4)),
                LineInvariantError::InvalidSpanFragment { index: 0 },
            ),
        ] {
            let mut l = spanned_line();
            l.span_fragments = vec![frag];
            assert_eq!(l.validate(&ctx()), Err(expected));
        }
        let mut negative_extent = spanned_line();
        negative_extent.span_fragments = vec![PositionedSpanFragment {
            descent: -1.0,
            ..fragment(1, 0.0, 30.0, (0, 4))
        }];
        assert_eq!(
            negative_extent.validate(&ctx()),
            Err(LineInvariantError::InvalidSpanFragment { index: 0 })
        );
        let mut empty = empty_line(16.0);
        empty.span_fragments.push(fragment(1, 0.0, 0.0, (7, 7)));
        assert_eq!(
            empty.validate(&ctx()),
            Err(LineInvariantError::EmptyLineHasSpanFragments)
        );
    }

    #[test]
    fn span_members_must_lie_inside_a_fragment_of_their_span() {
        // Span 1 covers bytes 2..4 and a fragment [10, 30]: the second glyph fits.
        let mut l = spanned_line();
        l.span_fragments = vec![fragment(1, 10.0, 20.0, (2, 4))];
        assert!(l.is_valid(&ctx()));
        // A fragment too narrow for the glyph it owns.
        l.span_fragments = vec![fragment(1, 10.0, 15.0, (2, 4))];
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::GlyphOutsideSpanFragment {
                span_id: 1,
                run: 0,
                glyph: 1
            })
        );
        // A glyph outside every byte range is not a member and is not checked.
        l.span_fragments = vec![fragment(1, 10.0, 15.0, (3, 4))];
        assert!(l.is_valid(&ctx()));
        // Bidi can split one span into two visual groups; membership needs only
        // one fragment of the span to contain the glyph.
        l.span_fragments = vec![
            fragment(1, 0.0, 10.0, (0, 2)),
            fragment(1, 10.0, 20.0, (2, 4)),
        ];
        assert!(l.is_valid(&ctx()));
        // Padding makes a fragment larger than its content: still contained.
        let mut padded = spanned_line();
        set_width(&mut padded, 34.0);
        padded.span_fragments = vec![fragment(2, 0.0, 34.0, (0, 4))];
        assert!(padded.is_valid(&ctx()));
    }

    #[test]
    fn span_membership_covers_inline_boxes_and_exempts_hanging_and_trailing() {
        let mut l = line(vec![glyph(0.0, 5.0, false)], 30.0, 0.0, 0.0);
        l.visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(0, 1));
        l.visual_boxes = vec![inline_box(10.0, 12.0)]; // bytes 1..4
        l.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 4))];
        assert!(l.is_valid(&ctx()));
        l.span_fragments = vec![fragment(1, 0.0, 18.0, (0, 4))];
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::InlineBoxOutsideSpanFragment {
                span_id: 1,
                index: 0
            })
        );
        // A hanging glyph lies outside the body and so outside the fragment.
        let mut h = line(
            vec![glyph(0.0, 20.0, false), glyph(20.0, 5.0, true)],
            20.0,
            0.0,
            5.0,
        );
        h.visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(0, 3));
        h.visual_runs[0].glyphs[1].byte_range = Some(TextRange::new(3, 4));
        h.span_fragments = vec![fragment(1, 0.0, 20.0, (0, 4))];
        assert!(h.is_valid(&ctx()));
    }

    #[test]
    fn a_lone_overflowing_object_is_valid_when_the_width_equals_the_box() {
        // `AllowOverflow`: the line's visual_width is set to the box width, so
        // containment [0, visual_width] holds for a box wider than wrap_width.
        let mut l = line(vec![], 500.0, 0.0, 0.0);
        l.visual_runs.clear();
        l.visual_boxes = vec![inline_box(0.0, 500.0)];
        assert!(l.is_valid(&ctx()));
    }

    #[test]
    fn body_glyphs_must_stay_inside_the_block_and_extents_must_be_sane() {
        let outside = line(vec![glyph(0.0, 30.0, false)], 20.0, 0.0, 0.0);
        assert_eq!(
            outside.validate(&ctx()),
            Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
        );
        let negative = line(vec![glyph(0.0, 5.0, false)], 5.0, -1.0, 0.0);
        assert_eq!(
            negative.validate(&ctx()),
            Err(LineInvariantError::NonFiniteOrNegativeExtent)
        );
        let nan = line(vec![glyph(0.0, 5.0, false)], f32::NAN, 0.0, 0.0);
        assert_eq!(
            nan.validate(&ctx()),
            Err(LineInvariantError::NonFiniteOrNegativeExtent)
        );
    }

    #[test]
    fn errors_display_and_implement_std_error() {
        let e = LineInvariantError::BodyGlyphOutsideBlock { run: 1, glyph: 2 };
        let dynamic: &dyn std::error::Error = &e;
        assert_eq!(
            dynamic.to_string(),
            "body glyph 2 of run 1 lies outside the content of every slot"
        );
        let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(e);
        assert!(!boxed.to_string().is_empty());
    }

    fn no_hang() -> LineContext {
        LineContext {
            hang_mode: HangMode::NONE,
            ..ctx()
        }
    }

    /// A paragraph of `lines` laid out under the policy of `ctx`.
    fn paragraph(ctx: &LineContext, lines: Vec<PositionedLine>) -> PositionedParagraph {
        PositionedParagraph {
            policy: policy(ctx),
            base_direction: ctx.base_direction,
            truncated_range: None,
            total_block_size: 16.0,
            lines,
        }
    }

    fn caret(x: f32, offset: ByteIndex) -> CaretStop {
        CaretStop {
            x,
            offset,
            affinity: crate::middleend::CaretAffinity::Downstream,
            level: 0,
            line: 0,
        }
    }

    fn underline(x: f32, width: f32) -> DecorationLine {
        DecorationLine {
            kind: DecorationKind::Underline,
            byte_range: TextRange::new(0, 1),
            x,
            width,
            y_offset: 1.0,
            thickness: 1.0,
        }
    }

    #[test]
    fn caret_stops_must_be_finite_ordered_and_inside_the_line() {
        let base = || line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        let invalid = Err(LineInvariantError::InvalidCaretStop { index: 1 });
        let mut l = base();
        l.caret_stops = vec![caret(0.0, 0), caret(5.0, 4)];
        assert_eq!(l.validate(&ctx()), Ok(()));
        // Equal x (a bidi boundary has two stops) is in order.
        l.caret_stops = vec![caret(5.0, 4), caret(5.0, 2)];
        assert_eq!(l.validate(&ctx()), Ok(()));
        l.caret_stops = vec![caret(0.0, 0), caret(f32::NAN, 1)];
        assert_eq!(l.validate(&ctx()), invalid);
        l.caret_stops = vec![caret(5.0, 4), caret(1.0, 1)];
        assert_eq!(l.validate(&ctx()), invalid);
        l.caret_stops = vec![caret(0.0, 0), caret(5.0, 5)];
        assert_eq!(l.validate(&ctx()), invalid);
        l.caret_stops = vec![
            caret(0.0, 0),
            CaretStop {
                line: 1,
                ..caret(5.0, 4)
            },
        ];
        assert_eq!(l.validate(&ctx()), invalid);
        assert_eq!(
            l.validate(&LineContext {
                line_index: 1,
                ..ctx()
            }),
            Err(LineInvariantError::InvalidCaretStop { index: 0 })
        );
        l.caret_stops = vec![
            caret(0.0, 0),
            CaretStop {
                level: BIDI_MAX_DEPTH + 2,
                ..caret(5.0, 4)
            },
        ];
        assert_eq!(l.validate(&ctx()), invalid);
    }

    #[test]
    fn decorations_must_be_finite_nonnegative_and_inside_the_line() {
        let base = || line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        let invalid = Err(LineInvariantError::InvalidDecoration { index: 0 });
        let mut l = base();
        l.decorations = vec![underline(0.0, 5.0)];
        assert_eq!(l.validate(&ctx()), Ok(()));
        for bad in [
            underline(0.0, f32::INFINITY),
            underline(f32::NAN, 1.0),
            underline(0.0, -1.0),
            DecorationLine {
                thickness: -1.0,
                ..underline(0.0, 5.0)
            },
            DecorationLine {
                byte_range: TextRange::new(2, 9),
                ..underline(0.0, 5.0)
            },
        ] {
            l.decorations = vec![bad];
            assert_eq!(l.validate(&ctx()), invalid);
        }
    }

    #[test]
    fn overflowing_endpoints_are_rejected_before_comparison() {
        // Each coordinate is finite, but x + advance overflows to infinity.
        let l = line(vec![glyph(3.0e38, 3.0e38, false)], 5.0, 0.0, 0.0);
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::NonFiniteGlyph { run: 0, glyph: 0 })
        );
        let mut b = line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        b.visual_boxes.push(inline_box(3.0e38, 3.0e38));
        assert_eq!(
            b.validate(&ctx()),
            Err(LineInvariantError::InvalidInlineBox { index: 0 })
        );
        let mut t = line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        t.tabs.push(PositionedTab {
            byte_range: TextRange::new(1, 2),
            x: 3.0e38,
            width: 3.0e38,
        });
        assert_eq!(
            t.validate(&ctx()),
            Err(LineInvariantError::InvalidTab { index: 0 })
        );
    }

    #[test]
    fn negative_advance_is_judged_by_its_normalised_extent() {
        // Occupies [20, 25] although `x + advance` is 20: outside a 20-wide body.
        let l = line(vec![glyph(25.0, -5.0, false)], 20.0, 0.0, 0.0);
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
        );
        // Inside the body it is fine.
        let body = vec![
            glyph(0.0, 10.0, false),
            glyph(10.0, -5.0, false),
            glyph(10.0, 10.0, false),
        ];
        assert!(line(body, 20.0, 0.0, 0.0).is_valid(&ctx()));
    }

    #[test]
    fn synthesized_hyphen_with_no_range_is_not_trailing_whitespace() {
        // "abc" + generated hyphen whose range is empty at visible_byte_end.
        let mut l = line(
            vec![glyph(0.0, 15.0, false), glyph(15.0, 5.0, false)],
            20.0,
            0.0,
            0.0,
        );
        l.byte_range = TextRange::new(0, 4);
        l.visible_byte_end = 4;
        l.visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(0, 4));
        l.visual_runs[0].glyphs[1] = PositionedGlyph {
            is_synthesized: true,
            byte_range: None,
            ..glyph(15.0, 5.0, false)
        };
        assert_eq!(l.validate(&ctx()), Ok(()));
        // Ordinary text must own at least one source byte.
        l.visual_runs[0].glyphs[1].is_synthesized = false;
        l.visual_runs[0].glyphs[1].byte_range = Some(TextRange::empty_at(4));
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::InvalidGlyphSource { run: 0, glyph: 1 })
        );
    }

    #[test]
    fn trailing_whitespace_cannot_be_marked_hanging() {
        let mut l = line(
            vec![glyph(0.0, 20.0, false), trailing_space(20.0, 0.0)],
            20.0,
            0.0,
            0.0,
        );
        l.visible_byte_end = 3;
        l.visual_runs[0].glyphs[1].is_hanging = true;
        assert_eq!(
            l.validate(&no_hang()),
            Err(LineInvariantError::TrailingWhitespaceMarkedHanging { run: 0, glyph: 1 })
        );
    }

    #[test]
    fn a_hanging_glyph_with_negligible_overflow_is_rejected() {
        // Extent 1e-4 sits inside the tolerance, so `hanging_left` is 0.0.
        let l = line(
            vec![glyph(-1.0e-4, 1.0e-4, true), glyph(0.0, 20.0, false)],
            20.0,
            0.0,
            0.0,
        );
        let expected = Err(LineInvariantError::HangingGlyphInsideBlock { run: 0, glyph: 0 });
        assert_eq!(l.validate(&no_hang()), expected);
        assert_eq!(l.validate(&ctx()), expected);
    }

    #[test]
    fn paragraph_validation_uses_the_paragraphs_own_direction() {
        // Preserved space left of the body is valid only in an RTL paragraph.
        let mut l = line(
            vec![glyph(5.0, 20.0, false), trailing_space(0.0, 5.0)],
            20.0,
            0.0,
            0.0,
        );
        l.visible_byte_end = 3;
        l.visual_runs[0].glyphs[0].x = 0.0;
        l.visual_runs[0].glyphs[1].x = -5.0;
        // Screen order: the space, left of the body, comes first.
        l.visual_runs[0].glyphs.swap(0, 1);
        let rtl = LineContext {
            whitespace: HangingWhitespaceMode::Preserved,
            base_direction: BidiDirection::Rtl,
            ..no_hang()
        };
        let mut para = paragraph(&rtl, vec![l]);
        assert_eq!(para.validate_geometry(), Ok(()));
        para.base_direction = BidiDirection::Ltr;
        assert_eq!(
            para.validate_geometry(),
            Err((
                0,
                LineInvariantError::TrailingWhitespaceOnWrongSide { run: 0, glyph: 0 }
            ))
        );
    }

    // ---- wrap width (spec §2.6, Pass 3 (h)) and fragment hit-testing (spec §6.3) ----

    fn wrap(w: f32) -> LineContext {
        LineContext {
            wrap_width: w,
            ..ctx()
        }
    }

    /// Validation of a single-slot line against a context of wrap width `w`.
    trait ValidateAt {
        fn validate_at(&self, w: f32) -> Result<(), LineInvariantError>;
    }

    impl ValidateAt for PositionedLine {
        fn validate_at(&self, w: f32) -> Result<(), LineInvariantError> {
            let mut l = self.clone();
            l.slots[0].constraint.end = w;
            PositionedLine::validate(&l, &wrap(w))
        }
    }

    fn word_glyph(x: f32, advance: f32, bytes: (usize, usize)) -> PositionedGlyph {
        let mut g = glyph(x, advance, false);
        g.byte_range = Some(TextRange::new(bytes.0, bytes.1));
        g
    }

    #[test]
    fn a_line_must_fit_the_wrap_width() {
        // Two clusters, 30 wide: fits 30 (and within tolerance), not 29.
        let two = || {
            line(
                vec![
                    word_glyph(0.0, 10.0, (0, 1)),
                    word_glyph(10.0, 20.0, (1, 2)),
                ],
                30.0,
                0.0,
                0.0,
            )
        };
        assert!(two().validate_at(30.0).is_ok());
        assert!(two().validate_at(30.0 - 1e-4).is_ok());
        assert_eq!(
            two().validate_at(29.0),
            Err(LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 30.0,
                wrap_width: 29.0
            })
        );
        // An infinite wrap width imposes no limit.
        assert!(two().validate(&ctx()).is_ok());
    }

    #[test]
    fn a_single_wide_cluster_may_overflow() {
        // One cluster of two glyphs (base + mark share the byte range).
        let l = line(
            vec![
                word_glyph(0.0, 25.0, (0, 3)),
                word_glyph(25.0, 15.0, (0, 3)),
            ],
            40.0,
            0.0,
            0.0,
        );
        assert!(l.validate_at(30.0).is_ok());
        // Two different clusters are divisible, so they must wrap.
        let l = line(
            vec![
                word_glyph(0.0, 25.0, (0, 3)),
                word_glyph(25.0, 15.0, (3, 6)),
            ],
            40.0,
            0.0,
            0.0,
        );
        assert_eq!(
            l.validate_at(30.0),
            Err(LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 40.0,
                wrap_width: 30.0
            })
        );
        // A lone cluster whose extent is not the line width is not the overflow
        // state either (the line is wider than what it holds).
        let l = line(vec![word_glyph(0.0, 25.0, (0, 3))], 40.0, 0.0, 0.0);
        assert_eq!(
            l.validate_at(30.0),
            Err(LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 40.0,
                wrap_width: 30.0
            })
        );
    }

    #[test]
    fn a_single_wide_inline_box_may_overflow() {
        let object = |x: f32, width: f32, policy| PositionedInlineBox {
            baseline_align: BaselineAlignment::Auto,
            box_id: 1,
            byte_range: TextRange::new(0, 1),
            x,
            y: 0.0,
            width,
            ascent: 5.0,
            descent: 1.0,
            overflow_policy: policy,
        };
        let allow = BoxOverflowPolicy::AllowOverflow;
        // Exemption A: no visual runs, no tabs, one AllowOverflow box at x = 0
        // that is as wide as the line.
        let mut l = line(vec![], 50.0, 0.0, 0.0);
        l.byte_range = TextRange::new(0, 1);
        l.visible_byte_end = 1;
        l.visual_runs.clear();
        l.visual_boxes = vec![object(0.0, 50.0, allow)];
        assert!(l.validate_at(30.0).is_ok());
        // A rigid policy must fit: Clip and ScaleToFit boxes wider than the line
        // are rejected, by the box rule and then by the width rule.
        for policy in [BoxOverflowPolicy::Clip, BoxOverflowPolicy::ScaleToFit] {
            let mut rigid = l.clone();
            rigid.visual_boxes = vec![object(0.0, 50.0, policy)];
            assert_eq!(
                rigid.validate_at(30.0),
                Err(LineInvariantError::InlineBoxWiderThanWrapWidth { index: 0 })
            );
            // A rigid box that fits is fine.
            rigid.visual_boxes = vec![object(0.0, 30.0, policy)];
            set_width(&mut rigid, 30.0);
            assert!(rigid.validate_at(30.0).is_ok());
        }
        // Not when it shares the line with a glyph run, a tab or a second box.
        let mut with_run = l.clone();
        with_run.visual_runs =
            line(vec![word_glyph(0.0, 50.0, (0, 1))], 50.0, 0.0, 0.0).visual_runs;
        with_run.visual_runs[0].byte_range = with_run.byte_range;
        assert_eq!(
            with_run.validate_at(30.0),
            Err(LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 50.0,
                wrap_width: 30.0
            })
        );
        let mut with_tab = l.clone();
        with_tab.tabs = vec![PositionedTab {
            byte_range: TextRange::new(0, 1),
            x: 50.0,
            width: 0.0,
        }];
        assert_eq!(
            with_tab.validate_at(30.0),
            Err(LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 50.0,
                wrap_width: 30.0
            })
        );
        let mut two = l.clone();
        two.visual_boxes = vec![object(0.0, 25.0, allow), object(25.0, 25.0, allow)];
        assert_eq!(
            two.validate_at(30.0),
            Err(LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 50.0,
                wrap_width: 30.0
            })
        );
        // The box must start at 0 and be as wide as the line.
        let mut narrow = l.clone();
        narrow.visual_boxes = vec![object(0.0, 20.0, allow)];
        assert_eq!(
            narrow.validate_at(30.0),
            Err(LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 50.0,
                wrap_width: 30.0
            })
        );
        let mut shifted = l;
        shifted.visual_boxes = vec![object(5.0, 45.0, allow)];
        assert_eq!(
            shifted.validate_at(30.0),
            Err(LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 50.0,
                wrap_width: 30.0
            })
        );
    }

    #[test]
    fn padded_single_entities_may_overflow_inside_their_fragments() {
        let wide = |paddings: (f32, f32)| {
            // One 26-wide cluster between the paddings, in a chip over its bytes.
            let (left, right) = paddings;
            let mut l = line(
                vec![word_glyph(left, 26.0, (0, 3))],
                left + 26.0 + right,
                0.0,
                0.0,
            );
            l.span_fragments = vec![fragment(1, 0.0, left + 26.0 + right, (0, 3))];
            // Byte 3 is the line's trailing whitespace.
            l.visible_byte_end = 3;
            l
        };
        // 3 + 26 + 3 = 32 > 30: legal, the padding rides with the cluster.
        assert!(wide((3.0, 3.0)).validate_at(30.0).is_ok());
        assert!(wide((3.0, 0.0)).validate_at(30.0).is_ok());
        assert!(wide((0.0, 3.0)).validate_at(30.0).is_ok());
        // Nested fragments of the same bytes: all must wrap the entity.
        let mut nested = wide((3.0, 3.0));
        let mut inner = fragment(2, 3.0, 26.0, (0, 3));
        inner.is_first_fragment = true;
        nested.span_fragments.push(inner);
        assert!(nested.validate_at(30.0).is_ok());
        // Every padded fixture here is 3 + 26 + 3 = 32 wide.
        let too_wide = |l: &PositionedLine| {
            assert_eq!(
                l.validate_at(30.0),
                Err(LineInvariantError::LineWiderThanWrapWidth {
                    visual_width: 32.0,
                    wrap_width: 30.0
                })
            );
        };
        // A fragment may run on over the trailing whitespace ...
        let mut trailing = wide((3.0, 3.0));
        trailing.span_fragments[0].byte_range = TextRange::new(0, 4);
        trailing.span_fragments[0].span_range = TextRange::new(0, 4);
        assert!(trailing.validate_at(30.0).is_ok());
        // ... but not over a visible byte the entity does not own,
        let mut visible = trailing.clone();
        visible.visible_byte_end = 4;
        too_wide(&visible);
        // not start elsewhere or stop short of the entity,
        for bytes in [(1, 3), (0, 2)] {
            let mut other = wide((3.0, 3.0));
            other.span_fragments[0].byte_range = TextRange::new(bytes.0, bytes.1);
            other.span_fragments[0].span_range = TextRange::new(bytes.0, bytes.1);
            too_wide(&other);
        }
        // and not leave the line (Pass 3(g)).
        let mut beyond = wide((3.0, 3.0));
        beyond.span_fragments[0].byte_range = TextRange::new(0, 5);
        beyond.span_fragments[0].span_range = TextRange::new(0, 5);
        assert_eq!(
            beyond.validate_at(30.0),
            Err(LineInvariantError::SpanFragmentOutsideLine { index: 0 })
        );
        // Fragments that stop short of an edge of the body.
        let mut short = wide((3.0, 3.0));
        short.span_fragments = vec![fragment(1, 0.0, 29.0, (0, 3))];
        too_wide(&short);
        let mut late = wide((3.0, 3.0));
        late.span_fragments = vec![fragment(1, 3.0, 29.0, (0, 3))];
        too_wide(&late);
        // The padding is only a claim: the entity cannot be outside the body.
        // (Pass 3 rejects it earlier, as containment.)
        let mut outside = wide((3.0, 3.0));
        outside.visual_runs[0].glyphs[0].x = 10.0;
        assert_eq!(
            outside.validate_at(30.0),
            Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
        );
        // Unpadded, the width must still be exactly the entity's.
        let mut bare = line(vec![word_glyph(0.0, 26.0, (0, 3))], 32.0, 0.0, 0.0);
        too_wide(&bare);
        set_width(&mut bare, 26.0);
        assert!(bare.validate_at(30.0).is_ok());

        // Exemption A, padded: one AllowOverflow box in a chip over its bytes.
        let object = |x: f32, width: f32| PositionedInlineBox {
            baseline_align: BaselineAlignment::Auto,
            box_id: 1,
            byte_range: TextRange::new(0, 1),
            x,
            y: 0.0,
            width,
            ascent: 5.0,
            descent: 1.0,
            overflow_policy: BoxOverflowPolicy::AllowOverflow,
        };
        let mut l = line(vec![], 32.0, 0.0, 0.0);
        l.visual_runs.clear();
        l.byte_range = TextRange::new(0, 2);
        l.visible_byte_end = 1;
        l.visual_runs.clear();
        l.visual_boxes = vec![object(3.0, 26.0)];
        l.span_fragments = vec![fragment(1, 0.0, 32.0, (0, 1))];
        assert!(l.validate_at(30.0).is_ok());
        // Trailing whitespace inside the fragment is fine; stopping short is not.
        let mut trailing = l.clone();
        trailing.span_fragments[0].byte_range = TextRange::new(0, 2);
        trailing.span_fragments[0].span_range = TextRange::new(0, 2);
        assert!(trailing.validate_at(30.0).is_ok());
        let mut other = l.clone();
        other.span_fragments[0].byte_range = TextRange::new(0, 0);
        other.span_fragments[0].span_range = TextRange::new(0, 0);
        too_wide(&other);
        let mut short = l.clone();
        short.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 1))];
        too_wide(&short);
        // Without fragments the box must still start at 0.
        let mut bare = l;
        bare.span_fragments.clear();
        too_wide(&bare);
    }

    #[test]
    fn paragraph_validation_applies_the_wrap_width_to_every_line() {
        let two = line(
            vec![
                word_glyph(0.0, 10.0, (0, 1)),
                word_glyph(10.0, 20.0, (1, 2)),
            ],
            30.0,
            0.0,
            0.0,
        );
        let mut p = paragraph(&wrap(30.0), vec![two]);
        p.lines[0].slots[0].constraint.end = 30.0;
        assert!(p.validate_geometry().is_ok());
        p.policy.wrap_width = 10.0;
        p.lines[0].slots[0].constraint.end = 10.0;
        assert_eq!(
            p.validate_geometry(),
            Err((
                0,
                LineInvariantError::LineWiderThanWrapWidth {
                    visual_width: 30.0,
                    wrap_width: 10.0
                }
            ))
        );
    }

    #[test]
    fn fragment_bidi_level_is_range_checked() {
        let mut l = spanned_line();
        let mut f = fragment(1, 0.0, 30.0, (0, 4));
        f.bidi_level = BIDI_MAX_DEPTH + 1;
        l.span_fragments = vec![f];
        assert!(l.validate(&ctx()).is_ok());
        l.span_fragments[0].bidi_level = BIDI_MAX_DEPTH + 2;
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::InvalidSpanFragment { index: 0 })
        );
    }

    #[test]
    fn span_fragment_bytes_stay_inside_the_line() {
        let mut l = spanned_line();
        l.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 4))];
        assert!(l.validate(&ctx()).is_ok());
        // Beyond the line's bytes, or inverted.
        l.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 9))];
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::SpanFragmentOutsideLine { index: 0 })
        );
        // Starting before the line (a nonzero-start line).
        let mut later = spanned_line();
        later.byte_range = TextRange::new(1, 4);
        later.visual_runs[0].byte_range = TextRange::new(1, 4);
        later.visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(1, 2));
        later.span_fragments = vec![fragment(1, 0.0, 30.0, (1, 4))];
        assert!(later.validate(&ctx()).is_ok());
        later.span_fragments[0].byte_range = TextRange::new(0, 4);
        later.span_fragments[0].span_range = TextRange::new(0, 4);
        assert_eq!(
            later.validate(&ctx()),
            Err(LineInvariantError::SpanFragmentOutsideLine { index: 0 })
        );
        let mut inverted = fragment(1, 0.0, 30.0, (0, 4));
        inverted.byte_range = TextRange { start: 3, end: 1 };
        l.span_fragments = vec![inverted];
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::SpanFragmentOutsideLine { index: 0 })
        );
    }
}
