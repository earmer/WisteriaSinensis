//! What the output gate G4 proves for the queries behind it (ADR-0004):
//! line boxes keep their height (`LineBoxCollapsed`) and their bottoms
//! ascend exactly (`LineBoxesOverlap`), caret stops are in exact visual
//! order and on character boundaries with no offset repeated at one
//! position (CRT-1, INT-20), and the recorded policy is one `resolve`
//! could produce (PAR-1). Also INT-13 in a clamped paragraph.

use crate::common::*;
use mutil_engine::*;

fn invalid(line: usize, reason: ParagraphErrorReason) -> Result<(), (usize, LineInvariantError)> {
    Err((line, LineInvariantError::InvalidParagraph { reason }))
}

fn stop(x: f32, offset: usize, affinity: CaretAffinity) -> CaretStop {
    CaretStop {
        x,
        offset,
        affinity,
        level: BidiLevel::LTR,
    }
}

/// Line `index` of 10-unit lines: bytes `start..end` of one-byte
/// characters, one glyph filling the 100-unit slot, and a caret stop per
/// byte boundary spread evenly over it (the last one upstream).
fn text_line(index: usize, start: usize, end: usize) -> PositionedLine {
    let mut l = line();
    l.byte_range = TextRange::new(start, end);
    l.visible_byte_end = end;
    l.baseline_y = 8. + 10. * index as f32;
    let mut g = glyph(0., 100.);
    g.byte_range = Some(l.byte_range);
    l.visual_runs = vec![run(g)];
    let n = (end - start) as f32;
    l.caret_stops = (start..=end)
        .map(|offset| {
            let affinity = if offset == end {
                CaretAffinity::Upstream
            } else {
                CaretAffinity::Downstream
            };
            stop(100. * (offset - start) as f32 / n, offset, affinity)
        })
        .collect();
    l
}

const TEXT: &str = "abcdef";

/// "abcdef" on three lines of two bytes each, with caret stops.
fn three_lines() -> PositionedParagraph {
    let lines = (0..3).map(|i| text_line(i, 2 * i, 2 * i + 2)).collect();
    let mut p = paragraph(&ctx(), lines);
    p.total_block_size = 30.;
    p
}

#[test]
fn the_fixture_passes_the_output_gate() {
    let p = three_lines();
    assert_eq!(p.validate(TEXT), Ok(()));
    assert!(ValidatedLayout::try_from_paragraph(&p, TEXT).is_ok());
}

#[test]
fn par2_a_line_box_far_from_the_origin_is_collapsed() {
    // At 1e8 the f32 step is 8 and at 1e9 it is 64: the box `baseline - 8 ..
    // baseline + 2` no longer spans the line's ascent + descent of 10.
    for baseline_y in [1e8_f32, 1e9] {
        let mut far = text_line(0, 0, 6);
        far.baseline_y = baseline_y;
        let mut p = paragraph(&ctx(), vec![far]);
        p.total_block_size = 2.0 * baseline_y;
        assert_eq!(
            p.validate_geometry(),
            invalid(0, ParagraphErrorReason::LineBoxCollapsed),
            "{baseline_y}"
        );
        assert_eq!(
            ValidatedLayout::try_from_paragraph(&p, TEXT).map(drop),
            invalid(0, ParagraphErrorReason::LineBoxCollapsed),
            "{baseline_y}"
        );
    }
    // Near the origin the same line is fine.
    let mut p = three_lines();
    for l in &mut p.lines {
        l.baseline_y += 1000.0;
    }
    p.total_block_size = 1030.;
    assert_eq!(p.validate(TEXT), Ok(()));
}

#[test]
fn par2_a_line_whose_bottom_is_above_the_previous_bottom_overlaps() {
    // Line 0's box is [0, 10]. Line 1 starts within the tolerance of 10 but
    // is so short that it also ends above 10.
    let mut short = three_lines();
    let l = &mut short.lines[1];
    l.ascent = 0.0004;
    l.descent = 0.0001;
    l.baseline_y = 9.9996;
    assert_eq!(
        short.validate_geometry(),
        invalid(1, ParagraphErrorReason::LineBoxesOverlap)
    );
    // Ending exactly at the previous bottom is ascending.
    let mut level = three_lines();
    let l = &mut level.lines[1];
    l.ascent = 0.0005;
    l.descent = 0.0;
    l.baseline_y = 10.0;
    assert_eq!(level.validate_geometry(), Ok(()));
}

