//! Rich inline spans end to end (spec §2.2, §2.4, §2.6, §6; Revision 2.1).
//!
//! A Notion-style mention chip that wraps across two lines with cloned padding
//! and is atomic for editing, and a flattened inline formula. Every width and
//! position is a hand-written fixture; no layout algorithm runs.

use crate::common::{ADV, shaping_run};
use mutil_engine::*;

// bytes: 0..2 "go" | 2..3 " " | 3..11 "@Ann-Lee" | 11..12 " " | 12..14 "ok"
const TEXT: &str = "go @Ann-Lee ok";
const PAD: f32 = 3.0;
const SPACE_W: f32 = 4.0;
const CHIP: TextRange = TextRange { start: 3, end: 11 };
const SPAN_ID: u64 = 77;

/// Atoms inside the chip carry the atomic-edit flag.
fn atom(offset: usize) -> GraphemeAtom {
    let flags = if CHIP.contains(offset) {
        AtomFlags::IN_ATOMIC_EDIT_SPAN
    } else {
        AtomFlags::EMPTY
    };
    GraphemeAtom {
        flags,
        ..crate::common::atom(offset)
    }
}

/// The cluster's flags are the aggregate of its atoms (spec §2.2).
fn cluster(base: usize, local: usize) -> ShapedCluster {
    ShapedCluster {
        flags: AtomFlags::for_cluster(&[atom(base + local)]),
        ..crate::common::cluster(TEXT, base, local)
    }
}

fn run() -> ShapingRun {
    shaping_run(TEXT, "en-GB")
}

fn text(start: usize, end: usize) -> WordNode {
    let atoms: Vec<_> = (start..end).map(atom).collect();
    let clusters: Vec<_> = (0..atoms.len()).map(|i| cluster(start, i)).collect();
    WordNode::Box(BoxNode::Text {
        run_index: 0,
        byte_range: TextRange::new(start, end),
        atoms,
        clusters,
        natural_width: (end - start) as f32 * ADV,
    })
}

fn space(at: usize) -> WordNode {
    WordNode::Glue(GlueNode {
        original_bidi_class: BidiClass::Ws,
        bidi_level: 0,
        is_collapsible: true,

        byte_range: TextRange::new(at, at + 1),
        kind: GlueKind::WordSpace,
        tier: GlueTier::Tier0,
        natural_width: SPACE_W,
        max_stretch: 2.0,
        max_shrink: 1.0,
    })
}

fn boundary(edge: SpanEdge, at: usize) -> WordNode {
    WordNode::Box(BoxNode::SpanBoundary {
        byte_range: TextRange::empty_at(at),
        span_id: SPAN_ID,
        edge,
        width: PAD,
        bidi_level: 0,
    })
}

fn sentinel(len: usize) -> WordNode {
    WordNode::Discretionary(DiscretionaryNode::hard_break(
        TextRange::empty_at(len),
        true,
    ))
}

