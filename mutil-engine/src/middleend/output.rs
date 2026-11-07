//! Middle-end output: positioned lines and glyphs (spec §2.6).

use super::CaretStop;
use crate::types::{
    BidiDirection, BidiLevel, ByteIndex, FontRef, Size, TextRange, VerticalMetrics,
};

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

/// Tolerance for comparing summed `f32` widths in [`PositionedLine::validate`].
/// Heights copied from the strut are compared exactly.
pub const LINE_GEOMETRY_EPSILON: f32 = 1e-3;

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
pub enum LineInvariantError {
    /// `visual_width`, `hanging_left` or `hanging_right` is negative, NaN or infinite.
    NonFiniteOrNegativeExtent,
    /// An empty line (`byte_range.start == byte_range.end`) has visual runs.
    EmptyLineHasVisualRuns,
    /// An empty line's `line_height` differs from the strut's.
    EmptyLineHeightMismatch {
        /// `VerticalMetrics::natural_line_height()` of the strut.
        expected: f32,
        /// The line's `line_height`.
        actual: f32,
    },
    /// An empty line has a non-zero `visual_width` or hanging extent.
    EmptyLineHasExtent,
    /// A glyph flagged `is_hanging` overlaps the text block, i.e. hanging
    /// punctuation is being counted inside `visual_width`.
    HangingGlyphInsideBlock {
        /// Index into `visual_runs`.
        run: usize,
        /// Index into that run's `glyphs`.
        glyph: usize,
    },
    /// A body glyph (not `is_hanging`) lies outside `[0, visual_width]`.
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
}

