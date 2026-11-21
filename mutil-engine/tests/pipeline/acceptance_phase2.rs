//! The Phase 2 acceptance criteria (spec RM-6, RM-14): the bidi clauses over
//! the vendored Hebrew and Arabic subsets, and the autospace, justification,
//! tolerance and cursive letter-spacing clauses over Noto Sans JP, Noto
//! Serif and the Arabic and Hebrew subsets (`tests/assets/README.md`).

use crate::common::*;
use mutil_engine::*;

/// Noto Sans Hebrew's space at 10 units per em: 270 units (README).
const HEBREW_SPACE: f32 = 2.7;

/// The level of the atom that starts at `offset`.
#[track_caller]
pub fn level_at(ast: &ParagraphAst, offset: usize) -> u8 {
    ast.atoms
        .iter()
        .find(|a| a.byte_range.start() == offset)
        .unwrap_or_else(|| panic!("no atom at {offset}"))
        .bidi_level
        .get()
}

/// Every glyph of `line` in output order.
pub fn glyphs(line: &PositionedLine) -> impl Iterator<Item = &PositionedGlyph> {
    line.visual_runs.iter().flat_map(|r| r.glyphs.iter())
}

/// The first glyph of `line` whose source starts at `offset`.
#[track_caller]
pub fn glyph_at(line: &PositionedLine, offset: usize) -> &PositionedGlyph {
    glyphs(line)
        .find(|g| g.byte_range.is_some_and(|r| r.start() == offset))
        .unwrap_or_else(|| panic!("no glyph at {offset}"))
}

/// The pen position of the glyph at `offset`.
#[track_caller]
pub fn x_of(line: &PositionedLine, offset: usize) -> f32 {
    glyph_at(line, offset).x
}

/// The sum of the natural widths of the clusters inside `range`.
pub fn width_of(ast: &ParagraphAst, range: TextRange) -> f32 {
    ast.clusters
        .iter()
        .filter(|c| range.contains_range(c.byte_range))
        .map(|c| c.natural_width)
        .sum()
}

/// The glyph ids of the cluster whose bytes start at `offset`.
#[track_caller]
pub fn cluster_glyph_ids(ast: &ParagraphAst, offset: usize) -> Vec<u32> {
    let cluster = ast
        .clusters
        .iter()
        .find(|c| c.byte_range.start() == offset)
        .unwrap_or_else(|| panic!("no cluster at {offset}"));
    ast.cluster_glyphs(cluster)
        .iter()
        .map(|g| g.glyph_id)
        .collect()
}

/// OUT-3: pen positions never decrease, within a run and across runs.
#[track_caller]
fn assert_screen_order(line: &PositionedLine) {
    let xs: Vec<f32> = glyphs(line).map(|g| g.x).collect();
    assert!(
        xs.windows(2).all(|w| w[0] <= w[1] + 1e-3),
        "not in screen order: {xs:?}"
    );
}

/// Asserts the strictly increasing order of the glyphs at `offsets`.
#[track_caller]
fn assert_left_to_right(line: &PositionedLine, offsets: &[usize]) {
    let xs: Vec<f32> = offsets.iter().map(|&o| x_of(line, o)).collect();
    assert!(
        xs.windows(2).all(|w| w[0] < w[1]),
        "{offsets:?} are not left to right: {xs:?}"
    );
}

