//! Line-height policy and the resolved paragraph strut (spec §5 Stage 4.1).

use crate::types::{FontMetrics, WritingMode, is_measurement};

/// Line height policy (spec §5 Stage 4.1; CSS Inline Layout 3 `[B7]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LineHeight {
    /// `line-height: normal`: the union of the glyph bounds of every font used,
    /// plus the font line gap.
    #[default]
    Normal,
    /// A multiple of the font size taken from the first available font only;
    /// fallback glyphs may overflow the line box (standard CSS behaviour). The
    /// multiplier must be finite and positive ([`LineHeight::is_valid`]).
    Fixed(f32),
}

impl LineHeight {
    /// `true` for `Normal`, and for `Fixed(m)` when `m` is finite and positive.
    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::Normal => true,
            Self::Fixed(multiplier) => multiplier.is_finite() && multiplier > 0.0,
        }
    }

    /// Resolves the paragraph strut from the raw metrics of `font` at
    /// `font_size`, for text set in `mode` (spec §5 Stage 4.1). A vertical
    /// mode reads the font's vertical ascent, descent and line gap.
    ///
    /// * `Normal`: the font's own ascent, descent and line gap.
    /// * `Fixed(m)`: total height `m * font_size`; the leading
    ///   `total - (ascent + descent)` (possibly negative) is split in half above
    ///   and below, and the line gap becomes 0 (CSS half-leading).
    ///
    /// [`StrutMetrics::line_height`] of the result is the height of an empty
    /// line.
    #[must_use]
    pub fn resolve_strut(
        self,
        font: &FontMetrics,
        font_size: f32,
        mode: WritingMode,
    ) -> StrutMetrics {
        let font_strut = if mode.is_vertical() {
            StrutMetrics {
                ascent: font.vertical_ascent,
                descent: font.vertical_descent,
                line_gap: font.vertical_line_gap,
            }
        } else {
            StrutMetrics {
                ascent: font.ascent,
                descent: font.descent,
                line_gap: font.line_gap,
            }
        };
        match self {
            Self::Normal => font_strut,
            Self::Fixed(multiplier) => {
                let half_leading =
                    (multiplier * font_size - (font_strut.ascent + font_strut.descent)) / 2.0;
                StrutMetrics {
                    ascent: font_strut.ascent + half_leading,
                    descent: font_strut.descent + half_leading,
                    line_gap: 0.0,
                }
            }
        }
    }
}

/// The resolved paragraph strut: the line box every line has at least
/// (`LineHeight::Normal`) or exactly (`LineHeight::Fixed`), produced by
/// [`LineHeight::resolve_strut`] (spec §5 Stage 4.1).
///
/// Distinct from the raw [`FontMetrics`] it is resolved from: half-leading is
/// folded into `ascent` and `descent`, so under a tight `Fixed` multiplier
/// either may be **negative**, which raw font metrics never are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrutMetrics {
    /// Extent of the strut toward line-over, half-leading included.
    pub ascent: f32,
    /// Extent of the strut toward line-under, half-leading included.
    pub descent: f32,
    /// Font line gap; zero under `LineHeight::Fixed`.
    pub line_gap: f32,
}

impl StrutMetrics {
    /// Height of an empty line: `ascent + descent + line_gap`.
    #[must_use]
    pub fn line_height(&self) -> f32 {
        self.ascent + self.descent + self.line_gap
    }

    /// `true` when every field and the line height are measurements
    /// (spec MSR-1) and the line height is positive.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let height = self.line_height();
        [self.ascent, self.descent, self.line_gap, height]
            .into_iter()
            .all(is_measurement)
            && height > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strut_resolution_folds_half_leading() {
        let font = FontMetrics {
            vertical_ascent: 5.0,
            vertical_descent: 5.0,
            vertical_line_gap: 0.0,
            ascent: 8.0,
            descent: 2.0,
            line_gap: 1.0,
            ..Default::default()
        };
        let horizontal = WritingMode::HorizontalTb;
        let normal = LineHeight::Normal.resolve_strut(&font, 10.0, horizontal);
        assert_eq!(normal.line_height(), 11.0);
        let fixed = LineHeight::Fixed(1.5).resolve_strut(&font, 10.0, horizontal);
        assert_eq!(
            fixed,
            StrutMetrics {
                ascent: 10.5,
                descent: 4.5,
                line_gap: 0.0
            }
        );
        assert_eq!(fixed.line_height(), 15.0);
        // A line height below the glyph extent gives negative leading, and a
        // descent no raw font metric could have.
        let tight = LineHeight::Fixed(0.5).resolve_strut(&font, 10.0, horizontal);
        assert_eq!(tight.line_height(), 5.0);
        assert_eq!(tight.descent, -0.5);
        assert!(tight.is_valid());
        // A vertical mode reads the vertical metrics.
        let vertical = LineHeight::Normal.resolve_strut(&font, 10.0, WritingMode::VerticalRl);
        assert_eq!(
            vertical,
            StrutMetrics {
                ascent: 5.0,
                descent: 5.0,
                line_gap: 0.0
            }
        );
    }

    #[test]
    fn a_strut_needs_a_finite_positive_height() {
        let strut = |ascent, descent, line_gap| StrutMetrics {
            ascent,
            descent,
            line_gap,
        };
        assert!(strut(8.0, 2.0, 1.0).is_valid());
        assert!(!strut(0.0, 0.0, 0.0).is_valid());
        assert!(!strut(-8.0, 2.0, 0.0).is_valid());
        assert!(!strut(f32::NAN, 2.0, 0.0).is_valid());
        assert!(!strut(f32::MAX, f32::MAX, 0.0).is_valid());
    }

    #[test]
    fn line_height_multipliers_must_be_finite_and_positive() {
        assert!(LineHeight::Normal.is_valid());
        assert!(LineHeight::Fixed(1.5).is_valid());
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!LineHeight::Fixed(bad).is_valid(), "{bad}");
        }
    }
}
