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
fn a_tight_selection_inside_a_ligature_takes_its_font() {
    // "office" in Noto Serif over a monospace default (the strut): the
    // selection covers the second f of "ffi" only, and still takes the
    // serif face's extent, as the whole ligature does.
    let text = "office";
    let base = style(10.0);
    let spans = [StyleSpan {
        byte_range: TextRange::new(0, 6),
        style: script_style(&["Noto Serif"], 10.0),
    }];
    let paragraph = ParagraphInput {
        style_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SCRIPT_SHAPER).unwrap();
    let (constraints, options) = (column(200.0), LayoutOptions::default());
    let p = layout_paragraph(&ast, text, &*SCRIPT_SHAPER, &constraints, &options).unwrap();
    let l = ValidatedLayout::new(&p, text).unwrap();
    let rect =
        |r: TextRange| l.selection_geometry(&*SCRIPT_SHAPER, r, BoxHeightStyle::Tight)[0].rect;
    let part = rect(TextRange::new(2, 3));
    let whole = rect(TextRange::new(1, 4));
    let strut =
        l.selection_geometry(&*SCRIPT_SHAPER, TextRange::new(2, 3), BoxHeightStyle::Strut)[0].rect;
    assert_close(part.height, whole.height);
    assert_close(part.y, whole.y);
    assert!(
        (part.height - strut.height).abs() > 0.1,
        "{part:?} {strut:?}"
    );
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

/// "ab￼cd" in the monospace face with a 10-wide inline object on the
/// U+FFFC, `ascent` over and `descent` under the baseline.
fn with_object(
    ascent: f32,
    descent: f32,
    line_height: LineHeight,
) -> (&'static str, PositionedParagraph) {
    let text = "ab\u{fffc}cd";
    let base = style(10.0);
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(2, 5),
        box_id: 1,
        width: 10.0,
        ascent,
        descent,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::AllowOverflow,
    }];
    let paragraph = ParagraphInput {
        inline_objects: &objects,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SHAPER).unwrap();
    let options = LayoutOptions {
        line_height,
        ..Default::default()
    };
    let p = layout_paragraph(&ast, text, &*SHAPER, &column(200.0), &options).unwrap();
    ValidatedLayout::new(&p, text).unwrap_or_else(|(l, e)| panic!("line {l}: {e}"));
    (text, p)
}

/// `p` re-labelled as one laid out in `mode`: the horizontal layout is a
/// valid single-line flow-space layout in either vertical mode once its runs
/// are set sideways and the block size holds the mode's line bounds.
fn in_mode(p: &PositionedParagraph, mode: WritingMode) -> PositionedParagraph {
    let mut p = p.clone();
    if mode != WritingMode::HorizontalTb {
        p.policy.writing_mode = mode;
        for line in &mut p.lines {
            for run in &mut line.visual_runs {
                run.orientation = GlyphOrientation::VerticalSideways;
            }
        }
        let bottom = p
            .lines
            .iter()
            .map(|l| mode.line_bounds(l.baseline_y, l.ascent, l.descent).1)
            .fold(p.total_block_size, f32::max);
        p.total_block_size = bottom;
    }
    p
}

/// The flow-space block interval of a physical selection rectangle.
fn block_interval(p: &PositionedParagraph, rect: Rect) -> (f32, f32) {
    let flow = p
        .policy
        .writing_mode
        .physical_to_flow_rect(rect, p.total_block_size);
    (flow.y, flow.y + flow.height)
}

#[track_caller]
fn assert_interval(actual: (f32, f32), expected: (f32, f32), what: &str) {
    assert!(
        (actual.0 - expected.0).abs() < 1e-3 && (actual.1 - expected.1).abs() < 1e-3,
        "{what}: {actual:?} != {expected:?}"
    );
}

