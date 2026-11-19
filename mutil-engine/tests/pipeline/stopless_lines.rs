//! Lines without caret stops (CRT-1, INT-4): an `AtomicEditSpan` wrapped
//! over several lines leaves the lines wholly inside it stopless. G4 accepts
//! them, and every §6 query stays total on them (INT-10, INT-13, INT-20).
//! Also: a hit at a NaN coordinate is never `inside` (INT-20).

use crate::common::*;
use mutil_engine::*;

const MOTIONS: [Motion; 12] = [
    Motion::Left,
    Motion::Right,
    Motion::Previous,
    Motion::Next,
    Motion::WordLeft,
    Motion::WordRight,
    Motion::WordPrevious,
    Motion::WordNext,
    Motion::LineStart,
    Motion::LineEnd,
    Motion::Up,
    Motion::Down,
];

const AFFINITIES: [CaretAffinity; 2] = [CaretAffinity::Downstream, CaretAffinity::Upstream];

fn at(offset: usize, affinity: CaretAffinity) -> CursorPosition {
    CursorPosition {
        byte_offset: offset,
        affinity,
    }
}

/// `text` with one atomic span over `chip`, laid out `width` wide, through
/// the output gate.
#[track_caller]
fn lay_out_chip(text: &str, chip: TextRange, width: f32) -> (AtomicEditSpan, PositionedParagraph) {
    let base = style(10.0);
    let spans = [AtomicEditSpan {
        span_id: 1,
        byte_range: chip,
    }];
    let paragraph = ParagraphInput {
        atomic_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SHAPER)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    let p = layout_paragraph(
        &ast,
        text,
        &*SHAPER,
        &column(width),
        &LayoutOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&p, text) {
        panic!("{text:?} at {width}: line {line}: {error}\n{p:#?}");
    }
    (spans[0], p)
}

#[track_caller]
fn assert_rect(rect: Rect, what: &str) {
    assert!(
        [rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|v| v.is_finite()),
        "{what}: {rect:?}"
    );
    assert!(rect.width >= 0.0 && rect.height >= 0.0, "{what}: {rect:?}");
}

/// A caret position is a boundary of the text, never strictly inside the
/// chip (INT-2, INT-4).
#[track_caller]
fn assert_caret_position(text: &str, chip: &AtomicEditSpan, p: CursorPosition, what: &str) {
    assert!(
        p.byte_offset <= text.len() && text.is_char_boundary(p.byte_offset),
        "{what}: {p:?}"
    );
    assert!(!chip.is_interior(p.byte_offset), "{what}: {p:?}");
}

/// Every hit, caret, motion and selection query on `p`, from every offset
/// and at sampled points, neither panics nor answers non-finite geometry or
/// a position inside the chip.
fn assert_queries_total(text: &str, chip: &AtomicEditSpan, p: &PositionedParagraph) {
    let l = ValidatedLayout::try_from_paragraph(p, text).unwrap();
    let xs = [
        f32::NEG_INFINITY,
        -50.0,
        0.0,
        3.0,
        6.0,
        11.9,
        17.0,
        100.0,
        f32::INFINITY,
        f32::NAN,
    ];
    for (i, line) in p.lines.iter().enumerate() {
        for x in xs {
            let hit = l.hit_test_on_line(i, x);
            assert_caret_position(text, chip, hit.position, &format!("line {i} at {x}"));
            let ys = [line.baseline_y - line.ascent, line.baseline_y];
            for y in ys {
                let hit = l.hit_test(Point { x, y });
                assert_caret_position(text, chip, hit.position, &format!("({x}, {y})"));
            }
        }
    }
    for offset in 0..=text.len() + 2 {
        assert_rect(l.rect_for_offset(offset), &format!("rect at {offset}"));
        for affinity in AFFINITIES {
            let from = at(offset, affinity);
            let caret = l.caret_geometry(from);
            assert_rect(caret.primary_rect, &format!("{from:?}"));
            if let Some(secondary) = caret.secondary_rect {
                assert_rect(secondary, &format!("{from:?} secondary"));
            }
            for motion in MOTIONS {
                let what = format!("{from:?} {motion:?}");
                match l.move_cursor(from, motion, None) {
                    CursorMove::Moved(to) => assert_caret_position(text, chip, to, &what),
                    CursorMove::Exited {
                        inline_position, ..
                    } => assert!(inline_position.is_none_or(f32::is_finite), "{what}"),
                }
            }
        }
    }
    for start in 0..=text.len() {
        for end in start..=text.len() {
            let range = TextRange::new(start, end);
            for height in [
                BoxHeightStyle::Tight,
                BoxHeightStyle::Max,
                BoxHeightStyle::Strut,
            ] {
                for piece in l.selection_geometry(&*SHAPER, range, height) {
                    assert_rect(piece.rect, &format!("{range:?} {height:?}"));
                }
            }
            for rect in l.range_rects(range) {
                assert_rect(rect, &format!("{range:?}"));
            }
        }
    }
}

/// The caret positions of the chip's edges: start downstream, end upstream.
fn chip_edges(chip: &AtomicEditSpan) -> (CursorPosition, CursorPosition) {
    (
        at(chip.byte_range.start(), CaretAffinity::Downstream),
        at(chip.byte_range.end(), CaretAffinity::Upstream),
    )
}

