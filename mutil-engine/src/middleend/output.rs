//! Middle-end output: positioned lines and glyphs (spec §2.6).

use super::{HangMode, HangingWhitespaceMode, LayoutOptions, LineHeight};
use crate::frontend::BoxOverflowPolicy;
use crate::types::{
    BIDI_MAX_DEPTH, BidiDirection, BidiLevel, ByteIndex, FontMetrics, FontRef, Size, TextRange,
    VerticalMetrics,
};
use core::fmt;

/// One glyph placed on a line (spec §2.6).
///
/// `x` is **line-local**: measured from the line's left edge, so a line can be
/// moved without touching its glyphs. **[Design decision]** (spec silent).
/// It is the strict **pen advance position**, the cumulative advance along the
/// line, and not the drawing origin: the rasteriser draws the glyph at
/// `(line_origin_x + x + x_offset, baseline_y - y_offset)` (row 71). A
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
    /// Vertical offset from the baseline (clamped, §5 Stage 4.3).
    pub y_offset: f32,
    /// Advance width.
    pub advance: f32,
    /// Source bytes. `None` (or an empty range) for synthesized glyphs. The
    /// draft's mandatory field contradicted §6.6; see §2.6.
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
    pub x_start: f32,
    /// Right end, line-local.
    pub x_end: f32,
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
/// §2.6, row 77), after bidi reordering.
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
    /// UAX #9 level of the text in this fragment. Its parity decides which half
    /// of the fragment snaps to the logical start (see
    /// [`PositionedSpanFragment::snaps_to_logical_start`]).
    pub bidi_level: BidiLevel,
}

impl PositionedSpanFragment {
    /// Hit-test rule for a click inside a fragment of an **atomic** span (spec
    /// §6.3, ruling 7): `true` when `point_x` snaps to the span's logical start
    /// (the `AtomicEditSpan`'s `byte_range.start`, `Downstream`), `false` for
    /// its logical end (its `byte_range.end`, `Upstream`). The endpoints belong
    /// to the span, not to this fragment's own `byte_range`, which covers only
    /// the part of the span on its line.
    ///
    /// The midpoint is that of **this fragment**, not of the whole wrapped
    /// span, or the leading edge of a narrow second-line fragment would be
    /// unreachable. At an even level the left half is the start; at an odd level
    /// the halves swap. A point exactly on the midpoint counts as the right half.
    #[must_use]
    pub fn snaps_to_logical_start(&self, point_x: f32) -> bool {
        let in_left_half = point_x < self.x + 0.5 * self.width;
        in_left_half == (self.bidi_level & 1 == 0)
    }
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
    /// Physical left edge.
    pub x_start: f32,
    /// Physical right edge.
    pub x_end: f32,
    /// Resolved fragment direction.
    pub bidi_level: BidiLevel,
}

/// `true` when a fragment's geometry is finite and ordered, its ranges nest,
/// and its bidi level is representable.
fn atomic_fragment_measurements_are_valid(fragment: &AtomicFragment) -> bool {
    let geometry_ok = fragment.x_start.is_finite()
        && fragment.x_end.is_finite()
        && fragment.x_start <= fragment.x_end;
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
    /// Width of the visible content **excluding** hanging punctuation, so the
    /// text block spans `[0, visual_width]`.
    pub visual_width: f32,
    /// Width of punctuation hanging outside the **left** edge (glyphs there have
    /// `x < 0`). Zero unless `HangMode` hangs something. Physical, not logical:
    /// for RTL lines the line-start hang is on the right.
    /// **[Design decision]**: addition to §2.6, needed so caret, hit-test and
    /// selection geometry (§6) know the occupied extent.
    pub hanging_left: f32,
    /// Width of punctuation hanging outside the **right** edge (beyond
    /// `visual_width`, possibly beyond `wrap_width`). See `hanging_left`.
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
fn tolerance(a: f32, b: f32) -> f32 {
    LINE_GEOMETRY_ABS_EPSILON.max(LINE_GEOMETRY_REL_EPSILON * a.abs().max(b.abs()))
}

fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= tolerance(a, b)
}

