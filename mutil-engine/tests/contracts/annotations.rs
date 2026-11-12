//! Ruby annotation inputs and positioned annotation runs.

use crate::common::*;
use mutil_engine::*;

#[test]
fn annotation_input_boundaries_and_local_partition() {
    let mut a = ast("ab");
    a.annotations = vec![
        annotation(TextRange::new(0, 2), AnnotationSide::Over),
        annotation(TextRange::new(0, 2), AnnotationSide::Under),
    ];
    assert!(check(&a, "ab").is_ok());
    a.annotations[1].side = AnnotationSide::Over;
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 1 })
    );
    a.annotations.truncate(1);
    a.annotations[0].clusters.clear();
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
    a.annotations[0] = annotation(TextRange::new(0, 2), AnnotationSide::Over);
    rebuild_items(&mut a, |b| {
        vec![
            boxed(b, 0, 1, false),
            WordNode::Discretionary(DiscretionaryNode::direct(1)),
            boxed(b, 1, 2, false),
            end(2),
        ]
    });
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
    let mut a = ast("ab");
    a.annotations = vec![annotation(TextRange::new(0, 2), AnnotationSide::Over)];
    a.annotations[0].kind = AnnotationKind::Emphasis;
    assert!(check(&a, "ab").is_ok());
    a.annotations[0].run.letter_spacing = f32::NAN;
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
    a.annotations[0] = annotation(TextRange::new(0, 2), AnnotationSide::Over);
    rebuild_items(&mut a, |b| {
        vec![
            boxed(b, 0, 1, false),
            WordNode::Tab {
                bidi_level: 0,
                byte_range: TextRange::new(1, 2),
            },
            end(2),
        ]
    });
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
}

#[test]
fn annotation_output_and_synthesized_ownership() {
    let (mut l, c) = two_slots();
    l.ascent += 3.0;
    l.line_height += 3.0;
    l.baseline_y += 3.0;
    l.annotations.push(positioned_annotation());
    assert_eq!(l.validate(&c), Ok(()));
    let mut bad = l.clone();
    bad.annotations[0].glyphs[0].byte_range = Some(TextRange::new(0, 1));
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::InvalidGlyph
        })
    );
    let mut bad = l.clone();
    bad.annotations[0].base_byte_range.end = 2;
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::BaseSpansSlots
        })
    );
    let mut bad = l.clone();
    bad.ascent -= 3.0;
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::DoesNotFitLineBox
        })
    );
    let mut bad = l.clone();
    bad.annotations[0].glyphs[0].x = f32::NAN;
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::InvalidGlyph
        })
    );
    let mut bad = l;
    bad.visual_runs[0].glyphs[0].is_synthesized = true;
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::InvalidGlyphSource { run: 0, glyph: 0 })
    );
}

#[test]
fn annotation_base_allows_glyph_less_source_gaps() {
    let mut l = line();
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 3;
    // The body spans the first glyph, the glyph-less gap and the last glyph.
    set_visual_width(&mut l, 80.0);
    l.ascent += 3.0;
    l.line_height += 3.0;
    l.baseline_y += 3.0;
    let mut first = glyph(0.0, 10.0, false);
    first.byte_range = Some(TextRange::new(0, 1));
    let mut last = glyph(70.0, 10.0, false);
    last.byte_range = Some(TextRange::new(2, 3));
    l.visual_runs = vec![run(first), run(last)];
    let mut ann = positioned_annotation();
    ann.base_byte_range = TextRange::new(0, 3);
    ann.glyphs[0] = PositionedGlyph {
        byte_range: None,
        is_synthesized: true,
        ..glyph(25.0, 50.0, false)
    };
    l.annotations.push(ann);
    assert_eq!(l.validate(&ctx()), Ok(()));

    // A partially intersecting body glyph is still an ownership error even
    // when the base envelope would contain its painted extent.
    let mut bad = l.clone();
    bad.annotations[0].base_byte_range = TextRange::new(0, 2);
    bad.visual_runs[1].glyphs[0].byte_range = Some(TextRange::new(1, 3));
    assert_eq!(
        bad.validate(&ctx()),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::GlyphStraddlesBase
        })
    );

    // A base with no intersecting body glyph is not a visible base.
    let mut bad = l;
    bad.annotations[0].base_byte_range = TextRange::new(1, 2);
    assert_eq!(
        bad.validate(&ctx()),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::NoBaseGlyphs
        })
    );
}

