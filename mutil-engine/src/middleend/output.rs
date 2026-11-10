//! Middle-end output: positioned lines and glyphs (spec §2.6).

use super::{HangMode, HangingWhitespaceMode, LayoutOptions, LineClampPolicy, LineHeight};
use crate::frontend::BoxOverflowPolicy;
use crate::types::{
    BIDI_MAX_DEPTH, BidiDirection, BidiLevel, ByteIndex, FontMetrics, FontRef, TextRange,
};

/// One glyph placed on a line (spec §2.6).
///
/// `x` is **line-local**: measured from the line's left edge, so a line can be
/// moved without touching its glyphs. **[Design decision]** (spec silent).
/// It is the strict **pen advance position**, the cumulative advance along the
/// line, and not the drawing origin: the rasteriser draws the glyph at
/// `(line_origin_x + x + x_offset, baseline_y - y_offset)` (spec §2.6, geometry conventions). A
/// zero-advance combining mark with a negative `x_offset` at the start of a
/// line therefore has `x == 0.0`. `x` may still be **negative** for hanging
/// punctuation placed outside the left edge (§5 Stage 1.3), see
/// [`PositionedLine::hanging_left`]. Containment is judged on the pen interval
/// `[x, x + advance]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// Backend glyph index in the run's font.
    pub glyph_id: u32,
    /// Pen advance position from the line's left edge (**not** the drawing
    /// origin; see the type documentation).
    pub x: f32,
    /// Horizontal ink offset from the shaper. The drawing origin is
    /// `x + x_offset`.
    pub x_offset: f32,
    /// Shaper offset from the baseline, preserved unclamped.
    pub y_offset: f32,
    /// Advance width.
    pub advance: f32,
    /// Source bytes; absent if and only if this glyph is synthesized.
    pub byte_range: Option<TextRange>,
    /// `true` for the generated hyphen. Never copied to the clipboard (§6.6).
    /// (An actual U+00AD in the source is *not* synthesized.)
    pub is_synthesized: bool,
    /// `true` for punctuation hanging outside the text block (§5 Stage 1.3).
    /// May straddle the body edge; its normalized pen interval must overflow
    /// `[0, visual_width]` beyond tolerance. Only the overflow counts as hanging.
    pub is_hanging: bool,
}

/// Glyphs of one font and one bidi level, in screen order.
///
/// **[Design decision]** The spec names `VisualGlyphRun` (§2.6) without
/// defining it; it groups glyphs that share a font so a painter can bind one
/// face per run.
#[derive(Clone, Debug, PartialEq)]
pub struct VisualGlyphRun {
    /// Resolved glyph orientation.
    pub orientation: crate::types::GlyphOrientation,
    /// Requested size before fallback scaling.
    pub font_size: f32,
    /// Variable font instance coordinates.
    pub variations: smallvec::SmallVec<[crate::types::FontVariation; 2]>,
    /// Font and fallback scale shared by every glyph.
    pub font: FontRef,
    /// Bidi level shared by every glyph.
    pub bidi_level: BidiLevel,
    /// Source bytes covered by the run.
    pub byte_range: TextRange,
    /// Glyphs, screen left to right.
    pub glyphs: Vec<PositionedGlyph>,
}

/// Kind of decoration line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecorationKind {
    /// Underline.
    Underline,
    /// Strike-through.
    Strikethrough,
}

/// A decoration segment, merged across adjacent clusters and glue (§5 Stage 5).
/// Colour is not stored; the painter looks it up from `byte_range`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecorationLine {
    /// Underline or strike-through.
    pub kind: DecorationKind,
    /// Source bytes decorated.
    pub byte_range: TextRange,
    /// Left end, line-local.
    pub x: f32,
    /// Extent to the right of `x` (non-negative).
    pub width: f32,
    /// Offset from the baseline.
    pub y_offset: f32,
    /// Stroke thickness.
    pub thickness: f32,
}

/// An inline object placed on a line (spec §2.4 `InlineBox`, §2.6).
///
/// Glyph runs carry no objects, so this is where a painter and the hit-test
/// and selection code find them. **[Design decision]**; the spec left object
/// positions unrepresented.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedInlineBox {
    /// Alignment relative to the main line baseline.
    pub baseline_align: crate::types::BaselineAlignment,
    /// Caller-assigned identifier from `BoxNode::InlineObject::box_id`.
    pub box_id: u64,
    /// Source bytes the object stands for.
    pub byte_range: TextRange,
    /// Left edge, line-local. Lies within `[0, visual_width]`.
    pub x: f32,
    /// Vertical offset of the object's baseline from the line's baseline,
    /// y growing downwards (`0.0` = the object sits on the baseline).
    pub y: f32,
    /// Width (non-negative).
    pub width: f32,
    /// Extent above the object's baseline (non-negative).
    pub ascent: f32,
    /// Extent below the object's baseline (non-negative).
    pub descent: f32,
    /// The object's `BoxNode::InlineObject::overflow_policy`, so validation can
    /// tell a legitimate `AllowOverflow` from a `Clip` or `ScaleToFit` box that
    /// was not made to fit.
    pub overflow_policy: BoxOverflowPolicy,
}

/// A tab placed on a line (spec §2.4 `Tab`, §2.6).
///
/// A tab carries no glyph; this is where a painter (leaders, selection) and the
/// hit-test code find its extent. **[Design decision]**; the spec left tab
/// positions unrepresented. `width` is `next_tab_stop(x) - x`, resolved during
/// layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedTab {
    /// Source bytes of the tab character.
    pub byte_range: TextRange,
    /// Left edge, line-local. Lies within `[0, visual_width]`, except for a
    /// trailing tab under `Preserved` (trailing-whitespace rule).
    pub x: f32,
    /// Width (non-negative).
    pub width: f32,
}

/// The visual box of one line fragment of an inline decoration span (spec
/// §2.6), after bidi reordering.
///
/// Bidi reordering can split one span into several visual groups on a line;
/// each group is its own fragment. `is_first_fragment` marks the fragment
/// holding the span's logical start, `is_last_fragment` the one holding its
/// logical end (a span on one line, in one group, has both).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedSpanFragment {
    /// Identifier of the `InlineDecorationSpan`.
    pub span_id: u64,
    /// Source bytes of the span that this fragment covers.
    pub byte_range: TextRange,
    /// Left visual edge, line-local.
    pub x: f32,
    /// Visual width including the padding that applies (non-negative).
    pub width: f32,
    /// Extent above the baseline (non-negative).
    pub ascent: f32,
    /// Extent below the baseline (non-negative).
    pub descent: f32,
    /// `true` if the fragment contains the logical start of `span_id`.
    pub is_first_fragment: bool,
    /// `true` if the fragment contains the logical end of `span_id`.
    pub is_last_fragment: bool,
    /// UAX #9 level of the text in this fragment.
    pub bidi_level: BidiLevel,
}

