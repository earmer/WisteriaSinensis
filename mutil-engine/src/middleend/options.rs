//! Layout policy and container geometry (spec §5 Stage 1.3, Stage 4, §6.6).

use crate::frontend::is_line_or_paragraph_separator;
use crate::middleend::tolerance::tolerance;
use crate::middleend::{HangSide, SlotSource};
use crate::types::{Direction, FontMetrics, WritingMode, is_measurement};
use smallvec::SmallVec;
use std::sync::Arc;

/// Trailing whitespace at a soft wrap (spec §5 Stage 1.7; CSS `white-space` `[W1]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangingWhitespaceMode {
    /// CSS `normal`: trailing `WordSpace`/`CjkAutospace` collapse to zero width but stay
    /// in `PositionedLine::byte_range` (`visible_byte_end` marks the last visible cluster).
    #[default]
    Collapsible,
    /// CSS `pre-wrap`: trailing spaces hang outside the line yet keep caret slots.
    Preserved,
}

/// Line-breaking strategy (spec §5 Stage 1.6.1): one beam-lookahead machine `[L3][L6]`.
///
/// Phase 1 and 2 layout breaks every strategy greedily (spec ME-2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineBreakStrategy {
    /// First-fit with the overflow ladder; each line depends only on its start.
    Greedy,
    /// The engine default; stack-only state.
    #[default]
    Balanced,
    /// Knuth-Plass quality over a bounded window, with dominator pruning.
    Publication,
}

/// Copy policy for a source U+00AD (spec §6.6); a generated hyphen is never copied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SoftHyphenPolicy {
    /// Copy U+00AD as in the source (Firefox `[C4]`).
    #[default]
    Keep,
    /// Drop U+00AD from copied text.
    Strip,
}

/// Layout policy (spec §5, §6.6); the geometry is [`LayoutConstraints`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutOptions {
    /// Whether elastic capacity is used to justify content.
    pub justify: bool,
    /// Under `justify`, whether a paragraph-final or hard-broken line may shrink to fit
    /// (spec ST1-7, ST3-3); it never stretches. Off: it breaks earlier instead.
    pub compress_last_lines: bool,
    /// Optional truncation; must satisfy [`LineClampPolicy::is_valid`].
    pub clamp: Option<LineClampPolicy>,
    /// Line-breaking strategy.
    pub strategy: LineBreakStrategy,
    /// Hanging punctuation at both edges.
    pub hang_mode: HangMode,
    /// Line-height policy; must satisfy [`LineHeight::is_valid`].
    pub line_height: LineHeight,
    /// Trailing-whitespace handling at a soft wrap.
    pub whitespace: HangingWhitespaceMode,
    /// Clipboard policy for a source U+00AD.
    pub soft_hyphen: SoftHyphenPolicy,
}

/// Truncation limits; at least one must be set.
#[derive(Debug, Clone, PartialEq)]
pub struct LineClampPolicy {
    /// Maximum number of lines: at least 1.
    pub max_lines: Option<usize>,
    /// Maximum block extent: finite and positive; one line is always kept.
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

    fn with_ellipsis(max_lines: Option<usize>, max_block_size: Option<f32>) -> Self {
        Self {
            max_lines,
            max_block_size,
            ellipsis: "…".into(),
        }
    }

    /// Checks the limits and that the ellipsis has no line separator.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        (self.max_lines.is_some() || self.max_block_size.is_some())
            && self.max_lines != Some(0)
            && self.max_block_size.is_none_or(|h| h.is_finite() && h > 0.0)
            && !self.ellipsis.chars().any(is_line_or_paragraph_separator)
    }
}

/// Hanging at the end edge of a line (CSS Text 4 `hanging-punctuation` `[W2]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangEndMode {
    /// Nothing hangs.
    #[default]
    None,
    /// A stop or comma hangs only instead of wrapping (JLReq `[W4]`; CSS `allow-end`).
    Allow,
    /// A stop or comma at the line end always hangs (CSS `force-end`).
    Force,
}

