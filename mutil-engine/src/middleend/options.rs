//! Layout options (spec §5 Stage 1.3 and Stage 4).

use super::HangSide;
use crate::types::{CompactString, Direction, FontMetrics, WritingMode};
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
    pub fn resolve_strut(self, font: &FontMetrics, font_size: f32) -> FontMetrics {
        self.resolve_strut_for_mode(font, font_size, WritingMode::HorizontalTb)
    }
    /// Resolves the active strut using horizontal or vertical raw metrics.
    pub fn resolve_strut_for_mode(
        self,
        raw: &FontMetrics,
        font_size: f32,
        mode: WritingMode,
    ) -> FontMetrics {
        let mut font = *raw;
        if mode.is_vertical() {
            font.ascent = raw.vertical_ascent;
            font.descent = raw.vertical_descent;
            font.line_gap = raw.vertical_line_gap;
        }
        if let Self::Fixed(multiplier) = self {
            let leading = multiplier * font_size - (font.ascent + font.descent);
            font.ascent += leading / 2.0;
            font.descent += leading / 2.0;
            font.line_gap = 0.0;
        }
        font
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
#[derive(Clone, Debug)]
pub struct LayoutConstraints {
    /// Finite inline offset for the first line.
    pub first_line_indent: f32,
    /// Finite inline offset for subsequent lines.
    pub hanging_indent: f32,
    /// Optional obstacle-aware container geometry.
    pub geometry_provider: Option<Arc<dyn LineGeometryProvider>>,
    /// Positive container inline width; positive infinity disables wrapping.
    /// Indentation changes slot capacity, not this unobstructed reference width.
    pub wrap_width: f32,
    /// Tab stops.
    pub tab_stops: TabStops,
}

impl LayoutConstraints {
    /// Checks width, finite indents, positive standard-slot capacity and tab stops.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.wrap_width > 0.0
            && !self.wrap_width.is_nan()
            && self.first_line_indent.is_finite()
            && self.hanging_indent.is_finite()
            && (self.geometry_provider.is_some()
                || (self.first_line_indent < self.wrap_width
                    && self.hanging_indent < self.wrap_width))
            && self.tab_stops.is_valid()
    }
}

/// Layout **policy** (spec §5, §6.6): a pure collection of strategies. The
/// geometry lives in [`LayoutConstraints`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutOptions {
    /// Whether available elastic capacity is used to justify content.
    pub justify: bool,
    /// Optional line-count or block-size truncation policy.
    pub clamp: Option<LineClampPolicy>,
    /// Line-breaking strategy.
    pub strategy: LineBreakStrategy,
    /// Hanging punctuation at both edges.
    pub hang_mode: HangMode,
    /// Line-height policy.
    pub line_height: LineHeight,
    /// Trailing-whitespace handling at a soft wrap.
    pub whitespace: HangingWhitespaceMode,
    /// Clipboard policy for an actual U+00AD in the source.
    pub soft_hyphen: SoftHyphenPolicy,
}

/// One available inline interval; only its end may be positive infinity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineSlotConstraint {
    /// Finite inline start.
    pub start: f32,
    /// End strictly beyond start, possibly positive infinity.
    pub end: f32,
}
impl InlineSlotConstraint {
    /// Available advance.
    pub fn width(self) -> f32 {
        self.end - self.start
    }
    /// Whether endpoints and their difference obey the geometry contract.
    pub fn is_valid(self) -> bool {
        self.start.is_finite()
            && self.end > self.start
            && (self.end == f32::INFINITY || (self.end.is_finite() && self.width().is_finite()))
    }
}
/// Available slots for one block-axis envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct LineGeometry {
    /// Sorted, nonoverlapping available intervals.
    pub slots: SmallVec<[InlineSlotConstraint; 2]>,
    /// Next block start at which geometry may change.
    pub next_feature_block_offset: Option<f32>,
    /// Original unobstructed container width, before indentation.
    pub max_unobstructed_width: f32,
}
impl LineGeometry {
    /// Checks a provider response at the given query start.
    pub fn is_valid(&self, block_start: f32) -> bool {
        self.max_unobstructed_width > 0.0
            && !self.max_unobstructed_width.is_nan()
            && self.slots.iter().all(|s| s.is_valid())
            && self
                .slots
                .windows(2)
                .all(|w| w[0].start < w[1].start && w[0].end <= w[1].start)
            && self
                .next_feature_block_offset
                .is_none_or(|v| v.is_finite() && v > block_start)
    }
    /// Checks that an expanded block envelope introduced no new inline area.
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
impl Default for LayoutConstraints {
    fn default() -> Self {
        Self {
            wrap_width: f32::INFINITY,
            first_line_indent: 0.0,
            hanging_indent: 0.0,
            tab_stops: TabStops { step: 8.0 },
            geometry_provider: None,
        }
    }
}
impl LayoutConstraints {
    /// Queries custom geometry or constructs the standard indented column.
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
            slots: if start < self.wrap_width {
                smallvec![InlineSlotConstraint {
                    start,
                    end: self.wrap_width
                }]
            } else {
                SmallVec::new()
            },
            next_feature_block_offset: None,
            max_unobstructed_width: self.wrap_width,
        }
    }
}
/// Truncation limits; at least one limit must be configured when enabled.
#[derive(Debug, Clone, PartialEq)]
pub struct LineClampPolicy {
    /// Positive maximum number of output lines.
    pub max_lines: Option<usize>,
    /// Positive finite maximum block extent, except that one line is mandatory.
    pub max_block_size: Option<f32>,
    /// Synthesized suffix; empty means silent truncation.
    pub ellipsis: CompactString,
}
impl Default for LineClampPolicy {
    fn default() -> Self {
        Self {
            max_lines: None,
            max_block_size: None,
            ellipsis: "…".into(),
        }
    }
}
impl LineClampPolicy {
    /// Checks configured limits and the suffix source.
    pub fn is_valid(&self) -> bool {
        (self.max_lines.is_some() || self.max_block_size.is_some())
            && self.max_lines != Some(0)
            && self.max_block_size.is_none_or(|h| h.is_finite() && h > 0.0)
            && !self
                .ellipsis
                .chars()
                .any(|c| crate::frontend::is_paragraph_separator(c) || c == '\u{2028}')
    }
}

impl LayoutConstraints {
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
    ) -> Result<LineGeometry, super::LayoutInputError> {
        if !block_start.is_finite() || !block_end.is_finite() || block_end <= block_start {
            return Err(super::LayoutInputError::InvalidLineGeometry);
        }
        let geometry = self.resolve_line_geometry(line_index, block_start, block_end);
        if !geometry.is_valid(block_start) {
            return Err(super::LayoutInputError::InvalidLineGeometry);
        }
        if previous.is_some_and(|p| {
            !geometry.is_subset_of(p) || geometry.max_unobstructed_width != p.max_unobstructed_width
        }) {
            return Err(super::LayoutInputError::NonMonotonicGeometry);
        }
        Ok(geometry)
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
