//! Layout options (spec §5 Stage 1.3 and Stage 4).

use super::tolerance::tolerance;
use super::{HangSide, LayoutInputError};
use crate::frontend::is_line_or_paragraph_separator;
use crate::types::{Direction, FontMetrics, WritingMode};
use smallvec::{SmallVec, smallvec};
use std::sync::Arc;

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
        if side.is_line_start(direction) {
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
        let (ascent, descent, line_gap) = if mode.is_vertical() {
            (
                font.vertical_ascent,
                font.vertical_descent,
                font.vertical_line_gap,
            )
        } else {
            (font.ascent, font.descent, font.line_gap)
        };
        match self {
            Self::Normal => StrutMetrics {
                ascent,
                descent,
                line_gap,
            },
            Self::Fixed(multiplier) => {
                let half_leading = (multiplier * font_size - (ascent + descent)) / 2.0;
                StrutMetrics {
                    ascent: ascent + half_leading,
                    descent: descent + half_leading,
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

    /// `true` when every field is finite and the line height is finite and
    /// positive.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let height = self.line_height();
        [self.ascent, self.descent, self.line_gap]
            .iter()
            .all(|v| v.is_finite())
            && height.is_finite()
            && height > 0.0
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
    /// Distance between stops, in layout units. Must be finite and positive
    /// ([`TabStops::is_valid`]; `InvalidConstraints` otherwise).
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
#[derive(Clone, Debug)]
pub struct LayoutConstraints {
    /// Inline offset of the first line's standard slot. Without a geometry
    /// provider it must lie in `[0, wrap_width)`; a provider ignores it, and
    /// then it need only be finite.
    pub first_line_indent: f32,
    /// Inline offset of the standard slot of the subsequent lines. Same range
    /// as `first_line_indent`.
    pub hanging_indent: f32,
    /// Optional obstacle-aware container geometry.
    pub geometry_provider: Option<Arc<dyn LineGeometryProvider>>,
    /// Positive container inline width; positive infinity disables wrapping.
    /// Indentation changes slot capacity, not this unobstructed reference width.
    /// When finite, the inline envelope of every line is `[0, wrap_width]` and
    /// every slot, standard or provider-supplied, lies inside it.
    pub wrap_width: f32,
    /// Tab stops: `step` finite and positive.
    pub tab_stops: TabStops,
}

impl Default for LayoutConstraints {
    fn default() -> Self {
        Self {
            first_line_indent: 0.0,
            hanging_indent: 0.0,
            geometry_provider: None,
            wrap_width: f32::INFINITY,
            tab_stops: TabStops { step: 8.0 },
        }
    }
}

impl LayoutConstraints {
    /// Checks the width, the indents and the tab stops.
    ///
    /// * `wrap_width` is positive and not NaN (positive infinity is allowed);
    /// * both indents are finite;
    /// * without a geometry provider the standard slot starts at the indent and
    ///   must lie in the wrap envelope `[0, wrap_width]`, so each indent is in
    ///   `[0, wrap_width)`; negative indentation is not representable;
    /// * the tab stops are valid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let indent_is_in_envelope = |indent: f32| indent >= 0.0 && indent < self.wrap_width;
        self.wrap_width > 0.0
            && self.first_line_indent.is_finite()
            && self.hanging_indent.is_finite()
            && (self.geometry_provider.is_some()
                || (indent_is_in_envelope(self.first_line_indent)
                    && indent_is_in_envelope(self.hanging_indent)))
            && self.tab_stops.is_valid()
    }

    /// The geometry of line `line_index` over the block envelope
    /// `[block_start, block_end)`: the provider's answer, or the standard
    /// column `[indent, wrap_width]`. The response is not validated; see
    /// [`query_checked_geometry`](Self::query_checked_geometry).
    #[must_use]
    pub fn resolve_line_geometry(
        &self,
        line_index: usize,
        block_start: f32,
        block_end: f32,
    ) -> LineGeometry {
        if let Some(provider) = &self.geometry_provider {
            return provider.query_line_geometry(line_index, block_start, block_end);
        }
        let start = if line_index == 0 {
            self.first_line_indent
        } else {
            self.hanging_indent
        };
        LineGeometry {
            slots: smallvec![InlineSlotConstraint {
                start,
                end: self.wrap_width,
            }],
            next_feature_block_offset: None,
        }
    }

    /// Queries and validates geometry, optionally checking an earlier smaller envelope.
    ///
    /// # Errors
    /// Invalid query/response coordinates or a violation of provider monotonicity.
    pub fn query_checked_geometry(
        &self,
        line_index: usize,
        block_start: f32,
        block_end: f32,
        previous: Option<&LineGeometry>,
    ) -> Result<LineGeometry, LayoutInputError> {
        if !block_start.is_finite() || !block_end.is_finite() || block_end <= block_start {
            return Err(LayoutInputError::InvalidLineGeometry);
        }
        let geometry = self.resolve_line_geometry(line_index, block_start, block_end);
        if !geometry.is_valid(block_start, self.wrap_width) {
            return Err(LayoutInputError::InvalidLineGeometry);
        }
        if previous.is_some_and(|p| !geometry.is_subset_of(p)) {
            return Err(LayoutInputError::NonMonotonicGeometry);
        }
        Ok(geometry)
    }
}

/// Layout **policy** (spec §5, §6.6): a pure collection of strategies. The
/// geometry lives in [`LayoutConstraints`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutOptions {
    /// Whether available elastic capacity is used to justify content.
    pub justify: bool,
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
        Self {
            max_lines: Some(n),
            max_block_size: None,
            ellipsis: "…".into(),
        }
    }

    /// Truncates once the block extent would exceed `h`, with the ellipsis `"…"`.
    #[must_use]
    pub fn max_block_size(h: f32) -> Self {
        Self {
            max_lines: None,
            max_block_size: Some(h),
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

/// One available inline interval; only its end may be positive infinity.
///
/// Besides being well formed ([`is_valid`](Self::is_valid)), a slot of a line
/// lies in the line's inline envelope (see
/// [`lies_in_envelope`](Self::lies_in_envelope)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineSlotConstraint {
    /// Finite inline start; `>= 0` within the line's envelope.
    pub start: f32,
    /// End strictly beyond start, possibly positive infinity (only when the
    /// wrap width is infinite); `<= wrap_width` within the line's envelope.
    pub end: f32,
}

impl InlineSlotConstraint {
    /// Available advance.
    #[must_use]
    pub fn width(self) -> f32 {
        self.end - self.start
    }

    /// Whether endpoints and their difference obey the geometry contract.
    #[must_use]
    pub fn is_valid(self) -> bool {
        self.start.is_finite()
            && self.end > self.start
            && (self.end == f32::INFINITY || (self.end.is_finite() && self.width().is_finite()))
    }

    /// Whether the slot lies in the inline envelope of a line whose wrap width
    /// is `wrap_width`: `0 <= start` and, when `wrap_width` is finite,
    /// `end <= wrap_width`, both within the crate's geometry tolerance. With an
    /// infinite `wrap_width` only the lower bound applies. Assumes
    /// [`is_valid`](Self::is_valid).
    #[must_use]
    pub fn lies_in_envelope(self, wrap_width: f32) -> bool {
        self.start >= -tolerance(self.start, 0.0)
            && (wrap_width == f32::INFINITY
                || (self.end.is_finite()
                    && self.end <= wrap_width + tolerance(self.end, wrap_width)))
    }
}

/// `true` when `slots` are the slots of one line of a container `wrap_width`
/// wide: each well formed and inside the envelope `[0, wrap_width]`, in
/// ascending order and pairwise disjoint.
pub(super) fn slots_are_valid(slots: &[InlineSlotConstraint], wrap_width: f32) -> bool {
    slots
        .iter()
        .all(|s| s.is_valid() && s.lies_in_envelope(wrap_width))
        && slots.windows(2).all(|w| w[0].end <= w[1].start)
}

/// Available slots for one block-axis envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct LineGeometry {
    /// Sorted, nonoverlapping available intervals; empty when the envelope is
    /// fully obstructed.
    pub slots: SmallVec<[InlineSlotConstraint; 2]>,
    /// Next block start at which geometry may change.
    pub next_feature_block_offset: Option<f32>,
}

impl LineGeometry {
    /// Checks a provider response at the given query start, for a container of
    /// inline width `wrap_width`: every slot is well formed and lies in the
    /// envelope `[0, wrap_width]` ([`InlineSlotConstraint::lies_in_envelope`]),
    /// the slots are sorted and disjoint, and the next feature lies ahead.
    #[must_use]
    pub fn is_valid(&self, block_start: f32, wrap_width: f32) -> bool {
        slots_are_valid(&self.slots, wrap_width)
            && self
                .next_feature_block_offset
                .is_none_or(|v| v.is_finite() && v > block_start)
    }

    /// Checks that an expanded block envelope introduced no new inline area.
    #[must_use]
    pub fn is_subset_of(&self, previous: &Self) -> bool {
        let mut at = 0;
        for slot in &self.slots {
            while at < previous.slots.len() && previous.slots[at].end < slot.end {
                at += 1;
            }
            if !previous
                .slots
                .get(at)
                .is_some_and(|p| slot.start >= p.start && slot.end <= p.end)
            {
                return false;
            }
        }
        true
    }
}

/// Obstacle-aware geometry, queried in canonical flow coordinates.
pub trait LineGeometryProvider: std::fmt::Debug + Send + Sync {
    /// Increasing `block_end` for the same start/index may only remove space.
    fn query_line_geometry(
        &self,
        line_index: usize,
        block_start: f32,
        block_end: f32,
    ) -> LineGeometry;
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
    fn line_height_multipliers_must_be_finite_and_positive() {
        assert!(LineHeight::Normal.is_valid());
        assert!(LineHeight::Fixed(1.5).is_valid());
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!LineHeight::Fixed(bad).is_valid(), "{bad}");
        }
    }

    #[test]
    fn a_line_limit_takes_the_field_type() {
        let policy = LineClampPolicy::max_lines(usize::MAX);
        assert_eq!(policy.max_lines, Some(usize::MAX));
        assert!(policy.is_valid());
        assert!(!LineClampPolicy::max_lines(0).is_valid());
    }

    fn constraints(first: f32, hanging: f32, wrap_width: f32) -> LayoutConstraints {
        LayoutConstraints {
            first_line_indent: first,
            hanging_indent: hanging,
            wrap_width,
            ..Default::default()
        }
    }

    #[test]
    fn standard_slots_start_inside_the_wrap_envelope() {
        assert!(constraints(0.0, 0.0, 100.0).is_valid());
        assert!(constraints(99.0, 50.0, 100.0).is_valid());
        // Unbounded width: any non-negative finite indent.
        assert!(constraints(1e9, 0.0, f32::INFINITY).is_valid());
        for (first, hanging) in [
            (-1.0, 0.0),
            (0.0, -1.0),
            (-0.0 - f32::MIN_POSITIVE, 0.0),
            (100.0, 0.0),
            (0.0, 100.0),
            (f32::NAN, 0.0),
            (0.0, f32::INFINITY),
        ] {
            assert!(
                !constraints(first, hanging, 100.0).is_valid(),
                "{first} {hanging}"
            );
        }
        assert!(!constraints(-1.0, 0.0, f32::INFINITY).is_valid());
        // The indents are the standard slot's start: a provider replaces the
        // slots, so there they only have to be finite.
        let mut with_provider = constraints(-5.0, 500.0, 100.0);
        with_provider.geometry_provider = Some(Arc::new(NoSlots));
        assert!(with_provider.is_valid());
        with_provider.first_line_indent = f32::NAN;
        assert!(!with_provider.is_valid());
    }

    #[derive(Debug)]
    struct NoSlots;

    impl LineGeometryProvider for NoSlots {
        fn query_line_geometry(&self, _: usize, _: f32, _: f32) -> LineGeometry {
            LineGeometry {
                slots: SmallVec::new(),
                next_feature_block_offset: None,
            }
        }
    }

    #[test]
    fn provider_slots_lie_inside_the_wrap_envelope() {
        let geometry = |start: f32, end: f32| LineGeometry {
            slots: smallvec![InlineSlotConstraint { start, end }],
            next_feature_block_offset: None,
        };
        // A finite container is `[0, wrap_width]`.
        assert!(geometry(0.0, 100.0).is_valid(0.0, 100.0));
        assert!(geometry(10.0, 90.0).is_valid(0.0, 100.0));
        for (start, end) in [
            (-1.0, 50.0),
            (0.0, 101.0),
            (50.0, 150.0),
            (0.0, f32::INFINITY),
            (-5.0, 200.0),
        ] {
            assert!(!geometry(start, end).is_valid(0.0, 100.0), "{start}..{end}");
        }
        // Tolerance absorbs f32 noise, but only that.
        assert!(geometry(-1e-4, 100.0001).is_valid(0.0, 100.0));
        assert!(!geometry(-1e-2, 100.0).is_valid(0.0, 100.0));
        // An infinite container only needs `start >= 0`.
        assert!(geometry(0.0, f32::INFINITY).is_valid(0.0, f32::INFINITY));
        assert!(geometry(5.0, 1e9).is_valid(0.0, f32::INFINITY));
        assert!(!geometry(-1.0, 5.0).is_valid(0.0, f32::INFINITY));
        // `start < end` still holds first.
        assert!(!geometry(5.0, 5.0).is_valid(0.0, 100.0));
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
            first_line_indent: 0.0,
            geometry_provider: None,
            hanging_indent: 0.0,
            wrap_width: 100.0,
            tab_stops: t,
        };
        assert!(c.is_valid());
        for w in [0.0, -1.0, f32::NAN] {
            assert!(
                !LayoutConstraints {
                    wrap_width: w,
                    ..c.clone()
                }
                .is_valid()
            );
        }
    }
}