/// What a line is validated against.
///
/// Paragraph-wide fields plus `line_index`; use [`LineContext::for_line`] to
/// derive the context of each line from one paragraph context.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineContext {
    /// The **resolved** strut: the raw `ParagraphAst::strut_metrics` after
    /// `LineHeight::resolve_strut`, half-leading included.
    pub strut: VerticalMetrics,
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
            strut: VerticalMetrics::default(),
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
        }
    }

    /// The same context for line `line_index`.
    #[must_use]
    pub const fn for_line(self, line_index: usize) -> Self {
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

/// A violated [`PositionedLine`] invariant.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LineInvariantError {
    /// `visual_width`, `hanging_left` or `hanging_right` is negative, NaN or infinite.
    NonFiniteOrNegativeExtent,
    /// `baseline_y`, `ascent`, `descent` or `line_height` is NaN or infinite.
    NonFiniteLineMetrics,
    /// A line height or the resolved strut height is not positive.
    NonPositiveLineHeight {
        /// `VerticalMetrics::natural_line_height()` of the strut.
        expected: f32,
        /// The line's `line_height`.
        actual: f32,
    },
    /// `visible_byte_end` lies outside `byte_range`.
    VisibleEndOutsideRange,
    /// An empty line (`byte_range.start == byte_range.end`) has visual runs.
    EmptyLineHasVisualRuns,
    /// An empty line has inline boxes.
    EmptyLineHasBoxes,
    /// An empty line's `line_height` differs from the strut's.
    EmptyLineHeightMismatch {
        /// `VerticalMetrics::natural_line_height()` of the strut.
        expected: f32,
        /// The line's `line_height`.
        actual: f32,
    },
    /// An empty line has a non-zero `visual_width` or hanging extent.
    EmptyLineHasExtent,
    /// Under `LineHeight::Fixed`, `line_height` differs from the strut's.
    LineHeightMismatch {
        /// `VerticalMetrics::natural_line_height()` of the strut.
        expected: f32,
        /// The line's `line_height`.
        actual: f32,
    },
    /// Under `LineHeight::Normal`, `line_height` is smaller than the strut's.
    LineHeightBelowStrut {
        /// `VerticalMetrics::natural_line_height()` of the strut.
        minimum: f32,
        /// The line's `line_height`.
        actual: f32,
    },
    /// A visual run's font size, fallback scale or a variation coordinate is
    /// non-finite or non-positive.
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
    /// An inline box has a non-finite coordinate or a negative size.
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
    /// The line is wider than `ctx.wrap_width` although it is not a single
    /// overflowing entity.
    LineWiderThanWrapWidth {
        /// The line's `visual_width`.
        visual_width: f32,
        /// `LineContext::wrap_width`.
        wrap_width: f32,
    },
    /// A tab has a non-finite coordinate or a negative width.
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
    /// A caret stop has a non-finite `x`.
    NonFiniteCaretStop {
        /// Index into `caret_stops`.
        index: usize,
    },
    /// A decoration has a non-finite coordinate or thickness.
    NonFiniteDecoration {
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

impl fmt::Display for LineInvariantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteOrNegativeExtent => {
                f.write_str("visual_width or a hanging extent is negative or not finite")
            }
            Self::NonFiniteLineMetrics => {
                f.write_str("baseline_y, ascent, descent or line_height is not finite")
            }
            Self::NonPositiveLineHeight { expected, actual } => {
                write!(
                    f,
                    "line height {actual} or strut height {expected} is not positive"
                )
            }
            Self::InvalidVisualRun { run } => {
                write!(f, "visual run {run} has an invalid font instance")
            }
            Self::InvalidAtomicFragment { index } => {
                write!(f, "atomic fragment {index} has invalid geometry or ranges")
            }
            Self::VisibleEndOutsideRange => f.write_str("visible_byte_end lies outside byte_range"),
            Self::EmptyLineHasVisualRuns => f.write_str("an empty line has visual runs"),
            Self::EmptyLineHasBoxes => f.write_str("an empty line has inline boxes"),
            Self::EmptyLineHeightMismatch { expected, actual } => {
                write!(
                    f,
                    "empty line height {actual} differs from the strut height {expected}"
                )
            }
            Self::EmptyLineHasExtent => {
                f.write_str("an empty line has a non-zero width or hanging extent")
            }
            Self::LineHeightMismatch { expected, actual } => {
                write!(
                    f,
                    "line height {actual} differs from the fixed strut height {expected}"
                )
            }
            Self::LineHeightBelowStrut { minimum, actual } => {
                write!(
                    f,
                    "line height {actual} is below the strut height {minimum}"
                )
            }
            Self::NonFiniteGlyph { run, glyph } => {
                write!(f, "glyph {glyph} of run {run} has a non-finite coordinate")
            }
            Self::TrailingWhitespaceNotCollapsed { run, glyph } => {
                write!(
                    f,
                    "trailing whitespace glyph {glyph} of run {run} has width in Collapsible mode"
                )
            }
            Self::HangingGlyphInsideBlock { run, glyph } => {
                write!(
                    f,
                    "hanging glyph {glyph} of run {run} overlaps the text block"
                )
            }
            Self::BodyGlyphOutsideBlock { run, glyph } => {
                write!(
                    f,
                    "body glyph {glyph} of run {run} lies outside the text block"
                )
            }
            Self::HangingExtentMismatch {
                side,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "{side:?} hanging extent is {actual} but the glyphs occupy {expected}"
                )
            }
            Self::InvalidInlineBox { index } => {
                write!(
                    f,
                    "inline box {index} has a non-finite coordinate or a negative size"
                )
            }
            Self::InlineBoxOutsideBlock { index } => {
                write!(f, "inline box {index} lies outside the text block")
            }
            Self::EmptyLineHasTabs => f.write_str("an empty line has tabs"),
            Self::EmptyLineHasSpanFragments => f.write_str("an empty line has span fragments"),
            Self::EmptyLineHasAtomicFragments => f.write_str("an empty line has atomic fragments"),
            Self::InvalidSpanFragment { index } => {
                write!(
                    f,
                    "span fragment {index} has a non-finite coordinate or a negative size"
                )
            }
            Self::SpanFragmentOutsideBlock { index } => {
                write!(f, "span fragment {index} lies outside the text block")
            }
            Self::SpanFragmentOutsideLine { index } => {
                write!(f, "span fragment {index} lies outside the line's bytes")
            }
            Self::GlyphOutsideSpanFragment {
                span_id,
                run,
                glyph,
            } => {
                write!(
                    f,
                    "glyph {glyph} of run {run} lies outside every fragment of span {span_id}"
                )
            }
            Self::InlineBoxOutsideSpanFragment { span_id, index } => {
                write!(
                    f,
                    "inline box {index} lies outside every fragment of span {span_id}"
                )
            }
            Self::InlineBoxWiderThanWrapWidth { index } => {
                write!(f, "inline box {index} exceeds the wrap width")
            }
            Self::LineWiderThanWrapWidth {
                visual_width,
                wrap_width,
            } => {
                write!(
                    f,
                    "line width {visual_width} exceeds the wrap width {wrap_width} and is not a single overflowing entity"
                )
            }
            Self::InvalidTab { index } => {
                write!(
                    f,
                    "tab {index} has a non-finite coordinate or a negative width"
                )
            }
            Self::TabOutsideBlock { index } => {
                write!(f, "tab {index} lies outside the text block")
            }
            Self::TrailingTabNotCollapsed { index } => {
                write!(f, "trailing tab {index} has width in Collapsible mode")
            }
            Self::TrailingWhitespaceOnWrongSide { run, glyph } => {
                write!(
                    f,
                    "trailing whitespace glyph {glyph} of run {run} is not on the far edge of the base direction"
                )
            }
            Self::TrailingTabOnWrongSide { index } => {
                write!(
                    f,
                    "trailing tab {index} is not on the far edge of the base direction"
                )
            }
            Self::NonFiniteCaretStop { index } => {
                write!(f, "caret stop {index} has a non-finite x")
            }
            Self::NonFiniteDecoration { index } => {
                write!(f, "decoration {index} has a non-finite coordinate")
            }
            Self::TrailingWhitespaceMarkedHanging { run, glyph } => {
                write!(
                    f,
                    "trailing whitespace glyph {glyph} of run {run} is flagged as hanging"
                )
            }
            Self::HangingNotAllowed { side } => {
                write!(
                    f,
                    "{side:?} hanging punctuation is not allowed by the hang mode"
                )
            }
        }
    }
}

