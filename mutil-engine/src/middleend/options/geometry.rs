//! Container geometry (spec §5 Stage 1.3, GEO-3, GEO-4): the constraints the
//! layout must fit into, the per-line slots and the obstacle-aware provider.

use crate::middleend::LayoutInputError;
use crate::middleend::tolerance::tolerance;
use crate::types::is_measurement;
use smallvec::SmallVec;
use std::sync::Arc;

/// Tab-stop configuration: stops at every multiple of `step` along the line's
/// inline progression, measured from the line-start edge of its slot (spec
/// OPT-1, ST3-1).
/// A uniform step is the minimal configuration.
///
/// Parsed, not validated (spec §1.5, AX-3): [`new`](Self::new) refuses a step
/// that is not finite and positive, so a `TabStops` that exists is usable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabStops {
    step: f32,
}

impl TabStops {
    /// Stops every 8 layout units, the default of [`LayoutConstraints`].
    pub const DEFAULT: Self = Self { step: 8.0 };

    /// Stops every `step` layout units, or `None` unless `step` is finite and
    /// positive.
    #[must_use]
    pub fn new(step: f32) -> Option<Self> {
        (step.is_finite() && step > 0.0).then_some(Self { step })
    }

    /// The distance between stops: finite and positive.
    #[must_use]
    pub const fn step(self) -> f32 {
        self.step
    }

    /// The first stop strictly beyond the distance `x` (`x` on a stop moves
    /// to the next).
    /// Returns None when `x` is not finite or no finite forward stop is
    /// representable.
    #[must_use]
    pub fn next_stop(&self, x: f32) -> Option<f32> {
        if !x.is_finite() {
            return None;
        }
        let next = ((x / self.step).floor() + 1.0) * self.step;
        // On precision exhaustion (e.g. x=1e10, step=8 rounds back) or
        // overflow, fall back to x+step; if that also fails, signal exhaustion.
        [next, x + self.step]
            .into_iter()
            .find(|&stop| stop.is_finite() && stop > x)
    }
}

/// Geometric constraints of the container: what the layout must fit into, as
/// opposed to the policy ([`LayoutOptions`](super::LayoutOptions)) deciding how.
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
            tab_stops: TabStops::DEFAULT,
        }
    }
}

impl LayoutConstraints {
    /// Checks the width and the indents (tab stops are valid by construction).
    ///
    /// * `wrap_width` is positive and not NaN (positive infinity is allowed);
    /// * both indents are finite;
    /// * without a geometry provider the standard slot starts at the indent and
    ///   must lie in the wrap envelope `[0, wrap_width]`, so each indent is in
    ///   `[0, wrap_width)`; negative indentation is not representable.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let indent_is_in_envelope = |indent: f32| indent >= 0.0 && indent < self.wrap_width;
        self.wrap_width > 0.0
            && self.first_line_indent.is_finite()
            && self.hanging_indent.is_finite()
            && (self.geometry_provider.is_some()
                || (indent_is_in_envelope(self.first_line_indent)
                    && indent_is_in_envelope(self.hanging_indent)))
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
        // An indent that leaves no well-formed slot (which
        // `LayoutConstraints::is_valid` rejects) leaves no slot at all.
        LineGeometry {
            slots: InlineSlotConstraint::new(start, self.wrap_width)
                .into_iter()
                .collect(),
            next_feature_block_offset: None,
        }
    }

    /// Queries the geometry of a line. A provider's answer is untrusted and is
    /// validated, optionally against an earlier smaller envelope; the built-in
    /// geometry derives from constraints the gate accepted
    /// ([`is_valid`](Self::is_valid)) and is returned as is (spec GEO-3, AX-3).
    ///
    /// # Precondition
    /// `block_start` and `block_end` are finite and `block_end > block_start`.
    /// Layout computes the envelope from gated measurements (spec MSR-1), so it
    /// is not checked at run time (spec AX-3, GEO-3); a violation is a bug in
    /// the caller and is caught by a debug assertion, with or without a
    /// provider.
    ///
    /// # Errors
    /// `InvalidLineGeometry` for a provider answer outside the envelope, out
    /// of order or stalling, and `NonMonotonicGeometry` for one that breaks
    /// monotonicity against `previous`.
    pub fn query_checked_geometry(
        &self,
        line_index: usize,
        block_start: f32,
        block_end: f32,
        previous: Option<&LineGeometry>,
    ) -> Result<LineGeometry, LayoutInputError> {
        debug_assert!(
            block_start.is_finite() && block_end.is_finite() && block_end > block_start,
            "query_checked_geometry precondition violated (spec GEO-3)"
        );
        let geometry = self.resolve_line_geometry(line_index, block_start, block_end);
        if self.geometry_provider.is_none() {
            return Ok(geometry);
        }
        if !geometry.is_valid(block_start, self.wrap_width) || geometry.stalls(block_start) {
            return Err(LayoutInputError::InvalidLineGeometry);
        }
        if previous.is_some_and(|p| !geometry.is_subset_of(p)) {
            return Err(LayoutInputError::NonMonotonicGeometry);
        }
        Ok(geometry)
    }
}