#[test]
fn crt1_caret_stops_out_of_exact_visual_order_are_invalid_even_within_tolerance() {
    let mut p = three_lines();
    // Line 0's stops are at 0, 50 and 100. Equal positions are in order.
    p.lines[0].caret_stops[2].x = 50.0;
    assert_eq!(p.validate(TEXT), Ok(()));
    // A step back far below the geometry tolerance is still out of order.
    for behind in [50.0 - 1e-4, 50.0_f32.next_down()] {
        let mut p = three_lines();
        p.lines[0].caret_stops[2].x = behind;
        assert_eq!(
            p.validate(TEXT),
            Err((0, LineInvariantError::InvalidCaretStop { index: 2 })),
            "{behind}"
        );
        assert_eq!(
            ValidatedLayout::try_from_paragraph(&p, TEXT).map(drop),
            Err((0, LineInvariantError::InvalidCaretStop { index: 2 })),
            "{behind}"
        );
    }
}

#[test]
fn crt1_a_caret_stop_off_a_character_boundary_is_invalid() {
    // "é" is two bytes: offset 1 lies inside it.
    let text = "\u{e9}b";
    let mut l = line();
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 3;
    let mut e = glyph(0., 50.);
    e.byte_range = Some(TextRange::new(0, 2));
    let mut b = glyph(50., 50.);
    b.byte_range = Some(TextRange::new(2, 3));
    let mut r = run(e);
    r.byte_range = l.byte_range;
    r.glyphs.push(b);
    l.visual_runs = vec![r];
    l.caret_stops = vec![
        stop(0., 0, CaretAffinity::Downstream),
        stop(50., 2, CaretAffinity::Downstream),
        stop(100., 3, CaretAffinity::Upstream),
    ];
    let mut p = paragraph(&ctx(), vec![l]);
    p.total_block_size = 10.;
    assert_eq!(p.validate(text), Ok(()));
    p.lines[0].caret_stops[1].offset = 1;
    assert_eq!(
        p.validate(text),
        Err((0, LineInvariantError::InvalidCaretStop { index: 1 }))
    );
    assert!(ValidatedLayout::try_from_paragraph(&p, text).is_err());
}

/// G4 on one line of `TEXT` (one LTR glyph over all six bytes) with the
/// given caret stops: the line check, the paragraph check and the
/// `ValidatedLayout` gate agree.
fn check_stops(stops: Vec<CaretStop>, expected: Result<(), LineInvariantError>) {
    let mut l = text_line(0, 0, 6);
    l.caret_stops = stops;
    let mut p = paragraph(&ctx(), vec![l]);
    p.total_block_size = 10.;
    let stops = &p.lines[0].caret_stops;
    assert_eq!(p.lines[0].validate(&ctx()), expected, "{stops:?}");
    let at_line = expected.map_err(|e| (0, e));
    assert_eq!(p.validate(TEXT), at_line, "{stops:?}");
    assert_eq!(
        ValidatedLayout::try_from_paragraph(&p, TEXT).map(drop),
        at_line,
        "{stops:?}"
    );
}

fn duplicate(index: usize) -> Result<(), LineInvariantError> {
    Err(LineInvariantError::DuplicateCaretStop { index })
}

#[test]
fn crt1_two_stops_with_one_offset_and_affinity_at_one_x_are_a_duplicate() {
    use CaretAffinity::*;
    check_stops(
        vec![stop(0., 0, Downstream), stop(0., 0, Downstream)],
        duplicate(1),
    );
    check_stops(
        vec![
            stop(0., 0, Downstream),
            stop(50., 3, Downstream),
            stop(50., 3, Downstream),
            stop(100., 6, Upstream),
        ],
        duplicate(2),
    );
}

#[test]
fn crt1_a_repeated_offset_at_one_x_is_a_duplicate_whatever_its_affinity() {
    use CaretAffinity::*;
    check_stops(
        vec![
            stop(0., 0, Downstream),
            stop(50., 3, Upstream),
            stop(50., 3, Downstream),
            stop(100., 6, Upstream),
        ],
        duplicate(2),
    );
    check_stops(
        vec![
            stop(0., 0, Downstream),
            stop(50., 3, Downstream),
            stop(50., 3, Upstream),
            stop(100., 6, Upstream),
        ],
        duplicate(2),
    );
}

#[test]
fn crt1_a_repeat_after_another_offset_at_the_same_x_is_a_duplicate() {
    use CaretAffinity::*;
    check_stops(
        vec![
            stop(50., 2, Downstream),
            stop(50., 3, Downstream),
            stop(50., 2, Downstream),
        ],
        duplicate(2),
    );
    check_stops(
        vec![
            stop(0., 0, Downstream),
            stop(50., 2, Downstream),
            stop(50., 3, Downstream),
            stop(50., 4, Downstream),
            stop(50., 2, Upstream),
            stop(100., 6, Upstream),
        ],
        duplicate(4),
    );
}

