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
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
    a.annotations.truncate(1);
    a.annotations[0].clusters.clear();
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan)
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
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
    let mut a = ast("ab");
    a.annotations = vec![annotation(TextRange::new(0, 2), AnnotationSide::Over)];
    a.annotations[0].kind = AnnotationKind::Emphasis;
    assert!(check(&a, "ab").is_ok());
    a.annotations[0].run.letter_spacing = f32::NAN;
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
    a.annotations[0] = annotation(TextRange::new(0, 2), AnnotationSide::Over);
    rebuild_items(&mut a, |b| {
        vec![
            boxed(b, 0, 1, false),
            WordNode::Tab {
                byte_range: TextRange::new(1, 2),
            },
            end(2),
        ]
    });
    assert!(check(&a, "ab").is_err());
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
    assert!(bad.validate(&c).is_err());
    let mut bad = l.clone();
    bad.annotations[0].base_byte_range.end = 2;
    assert!(bad.validate(&c).is_err());
    let mut bad = l.clone();
    bad.ascent -= 3.0;
    assert!(bad.validate(&c).is_err());
    let mut bad = l.clone();
    bad.annotations[0].glyphs[0].x = f32::NAN;
    assert!(bad.validate(&c).is_err());
    let mut bad = l;
    bad.visual_runs[0].glyphs[0].is_synthesized = true;
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::InvalidGlyphSource)
    );
}

#[test]
fn annotation_base_allows_glyph_less_source_gaps() {
    let mut l = line();
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 3;
    l.visual_width = 100.0;
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
        Err(LineInvariantError::InvalidAnnotation)
    );

    // A base with no intersecting body glyph is not a visible base.
    let mut bad = l;
    bad.annotations[0].base_byte_range = TextRange::new(1, 2);
    assert_eq!(
        bad.validate(&ctx()),
        Err(LineInvariantError::InvalidAnnotation)
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
                tier: GlueTier::Tier0,
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
        Err(LayoutInputError::InvalidAnnotationSpan)
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
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
}
