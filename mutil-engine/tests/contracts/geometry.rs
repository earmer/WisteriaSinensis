//! Line geometry providers, clamping, truncation and ellipsis output.

use crate::common::*;
use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;

#[test]
fn unbounded_width_indent_and_clamp_options() {
    let a = ast("a");
    let mut constraints = LayoutConstraints::default();
    let c = LineContext::for_paragraph(&a, &constraints, &LayoutOptions::default(), 0).unwrap();
    assert!(c.wrap_width.is_infinite());
    let mut empty_body = line();
    set_visual_width(&mut empty_body, 0.0);
    empty_body.slots[0].constraint.end = f32::INFINITY;
    assert_eq!(empty_body.validate(&c), Ok(()));
    constraints.wrap_width = 10.0;
    constraints.first_line_indent = 10.0;
    assert_eq!(
        check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()),
        Err(LayoutInputError::InvalidConstraints)
    );
    // Standard slots start at the indent and must lie in `[0, wrap_width]`.
    constraints.first_line_indent = -2.0;
    assert_eq!(
        check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()),
        Err(LayoutInputError::InvalidConstraints)
    );
    constraints.first_line_indent = 0.0;
    constraints.hanging_indent = -2.0;
    assert_eq!(
        check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()),
        Err(LayoutInputError::InvalidConstraints)
    );
    constraints.hanging_indent = 0.0;
    constraints.first_line_indent = 2.0;
    assert_eq!(
        check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()),
        Ok(())
    );
    // The first line's standard slot starts at the indent.
    assert_eq!(
        constraints.resolve_line_geometry(0, 0.0, 10.0).slots[..],
        [InlineSlotConstraint {
            start: 2.0,
            end: 10.0
        }]
    );
    let c = LineContext::for_paragraph(&a, &constraints, &LayoutOptions::default(), 0).unwrap();
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
    let mut p = paragraph(&ctx(), vec![l]);
    p.total_block_size = 10.0;
    p.truncated_range = Some(TextRange::new(1, 3));
    p.policy.clamp = Some(LineClampPolicy::max_lines(1));
    assert_eq!(p.validate("abc"), Ok(()));
    let mut bad = p.clone();
    bad.truncated_range = Some(TextRange::new(2, 3));
    assert_eq!(
        bad.validate("abc"),
        Err((
            0,
            LineInvariantError::InvalidParagraph {
                reason: ParagraphErrorReason::InvalidTruncation
            }
        ))
    );
    let mut bad = p.clone();
    let mut next = bad.lines[0].clone();
    next.baseline_y += 10.0;
    next.byte_range = TextRange::new(1, 2);
    next.visible_byte_end = 2;
    next.visual_runs[0].byte_range = next.byte_range;
    next.visual_runs[0].glyphs[0].byte_range = Some(next.byte_range);
    bad.lines.push(next);
    bad.total_block_size = 20.0;
    // Only the last line may carry the ellipsis.
    assert_eq!(
        bad.validate("abc"),
        Err((
            0,
            LineInvariantError::InvalidParagraph {
                reason: ParagraphErrorReason::InvalidTruncation
            }
        ))
    );
    // A truncated paragraph was laid out under a clamp.
    let mut unclamped = p.clone();
    unclamped.policy.clamp = None;
    assert_eq!(
        unclamped.validate("abc"),
        Err((
            0,
            LineInvariantError::InvalidParagraph {
                reason: ParagraphErrorReason::InvalidTruncation
            }
        ))
    );
    // The recorded clamp must itself be valid.
    let mut invalid = p;
    invalid.policy.clamp = Some(LineClampPolicy::max_lines(0));
    assert_eq!(
        invalid.validate("abc"),
        Err((
            0,
            LineInvariantError::InvalidParagraph {
                reason: ParagraphErrorReason::InvalidPolicy
            }
        ))
    );
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
        bidi_level: 0,
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
    let mut p = paragraph(&c, vec![l]);
    p.total_block_size = 10.0;
    // Two slots come only from a provider: the standard column has one.
    assert_eq!(
        p.validate("ab"),
        Err((
            0,
            LineInvariantError::InvalidParagraph {
                reason: ParagraphErrorReason::NotStandardSlots
            }
        ))
    );
    p.policy.slot_source = SlotSource::Provider;
    assert_eq!(p.validate("ab"), Ok(()));
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
                bidi_level: 0,
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
        )
        .map(|paragraph| paragraph.lines.len()),
        Ok(0)
    );
}