#[test]
fn crt1_a_repeat_within_the_geometry_tolerance_is_at_the_same_position() {
    use CaretAffinity::*;
    // At 50 the tolerance is the absolute 1e-3 (OUT-2).
    for (second, expected) in [
        (50.0005_f32, duplicate(2)),
        (50.000_9, duplicate(2)),
        (50.01, Ok(())),
        (51.0, Ok(())),
    ] {
        check_stops(
            vec![
                stop(0., 0, Downstream),
                stop(50., 3, Upstream),
                stop(second, 3, Downstream),
                stop(100., 6, Upstream),
            ],
            expected,
        );
    }
}

#[test]
fn crt1_an_invalid_stop_reports_invalid_caret_stop_before_duplicate() {
    use CaretAffinity::*;
    // Within the tolerance of the stop before it, but a step back: out of
    // exact visual order first.
    check_stops(
        vec![
            stop(0., 0, Downstream),
            stop(50., 3, Upstream),
            stop(50.0_f32.next_down(), 3, Downstream),
            stop(100., 6, Upstream),
        ],
        Err(LineInvariantError::InvalidCaretStop { index: 2 }),
    );
    // A repeat at the same x whose level no entity at offset 3 has (CRT-2).
    let rtl = CaretStop {
        level: BidiLevel::RTL,
        ..stop(50., 3, Downstream)
    };
    check_stops(
        vec![
            stop(0., 0, Downstream),
            stop(50., 3, Upstream),
            rtl,
            stop(100., 6, Upstream),
        ],
        Err(LineInvariantError::InvalidCaretStop { index: 2 }),
    );
}

#[test]
fn crt3_one_offset_at_two_positions_is_a_split_caret_not_a_duplicate() {
    use CaretAffinity::*;
    check_stops(
        vec![
            stop(0., 0, Downstream),
            stop(40., 3, Upstream),
            stop(60., 3, Downstream),
            stop(100., 6, Upstream),
        ],
        Ok(()),
    );
}

#[test]
fn crt1_distinct_offsets_at_one_x_are_coincident_stops_not_duplicates() {
    use CaretAffinity::*;
    check_stops(
        vec![
            stop(0., 0, Downstream),
            stop(50., 2, Downstream),
            stop(50., 3, Downstream),
            stop(50., 4, Upstream),
            stop(100., 6, Upstream),
        ],
        Ok(()),
    );
}

#[test]
fn crt1_caret_stop_with_non_finite_span_from_first_stop_is_invalid() {
    use CaretAffinity::*;
    // Every `x` is finite and sorted, and index 1 has a finite span from
    // index 0 (`0 - -3e38 = 3e38`), but index 2 overflows `x - stops[0].x`.
    assert!((0.0_f32 - -3.0e38_f32).is_finite());
    assert!((3.0e38_f32 - -3.0e38_f32).is_infinite());
    check_stops(
        vec![stop(-3.0e38, 0, Downstream), stop(3.0e38, 6, Upstream)],
        Err(LineInvariantError::InvalidCaretStop { index: 1 }),
    );
    check_stops(
        vec![
            stop(-3.0e38, 0, Downstream),
            stop(0., 3, Downstream),
            stop(3.0e38, 6, Upstream),
        ],
        Err(LineInvariantError::InvalidCaretStop { index: 2 }),
    );

    // Control: a large but finite span (`1e38 - -1e38 = 2e38`) passes G4 and
    // keeps `range_rects` and `selection_geometry` finite and non-negative.
    assert!((1.0e38_f32 - -1.0e38_f32).is_finite());
    let mut l = text_line(0, 0, 6);
    l.caret_stops = vec![
        stop(-1.0e38, 0, Downstream),
        stop(0., 3, Downstream),
        stop(1.0e38, 6, Upstream),
    ];
    let mut p = paragraph(&ctx(), vec![l]);
    p.total_block_size = 10.;
    assert_eq!(p.lines[0].validate(&ctx()), Ok(()));
    assert_eq!(p.validate_geometry(), Ok(()));
    assert_eq!(p.validate(TEXT), Ok(()));
    let layout = ValidatedLayout::try_from_paragraph(&p, TEXT).expect("finite stop span is valid");
    let full = TextRange::new(0, TEXT.len());
    let rects = layout.range_rects(full);
    assert!(!rects.is_empty());
    for rect in rects {
        assert_rect_finite(rect, "range_rects");
    }
    for style in [
        BoxHeightStyle::Tight,
        BoxHeightStyle::Max,
        BoxHeightStyle::Strut,
    ] {
        let sel = layout.selection_geometry(&TEST_SHAPER, full, style);
        assert!(!sel.is_empty(), "{style:?}");
        for piece in sel {
            assert_rect_finite(piece.rect, &format!("selection_geometry({style:?})"));
        }
    }
}

