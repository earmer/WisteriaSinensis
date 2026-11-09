//! Writing modes, glyph orientation, text-combine-upright and vertical bounds.

use crate::common::*;
use mutil_engine::*;

#[test]
fn orientation_and_tcy_matrix() {
    for (mode, orientation, level, valid) in [
        (
            WritingMode::HorizontalTb,
            GlyphOrientation::Horizontal,
            0,
            true,
        ),
        (
            WritingMode::HorizontalTb,
            GlyphOrientation::VerticalUpright,
            0,
            false,
        ),
        (
            WritingMode::VerticalRl,
            GlyphOrientation::Horizontal,
            0,
            false,
        ),
        (
            WritingMode::VerticalLr,
            GlyphOrientation::VerticalUpright,
            0,
            true,
        ),
        (
            WritingMode::VerticalRl,
            GlyphOrientation::VerticalUpright,
            1,
            false,
        ),
        (
            WritingMode::VerticalLr,
            GlyphOrientation::HorizontalInVertical,
            1,
            false,
        ),
        (
            WritingMode::VerticalRl,
            GlyphOrientation::VerticalSideways,
            1,
            true,
        ),
    ] {
        let mut a = ast("a");
        orient(&mut a, mode, orientation, level);
        assert_eq!(
            check(&a, "a").is_ok(),
            valid,
            "{mode:?} {orientation:?} {level}"
        );
    }
    let mut a = ast("ab");
    orient(
        &mut a,
        WritingMode::VerticalRl,
        GlyphOrientation::HorizontalInVertical,
        0,
    );
    assert_eq!(check(&a, "ab"), Err(LayoutInputError::OrientationMismatch));
    a.items = vec![
        boxed(0, 1, false),
        WordNode::Discretionary(DiscretionaryNode::direct(1)),
        boxed(1, 2, false),
        end(2),
    ];
    orient(
        &mut a,
        WritingMode::VerticalRl,
        GlyphOrientation::HorizontalInVertical,
        0,
    );
    assert_eq!(check(&a, "ab"), Err(LayoutInputError::OrientationMismatch));
    a.items.remove(1);
    assert!(check(&a, "ab").is_ok());
    if let WordNode::Box(BoxNode::Text { atoms, .. }) = &mut a.items[0] {
        atoms[0].orientation = GlyphOrientation::VerticalSideways;
    }
    assert_eq!(check(&a, "ab"), Err(LayoutInputError::OrientationMismatch));
}

#[test]
fn writing_mode_roundtrips_and_over_sides() {
    let point = Point { x: 13.0, y: 27.0 };
    let rect = Rect::new(3.0, 7.0, 11.0, 19.0);
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        assert_eq!(
            mode.physical_to_flow_point(mode.flow_to_physical_point(point, 100.0), 100.0),
            point
        );
        assert_eq!(
            mode.physical_to_flow_rect(mode.flow_to_physical_rect(rect, 100.0), 100.0),
            rect
        );
        let baseline = mode.flow_to_physical_point(point, 100.0);
        let over = mode.flow_to_physical_point(
            Point {
                y: point.y + mode.over_to_block(5.0),
                ..point
            },
            100.0,
        );
        if mode.is_vertical() {
            assert_eq!(over.x, baseline.x + 5.0);
        } else {
            assert_eq!(over.y, baseline.y - 5.0);
        }
        let (start, end) = mode.line_bounds(20.0, 8.0, 2.0);
        assert_eq!(
            (start, end),
            if mode == WritingMode::VerticalLr {
                (18.0, 28.0)
            } else {
                (12.0, 22.0)
            }
        );
    }
    use FlowMotion::*;
    assert_eq!(
        Motion::Left.flow_direction(WritingMode::VerticalRl),
        Some(BlockNext)
    );
    assert_eq!(
        Motion::Left.flow_direction(WritingMode::VerticalLr),
        Some(BlockPrev)
    );
    assert_eq!(
        Motion::Down.flow_direction(WritingMode::VerticalLr),
        Some(InlineNext)
    );
    assert_eq!(
        Motion::Previous.flow_direction(WritingMode::VerticalLr),
        None
    );
}

#[test]
fn vertical_lr_paragraph_uses_line_under_for_block_start() {
    let mut l = line();
    l.baseline_y = 2.0;
    let mut r = run(glyph(0.0, 100.0, false));
    r.orientation = GlyphOrientation::VerticalUpright;
    l.visual_runs = vec![r];
    let mut p = PositionedParagraph::empty(Direction::Ltr);
    p.writing_mode = WritingMode::VerticalLr;
    p.total_block_size = 10.0;
    p.lines = vec![l];
    assert_eq!(p.validate("a", &LayoutOptions::default(), &ctx()), Ok(()));
    p.lines[0].baseline_y = 1.0;
    assert!(p.validate("a", &LayoutOptions::default(), &ctx()).is_err());
}

#[test]
fn vertical_lr_under_annotation_moves_first_baseline() {
    let mut l = line();
    l.baseline_y = 3.0;
    l.descent = 3.0;
    l.line_height = 11.0;
    let mut r = run(glyph(0.0, 100.0, false));
    r.orientation = GlyphOrientation::VerticalUpright;
    l.visual_runs = vec![r];
    let mut ann = positioned_annotation();
    ann.side = AnnotationSide::Under;
    ann.orientation = GlyphOrientation::VerticalUpright;
    ann.block_extent = 1.0;
    ann.baseline_offset = -2.0;
    ann.glyphs[0].x = 0.0;
    ann.glyphs[0].advance = 10.0;
    l.annotations.push(ann);
    let mut p = PositionedParagraph::empty(Direction::Ltr);
    p.writing_mode = WritingMode::VerticalLr;
    p.total_block_size = 11.0;
    p.lines = vec![l];
    assert_eq!(p.validate("a", &LayoutOptions::default(), &ctx()), Ok(()));
    // The line-under side is the block start in VerticalLr, so shrinking the
    // baseline to the strut descent alone pushes the line into negative block
    // space once the annotation has extended descent.
    p.lines[0].baseline_y = 2.0;
    assert!(p.validate("a", &LayoutOptions::default(), &ctx()).is_err());
}