#[test]
fn int16_a_strut_selection_takes_the_resolved_strut_in_every_writing_mode() {
    // A tall object makes the line box taller than the strut under `Normal`;
    // `Fixed(1.0)` resolves a strut with negative half-leading.
    for line_height in [
        LineHeight::Normal,
        LineHeight::Fixed(1.0),
        LineHeight::Fixed(2.0),
    ] {
        let (text, horizontal) = with_object(30.0, 10.0, line_height);
        for mode in [
            WritingMode::HorizontalTb,
            WritingMode::VerticalLr,
            WritingMode::VerticalRl,
        ] {
            let p = in_mode(&horizontal, mode);
            let what = format!("{line_height:?} {mode:?}");
            let l = ValidatedLayout::new(&p, text)
                .unwrap_or_else(|(line, e)| panic!("{what}: line {line}: {e}"));
            let line = &p.lines[0];
            let strut = p.policy.strut;
            let range = TextRange::new(0, 2);
            let strut_rects = l.selection_geometry(&*SHAPER, range, BoxHeightStyle::Strut);
            let max_rects = l.selection_geometry(&*SHAPER, range, BoxHeightStyle::Max);
            assert_eq!((strut_rects.len(), max_rects.len()), (1, 1), "{what}");
            let (strut_rect, max_rect) = (strut_rects[0].rect, max_rects[0].rect);
            // The strut placed by the mode's rule: in `VerticalLr` the
            // descent lies before the baseline and the ascent after it.
            assert_interval(
                block_interval(&p, strut_rect),
                mode.line_bounds(line.baseline_y, strut.ascent, strut.descent),
                &format!("Strut, {what}"),
            );
            // `Max` spans the line box, placed by the same rule.
            let (top, _) = mode.line_bounds(line.baseline_y, line.ascent, line.descent);
            assert_interval(
                block_interval(&p, max_rect),
                (top, top + line.line_height),
                &format!("Max, {what}"),
            );
            // Both cover the same inline extent.
            let flow = |r: Rect| mode.physical_to_flow_rect(r, p.total_block_size);
            assert_close(flow(strut_rect).x, flow(max_rect).x);
            assert_close(flow(strut_rect).width, flow(max_rect).width);
        }
    }
}

#[test]
fn int16_a_tight_selection_takes_a_wholly_selected_objects_extent() {
    let (text, p) = with_object(30.0, 10.0, LineHeight::Normal);
    let l = ValidatedLayout::new(&p, text).unwrap();
    let line = &p.lines[0];
    // The object is taller and deeper than the monospace instance, so the
    // union is the object's extent wherever the object is selected whole:
    // from its left edge, exactly, or inside a wider interval.
    for (start, end) in [(2, 5), (2, 7), (1, 5), (1, 6), (0, 7)] {
        let rects =
            l.selection_geometry(&*SHAPER, TextRange::new(start, end), BoxHeightStyle::Tight);
        let top = rects.iter().map(|r| r.rect.y).fold(f32::INFINITY, f32::min);
        let bottom = rects
            .iter()
            .map(|r| r.rect.y + r.rect.height)
            .fold(f32::NEG_INFINITY, f32::max);
        assert_interval(
            (top, bottom),
            (line.baseline_y - 30.0, line.baseline_y + 10.0),
            &format!("{start}..{end}: {rects:?}"),
        );
    }
    // Text alone keeps the instance's own extent.
    let rects = l.selection_geometry(&*SHAPER, TextRange::new(0, 2), BoxHeightStyle::Tight);
    assert_close(rects[0].rect.height, 13.2);
}

