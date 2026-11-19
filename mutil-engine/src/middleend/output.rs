//! Middle-end output: positioned lines and glyphs (spec §2.6), checked by
//! [`PositionedLine::validate`] and [`PositionedParagraph::validate`].

use super::{
    HangMode, HangingWhitespaceMode, InlineSlotConstraint, LayoutConstraints, LayoutInputError,
    LayoutOptions, LineClampPolicy, LineHeight, StrutMetrics,
};
use crate::frontend::{AnnotationSide, BoxOverflowPolicy, ParagraphAst};
use crate::types::{
    BaselineAlignment, BidiDirection, BidiLevel, ByteIndex, Direction, FontRef, FontVariation,
    GlyphOrientation, TextRange, WritingMode, is_extent, is_measurement,
};
use smallvec::SmallVec;

/// One glyph placed on a line (spec §2.6). `x` is line-local (OUT-1; `0` at
/// every line's envelope start, SLT-1, GEO-3) and is the pen position, not the
/// drawing origin: the flow-space origin is `x + x_offset` and
/// `baseline_y - y_offset`, except in `VerticalLr` (OUT-1, ORI-2). Renderers
/// take the physical origin from [`PlacedRun::glyphs`](super::PlacedRun::glyphs)
/// (HST-4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// Backend glyph index in the run's font.
    pub glyph_id: u32,
    /// Pen position from the line's left edge.
    pub x: f32,
    /// Inline drawing offset, positive towards the inline end (SHP-19).
    pub x_offset: f32,
    /// Drawing offset towards line-over, unclamped (SHP-19); its block
    /// direction depends on the writing mode (ORI-2).
    pub y_offset: f32,
    /// Advance width; may be negative.
    pub advance: f32,
    /// Non-empty source bytes; `None` exactly for synthesized and annotation
    /// glyphs, which claim no body bytes (spec §1.5, AX-1).
    pub byte_range: Option<TextRange>,
}

impl PositionedGlyph {
    /// No source counterpart (AX-1); never copied to the clipboard (§6.6).
    #[must_use]
    pub const fn is_synthesized(&self) -> bool {
        self.byte_range.is_none()
    }

    /// The normalized pen interval `(lo, hi)` (OUT-1).
    #[must_use]
    pub(crate) fn pen_interval(&self) -> (f32, f32) {
        let end = self.x + self.advance;
        (self.x.min(end), self.x.max(end))
    }
}

/// Glyphs of one font and one bidi level, in screen order (OUT-3).
/// **[Design decision]**: §2.6 names this type without defining it.
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
    /// Glyphs, screen left to right; pen positions never decrease.
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

/// A decoration segment, merged across adjacent clusters and glue (§5 Stage
/// 5). The painter looks its colour up from `byte_range`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecorationLine {
    /// Underline or strike-through.
    pub kind: DecorationKind,
    /// Source bytes decorated.
    pub byte_range: TextRange,
    /// Left end, line-local.
    pub x: f32,
    /// Non-negative extent to the right of `x`.
    pub width: f32,
    /// Offset from the baseline.
    pub y_offset: f32,
    /// Stroke thickness.
    pub thickness: f32,
}

/// An inline object placed on a line (spec §2.4 `InlineBox`, §2.6).
/// **[Design decision]**: the spec left object positions unrepresented.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedInlineBox {
    /// Alignment relative to the main line baseline.
    pub baseline_align: BaselineAlignment,
    /// `BoxNode::InlineObject::box_id`.
    pub box_id: u64,
    /// Source bytes the object stands for.
    pub byte_range: TextRange,
    /// Left edge, line-local.
    pub x: f32,
    /// Offset of the object's baseline below the line's baseline.
    pub y: f32,
    /// Non-negative width.
    pub width: f32,
    /// Non-negative extent above the object's baseline.
    pub ascent: f32,
    /// Non-negative extent below the object's baseline.
    pub descent: f32,
    /// `BoxNode::InlineObject::overflow_policy` (SLT-5, WN-7).
    pub overflow_policy: BoxOverflowPolicy,
    /// `BoxNode::InlineObject::bidi_level`.
    pub bidi_level: BidiLevel,
}