/// Which side of an ambiguous boundary a caret belongs to (spec §6.1).
///
/// Flutter's definition: upstream leans toward the start of the string. At a
/// soft wrap, upstream is the end of the first line and downstream the start of
/// the second; affinity also resolves a bidi boundary. Explicit newlines are
/// unambiguous `[C1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaretAffinity {
    /// Leans toward the following text.
    #[default]
    Downstream,
    /// Leans toward the preceding text.
    Upstream,
}

/// One visual boundary on a line, in visual order (spec §6.1). Two stops sit at
/// each direction boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretStop {
    /// Line-local x position.
    pub x: f32,
    /// Logical byte offset at this stop.
    pub offset: ByteIndex,
    /// Affinity this stop represents.
    pub affinity: CaretAffinity,
    /// Bidi level of the slot.
    pub level: BidiLevel,
    /// Index into `PositionedParagraph::lines`.
    pub line: usize,
}

/// Visual fragment of an atomic edit span, independent of decorations.
///
/// Lives in the output layer because it is part of a [`PositionedLine`]'s
/// geometry; hit-testing behaviour is implemented in the interaction layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtomicFragment {
    /// Atomic span id (independent id space).
    pub atomic_id: u64,
    /// Entire atomic logical range.
    pub full_byte_range: TextRange,
    /// Source bytes on this visual fragment.
    pub fragment_byte_range: TextRange,
    /// Physical left edge, line-local.
    pub x: f32,
    /// Extent to the right of `x` (non-negative).
    pub width: f32,
    /// Resolved fragment direction.
    pub bidi_level: BidiLevel,
}

/// `true` when a fragment's geometry is finite and ordered, its ranges nest,
/// and its bidi level is representable.
fn atomic_fragment_measurements_are_valid(fragment: &AtomicFragment) -> bool {
    let geometry_ok = fragment.x.is_finite()
        && fragment.width.is_finite()
        && (fragment.x + fragment.width).is_finite()
        && fragment.width >= 0.0;
    let ranges_ok = fragment.full_byte_range.start <= fragment.full_byte_range.end
        && fragment.fragment_byte_range.start <= fragment.fragment_byte_range.end
        && fragment
            .full_byte_range
            .contains_range(fragment.fragment_byte_range);
    let level_ok = fragment.bidi_level <= BIDI_MAX_DEPTH + 1;
    geometry_ok && ranges_ok && level_ok
}

/// One laid-out line (spec §2.6).
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedLine {
    /// Actual content advance, one entry per slot of the line context.
    pub slots: smallvec::SmallVec<[PositionedSlot; 1]>,
    /// Final geometry snapshot; validation never calls the provider again.
    pub resolved_slot_constraints: smallvec::SmallVec<[super::InlineSlotConstraint; 2]>,
    /// Whether the suffix represents clamped source.
    pub is_ellipsized: bool,
    /// Interlinear glyph sequences.
    pub annotations: Vec<PositionedAnnotationRun>,
    /// Atomic editing geometry, independent of decoration ids.
    pub atomic_fragments: Vec<AtomicFragment>,
    /// Source bytes, **including** trailing whitespace.
    pub byte_range: TextRange,
    /// End of the last visible cluster (trailing whitespace excluded). Glyphs
    /// whose `byte_range` starts at or after it are trailing whitespace.
    pub visible_byte_end: ByteIndex,
    /// Baseline y within the paragraph.
    pub baseline_y: f32,
    /// Ascent of the line box.
    pub ascent: f32,
    /// Descent of the line box.
    pub descent: f32,
    /// Line box height.
    pub line_height: f32,
    /// Sum of occupied slot advances, excluding hanging punctuation and obstacles.
    pub visual_width: f32,
    /// Overflow beyond the first occupied slot's negative inline edge.
    pub hanging_left: f32,
    /// Overflow beyond the last occupied slot's positive inline edge.
    pub hanging_right: f32,
    /// Whether justification adjusted this line.
    pub is_justified: bool,
    /// Glyph runs, screen left to right.
    pub visual_runs: Vec<VisualGlyphRun>,
    /// Inline objects on this line, in logical order.
    pub visual_boxes: Vec<PositionedInlineBox>,
    /// Tabs on this line, in logical order.
    pub tabs: Vec<PositionedTab>,
    /// Visual boxes of inline decoration spans, screen left to right.
    pub span_fragments: Vec<PositionedSpanFragment>,
    /// Caret stops in visual order (§6.1).
    pub caret_stops: Vec<CaretStop>,
    /// Decorations on this line.
    pub decorations: Vec<DecorationLine>,
}

/// Absolute floor of the tolerance used by [`PositionedLine::validate`].
pub const LINE_GEOMETRY_ABS_EPSILON: f32 = 1e-3;
/// Relative part of that tolerance. `f32` carries about 7 significant digits,
/// so sums over hundreds of glyphs at coordinates in the tens of thousands
/// drift by more than the absolute floor; `1e-5` covers that drift.
pub const LINE_GEOMETRY_REL_EPSILON: f32 = 1e-5;

/// Mixed tolerance: `max(ABS, REL * max(|a|, |b|))`.
pub(super) fn tolerance(a: f32, b: f32) -> f32 {
    LINE_GEOMETRY_ABS_EPSILON.max(LINE_GEOMETRY_REL_EPSILON * a.abs().max(b.abs()))
}

fn approx_eq(a: f32, b: f32) -> bool {
    a == b || (a.is_finite() && b.is_finite() && (a - b).abs() <= tolerance(a, b))
}

/// Whether `[lo, hi]` lies in the body of `slot`, `[start, start + visual_width]`.
/// The one rule that decides both whether content belongs to a slot and
/// whether the slot counts as occupied.
fn slot_holds(slot: &PositionedSlot, lo: f32, hi: f32) -> bool {
    let local_eps = tolerance(slot.visual_width, 0.0);
    lo >= slot.start - local_eps && hi <= slot.start + slot.visual_width + local_eps
}