#[test]
fn ruby_rejects_glue_and_cluster_internal_edges() {
    let mut a = ast("a b");
    rebuild_items(&mut a, |b| {
        vec![
            boxed(b, 0, 1, false),
            WordNode::Glue(GlueNode {
                original_bidi_class: BidiClass::Ws,
                bidi_level: 0,
                is_collapsible: true,
                byte_range: TextRange::new(1, 2),
                kind: GlueKind::WordSpace,
                natural_width: 1.0,
                max_stretch: 0.0,
                max_shrink: 0.0,
            }),
            boxed(b, 2, 3, false),
            end(3),
        ]
    });
    assert!(check(&a, "a b").is_ok());
    a.annotations = vec![annotation(TextRange::new(0, 3), AnnotationSide::Over)];
    assert_eq!(
        check(&a, "a b"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
    let mut a = ast("ab");
    edit_text_box(&mut a, 0, |_, clusters| {
        clusters.truncate(1);
        clusters[0].byte_range.end = 2;
        clusters[0].atom_range.end = 2;
        clusters[0].natural_width = 2.0;
    });
    assert!(check(&a, "ab").is_ok());
    a.annotations = vec![annotation(TextRange::new(0, 1), AnnotationSide::Over)];
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
}

#[test]
fn emphasis_mark_is_one_atom_even_when_it_is_two_chars() {
    let mut a = ast("ab");
    a.annotations = vec![annotation(TextRange::new(0, 2), AnnotationSide::Over)];
    let ann = &mut a.annotations[0];
    ann.kind = AnnotationKind::Emphasis;
    ann.annotation_text = "\u{FE45}\u{FE00}".into();
    ann.run.byte_range = TextRange::new(0, 6);
    ann.atoms[0].byte_range = TextRange::new(0, 6);
    ann.clusters[0].byte_range = TextRange::new(0, 6);
    assert!(check(&a, "ab").is_ok());
}

#[test]
fn annotation_atom_script_must_match_its_run_and_be_resolved() {
    let mut a = ast("ab");
    a.annotations = vec![annotation(TextRange::new(0, 2), AnnotationSide::Over)];
    a.annotations[0].atoms[0].script = ScriptTag::GREEK;
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
    let mut a = ast("ab");
    a.annotations = vec![annotation(TextRange::new(0, 2), AnnotationSide::Over)];
    a.annotations[0].run.script = ScriptTag::COMMON;
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
}

/// A line of two bases, "a" over `[0, 100]` and "b" over `[150, 250]`, with
/// room above and below for annotations.
fn annotated_line() -> (PositionedLine, LineContext) {
    let (mut l, c) = two_slots();
    l.ascent += 3.0;
    l.descent += 3.0;
    l.line_height += 6.0;
    l.baseline_y += 3.0;
    let first = positioned_annotation();
    let mut second = positioned_annotation();
    second.annotation_id = 2;
    second.base_byte_range = TextRange::new(1, 2);
    second.glyphs[0].x = 175.0;
    l.annotations = vec![first, second];
    (l, c)
}

#[test]
fn annotations_need_glyphs_unique_ids_and_disjoint_bases_per_side() {
    let (l, c) = annotated_line();
    assert_eq!(l.validate(&c), Ok(()));

    let mut empty = l.clone();
    empty.annotations[1].glyphs.clear();
    assert_eq!(
        empty.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 1,
            reason: AnnotationErrorReason::NoGlyphs
        })
    );

    let mut repeated = l.clone();
    repeated.annotations[1].annotation_id = 1;
    assert_eq!(
        repeated.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 1,
            reason: AnnotationErrorReason::DuplicateId
        })
    );

    let mut overlapping = l.clone();
    overlapping.annotations[1].base_byte_range = TextRange::new(0, 2);
    assert_eq!(
        overlapping.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 1,
            reason: AnnotationErrorReason::OverlapsOnSide
        })
    );

    // The other side of the same base is a different annotation slot.
    let mut other_side = l;
    other_side.annotations[1].base_byte_range = TextRange::new(0, 1);
    other_side.annotations[1].side = AnnotationSide::Under;
    other_side.annotations[1].glyphs[0].x = 25.0;
    other_side.annotations[1].baseline_offset = -2.0;
    assert_eq!(other_side.validate(&c), Ok(()));
}

#[test]
fn an_annotation_id_appears_once_per_paragraph() {
    let annotated = |range: (usize, usize), id: u64, baseline_y: f32| {
        let mut l = line();
        l.byte_range = TextRange::new(range.0, range.1);
        l.visible_byte_end = range.1;
        l.ascent += 3.0;
        l.line_height += 3.0;
        l.baseline_y = baseline_y;
        let mut g = glyph(0.0, 10.0, false);
        g.byte_range = Some(l.byte_range);
        l.visual_runs = vec![run(g)];
        set_visual_width(&mut l, 10.0);
        let mut ann = positioned_annotation();
        ann.annotation_id = id;
        ann.base_byte_range = l.byte_range;
        ann.glyphs[0].x = 0.0;
        ann.glyphs[0].advance = 10.0;
        l.annotations = vec![ann];
        l
    };
    let mut p = paragraph(
        &ctx(),
        vec![annotated((0, 1), 1, 11.0), annotated((1, 2), 2, 24.0)],
    );
    p.total_block_size = 26.0;
    assert_eq!(p.validate("ab"), Ok(()));
    p.lines[1].annotations[0].annotation_id = 1;
    assert_eq!(
        p.validate("ab"),
        Err((
            1,
            LineInvariantError::InvalidParagraph {
                reason: ParagraphErrorReason::DuplicateAnnotation
            }
        ))
    );
}