#[test]
fn rm6_hebrew_with_a_level_2_run_reorders_each_line_across_a_wrap() {
    // "אבג abc 123 דהו": the Latin and the digits after it are one level-2
    // run inside the RTL paragraph (W7 makes the digits L, N1 the space
    // between them L).
    let text = "אבג abc 123 דהו";
    let ast = compile_bidi(text, &[HEBREW]);
    assert_eq!(ast.base_direction, Direction::Rtl);
    for (offset, level) in [(0, 1), (4, 1), (7, 2), (9, 2), (11, 2), (13, 2), (15, 1)] {
        assert_eq!(level_at(&ast, offset), level, "atom at {offset}");
    }
    // Unwrapped: אבג on the right, then "abc 123" left to right, then דהו.
    let p = lay_out_bidi_ast(&ast, text, &column(200.0), &LayoutOptions::default());
    assert_eq!(p.lines.len(), 1);
    let line = &p.lines[0];
    assert_screen_order(line);
    assert_left_to_right(line, &[19, 17, 15, 7, 8, 9, 11, 12, 13, 4, 2, 0]);
    // Wrapped after "abc ": L2 runs on each line on its own (ST2-2), so the
    // digits that open the second line sit at its right, left to right.
    let p = lay_out_bidi_ast(&ast, text, &column(45.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["אבג abc ", "123 דהו"]);
    for line in &p.lines {
        assert_screen_order(line);
    }
    assert_left_to_right(&p.lines[0], &[7, 8, 9, 4, 2, 0]);
    assert_left_to_right(&p.lines[1], &[19, 17, 15, 11, 12, 13]);
}

#[test]
fn rm6_arabic_runs_right_to_left_on_every_line_of_a_wrapped_paragraph() {
    let text = "سلام عليكم يا صديقي العزيز";
    let ast = compile_bidi(text, &[ARABIC]);
    assert_eq!(ast.base_direction, Direction::Rtl);
    assert!(ast.atoms.iter().all(|a| a.bidi_level.get() == 1));
    assert!(ast.clusters.iter().all(|c| c.font.font_id == ARABIC_ID));
    let total: f32 = ast.clusters.iter().map(|c| c.natural_width).sum();
    let p = lay_out_bidi_ast(&ast, text, &column(total / 2.5), &LayoutOptions::default());
    assert!(p.lines.len() >= 3, "{:?}", line_texts(&p, text));
    for line in &p.lines {
        assert_screen_order(line);
        // Right to left: the further left a glyph, the later its bytes.
        let starts: Vec<usize> = glyphs(line)
            .filter_map(|g| g.byte_range.map(|r| r.start()))
            .collect();
        assert!(
            starts.windows(2).all(|w| w[0] >= w[1]),
            "{:?}: {starts:?}",
            &text[line.byte_range.as_range()]
        );
        // The line's first byte is its rightmost glyph.
        assert_eq!(starts.last(), Some(&line.byte_range.start()));
    }
}

#[test]
fn rm6_whitespace_before_a_mid_line_tab_resets_to_the_paragraph_level() {
    // In "אבג abc \tdef" the space before the tab resolves to level 2
    // between two L runs, and L1 resets it with the tab to level 1, so it
    // lies between the tab and "abc", not right of "abc" (ST2-1).
    let text = "אבג abc \tdef";
    let ast = compile_bidi(text, &[HEBREW]);
    let constraints = column(200.0);
    let p = lay_out_bidi_ast(&ast, text, &constraints, &LayoutOptions::default());
    let line = &p.lines[0];
    assert_screen_order(line);
    let [tab] = line.tabs.as_slice() else {
        panic!("{:?}", line.tabs)
    };
    let f = glyph_at(line, 14);
    assert_close(f.x + f.advance, tab.x);
    assert_close(x_of(line, 7) - (tab.x + tab.width), HEBREW_SPACE);
    assert_left_to_right(line, &[12, 13, 14, 7, 8, 9, 4, 2, 0]);
    // ST3-1: the tab measures from the line-start edge, the slot's end in
    // an RTL paragraph, over everything logically before it.
    let before = width_of(&ast, TextRange::new(0, 10)) + 2.0 * HEBREW_SPACE;
    let stop = TabStops::DEFAULT.next_stop(before).unwrap();
    assert_close(tab.width, stop - before);
    assert_close(tab.x, line.slots[0].constraint.end() - stop);
}

#[test]
fn rm6_a_tab_in_an_ltr_line_measures_from_the_slot_start() {
    let text = "abc \tdef";
    let ast = compile_bidi(text, &[HEBREW]);
    let constraints = LayoutConstraints {
        first_line_indent: 10.0,
        ..column(200.0)
    };
    let p = lay_out_bidi_ast(&ast, text, &constraints, &LayoutOptions::default());
    let tab = &p.lines[0].tabs[0];
    let before = width_of(&ast, TextRange::new(0, 3)) + HEBREW_SPACE;
    let stop = TabStops::DEFAULT.next_stop(before).unwrap();
    assert_close(tab.width, stop - before);
    assert_close(tab.x + tab.width, 10.0 + stop);
}

#[test]
fn rm6_trailing_whitespace_lies_on_the_far_edge_of_an_rtl_line() {
    // HNG-3, INT-20: the paragraph's last line keeps stops on its trailing
    // spaces, on the left of an RTL line: spread by their widths under
    // `Preserved`, all at the content's edge under `Collapsible`.
    let text = "אבג  ";
    for (whitespace, spread) in [
        (HangingWhitespaceMode::Preserved, HEBREW_SPACE),
        (HangingWhitespaceMode::Collapsible, 0.0),
    ] {
        let options = LayoutOptions {
            whitespace,
            ..Default::default()
        };
        let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &options);
        let line = &p.lines[0];
        assert_eq!(line.visible_byte_end, 6, "{whitespace:?}");
        let left = glyphs(line).map(|g| g.x).fold(f32::INFINITY, f32::min);
        let stop_x = |offset: usize| {
            line.caret_stops
                .iter()
                .find(|s| s.offset == offset)
                .unwrap_or_else(|| panic!("{whitespace:?}: no stop at {offset}"))
                .x
        };
        assert_close(stop_x(6), left);
        assert_close(stop_x(7), left - spread);
        assert_close(stop_x(8), left - 2.0 * spread);
    }
}