/// A tab placed on a line (spec §2.4 `Tab`, §2.6). It stores no bidi level:
/// UAX #9 L1 puts it at the paragraph level (AX-2). **[Design decision]**.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedTab {
    /// Source bytes of the tab character.
    pub byte_range: TextRange,
    /// Left edge, line-local.
    pub x: f32,
    /// Non-negative width, `TabStops::next_stop(d) - d` where `d` is the tab's
    /// distance from the slot's line-start edge (not `x`); zero when no stop
    /// is reachable (ST3-1).
    pub width: f32,
}

/// The visual box of one bidi-reordered line fragment of an inline decoration
/// span (spec §2.6). First/last-ness follows from the two ranges (AX-2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedSpanFragment {
    /// `InlineDecorationSpan` identifier.
    pub span_id: u64,
    /// The whole `InlineDecorationSpan::byte_range`.
    pub span_range: TextRange,
    /// The span's bytes in this fragment, inside the line.
    pub byte_range: TextRange,
    /// Left visual edge, line-local.
    pub x: f32,
    /// Non-negative visual width, padding included.
    pub width: f32,
    /// Non-negative extent above the baseline.
    pub ascent: f32,
    /// Non-negative extent below the baseline.
    pub descent: f32,
    /// UAX #9 level of the fragment's text.
    pub bidi_level: BidiLevel,
}

impl PositionedSpanFragment {
    /// The fragment holds the logical start of its span.
    #[must_use]
    pub const fn is_first_fragment(&self) -> bool {
        self.byte_range.start() == self.span_range.start()
    }

    /// The fragment holds the logical end of its span.
    #[must_use]
    pub const fn is_last_fragment(&self) -> bool {
        self.byte_range.end() == self.span_range.end()
    }
}

/// Which side of an ambiguous boundary (soft wrap, bidi) a caret belongs to
/// (spec §6.1); upstream leans toward the start of the string.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaretAffinity {
    /// Leans toward the following text.
    #[default]
    Downstream,
    /// Leans toward the preceding text.
    Upstream,
}

/// One visual boundary on a line, in visual order (spec §6.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretStop {
    /// Line-local x position.
    pub x: f32,
    /// Logical byte offset.
    pub offset: ByteIndex,
    /// Affinity this stop represents.
    pub affinity: CaretAffinity,
    /// Level of a glyph's visual run, inline box or tab (paragraph level)
    /// touching `offset`; otherwise the paragraph level or that of the
    /// nearest entity, as on an empty line (CRT-2).
    pub level: BidiLevel,
}

/// Visual fragment of an atomic edit span, independent of decorations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtomicFragment {
    /// Atomic span id (independent id space).
    pub atomic_id: u64,
    /// The span's whole logical range.
    pub full_byte_range: TextRange,
    /// Non-empty bytes of this fragment; disjoint from the span's others.
    pub fragment_byte_range: TextRange,
    /// Physical left edge, line-local.
    pub x: f32,
    /// Non-negative extent to the right of `x`.
    pub width: f32,
    /// Resolved fragment direction.
    pub bidi_level: BidiLevel,
}

/// The content placed in one available interval of a line (SLT-1 to SLT-3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedSlot {
    /// The available interval the line was laid out in.
    pub constraint: InlineSlotConstraint,
    /// Left edge of the content block.
    pub content_start: f32,
    /// Advance of the content block.
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
    /// Non-empty visible base range.
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
    /// Largest finite non-negative overhang beyond the base.
    pub max_overhang: f32,
    /// Declared additional block extent, gap included.
    pub block_extent: f32,
    /// Non-negative over-side extent of the base's glyphs and boxes.
    pub base_over: f32,
    /// The under-side mirror of `base_over`.
    pub base_under: f32,
    /// Synthesized glyphs shaped from the annotation's own text.
    pub glyphs: Vec<PositionedGlyph>,
}

