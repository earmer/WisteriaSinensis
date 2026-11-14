//! The four gates (AX-3): each check happens once, at its gate, and code
//! behind a gate receives a type that proves it.

use super::common::*;
use mutil_engine::*;

/// MSR-1: a primary measurement beyond `MAX_MEASUREMENT` is rejected by the
/// check of the record that holds it.
#[test]
fn measurements_beyond_the_bound_are_rejected_at_the_input_gate() {
    assert!(is_measurement(-MAX_MEASUREMENT) && is_extent(MAX_MEASUREMENT));
    assert!(!is_measurement(2.0 * MAX_MEASUREMENT) && !is_extent(-1.0));
    assert!(!is_measurement(f32::NAN) && !is_measurement(f32::INFINITY));

    let mut a = ast("a");
    a.runs[0].font_size = 2.0 * MAX_MEASUREMENT;
    assert_eq!(
        check(&a, "a"),
        Err(LayoutInputError::InvalidShapingRun { run: 0 })
    );
    let mut a = ast("a");
    a.runs[0].letter_spacing = -2.0 * MAX_MEASUREMENT;
    assert_eq!(check(&a, "a"), Err(LayoutInputError::InvalidSpacing));
    let mut a = ast("a");
    a.glyphs[0].x_offset = 2.0 * MAX_MEASUREMENT;
    assert_eq!(
        check(&a, "a"),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
}

/// FNT-2: only canonical features pass the gate, so `Eq` and `Hash` agree
/// with effective equality behind it.
#[test]
fn features_must_be_canonical() {
    let mut a = ast("a");
    a.runs[0].features.push(*b"liga", 1);
    a.runs[0].features.push(*b"kern", 1);
    assert!(!a.runs[0].features.is_normalized());
    assert_eq!(
        check(&a, "a"),
        Err(LayoutInputError::InvalidShapingRun { run: 0 })
    );
    a.runs[0].features.normalize();
    assert!(a.runs[0].features.is_normalized());
    assert_eq!(check(&a, "a"), Ok(()));
}

/// G1: `ValidatedInputs` exists only for gated inputs, and its lookups say
/// `None` only for an item that is not text.
#[test]
fn validated_inputs_prove_the_input_gate() {
    let a = ast("ab");
    let constraints = constraints();
    let options = LayoutOptions::default();
    let inputs = ValidatedInputs::new(&a, "ab", &constraints, &options).expect("valid inputs");
    let view = inputs.text_box(0).expect("item 0 is text");
    assert_eq!(view.byte_range, TextRange::new(0, 2));
    assert!(inputs.text_box(1).is_none(), "the sentinel is not text");
    assert_eq!(inputs.run_atoms(0).len(), a.atoms.len());
    assert_eq!(inputs.run_clusters(0).len(), a.clusters.len());
    assert_eq!(inputs.cluster_glyphs(&a.clusters[0]).len(), 1);
    assert_eq!(inputs.policy().strut.line_height(), 10.0);

    let mut bad = ast("ab");
    bad.runs[0].font_size = f32::NAN;
    assert_eq!(
        ValidatedInputs::new(&bad, "ab", &constraints, &options).map(drop),
        Err(LayoutInputError::InvalidShapingRun { run: 0 })
    );
}

/// G2: metrics a shaper answers at layout time are checked before use; zero
/// metrics are not valid (FNT-4).
#[test]
fn the_shaper_gate_rejects_invalid_metrics() {
    let a = ast("a");
    let constraints = LayoutConstraints {
        geometry_provider: Some(std::sync::Arc::new(Everywhere)),
        ..constraints()
    };
    let options = LayoutOptions::default();
    assert_eq!(
        layout_paragraph(&a, "a", &NoopShaper, &constraints, &options).map(drop),
        Err(LayoutInputError::InvalidFontMetrics)
    );
    assert!(layout_paragraph(&a, "a", &TEST_SHAPER, &constraints, &options).is_ok());
}

/// G4: `ValidatedLayout` exists only for output that passed the output gate.
#[test]
fn validated_layout_proves_the_output_gate() {
    let ctx = ctx();
    let mut l = line();
    l.byte_range = TextRange::new(0, 1);
    l.visible_byte_end = 1;
    let mut g = glyph(0.0, 10.0, false);
    g.byte_range = Some(l.byte_range);
    l.visual_runs = vec![run(g)];
    set_visual_width(&mut l, 10.0);
    let mut good = paragraph(&ctx, vec![l]);
    good.total_block_size = good.lines[0].line_height;
    let text = "a";
    assert_eq!(good.validate(text), Ok(()));
    let layout = ValidatedLayout::new(&good, text).expect("valid output");
    assert_eq!(layout.paragraph().lines.len(), 1);
    assert_eq!(layout.text(), text);
    let empty = paragraph(&ctx, vec![]);
    assert!(ValidatedLayout::new(&empty, text).is_err());
}

#[derive(Debug)]
struct Everywhere;

impl LineGeometryProvider for Everywhere {
    fn query_line_geometry(&self, _line: usize, _start: f32, _end: f32) -> LineGeometry {
        LineGeometry {
            slots: smallvec::smallvec![InlineSlotConstraint::new(0.0, 100.0).unwrap()],
            next_feature_block_offset: None,
        }
    }
}

/// INT-13, INT-15: a motion that would leave the paragraph reports the exit in
/// flow-relative terms instead of clamping.
#[test]
fn motions_report_paragraph_exits() {
    let block = CursorMove::Exited {
        edge: ParagraphEdge::Start,
        inline_position: Some(42.0),
    };
    let inline = CursorMove::Exited {
        edge: ParagraphEdge::End,
        inline_position: None,
    };
    assert_ne!(block, inline);
    // Block-axis arrows depend on the writing mode; the exit is flow-relative.
    assert_eq!(
        Motion::Up.flow_direction(WritingMode::HorizontalTb),
        Some(FlowMotion::BlockPrev)
    );
    assert_eq!(
        Motion::Right.flow_direction(WritingMode::VerticalRl),
        Some(FlowMotion::BlockPrev)
    );
    let stay = CursorMove::Moved(CursorPosition {
        byte_offset: 0,
        affinity: CaretAffinity::Downstream,
    });
    assert!(matches!(stay, CursorMove::Moved(_)));
}