/// One line "a" with a single annotation on `side`, sized for the fixture strut
/// (ascent 8, descent 2) plus the annotation's block extent of 3 on its side.
fn single_annotation_line(side: AnnotationSide) -> (PositionedLine, LineContext) {
    let mut l = line();
    let mut r = run(glyph(0.0, 100.0, false));
    r.byte_range = l.byte_range;
    l.visual_runs = vec![r];
    let mut ann = positioned_annotation();
    ann.side = side;
    ann.glyphs[0].x = 25.0;
    if side == AnnotationSide::Over {
        l.ascent += 3.0;
        l.baseline_y += 3.0;
    } else {
        ann.baseline_offset = -2.0;
        l.descent += 3.0;
    }
    l.line_height += 3.0;
    l.annotations = vec![ann];
    (l, ctx())
}

fn fixed(c: LineContext) -> LineContext {
    LineContext {
        line_height: LineHeight::Fixed(1.0),
        ..c
    }
}

#[test]
fn over_room_is_measured_from_the_base_not_the_strut() {
    let (l, c) = single_annotation_line(AnnotationSide::Over);
    assert_eq!(l.validate(&c), Ok(()));
    // A tall base needs more ascent than strut ascent + block extent.
    let mut tall = l.clone();
    tall.annotations[0].base_over = 12.0;
    tall.annotations[0].baseline_offset = 12.0;
    assert_eq!(
        tall.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::DoesNotFitLineBox
        })
    );
    // With enough ascent it passes.
    tall.ascent = 15.0;
    tall.baseline_y = 15.0;
    tall.line_height = 17.0;
    assert_eq!(tall.validate(&c), Ok(()));
}

#[test]
fn under_room_is_measured_from_the_base_not_the_strut() {
    let (l, c) = single_annotation_line(AnnotationSide::Under);
    assert_eq!(l.validate(&c), Ok(()));
    let mut tall = l.clone();
    tall.annotations[0].base_under = 6.0;
    tall.annotations[0].baseline_offset = -6.0;
    assert_eq!(
        tall.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::DoesNotFitLineBox
        })
    );
    tall.descent = 9.0;
    tall.line_height = 17.0;
    assert_eq!(tall.validate(&c), Ok(()));
}

#[test]
fn annotation_sits_beyond_its_base() {
    let (l, c) = single_annotation_line(AnnotationSide::Over);
    let mut low = l.clone();
    low.annotations[0].baseline_offset = 7.0;
    assert_eq!(
        low.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::InsideBase
        })
    );
    let (l, c) = single_annotation_line(AnnotationSide::Under);
    let mut low = l.clone();
    low.annotations[0].baseline_offset = -1.0;
    assert_eq!(
        low.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::InsideBase
        })
    );
}

#[test]
fn base_extents_are_finite_and_nonnegative() {
    let (l, c) = single_annotation_line(AnnotationSide::Over);
    for bad in [f32::NAN, f32::INFINITY, -1.0] {
        let mut over = l.clone();
        over.annotations[0].base_over = bad;
        assert_eq!(
            over.validate(&c),
            Err(LineInvariantError::InvalidAnnotation {
                index: 0,
                reason: AnnotationErrorReason::InvalidMetrics
            })
        );
        let mut under = l.clone();
        under.annotations[0].base_under = bad;
        assert_eq!(
            under.validate(&c),
            Err(LineInvariantError::InvalidAnnotation {
                index: 0,
                reason: AnnotationErrorReason::InvalidMetrics
            })
        );
    }
}

#[test]
fn fixed_line_boxes_do_not_grow_for_annotations() {
    let (mut l, c) = single_annotation_line(AnnotationSide::Over);
    let c = fixed(c);
    // The ruby overflows into the inter-line space: metrics equal the strut's.
    l.ascent = 8.0;
    l.baseline_y = 8.0;
    l.line_height = 10.0;
    l.annotations[0].base_over = 8.0;
    assert!(l.annotations[0].base_over + l.annotations[0].block_extent > l.ascent);
    assert_eq!(l.validate(&c), Ok(()));
    // Growing the ascent to fit the ruby is rejected.
    let mut grown = l.clone();
    grown.ascent = 11.0;
    grown.baseline_y = 11.0;
    assert_eq!(
        grown.validate(&c),
        Err(LineInvariantError::LineMetricsMismatch {
            expected_ascent: 8.0,
            expected_descent: 2.0,
            actual_ascent: 11.0,
            actual_descent: 2.0,
        })
    );
    // Placement rules still apply.
    let mut low = l;
    low.annotations[0].baseline_offset = 7.0;
    assert_eq!(
        low.validate(&c),
        Err(LineInvariantError::InvalidAnnotation {
            index: 0,
            reason: AnnotationErrorReason::InsideBase
        })
    );
}
