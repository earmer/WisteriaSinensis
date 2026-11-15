//! The §6 queries on laid-out text: hit testing, caret geometry and cursor
//! movement (spec INT-7 to INT-15).

use crate::common::*;
use mutil_engine::*;

fn at(offset: usize) -> CursorPosition {
    CursorPosition {
        byte_offset: offset,
        affinity: CaretAffinity::Downstream,
    }
}

fn moved(m: CursorMove) -> usize {
    match m {
        CursorMove::Moved(p) => p.byte_offset,
        CursorMove::Exited { .. } => panic!("unexpected exit: {m:?}"),
    }
}

#[test]
fn a_click_lands_on_the_nearest_grapheme_boundary() {
    let text = "abc";
    let p = lay_out(text, &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    let y = 5.0;
    // Glyphs are 6 wide: the left half of `b` (6..9) maps to 1.
    for (x, offset) in [(0.5, 0), (2.9, 0), (3.1, 1), (8.0, 1), (9.5, 2), (17.0, 3)] {
        let hit = l.hit_test(Point { x, y });
        assert_eq!(hit.position.byte_offset, offset, "x = {x}");
        assert!(hit.inside);
    }
    // Beyond the line's end, and above the first line: clamped.
    let beyond = l.hit_test(Point { x: 80.0, y });
    assert_eq!((beyond.position.byte_offset, beyond.inside), (3, false));
    let above = l.hit_test(Point { x: 7.0, y: -20.0 });
    assert_eq!((above.position.byte_offset, above.inside), (1, false));
}

#[test]
fn a_click_past_a_wrapped_line_lands_upstream_at_its_visible_end() {
    // INT-8, INT-9.
    let text = "ab cd";
    let p = lay_out(text, &column(20.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    let end_of_first = l.hit_test(Point { x: 19.0, y: 1.0 });
    assert_eq!(
        end_of_first.position,
        CursorPosition {
            byte_offset: 2,
            affinity: CaretAffinity::Upstream
        }
    );
    let second_line_y = p.lines[1].baseline_y;
    let start_of_second = l.hit_test(Point {
        x: -4.0,
        y: second_line_y,
    });
    assert_eq!(start_of_second.position, at(3));
    assert_eq!(l.hit_test_on_line(1, 7.0).position, at(4));
}

#[test]
fn caret_rectangles_span_the_line_box() {
    let text = "ab\u{2028}cd";
    let p = lay_out(text, &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    let caret = l.caret_geometry(at(6));
    assert_close(caret.primary_rect.x, 6.0);
    assert_close(
        caret.primary_rect.y,
        p.lines[1].baseline_y - p.lines[1].ascent,
    );
    assert_close(caret.primary_rect.height, p.lines[1].line_height);
    assert!(caret.secondary_rect.is_none());
    assert_eq!(l.rect_for_offset(1), l.caret_geometry(at(1)).primary_rect);
}

#[test]
fn arrows_move_one_stop_and_exit_at_the_paragraph_edges() {
    let text = "ab cd";
    let p = lay_out(text, &column(20.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    assert_eq!(moved(l.move_cursor(at(0), Motion::Right, None)), 1);
    assert_eq!(
        l.move_cursor(at(0), Motion::Left, None),
        CursorMove::Exited {
            edge: ParagraphEdge::Start,
            inline_position: None
        }
    );
    // Past the end of the first line, over its collapsed space, onto the
    // start of the second (INT-13).
    let upstream = CursorPosition {
        byte_offset: 2,
        affinity: CaretAffinity::Upstream,
    };
    assert_eq!(
        l.move_cursor(upstream, Motion::Right, None),
        CursorMove::Moved(at(3))
    );
    assert_eq!(moved(l.move_cursor(at(3), Motion::Left, None)), 2);
    assert_eq!(
        l.move_cursor(at(5), Motion::Right, None),
        CursorMove::Exited {
            edge: ParagraphEdge::End,
            inline_position: None
        }
    );
    // Logical motions.
    assert_eq!(moved(l.move_cursor(at(1), Motion::Next, None)), 2);
    assert_eq!(moved(l.move_cursor(at(2), Motion::Next, None)), 3);
    assert_eq!(moved(l.move_cursor(at(3), Motion::Previous, None)), 2);
    assert!(matches!(
        l.move_cursor(at(5), Motion::Next, None),
        CursorMove::Exited {
            edge: ParagraphEdge::End,
            ..
        }
    ));
}

#[test]
fn line_edges_and_block_motion() {
    let text = "ab cd";
    let p = lay_out(text, &column(20.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    assert_eq!(moved(l.move_cursor(at(1), Motion::LineStart, None)), 0);
    assert_eq!(
        l.move_cursor(at(0), Motion::LineEnd, None),
        CursorMove::Moved(CursorPosition {
            byte_offset: 2,
            affinity: CaretAffinity::Upstream
        })
    );
    // Down keeps the inline position: x 6 on line 0 is x 6 on line 1.
    assert_eq!(moved(l.move_cursor(at(1), Motion::Down, None)), 4);
    assert_eq!(moved(l.move_cursor(at(4), Motion::Up, None)), 1);
    assert_eq!(
        l.move_cursor(at(1), Motion::Up, Some(3.0)),
        CursorMove::Exited {
            edge: ParagraphEdge::Start,
            inline_position: Some(3.0)
        }
    );
}

#[test]
fn word_motions_follow_word_boundaries() {
    let text = "one two three";
    let p = lay_out(text, &column(1000.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    assert_eq!(moved(l.move_cursor(at(0), Motion::WordNext, None)), 3);
    assert_eq!(moved(l.move_cursor(at(3), Motion::WordNext, None)), 4);
    assert_eq!(moved(l.move_cursor(at(5), Motion::WordPrevious, None)), 4);
    assert_eq!(moved(l.move_cursor(at(5), Motion::WordRight, None)), 7);
}

#[test]
fn a_combining_mark_moves_with_its_base() {
    let text = "e\u{301}x";
    let p = lay_out(text, &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    assert_eq!(moved(l.move_cursor(at(0), Motion::Right, None)), 3);
    assert_eq!(moved(l.move_cursor(at(0), Motion::Next, None)), 3);
}
