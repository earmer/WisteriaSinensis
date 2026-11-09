//! Tiered piecewise badness of a candidate line (spec §5 Stage 1.6.3).
//!
//! A pure function of the line's slack and its stretch / shrink capacities per
//! glue tier. The beam line breaker (not yet implemented) calls it once per
//! candidate; keeping it free of any layout state makes the curve testable on
//! its own.
//!
//! The capacities are the net sums over the visible content `(a, b_vis]`:
//! `Y_t` for `t in {0, 1, 2}` (stretch) and `Z_t` for `t in {0, 1}` (shrink;
//! Tier 2, the ideographs, never shrinks below natural width).

/// Slack with an absolute value at or below this counts as an exact fit.
pub const FIT_EPSILON: f32 = 1e-3;
/// Capacities at or below this count as zero.
pub const CAPACITY_EPSILON: f32 = 1e-5;
/// Badness of an overfull line (feasible in the emergency pass only).
pub const OVERFULL_BADNESS: f32 = 1_000_000.0;
/// Adjustment ratio reported for an overfull line.
pub const OVERFULL_RATIO: f32 = -2.0;
/// Ceiling of the badness of an underfull line with nothing to stretch.
pub const UNDERFULL_BADNESS_CAP: f32 = 100_000.0;
/// Maximum underfull badness; saturated candidates retain excess-slack ordering.
pub const MAX_BADNESS: f32 = UNDERFULL_BADNESS_CAP;

/// Stretch and shrink capacity of a candidate line, per tier.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TierCapacities {
    /// `Y_0`, `Y_1`, `Y_2`: how much each tier can widen the line.
    pub stretch: [f32; 3],
    /// `Z_0`, `Z_1`: how much each tier can narrow it.
    pub shrink: [f32; 2],
}

impl TierCapacities {
    /// `Y_0 + Y_1`: the primary stretch, spent before Tier 2.
    #[must_use]
    pub fn primary_stretch(&self) -> f32 {
        self.stretch[0] + self.stretch[1]
    }

    /// `Y_0 + Y_1 + Y_2`.
    #[must_use]
    pub fn total_stretch(&self) -> f32 {
        self.primary_stretch() + self.stretch[2]
    }

    /// `Z_0 + Z_1`.
    #[must_use]
    pub fn total_shrink(&self) -> f32 {
        self.shrink[0] + self.shrink[1]
    }
}

/// Adjustment ratio and badness of a candidate line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineFit {
    /// Negative when the line is compressed, in `[-1, 0)`; `0` for an exact
    /// fit; in `(0, 1]` while only Tiers 0 and 1 stretch; in `(1, 2]` while
    /// Tier 2 stretches; above `2` when even that is exhausted. `-2` for an
    /// overfull line.
    pub ratio: f32,
    /// Knuth-Plass badness; `0` is perfect.
    pub badness: f32,
    /// `true` when the line cannot be compressed to `wrap_width`.
    pub overfull: bool,
    /// `f64` bits of finite nonnegative excess slack, for saturated-score
    /// ties. Kept at `f64` precision so distinct excesses do not collapse to
    /// the same key.
    pub excess_slack_bits: u64,
}

impl LineFit {
    /// Lower keys win; excess slack breaks ties between saturated scores.
    #[must_use]
    pub fn comparison_key(&self) -> (u32, u64) {
        (
            // Adding zero turns `-0.0` into `+0.0`, so both sort alike.
            (self.badness + 0.0).to_bits(),
            self.excess_slack_bits,
        )
    }

    const OVERFULL: Self = Self {
        ratio: OVERFULL_RATIO,
        badness: OVERFULL_BADNESS,
        overfull: true,
        excess_slack_bits: 0,
    };
}

