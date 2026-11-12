//! Middle-end output: positioned lines and glyphs (spec §2.6).
//!
//! Data only. The invariants of these types are checked by
//! [`PositionedLine::validate`] and [`PositionedParagraph::validate`].

use super::{
    HangMode, HangingWhitespaceMode, InlineSlotConstraint, LayoutConstraints, LayoutInputError,
    LayoutOptions, LineClampPolicy, LineHeight, StrutMetrics,
};
use crate::frontend::{AnnotationSide, BoxOverflowPolicy, ParagraphAst};
use crate::types::{
    BaselineAlignment, BidiDirection, BidiLevel, ByteIndex, Direction, FontRef, FontVariation,
    GlyphOrientation, TextRange, WritingMode,
};
use smallvec::SmallVec;

/// One glyph placed on a line (spec §2.6).
///
/// `x` is **line-local**: measured from the line's left edge, so a line can be
/// moved without touching its glyphs. **[Design decision]** (spec silent).
/// It is the strict **pen advance position**, the cumulative advance along the
/// line, and not the drawing origin: the rasteriser draws the glyph at
/// `(line_origin_x + x + x_offset, baseline_y - y_offset)` (spec §2.6, geometry conventions). A
/// zero-advance combining mark with a negative `x_offset` at the start of a
/// line therefore sits at the content start of its slot. `x` may lie left of
/// that start for hanging punctuation placed outside the edge (§5 Stage 1.3),
/// see [`PositionedLine::hanging_left`]. Containment is judged on the pen
/// interval `[x, x + advance]`.
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
    /// Source bytes, non-empty. `None` exactly for a **synthesized** glyph,
    /// one with no source counterpart (an inserted hyphen, a repeated math
    /// operator, an ellipsis), and for annotation glyphs, which never claim
    /// body bytes (spec §1.5, A1). A glyph replacing source atoms after a
    /// boundary re-shape keeps their range; an actual U+00AD in the source
    /// is not synthesized.
    pub byte_range: Option<TextRange>,
    /// `true` for punctuation hanging outside the text block (§5 Stage 1.3).
    /// May straddle the body edge; its normalized pen interval must overflow
    /// the content of the line's outer slots beyond tolerance. Only the
    /// overflow counts as hanging.
    pub is_hanging: bool,
}

impl PositionedGlyph {
    /// `true` for a glyph with no source counterpart: `byte_range` is `None`,
    /// the only marker of synthesis (spec §1.5, A1). Synthesized glyphs are
    /// never copied to the clipboard (§6.6).
    #[must_use]
    pub const fn is_synthesized(&self) -> bool {
        self.byte_range.is_none()
    }
}

/// Glyphs of one font and one bidi level, in screen order.
///
/// **[Design decision]** The spec names `VisualGlyphRun` (§2.6) without
/// defining it; it groups glyphs that share a font so a painter can bind one
/// face per run.
#[derive(Clone, Debug, PartialEq)]
pub struct VisualGlyphRun {
    /// Resolved glyph orientation.
    pub orientation: GlyphOrientation,
    /// Requested size before fallback scaling.
    pub font_size: f32,
    /// Variable font instance coordinates.
    pub variations: SmallVec<[FontVariation; 2]>,
    /// Font and fallback scale shared by every glyph.
    pub font: FontRef,
    /// Bidi level shared by every glyph.
    pub bidi_level: BidiLevel,
    /// Source bytes covered by the run.
    pub byte_range: TextRange,
    /// Glyphs, screen left to right: pen positions never decrease
    /// (`glyphs[i].x <= glyphs[i + 1].x`; a mark shares its base's position
    /// and keeps its drawing offset in `x_offset`).
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
    pub baseline_align: BaselineAlignment,
    /// Caller-assigned identifier from `BoxNode::InlineObject::box_id`.
    pub box_id: u64,
    /// Source bytes the object stands for.
    pub byte_range: TextRange,
    /// Left edge, line-local. The object lies inside the content of one slot.
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
    /// The object's resolved bidi level (`BoxNode::InlineObject::bidi_level`).
    pub bidi_level: BidiLevel,
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
    /// Left edge, line-local. The tab lies inside the content of one slot,
    /// except for a trailing tab under `Preserved` (trailing-whitespace rule).
    pub x: f32,
    /// Width (non-negative). A tab stores no bidi level: UAX #9 rule L1 puts
    /// it at the paragraph level (spec §1.5, A2).
    pub width: f32,
}