/// Hanging at the start edge of a line (CSS Text 4 `hanging-punctuation` `[W2]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangStartMode {
    /// Nothing hangs.
    #[default]
    None,
    /// An opening mark may hang on line index 0 only (CSS `first`), not after a hard break.
    FirstLine,
    /// An opening mark may hang on every line.
    AllLines,
}

/// Hanging punctuation (spec §5 Stage 1.3; JLReq 2.5.1, CSS Text 4 `[W4][W2]`): at most
/// one character per logical edge. [`HangMode::allows`] maps the edges to the physical
/// `PositionedLine::hanging_left()`/`hanging_right()` (`PositionedLine::validate`).
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
    /// `direction` paragraph; `is_first_line` is whether it is line index 0.
    #[must_use]
    pub const fn allows(self, side: HangSide, direction: Direction, is_first_line: bool) -> bool {
        match self.start {
            _ if !side.is_line_start(direction) => !matches!(self.end, HangEndMode::None),
            HangStartMode::None => false,
            HangStartMode::FirstLine => is_first_line,
            HangStartMode::AllLines => true,
        }
    }
}

/// Line height policy (spec §5 Stage 4.1; CSS Inline Layout 3 `[B7]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LineHeight {
    /// CSS `normal`: the union of every used font's glyph bounds, plus the line gap.
    #[default]
    Normal,
    /// A finite, positive multiple of the font size (first available font; fallbacks may
    /// overflow the line box).
    Fixed(f32),
}

impl LineHeight {
    /// `true` for `Normal` and for a finite, positive `Fixed` multiplier.
    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::Normal => true,
            Self::Fixed(multiplier) => multiplier.is_finite() && multiplier > 0.0,
        }
    }

    /// Resolves the paragraph strut from `font` at `size`, from the vertical metrics in a
    /// vertical `mode` (spec §5 Stage 4.1). `Fixed(m)` splits the possibly negative leading
    /// `m * size - (ascent + descent)` evenly above and below and zeroes the line gap (CSS
    /// half-leading).
    #[must_use]
    pub fn resolve_strut(self, font: &FontMetrics, size: f32, mode: WritingMode) -> StrutMetrics {
        let [ascent, descent, line_gap] = if mode.is_vertical() {
            [
                font.vertical_ascent,
                font.vertical_descent,
                font.vertical_line_gap,
            ]
        } else {
            [font.ascent, font.descent, font.line_gap]
        };
        let Self::Fixed(multiplier) = self else {
            return StrutMetrics {
                ascent,
                descent,
                line_gap,
            };
        };
        let half_leading = (multiplier * size - (ascent + descent)) / 2.0;
        StrutMetrics {
            ascent: ascent + half_leading,
            descent: descent + half_leading,
            line_gap: 0.0,
        }
    }
}

/// The resolved paragraph strut (spec §5 Stage 4.1): the minimum (`Normal`) or exact (`Fixed`)
/// line box. With half-leading folded in, `ascent`/`descent` may be negative (raw
/// [`FontMetrics`] never are).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrutMetrics {
    /// Extent toward line-over, half-leading included.
    pub ascent: f32,
    /// Extent toward line-under, half-leading included.
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

    /// Every field and the line height are measurements (spec MSR-1); the height is positive.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let height = self.line_height();
        [self.ascent, self.descent, self.line_gap, height]
            .into_iter()
            .all(is_measurement)
            && height > 0.0
    }
}

/// Tab stops at every multiple of `step` from the line-start edge of the slot
/// (spec OPT-1, ST3-1). Parsed, not validated (spec §1.5, AX-3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStops {
    step: f32,
}

impl TabStops {
    /// Stops every 8 layout units, the default of [`LayoutConstraints`].
    pub const DEFAULT: Self = Self { step: 8.0 };