#[test]
fn par1_a_strut_with_an_inverted_box_is_not_a_valid_policy() {
    let mut p = three_lines();
    // A positive line height from the gap alone, over a box of -2.
    p.policy.strut = StrutMetrics {
        ascent: 2.0,
        descent: -4.0,
        line_gap: 10.0,
    };
    assert!(!p.policy.is_valid());
    assert_eq!(
        p.validate_geometry(),
        invalid(0, ParagraphErrorReason::InvalidPolicy)
    );
}

#[test]
fn par1_a_fixed_line_height_has_no_line_gap() {
    let mut normal = policy(&ctx());
    normal.strut.line_gap = 1.0;
    assert!(normal.is_valid());
    let fixed = LayoutPolicySnapshot {
        line_height: LineHeight::Fixed(1.0),
        ..normal
    };
    assert!(!fixed.is_valid());
    let mut p = three_lines();
    p.policy = fixed;
    assert_eq!(
        p.validate_geometry(),
        invalid(0, ParagraphErrorReason::InvalidPolicy)
    );
}

#[test]
fn par1_every_resolved_policy_is_valid_even_with_negative_half_leading() {
    // The fixture strut is 8 + 2 at size 10: a small fixed multiple makes
    // the half-leading negative, so the resolved descent is negative.
    let a = ast("a");
    for multiplier in [0.1_f32, 0.001, 0.5, 1.0, 3.0] {
        let options = LayoutOptions {
            line_height: LineHeight::Fixed(multiplier),
            ..LayoutOptions::default()
        };
        let policy = LayoutPolicySnapshot::resolve(&a, &constraints(), &options)
            .unwrap_or_else(|e| panic!("{multiplier}: {e}"));
        assert!(policy.is_valid(), "{multiplier}: {policy:?}");
        assert_eq!(policy.strut.line_gap, 0.0, "{multiplier}");
        assert!(policy.strut.ascent + policy.strut.descent >= 0.0);
    }
}

/// "abcdef" clamped to its first two lines: bytes 4..6 are the hidden tail.
fn clamped() -> PositionedParagraph {
    let mut p = three_lines();
    p.lines.pop();
    p.total_block_size = 20.;
    p.truncated_range = Some(TextRange::new(4, 6));
    p.policy.clamp = Some(LineClampPolicy::max_lines(2));
    p.lines[1].is_ellipsized = true;
    p
}

#[test]
fn int13_logical_motions_never_enter_the_truncated_tail() {
    let p = clamped();
    let l = ValidatedLayout::try_from_paragraph(&p, TEXT).expect("the clamped fixture is valid");
    let tail = p.truncated_range.unwrap().start();
    for offset in 0..=tail {
        for affinity in [CaretAffinity::Downstream, CaretAffinity::Upstream] {
            let at = CursorPosition {
                byte_offset: offset,
                affinity,
            };
            for motion in [Motion::Next, Motion::WordNext] {
                match l.move_cursor(at, motion, None) {
                    CursorMove::Moved(to) => {
                        assert!(to.byte_offset <= tail, "{at:?} {motion:?} -> {to:?}");
                        assert_ne!(offset, tail, "{at:?} {motion:?} moved to {to:?}");
                    }
                    CursorMove::Exited { edge, .. } => {
                        assert_eq!(edge, ParagraphEdge::End, "{at:?} {motion:?}");
                    }
                }
            }
        }
    }
    // The rendered end is where they exit.
    let end = CursorPosition {
        byte_offset: tail,
        affinity: CaretAffinity::Upstream,
    };
    for motion in [Motion::Next, Motion::WordNext] {
        assert!(
            matches!(
                l.move_cursor(end, motion, None),
                CursorMove::Exited {
                    edge: ParagraphEdge::End,
                    ..
                }
            ),
            "{motion:?}"
        );
    }
    // Next walks the rendered offsets one by one.
    let mut at = CursorPosition {
        byte_offset: 0,
        affinity: CaretAffinity::Downstream,
    };
    let mut visited = vec![0];
    while let CursorMove::Moved(to) = l.move_cursor(at, Motion::Next, None) {
        visited.push(to.byte_offset);
        at = to;
        assert!(visited.len() < 10, "{visited:?}");
    }
    assert_eq!(visited, [0, 1, 2, 3, 4]);
}