/// The visual box of one line fragment of an inline decoration span (spec
/// §2.6), after bidi reordering.
///
/// Bidi reordering can split one span into several visual groups on a line;
/// each group is its own fragment. The fragment carries the span's whole
/// range, so [`is_first_fragment`](Self::is_first_fragment) and
/// [`is_last_fragment`](Self::is_last_fragment) follow from the fragment
/// alone: a painter that joins borders across fragments reads them from the
/// two ranges, which no stored flag can contradict (spec §1.5, A2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedSpanFragment {
    /// Identifier of the `InlineDecorationSpan`.
    pub span_id: u64,
    /// The whole `InlineDecorationSpan::byte_range`; the same on every
    /// fragment of the span.
    pub span_range: TextRange,
    /// Source bytes of the span that this fragment covers: inside
    /// `span_range` and inside the line.
    pub byte_range: TextRange,
    /// Left visual edge, line-local.
    pub x: f32,
    /// Visual width including the padding that applies (non-negative).
    pub width: f32,
    /// Extent above the baseline (non-negative).
    pub ascent: f32,
    /// Extent below the baseline (non-negative).
    pub descent: f32,
    /// UAX #9 level of the text in this fragment.
    pub bidi_level: BidiLevel,
}

impl PositionedSpanFragment {
    /// Whether the fragment holds the logical start of its span:
    /// `byte_range.start == span_range.start`.
    #[must_use]
    pub const fn is_first_fragment(&self) -> bool {
        self.byte_range.start == self.span_range.start
    }

    /// Whether the fragment holds the logical end of its span:
    /// `byte_range.end == span_range.end`.
    #[must_use]
    pub const fn is_last_fragment(&self) -> bool {
        self.byte_range.end == self.span_range.end
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
    /// Bidi level of the text the stop belongs to: the `bidi_level` of a
    /// visual run with a glyph whose bytes contain or touch `offset` (at a
    /// run boundary, of either adjacent run).
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
    /// Entire atomic logical range; the same on every fragment of the span.
    pub full_byte_range: TextRange,
    /// Source bytes on this visual fragment: non-empty, inside
    /// `full_byte_range` and inside the line. The fragments of one span on one
    /// line are disjoint and together cover the part of the span on that line.
    pub fragment_byte_range: TextRange,
    /// Physical left edge, line-local.
    pub x: f32,
    /// Extent to the right of `x` (non-negative).
    pub width: f32,
    /// Resolved fragment direction.
    pub bidi_level: BidiLevel,
}

/// The content placed in one available interval of a line.
///
/// The slot carries the interval it was laid out in, so a line describes its
/// own geometry: validation never queries a geometry provider again.
///
/// ```text
/// constraint.start      content_start                content_end      constraint.end
///        |<- alignment ->|<------- visual_width ------->|<- slack ------->|
/// ```
///
/// The content block `[content_start, content_start + visual_width]` lies
/// inside `constraint` (but see the overflow exemption on
/// [`PositionedLine::validate`]). What the constraint has outside the block is
/// alignment offset and slack; an unjustified or last line keeps its slack.
///
/// Inside the block the **line-end** edge is exact and the **line-start** edge
/// may be preceded by preserved leading glue, which has no output entity of
/// its own. In an LTR paragraph the last item ends at `content_end`
/// (`visual_width == max(right edge) - content_start`) and the gap
/// `[content_start, first item)` is leading glue; in an RTL paragraph the
/// mirror image holds: the leftmost item starts at `content_start` and the gap
/// `(last item, content_end]` is leading glue. A slot without content has
/// `visual_width == 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedSlot {
    /// The available interval, as resolved when the line was laid out.
    pub constraint: InlineSlotConstraint,
    /// Left edge of the content block; finite and inside `constraint`.
    pub content_start: f32,
    /// Advance of the content block; finite and non-negative.
    pub visual_width: f32,
}

impl PositionedSlot {
    /// Right edge of the content block: `content_start + visual_width`.
    #[must_use]
    pub fn content_end(&self) -> f32 {
        self.content_start + self.visual_width
    }
}

/// One independently shaped annotation attached to base text in one slot.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionedAnnotationRun {
    /// Source annotation identifier.
    pub annotation_id: u64,
    /// Nonempty visible base range.
    pub base_byte_range: TextRange,
    /// Physical line-over or line-under side.
    pub side: AnnotationSide,
    /// Font face and fallback scale.
    pub font: FontRef,
    /// Positive finite size.
    pub font_size: f32,
    /// Finite variation coordinates.
    pub variations: SmallVec<[FontVariation; 2]>,
    /// Resolved orientation.
    pub orientation: GlyphOrientation,
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