/// One laid-out line (spec §2.6).
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedLine {
    /// One slot per available interval, left to right; never empty.
    pub slots: SmallVec<[PositionedSlot; 1]>,
    /// Whether the suffix represents clamped source.
    pub is_ellipsized: bool,
    /// Interlinear glyph sequences.
    pub annotations: Vec<PositionedAnnotationRun>,
    /// Atomic editing geometry.
    pub atomic_fragments: Vec<AtomicFragment>,
    /// Source bytes, trailing whitespace included.
    pub byte_range: TextRange,
    /// End of the visible content (clusters, inline boxes, tabs); trailing
    /// whitespace starts here.
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
    /// Inline objects, in logical order.
    pub visual_boxes: Vec<PositionedInlineBox>,
    /// Tabs, in logical order.
    pub tabs: Vec<PositionedTab>,
    /// Inline decoration span boxes, screen left to right.
    pub span_fragments: Vec<PositionedSpanFragment>,
    /// Caret stops in visual order (§6.1).
    pub caret_stops: Vec<CaretStop>,
    /// Decorations on this line.
    pub decorations: Vec<DecorationLine>,
}

impl PositionedLine {
    /// The extreme caret stop towards `side` (spec §6.3 rule 3); equal-`x`
    /// ties go to the first stop for `Left` and the last for `Right`.
    #[must_use]
    pub fn extreme_caret_stop(&self, side: HangSide) -> Option<&CaretStop> {
        let by_x = |a: &&CaretStop, b: &&CaretStop| {
            a.x.partial_cmp(&b.x).unwrap_or(core::cmp::Ordering::Equal)
        };
        match side {
            HangSide::Left => self.caret_stops.iter().min_by(by_x),
            HangSide::Right => self.caret_stops.iter().max_by(by_x),
        }
    }
}

/// A physical edge of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HangSide {
    /// Physical left edge.
    Left,
    /// Physical right edge.
    Right,
}

impl HangSide {
    /// Whether a line of base `direction` starts at this edge.
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

/// Positioned lines in canonical flow space (§2.7), with the policy they were
/// laid out under so that [`validate`](Self::validate) needs only the text.
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedParagraph {
    /// The policy the lines were laid out under.
    pub policy: LayoutPolicySnapshot,
    /// Paragraph base direction, copied from the AST.
    pub base_direction: BidiDirection,
    /// Unrendered logical tail, when clamped.
    pub truncated_range: Option<TextRange>,
    /// Authoritative block-axis extent, clearance included.
    pub total_block_size: f32,
    /// Lines top to bottom.
    pub lines: Vec<PositionedLine>,
}

/// The policy a paragraph was laid out under, recorded so that it cannot be
/// validated against a more permissive one.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutPolicySnapshot {
    /// `ParagraphAst::writing_mode`.
    pub writing_mode: WritingMode,
    /// The strut after `LineHeight::resolve_strut`.
    pub strut: StrutMetrics,
    /// The line-height policy.
    pub line_height: LineHeight,
    /// The trailing-whitespace mode.
    pub whitespace: HangingWhitespaceMode,
    /// The hanging-punctuation mode.
    pub hang_mode: HangMode,
    /// `LayoutConstraints::wrap_width`: positive, possibly infinite (SLT-5).
    pub wrap_width: f32,
    /// Where the lines' slots came from.
    pub slot_source: SlotSource,
    /// `LayoutOptions::clamp`.
    pub clamp: Option<LineClampPolicy>,
}

/// Where a paragraph's slots came from (spec §5, GEO-2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SlotSource {
    /// No geometry provider: each line has the one slot `[indent, wrap_width]`.
    Standard {
        /// `LayoutConstraints::first_line_indent`, for line 0.
        first_line_indent: f32,
        /// `LayoutConstraints::hanging_indent`, for later lines.
        hanging_indent: f32,
    },
    /// A `LineGeometryProvider`; only the shape of its answers is validated.
    Provider,
}

impl SlotSource {
    /// The one slot of line `line_index` under [`Standard`](Self::Standard),
    /// if the indent leaves a well-formed one.
    #[must_use]
    pub fn standard_slot(self, line_index: usize, wrap_width: f32) -> Option<InlineSlotConstraint> {
        let Self::Standard {
            first_line_indent,
            hanging_indent,
        } = self
        else {
            return None;
        };
        let indent = match line_index {
            0 => first_line_indent,
            _ => hanging_indent,
        };
        InlineSlotConstraint::try_new(indent, wrap_width)
    }