impl std::error::Error for LineInvariantError {}

impl PositionedLine {
    /// Checks the line invariants (spec §5 Stage 1.3, 1.7 and Stage 4) against
    /// `ctx`, in three passes.
    ///
    /// **Pass 1, finiteness.** Every number of every entity is scanned before
    /// anything else is judged: the line metrics and extents, every glyph (`x`,
    /// `x_offset`, `advance`, `y_offset`), every inline box (all coordinates and
    /// sizes), every tab (`x`, `width`) and every span fragment (`x`, `width`,
    /// `ascent`, `descent`). A NaN or infinity, or a negative extent, box, tab
    /// or fragment size, is rejected at once. A negative glyph `advance` is *not*
    /// rejected: kerning can produce one.
    ///
    /// **Pass 2, envelope.** `visible_byte_end` lies within `byte_range`. An
    /// empty line (`byte_range` empty) has no runs, boxes or tabs, zero
    /// `visual_width` and hanging extents, no span fragments, and `line_height`
    /// equal to the resolved strut. Otherwise `line_height` equals the strut under
    /// `LineHeight::Fixed` and is at least the strut under `LineHeight::Normal`
    /// (fallback glyphs may enlarge it, §5 Stage 4.1).
    ///
    /// **Pass 3, containment and hanging.** `visual_width` is the body width,
    /// so the body is `[0, visual_width]`:
    /// * every non-hanging, non-trailing glyph, inline box and tab starts at
    ///   `>= 0` and ends at `<= visual_width` (within tolerance). A glyph is
    ///   judged by its **pen interval** `[x, x + advance]`; `x_offset` is the
    ///   ink offset and plays no part, so a zero-advance mark with a negative
    ///   `x_offset` at the line start (`x == 0`) is fine. A zero-advance glyph
    ///   flagged `is_hanging` rides on a hanging base and is not classified;
    /// * every span fragment lies within `[0, visual_width]`, and every glyph
    ///   or inline box whose `byte_range` lies in the range of a fragment of
    ///   span `S` has its interval contained in at least one fragment of `S`.
    ///   Hanging glyphs, trailing whitespace and synthesized glyphs are exempt.
    ///   A line holding a single `AllowOverflow` object wider than `wrap_width`
    ///   is valid because its `visual_width` equals the object's width;
    /// * every `Clip` or `ScaleToFit` inline box ends at or before
    ///   `ctx.wrap_width` (within tolerance);
    /// * `visual_width <= ctx.wrap_width` (within tolerance), with two
    ///   single-entity exemptions. **A:** no visual runs, no tabs and exactly one
    ///   inline box, whose policy is `AllowOverflow`. **B:** no inline boxes, no
    ///   tabs, and glyphs that all belong to one indivisible cluster (they share
    ///   one non-empty `byte_range`; the emergency cluster break). Let the entity
    ///   be that box or cluster, with `entity_width` its width (the box's
    ///   `width`, or the sum of the glyph advances) and `[entity_start_x,
    ///   entity_end_x]` its extent. Without span fragments, `entity_start_x == 0`
    ///   and `visual_width == entity_width`. With span fragments (the entity sits
    ///   in padded decoration spans), every fragment starts where the entity
    ///   starts and ends at or after the entity's end, the entity ends at or
    ///   after `visible_byte_end` (so any bytes a fragment owns past the entity
    ///   are trailing whitespace, since fragments stay inside the line), the
    ///   entity lies within `[0, visual_width]`, and the fragments reach both
    ///   edges of the body (`min x == 0`, `max (x + width) == visual_width`).
    ///   The padding thus lets a 40-wide cluster with 3 units on each side pass
    ///   at `wrap_width = 30`, while an unpadded entity cannot inflate
    ///   `visual_width`;
    /// * every `is_hanging` glyph has actual overflow (may straddle the body);
    /// * `hanging_left`/`hanging_right` equal the extent those glyphs occupy
    ///   (so are positive exactly when such glyphs exist), and are zero on an
    ///   edge that `ctx.hang_mode` does not allow for `ctx.base_direction`;
    /// * trailing whitespace (glyphs and tabs whose `byte_range` starts at or
    ///   after `visible_byte_end`): under `Collapsible` it has zero width; under
    ///   `Preserved` it is exempt from containment but, by UAX #9 L1, must lie
    ///   on the far edge of the base direction (right of the body for LTR, left
    ///   of it for RTL). It never counts towards the hanging extents.
    ///
    /// This is containment, not tiling: glue produces no output entity, so the
    /// entities need not cover `[0, visual_width]` without gaps.
    ///
    /// Comparisons use a mixed absolute and relative tolerance
    /// ([`LINE_GEOMETRY_ABS_EPSILON`], [`LINE_GEOMETRY_REL_EPSILON`]).
    ///
    /// # Errors
    /// The first violated invariant.
    pub fn validate(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        self.check_finite()?;
        self.check_envelope(ctx)?;
        if self.byte_range.is_empty() {
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
        let metrics = [self.baseline_y, self.ascent, self.descent, self.line_height];
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
                return Err(E::NonFiniteCaretStop { index });
            }
        }
        for (index, d) in self.decorations.iter().enumerate() {
            if ![d.x_start, d.x_end, d.y_offset, d.thickness]
                .iter()
                .all(|v| v.is_finite())
            {
                return Err(E::NonFiniteDecoration { index });
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
        if range.is_empty() {
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

    /// Pass 3: containment of glyphs, boxes and tabs; hanging; trailing
    /// whitespace.
    fn check_containment_and_hanging(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let vw = self.visual_width;
        let collapsible = ctx.whitespace == HangingWhitespaceMode::Collapsible;
        // UAX #9 L1: trailing whitespace takes the paragraph level, so it lies
        // right of the body in an LTR paragraph and left of it in an RTL one.
        let on_far_edge = |x: f32, end: f32| match ctx.base_direction {
            BidiDirection::Ltr => x >= vw - tolerance(x, vw),
            BidiDirection::Rtl => end <= tolerance(end, 0.0),
        };

        let (mut max_left_overflow, mut max_right_overflow) = (0.0_f32, 0.0_f32);
        let (mut has_left_overflow_glyph, mut has_right_overflow_glyph) = (false, false);
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                // Normalised endpoints: a negative advance (kerning) must not
                // turn containment or hanging upside down.
                let (lo, hi) = (g.x.min(g.x + g.advance), g.x.max(g.x + g.advance));
                // A synthesized glyph (generated hyphen) is never trailing
                // whitespace, even when its range is empty at `visible_byte_end`.
                let trailing = !g.is_synthesized
                    && g.byte_range
                        .is_some_and(|b| b.start >= self.visible_byte_end);
                if trailing {
                    if g.is_hanging {
                        return Err(E::TrailingWhitespaceMarkedHanging { run, glyph });
                    }
                    if collapsible {
                        if g.advance != 0.0 {
                            return Err(E::TrailingWhitespaceNotCollapsed { run, glyph });
                        }
                    } else if g.advance != 0.0 && !on_far_edge(lo, hi) {
                        return Err(E::TrailingWhitespaceOnWrongSide { run, glyph });
                    }
                    continue;
                }
                let tol = tolerance(lo.abs().max(hi.abs()), vw);
                // A zero-advance glyph (combining mark) attaches to a base and
                // has no horizontal extent of its own. It is exempt from both
                // containment and hanging: it neither overflows nor contributes
                // a hanging extent. A line that declares hanging extent must
                // still be backed by a non-zero-advance hanging glyph (checked
                // below), so zero-width marks cannot fake it.
                if g.advance == 0.0 {
                    continue;
                }
                if g.is_hanging {
                    let over_left = (0.0_f32 - lo).max(0.0);
                    let over_right = (hi - vw).max(0.0);
                    if over_left <= tol && over_right <= tol {
                        return Err(E::HangingGlyphInsideBlock { run, glyph });
                    }
                    if over_left > 0.0 {
                        max_left_overflow = max_left_overflow.max(over_left);
                        has_left_overflow_glyph = true;
                    }
                    if over_right > 0.0 {
                        max_right_overflow = max_right_overflow.max(over_right);
                        has_right_overflow_glyph = true;
                    }
                } else if lo < -tol || hi > vw + tol {
                    return Err(E::BodyGlyphOutsideBlock { run, glyph });
                }
            }
        }
        for (side, expected, actual, present) in [
            (
                HangSide::Left,
                max_left_overflow,
                self.hanging_left,
                has_left_overflow_glyph,
            ),
            (
                HangSide::Right,
                max_right_overflow,
                self.hanging_right,
                has_right_overflow_glyph,
            ),
        ] {
            // A declared extent must match the glyphs and come from at least
            // one non-zero-advance hanging glyph, never from zero-width marks.
            let extent_is_backed = actual <= 0.0 || present;
            if !approx_eq(expected, actual) || !extent_is_backed {
                return Err(E::HangingExtentMismatch {
                    side,
                    expected,
                    actual,
                });
            }
            // A hanging glyph is judged by its presence, not by an extent that
            // may be zero or inside the tolerance.
            if (present || actual > 0.0)
                && !ctx
                    .hang_mode
                    .allows(side, ctx.base_direction, ctx.line_index == 0)
            {
                return Err(E::HangingNotAllowed { side });
            }
        }

        for (index, b) in self.visual_boxes.iter().enumerate() {
            let end = b.x + b.width;
            let tol = tolerance(b.x.abs().max(end.abs()), vw);
            if b.x < -tol || end > vw + tol {
                return Err(E::InlineBoxOutsideBlock { index });
            }
        }
        for (index, t) in self.tabs.iter().enumerate() {
            let end = t.x + t.width;
            if t.byte_range.start >= self.visible_byte_end {
                if collapsible {
                    if t.width != 0.0 {
                        return Err(E::TrailingTabNotCollapsed { index });
                    }
                } else if t.width != 0.0 && !on_far_edge(t.x, end) {
                    return Err(E::TrailingTabOnWrongSide { index });
                }
                continue;
            }
            let tol = tolerance(t.x.abs().max(end.abs()), vw);
            if t.x < -tol || end > vw + tol {
                return Err(E::TabOutsideBlock { index });
            }
        }
        self.check_span_fragments()?;
        self.check_wrap_width(ctx)
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
    fn is_single_entity_overflow(&self) -> bool {
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
                                && !g.is_synthesized
                                && g.byte_range.is_none_or(|b| b.start < self.visible_byte_end)
                        })
                };
                let mut ranges = glyphs().map(|g| g.byte_range);
                let Some(first @ Some(r)) = ranges.next() else {
                    return false;
                };
                if r.is_empty() || !ranges.all(|o| o == first) {
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
            return start.abs() <= tolerance(start, vw) && approx_eq(width, vw);
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
            && start >= -tolerance(start, vw)
            && end <= vw + tolerance(end, vw)
            && left.abs() <= tolerance(left, vw)
            && approx_eq(right, vw)
    }

