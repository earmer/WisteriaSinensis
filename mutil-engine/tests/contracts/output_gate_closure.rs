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