    /// The slot source `constraints` lay lines out from.
    pub(crate) fn of(constraints: &LayoutConstraints) -> Self {
        match constraints.geometry_provider {
            Some(_) => Self::Provider,
            None => Self::Standard {
                first_line_indent: constraints.first_line_indent,
                hanging_indent: constraints.hanging_indent,
            },
        }
    }
}

impl LayoutPolicySnapshot {
    /// Resolves a paragraph's policy, resolving the strut once from the AST's
    /// raw strut metrics and `options.line_height`.
    ///
    /// # Errors
    /// `InvalidConstraints`, `InvalidLayoutOptions`, `InvalidStrut` or
    /// `InvalidFontMetrics` for inputs no policy can be resolved from. Raw
    /// strut metrics fail as `InvalidStrut` (the strut's own ascent, descent
    /// and line gap) or `InvalidFontMetrics` (the rest), never as the shaper
    /// gate's `InvalidShaperMetrics`.
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
        let (raw, font_size) = (&ast.strut_metrics, ast.strut_font_size);
        if !(is_extent(raw.ascent)
            && is_extent(raw.descent)
            && is_measurement(raw.line_gap)
            && raw.ascent + raw.descent > 0.0
            && is_extent(font_size)
            && font_size > 0.0
            && options.line_height.is_valid())
        {
            return Err(LayoutInputError::InvalidStrut);
        }
        if !raw.is_valid() {
            return Err(LayoutInputError::InvalidFontMetrics);
        }
        let strut = Some(
            options
                .line_height
                .resolve_strut(raw, font_size, ast.writing_mode),
        )
        .filter(StrutMetrics::is_valid)
        .ok_or(LayoutInputError::InvalidStrut)?;
        Ok(Self {
            writing_mode: ast.writing_mode,
            strut,
            line_height: options.line_height,
            whitespace: options.whitespace,
            hang_mode: options.hang_mode,
            wrap_width: constraints.wrap_width,
            slot_source: SlotSource::of(constraints),
            clamp: options.clamp.clone(),
        })
    }

    /// The PAR-1 field checks: a valid strut whose box (`ascent + descent`)
    /// is not inverted and, under `LineHeight::Fixed`, has no line gap; a
    /// valid line height and clamp; a positive wrap width; and standard
    /// indents in `[0, wrap_width)`. Every snapshot
    /// [`resolve`](Self::resolve) produces passes; passing does not prove
    /// `resolve` produced it.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let indents_are_valid = match self.slot_source {
            SlotSource::Standard {
                first_line_indent,
                hanging_indent,
            } => [first_line_indent, hanging_indent]
                .into_iter()
                .all(|indent| (0.0..self.wrap_width).contains(&indent)),
            SlotSource::Provider => true,
        };
        let strut_box_is_valid = self.strut.ascent + self.strut.descent >= 0.0
            && (!matches!(self.line_height, LineHeight::Fixed(_)) || self.strut.line_gap == 0.0);
        self.strut.is_valid()
            && strut_box_is_valid
            && self.line_height.is_valid()
            && self.wrap_width > 0.0
            && indents_are_valid
            && self.clamp.as_ref().is_none_or(LineClampPolicy::is_valid)
    }

    /// The context line `line_index` is validated against.
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

/// What one line is validated against: its paragraph's
/// [`LayoutPolicySnapshot`], base direction and the line's index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineContext {
    /// Active writing mode.
    pub writing_mode: WritingMode,
    /// Paragraph base direction.
    pub base_direction: BidiDirection,
    /// The resolved strut.
    pub strut: StrutMetrics,
    /// The line-height policy.
    pub line_height: LineHeight,
    /// The trailing-whitespace mode.
    pub whitespace: HangingWhitespaceMode,
    /// The hanging-punctuation mode.
    pub hang_mode: HangMode,
    /// `LayoutConstraints::wrap_width`.
    pub wrap_width: f32,
    /// Index of the line in the paragraph (`HangStartMode::FirstLine` is 0).
    pub line_index: usize,
}

impl LineContext {
    /// The context of line `line_index` of `ast` under `constraints`, `options`.
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