/// What a line is validated against.
///
/// Paragraph-wide fields plus `line_index`; use [`LineContext::for_line`] to
/// derive the context of each line from one paragraph context.
#[derive(Clone, Debug, PartialEq)]
pub struct LineContext {
    /// Active writing mode.
    pub writing_mode: crate::types::WritingMode,
    /// Available geometric intervals for this line.
    pub slot_constraints: smallvec::SmallVec<[super::InlineSlotConstraint; 2]>,
    /// The **resolved** strut: the raw `ParagraphAst::strut_metrics` after
    /// `LineHeight::resolve_strut`, half-leading included.
    pub strut: FontMetrics,
    /// The active line-height policy.
    pub line_height: LineHeight,
    /// The active trailing-whitespace mode.
    pub whitespace: HangingWhitespaceMode,
    /// The active hanging-punctuation mode.
    pub hang_mode: HangMode,
    /// The paragraph base direction. It decides which physical edge is the
    /// line start (hanging) and where trailing whitespace lies (UAX #9 L1).
    pub base_direction: BidiDirection,
    /// Index of the line within the paragraph; `0` is the first line, which
    /// `HangStartMode::FirstLine` refers to.
    pub line_index: usize,
    /// The paragraph's `LayoutConstraints::wrap_width`. A line wider than this
    /// is valid only in the single-entity overflow state (see
    /// [`PositionedLine::validate`]). [`Default`] is `f32::INFINITY` (no limit).
    pub wrap_width: f32,
}

impl Default for LineContext {
    fn default() -> Self {
        Self {
            strut: FontMetrics::default(),
            writing_mode: crate::types::WritingMode::HorizontalTb,
            slot_constraints: smallvec::smallvec![super::InlineSlotConstraint {
                start: 0.0,
                end: f32::INFINITY
            }],
            line_height: LineHeight::default(),
            whitespace: HangingWhitespaceMode::default(),
            hang_mode: HangMode::default(),
            base_direction: BidiDirection::default(),
            line_index: 0,
            wrap_width: f32::INFINITY,
        }
    }
}

impl LineContext {
    /// Builds the paragraph context: resolves the active strut from the **raw**
    /// primary-font metrics (`ParagraphAst::strut_metrics`) and the font size
    /// they were requested at (`ParagraphAst::strut_font_size`), using
    /// `options.line_height`. `wrap_width` is
    /// `LayoutConstraints::wrap_width`. `line_index` is 0.
    ///
    /// The context is a `HorizontalTb` one with the **horizontal** strut and a
    /// single slot `[0, wrap_width]`. A vertical caller must set `writing_mode`
    /// and re-resolve the strut with `LineHeight::resolve_strut_for_mode`, or
    /// use the context returned by
    /// [`check_layout_inputs`](super::check_layout_inputs), which does both.
    #[must_use]
    pub fn from_options(
        raw_strut: &FontMetrics,
        strut_font_size: f32,
        base_direction: BidiDirection,
        options: &LayoutOptions,
        wrap_width: f32,
    ) -> Self {
        Self {
            strut: options
                .line_height
                .resolve_strut(raw_strut, strut_font_size),
            line_height: options.line_height,
            whitespace: options.whitespace,
            hang_mode: options.hang_mode,
            base_direction,
            line_index: 0,
            wrap_width,
            writing_mode: crate::types::WritingMode::HorizontalTb,
            slot_constraints: smallvec::smallvec![super::InlineSlotConstraint {
                start: 0.0,
                end: wrap_width
            }],
        }
    }

    /// Replaces the available slots and unobstructed reference width.
    pub fn with_slots(
        mut self,
        slots: smallvec::SmallVec<[super::InlineSlotConstraint; 2]>,
        max_unobstructed_width: f32,
    ) -> Self {
        self.slot_constraints = slots;
        self.wrap_width = max_unobstructed_width;
        self
    }
    /// The same context for line `line_index`.
    #[must_use]
    pub fn for_line(self, line_index: usize) -> Self {
        Self { line_index, ..self }
    }
}

/// Which edge a hanging extent belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HangSide {
    /// Physical left edge.
    Left,
    /// Physical right edge.
    Right,
}

impl core::fmt::Display for HangSide {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Left => "left",
            Self::Right => "right",
        })
    }
}

/// A violated [`PositionedLine`] invariant.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LineInvariantError {
    /// Invalid or inconsistent slots, capacity, or conserved total width.
    InvalidSlots,
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
    /// An annotation run has invalid metrics, no glyphs, a duplicate id, an
    /// overlapping base on its side, or a base or glyphs outside the line's
    /// geometry.
    InvalidAnnotation {
        /// Index into `annotations`.
        index: usize,
    },
    /// Invalid paragraph coverage, truncation, or block-axis envelope.
    InvalidParagraph,
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
    /// An atomic fragment's geometry, byte ranges or bidi level is invalid.
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
    /// `[0, visual_width]`.
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
    /// An inline box lies outside `[0, visual_width]`.
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
    /// A span fragment lies outside `[0, visual_width]`.
    SpanFragmentOutsideBlock {
        /// Index into `span_fragments`.
        index: usize,
    },
    /// A span fragment's `byte_range` is inverted or outside the line's.
    SpanFragmentOutsideLine {
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
    /// A slot's content is wider than the slot's own capacity although it is
    /// not a single overflowing entity, and the line as a whole still fits
    /// `ctx.wrap_width`.
    SlotOverflow {
        /// Index into `slots`.
        slot: usize,
        /// The slot's `visual_width`.
        visual_width: f32,
        /// The slot's `constraint_width`.
        capacity: f32,
    },
    /// A tab has a non-finite coordinate, a negative width, or empty bytes or
    /// bytes outside the line.
    InvalidTab {
        /// Index into `tabs`.
        index: usize,
    },
    /// A body tab lies outside `[0, visual_width]`.
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
    /// the line's bytes or belongs to another line, or has an unrepresentable
    /// bidi level.
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
}