/// One available inline interval; only its end may be positive infinity.
///
/// Parsed, not validated (spec §1.5, AX-3): [`new`](Self::new) accepts only a
/// finite start, an end strictly beyond it and a finite width (or an end of
/// positive infinity), so a constraint that exists is well formed. Whether it
/// suits a particular line, inside that line's inline envelope, depends on
/// the container and is checked there
/// ([`lies_in_envelope`](Self::lies_in_envelope)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InlineSlotConstraint {
    start: f32,
    end: f32,
}

impl InlineSlotConstraint {
    /// The interval `[start, end)`, or `None` unless `start` is finite, `end`
    /// lies strictly beyond it, and `end` is finite with a finite width or is
    /// positive infinity.
    #[must_use]
    pub fn new(start: f32, end: f32) -> Option<Self> {
        let is_well_formed = start.is_finite()
            && end > start
            && (end == f32::INFINITY || (end.is_finite() && (end - start).is_finite()));
        is_well_formed.then_some(Self { start, end })
    }

    /// Finite inline start; `>= 0` within the line's envelope.
    #[must_use]
    pub const fn start(self) -> f32 {
        self.start
    }

    /// End strictly beyond the start, possibly positive infinity (only when
    /// the wrap width is infinite); `<= wrap_width` within the line's envelope.
    #[must_use]
    pub const fn end(self) -> f32 {
        self.end
    }

    /// Available advance.
    #[must_use]
    pub fn width(self) -> f32 {
        self.end - self.start
    }

    /// Whether the slot lies in the inline envelope of a line whose wrap width
    /// is `wrap_width`: `0 <= start` and, when `wrap_width` is finite,
    /// `end <= wrap_width`, both within the crate's geometry tolerance. With an
    /// infinite `wrap_width` only the lower bound applies.
    #[must_use]
    pub fn lies_in_envelope(self, wrap_width: f32) -> bool {
        self.start >= -tolerance(self.start, 0.0)
            && (wrap_width == f32::INFINITY
                || (self.end.is_finite()
                    && self.end <= wrap_width + tolerance(self.end, wrap_width)))
    }
}

/// `true` when `slots` are the slots of one line of a container `wrap_width`
/// wide: each inside the envelope `[0, wrap_width]`, in
/// ascending order and pairwise disjoint.
pub(in crate::middleend) fn slots_are_valid(
    slots: &[InlineSlotConstraint],
    wrap_width: f32,
) -> bool {
    slots.iter().all(|s| s.lies_in_envelope(wrap_width))
        && slots.windows(2).all(|w| w[0].end <= w[1].start)
}

/// The least block progress an obstructed provider answer must offer
/// (spec GEO-4).
const MIN_CLEARANCE_STEP: f32 = 1e-4;

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
    /// inline width `wrap_width`: every slot lies in the
    /// envelope `[0, wrap_width]` ([`InlineSlotConstraint::lies_in_envelope`]),
    /// the slots are sorted and disjoint, and the next feature lies ahead by
    /// at most [`MAX_MEASUREMENT`](crate::types::MAX_MEASUREMENT) (spec MSR-1).
    #[must_use]
    pub fn is_valid(&self, block_start: f32, wrap_width: f32) -> bool {
        slots_are_valid(&self.slots, wrap_width)
            && self
                .next_feature_block_offset
                .is_none_or(|v| v > block_start && is_measurement(v - block_start))
    }

    /// Checks that an expanded block envelope introduced no new inline area.
    #[must_use]
    pub fn is_subset_of(&self, previous: &Self) -> bool {
        // Both slot lists are sorted: the only candidate container of a slot
        // is the first earlier slot that does not end before it.
        let mut candidates = previous.slots.iter().peekable();
        self.slots.iter().all(|slot| {
            while candidates.next_if(|p| p.end < slot.end).is_some() {}
            candidates
                .peek()
                .is_some_and(|p| slot.start >= p.start && slot.end <= p.end)
        })
    }

    /// Whether a fully obstructed answer fails to point at least
    /// `MIN_CLEARANCE_STEP` past `block_start`, so layout could not progress
    /// (spec GEO-4, G3).
    fn stalls(&self, block_start: f32) -> bool {
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
            slots: smallvec![InlineSlotConstraint::new(start, end).unwrap()],
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
        assert_eq!(InlineSlotConstraint::new(5.0, 5.0), None);
        assert_eq!(InlineSlotConstraint::new(f32::NAN, 5.0), None);
        assert_eq!(InlineSlotConstraint::new(0.0, f32::NAN), None);
        assert_eq!(InlineSlotConstraint::new(f32::NEG_INFINITY, 5.0), None);
        assert_eq!(InlineSlotConstraint::new(-f32::MAX, f32::MAX), None);
    }

    #[test]
    fn tab_stops_step_and_validity() {
        let t = TabStops::DEFAULT;
        assert_eq!(TabStops::new(8.0), Some(t));
        assert_eq!(t.next_stop(0.0), Some(8.0));
        assert_eq!(t.next_stop(7.9), Some(8.0));
        assert_eq!(t.next_stop(8.0), Some(16.0)); // on a stop: advance to the next
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(TabStops::new(bad), None);
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