/// A 10-wide inline box at `x` over `byte_range`, whose baseline is `y` below
/// the line's, `ascent` above it and `descent` below it.
fn inline_box(
    box_id: u64,
    byte_range: TextRange,
    x: f32,
    y: f32,
    ascent: f32,
    descent: f32,
) -> PositionedInlineBox {
    PositionedInlineBox {
        bidi_level: BidiLevel::LTR,
        baseline_align: BaselineAlignment::Auto,
        box_id,
        byte_range,
        x,
        y,
        width: 10.,
        ascent,
        descent,
        overflow_policy: BoxOverflowPolicy::Clip,
    }
}

/// One line of `TEXT` in `mode` carrying `boxes`.
fn boxed_line_in(mode: WritingMode, boxes: Vec<PositionedInlineBox>) -> PositionedParagraph {
    let mut l = text_line(0, 0, 6);
    if mode != WritingMode::HorizontalTb {
        for run in &mut l.visual_runs {
            run.orientation = GlyphOrientation::VerticalUpright;
        }
    }
    l.visual_boxes = boxes;
    let line_ctx = LineContext {
        writing_mode: mode,
        ..ctx()
    };
    let mut p = paragraph(&line_ctx, vec![l]);
    p.total_block_size = 10.;
    p
}

/// One line of `TEXT` carrying a 10-wide box over byte 0 whose baseline is
/// `y` below the line's, `ascent` above it and `descent` below it.
fn boxed_line(y: f32, ascent: f32, descent: f32) -> PositionedParagraph {
    boxed_line_in(
        WritingMode::HorizontalTb,
        vec![inline_box(1, TextRange::new(0, 1), 20., y, ascent, descent)],
    )
}

/// Asserts that `r` has finite coordinates and finite, non-negative extents.
fn assert_rect_finite(r: Rect, what: &str) {
    assert!(
        r.x.is_finite()
            && r.y.is_finite()
            && r.width.is_finite()
            && r.height.is_finite()
            && r.width >= 0.
            && r.height >= 0.,
        "non-finite or negative rect for {what}: {r:?}"
    );
}

/// Asserts that `Tight` selection over the line produces finite, non-negative
/// physical rectangles (INT-16).
fn assert_tight_selection_finite(layout: &ValidatedLayout<'_>) {
    for range in [TextRange::new(0, 1), TextRange::new(0, TEXT.len())] {
        let rects = layout.selection_geometry(&TEST_SHAPER, range, BoxHeightStyle::Tight);
        assert!(!rects.is_empty(), "range {range:?} produced no rects");
        for piece in rects {
            assert_rect_finite(piece.rect, &format!("Tight selection {range:?}"));
        }
    }
}

/// G4 on `boxed_line(y, ascent, descent)`.
fn check_box(y: f32, ascent: f32, descent: f32, expected: Result<(), LineInvariantError>) {
    check_boxed(boxed_line(y, ascent, descent), expected);
}

/// G4 on a one-line paragraph carrying boxes: the line check, the
/// paragraph checks and the `ValidatedLayout` gate agree, and every accepted
/// paragraph yields finite `Tight` selection rectangles.
fn check_boxed(p: PositionedParagraph, expected: Result<(), LineInvariantError>) {
    let l = &p.lines[0];
    let line_ctx = p.policy.line_context(p.base_direction, 0);
    let fields = (p.policy.writing_mode, l.baseline_y, &l.visual_boxes);
    assert_eq!(l.validate(&line_ctx), expected, "{fields:?}");
    let at_line = expected.map_err(|e| (0, e));
    assert_eq!(p.validate_geometry(), at_line, "{fields:?}");
    assert_eq!(p.validate(TEXT), at_line, "{fields:?}");
    let validated = ValidatedLayout::try_from_paragraph(&p, TEXT);
    assert_eq!(
        validated.as_ref().map(drop),
        at_line.as_ref().map(drop),
        "{fields:?}"
    );
    if let Ok(layout) = validated {
        assert_tight_selection_finite(&layout);
    }
}

#[test]
fn the_boxed_fixture_passes_the_output_gate() {
    check_box(0., 5., 1., Ok(()));
}

#[test]
fn int16_a_box_whose_extent_above_the_baseline_overflows_is_invalid() {
    // Every field is finite, but `ascent - y` (the box's top for Tight
    // selection) is 6e38: infinite in f32.
    assert!((3e38_f32 - -3e38_f32).is_infinite());
    check_box(
        -3e38,
        3e38,
        1.,
        Err(LineInvariantError::InvalidInlineBox { index: 0 }),
    );
}