fn chip_ast() -> ParagraphAst {
    // Inside a `Clone` span every interior break carries +300, the padding on
    // both sides, and lists the span in `cloned_spans` (ruling 5): cloning is
    // orthogonal to the action, which stays `Direct` here.
    let interior_break = DiscretionaryNode {
        no_break_spacing: 0.0,
        byte_range: TextRange::empty_at(8),
        penalty: DiscretionaryNode::NATURAL_PENALTY + DiscretionaryNode::CLONED_SPAN_PENALTY,
        flagged: false,
        pre_break_width: PAD,
        post_break_width: PAD,
        no_break_width: 0.0,
        action: BreakAction::Direct,
        cloned_spans: smallvec::smallvec![SPAN_ID],
    };
    ParagraphAst {
        annotations: Vec::new(),
        bidi_spans: Vec::new(),
        writing_mode: mutil_engine::types::WritingMode::HorizontalTb,
        base_direction: BidiDirection::Ltr,
        strut_metrics: FontMetrics {
            vertical_ascent: 5.0,
            vertical_descent: 5.0,
            vertical_line_gap: 0.0,
            ascent: 12.0,
            descent: 4.0,
            ..Default::default()
        },
        strut_font_size: 16.0,
        runs: vec![run()],
        items: vec![
            text(0, 2),
            // A breakable space is [Glue, Discretionary] (ruling 1): the space
            // stays on the trailing edge of the line that breaks after it.
            space(2),
            WordNode::Discretionary(DiscretionaryNode::direct(3)),
            boundary(SpanEdge::Leading, 3),
            text(3, 8), // "@Ann-"
            WordNode::Discretionary(interior_break),
            text(8, 11), // "Lee"
            boundary(SpanEdge::Trailing, 11),
            space(11),
            WordNode::Discretionary(DiscretionaryNode::direct(12)),
            text(12, 14),
            sentinel(TEXT.len()),
        ],
        decoration_spans: vec![InlineDecorationSpan {
            span_id: SPAN_ID,
            byte_range: CHIP,
            start_padding: PAD,
            end_padding: PAD,
            break_mode: BoxDecorationBreak::Clone,
        }],
        atomic_spans: vec![AtomicEditSpan {
            span_id: SPAN_ID,
            byte_range: CHIP,
        }],
    }
}

fn pen_glyph(range: (usize, usize), x: f32) -> PositionedGlyph {
    PositionedGlyph {
        glyph_id: TEXT.as_bytes()[range.0] as u32,
        x,
        x_offset: 0.0,
        y_offset: 0.0,
        advance: ADV,
        byte_range: Some(TextRange::new(range.0, range.1)),
        is_synthesized: false,
        is_hanging: false,
    }
}

fn fragment(
    range: (usize, usize),
    x: f32,
    width: f32,
    first: bool,
    last: bool,
) -> PositionedSpanFragment {
    PositionedSpanFragment {
        span_id: SPAN_ID,
        byte_range: TextRange::new(range.0, range.1),
        x,
        width,
        ascent: 12.0,
        descent: 4.0,
        is_first_fragment: first,
        is_last_fragment: last,
        bidi_level: 0,
    }
}

fn line(byte_range: (usize, usize), visible_end: usize, y: f32, width: f32) -> PositionedLine {
    PositionedLine {
        slots: smallvec::SmallVec::new(),
        annotations: Vec::new(),
        is_ellipsized: false,
        resolved_slot_constraints: smallvec::SmallVec::new(),
        atomic_fragments: vec![],

        byte_range: TextRange::new(byte_range.0, byte_range.1),
        visible_byte_end: visible_end,
        baseline_y: y,
        ascent: 12.0,
        descent: 4.0,
        line_height: 16.0,
        visual_width: width,
        hanging_left: 0.0,
        hanging_right: 0.0,
        is_justified: false,
        visual_runs: vec![],
        visual_boxes: vec![],
        tabs: vec![],
        span_fragments: vec![],
        caret_stops: vec![],
        decorations: vec![],
    }
}

fn stop(x: f32, offset: usize, affinity: CaretAffinity, line: usize) -> CaretStop {
    CaretStop {
        x,
        offset,
        affinity,
        level: 0,
        line,
    }
}