impl PositionedLine {
    /// Validates in three passes: finite measurements and source ownership;
    /// line envelope, slot alignment and conserved widths; then per-slot body,
    /// annotation, hanging and preserved-whitespace containment.
    ///
    /// `slots` holds exactly one entry per slot of the supplied context and
    /// matches any stored geometry snapshot.
    /// Only a single full-capacity slot may use the indivisible-entity overflow
    /// exemption. Synthesized ellipsis sequences may accompany that entity.
    /// Geometry describes pen advances, not glyph ink bounds; zero-advance
    /// combining marks retain their existing containment exemption.
    ///
    /// # Errors
    /// The first violated invariant.
    pub fn validate(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        self.check_finite()?;
        self.check_envelope(ctx)?;
        self.check_sources_and_annotations(ctx)?;
        self.check_slots(ctx)?;
        if self.byte_range.is_empty() && !self.is_ellipsized {
            return Ok(());
        }
        self.check_containment_and_hanging(ctx)
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
        if let Some(index) = self
            .atomic_fragments
            .iter()
            .position(|fragment| !atomic_fragment_measurements_are_valid(fragment))
        {
            return Err(E::InvalidAtomicFragment { index });
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

    /// Pass 2: byte range, empty-line state, line height.
    fn check_envelope(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let range = self.byte_range;
        if !(range.start <= self.visible_byte_end && self.visible_byte_end <= range.end) {
            return Err(E::VisibleEndOutsideRange);
        }
        // `full_byte_range` may span the whole wrapped atomic span; only the
        // bytes on this visual fragment must lie inside the line.
        if let Some(index) = self
            .atomic_fragments
            .iter()
            .position(|f| !range.contains_range(f.fragment_byte_range))
        {
            return Err(E::InvalidAtomicFragment { index });
        }
        let expected = ctx.strut.natural_line_height();
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
        if range.is_empty() && self.is_ellipsized {
            // A clamp may remove every base cluster; only its synthetic suffix remains.
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
                return Err(E::InvalidAnnotation { index: 0 });
            }
            for (run, r) in self.visual_runs.iter().enumerate() {
                if let Some(glyph) = r.glyphs.iter().position(|g| !g.is_synthesized) {
                    return Err(E::InvalidGlyphSource { run, glyph });
                }
            }
        }
        if range.is_empty() && !self.is_ellipsized {
            if !self.annotations.is_empty() {
                return Err(E::InvalidAnnotation { index: 0 });
            }
            if !self.visual_runs.is_empty() {
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
            if !approx_eq(self.line_height, expected) {
                return Err(E::EmptyLineHeightMismatch {
                    expected,
                    actual: self.line_height,
                });
            }
            if self.visual_width != 0.0 || self.hanging_left != 0.0 || self.hanging_right != 0.0 {
                return Err(E::EmptyLineHasExtent);
            }
            // A fixed line box is the strut's even when the line is empty.
            if matches!(ctx.line_height, LineHeight::Fixed(_)) {
                self.check_fixed_metrics(ctx)?;
            }
            return Ok(());
        }
        match ctx.line_height {
            LineHeight::Fixed(_) => {
                if !approx_eq(self.line_height, expected) {
                    return Err(E::LineHeightMismatch {
                        expected,
                        actual: self.line_height,
                    });
                }
                self.check_fixed_metrics(ctx)?;
            }
            LineHeight::Normal => {
                if self.line_height < expected - tolerance(self.line_height, expected) {
                    return Err(E::LineHeightBelowStrut {
                        minimum: expected,
                        actual: self.line_height,
                    });
                }
            }
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

    /// Pass 3: containment of glyphs, boxes and tabs; hanging; trailing
    /// whitespace.
    fn check_containment_and_hanging(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let slots = &self.slots;
        let fits = slot_holds;
        let occupied: Vec<_> = slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| {
                slot.visual_width > 0.0
                    || self.visual_runs.iter().flat_map(|r| &r.glyphs).any(|g| {
                        !g.is_hanging
                            && g.byte_range
                                .is_some_and(|r| r.start < self.visible_byte_end)
                            && fits(slot, g.x.min(g.x + g.advance), g.x.max(g.x + g.advance))
                    })
                    || self
                        .visual_boxes
                        .iter()
                        .any(|b| fits(slot, b.x, b.x + b.width))
                    || self.tabs.iter().any(|t| {
                        t.byte_range.start < self.visible_byte_end && fits(slot, t.x, t.x + t.width)
                    })
            })
            .collect();
        let Some(&(first_index, first)) = occupied.first() else {
            if self.visual_runs.iter().all(|r| r.glyphs.is_empty())
                && self.visual_boxes.is_empty()
                && self.tabs.is_empty()
                && self.span_fragments.is_empty()
                && self.atomic_fragments.is_empty()
                && self.annotations.is_empty()
                && self.hanging_left == 0.0
                && self.hanging_right == 0.0
            {
                return Ok(());
            }
            return Err(E::InvalidSlots);
        };
        let &(last_index, last) = occupied.last().unwrap();
        let left = first.start;
        let right = last.start + last.visual_width;
        let eps = tolerance(self.visual_width, 0.0);
        let inside = |lo: f32, hi: f32| slots.iter().any(|s| fits(s, lo, hi));
        let external_clear = |lo: f32, hi: f32| {
            // Outer whitespace/hanging cannot occupy any internal obstacle or unused slot.
            (lo >= left - eps || first_index == 0)
                && (hi <= right + eps || last_index + 1 == slots.len())
        };
        let far = |lo: f32, hi: f32| {
            external_clear(lo, hi)
                && match ctx.base_direction {
                    BidiDirection::Ltr => lo >= right - eps,
                    BidiDirection::Rtl => hi <= left + eps,
                }
        };
        let collapsible = ctx.whitespace == HangingWhitespaceMode::Collapsible;
        let (mut hanging_left, mut hanging_right) = (0.0_f32, 0.0_f32);
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let (lo, hi) = (g.x.min(g.x + g.advance), g.x.max(g.x + g.advance));
                let trailing = !g.is_synthesized
                    && g.byte_range
                        .is_some_and(|b| b.start >= self.visible_byte_end);
                if trailing {
                    if g.is_hanging {
                        return Err(E::TrailingWhitespaceMarkedHanging { run, glyph });
                    }
                    if collapsible && g.advance != 0.0 {
                        return Err(E::TrailingWhitespaceNotCollapsed { run, glyph });
                    }
                    if !collapsible && g.advance != 0.0 && !far(lo, hi) {
                        return Err(E::TrailingWhitespaceOnWrongSide { run, glyph });
                    }
                } else if g.advance == 0.0 {
                    continue;
                } else if g.is_hanging {
                    let l = (left - lo).max(0.0);
                    let h = (hi - right).max(0.0);
                    if l <= eps && h <= eps {
                        return Err(E::HangingGlyphInsideBlock { run, glyph });
                    }
                    if !external_clear(lo, hi)
                        || !((l > eps && hi <= first.start + first.visual_width + eps)
                            || (h > eps && lo >= last.start - eps))
                    {
                        return Err(E::BodyGlyphOutsideBlock { run, glyph });
                    }
                    hanging_left = hanging_left.max(l);
                    hanging_right = hanging_right.max(h);
                } else if !inside(lo, hi) {
                    return Err(E::BodyGlyphOutsideBlock { run, glyph });
                }
            }
        }
        for (side, expected, actual) in [
            (HangSide::Left, hanging_left, self.hanging_left),
            (HangSide::Right, hanging_right, self.hanging_right),
        ] {
            if !approx_eq(expected, actual) || (actual > 0.0 && expected == 0.0) {
                return Err(E::HangingExtentMismatch {
                    side,
                    expected,
                    actual,
                });
            }
            if (expected > 0.0 || actual > 0.0)
                && !ctx
                    .hang_mode
                    .allows(side, ctx.base_direction, ctx.line_index == 0)
            {
                return Err(E::HangingNotAllowed { side });
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
                if !collapsible && t.width != 0.0 && !far(t.x, t.x + t.width) {
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
        for (index, ann) in self.annotations.iter().enumerate() {
            // The base is the envelope of the non-synthesized body glyphs that
            // intersect its source range. Glue and other glyph-less nodes may
            // sit inside the base, so contiguous byte coverage is deliberately
            // not required. Every intersecting glyph must belong wholly to the
            // base and to one slot; a glyph straddling either boundary is a
            // source-ownership error, not an invitation to clip it.
            let mut base = Vec::new();
            for run in &self.visual_runs {
                for g in &run.glyphs {
                    let Some(range) = g.byte_range else {
                        continue;
                    };
                    if range.start < ann.base_byte_range.end
                        && range.end > ann.base_byte_range.start
                    {
                        if !ann.base_byte_range.contains_range(range) {
                            return Err(E::InvalidAnnotation { index });
                        }
                        base.push(g);
                    }
                }
            }
            if base.is_empty() {
                return Err(E::InvalidAnnotation { index });
            }
            let slot = slots
                .iter()
                .find(|s| {
                    base.iter().all(|g| {
                        let end = g.x + g.advance;
                        g.x.min(end) >= s.start - eps
                            && g.x.max(end) <= s.start + s.visual_width + eps
                    })
                })
                .ok_or(E::InvalidAnnotation { index })?;
            let base_lo = base
                .iter()
                .map(|g| g.x.min(g.x + g.advance))
                .fold(f32::INFINITY, f32::min);
            let base_hi = base
                .iter()
                .map(|g| g.x.max(g.x + g.advance))
                .fold(f32::NEG_INFINITY, f32::max);
            let lo = slot.start.max(base_lo - ann.max_overhang);
            let hi = (slot.start + slot.visual_width).min(base_hi + ann.max_overhang);
            if ann
                .glyphs
                .iter()
                .any(|g| g.x.min(g.x + g.advance) < lo - eps || g.x.max(g.x + g.advance) > hi + eps)
            {
                return Err(E::InvalidAnnotation { index });
            }
            let (base_side, placed) = match ann.side {
                crate::frontend::AnnotationSide::Over => {
                    (ann.base_over, ann.baseline_offset >= ann.base_over - eps)
                }
                crate::frontend::AnnotationSide::Under => {
                    (ann.base_under, ann.baseline_offset <= -ann.base_under + eps)
                }
            };
            if !placed {
                return Err(E::InvalidAnnotation { index });
            }
            if matches!(ctx.line_height, LineHeight::Normal) {
                let required = base_side + ann.block_extent;
                let (have, strut) = match ann.side {
                    crate::frontend::AnnotationSide::Over => (self.ascent, ctx.strut.ascent),
                    crate::frontend::AnnotationSide::Under => (self.descent, ctx.strut.descent),
                };
                if have + eps < required.max(strut) {
                    return Err(E::InvalidAnnotation { index });
                }
            }
        }
        self.check_span_fragments()?;
        for (index, slot) in slots.iter().enumerate() {
            let eps = tolerance(slot.visual_width, 0.0);
            if slot.visual_width > slot.constraint_width + eps
                && !(slots.len() == 1
                    && slot.constraint_width >= ctx.wrap_width - eps
                    && self.is_single_entity_overflow(slot.start))
            {
                // The whole line is wider than the container, or only this
                // slot is wider than its own capacity.
                return Err(
                    if self.visual_width > ctx.wrap_width + tolerance(self.visual_width, 0.0) {
                        E::LineWiderThanWrapWidth {
                            visual_width: self.visual_width,
                            wrap_width: ctx.wrap_width,
                        }
                    } else {
                        E::SlotOverflow {
                            slot: index,
                            visual_width: slot.visual_width,
                            capacity: slot.constraint_width,
                        }
                    },
                );
            }
        }
        self.check_slot_spans(slots)
    }

    /// Each slot's `visual_width` is the geometric span of the content placed in
    /// it: `max(right edge) - min(left edge)` over the body glyphs (pen `x` to
    /// `x + advance`), inline boxes, tabs and span fragments (which carry the
    /// padding of a decoration span, advance that no glyph covers) that fit the
    /// slot, `0` for a slot without content. A slot's width is not its capacity
    /// (an unjustified or last line keeps its slack), but it is exactly what
    /// the slot holds.
    ///
    /// Not content: trailing whitespace and trailing tabs (they lie outside the
    /// body under `Preserved` and have no width under `Collapsible`),
    /// zero-advance glyphs (a pen position, not an extent) and hanging
    /// punctuation. A hanging glyph counts only through the part of it that lies
    /// inside the slot's body, so a mark that straddles the edge is content of
    /// the slot it hangs from and one entirely outside is not.
    fn check_slot_spans(&self, slots: &[PositionedSlot]) -> Result<(), LineInvariantError> {
        let mut spans: Vec<Option<(f32, f32)>> = vec![None; slots.len()];
        let mut place = |lo: f32, hi: f32| {
            if let Some(i) = slots.iter().position(|s| slot_holds(s, lo, hi)) {
                let (a, b) = spans[i].get_or_insert((lo, hi));
                *a = a.min(lo);
                *b = b.max(hi);
            }
        };
        for g in self.visual_runs.iter().flat_map(|r| &r.glyphs) {
            let (lo, hi) = (g.x.min(g.x + g.advance), g.x.max(g.x + g.advance));
            let trailing = !g.is_synthesized
                && g.byte_range
                    .is_some_and(|b| b.start >= self.visible_byte_end);
            if trailing || g.advance == 0.0 {
                continue;
            }
            if g.is_hanging {
                for s in slots {
                    let (lo, hi) = (lo.max(s.start), hi.min(s.start + s.visual_width));
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
            let width = span.map_or(0.0, |(lo, hi)| hi - lo);
            if !approx_eq(width, slot.visual_width) {
                return Err(LineInvariantError::InvalidSlots);
            }
        }
        Ok(())
    }

    /// The single-entity overflow exemptions (spec §2.6, Pass 3(h)).
    ///
    /// **A:** nothing but one `AllowOverflow` box (no visual runs, no tabs).
    /// **B:** nothing but the glyphs of one indivisible cluster (a shared
    /// non-empty `byte_range`; no boxes, no tabs). The entity spans
    /// `[entity_start_x, entity_end_x]` and is `entity_width` wide.
    ///
    /// * No span fragments: the entity starts at `0` and `visual_width` equals
    ///   `entity_width`.
    /// * Span fragments: they are the padding around the one entity. Every
    ///   fragment starts at the entity's start and ends at or after its end;
    ///   the entity ends at or after `visible_byte_end`, so the extra bytes lie
    ///   in the trailing whitespace (`check_span_fragments` keeps fragments
    ///   inside the line). The entity lies inside the body and the fragments
    ///   reach both edges of the body (`min x == 0`, `max end ==
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

    /// Span containment (spec §2.6, Pass 3 (g)): fragments inside the body, and
    /// every glyph or inline box that belongs to a span inside one of its
    /// fragments.
    fn check_span_fragments(&self) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        if self.span_fragments.is_empty() {
            return Ok(());
        }
        for (index, f) in self.span_fragments.iter().enumerate() {
            // The fragment covers part of the span on this line: a well-formed
            // range inside the line's own bytes.
            if f.byte_range.start > f.byte_range.end
                || f.byte_range.start < self.byte_range.start
                || f.byte_range.end > self.byte_range.end
            {
                return Err(E::SpanFragmentOutsideLine { index });
            }
        }
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let Some(range) = g.byte_range.filter(|b| !b.is_empty()) else {
                    continue;
                };
                let trailing = range.start >= self.visible_byte_end;
                if g.is_hanging || g.is_synthesized || trailing {
                    continue;
                }
                let (lo, hi) = (g.x.min(g.x + g.advance), g.x.max(g.x + g.advance));
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
            if b.byte_range.is_empty() {
                continue;
            }
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

    /// The extreme caret stop towards the physical `side` (spec §6.3 rule 3):
    /// where a click beyond that edge of the line's visual content converges.
    ///
    /// `caret_stops` are in visual order, so equal-`x` ties (a bidi boundary
    /// has two stops at one `x`) resolve by that order: the first for `Left`,
    /// the last for `Right`. `None` if the line has no stops.
    ///
    /// When the content at that edge flows in the paragraph's base direction the
    /// stop is `(visible_byte_end, Upstream)`; when it is written in the
    /// opposite direction the stop is the outer edge of that text, never
    /// `visible_byte_end` as a blanket rule (which would jump the caret to the
    /// middle of the line).
    #[must_use]
    pub fn extreme_caret_stop(&self, side: HangSide) -> Option<&CaretStop> {
        let mut best: Option<&CaretStop> = None;
        for stop in &self.caret_stops {
            best = match (best, side) {
                (None, _) => Some(stop),
                (Some(b), HangSide::Left) if stop.x < b.x => Some(stop),
                (Some(b), HangSide::Right) if stop.x >= b.x => Some(stop),
                (b, _) => b,
            };
        }
        best
    }

    /// `true` when [`PositionedLine::validate`] succeeds.
    #[must_use]
    pub fn is_valid(&self, ctx: &LineContext) -> bool {
        self.validate(ctx).is_ok()
    }
}

/// Positioned lines in canonical flow space, with paragraph-local physical mapping.
/// The lines and `total_block_size` are the paragraph's extent; block transforms
/// and validation use `total_block_size` as the authoritative block extent.
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedParagraph {
    /// Paragraph-local physical mapping.
    pub writing_mode: crate::types::WritingMode,
    /// Unrendered logical tail, when clamped.
    pub truncated_range: Option<TextRange>,
    /// Final block-axis extent including clearance.
    pub total_block_size: f32,
    /// Paragraph base direction, copied from the AST.
    pub base_direction: BidiDirection,
    /// Lines top to bottom.
    pub lines: Vec<PositionedLine>,
}

impl PositionedParagraph {
    /// Validates every line against `ctx`, giving each its own `line_index`
    /// (so `HangStartMode::FirstLine` applies to line 0 only) and this
    /// paragraph's `base_direction`, whatever direction `ctx` carries.
    ///
    /// # Errors
    /// The index of the first invalid line and its violation.
    pub fn validate_geometry(&self, ctx: &LineContext) -> Result<(), (usize, LineInvariantError)> {
        if self.lines.is_empty()
            || !self.total_block_size.is_finite()
            || self.total_block_size < 0.0
        {
            return Err((0, LineInvariantError::InvalidParagraph));
        }
        let mut previous_bottom = 0.0;
        for (i, line) in self.lines.iter().enumerate() {
            let mut line_ctx = ctx.clone().for_line(i);
            line_ctx.base_direction = self.base_direction;
            line_ctx.writing_mode = self.writing_mode;
            if !line.resolved_slot_constraints.is_empty() {
                line_ctx.slot_constraints = line.resolved_slot_constraints.clone();
            }
            line.validate(&line_ctx).map_err(|e| (i, e))?;
            let (top, bottom) =
                self.writing_mode
                    .line_bounds(line.baseline_y, line.ascent, line.descent);
            let eps = tolerance(line.line_height, 0.0);
            if !top.is_finite()
                || !bottom.is_finite()
                || top < previous_bottom - eps
                || bottom < top
                || line.ascent + line.descent > line.line_height + eps
            {
                return Err((i, LineInvariantError::InvalidParagraph));
            }
            previous_bottom = bottom;
        }
        if self.total_block_size < previous_bottom - tolerance(previous_bottom, 0.0) {
            return Err((self.lines.len() - 1, LineInvariantError::InvalidParagraph));
        }
        Ok(())
    }

    /// Validates source coverage, clamp conservation (against `clamp`, the
    /// active [`LayoutOptions::clamp`]) and annotation id uniqueness in addition
    /// to geometry.
    ///
    /// # Errors
    /// First invalid line, or the final line for a coverage/truncation error.
    pub fn validate(
        &self,
        text: &str,
        clamp: Option<&LineClampPolicy>,
        ctx: &LineContext,
    ) -> Result<(), (usize, LineInvariantError)> {
        self.validate_geometry(ctx)?;
        let mut end = 0;
        let last = self.lines.len() - 1;
        let mut annotation_ids = std::collections::HashSet::new();
        for (i, line) in self.lines.iter().enumerate() {
            if line.byte_range.start != end
                || line.byte_range.end > text.len()
                || !text.is_char_boundary(line.byte_range.start)
                || !text.is_char_boundary(line.byte_range.end)
                || (line.is_ellipsized && (i != last || self.truncated_range.is_none()))
            {
                return Err((i, LineInvariantError::InvalidParagraph));
            }
            if !line
                .annotations
                .iter()
                .all(|a| annotation_ids.insert(a.annotation_id))
            {
                return Err((i, LineInvariantError::InvalidParagraph));
            }
            end = line.byte_range.end;
        }
        let valid = match self.truncated_range {
            None => end == text.len() && !self.lines[last].is_ellipsized,
            Some(tail) => {
                clamp.is_some_and(|c| c.is_valid())
                    && tail.start == end
                    && tail.end == text.len()
                    && tail.start < tail.end
                    && self.lines[last].is_ellipsized
            }
        };
        if !valid {
            return Err((last, LineInvariantError::InvalidParagraph));
        }
        Ok(())
    }

    /// A paragraph with no lines and zero size.
    #[must_use]
    pub fn empty(base_direction: BidiDirection) -> Self {
        Self {
            base_direction,
            writing_mode: crate::types::WritingMode::HorizontalTb,
            truncated_range: None,
            total_block_size: 0.0,
            lines: Vec::new(),
        }
    }
}

/// Actual content within an available geometric slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedSlot {
    /// Finite inline start.
    pub start: f32,
    /// Finite nonnegative content advance.
    pub visual_width: f32,
    /// Positive capacity, possibly infinite.
    pub constraint_width: f32,
}
/// One independently shaped annotation attached to base text in one slot.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionedAnnotationRun {
    /// Source annotation identifier.
    pub annotation_id: u64,
    /// Nonempty visible base range.
    pub base_byte_range: TextRange,
    /// Physical line-over or line-under side.
    pub side: crate::frontend::AnnotationSide,
    /// Font face and fallback scale.
    pub font: FontRef,
    /// Positive finite size.
    pub font_size: f32,
    /// Finite variation coordinates.
    pub variations: smallvec::SmallVec<[crate::types::FontVariation; 2]>,
    /// Resolved orientation.
    pub orientation: crate::types::GlyphOrientation,
    /// Finite baseline offset toward line-over.
    pub baseline_offset: f32,
    /// Maximum finite nonnegative overhang beyond the base.
    pub max_overhang: f32,
    /// Declared additional block extent, including gap.
    pub block_extent: f32,
    /// Finite nonnegative extent of the annotated base content above the
    /// baseline (toward line-over): the largest over-side extent of the base's
    /// glyphs and inline boxes, as the producer measured it.
    ///
    /// An `Over` annotation sits at `baseline_offset >= base_over` and needs
    /// `base_over + block_extent` of ascent under [`LineHeight::Normal`].
    pub base_over: f32,
    /// Finite nonnegative extent of the annotated base content below the
    /// baseline (toward line-under); the mirror of `base_over`. An `Under`
    /// annotation sits at `baseline_offset <= -base_under` and needs
    /// `base_under + block_extent` of descent under [`LineHeight::Normal`].
    pub base_under: f32,
    /// Glyphs shaped from the annotation's own text; they carry no body byte
    /// range, so they are marked synthesized.
    pub glyphs: Vec<PositionedGlyph>,
}

impl PositionedLine {
    fn check_slots(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError::InvalidSlots;
        if ctx.wrap_width.is_nan()
            || ctx.wrap_width <= 0.0
            || ctx.slot_constraints.is_empty()
            || ctx
                .slot_constraints
                .iter()
                .any(|s| !s.is_valid() || !s.lies_in_envelope(ctx.wrap_width))
            || ctx
                .slot_constraints
                .windows(2)
                .any(|s| s[0].start >= s[1].start || s[0].end > s[1].start)
        {
            return Err(InvalidSlots);
        }
        if !self.resolved_slot_constraints.is_empty()
            && self.resolved_slot_constraints != ctx.slot_constraints
        {
            return Err(InvalidSlots);
        }
        if self.slots.len() != ctx.slot_constraints.len() {
            return Err(InvalidSlots);
        }
        for s in &self.slots {
            if !s.start.is_finite()
                || !s.visual_width.is_finite()
                || s.visual_width < 0.0
                || s.constraint_width.is_nan()
                || s.constraint_width <= 0.0
                || !(s.start + s.visual_width).is_finite()
            {
                return Err(InvalidSlots);
            }
        }
        let eps = tolerance(self.visual_width, 0.0);
        for (s, c) in self.slots.iter().zip(&ctx.slot_constraints) {
            let width_eq =
                s.constraint_width == c.width() || approx_eq(s.constraint_width, c.width());
            if !approx_eq(s.start - c.start, 0.0) || !width_eq {
                return Err(InvalidSlots);
            }
        }
        let sum: f32 = self.slots.iter().map(|s| s.visual_width).sum();
        if !sum.is_finite()
            || !approx_eq(sum, self.visual_width)
            || self
                .slots
                .windows(2)
                .any(|s| s[0].start + s[0].visual_width > s[1].start + eps)
        {
            return Err(InvalidSlots);
        }
        Ok(())
    }

    /// Source ownership and annotation validity: every run, glyph, box, tab,
    /// decoration, caret stop and annotation refers to bytes of this line.
    fn check_sources_and_annotations(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
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
            if r.glyphs.is_empty()
                || r.byte_range.start > r.byte_range.end
                || !self.byte_range.contains_range(r.byte_range)
            {
                return Err(E::InvalidVisualRun { run });
            }
        }
        for (index, b) in self.visual_boxes.iter().enumerate() {
            if !b.baseline_align.is_valid() || !self.owns_bytes(b.byte_range) {
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
        for (index, c) in self.caret_stops.iter().enumerate() {
            let in_order = index == 0 || {
                let previous = self.caret_stops[index - 1].x;
                c.x >= previous - tolerance(previous, c.x)
            };
            if !in_order
                || !(self.byte_range.start..=self.byte_range.end).contains(&c.offset)
                || c.line != ctx.line_index
                || c.level > BIDI_MAX_DEPTH + 1
            {
                return Err(E::InvalidCaretStop { index });
            }
        }
        for (index, ann) in self.annotations.iter().enumerate() {
            if !ann.font_size.is_finite()
                || ann.font_size <= 0.0
                || !ann.font.scale.is_finite()
                || ann.font.scale <= 0.0
                || !ann.baseline_offset.is_finite()
                || !ann.max_overhang.is_finite()
                || ann.max_overhang < 0.0
                || !ann.block_extent.is_finite()
                || ann.block_extent < 0.0
                || !ann.base_over.is_finite()
                || ann.base_over < 0.0
                || !ann.base_under.is_finite()
                || ann.base_under < 0.0
                || ann.variations.iter().any(|v| !v.value().is_finite())
                || !ann.orientation.is_valid(ctx.writing_mode, 0)
                || ann.base_byte_range.is_empty()
                || ann.base_byte_range.start < self.byte_range.start
                || ann.base_byte_range.end > self.visible_byte_end
                || ann.base_byte_range.start > ann.base_byte_range.end
                || ann.glyphs.is_empty()
                || ann.glyphs.iter().any(|g| {
                    !g.is_synthesized
                        || g.byte_range.is_some()
                        || ![
                            g.x,
                            g.x_offset,
                            g.y_offset,
                            g.advance,
                            g.x + g.advance,
                            g.x + g.x_offset,
                        ]
                        .iter()
                        .all(|v| v.is_finite())
                })
            {
                return Err(E::InvalidAnnotation { index });
            }
        }
        self.check_visible_end_boundary()?;
        self.check_annotation_uniqueness()
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
                .filter(|g| !g.is_synthesized)
                .any(|g| g.byte_range.is_some_and(on_edge))
            || self.visual_boxes.iter().any(|b| on_edge(b.byte_range))
            || self.tabs.iter().any(|t| on_edge(t.byte_range));
        if ok {
            Ok(())
        } else {
            Err(LineInvariantError::VisibleEndNotOnBoundary)
        }
    }

    /// `true` when `range` is well formed and inside this line's bytes.
    fn owns_bytes(&self, range: TextRange) -> bool {
        range.start <= range.end && self.byte_range.contains_range(range)
    }

    /// Annotation ids are unique on the line, and two annotations on one side
    /// never share base bytes.
    fn check_annotation_uniqueness(&self) -> Result<(), LineInvariantError> {
        let mut ids = std::collections::HashSet::new();
        if let Some(index) = self
            .annotations
            .iter()
            .position(|a| !ids.insert(a.annotation_id))
        {
            return Err(LineInvariantError::InvalidAnnotation { index });
        }
        for side in [
            crate::frontend::AnnotationSide::Over,
            crate::frontend::AnnotationSide::Under,
        ] {
            let mut on_side: Vec<usize> = (0..self.annotations.len())
                .filter(|&i| self.annotations[i].side == side)
                .collect();
            on_side.sort_by_key(|&i| self.annotations[i].base_byte_range.start);
            let mut reach = 0;
            for &index in &on_side {
                let base = self.annotations[index].base_byte_range;
                if base.start < reach {
                    return Err(LineInvariantError::InvalidAnnotation { index });
                }
                reach = reach.max(base.end);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FontId;

    const STRUT: FontMetrics = FontMetrics {
        vertical_descent: 5.0,
        vertical_line_gap: 0.0,
        x_height: 0.0,

        underline_thickness: 0.0,
        use_typo_metrics: false,
        vertical_ascent: 5.0,
        cap_height: 0.0,
        strikeout_position: 0.0,
        underline_position: 0.0,

        ideographic_baseline_offset: 0.0,
        mathematical_baseline_offset: 0.0,
        central_baseline_offset: 0.0,
        hanging_baseline_offset: 0.0,
        ascent: 12.0,
        descent: 4.0,
        line_gap: 0.0,
    };

    /// Permissive on hanging so the older geometry tests stay about geometry.
    fn ctx() -> LineContext {
        LineContext {
            strut: STRUT,
            hang_mode: HangMode {
                start: crate::middleend::HangStartMode::AllLines,
                end: crate::middleend::HangEndMode::Force,
            },
            ..Default::default()
        }
    }

    fn preserved() -> LineContext {
        LineContext {
            whitespace: HangingWhitespaceMode::Preserved,
            ..ctx()
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

    fn one_slot(visual_width: f32, wrap_width: f32) -> smallvec::SmallVec<[PositionedSlot; 1]> {
        smallvec::smallvec![PositionedSlot {
            start: 0.0,
            visual_width,
            constraint_width: wrap_width,
        }]
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
            resolved_slot_constraints: smallvec::SmallVec::new(),
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
                orientation: crate::types::GlyphOrientation::Horizontal,
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
            baseline_align: crate::types::BaselineAlignment::Auto,
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
            let mut glyphs = vec![mark];
            glyphs.push(glyph(0.0, 20.0, false));
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
            "body glyph 2 of run 1 lies outside the text block"
        );
        let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(e);
        assert!(!boxed.to_string().is_empty());
    }

    fn no_hang() -> LineContext {
        LineContext {
            strut: STRUT,
            ..Default::default()
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
            l.validate(&ctx().for_line(1)),
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
        let mut para = PositionedParagraph::empty(BidiDirection::Rtl);
        para.lines = vec![l];
        para.total_block_size = 16.0;
        // The caller's context says LTR; the paragraph's direction wins.
        let c = LineContext {
            whitespace: HangingWhitespaceMode::Preserved,
            base_direction: BidiDirection::Ltr,
            ..no_hang()
        };
        assert_eq!(para.validate_geometry(&c), Ok(()));
        para.base_direction = BidiDirection::Ltr;
        assert_eq!(
            para.validate_geometry(&c),
            Err((
                0,
                LineInvariantError::TrailingWhitespaceOnWrongSide { run: 0, glyph: 1 }
            ))
        );
    }

    // ---- wrap width (spec §2.6, Pass 3 (h)) and fragment hit-testing (spec §6.3) ----

    fn wrap(w: f32) -> LineContext {
        ctx().with_slots(
            smallvec::smallvec![crate::middleend::InlineSlotConstraint { start: 0.0, end: w }],
            w,
        )
    }

    /// Validation of a single-slot line against a context of wrap width `w`.
    trait ValidateAt {
        fn validate_at(&self, w: f32) -> Result<(), LineInvariantError>;
    }

    impl ValidateAt for PositionedLine {
        fn validate_at(&self, w: f32) -> Result<(), LineInvariantError> {
            let mut l = self.clone();
            l.slots[0].constraint_width = w;
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
        // The default context imposes no limit.
        assert!(two().validate(&ctx()).is_ok());
        assert!(LineContext::default().wrap_width.is_infinite());
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
            baseline_align: crate::types::BaselineAlignment::Auto,
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
        assert!(trailing.validate_at(30.0).is_ok());
        // ... but not over a visible byte the entity does not own,
        let mut visible = trailing.clone();
        visible.visible_byte_end = 4;
        too_wide(&visible);
        // not start elsewhere or stop short of the entity,
        for bytes in [(1, 3), (0, 2)] {
            let mut other = wide((3.0, 3.0));
            other.span_fragments[0].byte_range = TextRange::new(bytes.0, bytes.1);
            too_wide(&other);
        }
        // and not leave the line (Pass 3(g)).
        let mut beyond = wide((3.0, 3.0));
        beyond.span_fragments[0].byte_range = TextRange::new(0, 5);
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
            baseline_align: crate::types::BaselineAlignment::Auto,
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
        assert!(trailing.validate_at(30.0).is_ok());
        let mut other = l.clone();
        other.span_fragments[0].byte_range = TextRange::new(0, 0);
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
        let mut p = PositionedParagraph::empty(BidiDirection::Ltr);
        p.lines = vec![line(
            vec![
                word_glyph(0.0, 10.0, (0, 1)),
                word_glyph(10.0, 20.0, (1, 2)),
            ],
            30.0,
            0.0,
            0.0,
        )];
        p.total_block_size = 16.0;
        p.lines[0].slots[0].constraint_width = 30.0;
        assert!(p.validate_geometry(&wrap(30.0)).is_ok());
        p.lines[0].slots[0].constraint_width = 10.0;
        assert_eq!(
            p.validate_geometry(&wrap(10.0)),
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
