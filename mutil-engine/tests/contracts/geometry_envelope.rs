//! The envelope check of the provider gate G3 (GEO-3): an envelope `f32`
//! cannot hold is `BlockPositionUnrepresentable`, with or without a
//! provider, before any answer is judged; and the sealed answer G3 returns.

use crate::common::*;
use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;

/// Answers `self.0` whatever the query.
#[derive(Debug)]
struct Fixed(LineGeometry);

impl LineGeometryProvider for Fixed {
    fn query_line_geometry(&self, _: usize, _: f32, _: f32) -> LineGeometry {
        self.0.clone()
    }
}

fn one_slot(start: f32, end: f32) -> LineGeometry {
    LineGeometry {
        slots: smallvec![InlineSlotConstraint::try_new(start, end).unwrap()],
        next_feature_block_offset: None,
    }
}

fn obstructed(next: Option<f32>) -> LineGeometry {
    LineGeometry {
        slots: smallvec![],
        next_feature_block_offset: next,
    }
}

/// G3's answer for line `line` at `block_start` with `extent`, under
/// `constraints` (and the provider in them, if any).
fn query(
    constraints: &LayoutConstraints,
    line: usize,
    block_start: f32,
    extent: f32,
) -> Result<GeometryAnswer, LayoutInputError> {
    let ast = ast("a");
    let options = LayoutOptions::default();
    let validated = ValidatedInputs::try_from_ast(&ast, "a", constraints, &options)
        .expect("the inputs are valid");
    validated.line_geometry(line, block_start, extent, None)
}

fn with_provider(geometry: LineGeometry) -> LayoutConstraints {
    LayoutConstraints {
        geometry_provider: Some(Arc::new(Fixed(geometry))),
        ..constraints()
    }
}

/// Both kinds of constraints: the standard slot and a provider's open slot.
fn both() -> [(&'static str, LayoutConstraints); 2] {
    [
        ("standard", constraints()),
        ("provider", with_provider(one_slot(0.0, 50.0))),
    ]
}

/// The `f32` step just above `value` (positive `value`).
fn step_at(value: f32) -> f32 {
    value.next_up() - value
}

#[test]
fn geo3_the_position_step_is_a_quarter_of_the_extent_rounding() {
    assert_eq!(MAX_EXTENT_ROUNDING, 0.125);
    assert_eq!(MAX_POSITION_STEP, MAX_EXTENT_ROUNDING / 4.0);
}

#[test]
fn geo3_an_envelope_far_from_the_origin_is_block_position_unrepresentable() {
    for (name, constraints) in both() {
        for block_start in [3e8, 5e7, -3e8, 1e30] {
            assert_eq!(
                query(&constraints, 0, block_start, 10.0).map(drop),
                Err(LayoutInputError::BlockPositionUnrepresentable),
                "{name} at {block_start}"
            );
        }
        // Near the origin the same envelope is fine.
        for block_start in [0.0, 1000.0, 1e5] {
            assert!(
                query(&constraints, 0, block_start, 10.0).is_ok(),
                "{name} at {block_start}"
            );
        }
    }
}

#[test]
fn geo3_a_non_finite_or_empty_envelope_is_block_position_unrepresentable() {
    for (name, constraints) in both() {
        for (block_start, extent) in [
            (f32::NAN, 10.0),
            (f32::INFINITY, 10.0),
            (f32::NEG_INFINITY, 10.0),
            (0.0, 0.0),
            (5.0, -1.0),
            (5.0, f32::NAN),
            (5.0, f32::INFINITY),
            // The end rounds back onto the start.
            (1.0, 1e-9),
            (f32::MAX, 10.0),
        ] {
            assert_eq!(
                query(&constraints, 0, block_start, extent).map(drop),
                Err(LayoutInputError::BlockPositionUnrepresentable),
                "{name}: start {block_start}, extent {extent}"
            );
        }
    }
}

#[test]
fn geo3_the_step_at_the_envelope_end_is_bounded_by_max_position_step_of_the_extent() {
    let extent = 10.0;
    let bound = MAX_POSITION_STEP * extent;
    // 2e6 + 10 has a step of 0.125, within 0.3125; 4.2e6 + 10 one of 0.5.
    let (near, far) = (2.0e6_f32, 4.2e6_f32);
    assert!(step_at(near + extent) <= bound);
    assert!(step_at(far + extent) > bound);
    for (name, constraints) in both() {
        assert!(query(&constraints, 0, near, extent).is_ok(), "{name}");
        assert_eq!(
            query(&constraints, 0, far, extent).map(drop),
            Err(LayoutInputError::BlockPositionUnrepresentable),
            "{name}"
        );
        // A taller envelope at the same far start has room again.
        let tall = 2.0 * step_at(far + 40.0) / MAX_POSITION_STEP;
        assert!(query(&constraints, 0, far, tall).is_ok(), "{name}: {tall}");
    }
}

#[test]
fn geo3_the_envelope_is_checked_before_the_provider_answer() {
    // An answer G3 would reject as `InvalidLineGeometry` ...
    let bad = with_provider(obstructed(None));
    assert_eq!(
        query(&bad, 0, 2.0, 10.0).map(drop),
        Err(LayoutInputError::InvalidLineGeometry)
    );
    // ... is not reached when the envelope itself is unrepresentable.
    assert_eq!(
        query(&bad, 0, 3e8, 10.0).map(drop),
        Err(LayoutInputError::BlockPositionUnrepresentable)
    );
}

#[test]
fn geo3_an_open_answer_has_a_first_slot() {
    let geometry = LineGeometry {
        slots: smallvec![
            InlineSlotConstraint::try_new(5.0, 20.0).unwrap(),
            InlineSlotConstraint::try_new(30.0, 60.0).unwrap()
        ],
        next_feature_block_offset: None,
    };
    let Ok(GeometryAnswer::Open(open)) = query(&with_provider(geometry.clone()), 0, 2.0, 10.0)
    else {
        panic!("a non-empty answer is open");
    };
    assert_eq!(open.geometry(), &geometry);
    assert_eq!(open.first_slot(), geometry.slots[0]);
}

#[test]
fn geo3_without_a_provider_the_answer_is_the_standard_slot() {
    let indented = LayoutConstraints {
        first_line_indent: 5.0,
        hanging_indent: 7.0,
        ..constraints()
    };
    for (line, start) in [(0, 5.0), (1, 7.0), (4, 7.0)] {
        let Ok(GeometryAnswer::Open(open)) = query(&indented, line, 12.0, 10.0) else {
            panic!("the standard geometry is open");
        };
        assert_eq!(
            open.first_slot(),
            InlineSlotConstraint::try_new(start, 100.0).unwrap(),
            "line {line}"
        );
        assert_eq!(open.geometry().slots.len(), 1);
    }
}

#[test]
fn geo3_an_obstructed_answer_carries_the_clearance_it_proved() {
    let Ok(GeometryAnswer::Obstructed(clearance)) =
        query(&with_provider(obstructed(Some(40.0))), 0, 2.0, 10.0)
    else {
        panic!("an empty answer with progress is obstructed");
    };
    assert_eq!(clearance.clear_at(), 40.0);
}
