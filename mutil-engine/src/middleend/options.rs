//! Layout options (spec §5 Stage 1.3 and Stage 4).

use super::HangSide;
use crate::types::{Direction, FontMetrics, VerticalMetrics};

/// Hanging at the trailing edge of a line (spec §5 Stage 1.3; CSS Text 4
/// `hanging-punctuation: force-end | allow-end` `[W2]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangEndMode {
    /// Nothing hangs at the line end.
    #[default]
    None,
    /// A stop or comma hangs only if it would otherwise have to move to the
    /// next line (JLReq 2.5.1 `[W4]`; CSS `allow-end`).
    Allow,
    /// A stop or comma at the line end always hangs outside the text block,
    /// even if the line could have fitted it (CSS `force-end`).
    Force,
}

/// Hanging punctuation configuration (spec §5 Stage 1.3; JLReq 2.5.1, CSS Text
/// 4 `hanging-punctuation` `[W4][W2]`). At most one character hangs at each
/// edge.
///
/// The two edges are independent, hence a struct rather than one enum. They are
/// **logical** (start and end of the line in the paragraph's base direction),
/// whereas `PositionedLine::hanging_left` and `hanging_right` are **physical**:
/// the start edge is the left one for an LTR paragraph and the right one for an
/// RTL paragraph, the end edge the opposite. [`HangMode::allows`] is that
/// mapping, and `PositionedLine::validate` enforces it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HangMode {
    /// An opening mark may hang at the **start** of a line, outside the text
    /// block (CSS `first`).
    pub start: bool,
    /// Hanging at the **end** of a line.
    pub end: HangEndMode,
}

impl HangMode {
    /// Nothing hangs anywhere (the default).
    pub const NONE: Self = Self {
        start: false,
        end: HangEndMode::None,
    };

    /// Whether punctuation may hang at the physical `side` of a line in a
    /// paragraph of the given base `direction`.
    #[must_use]
    pub const fn allows(self, side: HangSide, direction: Direction) -> bool {
        let at_start = matches!(
            (side, direction),
            (HangSide::Left, Direction::Ltr) | (HangSide::Right, Direction::Rtl)
        );
        if at_start {
            self.start
        } else {
            !matches!(self.end, HangEndMode::None)
        }
    }
}

/// Line height policy (spec §5 Stage 4.1; CSS Inline Layout 3 `[B7]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LineHeight {
    /// `line-height: normal`: the union of the glyph bounds of every font used,
    /// plus the font line gap.
    #[default]
    Normal,
    /// A multiple of the font size taken from the first available font only;
    /// fallback glyphs may overflow the line box (standard CSS behaviour).
    Fixed(f32),
}

impl LineHeight {
    /// Resolves the paragraph strut for `font` at `font_size` (spec §5 Stage 4.1).
    ///
    /// * `Normal`: the font's own ascent, descent and line gap.
    /// * `Fixed(m)`: total height `m * font_size`; the leading
    ///   `total - (ascent + descent)` (possibly negative) is split in half above
    ///   and below, and the line gap becomes 0 (CSS half-leading).
    ///
    /// `natural_line_height()` of the result is the line height of an empty line.
    #[must_use]
    pub fn resolve_strut(self, font: &FontMetrics, font_size: f32) -> VerticalMetrics {
        match self {
            Self::Normal => VerticalMetrics::from_font_metrics(font),
            Self::Fixed(multiplier) => {
                let leading = multiplier * font_size - (font.ascent + font.descent);
                VerticalMetrics {
                    ascent: font.ascent + leading / 2.0,
                    descent: font.descent + leading / 2.0,
                    line_gap: 0.0,
                }
            }
        }
    }
}

/// Trailing whitespace handling at a soft wrap (spec §5 Stage 1.7).
/// **[Design decision]** in the spec; CSS Text 3 `white-space` is the model `[W1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangingWhitespaceMode {
    /// CSS `normal`-like: trailing `WordSpace` and `CjkAutospace` collapse to
    /// zero visual width. They stay in `PositionedLine::byte_range`;
    /// `visible_byte_end` marks the last visible cluster.
    #[default]
    Collapsible,
    /// CSS `pre-wrap`-like: trailing spaces hang outside the line width but keep
    /// selectable, caret-reachable slots. An editor normally uses this on the
    /// line containing the caret.
    Preserved,
}

