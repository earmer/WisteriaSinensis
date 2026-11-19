//! The one geometry tolerance of the middle-end.
//!
//! Every comparison of layout coordinates, on the input side as well as on the
//! output side, uses the same mixed tolerance, so two validators never
//! disagree about whether two numbers are equal.

/// Absolute floor of the tolerance used when comparing layout coordinates.
pub const LINE_GEOMETRY_ABS_EPSILON: f32 = 1e-3;
/// Relative part of that tolerance. `f32` carries about 7 significant digits,
/// so sums over hundreds of glyphs at coordinates in the tens of thousands
/// drift by more than the absolute floor; `1e-5` covers that drift.
pub const LINE_GEOMETRY_REL_EPSILON: f32 = 1e-5;

/// The largest share of a line's height that `f32` rounding of its box may
/// lose (spec GEO-3, the output gate's `LineBoxCollapsed`).
pub const MAX_EXTENT_ROUNDING: f32 = 0.125;

/// The largest `f32` step at a block envelope's end, as a share of the
/// envelope, that the provider gate accepts (spec GEO-3). A quarter of
/// [`MAX_EXTENT_ROUNDING`]: a line box's edges are a few roundings away from
/// its envelope, and each loses at most half a step.
pub const MAX_POSITION_STEP: f32 = MAX_EXTENT_ROUNDING / 4.0;

/// Whether `f32` resolves an envelope `extent` long ending at `end` finely
/// enough: far from the origin `f32` steps grow, and a line smaller than a
/// few steps collapses.
pub(super) fn resolves_extent(end: f32, extent: f32) -> bool {
    end.next_up() - end <= MAX_POSITION_STEP * extent
}

/// Whether the span `[start, end)` of a line `line_height` tall keeps
/// `extent` within [`MAX_EXTENT_ROUNDING`] of the height (or the absolute
/// floor).
pub(super) fn keeps_extent(start: f32, end: f32, extent: f32, line_height: f32) -> bool {
    ((end - start) - extent).abs()
        <= LINE_GEOMETRY_ABS_EPSILON.max(MAX_EXTENT_ROUNDING * line_height.abs())
}

/// Mixed tolerance: `max(ABS, REL * max(|a|, |b|))`.
pub(super) fn tolerance(a: f32, b: f32) -> f32 {
    LINE_GEOMETRY_ABS_EPSILON.max(LINE_GEOMETRY_REL_EPSILON * a.abs().max(b.abs()))
}

/// `a` and `b` are equal within [`tolerance`]. Two equal infinities are equal;
/// NaN equals nothing.
pub(super) fn approx_eq(a: f32, b: f32) -> bool {
    a == b || (a.is_finite() && b.is_finite() && (a - b).abs() <= tolerance(a, b))
}

/// `a` is smaller than `b` by more than [`tolerance`]. NaN is smaller than
/// nothing.
pub(super) fn approx_lt(a: f32, b: f32) -> bool {
    a < b - tolerance(a, b)
}

/// [`approx_eq`] for sums accumulated in `f64`, with the same `f32`-scaled
/// tolerance.
pub(super) fn approx_eq_f64(a: f64, b: f64) -> bool {
    let tolerance = f64::from(LINE_GEOMETRY_ABS_EPSILON)
        .max(f64::from(LINE_GEOMETRY_REL_EPSILON) * a.abs().max(b.abs()));
    a == b || (a.is_finite() && b.is_finite() && (a - b).abs() <= tolerance)
}