#[test]
fn rm6_brackets_mirror_under_bd16_and_n0() {
    let open = glyph_for(RTL_FONTS[0].1, '(').unwrap();
    let close = glyph_for(RTL_FONTS[0].1, ')').unwrap();
    // (text, offsets of '(' and ')', their level, base direction)
    let cases = [
        // Strong R inside an RTL paragraph: R brackets.
        ("אב (גד) הו", 5, 10, 1, None),
        // L inside, R before them, RTL paragraph: N0 c.2, the embedding
        // direction.
        ("אב (cd) הו", 5, 8, 1, None),
        // L inside and before them in an RTL paragraph: N0 c.1, L.
        ("abc (def) אב", 4, 8, 2, Some(Direction::Rtl)),
        // R inside, L before them in an LTR paragraph: N0 c.2, L, although
        // N1 alone would make the closer R between א and ב.
        ("a (א) ב", 2, 5, 0, None),
    ];
    for (text, opener, closer, level, base) in cases {
        let style = script_style(&[HEBREW], 10.0);
        let options = FrontendOptions {
            base_direction: base,
            ..Default::default()
        };
        let ast = compile_bidi_input(&input(text, &style), &options);
        assert_eq!(level_at(&ast, opener), level, "{text}: opener");
        assert_eq!(level_at(&ast, closer), level, "{text}: closer");
        // Single-owner mirroring (SHP-11): the source keeps its brackets and
        // the shaper draws them mirrored at an odd level.
        assert_eq!(&text[opener..opener + 1], "(");
        let (shown_open, shown_close) = if level % 2 == 1 {
            (close, open)
        } else {
            (open, close)
        };
        assert_eq!(cluster_glyph_ids(&ast, opener), [shown_open], "{text}");
        assert_eq!(cluster_glyph_ids(&ast, closer), [shown_close], "{text}");
        let p = lay_out_bidi_ast(&ast, text, &column(200.0), &LayoutOptions::default());
        let line = &p.lines[0];
        assert_screen_order(line);
        // The opener is on the side the bracketed text starts from.
        let (o, c) = (x_of(line, opener), x_of(line, closer));
        assert_eq!(o > c, level % 2 == 1, "{text}: ( at {o}, ) at {c}");
    }
}

#[test]
fn rm6_guillemets_in_an_arabic_paragraph_resolve_as_neutrals() {
    // « and » are not in BidiBrackets.txt (FE-9's Table B), so N0 does not
    // pair them: « between two L runs is L (level 2), » between L and R takes
    // the embedding direction (level 1). The same text with parentheses is
    // a pair under N0 c.1, both L.
    let guillemets = "مرحبا abc «def» عالم";
    let ast = compile_bidi(guillemets, &[ARABIC]);
    assert_eq!(ast.base_direction, Direction::Rtl);
    let open = guillemets.find('«').unwrap();
    let close = guillemets.find('»').unwrap();
    assert_eq!(level_at(&ast, open), 2);
    assert_eq!(level_at(&ast, close), 1);
    lay_out_bidi_ast(&ast, guillemets, &column(300.0), &LayoutOptions::default());
    let parens = "مرحبا abc (def) عالم";
    let ast = compile_bidi(parens, &[ARABIC]);
    assert_eq!(level_at(&ast, parens.find('(').unwrap()), 2);
    assert_eq!(level_at(&ast, parens.find(')').unwrap()), 2);
}

