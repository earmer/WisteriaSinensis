//! Line geometry providers, clamping, truncation and ellipsis output.

use crate::common::*;
use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;

#[test]
fn unbounded_width_indent_and_clamp_options() {
    let a = ast("a");
    let mut constraints = LayoutConstraints::default();
    let c = check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()).unwrap();
    assert!(c.wrap_width.is_infinite());
    assert_eq!(line().validate(&c), Ok(()));
    constraints.wrap_width = 10.0;
    constraints.first_line_indent = 10.0;
    assert_eq!(
        check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()),
        Err(LayoutInputError::InvalidConstraints)
    );
    constraints.first_line_indent = -2.0;
    let c = check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()).unwrap();
    assert_eq!(c.slot_constraints[0].start, -2.0);
    assert_eq!(c.wrap_width, 10.0);
    for policy in [
        LineClampPolicy {
            max_lines: None,
            max_block_size: None,
            ellipsis: "…".into(),
        },
        LineClampPolicy::max_lines(0),
        LineClampPolicy::max_block_size(f32::INFINITY),
        LineClampPolicy {
            ellipsis: "\u{2028}".into(),
            ..LineClampPolicy::max_lines(1)
        },
    ] {
        let options = LayoutOptions {
            clamp: Some(policy),
            ..Default::default()
        };
        assert_eq!(
            check_layout_inputs(&a, "a", &constraints, &options),
            Err(LayoutInputError::InvalidLayoutOptions)
        );
    }
}

#[test]
fn truncation_and_multiple_synthesized_ellipsis_glyphs() {
    let mut l = line();
    set_visual_width(&mut l, 120.0);
    l.is_ellipsized = true;
    l.visual_runs = vec![run(glyph(0.0, 100.0, false))];
    for x in [100.0, 110.0] {
        l.visual_runs[0].glyphs.push(PositionedGlyph {
            byte_range: None,
            is_synthesized: true,
            ..glyph(x, 10.0, false)
        });
    }
    let p = PositionedParagraph {
        writing_mode: WritingMode::HorizontalTb,
        base_direction: Direction::Ltr,
        total_block_size: 10.0,
        lines: vec![l],
        truncated_range: Some(TextRange::new(1, 3)),
    };
    let options = LayoutOptions {
        clamp: Some(LineClampPolicy::max_lines(1)),
        ..Default::default()
    };
    assert_eq!(p.validate("abc", options.clamp.as_ref(), &ctx()), Ok(()));
    let mut bad = p.clone();
    bad.truncated_range = Some(TextRange::new(2, 3));
    assert!(bad.validate("abc", options.clamp.as_ref(), &ctx()).is_err());
    let mut bad = p.clone();
    let mut next = bad.lines[0].clone();
    next.baseline_y += 10.0;
    next.byte_range = TextRange::new(1, 2);
    bad.lines.push(next);
    bad.total_block_size = 20.0;
    assert!(bad.validate("abc", options.clamp.as_ref(), &ctx()).is_err());
    assert!(p.validate("abc", None, &ctx()).is_err());
}

#[derive(Debug)]
struct GrowingGeometry;

impl LineGeometryProvider for GrowingGeometry {
    fn query_line_geometry(&self, _: usize, _: f32, end: f32) -> LineGeometry {
        LineGeometry {
            slots: smallvec![InlineSlotConstraint {
                start: 0.0,
                end: if end > 10.0 { 200.0 } else { 100.0 }
            }],
            next_feature_block_offset: None,
            max_unobstructed_width: 200.0,
        }
    }
}

#[test]
fn provider_monotonicity_clearance_and_snapshots() {
    let constraints = LayoutConstraints {
        geometry_provider: Some(Arc::new(GrowingGeometry)),
        ..Default::default()
    };
    let mut a = ast("a");
    a.items[0] = WordNode::Box(BoxNode::InlineObject {
        baseline_align: BaselineAlignment::Auto,
        byte_range: TextRange::new(0, 1),
        box_id: 1,
        width: 10.0,
        ascent: 20.0,
        descent: 2.0,
        overflow_policy: BoxOverflowPolicy::Clip,
    });
    clear_text(&mut a);
    assert_eq!(
        layout_paragraph(
            &a,
            "a",
            &NoopShaper,
            &constraints,
            &LayoutOptions::default()
        ),
        Err(LayoutInputError::NonMonotonicGeometry)
    );
    let previous = LineGeometry {
        slots: smallvec![InlineSlotConstraint {
            start: 0.0,
            end: 100.0
        }],
        next_feature_block_offset: None,
        max_unobstructed_width: 100.0,
    };
    let split = LineGeometry {
        slots: smallvec![
            InlineSlotConstraint {
                start: 0.0,
                end: 20.0
            },
            InlineSlotConstraint {
                start: 40.0,
                end: 100.0
            }
        ],
        ..previous.clone()
    };
    assert!(split.is_subset_of(&previous));
    assert!(!previous.is_subset_of(&split));
    let (l, c) = two_slots();
    let p = PositionedParagraph {
        writing_mode: WritingMode::HorizontalTb,
        base_direction: Direction::Ltr,
        total_block_size: 10.0,
        truncated_range: None,
        lines: vec![l],
    };
    assert_eq!(p.validate("ab", None, &c), Ok(()));
}

