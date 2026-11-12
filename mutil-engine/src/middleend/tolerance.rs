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

/// Mixed tolerance: `max(ABS, REL * max(|a|, |b|))`.
pub(super) fn tolerance(a: f32, b: f32) -> f32 {
    LINE_GEOMETRY_ABS_EPSILON.max(LINE_GEOMETRY_REL_EPSILON * a.abs().max(b.abs()))
}

/// `a` and `b` are equal within [`tolerance`]. Two equal infinities are equal;
/// NaN equals nothing.
pub(super) fn approx_eq(a: f32, b: f32) -> bool {
    a == b || (a.is_finite() && b.is_finite() && (a - b).abs() <= tolerance(a, b))
}

/// [`approx_eq`] for sums accumulated in `f64`, with the same `f32`-scaled
/// tolerance.
pub(super) fn approx_eq_f64(a: f64, b: f64) -> bool {
    let tolerance = f64::from(LINE_GEOMETRY_ABS_EPSILON)
        .max(f64::from(LINE_GEOMETRY_REL_EPSILON) * a.abs().max(b.abs()));
    a == b || (a.is_finite() && b.is_finite() && (a - b).abs() <= tolerance)
}
