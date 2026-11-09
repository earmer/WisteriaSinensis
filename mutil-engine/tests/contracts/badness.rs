//! Piecewise badness across glue tiers.

use mutil_engine::*;

#[test]
fn badness_has_no_primary_only_capacity_cliff() {
    let c = TierCapacities {
        stretch: [0., 10., 0.],
        shrink: [0., 0.],
    };
    let a = line_fit(10., &c, 16.).badness;
    let b = line_fit(10.001, &c, 16.).badness;
    assert!(b >= a && b - a < 0.1, "{a} -> {b}");
}

fn capacity_endpoint(stretch: [f32; 3], expected: f32) {
    let cap = TierCapacities {
        stretch,
        shrink: [0., 0.],
    };
    let seam = cap.total_stretch();
    let at = line_fit(seam, &cap, 16.).badness;
    let after = line_fit(seam + 0.001, &cap, 16.).badness;
    assert!(
        (at - expected).abs() < 0.01,
        "endpoint {at}, expected {expected}"
    );
    assert!(after >= at && after - at < 0.1, "cliff {at} -> {after}");
}

#[test]
fn tier_zero_only_excess_starts_at_100() {
    capacity_endpoint([10., 0., 0.], 100.);
}

#[test]
fn tier_two_excess_starts_at_400() {
    capacity_endpoint([6., 4., 10.], 400.);
}

#[test]
fn zero_capacity_excess_starts_at_zero() {
    let cap = TierCapacities::default();
    let near = line_fit(0.002, &cap, 16.).badness;
    assert!(near > 0. && near < 0.01, "zero-capacity origin: {near}");
}