#[test]
fn probe_ignores_tall_input_after_first_break() {
    let constraints = LayoutConstraints {
        geometry_provider: Some(Arc::new(GrowingGeometry)),
        ..Default::default()
    };
    let text = "a\u{2028}b";
    let mut a = ast(text);
    rebuild_items(&mut a, |b| {
        vec![
            boxed(b, 0, 1, false),
            WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(1, 4), false)),
            WordNode::Box(BoxNode::InlineObject {
                baseline_align: BaselineAlignment::Auto,
                byte_range: TextRange::new(4, 5),
                box_id: 1,
                width: 10.0,
                ascent: 20.0,
                descent: 2.0,
                overflow_policy: BoxOverflowPolicy::Clip,
            }),
            end(5),
        ]
    });
    assert_eq!(
        layout_paragraph(
            &a,
            text,
            &NoopShaper,
            &constraints,
            &LayoutOptions::default()
        ),
        Ok(PositionedParagraph::empty(Direction::Ltr))
    );
}

#[test]
fn infinity_does_not_equal_finite_slot_capacity() {
    let (mut l, c) = two_slots();
    l.slots[0].constraint_width = f32::INFINITY;
    assert_eq!(l.validate(&c), Err(LineInvariantError::InvalidSlots));
    let mut c = c;
    c.slot_constraints[0].start = -f32::MAX;
    c.slot_constraints[0].end = f32::MAX;
    assert!(l.validate(&c).is_err());
}

#[test]
fn ellipsis_only_empty_and_empty_paragraph_contracts() {
    let mut l = line();
    l.byte_range = TextRange::empty_at(0);
    l.visible_byte_end = 0;
    l.is_ellipsized = true;
    set_visual_width(&mut l, 120.0);
    let mut r = run(glyph(0.0, 120.0, false));
    r.byte_range = TextRange::empty_at(0);
    r.glyphs[0].is_synthesized = true;
    r.glyphs[0].byte_range = None;
    l.visual_runs = vec![r];
    let mut p = PositionedParagraph::empty(Direction::Ltr);
    p.total_block_size = 10.0;
    p.lines = vec![l];
    p.truncated_range = Some(TextRange::new(0, 1));
    let mut options = LayoutOptions {
        clamp: Some(LineClampPolicy::max_lines(1)),
        ..Default::default()
    };
    assert_eq!(p.validate("a", options.clamp.as_ref(), &ctx()), Ok(()));
    p.lines[0].visual_runs.clear();
    set_visual_width(&mut p.lines[0], 0.0);
    options.clamp.as_mut().unwrap().ellipsis = "".into();
    assert_eq!(p.validate("a", options.clamp.as_ref(), &ctx()), Ok(()));
    p.truncated_range = None;
    p.lines[0].is_ellipsized = false;
    assert_eq!(p.validate("", None, &ctx()), Ok(()));
    p.lines.clear();
    assert!(p.validate("", None, &ctx()).is_err());
}

#[derive(Debug)]
struct EmptyGeometry {
    next: Option<f32>,
}

impl LineGeometryProvider for EmptyGeometry {
    fn query_line_geometry(&self, _: usize, _: f32, _: f32) -> LineGeometry {
        LineGeometry {
            slots: smallvec![],
            next_feature_block_offset: self.next,
            max_unobstructed_width: 100.0,
        }
    }
}

#[test]
fn geometry_rejects_empty_without_progress_and_invalid_intervals() {
    for next in [
        None,
        Some(0.0),
        Some(1e-6),
        Some(f32::NAN),
        Some(f32::INFINITY),
    ] {
        let constraints = LayoutConstraints {
            geometry_provider: Some(Arc::new(EmptyGeometry { next })),
            ..Default::default()
        };
        assert_eq!(
            layout_paragraph(
                &ast("a"),
                "a",
                &NoopShaper,
                &constraints,
                &LayoutOptions::default()
            ),
            Err(LayoutInputError::InvalidLineGeometry)
        );
    }
    let mut geometry = LineGeometry {
        slots: smallvec![
            InlineSlotConstraint {
                start: 0.0,
                end: 100.0
            },
            InlineSlotConstraint {
                start: 90.0,
                end: 200.0
            }
        ],
        next_feature_block_offset: None,
        max_unobstructed_width: 200.0,
    };
    assert!(!geometry.is_valid(0.0));
    geometry.slots[1].start = 100.0;
    assert!(geometry.is_valid(0.0));
    geometry.slots[1].end = f32::INFINITY;
    assert!(geometry.is_valid(0.0));
    geometry.slots[0].start = -f32::MAX;
    geometry.slots[0].end = f32::MAX;
    assert!(!geometry.is_valid(0.0));
}