#[test]
fn int16_a_box_whose_extent_below_the_baseline_overflows_is_invalid() {
    // The mirror case: `descent + y` (the box's bottom) is 6e38.
    assert!((3e38_f32 + 3e38_f32).is_infinite());
    check_box(
        3e38,
        5.,
        3e38,
        Err(LineInvariantError::InvalidInlineBox { index: 0 }),
    );
}

#[test]
fn int16_a_box_with_large_but_finite_extents_about_the_baseline_is_valid() {
    // `ascent - y` is 2e38 and `descent + y` is 1e38 + 1: both finite.
    check_box(-1e38, 1e38, 1., Ok(()));
    check_box(1e38, 5., 1e38, Ok(()));
}

/// A line in `mode` with its baseline moved to `FAR` and a line box
/// `FAR - 1e38 .. FAR` tall enough that f32 steps there do not collapse it
/// (PAR-2), ending the block. `FAR ± 1e38` is finite, so the line's own
/// metrics are (BOX-1).
fn far_boxed_line_in(mode: WritingMode, y: f32, ascent: f32, descent: f32) -> PositionedParagraph {
    let mut p = boxed_line_in(
        mode,
        vec![inline_box(1, TextRange::new(0, 1), 20., y, ascent, descent)],
    );
    let l = &mut p.lines[0];
    l.baseline_y = FAR;
    match mode {
        WritingMode::HorizontalTb | WritingMode::VerticalRl => {
            l.ascent = 1e38;
            l.descent = 0.;
        }
        WritingMode::VerticalLr => {
            l.ascent = 0.;
            l.descent = 1e38;
        }
    }
    l.line_height = 1e38;
    p.total_block_size = FAR;
    p
}

fn far_boxed_line(y: f32, ascent: f32, descent: f32) -> PositionedParagraph {
    far_boxed_line_in(WritingMode::HorizontalTb, y, ascent, descent)
}

const FAR: f32 = 2e38;

#[test]
fn int16_a_box_whose_bottom_edge_from_a_far_baseline_overflows_is_invalid() {
    // `descent + y` is 2e38 and `ascent - y` is -1e38: both finite. In
    // HorizontalTb, the bottom edge `baseline_y + (descent + y)` is 4e38:
    // infinite in f32.
    assert!((1e38_f32 + 1e38_f32).is_finite());
    assert!((FAR + 2e38).is_infinite());
    check_boxed(
        far_boxed_line(1e38, 5., 1e38),
        Err(LineInvariantError::InvalidInlineBox { index: 0 }),
    );
}

#[test]
fn int16_a_box_whose_unused_sign_overflows_in_horizontal_tb_is_valid() {
    // `above = ascent - y = 2e38` and `below = descent + y = -1e38`: both
    // finite. Although `baseline_y + above` would overflow (the VerticalLr
    // sign), HorizontalTb uses `(baseline_y - above, baseline_y + below)` =
    // `(0, 1e38)` with finite span `1e38`, so the box is valid and Tight
    // selection produces finite rectangles.
    let above = 1e38_f32 - -1e38_f32;
    let below = 1.0_f32 + -1e38_f32;
    assert!(above.is_finite() && below.is_finite());
    assert!((FAR + above).is_infinite());
    let (top, bottom) = WritingMode::HorizontalTb.line_bounds(FAR, above, below);
    assert!(top.is_finite() && bottom.is_finite() && (bottom - top).is_finite());
    check_boxed(far_boxed_line(-1e38, 1e38, 1.), Ok(()));
}

#[test]
fn int16_a_box_whose_top_extent_overflows_in_vertical_lr_is_invalid() {
    // Valid control: the VerticalLr far-baseline fixture passes on its own
    // and with a small box.
    let mut bare = far_boxed_line_in(WritingMode::VerticalLr, 0., 5., 1.);
    bare.lines[0].visual_boxes.clear();
    assert_eq!(bare.validate(TEXT), Ok(()));
    check_boxed(
        far_boxed_line_in(WritingMode::VerticalLr, 0., 5., 1.),
        Ok(()),
    );

    // The same box accepted in HorizontalTb overflows in VerticalLr because
    // `line_bounds` computes `(baseline_y - below, baseline_y + above)` and
    // `baseline_y + above = 4e38` is infinite.
    let above = 1e38_f32 - -1e38_f32;
    let below = 1.0_f32 + -1e38_f32;
    assert!(above.is_finite() && below.is_finite());
    assert!(
        WritingMode::VerticalLr
            .line_bounds(FAR, above, below)
            .1
            .is_infinite()
    );
    check_boxed(
        far_boxed_line_in(WritingMode::VerticalLr, -1e38, 1e38, 1.),
        Err(LineInvariantError::InvalidInlineBox { index: 0 }),
    );

    // Mirror: the box whose `baseline_y + below` overflows in HorizontalTb
    // stays finite in VerticalLr, where `below` is subtracted from `FAR`.
    check_boxed(
        far_boxed_line_in(WritingMode::VerticalLr, 1e38, 5., 1e38),
        Ok(()),
    );
}

