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

#[test]
fn a_selection_is_one_rectangle_per_line_piece() {
    let text = "ab cd";
    let p = lay_out(text, &column(20.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    assert!(
        l.selection_geometry(&*SHAPER, TextRange::empty_at(2), BoxHeightStyle::Max)
            .is_empty()
    );
    let rects = l.selection_geometry(&*SHAPER, TextRange::new(1, 4), BoxHeightStyle::Strut);
    let pieces: Vec<(usize, f32, f32)> = rects
        .iter()
        .map(|r| (r.line, r.rect.x, r.rect.width))
        .collect();
    // "b" on the first line (the collapsed space has no slot), "c" on the
    // second.
    assert_eq!(pieces, [(0, 6.0, 6.0), (1, 0.0, 6.0)]);
    for r in &rects {
        // The strut: 10.2 above the baseline and 3.0 below.
        assert_close(r.rect.height, 13.2);
        assert!(!r.is_line_end_extension);
    }
    let max = l.selection_geometry(&*SHAPER, TextRange::new(0, 2), BoxHeightStyle::Max);
    let tight = l.selection_geometry(&*SHAPER, TextRange::new(0, 2), BoxHeightStyle::Tight);
    assert_close(max[0].rect.height, p.lines[0].line_height);
    // The monospace instance's own ascent and descent: 10.2 + 3.0.
    assert_close(tight[0].rect.height, 13.2);
    assert_eq!(l.range_rects(TextRange::new(1, 4)).len(), 2);
}

#[test]
fn a_tight_selection_follows_the_fonts_it_crosses() {
    let text = "ab日本";
    let p = lay_out_in(
        text,
        &[FAMILY, "Noto Sans JP"],
        &column(200.0),
        &LayoutOptions::default(),
    );
    let l = ValidatedLayout::new(&p, text).unwrap();
    let latin = l.selection_geometry(&*SCRIPT_SHAPER, TextRange::new(0, 2), BoxHeightStyle::Tight);
    let both = l.selection_geometry(&*SCRIPT_SHAPER, TextRange::new(0, 8), BoxHeightStyle::Tight);
    assert_eq!((latin.len(), both.len()), (1, 1));
    // Noto Sans JP's hhea box (1.16 + 0.288 em) is taller than the
    // monospace face's (1.02 + 0.3 em).
    assert!(both[0].rect.height > latin[0].rect.height + 0.1);
    assert!(both[0].rect.width > latin[0].rect.width);
}

#[test]
fn deletion_removes_marks_alone_and_emoji_objects_and_chips_whole() {
    let text = "e\u{301}\u{1F44B}\u{1F3FD} @al x";
    let base = style(10.0);
    let spans = [AtomicEditSpan {
        span_id: 1,
        byte_range: TextRange::new(12, 15),
    }];
    let paragraph = ParagraphInput {
        atomic_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SHAPER).unwrap();
    let (constraints, options) = (column(200.0), LayoutOptions::default());
    let inputs = ValidatedInputs::new(&ast, text, &constraints, &options).unwrap();
    // Backspace after "e\u{301}" removes the mark alone; Delete before it
    // removes the whole atom.
    assert_eq!(inputs.delete_backward_range(3), TextRange::new(1, 3));
    assert_eq!(inputs.delete_forward_range(0), TextRange::new(0, 3));
    // The emoji modifier sequence goes whole either way.
    assert_eq!(inputs.delete_backward_range(11), TextRange::new(3, 11));
    assert_eq!(inputs.delete_forward_range(3), TextRange::new(3, 11));
    // The chip "@al" goes whole from either side and from inside.
    for (backward, forward) in [(15, 12), (14, 13)] {
        assert_eq!(inputs.delete_backward_range(backward), spans[0].byte_range);
        assert_eq!(inputs.delete_forward_range(forward), spans[0].byte_range);
    }
    assert_eq!(inputs.delete_backward_range(0), TextRange::empty_at(0));
    assert_eq!(
        inputs.delete_forward_range(text.len()),
        TextRange::empty_at(text.len())
    );
    assert_eq!(inputs.copied_text(TextRange::new(12, 15)), "@al");
}

#[test]
fn copying_keeps_or_strips_soft_hyphens() {
    let text = "co\u{AD}op";
    let ast = compile(text);
    let constraints = column(200.0);
    for (policy, expected) in [
        (SoftHyphenPolicy::Keep, "co\u{AD}op"),
        (SoftHyphenPolicy::Strip, "coop"),
    ] {
        let options = LayoutOptions {
            soft_hyphen: policy,
            ..Default::default()
        };
        let inputs = ValidatedInputs::new(&ast, text, &constraints, &options).unwrap();
        assert_eq!(inputs.copied_text(TextRange::new(0, text.len())), expected);
    }
}

#[test]
fn utf16_offsets_convert_and_reject_split_code_points() {
    let text = "a\u{e9}\u{1F600}b";
    assert_eq!(utf8_to_utf16(text, 0), Some(0));
    assert_eq!(utf8_to_utf16(text, 3), Some(2));
    assert_eq!(utf8_to_utf16(text, 7), Some(4));
    assert_eq!(utf8_to_utf16(text, 8), Some(5));
    assert_eq!(utf8_to_utf16(text, 2), None);
    assert_eq!(utf8_to_utf16(text, 9), None);
    assert_eq!(utf16_to_utf8(text, 2), Some(3));
    assert_eq!(utf16_to_utf8(text, 4), Some(7));
    assert_eq!(utf16_to_utf8(text, 5), Some(8));
    // Between the surrogates of U+1F600, and past the text.
    assert_eq!(utf16_to_utf8(text, 3), None);
    assert_eq!(utf16_to_utf8(text, 6), None);
}