#[test]
fn rm6_a_64_deep_opener_nest_stops_bd16_and_keeps_the_pairs_found_so_far() {
    // "a (א) ב " pairs first; `n` unmatched openers follow, then
    // " c (א) ב". In an LTR paragraph a paired closer around R text after L
    // is L (N0 c.2, level 0); unpaired it is R between two R (N1, level 1).
    // BD16's stack holds 63 openers: with 62 the second pair still fits, with
    // 63 or 64 its opener overflows the stack and BD16 stops, keeping the
    // first pair.
    for (n, second_paired) in [(62, true), (63, false), (64, false)] {
        let text = format!("a (א) ב {} c (א) ב", "(".repeat(n));
        let ast = compile_bidi(&text, &[HEBREW]);
        assert_eq!(ast.base_direction, Direction::Ltr);
        let first_closer = text.find(')').unwrap();
        let second_closer = text.rfind(')').unwrap();
        assert_eq!(level_at(&ast, first_closer), 0, "{n}: first pair");
        assert_eq!(
            level_at(&ast, second_closer),
            if second_paired { 0 } else { 1 },
            "{n}: second pair"
        );
        lay_out_bidi_ast(&ast, &text, &column(2000.0), &LayoutOptions::default());
    }
}

fn decorated(
    text: &str,
    span: TextRange,
    break_mode: BoxDecorationBreak,
    padding: (f32, f32),
) -> ParagraphAst {
    let style = script_style(&[HEBREW], 10.0);
    let spans = [InlineDecorationSpan {
        span_id: 7,
        byte_range: span,
        start_padding: padding.0,
        end_padding: padding.1,
        break_mode,
        height_style: mutil_engine::BoxHeightStyle::Strut,
    }];
    let paragraph = ParagraphInput {
        decoration_spans: &spans,
        ..input(text, &style)
    };
    compile_bidi_input(&paragraph, &FrontendOptions::default())
}

#[test]
fn rm6_a_span_split_by_l2_yields_two_fragments() {
    // "ab אבג cd" with a span over "b אב" (1..7): visually "a b ␣ ג ב א ␣ c
    // d", so ג splits the span's items into "b ␣" (1..3) and "ב א" (3..7),
    // each contiguous visually and logically (FRG-1, ST2-3).
    let text = "ab אבג cd";
    let span = TextRange::new(1, 7);
    let ast = decorated(text, span, BoxDecorationBreak::Slice, (2.0, 3.0));
    let p = lay_out_bidi_ast(&ast, text, &column(200.0), &LayoutOptions::default());
    let line = &p.lines[0];
    let mut fragments = line.span_fragments.clone();
    fragments.sort_by(|a, b| a.x.total_cmp(&b.x));
    let [left, right] = fragments.as_slice() else {
        panic!("{fragments:#?}")
    };
    for f in [left, right] {
        assert_eq!((f.span_id, f.span_range), (7, span));
    }
    assert_eq!(left.byte_range, TextRange::new(1, 3));
    assert_eq!(right.byte_range, TextRange::new(3, 7));
    assert!(left.is_first_fragment() && !left.is_last_fragment());
    assert!(right.is_last_fragment() && !right.is_first_fragment());
    assert_eq!(left.bidi_level.get(), 0);
    assert_eq!(right.bidi_level.get(), 1);
    // The start padding opens the left fragment before "b"; the end padding
    // follows ב, the span's last atom, on its left at level 1.
    assert_close(x_of(line, 1) - left.x, 2.0);
    assert_close(left.width, 2.0 + glyph_at(line, 1).advance + HEBREW_SPACE);
    assert_close(x_of(line, 5) - right.x, 3.0);
    let alef = glyph_at(line, 3);
    assert_close(right.x + right.width, alef.x + alef.advance);
}