/// One laid-out line (spec §2.6).
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedLine {
    /// The line's slots, left to right: every available interval of the line
    /// with the content placed in it. Never empty.
    pub slots: SmallVec<[PositionedSlot; 1]>,
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

/// Which physical edge of a line a hanging extent belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HangSide {
    /// Physical left edge.
    Left,
    /// Physical right edge.
    Right,
}

impl HangSide {
    /// Whether this edge is where a line of base `direction` starts: the left
    /// one for LTR, the right one for RTL.
    #[must_use]
    pub const fn is_line_start(self, direction: Direction) -> bool {
        matches!(
            (self, direction),
            (Self::Left, Direction::Ltr) | (Self::Right, Direction::Rtl)
        )
    }
}

impl core::fmt::Display for HangSide {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Left => "left",
            Self::Right => "right",
        })
    }
}

/// The policy a paragraph was laid out under, recorded in the output so that
/// validating it needs nothing from the caller: a paragraph cannot be checked
/// against a more permissive policy than the one that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutPolicySnapshot {
    /// Writing mode of the paragraph (`ParagraphAst::writing_mode`).
    pub writing_mode: WritingMode,
    /// The **resolved** strut: `ParagraphAst::strut_metrics` after
    /// `LineHeight::resolve_strut`, half-leading included.
    pub strut: StrutMetrics,
    /// The line-height policy.
    pub line_height: LineHeight,
    /// The trailing-whitespace mode.
    pub whitespace: HangingWhitespaceMode,
    /// The hanging-punctuation mode.
    pub hang_mode: HangMode,
    /// `LayoutConstraints::wrap_width`: positive, possibly infinite. A line
    /// wider than this is valid only in the single-entity overflow state (see
    /// [`PositionedLine::validate`]).
    pub wrap_width: f32,
    /// Where the lines' slots came from.
    pub slot_source: SlotSource,
    /// The truncation policy, if any (`LayoutOptions::clamp`).
    pub clamp: Option<LineClampPolicy>,
}

/// Where a paragraph's slots came from, recorded so that validation can
/// rebuild the standard column (spec §5.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SlotSource {
    /// No geometry provider: every line has exactly the one slot
    /// `[indent, wrap_width]`, with `first_line_indent` on line 0 and
    /// `hanging_indent` after it. Both indents lie in `[0, wrap_width)`.
    Standard {
        /// `LayoutConstraints::first_line_indent`.
        first_line_indent: f32,
        /// `LayoutConstraints::hanging_indent`.
        hanging_indent: f32,
    },
    /// A `LineGeometryProvider` answered for every line. Its answers are not
    /// recomputable after layout, so only their shape is validated; the
    /// constraints' indents do not apply.
    Provider,
}

impl SlotSource {
    /// The one slot line `line_index` must have under
    /// [`Standard`](Self::Standard) in a container `wrap_width` wide; `None`
    /// for [`Provider`](Self::Provider).
    #[must_use]
    pub fn standard_slot(self, line_index: usize, wrap_width: f32) -> Option<InlineSlotConstraint> {
        match self {
            Self::Standard {
                first_line_indent,
                hanging_indent,
            } => Some(InlineSlotConstraint {
                start: if line_index == 0 {
                    first_line_indent
                } else {
                    hanging_indent
                },
                end: wrap_width,
            }),
            Self::Provider => None,
        }
    }
}

impl LayoutPolicySnapshot {
    /// Resolves the policy of one paragraph from the layout inputs. The strut
    /// is resolved here, once, from the AST's **raw** strut metrics,
    /// `ParagraphAst::strut_font_size`, the writing mode and
    /// `options.line_height`.
    ///
    /// # Errors
    /// `InvalidConstraints`, `InvalidLayoutOptions`, `InvalidStrut` or
    /// `InvalidFontMetrics` for inputs no policy can be resolved from.
    pub fn resolve(
        ast: &ParagraphAst,
        constraints: &LayoutConstraints,
        options: &LayoutOptions,
    ) -> Result<Self, LayoutInputError> {
        if !constraints.is_valid() {
            return Err(LayoutInputError::InvalidConstraints);
        }
        if options.clamp.as_ref().is_some_and(|c| !c.is_valid()) {
            return Err(LayoutInputError::InvalidLayoutOptions);
        }
        let raw = &ast.strut_metrics;
        let extents_are_valid = [raw.ascent, raw.descent, raw.line_gap]
            .iter()
            .all(|v| v.is_finite())
            && raw.ascent >= 0.0
            && raw.descent >= 0.0
            && raw.ascent + raw.descent > 0.0;
        let font_size_is_valid = ast.strut_font_size.is_finite() && ast.strut_font_size > 0.0;
        if !(extents_are_valid && font_size_is_valid && options.line_height.is_valid()) {
            return Err(LayoutInputError::InvalidStrut);
        }
        if !raw.is_valid() {
            return Err(LayoutInputError::InvalidFontMetrics);
        }
        let strut = options
            .line_height
            .resolve_strut(raw, ast.strut_font_size, ast.writing_mode);
        if !strut.is_valid() {
            return Err(LayoutInputError::InvalidStrut);
        }
        Ok(Self {
            writing_mode: ast.writing_mode,
            strut,
            line_height: options.line_height,
            whitespace: options.whitespace,
            hang_mode: options.hang_mode,
            wrap_width: constraints.wrap_width,
            slot_source: if constraints.geometry_provider.is_some() {
                SlotSource::Provider
            } else {
                SlotSource::Standard {
                    first_line_indent: constraints.first_line_indent,
                    hanging_indent: constraints.hanging_indent,
                }
            },
            clamp: options.clamp.clone(),
        })
    }