    /// Stops every `step` units, or `None` unless `step` is finite and positive.
    #[must_use]
    pub fn try_new(step: f32) -> Option<Self> {
        (step.is_finite() && step > 0.0).then_some(Self { step })
    }

    /// The distance between stops: finite and positive.
    #[must_use]
    pub const fn step(self) -> f32 {
        self.step
    }

    /// The first stop strictly beyond `x`, or `None` if no finite one is representable.
    #[must_use]
    pub fn next_stop(&self, x: f32) -> Option<f32> {
        // Where the product rounds back to `x` or overflows, try `x + step`.
        let next = ((x / self.step).floor() + 1.0) * self.step;
        [next, x + self.step]
            .into_iter()
            .find(|&stop| stop.is_finite() && stop > x)
    }
}

/// Container geometry: what layout must fit into (the policy is [`LayoutOptions`]).
#[derive(Clone, Debug)]
pub struct LayoutConstraints {
    /// Inline start of the first line's standard slot: in `[0, wrap_width)`,
    /// or merely finite with a geometry provider (which ignores it).
    pub first_line_indent: f32,
    /// Same, for the subsequent lines.
    pub hanging_indent: f32,
    /// Optional obstacle-aware container geometry.
    pub geometry_provider: Option<Arc<dyn LineGeometryProvider>>,
    /// Positive inline width (infinity disables wrapping); slots lie in `[0, wrap_width]`.
    pub wrap_width: f32,
    /// Tab stops.
    pub tab_stops: TabStops,
}

impl Default for LayoutConstraints {
    fn default() -> Self {
        Self {
            first_line_indent: 0.0,
            hanging_indent: 0.0,
            geometry_provider: None,
            wrap_width: f32::INFINITY,
            tab_stops: TabStops::DEFAULT,
        }
    }
}

impl LayoutConstraints {
    /// Positive `wrap_width`, finite indents, each in `[0, wrap_width)` without a provider.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let in_envelope = |indent: f32| indent >= 0.0 && indent < self.wrap_width;
        self.wrap_width > 0.0
            && self.first_line_indent.is_finite()
            && self.hanging_indent.is_finite()
            && (self.geometry_provider.is_some()
                || (in_envelope(self.first_line_indent) && in_envelope(self.hanging_indent)))
    }

    /// Line `line_index`'s unvalidated geometry over `[block_start, block_end)`: the
    /// provider's answer, or the standard slot `[indent, wrap_width]` (none if malformed).
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
        LineGeometry {
            slots: SlotSource::of(self)
                .standard_slot(line_index, self.wrap_width)
                .into_iter()
                .collect(),
            next_feature_block_offset: None,
        }
    }
}

/// An available inline interval `[start, end)`; parsed, not validated (spec §1.5, AX-3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineSlotConstraint {
    start: f32,
    end: f32,
}

impl InlineSlotConstraint {
    /// `[start, end)`, or `None` unless `start` is finite and `end` lies beyond
    /// it, at a finite distance or at positive infinity.
    #[must_use]
    pub fn try_new(start: f32, end: f32) -> Option<Self> {
        (start.is_finite() && end > start && (end == f32::INFINITY || (end - start).is_finite()))
            .then_some(Self { start, end })
    }

    /// Finite inline start.
    #[must_use]
    pub const fn start(self) -> f32 {
        self.start
    }

    /// End beyond the start; infinite only under an infinite wrap width.
    #[must_use]
    pub const fn end(self) -> f32 {
        self.end
    }

    /// Available advance.
    #[must_use]
    pub fn width(self) -> f32 {
        self.end - self.start
    }

    /// Whether the slot lies in the envelope `[0, wrap_width]` within the
    /// geometry tolerance (only `start >= 0` for an infinite `wrap_width`).
    #[must_use]
    pub fn lies_in_envelope(self, wrap_width: f32) -> bool {
        self.start >= -tolerance(self.start, 0.0)
            && (wrap_width == f32::INFINITY
                || (self.end.is_finite()
                    && self.end <= wrap_width + tolerance(self.end, wrap_width)))
    }
}