#[test]
fn rm6_a_chip_over_two_rtl_lines_clones_or_slices_its_padding() {
    // "אבג דהו זחט" with a span over "דהו זחט" that wraps after "דהו": the
    // line start is the right edge of an RTL line and the line end its left
    // edge (ST2-3, AX-5).
    let text = "אבג דהו זחט";
    let span = TextRange::new(7, 20);
    let (start, end) = (3.0, 5.0);
    let probe = decorated(text, span, BoxDecorationBreak::Clone, (start, end));
    let first_line = width_of(&probe, TextRange::new(0, 13)) + HEBREW_SPACE + start + end;
    let constraints = column(first_line + 1.0);
    for (mode, sides) in [
        // (right padding, left padding) of the fragment on each line.
        (BoxDecorationBreak::Clone, [(start, end), (start, end)]),
        (BoxDecorationBreak::Slice, [(start, 0.0), (0.0, end)]),
    ] {
        let ast = decorated(text, span, mode, (start, end));
        let p = lay_out_bidi_ast(&ast, text, &constraints, &LayoutOptions::default());
        assert_eq!(line_texts(&p, text), ["אבג דהו ", "זחט"], "{mode:?}");
        for (i, (line, (right, left))) in p.lines.iter().zip(sides).enumerate() {
            let [fragment] = line.span_fragments.as_slice() else {
                panic!("{mode:?} line {i}: {:#?}", line.span_fragments)
            };
            assert_eq!(fragment.is_first_fragment(), i == 0, "{mode:?} line {i}");
            assert_eq!(fragment.is_last_fragment(), i == 1, "{mode:?} line {i}");
            let inside: Vec<&PositionedGlyph> = glyphs(line)
                .filter(|g| g.byte_range.is_some_and(|r| span.contains_range(r)))
                .collect();
            let ink_left = inside.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            let ink_right = inside
                .iter()
                .map(|g| g.x + g.advance)
                .fold(f32::NEG_INFINITY, f32::max);
            assert_close(fragment.x + fragment.width - ink_right, right);
            assert_close(ink_left - fragment.x, left);
        }
    }
}

