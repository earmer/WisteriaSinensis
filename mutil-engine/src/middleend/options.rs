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

/// Hanging at the leading edge of a line (spec §5 Stage 1.3; CSS Text 4
/// `hanging-punctuation: first` `[W2]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangStartMode {
    /// Nothing hangs at the line start.
    #[default]
    None,
    /// An opening mark may hang at the start of the **first line of the
    /// paragraph** only (CSS `first`). "First line" is line index 0; a line
    /// after a hard break is not a first line.
    FirstLine,
    /// An opening mark may hang at the start of **every** line.
    AllLines,
}

/// Hanging punctuation configuration (spec §5 Stage 1.3; JLReq 2.5.1, CSS Text
/// 4 `hanging-punctuation` `[W4][W2]`). At most one character hangs at each
/// edge.
///
/// The two edges are independent. They are **logical** (start and end of the
/// line in the paragraph's base direction), whereas
/// `PositionedLine::hanging_left` and `hanging_right` are **physical**: the
/// start edge is the left one for an LTR paragraph and the right one for an RTL
/// paragraph, the end edge the opposite. [`HangMode::allows`] is that mapping
/// (plus the first-line rule), and `PositionedLine::validate` enforces it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HangMode {
    /// Hanging at the start of a line.
    pub start: HangStartMode,
    /// Hanging at the end of a line.
    pub end: HangEndMode,
}

impl HangMode {
    /// Nothing hangs anywhere (the default).
    pub const NONE: Self = Self {
        start: HangStartMode::None,
        end: HangEndMode::None,
    };

    /// Whether punctuation may hang at the physical `side` of a line in a
    /// paragraph of the given base `direction`; `is_first_line` is whether the
    /// line is the paragraph's first (line index 0).
    #[must_use]
    pub const fn allows(self, side: HangSide, direction: Direction, is_first_line: bool) -> bool {
        let at_start = matches!(
            (side, direction),
            (HangSide::Left, Direction::Ltr) | (HangSide::Right, Direction::Rtl)
        );
        if at_start {
            match self.start {
                HangStartMode::None => false,
                HangStartMode::FirstLine => is_first_line,
                HangStartMode::AllLines => true,
            }
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

/// Line-breaking strategy (spec §5 Stage 1.6.1): a **profile** of one
/// parameterised beam-lookahead state machine, not a choice between code paths.
///
/// | Profile | `beam_width` | `window_depth` | Use |
/// |---|---|---|---|
/// | `Greedy` | 1 | 1 | monospace code, logs, UAX #14 oracle tests |
/// | `Balanced` (default) | 2 | 2 | interactive rich-text editing |
/// | `Publication` | 8 | 6 | static reading view, print export |
///
/// The pair (Typst `Simple`/`Optimized`, Android `SIMPLE`/`HIGH_QUALITY`) is the
/// standard dichotomy `[L3][L6]`; here it is the two ends of a continuum.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineBreakStrategy {
    /// Beam 1, window 1: first-fit with the overflow ladder. Each line depends
    /// only on its start, which allows resynchronising incremental relayout.
    Greedy,
    /// Beam 2, window 2: the engine default. Stack-only state.
    #[default]
    Balanced,
    /// Beam 8, window 6, with dominator pruning: Knuth-Plass quality over a
    /// bounded window.
    Publication,
}

impl LineBreakStrategy {
    /// Number of candidate paths kept alive (`K`).
    #[must_use]
    pub const fn beam_width(self) -> usize {
        match self {
            Self::Greedy => 1,
            Self::Balanced => 2,
            Self::Publication => 8,
        }
    }

    /// Number of lines a candidate is looked ahead before the oldest is
    /// committed (`W`).
    #[must_use]
    pub const fn window_depth(self) -> usize {
        match self {
            Self::Greedy => 1,
            Self::Balanced => 2,
            Self::Publication => 6,
        }
    }
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

/// Tab-stop configuration: stops at every multiple of `step` from the line's
/// left edge. **[Design decision]**: the spec names `next_tab_stop(x)` without
/// defining the stops; a uniform step is the minimal configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStops {
    /// Distance between stops, in layout units. Must be finite and positive.
    pub step: f32,
}

impl TabStops {
    /// `true` when `step` is finite and positive.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.step.is_finite() && self.step > 0.0
    }

    /// The first stop strictly right of `x` (`x` on a stop moves to the next).
    /// Returns None when invalid or no finite forward stop is representable.
    #[must_use]
    pub fn next_stop(&self, x: f32) -> Option<f32> {
        if !self.is_valid() || !x.is_finite() {
            return None;
        }
        let next = ((x / self.step).floor() + 1.0) * self.step;
        if next.is_finite() && next > x {
            Some(next)
        } else {
            // Precision exhaustion (e.g. x=1e10, step=8 rounds back) or overflow.
            // Try one ulp step via x+step; if that also fails, signal exhaustion.
            let stepped = x + self.step;
            if stepped.is_finite() && stepped > x {
                Some(stepped)
            } else {
                None
            }
        }
    }
}

/// Geometric constraints of the container: what the layout must fit into, as
/// opposed to the policy ([`LayoutOptions`]) deciding how.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutConstraints {
    /// Available layout width of the container, in layout units. Must be finite
    /// and non-negative. Hanging punctuation and `Preserved` trailing
    /// whitespace may extend beyond it. `PositionedLine::validate` checks a
    /// line's `visual_width` against it within tolerance; a line may be wider
    /// only in the single-entity overflow state (a lone `AllowOverflow` box, or
    /// one indivisible cluster after the emergency break, §5 Stage 1.5). A
    /// line of several clusters wider than the width fails validation: an
    /// unbreakable word is split by the emergency break, not left to overflow.
    pub wrap_width: f32,
    /// Tab stops.
    pub tab_stops: TabStops,
}