    /// Pass 3, last check: width against `ctx.wrap_width`.
    fn check_wrap_width(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        for (index, b) in self.visual_boxes.iter().enumerate() {
            let rigid = b.overflow_policy != BoxOverflowPolicy::AllowOverflow;
            let end = b.x + b.width;
            if rigid && end > ctx.wrap_width + tolerance(end, ctx.wrap_width) {
                return Err(LineInvariantError::InlineBoxWiderThanWrapWidth { index });
            }
        }
        let vw = self.visual_width;
        if vw <= ctx.wrap_width + tolerance(vw, ctx.wrap_width) || self.is_single_entity_overflow()
        {
            return Ok(());
        }
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: vw,
            wrap_width: ctx.wrap_width,
        })
    }

    /// Span containment (spec §2.6, row 77): fragments inside the body, and
    /// every glyph or inline box that belongs to a span inside one of its
    /// fragments.
    fn check_span_fragments(&self) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        if self.span_fragments.is_empty() {
            return Ok(());
        }
        let vw = self.visual_width;
        for (index, f) in self.span_fragments.iter().enumerate() {
            let end = f.x + f.width;
            let tol = tolerance(f.x.abs().max(end.abs()), vw);
            if f.x < -tol || end > vw + tol {
                return Err(E::SpanFragmentOutsideBlock { index });
            }
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

/// Laid-out paragraph.
///
/// **[Design decision]** The spec's §2.6 defines `PositionedLine` and
/// `PositionedGlyph` but gives `PositionedParagraph` no fields. It holds the
/// lines plus the direction and overall size that every consumer needs. Owns
/// all its data (`'static`, `Send + Sync`).
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedParagraph {
    /// Paragraph base direction, copied from the AST.
    pub base_direction: BidiDirection,
    /// Overall extent: widest line by total line height.
    pub size: Size,
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
    pub fn validate(&self, ctx: &LineContext) -> Result<(), (usize, LineInvariantError)> {
        for (i, line) in self.lines.iter().enumerate() {
            let line_ctx = LineContext {
                base_direction: self.base_direction,
                ..ctx.for_line(i)
            };
            line.validate(&line_ctx).map_err(|e| (i, e))?;
        }
        Ok(())
    }

    /// A paragraph with no lines and zero size.
    #[must_use]
    pub fn empty(base_direction: BidiDirection) -> Self {
        Self {
            base_direction,
            size: Size::default(),
            lines: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FontId;

    const STRUT: VerticalMetrics = VerticalMetrics {
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

    fn line(glyphs: Vec<PositionedGlyph>, width: f32, hang_l: f32, hang_r: f32) -> PositionedLine {
        PositionedLine {
            atomic_fragments: vec![],

            byte_range: TextRange::new(0, 4),
            visible_byte_end: 4,
            baseline_y: 12.0,
            ascent: 12.0,
            descent: 4.0,
            line_height: 16.0,
            visual_width: width,
            hanging_left: hang_l,
            hanging_right: hang_r,
            is_justified: false,
            visual_runs: vec![VisualGlyphRun {
                font_size: 16.0,
                variations: Default::default(),

                font: FontRef::unscaled(FontId(0)),
                bidi_level: 0,
                byte_range: TextRange::new(0, 4),
                glyphs,
            }],
            visual_boxes: vec![],
            tabs: vec![],
            span_fragments: vec![],
            caret_stops: vec![],
            decorations: vec![],
        }
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
        assert!(matches!(
            wrong.validate(&ctx()),
            Err(LineInvariantError::HangingExtentMismatch {
                side: HangSide::Left,
                ..
            })
        ));
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
        assert!(matches!(
            claimed.validate(&preserved()),
            Err(LineInvariantError::HangingExtentMismatch {
                side: HangSide::Right,
                ..
            })
        ));
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
        assert!(matches!(
            l.validate(&ctx()),
            Err(LineInvariantError::BodyGlyphOutsideBlock { .. })
        ));
        // Small coordinates keep the strict absolute floor.
        let l = line(vec![glyph(0.0, 20.01, false)], 20.0, 0.0, 0.0);
        assert!(matches!(
            l.validate(&ctx()),
            Err(LineInvariantError::BodyGlyphOutsideBlock { .. })
        ));
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
            assert!(matches!(
                line(vec![g], 5.0, 0.0, 0.0).validate(&ctx()),
                Err(LineInvariantError::NonFiniteGlyph { .. })
            ));
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
        assert!(line(vec![far], 20.0, 0.0, 0.0).is_valid(&ctx()));
        // A zero advance has no horizontal extent, so its pen position is
        // exempt from containment even outside the body.
        assert!(line(vec![glyph(-3.0, 0.0, false)], 20.0, 0.0, 0.0).is_valid(&ctx()));
        assert!(line(vec![glyph(25.0, 0.0, false)], 20.0, 0.0, 0.0).is_valid(&ctx()));
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
        padded.visual_width = 34.0;
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

    #[test]
    fn caret_stops_and_decorations_must_be_finite() {
        let mut l = line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        l.caret_stops.push(CaretStop {
            x: f32::NAN,
            offset: 0,
            affinity: crate::middleend::CaretAffinity::Downstream,
            level: 0,
            line: 0,
        });
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::NonFiniteCaretStop { index: 0 })
        );
        let mut l = line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        l.decorations.push(DecorationLine {
            kind: DecorationKind::Underline,
            byte_range: TextRange::new(0, 1),
            x_start: 0.0,
            x_end: f32::INFINITY,
            y_offset: 1.0,
            thickness: 1.0,
        });
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::NonFiniteDecoration { index: 0 })
        );
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
        assert!(line(vec![glyph(10.0, -5.0, false)], 20.0, 0.0, 0.0).is_valid(&ctx()));
    }

    #[test]
    fn synthesized_hyphen_with_an_empty_range_is_not_trailing_whitespace() {
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
            byte_range: Some(TextRange::empty_at(4)),
            ..glyph(15.0, 5.0, false)
        };
        assert_eq!(l.validate(&ctx()), Ok(()));
        // The same glyph as ordinary text at that position *would* be trailing.
        l.visual_runs[0].glyphs[1].is_synthesized = false;
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::TrailingWhitespaceNotCollapsed { run: 0, glyph: 1 })
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
        // The caller's context says LTR; the paragraph's direction wins.
        let c = LineContext {
            whitespace: HangingWhitespaceMode::Preserved,
            base_direction: BidiDirection::Ltr,
            ..no_hang()
        };
        assert_eq!(para.validate(&c), Ok(()));
        para.base_direction = BidiDirection::Ltr;
        assert!(para.validate(&c).is_err());
    }

    // ---- wrap width (ruling 8) and fragment hit-testing (ruling 7) ----

    fn wrap(w: f32) -> LineContext {
        LineContext {
            wrap_width: w,
            ..ctx()
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
        assert!(two().validate(&wrap(30.0)).is_ok());
        assert!(two().validate(&wrap(30.0 - 1e-4)).is_ok());
        assert_eq!(
            two().validate(&wrap(29.0)),
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
        assert!(l.validate(&wrap(30.0)).is_ok());
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
        assert!(matches!(
            l.validate(&wrap(30.0)),
            Err(LineInvariantError::LineWiderThanWrapWidth { .. })
        ));
        // A lone cluster whose extent is not the line width is not the overflow
        // state either (the line is wider than what it holds).
        let l = line(vec![word_glyph(0.0, 25.0, (0, 3))], 40.0, 0.0, 0.0);
        assert!(matches!(
            l.validate(&wrap(30.0)),
            Err(LineInvariantError::LineWiderThanWrapWidth { .. })
        ));
    }

    #[test]
    fn a_single_wide_inline_box_may_overflow() {
        let object = |x: f32, width: f32, policy| PositionedInlineBox {
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
        assert!(l.validate(&wrap(30.0)).is_ok());
        // A rigid policy must fit: Clip and ScaleToFit boxes wider than the line
        // are rejected, by the box rule and then by the width rule.
        for policy in [BoxOverflowPolicy::Clip, BoxOverflowPolicy::ScaleToFit] {
            let mut rigid = l.clone();
            rigid.visual_boxes = vec![object(0.0, 50.0, policy)];
            assert_eq!(
                rigid.validate(&wrap(30.0)),
                Err(LineInvariantError::InlineBoxWiderThanWrapWidth { index: 0 })
            );
            // A rigid box that fits is fine.
            rigid.visual_boxes = vec![object(0.0, 30.0, policy)];
            rigid.visual_width = 30.0;
            assert!(rigid.validate(&wrap(30.0)).is_ok());
        }
        // Not when it shares the line with a glyph run, a tab or a second box.
        let mut with_run = l.clone();
        with_run.visual_runs = line(vec![], 50.0, 0.0, 0.0).visual_runs;
        assert!(with_run.validate(&wrap(30.0)).is_err());
        let mut with_tab = l.clone();
        with_tab.tabs = vec![PositionedTab {
            byte_range: TextRange::new(0, 1),
            x: 50.0,
            width: 0.0,
        }];
        assert!(with_tab.validate(&wrap(30.0)).is_err());
        let mut two = l.clone();
        two.visual_boxes = vec![object(0.0, 25.0, allow), object(25.0, 25.0, allow)];
        assert!(two.validate(&wrap(30.0)).is_err());
        // The box must start at 0 and be as wide as the line.
        let mut narrow = l.clone();
        narrow.visual_boxes = vec![object(0.0, 20.0, allow)];
        assert!(narrow.validate(&wrap(30.0)).is_err());
        let mut shifted = l;
        shifted.visual_boxes = vec![object(5.0, 45.0, allow)];
        assert!(shifted.validate(&wrap(30.0)).is_err());
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
        assert!(wide((3.0, 3.0)).validate(&wrap(30.0)).is_ok());
        assert!(wide((3.0, 0.0)).validate(&wrap(30.0)).is_ok());
        assert!(wide((0.0, 3.0)).validate(&wrap(30.0)).is_ok());
        // Nested fragments of the same bytes: all must wrap the entity.
        let mut nested = wide((3.0, 3.0));
        let mut inner = fragment(2, 3.0, 26.0, (0, 3));
        inner.is_first_fragment = true;
        nested.span_fragments.push(inner);
        assert!(nested.validate(&wrap(30.0)).is_ok());
        let too_wide = |l: &PositionedLine| {
            matches!(
                l.validate(&wrap(30.0)),
                Err(LineInvariantError::LineWiderThanWrapWidth { .. })
            )
        };
        // A fragment may run on over the trailing whitespace ...
        let mut trailing = wide((3.0, 3.0));
        trailing.span_fragments[0].byte_range = TextRange::new(0, 4);
        assert!(trailing.validate(&wrap(30.0)).is_ok());
        // ... but not over a visible byte the entity does not own,
        let mut visible = trailing.clone();
        visible.visible_byte_end = 4;
        assert!(too_wide(&visible));
        // not start elsewhere or stop short of the entity,
        for bytes in [(1, 3), (0, 2)] {
            let mut other = wide((3.0, 3.0));
            other.span_fragments[0].byte_range = TextRange::new(bytes.0, bytes.1);
            assert!(too_wide(&other), "{bytes:?}");
        }
        // and not leave the line (Pass 3(g)).
        let mut beyond = wide((3.0, 3.0));
        beyond.span_fragments[0].byte_range = TextRange::new(0, 5);
        assert_eq!(
            beyond.validate(&wrap(30.0)),
            Err(LineInvariantError::SpanFragmentOutsideLine { index: 0 })
        );
        // Fragments that stop short of an edge of the body.
        let mut short = wide((3.0, 3.0));
        short.span_fragments = vec![fragment(1, 0.0, 29.0, (0, 3))];
        assert!(too_wide(&short));
        let mut late = wide((3.0, 3.0));
        late.span_fragments = vec![fragment(1, 3.0, 29.0, (0, 3))];
        assert!(too_wide(&late));
        // The padding is only a claim: the entity cannot be outside the body.
        // (Pass 3 rejects it earlier, as containment.)
        let mut outside = wide((3.0, 3.0));
        outside.visual_runs[0].glyphs[0].x = 10.0;
        assert!(outside.validate(&wrap(30.0)).is_err());
        // Unpadded, the width must still be exactly the entity's.
        let mut bare = line(vec![word_glyph(0.0, 26.0, (0, 3))], 32.0, 0.0, 0.0);
        assert!(too_wide(&bare));
        bare.visual_width = 26.0;
        assert!(bare.validate(&wrap(30.0)).is_ok());

        // Exemption A, padded: one AllowOverflow box in a chip over its bytes.
        let object = |x: f32, width: f32| PositionedInlineBox {
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
        l.byte_range = TextRange::new(0, 2);
        l.visible_byte_end = 1;
        l.visual_runs.clear();
        l.visual_boxes = vec![object(3.0, 26.0)];
        l.span_fragments = vec![fragment(1, 0.0, 32.0, (0, 1))];
        assert!(l.validate(&wrap(30.0)).is_ok());
        // Trailing whitespace inside the fragment is fine; stopping short is not.
        let mut trailing = l.clone();
        trailing.span_fragments[0].byte_range = TextRange::new(0, 2);
        assert!(trailing.validate(&wrap(30.0)).is_ok());
        let mut other = l.clone();
        other.span_fragments[0].byte_range = TextRange::new(0, 0);
        assert!(too_wide(&other));
        let mut short = l.clone();
        short.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 1))];
        assert!(too_wide(&short));
        // Without fragments the box must still start at 0.
        let mut bare = l;
        bare.span_fragments.clear();
        assert!(too_wide(&bare));
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
        assert!(p.validate(&wrap(30.0)).is_ok());
        assert_eq!(p.validate(&wrap(10.0)).map_err(|(i, _)| i), Err(0));
    }

    #[test]
    fn fragment_snapping_uses_its_own_midpoint_and_bidi_parity() {
        let mut f = fragment(1, 100.0, 20.0, (0, 4)); // [100, 120], mid 110
        assert!(f.snaps_to_logical_start(105.0)); // LTR left half: start
        assert!(!f.snaps_to_logical_start(115.0)); // right half: end
        assert!(!f.snaps_to_logical_start(110.0)); // the midpoint is the right half
        f.bidi_level = 1;
        assert!(!f.snaps_to_logical_start(105.0)); // RTL: halves swap
        assert!(f.snaps_to_logical_start(115.0));
        assert!(f.snaps_to_logical_start(110.0)); // the midpoint is the physical right half
        f.bidi_level = 2;
        assert!(f.snaps_to_logical_start(105.0));
        // A narrow second-line fragment keeps its own start reachable: its left
        // half is judged against its own edges, not those of the whole span.
        let narrow = fragment(1, 0.0, 6.0, (3, 4));
        assert!(narrow.snaps_to_logical_start(1.0));
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