#[test]
fn infinity_does_not_equal_finite_slot_capacity() {
    // Only an unbounded container admits an unbounded slot.
    let (mut l, c) = two_slots();
    l.slots[1].constraint.end = f32::INFINITY;
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::InvalidSlots {
            reason: SlotErrorReason::InvalidConstraint
        })
    );
    // A slot whose width overflows is malformed.
    let (mut l, c) = two_slots();
    l.slots[0].constraint.start = -f32::MAX;
    l.slots[0].constraint.end = f32::MAX;
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::InvalidSlots {
            reason: SlotErrorReason::InvalidConstraint
        })
    );
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
    let mut p = paragraph(&ctx(), vec![l]);
    p.total_block_size = 10.0;
    p.truncated_range = Some(TextRange::new(0, 1));
    p.policy.clamp = Some(LineClampPolicy::max_lines(1));
    assert_eq!(p.validate("a"), Ok(()));
    p.lines[0].visual_runs.clear();
    set_visual_width(&mut p.lines[0], 0.0);
    p.policy.clamp.as_mut().unwrap().ellipsis = "".into();
    assert_eq!(p.validate("a"), Ok(()));
    p.truncated_range = None;
    p.policy.clamp = None;
    p.lines[0].is_ellipsized = false;
    assert_eq!(p.validate(""), Ok(()));
    p.lines.clear();
    assert_eq!(
        p.validate(""),
        Err((
            0,
            LineInvariantError::InvalidParagraph {
                reason: ParagraphErrorReason::NoLines
            }
        ))
    );
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
    };
    assert!(!geometry.is_valid(0.0, 200.0));
    geometry.slots[1].start = 100.0;
    assert!(geometry.is_valid(0.0, 200.0));
    // Only an infinite container admits an infinite slot end.
    geometry.slots[1].end = f32::INFINITY;
    assert!(!geometry.is_valid(0.0, 200.0));
    assert!(geometry.is_valid(0.0, f32::INFINITY));
    geometry.slots[0].start = -f32::MAX;
    geometry.slots[0].end = f32::MAX;
    assert!(!geometry.is_valid(0.0, f32::INFINITY));
}

#[derive(Debug)]
struct OneSlot {
    start: f32,
    end: f32,
}

impl LineGeometryProvider for OneSlot {
    fn query_line_geometry(&self, _: usize, _: f32, _: f32) -> LineGeometry {
        LineGeometry {
            slots: smallvec![InlineSlotConstraint {
                start: self.start,
                end: self.end
            }],
            next_feature_block_offset: None,
        }
    }
}

#[test]
fn provider_slots_must_lie_inside_the_wrap_envelope() {
    let laid_out = |wrap_width: f32, start: f32, end: f32| {
        let constraints = LayoutConstraints {
            wrap_width,
            geometry_provider: Some(Arc::new(OneSlot { start, end })),
            ..Default::default()
        };
        layout_paragraph(
            &ast("a"),
            "a",
            &NoopShaper,
            &constraints,
            &LayoutOptions::default(),
        )
        .map(|paragraph| paragraph.lines.len())
    };
    let accepted = Ok(0);
    assert_eq!(laid_out(100.0, 0.0, 100.0), accepted);
    assert_eq!(laid_out(100.0, 10.0, 60.0), accepted);
    for (start, end) in [(-5.0, 50.0), (0.0, 150.0), (0.0, f32::INFINITY)] {
        assert_eq!(
            laid_out(100.0, start, end),
            Err(LayoutInputError::InvalidLineGeometry),
            "{start}..{end}"
        );
    }
    // An unbounded container only needs a non-negative start.
    assert_eq!(laid_out(f32::INFINITY, 0.0, f32::INFINITY), accepted);
    assert_eq!(laid_out(f32::INFINITY, 20.0, 500.0), accepted);
    assert_eq!(
        laid_out(f32::INFINITY, -1.0, 5.0),
        Err(LayoutInputError::InvalidLineGeometry)
    );
}

#[test]
fn layout_options_and_tab_stops_reject_every_bad_float() {
    let a = ast("a");
    let invalid_options = |options: LayoutOptions| {
        check_layout_inputs(&a, "a", &LayoutConstraints::default(), &options)
    };
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(
            invalid_options(LayoutOptions {
                clamp: Some(LineClampPolicy::max_block_size(bad)),
                ..Default::default()
            }),
            Err(LayoutInputError::InvalidLayoutOptions),
            "max_block_size {bad}"
        );
    }
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            invalid_options(LayoutOptions {
                line_height: LineHeight::Fixed(bad),
                ..Default::default()
            }),
            Err(LayoutInputError::InvalidStrut),
            "line height {bad}"
        );
        assert_eq!(
            check_layout_inputs(
                &a,
                "a",
                &LayoutConstraints {
                    tab_stops: TabStops { step: bad },
                    ..Default::default()
                },
                &LayoutOptions::default()
            ),
            Err(LayoutInputError::InvalidConstraints),
            "tab step {bad}"
        );
    }
    for bad in [f32::NAN, f32::NEG_INFINITY, 0.0, -1.0] {
        assert_eq!(
            check_layout_inputs(
                &a,
                "a",
                &LayoutConstraints {
                    wrap_width: bad,
                    ..Default::default()
                },
                &LayoutOptions::default()
            ),
            Err(LayoutInputError::InvalidConstraints),
            "wrap width {bad}"
        );
    }
    // Positive infinity disables wrapping.
    assert!(
        check_layout_inputs(
            &a,
            "a",
            &LayoutConstraints::default(),
            &LayoutOptions::default()
        )
        .is_ok()
    );
}