impl PositionedLine {
    /// Checks the line invariants (spec §5 Stage 1.3, Stage 4.2). `strut` is the
    /// paragraph's `strut_metrics`, already resolved for the active `LineHeight`
    /// policy (half-leading folded into ascent and descent, as in CSS Inline 3).
    ///
    /// 1. **Empty line** (`byte_range` empty): no `visual_runs`;
    ///    `line_height == strut.natural_line_height()` exactly; zero
    ///    `visual_width` and hanging extents.
    /// 2. **`visual_width` is the body width only.** Every `is_hanging` glyph lies
    ///    wholly outside `[0, visual_width]`; every other glyph lies inside it.
    ///    Hanging punctuation therefore cannot be counted in `visual_width`.
    ///    (Equality with the body width is not derivable from glyphs alone,
    ///    because inline boxes and tabs occupy width without glyphs; the layout
    ///    stage owns that equality.)
    /// 3. **`hanging_left`/`hanging_right` are positive iff** the line has
    ///    left/right hanging glyphs, and equal the extent those glyphs occupy
    ///    outside the block (within [`LINE_GEOMETRY_EPSILON`]).
    ///
    /// # Errors
    /// The first violated invariant.
    pub fn validate(&self, strut: &VerticalMetrics) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let ok = |v: f32| v.is_finite() && v >= 0.0;
        if !(ok(self.visual_width) && ok(self.hanging_left) && ok(self.hanging_right)) {
            return Err(E::NonFiniteOrNegativeExtent);
        }
        if self.byte_range.is_empty() {
            if !self.visual_runs.is_empty() {
                return Err(E::EmptyLineHasVisualRuns);
            }
            let expected = strut.natural_line_height();
            #[allow(clippy::float_cmp)] // exact by contract: the value is copied
            if self.line_height != expected {
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

        let eps = LINE_GEOMETRY_EPSILON;
        let (mut left, mut right) = (0.0_f32, 0.0_f32);
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let end = g.x + g.advance;
                if g.is_hanging {
                    if g.x < 0.0 && end <= eps {
                        left = left.max(-g.x);
                    } else if g.x >= self.visual_width - eps && end > g.x {
                        right = right.max(end - self.visual_width);
                    } else {
                        return Err(E::HangingGlyphInsideBlock { run, glyph });
                    }
                } else if g.x < -eps || end > self.visual_width + eps {
                    return Err(E::BodyGlyphOutsideBlock { run, glyph });
                }
            }
        }
        for (side, expected, actual) in [
            (HangSide::Left, left, self.hanging_left),
            (HangSide::Right, right, self.hanging_right),
        ] {
            if (expected - actual).abs() > eps {
                return Err(E::HangingExtentMismatch {
                    side,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }

    /// `true` when [`PositionedLine::validate`] succeeds.
    #[must_use]
    pub fn is_valid(&self, strut: &VerticalMetrics) -> bool {
        self.validate(strut).is_ok()
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

    #[test]
    fn empty_line_contract() {
        assert!(empty_line(16.0).is_valid(&STRUT));
        assert_eq!(
            empty_line(15.0).validate(&STRUT),
            Err(LineInvariantError::EmptyLineHeightMismatch {
                expected: 16.0,
                actual: 15.0
            })
        );
        let mut with_runs = line(vec![glyph(0.0, 5.0, false)], 5.0, 0.0, 0.0);
        with_runs.byte_range = TextRange::empty_at(0);
        assert_eq!(
            with_runs.validate(&STRUT),
            Err(LineInvariantError::EmptyLineHasVisualRuns)
        );
        let mut wide = empty_line(16.0);
        wide.visual_width = 1.0;
        assert_eq!(
            wide.validate(&STRUT),
            Err(LineInvariantError::EmptyLineHasExtent)
        );
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
        assert!(l.is_valid(&STRUT));
    }

    #[test]
    fn counting_hanging_punctuation_in_visual_width_is_rejected() {
        // visual_width swallowed the hanging full stop: it no longer lies outside.
        let l = line(
            vec![glyph(0.0, 20.0, false), glyph(20.0, 5.0, true)],
            25.0,
            0.0,
            0.0,
        );
        assert_eq!(
            l.validate(&STRUT),
            Err(LineInvariantError::HangingGlyphInsideBlock { run: 0, glyph: 1 })
        );
    }

    #[test]
    fn hanging_extent_is_positive_iff_hanging_glyphs_exist() {
        // Extent claimed, nothing hangs.
        let none = line(vec![glyph(0.0, 20.0, false)], 20.0, 0.0, 5.0);
        assert_eq!(
            none.validate(&STRUT),
            Err(LineInvariantError::HangingExtentMismatch {
                side: HangSide::Right,
                expected: 0.0,
                actual: 5.0
            })
        );
        // Hanging glyph present, extent left at zero.
        let missing = line(
            vec![glyph(0.0, 20.0, false), glyph(20.0, 5.0, true)],
            20.0,
            0.0,
            0.0,
        );
        assert_eq!(
            missing.validate(&STRUT),
            Err(LineInvariantError::HangingExtentMismatch {
                side: HangSide::Right,
                expected: 5.0,
                actual: 0.0
            })
        );
        // Wrong width on the left.
        let wrong = line(
            vec![glyph(-6.0, 6.0, true), glyph(0.0, 20.0, false)],
            20.0,
            3.0,
            0.0,
        );
        assert!(matches!(
            wrong.validate(&STRUT),
            Err(LineInvariantError::HangingExtentMismatch {
                side: HangSide::Left,
                ..
            })
        ));
    }

    #[test]
    fn body_glyphs_must_stay_inside_the_block_and_extents_must_be_sane() {
        let outside = line(vec![glyph(0.0, 30.0, false)], 20.0, 0.0, 0.0);
        assert_eq!(
            outside.validate(&STRUT),
            Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
        );
        let negative = line(vec![glyph(0.0, 5.0, false)], 5.0, -1.0, 0.0);
        assert_eq!(
            negative.validate(&STRUT),
            Err(LineInvariantError::NonFiniteOrNegativeExtent)
        );
        let nan = line(vec![glyph(0.0, 5.0, false)], f32::NAN, 0.0, 0.0);
        assert_eq!(
            nan.validate(&STRUT),
            Err(LineInvariantError::NonFiniteOrNegativeExtent)
        );
    }
}