#[test]
fn int16_a_box_whose_edges_from_a_far_baseline_stay_finite_is_valid() {
    // The far line on its own, then extents of 2e37 either way: every edge
    // `2e38 ± 2e37` is finite.
    let mut p = far_boxed_line(0., 5., 1.);
    p.lines[0].visual_boxes.clear();
    assert_eq!(p.validate(TEXT), Ok(()));
    assert!((FAR + 2e37).is_finite());
    check_boxed(far_boxed_line(1e37, 5., 1e37), Ok(()));
    check_boxed(far_boxed_line(-1e37, 1e37, 1.), Ok(()));
}

#[test]
fn int16_a_box_whose_block_span_overflows_with_finite_edges_is_invalid() {
    // `above = 2e38`, `below = 2e38`, `top = 8 - 2e38 = -2e38`, and
    // `bottom = 8 + 2e38 = 2e38` are each finite, yet `bottom - top = 4e38`
    // overflows f32.
    let (top, bottom) = WritingMode::HorizontalTb.line_bounds(8., 2e38, 2e38);
    assert!(top.is_finite() && bottom.is_finite());
    assert!((bottom - top).is_infinite());

    check_box(0., 1e38, 1e38, Ok(()));
    check_box(
        0.,
        2e38,
        2e38,
        Err(LineInvariantError::InvalidInlineBox { index: 0 }),
    );
    check_boxed(
        boxed_line_in(
            WritingMode::VerticalLr,
            vec![inline_box(1, TextRange::new(0, 1), 20., 0., 2e38, 2e38)],
        ),
        Err(LineInvariantError::InvalidInlineBox { index: 0 }),
    );
}

#[test]
fn int16_two_boxes_whose_union_block_span_overflows_are_invalid() {
    let high = inline_box(1, TextRange::new(0, 1), 20., 0., 2e38, 1.);
    let low = inline_box(2, TextRange::new(1, 2), 35., 0., 1., 2e38);

    // Each box alone has finite edges and a finite block span.
    check_boxed(boxed_line_in(WritingMode::HorizontalTb, vec![high]), Ok(()));
    check_boxed(boxed_line_in(WritingMode::HorizontalTb, vec![low]), Ok(()));

    // Together, their union reaches `-2e38 .. 2e38`, whose span overflows.
    let (union_top, union_bottom) = WritingMode::HorizontalTb.line_bounds(8., 2e38, 2e38);
    assert!(union_top.is_finite() && union_bottom.is_finite());
    assert!((union_bottom - union_top).is_infinite());
    check_boxed(
        boxed_line_in(WritingMode::HorizontalTb, vec![high, low]),
        Err(LineInvariantError::InvalidInlineBox { index: 1 }),
    );

    // Control: two boxes whose union span stays finite are valid.
    check_boxed(
        boxed_line_in(
            WritingMode::HorizontalTb,
            vec![
                inline_box(1, TextRange::new(0, 1), 20., 0., 1e38, 1.),
                inline_box(2, TextRange::new(1, 2), 35., 0., 1., 1e38),
            ],
        ),
        Ok(()),
    );
}