/// Whether `slots` lie in the envelope `[0, wrap_width]`, sorted and disjoint.
pub(super) fn slots_are_valid(slots: &[InlineSlotConstraint], wrap_width: f32) -> bool {
    slots.iter().all(|s| s.lies_in_envelope(wrap_width))
        && slots.windows(2).all(|w| w[0].end <= w[1].start)
}

/// The least block progress a fully obstructed answer must offer (spec GEO-4).
const MIN_CLEARANCE_STEP: f32 = 1e-4;

/// Available slots for one block-axis envelope.
#[derive(Debug, Clone, PartialEq)]
pub struct LineGeometry {
    /// Sorted, disjoint intervals; empty when fully obstructed.
    pub slots: SmallVec<[InlineSlotConstraint; 2]>,
    /// Next block start at which geometry may change.
    pub next_feature_block_offset: Option<f32>,
}

impl LineGeometry {
    /// Checks a provider answer at `block_start`: valid slots for `wrap_width`, and the
    /// next feature ahead by a measurement (spec MSR-1).
    #[must_use]
    pub fn is_valid(&self, block_start: f32, wrap_width: f32) -> bool {
        slots_are_valid(&self.slots, wrap_width)
            && self
                .next_feature_block_offset
                .is_none_or(|v| v > block_start && is_measurement(v - block_start))
    }

    /// Whether every slot lies in a slot of `previous`, i.e. growth added no inline area.
    #[must_use]
    pub fn is_subset_of(&self, previous: &Self) -> bool {
        // Both sorted: the only candidate container is the first one not ending earlier.
        let mut candidates = previous.slots.iter().peekable();
        self.slots.iter().all(|slot| {
            while candidates.next_if(|p| p.end < slot.end).is_some() {}
            candidates
                .peek()
                .is_some_and(|p| slot.start >= p.start && slot.end <= p.end)
        })
    }

    /// Whether a fully obstructed answer fails to advance at least
    /// `MIN_CLEARANCE_STEP` past `block_start` (spec GEO-4, G3).
    pub(super) fn stalls(&self, block_start: f32) -> bool {
        self.slots.is_empty()
            && self
                .next_feature_block_offset
                .is_none_or(|next| next - block_start < MIN_CLEARANCE_STEP)
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
    use smallvec::smallvec;

    #[test]
    fn a_line_limit_takes_the_field_type() {
        let policy = LineClampPolicy::max_lines(usize::MAX);
        assert_eq!(policy.max_lines, Some(usize::MAX));
        assert!(policy.is_valid());
        assert!(!LineClampPolicy::max_lines(0).is_valid());
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
            slots: smallvec![InlineSlotConstraint::try_new(start, end).unwrap()],
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
        // A malformed interval is not representable at all.
        assert_eq!(InlineSlotConstraint::try_new(5.0, 5.0), None);
        assert_eq!(InlineSlotConstraint::try_new(f32::NAN, 5.0), None);
        assert_eq!(InlineSlotConstraint::try_new(0.0, f32::NAN), None);
        assert_eq!(InlineSlotConstraint::try_new(f32::NEG_INFINITY, 5.0), None);
        assert_eq!(InlineSlotConstraint::try_new(-f32::MAX, f32::MAX), None);
    }

    #[test]
    fn tab_stops_step_and_validity() {
        let t = TabStops::DEFAULT;
        assert_eq!(TabStops::try_new(8.0), Some(t));
        assert_eq!(t.next_stop(0.0), Some(8.0));
        assert_eq!(t.next_stop(7.9), Some(8.0));
        assert_eq!(t.next_stop(8.0), Some(16.0)); // on a stop: advance to the next
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(TabStops::try_new(bad), None);
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