/// Line-breaking strategy (spec §5 Stage 1.6). The pair is the standard
/// dichotomy (Typst `Simple`/`Optimized`, Android `SIMPLE`/`HIGH_QUALITY`) `[L3][L6]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineBreakStrategy {
    /// Strategy A: first-fit greedy with the overflow ladder. Each line depends
    /// only on its start, which allows resynchronising incremental relayout.
    #[default]
    Greedy,
    /// Strategy B: global optimisation over the whole paragraph or a bounded
    /// window (Knuth-Plass demerits, §5 Stage 1.6).
    GlobalOptimal,
}

/// Clipboard policy for an actual U+00AD in the source text (spec §6.6, the
/// `copy_soft_hyphen` flag). The *generated* hyphen is never copied regardless.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SoftHyphenPolicy {
    /// Copy U+00AD as it is in the source (Firefox's behaviour `[C4]`).
    #[default]
    Keep,
    /// Drop U+00AD from copied text.
    Strip,
}

/// Everything the middle-end needs besides the AST (spec §5, §6.6): the single
/// input carrier for layout policy.
///
/// **Not yet included** (Open question 29): the wrap width (`wrap_width` in the
/// spec's prose), tab stops, and a base-direction override. Without the width
/// a layout entry point cannot be called, so none is defined in Phase 0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutOptions {
    /// Line-breaking strategy.
    pub strategy: LineBreakStrategy,
    /// Hanging punctuation at both edges.
    pub hang_mode: HangMode,
    /// Line-height policy.
    pub line_height: LineHeight,
    /// Trailing-whitespace handling at a soft wrap.
    pub whitespace: HangingWhitespaceMode,
    /// Clipboard policy for an actual U+00AD in the source.
    pub copy_soft_hyphen: SoftHyphenPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strut_resolution_folds_half_leading() {
        let font = FontMetrics {
            ascent: 8.0,
            descent: 2.0,
            line_gap: 1.0,
            ..Default::default()
        };
        let normal = LineHeight::Normal.resolve_strut(&font, 10.0);
        assert_eq!(normal.natural_line_height(), 11.0);
        let fixed = LineHeight::Fixed(1.5).resolve_strut(&font, 10.0);
        assert_eq!(
            (fixed.ascent, fixed.descent, fixed.line_gap),
            (10.5, 4.5, 0.0)
        );
        assert_eq!(fixed.natural_line_height(), 15.0);
        // A line height below the glyph extent gives negative leading.
        let tight = LineHeight::Fixed(0.5).resolve_strut(&font, 10.0);
        assert_eq!(tight.natural_line_height(), 5.0);
    }

    #[test]
    fn defaults_follow_the_spec_baselines() {
        assert_eq!(HangMode::default(), HangMode::NONE);
        assert_eq!(LineHeight::default(), LineHeight::Normal);
        let o = LayoutOptions::default();
        assert_eq!(o.strategy, LineBreakStrategy::Greedy);
        assert_eq!(o.hang_mode, HangMode::NONE);
        assert_eq!(o.whitespace, HangingWhitespaceMode::Collapsible);
        assert_eq!(o.copy_soft_hyphen, SoftHyphenPolicy::Keep);
    }

    #[test]
    fn hang_mode_combines_both_edges_independently() {
        use Direction::{Ltr, Rtl};
        const L: HangSide = HangSide::Left;
        const R: HangSide = HangSide::Right;
        let modes = [
            (HangMode::NONE, [false; 4]),
            // start only: left edge in LTR, right edge in RTL
            (
                HangMode {
                    start: true,
                    end: HangEndMode::None,
                },
                [true, false, false, true],
            ),
            // end only (Allow and Force alike): the opposite edges
            (
                HangMode {
                    start: false,
                    end: HangEndMode::Allow,
                },
                [false, true, true, false],
            ),
            (
                HangMode {
                    start: false,
                    end: HangEndMode::Force,
                },
                [false, true, true, false],
            ),
            // both edges at once, which the old single enum could not express
            (
                HangMode {
                    start: true,
                    end: HangEndMode::Force,
                },
                [true; 4],
            ),
        ];
        for (mode, [ltr_left, ltr_right, rtl_left, rtl_right]) in modes {
            assert_eq!(mode.allows(L, Ltr), ltr_left, "{mode:?} LTR left");
            assert_eq!(mode.allows(R, Ltr), ltr_right, "{mode:?} LTR right");
            assert_eq!(mode.allows(L, Rtl), rtl_left, "{mode:?} RTL left");
            assert_eq!(mode.allows(R, Rtl), rtl_right, "{mode:?} RTL right");
        }
        assert_ne!(HangEndMode::Allow, HangEndMode::Force);
        assert_eq!(HangEndMode::default(), HangEndMode::None);
    }
}