#[test]
fn int16_a_vertical_rl_box_whose_physical_coordinate_overflows_is_invalid() {
    const TOTAL_BLOCK: f32 = 3e38;
    let vertical_rl = |boxes| {
        let mut p = boxed_line_in(WritingMode::VerticalRl, boxes);
        p.total_block_size = TOTAL_BLOCK;
        p
    };

    // Valid controls: small and large-but-finite boxes in VerticalRl with
    // `total_block_size = 3e38` pass and yield finite Tight selection rects.
    check_boxed(
        vertical_rl(vec![inline_box(1, TextRange::new(0, 1), 20., 0., 5., 1.)]),
        Ok(()),
    );
    assert!((TOTAL_BLOCK - (8.0_f32 - 1e37_f32)).is_finite());
    check_boxed(
        vertical_rl(vec![inline_box(1, TextRange::new(0, 1), 20., 0., 1e37, 1.)]),
        Ok(()),
    );

    // Flow bounds `(-1e38, 9)` and span `1e38` are finite, so line-level
    // validation passes, but mapping `top = -1e38` to physical coordinates
    // computes `total_block_size - top = 4e38`, which overflows f32.
    let (top, bottom) = WritingMode::VerticalRl.line_bounds(8., 1e38, 1.);
    assert!(top.is_finite() && bottom.is_finite() && (bottom - top).is_finite());
    assert!((TOTAL_BLOCK - top).is_infinite());

    let p = vertical_rl(vec![inline_box(1, TextRange::new(0, 1), 20., 0., 1e38, 1.)]);
    let line_ctx = p.policy.line_context(p.base_direction, 0);
    assert_eq!(p.lines[0].validate(&line_ctx), Ok(()));
    let expected = Err((0, LineInvariantError::InvalidInlineBox { index: 0 }));
    assert_eq!(p.validate_geometry(), expected);
    assert_eq!(p.validate(TEXT), expected);
    assert_eq!(
        ValidatedLayout::try_from_paragraph(&p, TEXT).map(drop),
        expected
    );
}

#[test]
fn box1_vertical_rl_baseline_physical_overflow_is_non_finite_line_metrics() {
    const TOTAL_BLOCK: f32 = 2.0e38;
    let vertical_rl_line = |baseline_y: f32, ascent: f32, descent: f32, line_height: f32| {
        let mut p = boxed_line_in(WritingMode::VerticalRl, vec![]);
        let l = &mut p.lines[0];
        l.baseline_y = baseline_y;
        l.ascent = ascent;
        l.descent = descent;
        l.line_height = line_height;
        p.total_block_size = TOTAL_BLOCK;
        p
    };

    // Valid control: line box is `[0, 1e38]` inside `[0, 2e38]`, non-collapsed
    // (`ascent + descent == line_height == 1e38`), and `total_block_size -
    // baseline_y = 3e38` stays finite in VerticalRl physical coordinates.
    assert!((TOTAL_BLOCK - -1.0e38_f32).is_finite());
    let valid = vertical_rl_line(-1.0e38, -1.0e38, 2.0e38, 1.0e38);
    let valid_ctx = valid.policy.line_context(valid.base_direction, 0);
    assert_eq!(valid.lines[0].validate(&valid_ctx), Ok(()));
    assert_eq!(valid.validate_geometry(), Ok(()));
    assert_eq!(valid.validate(TEXT), Ok(()));
    let layout =
        ValidatedLayout::try_from_paragraph(&valid, TEXT).expect("finite VerticalRl control");
    let full = TextRange::new(0, TEXT.len());
    for style in [
        BoxHeightStyle::Tight,
        BoxHeightStyle::Max,
        BoxHeightStyle::Strut,
    ] {
        let rects = layout.selection_geometry(&TEST_SHAPER, full, style);
        assert!(!rects.is_empty(), "{style:?}");
        for piece in rects {
            assert_rect_finite(piece.rect, &format!("VerticalRl control {style:?}"));
        }
    }

    // Overflow case: the line box itself lies cleanly inside `[0, 2e38]` with
    // exact span `ascent + descent == line_height` (so line-level validation
    // passes and the line box does not collapse), yet `total_block_size -
    // baseline_y = 3.5e38` overflows f32 in VerticalRl physical coordinates.
    let (baseline_y, ascent, descent) = (-1.5e38_f32, -1.5e38_f32, 1.6e38_f32);
    let (top, bottom) = WritingMode::VerticalRl.line_bounds(baseline_y, ascent, descent);
    assert!(top == 0.0 && bottom > 0.0 && bottom <= TOTAL_BLOCK);
    assert_eq!(bottom - top, ascent + descent);
    assert!((TOTAL_BLOCK - baseline_y).is_infinite());

    let mut p = vertical_rl_line(baseline_y, ascent, descent, ascent + descent);
    // Shift the glyph off the far baseline so its own VerticalRl physical
    // coordinate (`total_block_size - (baseline_y - y_offset) = 2.5e38`) stays
    // finite, isolating the baseline/strut physical-mapping overflow.
    p.lines[0].visual_runs[0].glyphs[0].y_offset = -1.0e38;
    let line_ctx = p.policy.line_context(p.base_direction, 0);
    assert_eq!(p.lines[0].validate(&line_ctx), Ok(()));
    let expected = Err((0, LineInvariantError::NonFiniteLineMetrics));
    assert_eq!(p.validate_geometry(), expected);
    assert_eq!(p.validate(TEXT), expected);
    assert_eq!(
        ValidatedLayout::try_from_paragraph(&p, TEXT).map(drop),
        expected
    );
}
