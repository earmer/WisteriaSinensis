//! Layout policy: the strategy enums and [`LayoutOptions`] collecting them
//! (spec §5, §6.6).

use super::{HangMode, LineHeight};
use crate::frontend::is_line_or_paragraph_separator;

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

/// Layout **policy** (spec §5, §6.6): a pure collection of strategies. The
/// geometry lives in [`LayoutConstraints`](super::LayoutConstraints).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutOptions {
    /// Whether available elastic capacity is used to justify content.
    pub justify: bool,
    /// Under `justify`, whether a line that ends the paragraph or at a hard
    /// break may be fitted with shrink and compressed flush (spec ST1-7,
    /// ST3-3). Such a line is never stretched; when this is off (the
    /// default), a line that only shrink would fit ends at an earlier break.
    pub compress_last_lines: bool,
    /// Optional line-count or block-size truncation policy; when set it must
    /// satisfy [`LineClampPolicy::is_valid`] (`InvalidLayoutOptions` otherwise).
    pub clamp: Option<LineClampPolicy>,
    /// Line-breaking strategy.
    pub strategy: LineBreakStrategy,
    /// Hanging punctuation at both edges.
    pub hang_mode: HangMode,
    /// Line-height policy; a `Fixed` multiplier must be finite and positive
    /// ([`LineHeight::is_valid`], `InvalidStrut` otherwise).
    pub line_height: LineHeight,
    /// Trailing-whitespace handling at a soft wrap.
    pub whitespace: HangingWhitespaceMode,
    /// Clipboard policy for an actual U+00AD in the source.
    pub soft_hyphen: SoftHyphenPolicy,
}

/// Truncation limits; at least one limit must be configured when enabled.
#[derive(Debug, Clone, PartialEq)]
pub struct LineClampPolicy {
    /// Maximum number of output lines: at least 1.
    pub max_lines: Option<usize>,
    /// Maximum block extent: finite and positive, except that one line is
    /// mandatory.
    pub max_block_size: Option<f32>,
    /// Synthesized suffix; empty means silent truncation.
    pub ellipsis: Box<str>,
}

impl LineClampPolicy {
    /// Truncates after `n` lines, with the ellipsis `"…"`.
    #[must_use]
    pub fn max_lines(n: usize) -> Self {
        Self::with_ellipsis(Some(n), None)
    }

    /// Truncates once the block extent would exceed `h`, with the ellipsis `"…"`.
    #[must_use]
    pub fn max_block_size(h: f32) -> Self {
        Self::with_ellipsis(None, Some(h))
    }

    /// The given limits with the default ellipsis `"…"`.
    fn with_ellipsis(max_lines: Option<usize>, max_block_size: Option<f32>) -> Self {
        Self {
            max_lines,
            max_block_size,
            ellipsis: "…".into(),
        }
    }

    /// Checks configured limits and the suffix source.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        (self.max_lines.is_some() || self.max_block_size.is_some())
            && self.max_lines != Some(0)
            && self.max_block_size.is_none_or(|h| h.is_finite() && h > 0.0)
            && !self.ellipsis.chars().any(is_line_or_paragraph_separator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_line_limit_takes_the_field_type() {
        let policy = LineClampPolicy::max_lines(usize::MAX);
        assert_eq!(policy.max_lines, Some(usize::MAX));
        assert!(policy.is_valid());
        assert!(!LineClampPolicy::max_lines(0).is_valid());
    }
}