#[test]
fn chip_inputs_are_accepted_and_atoms_carry_the_atomic_flag() {
    let ast = chip_ast();
    let constraints = LayoutConstraints {
        first_line_indent: 0.0,
        geometry_provider: None,
        hanging_indent: 0.0,
        wrap_width: 100.0,
        tab_stops: TabStops { step: 8.0 },
    };
    assert!(check_layout_inputs(&ast, TEXT, &constraints, &LayoutOptions::default()).is_ok());
    assert!(ast.items.iter().all(WordNode::is_self_contained));
    // Atoms inside the chip carry the flag, and so do their clusters.
    let WordNode::Box(BoxNode::Text {
        atoms, clusters, ..
    }) = &ast.items[4]
    else {
        unreachable!()
    };
    assert!(
        atoms
            .iter()
            .all(|a| a.flags.contains(AtomFlags::IN_ATOMIC_EDIT_SPAN))
    );
    assert!(
        clusters
            .iter()
            .all(|c| c.flags.contains(AtomFlags::IN_ATOMIC_EDIT_SPAN))
    );
    let WordNode::Box(BoxNode::Text { clusters, .. }) = &ast.items[0] else {
        unreachable!()
    };
    assert!(
        clusters
            .iter()
            .all(|c| !c.flags.contains(AtomFlags::IN_ATOMIC_EDIT_SPAN))
    );
    // The only break opportunities are discretionaries; glue never breaks.
    let breaks: Vec<_> = ast
        .items
        .iter()
        .enumerate()
        .filter(|(_, n)| n.is_break_opportunity())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(breaks, vec![2, 5, 9, 11]); // the last is the sentinel
    // Padding is cloned only on the break that is inside the chip.
    assert!(!ast.decoration_spans[0].forbids_cjk_autospace_at(5));
    assert!(ast.decoration_spans[0].forbids_cjk_autospace_at(CHIP.start));
}

#[test]
fn a_chip_wrapped_across_two_lines_clones_its_padding_and_validates() {
    let ast = chip_ast();
    // Line 0: "go" ␠ [pad "@Ann-" pad]; the cloned end padding closes the chip.
    //   g 0..10, o 10..20, space 20..24, leading pad 24..27, "@Ann-" 27..77,
    //   cloned trailing pad 77..80.
    let mut l0 = line((0, 8), 8, 12.0, 80.0);
    let mut glyphs0: Vec<_> = (0..2)
        .map(|i| pen_glyph((i, i + 1), i as f32 * ADV))
        .collect();
    glyphs0.extend((3..8).map(|i| pen_glyph((i, i + 1), 27.0 + (i - 3) as f32 * ADV)));
    l0.visual_runs = vec![VisualGlyphRun {
        orientation: mutil_engine::types::GlyphOrientation::Horizontal,
        font_size: 16.0,
        variations: Default::default(),

        font: FontRef::unscaled(FontId(1)),
        bidi_level: 0,
        byte_range: TextRange::new(0, 8),
        glyphs: glyphs0,
    }];
    l0.span_fragments = vec![fragment((3, 8), 24.0, 56.0, true, false)];

    // Line 1: cloned leading pad 0..3, "Lee" 3..33, trailing pad 33..36,
    //   space 36..40, "ok" 40..60.
    let mut l1 = line((8, 14), 14, 28.0, 60.0);
    let mut glyphs1: Vec<_> = (8..11)
        .map(|i| pen_glyph((i, i + 1), PAD + (i - 8) as f32 * ADV))
        .collect();
    glyphs1.extend((12..14).map(|i| pen_glyph((i, i + 1), 40.0 + (i - 12) as f32 * ADV)));
    l1.visual_runs = vec![VisualGlyphRun {
        orientation: mutil_engine::types::GlyphOrientation::Horizontal,
        font_size: 16.0,
        variations: Default::default(),

        font: FontRef::unscaled(FontId(1)),
        bidi_level: 0,
        byte_range: TextRange::new(8, 14),
        glyphs: glyphs1,
    }];
    l1.span_fragments = vec![fragment((8, 11), 0.0, 36.0, false, true)];

    // Atomic editing: no caret stop strictly inside the chip, even at the wrap
    // (offset 8). Only the logical boundaries 3 and 11 survive.
    l0.caret_stops = vec![
        stop(0.0, 0, CaretAffinity::Downstream, 0),
        stop(10.0, 1, CaretAffinity::Downstream, 0),
        stop(20.0, 2, CaretAffinity::Downstream, 0),
        stop(24.0, 3, CaretAffinity::Downstream, 0),
    ];
    l1.caret_stops = vec![
        stop(36.0, 11, CaretAffinity::Upstream, 1),
        stop(40.0, 12, CaretAffinity::Downstream, 1),
        stop(50.0, 13, CaretAffinity::Downstream, 1),
        stop(60.0, 14, CaretAffinity::Downstream, 1),
    ];
    let chip = &ast.atomic_spans[0];
    for s in l0.caret_stops.iter().chain(&l1.caret_stops) {
        assert!(
            !chip.is_interior(s.offset),
            "stop {} is inside the chip",
            s.offset
        );
    }

    let ctx = LineContext::from_options(
        &ast.strut_metrics,
        ast.strut_font_size,
        ast.base_direction,
        &LayoutOptions::default(),
        100.0,
    );
    let mut para = PositionedParagraph::empty(ast.base_direction);
    para.size = Size {
        width: 80.0,
        height: 32.0,
    };
    para.lines = vec![l0, l1];
    para.total_block_size = 32.0;
    assert_eq!(para.validate_geometry(&ctx), Ok(()));

    // Slice vs Clone is visible in the fragments: both pads exist on both lines
    // under Clone, so the first fragment is wider than its text by two pads.
    let [f0, f1] = [
        para.lines[0].span_fragments[0],
        para.lines[1].span_fragments[0],
    ];
    assert!(f0.is_first_fragment && !f0.is_last_fragment);
    assert!(!f1.is_first_fragment && f1.is_last_fragment);
    assert_eq!(f0.width, 5.0 * ADV + 2.0 * PAD);
    assert_eq!(f1.width, 3.0 * ADV + 2.0 * PAD);

    // A fragment narrower than the text it owns is caught.
    let mut narrow = para.clone();
    narrow.lines[1].span_fragments[0].width = 20.0;
    assert_eq!(
        narrow.validate_geometry(&ctx),
        Err((
            1,
            LineInvariantError::GlyphOutsideSpanFragment {
                span_id: SPAN_ID,
                run: 0,
                glyph: 1 // "e" spans 13..23, past the 20-wide fragment
            }
        ))
    );
}