impl LayoutConstraints {
    /// `true` when the width is finite and non-negative and the tab stops are
    /// valid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.wrap_width.is_finite() && self.wrap_width >= 0.0 && self.tab_stops.is_valid()
    }
}

/// Layout **policy** (spec §5, §6.6): a pure collection of strategies. The
/// geometry lives in [`LayoutConstraints`].
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
    fn strategy_profiles_map_to_beam_parameters() {
        use LineBreakStrategy as S;
        assert_eq!((S::Greedy.beam_width(), S::Greedy.window_depth()), (1, 1));
        assert_eq!(
            (S::Balanced.beam_width(), S::Balanced.window_depth()),
            (2, 2)
        );
        assert_eq!(
            (S::Publication.beam_width(), S::Publication.window_depth()),
            (8, 6)
        );
    }

    #[test]
    fn hang_mode_grades_start_and_end_independently() {
        use Direction::{Ltr, Rtl};
        const L: HangSide = HangSide::Left;
        const R: HangSide = HangSide::Right;
        let mode = |start, end| HangMode { start, end };
        use HangEndMode as E;
        use HangStartMode as S;
        // (mode, direction, first line?) -> (left allowed, right allowed)
        let cases = [
            (HangMode::NONE, Ltr, true, (false, false)),
            (mode(S::FirstLine, E::None), Ltr, true, (true, false)),
            (mode(S::FirstLine, E::None), Ltr, false, (false, false)),
            (mode(S::FirstLine, E::None), Rtl, true, (false, true)),
            (mode(S::FirstLine, E::None), Rtl, false, (false, false)),
            (mode(S::AllLines, E::None), Ltr, false, (true, false)),
            (mode(S::AllLines, E::None), Rtl, false, (false, true)),
            // The end edge ignores the line index.
            (mode(S::None, E::Allow), Ltr, false, (false, true)),
            (mode(S::None, E::Force), Rtl, false, (true, false)),
            // Both edges at once.
            (mode(S::FirstLine, E::Force), Ltr, true, (true, true)),
            (mode(S::FirstLine, E::Force), Ltr, false, (false, true)),
            (mode(S::AllLines, E::Allow), Rtl, false, (true, true)),
        ];
        for (m, d, first, (left, right)) in cases {
            assert_eq!(
                m.allows(L, d, first),
                left,
                "{m:?} {d:?} first={first} left"
            );
            assert_eq!(
                m.allows(R, d, first),
                right,
                "{m:?} {d:?} first={first} right"
            );
        }
        assert_ne!(HangEndMode::Allow, HangEndMode::Force);
        assert_eq!(HangEndMode::default(), HangEndMode::None);
        assert_eq!(HangStartMode::default(), HangStartMode::None);
    }

    #[test]
    fn tab_stops_step_and_validity() {
        let t = TabStops { step: 8.0 };
        assert_eq!(t.next_stop(0.0), Some(8.0));
        assert_eq!(t.next_stop(7.9), Some(8.0));
        assert_eq!(t.next_stop(8.0), Some(16.0)); // on a stop: advance to the next
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert!(!TabStops { step: bad }.is_valid());
        }
        // At large x the f32 product rounds back to x; a stop still lies ahead.
        let x = 3.0e7_f32;
        assert!(t.next_stop(x).is_none_or(|next| next > x));
        let c = LayoutConstraints {
            wrap_width: 100.0,
            tab_stops: t,
        };
        assert!(c.is_valid());
        for w in [-1.0, f32::NAN, f32::INFINITY] {
            assert!(!LayoutConstraints { wrap_width: w, ..c }.is_valid());
        }
    }
}
