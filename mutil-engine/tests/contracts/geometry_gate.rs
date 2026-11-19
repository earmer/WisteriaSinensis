//! The provider gate G3 (GEO-3, GEO-4): obstructed answers, their progress
//! ahead, the clearance bound and the `GeometryAnswer` the gate returns.

use crate::common::*;
use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The most obstructed answers in a row one line accepts (GEO-4).
const CLEARANCE_LIMIT: usize = 1024;

fn one_slot(end: f32) -> LineGeometry {
    LineGeometry {
        slots: smallvec![InlineSlotConstraint::try_new(0.0, end).unwrap()],
        next_feature_block_offset: None,
    }
}

fn obstructed(next: Option<f32>) -> LineGeometry {
    LineGeometry {
        slots: smallvec![],
        next_feature_block_offset: next,
    }
}

fn lay_out_with(
    provider: impl LineGeometryProvider + 'static,
    text: &str,
) -> Result<PositionedParagraph, LayoutInputError> {
    let constraints = LayoutConstraints {
        geometry_provider: Some(Arc::new(provider)),
        ..Default::default()
    };
    layout_paragraph(
        &ast(text),
        text,
        &TEST_SHAPER,
        &constraints,
        &LayoutOptions::default(),
    )
}

/// Obstructed while the block start is below `until`, always pointing one
/// layout unit ahead; open afterwards. Counts its queries.
#[derive(Debug)]
struct ObstructedUntil {
    until: f32,
    queries: Arc<AtomicUsize>,
}

impl LineGeometryProvider for ObstructedUntil {
    fn query_line_geometry(&self, _: usize, block_start: f32, _: f32) -> LineGeometry {
        self.queries.fetch_add(1, Ordering::Relaxed);
        if block_start < self.until {
            obstructed(Some(block_start + 1.0))
        } else {
            one_slot(100.0)
        }
    }
}

#[test]
fn geo4_endless_obstruction_with_progress_is_clearance_exhausted() {
    let queries = Arc::new(AtomicUsize::new(0));
    let provider = ObstructedUntil {
        until: f32::INFINITY,
        queries: queries.clone(),
    };
    assert_eq!(
        lay_out_with(provider, "a").map(drop),
        Err(LayoutInputError::ClearanceExhausted)
    );
    let made = queries.load(Ordering::Relaxed);
    assert!(made > CLEARANCE_LIMIT, "{made} queries");
    assert!(made < 4 * CLEARANCE_LIMIT, "{made} queries");
}

#[test]
fn geo4_the_clearance_limit_is_more_than_1024_obstructed_answers_in_a_row() {
    let at = |obstructed_answers: usize| {
        lay_out_with(
            ObstructedUntil {
                until: obstructed_answers as f32,
                queries: Arc::default(),
            },
            "a",
        )
        .map(|p| p.lines.len())
    };
    assert_eq!(at(10), Ok(1));
    assert_eq!(at(CLEARANCE_LIMIT), Ok(1));
    assert_eq!(
        at(CLEARANCE_LIMIT + 1),
        Err(LayoutInputError::ClearanceExhausted)
    );
}

type Next = fn(f32) -> Option<f32>;

/// A one-unit slot for line 0 and a stalled obstruction for every later line.
#[derive(Debug)]
struct StallsAfterFirstLine {
    next: Next,
}

impl LineGeometryProvider for StallsAfterFirstLine {
    fn query_line_geometry(&self, line: usize, block_start: f32, _: f32) -> LineGeometry {
        if line == 0 {
            one_slot(1.0)
        } else {
            obstructed((self.next)(block_start))
        }
    }
}

#[test]
fn geo3_an_obstruction_without_progress_is_invalid_line_geometry_not_exhaustion() {
    let stalls: [(&str, Next); 6] = [
        ("none", |_| None),
        ("at the block start", Some),
        ("behind the block start", |start| Some(start - 0.5)),
        ("behind, at zero", |_| Some(0.0)),
        ("negative", |_| Some(-3.0)),
        ("too little progress", |start| Some(start + 1e-6)),
    ];
    for (name, next) in stalls {
        // A slot one unit wide forces the word onto several lines, so the
        // later lines' obstruction is reached.
        assert_eq!(
            lay_out_with(StallsAfterFirstLine { next }, "abcd").map(drop),
            Err(LayoutInputError::InvalidLineGeometry),
            "{name}"
        );
    }
}

#[derive(Debug)]
struct FirstLineObstructed(Option<f32>);

impl LineGeometryProvider for FirstLineObstructed {
    fn query_line_geometry(&self, _: usize, _: f32, _: f32) -> LineGeometry {
        obstructed(self.0)
    }
}

#[test]
fn geo3_a_first_line_obstruction_behind_or_at_the_start_is_invalid_line_geometry() {
    for next in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(-1e-3),
        Some(f32::NEG_INFINITY),
    ] {
        assert_eq!(
            lay_out_with(FirstLineObstructed(next), "a").map(drop),
            Err(LayoutInputError::InvalidLineGeometry),
            "{next:?}"
        );
    }
}

/// Answers `self.0` whatever the query.
#[derive(Debug)]
struct Fixed(LineGeometry);

impl LineGeometryProvider for Fixed {
    fn query_line_geometry(&self, _: usize, _: f32, _: f32) -> LineGeometry {
        self.0.clone()
    }
}

fn answer(geometry: LineGeometry, block_start: f32) -> Result<GeometryAnswer, LayoutInputError> {
    let ast = ast("a");
    let constraints = LayoutConstraints {
        geometry_provider: Some(Arc::new(Fixed(geometry))),
        ..Default::default()
    };
    let options = LayoutOptions::default();
    let validated = ValidatedInputs::try_from_ast(&ast, "a", &constraints, &options)
        .expect("the inputs are valid");
    validated.line_geometry(0, block_start, 10.0, None)
}

#[test]
fn an_empty_answer_with_a_valid_offset_is_obstructed_with_its_clear_at() {
    assert_eq!(
        answer(obstructed(Some(7.5)), 2.0).map(|a| a.geometry()),
        Ok(obstructed(Some(7.5)))
    );
    assert!(matches!(
        answer(obstructed(Some(7.5)), 2.0),
        Ok(GeometryAnswer::Obstructed(c)) if c.clear_at() == 7.5
    ));
}

#[test]
fn a_non_empty_answer_is_open() {
    let open = one_slot(40.0);
    assert_eq!(
        answer(open.clone(), 2.0).map(|a| a.geometry()),
        Ok(open.clone())
    );
    assert!(matches!(
        answer(open.clone(), 2.0),
        Ok(GeometryAnswer::Open(_))
    ));
    // An offset on an open answer changes nothing.
    let with_offset = LineGeometry {
        next_feature_block_offset: Some(9.0),
        ..open
    };
    assert_eq!(
        answer(with_offset.clone(), 2.0).map(|a| a.geometry()),
        Ok(with_offset)
    );
}

#[test]
fn the_gate_rejects_an_empty_answer_without_progress_ahead() {
    for next in [None, Some(2.0), Some(1.0), Some(f32::NAN)] {
        assert_eq!(
            answer(obstructed(next), 2.0),
            Err(LayoutInputError::InvalidLineGeometry),
            "{next:?}"
        );
    }
}
