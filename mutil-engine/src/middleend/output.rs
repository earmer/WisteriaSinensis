//! Middle-end output: positioned lines and glyphs (spec §2.6).

use super::CaretStop;
use crate::types::{BidiDirection, BidiLevel, ByteIndex, FontRef, Size, TextRange};

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
    /// Pen x position from the line's left edge.
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

/// One laid-out line (spec §2.6).
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedLine {
    /// Source bytes, **including** trailing whitespace.
    pub byte_range: TextRange,
    /// End of the last visible cluster (trailing collapsible space excluded).
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
    /// Caret stops in visual order (§6.1).
    pub caret_stops: Vec<CaretStop>,
    /// Decorations on this line.
    pub decorations: Vec<DecorationLine>,
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