    /// `true` when the snapshot could have come from
    /// [`resolve`](Self::resolve): a valid strut, line height and clamp, a
    /// positive wrap width, and standard indents inside `[0, wrap_width)`.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let indents_are_valid = match self.slot_source {
            SlotSource::Standard {
                first_line_indent,
                hanging_indent,
            } => [first_line_indent, hanging_indent]
                .iter()
                .all(|&indent| indent >= 0.0 && indent < self.wrap_width),
            SlotSource::Provider => true,
        };
        self.strut.is_valid()
            && self.line_height.is_valid()
            && self.wrap_width > 0.0
            && indents_are_valid
            && self.clamp.as_ref().is_none_or(LineClampPolicy::is_valid)
    }

    /// The context line `line_index` of a paragraph of base `base_direction`
    /// is validated against.
    #[must_use]
    pub fn line_context(&self, base_direction: BidiDirection, line_index: usize) -> LineContext {
        LineContext {
            writing_mode: self.writing_mode,
            base_direction,
            strut: self.strut,
            line_height: self.line_height,
            whitespace: self.whitespace,
            hang_mode: self.hang_mode,
            wrap_width: self.wrap_width,
            line_index,
        }
    }
}

/// What one line is validated against: the paragraph's policy
/// ([`LayoutPolicySnapshot`]), its base direction and the line's index.
///
/// There is no default context: an all-zero strut validates no line. Build one
/// with [`LineContext::for_paragraph`] or
/// [`LayoutPolicySnapshot::line_context`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineContext {
    /// Active writing mode.
    pub writing_mode: WritingMode,
    /// The paragraph base direction. It decides which physical edge is the
    /// line start (hanging, leading glue) and where trailing whitespace lies
    /// (UAX #9 L1).
    pub base_direction: BidiDirection,
    /// The **resolved** strut.
    pub strut: StrutMetrics,
    /// The active line-height policy.
    pub line_height: LineHeight,
    /// The active trailing-whitespace mode.
    pub whitespace: HangingWhitespaceMode,
    /// The active hanging-punctuation mode.
    pub hang_mode: HangMode,
    /// The paragraph's `LayoutConstraints::wrap_width`.
    pub wrap_width: f32,
    /// Index of the line within the paragraph; `0` is the first line, which
    /// `HangStartMode::FirstLine` refers to.
    pub line_index: usize,
}

impl LineContext {
    /// The context of line `line_index` of the paragraph `ast` laid out under
    /// `constraints` and `options`.
    ///
    /// # Errors
    /// As [`LayoutPolicySnapshot::resolve`].
    pub fn for_paragraph(
        ast: &ParagraphAst,
        constraints: &LayoutConstraints,
        options: &LayoutOptions,
        line_index: usize,
    ) -> Result<Self, LayoutInputError> {
        LayoutPolicySnapshot::resolve(ast, constraints, options)
            .map(|policy| policy.line_context(ast.base_direction, line_index))
    }
}

/// Positioned lines in canonical flow space, with paragraph-local physical mapping.
/// The lines and `total_block_size` are the paragraph's extent; block transforms
/// and validation use `total_block_size` as the authoritative block extent.
///
/// The paragraph records the policy it was laid out under, so
/// [`validate`](Self::validate) needs only the text.
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedParagraph {
    /// The policy the lines were laid out under, writing mode included.
    pub policy: LayoutPolicySnapshot,
    /// Paragraph base direction, copied from the AST.
    pub base_direction: BidiDirection,
    /// Unrendered logical tail, when clamped.
    pub truncated_range: Option<TextRange>,
    /// Final block-axis extent including clearance.
    pub total_block_size: f32,
    /// Lines top to bottom.
    pub lines: Vec<PositionedLine>,
}

impl PositionedLine {
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
}