/// Badness of a line whose natural width is `wrap_width - slack`.
///
/// `slack = wrap_width - W(a, b)`: positive when the line is short. Five
/// exhaustive cases, continuous (C0) across the stretch boundaries except one:
/// with no Tier 2 capacity, case C ends at 100 and case E starts at 400, the
/// ruled step (spec row 97).
///
/// * **A** `|slack| <= FIT_EPSILON`: ratio 0, badness 0.
/// * **B** `slack < 0`: with `Z = Z_0 + Z_1`, overfull (`Z <= CAPACITY_EPSILON`
///   or `-slack > Z`) is ratio `-2`, badness `1_000_000`; otherwise ratio
///   `slack / Z` and badness `100 |r|^3`.
/// * **C** `slack <= Y_01`: ratio `slack / Y_01`, badness `100 r^3`.
/// * **D** `slack <= Y_01 + Y_2`: with `r2 = (slack - Y_01) / Y_2`, ratio
///   `1 + r2`, badness `100 + 300 r2^3` (100 at the seam, 400 at `Y_total`).
/// * **E** `slack > Y_total`. With `r_excess = (slack - Y_total) /
///   max(font_size, 1)`: ratio `2 + r_excess` and badness
///   `min(100_000, 400 + 400 r_excess^3)`. There is no branch on `Y_total`.
///   The cap sits below the overfull badness, so an underfull line always beats
///   an overfull one.
///
/// `font_size` normalises the excess slack of case E. Non-finite `slack`
/// or capacities yield the overfull fit (never a NaN badness).
#[must_use]
pub fn line_fit(slack: f32, cap: &TierCapacities, font_size: f32) -> LineFit {
    let inputs = [
        slack,
        font_size,
        cap.stretch[0],
        cap.stretch[1],
        cap.stretch[2],
        cap.shrink[0],
        cap.shrink[1],
    ];
    if !inputs.iter().all(|v| v.is_finite())
        || cap.stretch.iter().chain(&cap.shrink).any(|v| *v < 0.0)
    {
        return LineFit::OVERFULL;
    }
    // The sums run in f64, so capacities near `f32::MAX` cannot overflow to
    // infinity before the ratios are formed.
    let slack = f64::from(slack);
    let stretch = cap.stretch.map(f64::from);
    let z = f64::from(cap.shrink[0]) + f64::from(cap.shrink[1]);
    // The seams are also accepted at their f32 rounding, so a caller passing
    // `primary_stretch()` / `total_stretch()` (f32 sums) as the slack lands on
    // the boundary instead of just past it.
    let seam = |sum: f64| {
        let rounded = f64::from(sum as f32);
        if rounded.is_finite() {
            sum.max(rounded)
        } else {
            sum
        }
    };
    let y01 = seam(stretch[0] + stretch[1]);
    let y_total = seam(stretch[0] + stretch[1] + stretch[2]);
    let fit = |ratio: f64, badness: f64| LineFit {
        ratio: ratio as f32,
        badness: badness as f32,
        overfull: false,
        excess_slack_bits: 0,
    };
    let fit_excess = |ratio: f64, badness: f64, excess: f64| LineFit {
        ratio: ratio as f32,
        badness: badness as f32,
        overfull: false,
        excess_slack_bits: excess.max(0.0).to_bits(),
    };
    if slack.abs() <= f64::from(FIT_EPSILON) {
        return fit(0.0, 0.0);
    }
    if slack < 0.0 {
        if z <= f64::from(CAPACITY_EPSILON) || -slack > z {
            return LineFit::OVERFULL;
        }
        let ratio = slack / z;
        return fit(ratio, 100.0 * ratio.abs().powi(3));
    }
    let has_tier2 = stretch[2] > 0.0;
    let has_primary = y01 > 0.0;
    if slack <= y01 {
        let r = slack / y01;
        fit(r, 100.0 * r.powi(3))
    } else if slack <= y_total {
        let r2 = (slack - y01) / stretch[2];
        if has_primary {
            fit(1.0 + r2, 100.0 + 300.0 * r2.powi(3))
        } else {
            // Tier2 only: start at 0, reach 400 at top (C0 with A and E base).
            fit(1.0 + r2, 400.0 * r2.powi(3))
        }
    } else {
        // E: C0 base 400 (Tier2) / 100 (primary only) / 0 (none).
        let base: f64 = if has_tier2 {
            400.0
        } else if has_primary {
            100.0
        } else {
            0.0
        };
        let r_excess = (slack - y_total) / f64::from(font_size).max(1.0);
        let excess = slack - y_total;
        let bad = (base + 400.0 * r_excess.powi(3)).min(f64::from(MAX_BADNESS));
        fit_excess(2.0 + r_excess, bad, excess)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: TierCapacities = TierCapacities {
        stretch: [6.0, 4.0, 10.0],
        shrink: [3.0, 1.0],
    };

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-3 * a.abs().max(b.abs()).max(1.0)
    }

    #[test]
    fn exact_fit_is_perfect() {
        for s in [0.0, 0.0009, -0.0009] {
            let f = line_fit(s, &CAP, 16.0);
            assert_eq!((f.ratio, f.badness, f.overfull), (0.0, 0.0, false));
        }
    }

    #[test]
    fn compression_is_cubic_in_the_shrink_used() {
        let f = line_fit(-2.0, &CAP, 16.0); // Z = 4
        assert!(close(f.ratio, -0.5) && close(f.badness, 12.5) && !f.overfull);
        let full = line_fit(-4.0, &CAP, 16.0);
        assert!(close(full.ratio, -1.0) && close(full.badness, 100.0));
    }

    #[test]
    fn overfull_when_shrink_cannot_cover_it() {
        for (s, c) in [
            (-4.5, CAP),
            (-1.0, TierCapacities::default()),
            (
                -1.0,
                TierCapacities {
                    shrink: [5e-6, 0.0],
                    ..CAP
                },
            ),
        ] {
            let f = line_fit(s, &c, 16.0);
            assert!(f.overfull);
            assert_eq!((f.ratio, f.badness), (OVERFULL_RATIO, OVERFULL_BADNESS));
        }
    }

    #[test]
    fn primary_expansion() {
        let f = line_fit(5.0, &CAP, 16.0); // Y_01 = 10
        assert!(close(f.ratio, 0.5) && close(f.badness, 12.5));
        let edge = line_fit(10.0, &CAP, 16.0);
        assert!(close(edge.ratio, 1.0) && close(edge.badness, 100.0));
    }

    #[test]
    fn secondary_expansion_runs_from_100_to_400() {
        let f = line_fit(15.0, &CAP, 16.0); // r2 = 0.5
        assert!(close(f.ratio, 1.5) && close(f.badness, 137.5));
        let top = line_fit(20.0, &CAP, 16.0);
        assert!(close(top.ratio, 2.0) && close(top.badness, 400.0));
    }

    #[test]
    fn beyond_total_stretch_measures_the_excess_against_the_font_size() {
        // Y_total = 20, excess 20, font 16: r_excess = 1.25.
        let f = line_fit(40.0, &CAP, 16.0);
        assert!(close(f.ratio, 3.25) && close(f.badness, 400.0 + 400.0 * 1.953_125));
        // C0 at the seam with case D.
        let below = line_fit(20.0, &CAP, 16.0);
        let above = line_fit(20.01, &CAP, 16.0);
        assert!((above.badness - below.badness).abs() < 1.0);
        assert!((above.ratio - below.ratio).abs() < 0.01);
    }

    #[test]
    fn a_line_with_nothing_to_stretch_uses_the_same_rule() {
        let none = TierCapacities::default();
        let f = line_fit(32.0, &none, 16.0); // r_excess = 2, base 0
        assert!(close(f.ratio, 4.0) && close(f.badness, 3200.0));
        // Tiny font sizes fall back to a unit of 1.
        assert!(close(line_fit(2.0, &none, 0.0).ratio, 4.0));
        // The cap holds, and sits below the overfull badness.
        assert_eq!(line_fit(1.0e6, &none, 16.0).badness, UNDERFULL_BADNESS_CAP);
        const { assert!(UNDERFULL_BADNESS_CAP < OVERFULL_BADNESS) };
    }

    #[test]
    fn underfull_badness_is_monotone_in_stretch_capacity_and_bounded() {
        // For a fixed slack, more stretch capacity never costs more, across the
        // old 1e-5 threshold, and nothing exceeds the cap (so no f32 overflow).
        let slack = 100.0;
        let mut last = f32::INFINITY;
        for y in [5e-6, 1e-5, 2e-5, 1e-4, 1e-2, 1.0, 10.0, 50.0] {
            // Y=0 uses base 0, not on this chain
            let cap = TierCapacities {
                stretch: [y, 0.0, 0.0],
                ..Default::default()
            };
            let f = line_fit(slack, &cap, 16.0);
            assert!(!f.overfull);
            assert!(f.badness <= UNDERFULL_BADNESS_CAP);
            assert!(f.badness <= last + 1e-3, "badness rose at Y = {y}");
            last = f.badness;
        }
    }

    #[test]
    fn badness_never_decreases_as_the_line_gets_shorter() {
        let mut last = 0.0_f32;
        let mut slack = 0.0;
        while slack < 200.0 {
            let b = line_fit(slack, &CAP, 16.0).badness;
            assert!(b >= last - 1e-3, "badness fell at slack {slack}");
            last = b;
            slack += 0.25;
        }
    }

    #[test]
    fn capacities_near_f32_max_do_not_overflow() {
        let big = f32::MAX;
        let cap = TierCapacities {
            stretch: [big, big, 0.0],
            shrink: [big, big],
        };
        // Primary stretch is 2·MAX in f64: a slack of MAX is half of it.
        let f = line_fit(big, &cap, 16.0);
        assert!(close(f.ratio, 0.5) && close(f.badness, 12.5) && !f.overfull);
        let g = line_fit(-big, &cap, 16.0);
        assert!(close(g.ratio, -0.5) && close(g.badness, 12.5));
    }

    #[test]
    fn the_f32_helper_sums_land_on_the_seams() {
        // 0.1 + 0.2 rounds up in f32; the widened sum is slightly smaller.
        let cap = TierCapacities {
            stretch: [0.1, 0.2, 0.0],
            shrink: [0.0, 0.0],
        };
        let f = line_fit(cap.primary_stretch(), &cap, 16.0);
        assert!(close(f.ratio, 1.0) && close(f.badness, 100.0));
        let cap = TierCapacities {
            stretch: [0.1, 0.2, 0.3],
            shrink: [0.0, 0.0],
        };
        let f = line_fit(cap.total_stretch(), &cap, 16.0);
        assert!(close(f.ratio, 2.0) && close(f.badness, 400.0));
    }

    #[test]
    fn negative_zero_badness_sorts_as_positive_zero() {
        let positive = LineFit {
            ratio: 0.0,
            badness: 0.0,
            overfull: false,
            excess_slack_bits: 0,
        };
        let negative = LineFit {
            badness: -0.0,
            ..positive
        };
        assert_eq!(positive.comparison_key(), negative.comparison_key());
    }

    #[test]
    fn non_finite_input_is_overfull_not_nan() {
        for s in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(line_fit(s, &CAP, 16.0).overfull);
        }
        let bad = TierCapacities {
            stretch: [f32::NAN, 0.0, 0.0],
            ..CAP
        };
        assert!(line_fit(1.0, &bad, 16.0).overfull);
        let neg = TierCapacities {
            shrink: [-1.0, 0.0],
            ..CAP
        };
        assert!(line_fit(1.0, &neg, 16.0).overfull);
    }
}