#[test]
fn int16_selection_rectangles_never_have_negative_height() {
    let styles = [
        BoxHeightStyle::Tight,
        BoxHeightStyle::Max,
        BoxHeightStyle::Strut,
    ];
    for (ascent, descent) in [(30.0, 10.0), (0.0, 0.0), (0.0, 5.0)] {
        for line_height in [
            LineHeight::Normal,
            LineHeight::Fixed(0.1),
            LineHeight::Fixed(0.5),
            LineHeight::Fixed(3.0),
        ] {
            let (text, horizontal) = with_object(ascent, descent, line_height);
            for mode in [WritingMode::HorizontalTb, WritingMode::VerticalLr] {
                let p = in_mode(&horizontal, mode);
                let l = ValidatedLayout::new(&p, text).unwrap();
                for start in 0..text.len() {
                    for end in start + 1..=text.len() {
                        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                            continue;
                        }
                        for style in styles {
                            let range = TextRange::new(start, end);
                            for r in l.selection_geometry(&*SHAPER, range, style) {
                                assert!(
                                    r.rect.height >= 0.0 && r.rect.width >= 0.0,
                                    "{ascent}/{descent} {line_height:?} {mode:?} \
                                     {start}..{end} {style:?}: {r:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn int16_range_rects_of_a_later_line_cover_that_line_only() {
    let text = "ab cd ef";
    let p = lay_out(text, &column(20.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["ab ", "cd ", "ef"]);
    let l = ValidatedLayout::new(&p, text).unwrap();
    for (range, index) in [(TextRange::new(3, 5), 1), (TextRange::new(6, 8), 2)] {
        let line = &p.lines[index];
        let rects = l.range_rects(range);
        assert_eq!(rects.len(), 1, "{range:?}: {rects:?}");
        assert_close(rects[0].y, line.baseline_y - line.ascent);
        assert_close(rects[0].height, line.line_height);
        let selected = l.selection_geometry(&*SHAPER, range, BoxHeightStyle::Max);
        assert!(!selected.is_empty());
        assert!(selected.iter().all(|r| r.line == index), "{selected:?}");
    }
}

#[test]
fn int7_a_point_maps_to_the_first_line_whose_bottom_lies_below_it() {
    let text = "ab cd ef";
    let mut p = lay_out(text, &column(20.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["ab ", "cd ", "ef"]);
    // Open a 5-unit gap between each pair of lines.
    p.lines[1].baseline_y += 5.0;
    p.lines[2].baseline_y += 10.0;
    p.total_block_size += 10.0;
    let l = ValidatedLayout::new(&p, text).unwrap_or_else(|(line, e)| panic!("line {line}: {e}"));
    let bounds: Vec<(f32, f32)> = p
        .lines
        .iter()
        .map(|line| {
            WritingMode::HorizontalTb.line_bounds(line.baseline_y, line.ascent, line.descent)
        })
        .collect();
    // x = 7 is in the left half of the second character of every line.
    let x = 7.0;
    let line_of = |y: f32| {
        let hit = l.hit_test(Point { x, y });
        let line = [1, 4, 7]
            .iter()
            .position(|&o| o == hit.position.byte_offset)
            .unwrap_or_else(|| panic!("y = {y}: {hit:?}"));
        (line, hit.inside)
    };
    for (i, &(top, bottom)) in bounds.iter().enumerate() {
        assert_eq!(line_of((top + bottom) / 2.0), (i, true), "middle of {i}");
        assert_eq!(line_of(top).0, i, "top of {i}");
    }
    // A line's bottom edge and the gap below it belong to the next line.
    for i in 0..2 {
        let (_, bottom) = bounds[i];
        let (next_top, _) = bounds[i + 1];
        assert!(next_top > bottom + 1.0);
        assert_eq!(line_of(bottom).0, i + 1, "bottom of {i}");
        assert_eq!(line_of((bottom + next_top) / 2.0).0, i + 1, "gap after {i}");
        assert_eq!(line_of(bottom - 0.01).0, i, "just above the bottom of {i}");
    }
    // Above the first line and below the last: clamped.
    assert_eq!(line_of(bounds[0].0 - 5.0), (0, false));
    assert_eq!(line_of(-100.0), (0, false));
    assert_eq!(line_of(bounds[2].1 + 5.0), (2, false));
    assert_eq!(line_of(p.total_block_size + 100.0), (2, false));
}

#[test]
fn int12_an_inline_object_is_deleted_whole_from_either_side_and_from_inside() {
    // A three-byte object "OBJ" between "ab" and "cd".
    let text = "abOBJcd";
    let base = style(10.0);
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(2, 5),
        box_id: 1,
        width: 10.0,
        ascent: 8.0,
        descent: 2.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::AllowOverflow,
    }];
    let paragraph = ParagraphInput {
        inline_objects: &objects,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SHAPER).unwrap();
    let (constraints, options) = (column(200.0), LayoutOptions::default());
    let inputs = ValidatedInputs::new(&ast, text, &constraints, &options).unwrap();
    let object = TextRange::new(2, 5);
    for offset in [3, 4, 5] {
        assert_eq!(
            inputs.delete_backward_range(offset),
            object,
            "back {offset}"
        );
    }
    for offset in [2, 3, 4] {
        assert_eq!(inputs.delete_forward_range(offset), object, "fwd {offset}");
    }
    // Next to the object, the unit on the other side is plain text.
    assert_eq!(inputs.delete_backward_range(2), TextRange::new(1, 2));
    assert_eq!(inputs.delete_forward_range(5), TextRange::new(5, 6));
}

#[test]
fn int12_a_multibyte_atomic_span_is_deleted_whole_only_from_the_unit_inside_it() {
    // A chip "@李华" with multi-byte characters, between "x " and " y".
    let text = "x @李华 y";
    let chip = TextRange::new(2, 2 + "@李华".len());
    let base = script_style(&[FAMILY, JAPANESE], 10.0);
    let spans = [AtomicEditSpan {
        span_id: 1,
        byte_range: chip,
    }];
    let paragraph = ParagraphInput {
        atomic_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SCRIPT_SHAPER).unwrap();
    let (constraints, options) = (column(200.0), LayoutOptions::default());
    let inputs = ValidatedInputs::new(&ast, text, &constraints, &options).unwrap();
    let inner = 3 + "李".len();
    for offset in [chip.end(), inner, 3] {
        assert_eq!(inputs.delete_backward_range(offset), chip, "back {offset}");
    }
    for offset in [chip.start(), 3, inner] {
        assert_eq!(inputs.delete_forward_range(offset), chip, "fwd {offset}");
    }
    // The space before the chip and the space after it go alone.
    assert_eq!(
        inputs.delete_backward_range(chip.start()),
        TextRange::new(1, 2)
    );
    assert_eq!(
        inputs.delete_forward_range(chip.end()),
        TextRange::new(chip.end(), chip.end() + 1)
    );
}
