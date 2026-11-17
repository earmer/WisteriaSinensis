//! The Phase 2 checks of the input and output gates: the level floor
//! (AST-13, `InvalidBidiLevel`) and screen order across visual runs (OUT-3,
//! `VisualRunsOutOfOrder`), with the messages of the new variants.

use super::common::*;
use mutil_engine::*;

#[test]
fn every_level_of_the_body_lies_at_or_above_the_base_level() {
    // An LTR text box in an RTL paragraph is below the floor.
    let mut a = ast("ab");
    a.base_direction = Direction::Rtl;
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidBidiLevel { item: 0 })
    );
    // At level 1 or 2 it is not.
    for level in [BidiLevel::RTL, BidiLevel::new(2).unwrap()] {
        orient(
            &mut a,
            WritingMode::HorizontalTb,
            GlyphOrientation::Horizontal,
            level,
        );
        assert_eq!(check(&a, "ab"), Ok(()), "{level:?}");
    }
    // An inline object and a glue below the floor.
    let text = "a\u{FFFC} b";
    let build = |object: BidiLevel, glue: BidiLevel| {
        let mut a = ast(text);
        a.base_direction = Direction::Rtl;
        rebuild_items(&mut a, |b| {
            vec![
                boxed(b, 0, 1),
                WordNode::Box(BoxNode::InlineObject {
                    bidi_level: object,
                    baseline_align: BaselineAlignment::Auto,
                    byte_range: TextRange::new(1, 4),
                    box_id: 1,
                    width: 1.0,
                    ascent: 1.0,
                    descent: 1.0,
                    overflow_policy: BoxOverflowPolicy::Clip,
                }),
                WordNode::Glue(GlueNode {
                    original_bidi_class: BidiClass::Ws,
                    bidi_level: glue,
                    is_collapsible: true,
                    byte_range: TextRange::new(4, 5),
                    kind: GlueKind::WordSpace,
                    natural_width: 2.0,
                    max_stretch: 1.0,
                    max_shrink: 0.5,
                }),
                boxed(b, 5, 6),
                end(6),
            ]
        });
        orient(
            &mut a,
            WritingMode::HorizontalTb,
            GlyphOrientation::Horizontal,
            BidiLevel::RTL,
        );
        a
    };
    let (ltr, rtl) = (BidiLevel::LTR, BidiLevel::RTL);
    assert_eq!(check(&build(rtl, rtl), text), Ok(()));
    assert_eq!(
        check(&build(ltr, rtl), text),
        Err(LayoutInputError::InvalidBidiLevel { item: 1 })
    );
    assert_eq!(
        check(&build(rtl, ltr), text),
        Err(LayoutInputError::InvalidBidiLevel { item: 2 })
    );
}

fn two_run_line(left_run_x: f32, right_run_x: f32) -> PositionedLine {
    let mut l = line();
    l.byte_range = TextRange::new(0, 2);
    l.visible_byte_end = 2;
    let mut second = glyph(right_run_x, 50., false);
    second.byte_range = Some(TextRange::new(1, 2));
    let mut second_run = run(second);
    second_run.bidi_level = BidiLevel::RTL;
    l.visual_runs = vec![run(glyph(left_run_x, 50., false)), second_run];
    l
}

#[test]
fn visual_runs_are_in_screen_order() {
    assert_eq!(two_run_line(0., 50.).validate(&ctx()), Ok(()));
    assert_eq!(
        two_run_line(50., 0.).validate(&ctx()),
        Err(LineInvariantError::VisualRunsOutOfOrder { run: 1 })
    );
    // A paragraph reports the line it was found on.
    let mut p = paragraph(&ctx(), vec![two_run_line(50., 0.)]);
    p.total_block_size = 10.0;
    assert_eq!(
        p.validate_geometry(),
        Err((0, LineInvariantError::VisualRunsOutOfOrder { run: 1 }))
    );
}

#[test]
fn the_new_gate_variants_have_their_own_messages() {
    // The messages are not pinned by the specification; each names its
    // index, differs from the others and is an `Error`.
    let input = LayoutInputError::InvalidBidiLevel { item: 7 };
    let lines = [
        LineInvariantError::VisualRunsOutOfOrder { run: 7 },
        LineInvariantError::HangingGlyphNotOutermost { run: 7, glyph: 8 },
    ];
    let mut messages = vec![input.to_string()];
    let _: &dyn std::error::Error = &input;
    for error in &lines {
        messages.push(error.to_string());
        let _: &dyn std::error::Error = error;
    }
    for side in [HangSide::Left, HangSide::Right] {
        messages.push(LineInvariantError::TooManyHangingGlyphs { side }.to_string());
    }
    for m in &messages[..3] {
        assert!(m.contains('7'), "{m}");
    }
    let mut unique = messages.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), messages.len(), "{messages:?}");
    assert!(
        messages
            .iter()
            .all(|m| !m.is_empty()
                && m != &LayoutInputError::InvalidBidiSpan { span: 7 }.to_string())
    );
}