#[test]
fn rm6_clicks_beyond_the_ends_of_an_rtl_line_take_its_extreme_stops() {
    let text = "אבג דהו";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::try_from_paragraph(&p, text).unwrap();
    let line = &p.lines[0];
    let y = line.baseline_y;
    // Left of an RTL line is its end: the content there has the base
    // direction, so the hit is the extreme stop at the visible end (INT-8).
    // INT-20 fixes no affinity for the paragraph end, so only the offset is
    // checked here; the soft-wrapped case below is upstream.
    let left = l.hit_test(Point { x: -50.0, y });
    let extreme = line.extreme_caret_stop(HangSide::Left).unwrap();
    assert_eq!(extreme.offset, 13);
    assert_eq!(
        left.position,
        CursorPosition {
            byte_offset: extreme.offset,
            affinity: extreme.affinity
        }
    );
    assert!(!left.inside);
    // Right of it is its start.
    let right = l.hit_test(Point { x: 500.0, y });
    assert_eq!(right.position.byte_offset, 0);
    assert!(!right.inside);
    let extreme = line.extreme_caret_stop(HangSide::Right).unwrap();
    assert_eq!(right.position.byte_offset, extreme.offset);
    // A wrapped line: past its left end is its visible end, upstream.
    let p = lay_out_bidi(text, &[HEBREW], &column(20.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["אבג ", "דהו"]);
    let l = ValidatedLayout::try_from_paragraph(&p, text).unwrap();
    let hit = l.hit_test(Point {
        x: -50.0,
        y: p.lines[0].baseline_y,
    });
    assert_eq!(
        hit.position,
        CursorPosition {
            byte_offset: 6,
            affinity: CaretAffinity::Upstream
        }
    );
    let hit = l.hit_test(Point {
        x: 500.0,
        y: p.lines[1].baseline_y,
    });
    assert_eq!(hit.position.byte_offset, 7);
}

#[test]
fn rm6_clicks_beyond_the_ends_of_a_mixed_line_take_its_extreme_stops() {
    // "abcאבג" in an LTR paragraph: the right end of the line is the start
    // of the Hebrew word (offset 3), not the visible end 9 (INT-8 forbids
    // the jump).
    let text = "abcאבג";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::try_from_paragraph(&p, text).unwrap();
    let line = &p.lines[0];
    let y = line.baseline_y;
    let right = l.hit_test(Point { x: 500.0, y });
    assert_eq!(
        right.position,
        CursorPosition {
            byte_offset: 3,
            affinity: CaretAffinity::Downstream
        }
    );
    assert!(!right.inside);
    let left = l.hit_test(Point { x: -50.0, y });
    assert_eq!(left.position.byte_offset, 0);
    assert!(!left.inside);
    // "אבג abc" in an RTL paragraph: the left end is the start of "abc"
    // (offset 7, level 2), not the visible end 10.
    let text = "אבג abc";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::try_from_paragraph(&p, text).unwrap();
    let y = p.lines[0].baseline_y;
    let left = l.hit_test(Point { x: -50.0, y });
    assert_eq!(
        left.position,
        CursorPosition {
            byte_offset: 7,
            affinity: CaretAffinity::Downstream
        }
    );
    assert!(!left.inside);
    let right = l.hit_test(Point { x: 500.0, y });
    assert_eq!(right.position.byte_offset, 0);
}

#[test]
fn rm6_clicks_above_and_below_the_text_clamp_to_the_first_and_last_lines() {
    let text = "אבג דהו זחט";
    let p = lay_out_bidi(text, &[HEBREW], &column(20.0), &LayoutOptions::default());
    assert_eq!(p.lines.len(), 3);
    let l = ValidatedLayout::try_from_paragraph(&p, text).unwrap();
    for x in [-10.0, 5.0, 12.0, 30.0] {
        let above = l.hit_test(Point { x, y: -100.0 });
        assert!(!above.inside, "{x}");
        assert_eq!(above.position, l.hit_test_on_line(0, x).position, "{x}");
        let below = l.hit_test(Point {
            x,
            y: p.total_block_size + 100.0,
        });
        assert!(!below.inside, "{x}");
        assert_eq!(below.position, l.hit_test_on_line(2, x).position, "{x}");
    }
    // A click on a glyph is inside.
    let line = &p.lines[1];
    let g = glyph_at(line, 9);
    let on = l.hit_test(Point {
        x: g.x + g.advance * 0.25,
        y: line.baseline_y,
    });
    assert!(on.inside);
    // The left quarter of ה is its trailing side in RTL: offset 11.
    assert_eq!(on.position.byte_offset, 11);
}

#[test]
fn rm6_mixed_cjk_and_latin_autospaces_by_profile_and_justifies_flush_through_the_ladder() {
    use crate::justification::{CJK_GAP, ladder};
    // "漢字abc漢字": a gap of 1/4 em (JLReq) or 1/8 em (CSS) opens at each
    // CJK-Latin seam of an unjustified line (FE-20).
    let text = "漢字abc漢字";
    let style = script_style(&[SERIF, JAPANESE], 10.0);
    for (profile, gap) in [
        (AutospaceProfile::Jlreq, 2.5),
        (AutospaceProfile::CssCompat, 1.25),
    ] {
        let options = FrontendOptions {
            autospace_profile: profile,
            ..Default::default()
        };
        let ast = compile_bidi_input(&input(text, &style), &options);
        let p = lay_out_bidi_ast(&ast, text, &column(200.0), &LayoutOptions::default());
        let line = &p.lines[0];
        let after = |offset: usize| {
            let g = glyph_at(line, offset);
            g.x + g.advance
        };
        assert_close(x_of(line, 6) - after(3), gap);
        assert_close(x_of(line, 9) - after(8), gap);
        assert_close(x_of(line, 3) - after(0), 0.0);
        assert_close(
            line.visual_width(),
            width_of(&ast, TextRange::new(0, 15)) + 2.0 * gap,
        );
    }
    // Justified, the ladder spends the word space, then the autospace (none
    // under CSS, which is rigid), then the CJK gaps, and the line is flush.
    let probe = ladder(AutospaceProfile::Jlreq, 0.0);
    let (s0, s1) = (probe.word_stretch, probe.autospace_stretch);
    ladder(AutospaceProfile::Jlreq, s0 + s1 + 2.0 * CJK_GAP).assert_levels(
        s0,
        s1,
        2.0 * CJK_GAP / 3.0,
    );
    let probe = ladder(AutospaceProfile::CssCompat, 0.0);
    let s0 = probe.word_stretch;
    ladder(AutospaceProfile::CssCompat, s0 + 3.0 * CJK_GAP).assert_levels(s0, 0.0, CJK_GAP);
}

#[test]
fn rm6_over_loose_lines_follow_the_tolerance_rule() {
    use crate::justification::{CJK_GAP, ladder};
    let probe = ladder(AutospaceProfile::Jlreq, 0.0);
    let total = probe.word_stretch + probe.autospace_stretch + 3.0 * CJK_GAP;
    // Beyond Tiers 0 to 2 by less than one 0.125 em Level 3 gap: Level 3
    // covers it and the line is flush (ST3-4).
    let l = ladder(AutospaceProfile::Jlreq, total + 1.0);
    let line = &l.justified.lines[0];
    assert!(line.is_justified);
    assert_close(line.visual_width(), l.natural + total + 1.0);
    // Far beyond: every level spends its cap and the rest stays ragged at
    // the line end.
    let l = ladder(AutospaceProfile::Jlreq, total + 30.0);
    let line = &l.justified.lines[0];
    assert!(!line.is_justified);
    assert!(line.visual_width() > l.natural + total);
    assert!(line.visual_width() < l.natural + total + 30.0 - 1.0);
    assert_close(line.slots[0].content_start, 0.0);
    // The last line is never justified, loose or not.
    assert!(!l.justified.lines[1].is_justified);
}

#[test]
fn rm6_cursive_scripts_get_no_letter_spacing() {
    // An Arabic paragraph asking for 1.5 units of letter spacing, wrapped and
    // justified: its runs take none, and its glyphs keep the advances of an
    // unspaced paragraph. A Hebrew one takes it.
    let text = "سلام عليكم يا صديقي العزيز";
    let spaced = TextStyle {
        letter_spacing: 1.5,
        ..script_style(&[ARABIC], 10.0)
    };
    let ast = compile_bidi_input(&input(text, &spaced), &FrontendOptions::default());
    let plain = compile_bidi(text, &[ARABIC]);
    assert!(ast.runs.iter().all(|r| r.letter_spacing == 0.0));
    assert!(
        ast.atoms
            .iter()
            .all(|a| a.flags.contains(AtomFlags::NO_LETTER_SPACING))
    );
    for (c, p) in ast.clusters.iter().zip(&plain.clusters) {
        assert_close(c.natural_width, p.natural_width);
    }
    let total: f32 = ast.clusters.iter().map(|c| c.natural_width).sum();
    let options = LayoutOptions {
        justify: true,
        ..Default::default()
    };
    let p = lay_out_bidi_ast(&ast, text, &column(total / 2.5), &options);
    assert!(p.lines.len() >= 3);
    for line in &p.lines {
        for g in glyphs(line) {
            let offset = g.byte_range.unwrap().start();
            let cluster = plain
                .clusters
                .iter()
                .find(|c| c.byte_range.start() == offset)
                .unwrap();
            let natural: f32 = plain
                .cluster_glyphs(cluster)
                .iter()
                .map(|g| g.x_advance)
                .sum();
            let drawn: f32 = glyphs(line)
                .filter(|h| h.byte_range == g.byte_range)
                .map(|h| h.advance)
                .sum();
            assert_close(drawn, natural);
        }
    }
    // Hebrew is not cursive: every cluster gains the spacing.
    let text = "אבג דהו";
    let spaced = TextStyle {
        letter_spacing: 1.5,
        ..script_style(&[HEBREW], 10.0)
    };
    let ast = compile_bidi_input(&input(text, &spaced), &FrontendOptions::default());
    let plain = compile_bidi(text, &[HEBREW]);
    assert!(ast.runs.iter().all(|r| r.letter_spacing == 1.5));
    for (c, p) in ast.clusters.iter().zip(&plain.clusters) {
        assert_close(c.natural_width, p.natural_width + 1.5);
    }
}