/// Inline arrows cross the chip whole (INT-13), and a block arrow never
/// lands back on the offset it left.
fn assert_arrows_cross_whole(text: &str, chip: &AtomicEditSpan, p: &PositionedParagraph) {
    let l = ValidatedLayout::try_from_paragraph(p, text).unwrap();
    let (start, end) = chip_edges(chip);
    let landed = |m: CursorMove| match m {
        CursorMove::Moved(to) => Some(to.byte_offset),
        CursorMove::Exited { .. } => None,
    };
    assert_eq!(
        landed(l.move_cursor(start, Motion::Right, None)),
        Some(chip.byte_range.end()),
        "Right from the chip start"
    );
    assert_eq!(
        landed(l.move_cursor(end, Motion::Left, None)),
        Some(chip.byte_range.start()),
        "Left from the chip end"
    );
    for offset in 0..=text.len() {
        for affinity in AFFINITIES {
            let from = at(offset, affinity);
            for motion in [Motion::Up, Motion::Down] {
                if let CursorMove::Moved(to) = l.move_cursor(from, motion, None) {
                    assert_ne!(to.byte_offset, offset, "{from:?} {motion:?}");
                }
            }
        }
    }
}

#[test]
fn crt1_int4_a_word_chip_split_letter_by_letter_leaves_stopless_lines() {
    let text = "abcd";
    let chip = TextRange::new(0, 4);
    // Each glyph is 6 wide: one letter per line.
    let (span, p) = lay_out_chip(text, chip, 7.0);
    assert_eq!(p.lines.len(), 4, "{p:#?}");
    for (i, line) in p.lines.iter().enumerate() {
        assert!(
            line.caret_stops.iter().all(|s| !span.is_interior(s.offset)),
            "line {i}"
        );
    }
    // The middle lines lie wholly inside the chip.
    assert!(p.lines[1].caret_stops.is_empty());
    assert!(p.lines[2].caret_stops.is_empty());
    assert_queries_total(text, &span, &p);
    assert_arrows_cross_whole(text, &span, &p);
}

#[test]
fn crt1_int13_a_mention_chip_over_three_lines_is_crossed_whole() {
    let text = "@aa bb cc dd";
    let chip = TextRange::new(0, text.len());
    // "@aa " | "bb cc " | "dd" at 6 units a character.
    let (span, p) = lay_out_chip(text, chip, 35.0);
    assert_eq!(line_texts(&p, text), ["@aa ", "bb cc ", "dd"]);
    assert!(p.lines[1].caret_stops.is_empty(), "{:?}", p.lines[1]);
    assert_queries_total(text, &span, &p);
    assert_arrows_cross_whole(text, &span, &p);
}

#[test]
fn int10_a_hit_on_a_stopless_line_snaps_to_an_end_of_the_chip() {
    let text = "@aa bb cc dd";
    let chip = TextRange::new(0, text.len());
    let (_, p) = lay_out_chip(text, chip, 35.0);
    let l = ValidatedLayout::try_from_paragraph(&p, text).unwrap();
    for x in [-5.0, 0.0, 10.0, 20.0, 29.0, 60.0, f32::NAN] {
        let hit = l.hit_test_on_line(1, x);
        assert!(
            [chip.start(), chip.end()].contains(&hit.position.byte_offset),
            "{x}: {hit:?}"
        );
        assert!(!(x.is_nan() && hit.inside), "{x}: {hit:?}");
    }
}

#[test]
fn crt1_a_chip_inside_text_wrapped_over_stopless_lines() {
    let text = "x @aa bb cc dd y";
    let chip = TextRange::new(2, 14);
    let (span, p) = lay_out_chip(text, chip, 35.0);
    assert!(p.lines.iter().any(|l| l.caret_stops.is_empty()), "{p:#?}");
    assert_queries_total(text, &span, &p);
    assert_arrows_cross_whole(text, &span, &p);
}

#[test]
fn int20_a_nan_inline_position_is_never_inside() {
    let text = "ab cd";
    let p = lay_out(text, &column(20.0), &LayoutOptions::default());
    let l = ValidatedLayout::try_from_paragraph(&p, text).unwrap();
    for (i, line) in p.lines.iter().enumerate() {
        let hit = l.hit_test_on_line(i, f32::NAN);
        assert!(!hit.inside, "line {i}: {hit:?}");
        // Not a number counts as beyond the left edge.
        assert_eq!(
            hit.position,
            l.hit_test_on_line(i, -1e9).position,
            "line {i}"
        );
        let y = line.baseline_y - 1.0;
        let hit = l.hit_test(Point { x: f32::NAN, y });
        assert!(!hit.inside, "line {i}: {hit:?}");
    }
    let hit = l.hit_test(Point {
        x: 3.0,
        y: f32::NAN,
    });
    assert!(!hit.inside, "{hit:?}");
    let hit = l.hit_test(Point {
        x: f32::NAN,
        y: f32::NAN,
    });
    assert!(!hit.inside, "{hit:?}");
}