#[test]
fn a_flattened_inline_formula_is_rigid_boxes_between_penalised_breaks() {
    // $a + b = c$ -> a + | b = | c: top-level relations 500, binary operators 700;
    // everything inside braces, fractions or radicals would stay one object.
    let obj = |id: u64, at: usize| {
        WordNode::Box(BoxNode::InlineObject {
            baseline_align: mutil_engine::types::BaselineAlignment::Auto,
            byte_range: TextRange::new(at, at + 1),
            box_id: id,
            width: 8.0,
            ascent: 9.0,
            descent: 2.0,
            overflow_policy: BoxOverflowPolicy::AllowOverflow,
        })
    };
    let op_break = |offset: usize, penalty: i32| {
        WordNode::Discretionary(DiscretionaryNode {
            no_break_spacing: 0.0,
            byte_range: TextRange::empty_at(offset),
            penalty,
            flagged: false,
            pre_break_width: 0.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::MathOperator {
                repeat_glyph_id: None,
            },
            cloned_spans: Default::default(),
        })
    };
    // "a+b=c": a, + | b, = | c
    let formula_run = ShapingRun {
        byte_range: TextRange::new(0, 5),
        ..run()
    };
    let ast = ParagraphAst {
        runs: vec![formula_run],
        decoration_spans: vec![],
        atomic_spans: vec![],
        items: vec![
            obj(1, 0),
            obj(2, 1),
            op_break(2, DiscretionaryNode::MATH_BINARY_OPERATOR_PENALTY),
            obj(3, 2),
            obj(4, 3),
            op_break(4, DiscretionaryNode::MATH_RELATION_PENALTY),
            obj(5, 4),
            sentinel(5),
        ],
        ..chip_ast()
    };
    let constraints = LayoutConstraints {
        first_line_indent: 0.0,
        geometry_provider: None,
        hanging_indent: 0.0,
        wrap_width: 100.0,
        tab_stops: TabStops { step: 8.0 },
    };
    assert_eq!(
        check_layout_inputs(&ast, "a+b=c", &constraints, &LayoutOptions::default()),
        Ok(LineContext::from_options(
            &ast.strut_metrics,
            ast.strut_font_size,
            ast.base_direction,
            &LayoutOptions::default(),
            100.0
        ))
    );
}
