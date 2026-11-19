//! Input validation scales linearly (spec AX-7): at 4x the input, the
//! best of three runs must stay well under the 16x of quadratic work.
//!
//! Elapsed-time ratios are too noisy for `cargo test`, so this runs in
//! release mode on its own:
//!
//!     cargo bench --bench validation_scaling

#[allow(dead_code)]
#[path = "../tests/contracts/common.rs"]
mod common;

use common::*;
use mutil_engine::*;
use std::hint::black_box;
use std::time::Duration;

/// The largest ratio of the 5000-item run to the 1250-item run accepted as
/// linear: 4x work plus headroom for noise, far below 16x.
const MAX_RATIO: f64 = 7.0;

/// The best of three runs of input validation over `a`.
fn best_of_three(a: &ParagraphAst, t: &str) -> Duration {
    assert!(check(a, t).is_ok());
    (0..3)
        .map(|_| {
            let start = std::time::Instant::now();
            assert!(black_box(check(black_box(a), t)).is_ok());
            start.elapsed()
        })
        .min()
        .expect("three runs")
}

/// Measures `build` at 1250 and 5000 items and fails when the ratio is
/// beyond `MAX_RATIO`.
fn scales_linearly(name: &str, build: impl Fn(usize) -> (String, ParagraphAst)) -> bool {
    let measure = |n: usize| {
        let (t, a) = build(n);
        best_of_three(&a, &t)
    };
    let (small, large) = (measure(1250), measure(5000));
    let ratio = large.as_secs_f64() / small.as_secs_f64();
    let linear = ratio < MAX_RATIO;
    println!(
        "{name}: 1250 {small:?}, 5000 {large:?}, ratio {ratio:.2} {}",
        if linear { "ok" } else { "NOT LINEAR" }
    );
    linear
}

fn main() {
    let spans = scales_linearly("five thousand spans", spanned);
    let hyphens = scales_linearly("long hyphen chains", |n| {
        hyphen_chain_ast(n, |offset, n| {
            ((offset - 1) as u32, (n + 1 - offset) as u32)
        })
    });
    assert!(
        spans && hyphens,
        "4x input should approach 4x work, not 16x"
    );
}
