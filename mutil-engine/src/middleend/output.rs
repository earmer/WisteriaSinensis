//! Middle-end output: positioned lines and glyphs (spec §2.6).

use super::{CaretStop, HangMode, HangingWhitespaceMode, LayoutOptions, LineHeight};
use crate::types::{
    BidiDirection, BidiLevel, ByteIndex, FontMetrics, FontRef, Size, TextRange, VerticalMetrics,
};
use core::fmt;

/// One glyph placed on a line (spec §2.6).
///
/// `x` is **line-local**: measured from the line's left edge, so a line can be
/// moved without touching its glyphs. **[Design decision]** (spec silent).
/// It is the final drawing origin (pen position plus the shaper's `x_offset`)
/// and may be **negative**: hanging punctuation placed outside the left edge
/// has `x < 0` (§5 Stage 1.3), see [`PositionedLine::hanging_left`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// Backend glyph index in the run's font.
    pub glyph_id: u32,
    /// Drawing-origin x from the line's left edge: pen position plus the
    /// shaper's `x_offset`.
    pub x: f32,
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
    /// Such a glyph lies wholly outside `[0, visual_width]`: left of 0
    /// (`x < 0`) or at or beyond `visual_width`. **[Design decision]**:
    /// addition to §2.6, so the hanging invariants of [`PositionedLine`] are
    /// checkable.
    pub is_hanging: bool,
}

/// Glyphs of one font and one bidi level, in screen order.
///
/// **[Design decision]** The spec names `VisualGlyphRun` (§2.6) without
/// defining it; it groups glyphs that share a font so a painter can bind one
/// face per run.
#[derive(Clone, Debug, PartialEq)]
pub struct VisualGlyphRun {
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
    /// Caller-assigned identifier from `WordNode::InlineBox::box_id`.
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

/// One laid-out line (spec §2.6).
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedLine {
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
}

impl LineContext {
    /// Builds the paragraph context: resolves the active strut from the **raw**
    /// primary-font metrics (`ParagraphAst::strut_metrics`) and the font size
    /// they were requested at (`ParagraphAst::strut_font_size`), using
    /// `options.line_height`. `line_index` is 0.
    #[must_use]
    pub fn from_options(
        raw_strut: &FontMetrics,
        strut_font_size: f32,
        base_direction: BidiDirection,
        options: &LayoutOptions,
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
    /// A glyph's `x`, `advance` or `y_offset` is NaN or infinite.
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
    /// `advance`, `y_offset`), every inline box (all coordinates and sizes) and
    /// every tab (`x`, `width`). A NaN or infinity, or a negative extent, box or
    /// tab size, is rejected at once. A negative glyph `advance` is *not*
    /// rejected: kerning can produce one.
    ///
    /// **Pass 2, envelope.** `visible_byte_end` lies within `byte_range`. An
    /// empty line (`byte_range` empty) has no runs, boxes or tabs, zero
    /// `visual_width` and hanging extents, and `line_height` equal to the
    /// resolved strut. Otherwise `line_height` equals the strut under
    /// `LineHeight::Fixed` and is at least the strut under `LineHeight::Normal`
    /// (fallback glyphs may enlarge it, §5 Stage 4.1).
    ///
    /// **Pass 3, containment and hanging.** `visual_width` is the body width,
    /// so the body is `[0, visual_width]`:
    /// * every non-hanging, non-trailing glyph, inline box and tab starts at
    ///   `>= 0` and ends at `<= visual_width` (within tolerance); zero-advance
    ///   glyphs are skipped, since `x` is a drawing origin that includes
    ///   `x_offset`;
    /// * every `is_hanging` glyph lies wholly outside the body;
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
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let finite = g.x.is_finite() && g.advance.is_finite() && g.y_offset.is_finite();
                if !(finite && (g.x + g.advance).is_finite()) {
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
        let expected = ctx.strut.natural_line_height();
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

        let (mut left, mut right) = (0.0_f32, 0.0_f32);
        let (mut has_left, mut has_right) = (false, false);
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
                // Zero-advance glyphs (combining marks) attach to a base and
                // carry no extent to contain.
                if g.advance == 0.0 {
                    continue;
                }
                let tol = tolerance(lo.abs().max(hi.abs()), vw);
                if g.is_hanging {
                    if lo < 0.0 && hi <= tol {
                        left = left.max(-lo);
                        has_left = true;
                    } else if lo >= vw - tol && hi > lo {
                        right = right.max(hi - vw);
                        has_right = true;
                    } else {
                        return Err(E::HangingGlyphInsideBlock { run, glyph });
                    }
                } else if lo < -tol || hi > vw + tol {
                    return Err(E::BodyGlyphOutsideBlock { run, glyph });
                }
            }
        }
        for (side, expected, actual, present) in [
            (HangSide::Left, left, self.hanging_left, has_left),
            (HangSide::Right, right, self.hanging_right, has_right),
        ] {
            if !approx_eq(expected, actual) {
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
        Ok(())
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
            y_offset: 0.0,
            advance,
            byte_range: Some(TextRange::new(0, 1)),
            is_synthesized: false,
            is_hanging,
        }
    }

    fn line(glyphs: Vec<PositionedGlyph>, width: f32, hang_l: f32, hang_r: f32) -> PositionedLine {
        PositionedLine {
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
                font: FontRef::unscaled(FontId(0)),
                bidi_level: 0,
                byte_range: TextRange::new(0, 4),
                glyphs,
            }],
            visual_boxes: vec![],
            tabs: vec![],
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
    fn zero_advance_marks_with_offsets_do_not_trip_containment() {
        let l = line(
            vec![glyph(-3.0, 0.0, false), glyph(0.0, 20.0, false)],
            20.0,
            0.0,
            0.0,
        );
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
    fn a_hanging_glyph_is_gated_even_when_its_extent_is_negligible() {
        // Extent 1e-4 sits inside the tolerance, so `hanging_left` is 0.0.
        let l = line(
            vec![glyph(-1.0e-4, 1.0e-4, true), glyph(0.0, 20.0, false)],
            20.0,
            0.0,
            0.0,
        );
        assert_eq!(
            l.validate(&no_hang()),
            Err(LineInvariantError::HangingNotAllowed {
                side: HangSide::Left
            })
        );
        assert_eq!(l.validate(&ctx()), Ok(()));
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
}
