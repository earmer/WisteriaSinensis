//! `layout_paragraph` on compiled text (spec §5): greedy breaking, the
//! overflow ladder, blank lines, line boxes, and that every result passes
//! the output gate (G4).

use crate::common::*;
use mutil_engine::*;

const TEXTS: &[&str] = &[
    "",
    "a",
    "The quick brown fox jumps over the lazy dog.",
    "  leading and trailing spaces  ",
    "well-known (parenthesised) words, and punctuation!",
    "a\u{2028}b",
    "abc\u{2028}",
    "\u{2028}\u{2028}",
    "tab\tstop\tagain",
    "no\u{a0}break\u{a0}spaces",
    "e\u{301}e\u{301} combining marks",
    "Supercalifragilisticexpialidocious",
    "!!!!!!!!!!!!!!!!!!!!!!!!",
    "a  b   c",
    " ",
];

#[test]
fn every_text_lays_out_validly_at_every_width_and_policy() {
    let options = [
        LayoutOptions::default(),
        LayoutOptions {
            whitespace: HangingWhitespaceMode::Preserved,
            ..Default::default()
        },
        LayoutOptions {
            line_height: LineHeight::Fixed(1.2),
            ..Default::default()
        },
    ];
    for text in TEXTS {
        for wrap_width in [1.0, 6.0, 13.0, 30.0, 61.0, 100.0, f32::INFINITY] {
            for indent in [0.0, 5.0] {
                let constraints = LayoutConstraints {
                    first_line_indent: if wrap_width > indent { indent } else { 0.0 },
                    ..column(wrap_width)
                };
                for options in &options {
                    lay_out(text, &constraints, options);
                }
            }
        }
    }
}

#[test]
fn lines_break_greedily_at_word_spaces() {
    // Every glyph and space is 6 units wide at size 10.
    let text = "aaa bbb ccc";
    let p = lay_out(text, &column(45.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["aaa bbb ", "ccc"]);
    assert_eq!(p.lines[0].visible_byte_end, 7);
    // Trailing spaces collapse: the body is the seven visible advances.
    assert_close(p.lines[0].visual_width(), 42.0);
    let unbounded = lay_out(text, &column(f32::INFINITY), &LayoutOptions::default());
    assert_eq!(unbounded.lines.len(), 1);
}

#[test]
fn a_unit_too_wide_for_the_column_moves_whole_then_splits_at_clusters() {
    // RM-4: a run of `!` wider than the column wraps at cluster boundaries
    // only once it reaches an empty line.
    let text = "ab cd!!!!!!";
    let p = lay_out(text, &column(30.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["ab ", "cd!!!", "!!!"]);
    // UAX #14 LB13: no break before `!`, even after a space, so the whole
    // text is one unit and splits only where it overflows.
    let text = "ab !!!!!!";
    let p = lay_out(text, &column(30.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["ab !!", "!!!!"]);
    // A cluster wider than the column takes a line of its own (SLT-5 B).
    let p = lay_out("ab", &column(1.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, "ab"), ["a", "b"]);
}

#[test]
fn hard_breaks_give_one_line_more_than_they_count() {
    // MET-3.
    for (text, lines) in [
        ("", 1),
        ("abc", 1),
        ("abc\u{2028}", 2),
        ("\u{2028}\u{2028}", 3),
    ] {
        let p = lay_out(text, &column(100.0), &LayoutOptions::default());
        assert_eq!(p.lines.len(), lines, "{text:?}");
    }
    let p = lay_out("abc\u{2028}", &column(100.0), &LayoutOptions::default());
    assert_eq!(p.lines[0].byte_range, TextRange::new(0, 6));
    assert_eq!(p.lines[0].visible_byte_end, 3);
    assert_eq!(p.lines[1].byte_range, TextRange::empty_at(6));
}

#[test]
fn line_boxes_stack_from_the_strut_and_the_fonts() {
    let p = lay_out("a\u{2028}b", &column(100.0), &LayoutOptions::default());
    // Typo metrics at size 10: 10.2 above, 3 below, no gap.
    for (i, line) in p.lines.iter().enumerate() {
        assert_close(line.ascent, 10.2);
        assert_close(line.descent, 3.0);
        assert_close(line.baseline_y, 10.2 + 13.2 * i as f32);
    }
    assert_close(p.total_block_size, 26.4);
    let fixed = LayoutOptions {
        line_height: LineHeight::Fixed(2.0),
        ..Default::default()
    };
    let p = lay_out("a", &column(100.0), &fixed);
    assert_close(p.lines[0].line_height, 20.0);
    assert_close(p.lines[0].ascent, 10.2 + 3.4);
}

#[test]
fn tabs_advance_to_the_next_stop() {
    let constraints = LayoutConstraints {
        tab_stops: TabStops::new(40.0).unwrap(),
        ..column(200.0)
    };
    let p = lay_out("ab\tc", &constraints, &LayoutOptions::default());
    let tab = p.lines[0].tabs[0];
    assert_close(tab.x, 12.0);
    assert_close(tab.width, 28.0);
}

#[test]
fn a_tab_with_no_reachable_stop_takes_no_advance_and_ends_the_line() {
    // ST3-1, AX-6: the first stop (8) lies beyond a 5-unit slot.
    let p = lay_out("\tc", &column(5.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, "\tc"), ["\t", "c"]);
    assert_eq!(p.lines[0].tabs[0].width, 0.0);
    // With content and an earlier opportunity, the line breaks before the
    // tab: after "a b" the pen is at 18 and the next stop, 24, lies beyond a
    // 20-unit slot. On the next line the stop after "b" (8) is reachable.
    let text = "a b\td";
    let p = lay_out(text, &column(20.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["a ", "b\td"]);
    let tab = p.lines[1].tabs[0];
    assert_close(tab.x, 6.0);
    assert_close(tab.width, 2.0);
    // A stop exactly at the slot's end is reachable.
    let p = lay_out("abc\td", &column(24.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, "abc\td"), ["abc\t", "d"]);
    assert_close(p.lines[0].tabs[0].width, 6.0);
}

#[test]
fn caret_stops_cover_every_grapheme_boundary_of_a_line() {
    let text = "ab cd";
    let p = lay_out(text, &column(100.0), &LayoutOptions::default());
    let stops: Vec<_> = p.lines[0]
        .caret_stops
        .iter()
        .map(|s| (s.offset, s.x))
        .collect();
    assert_eq!(stops.len(), 6);
    for (k, (offset, x)) in stops.into_iter().enumerate() {
        assert_eq!(offset, k);
        assert_close(x, 6.0 * k as f32);
    }
    // A wrapped line ends upstream at its visible end (INT-9).
    let p = lay_out(text, &column(20.0), &LayoutOptions::default());
    let last = p.lines[0].caret_stops.last().unwrap();
    assert_eq!((last.offset, last.affinity), (2, CaretAffinity::Upstream));
}

#[test]
fn glyph_runs_place_every_glyph_with_a_finite_origin() {
    let text = "Hello, world";
    let p = lay_out(text, &column(100.0), &LayoutOptions::default());
    let layout = ValidatedLayout::new(&p, text).unwrap();
    let glyphs: usize = layout.glyph_runs().map(|run| run.glyphs().count()).sum();
    assert_eq!(glyphs, text.chars().filter(|c| *c != ' ').count());
}

fn object(start: usize, width: f32, policy: BoxOverflowPolicy) -> InlineObjectInput {
    InlineObjectInput {
        byte_range: TextRange::new(start, start + 3),
        box_id: start as u64,
        width,
        ascent: 20.0,
        descent: 4.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: policy,
    }
}

#[test]
fn inline_objects_follow_their_overflow_policy() {
    let text = "ab \u{fffc} cd";
    let base = style(10.0);
    for (policy, width, expected) in [
        (BoxOverflowPolicy::AllowOverflow, 80.0, (80.0, 20.0)),
        (BoxOverflowPolicy::Clip, 80.0, (30.0, 20.0)),
        (BoxOverflowPolicy::ScaleToFit, 60.0, (30.0, 10.0)),
        (BoxOverflowPolicy::Clip, 12.0, (12.0, 20.0)),
    ] {
        let objects = [object(3, width, policy)];
        let input = ParagraphInput {
            inline_objects: &objects,
            ..input(text, &base)
        };
        let ast = compile_paragraph(&input, &FrontendOptions::default(), &*SHAPER).unwrap();
        for options in [
            LayoutOptions::default(),
            LayoutOptions {
                line_height: LineHeight::Fixed(1.0),
                ..Default::default()
            },
        ] {
            let p = layout_paragraph(&ast, text, &*SHAPER, &column(30.0), &options).unwrap();
            ValidatedLayout::new(&p, text).unwrap_or_else(|e| panic!("{policy:?}: {e:?}"));
            let b = p.lines.iter().flat_map(|l| &l.visual_boxes).next().unwrap();
            assert_eq!((b.width, b.ascent), expected, "{policy:?}");
            if options.line_height == LineHeight::Normal {
                let line = p.lines.iter().find(|l| !l.visual_boxes.is_empty()).unwrap();
                assert!(line.ascent >= b.ascent);
            }
        }
    }
}

#[test]
fn decorations_merge_across_words_and_spaces() {
    let text = "ab cd";
    let mut underlined = style(10.0);
    underlined.decoration.underline = true;
    underlined.decoration.strikethrough = true;
    let ast = compile_paragraph(
        &input(text, &underlined),
        &FrontendOptions::default(),
        &*SHAPER,
    )
    .unwrap();
    let p = layout_paragraph(
        &ast,
        text,
        &*SHAPER,
        &column(100.0),
        &LayoutOptions::default(),
    )
    .unwrap();
    ValidatedLayout::new(&p, text).unwrap();
    let lines = &p.lines[0].decorations;
    assert_eq!(lines.len(), 2);
    let underline = lines
        .iter()
        .find(|d| d.kind == DecorationKind::Underline)
        .unwrap();
    assert_eq!(underline.byte_range, TextRange::new(0, 5));
    assert_close(underline.width, 30.0);
    // Positive towards line-over: an underline sits below the baseline.
    assert_close(underline.y_offset, -1.55);
    let strike = lines
        .iter()
        .find(|d| d.kind == DecorationKind::Strikethrough)
        .unwrap();
    assert_close(strike.y_offset, 3.2);
}

#[test]
fn st5_1_trailing_whitespace_in_decoration_span_collapses_at_wrap() {
    let text = "word next";
    let mut underlined = style(10.0);
    underlined.decoration.underline = true;
    let spans = [InlineDecorationSpan {
        span_id: 1,
        byte_range: TextRange::new(0, 5), // "word " including trailing space
        start_padding: 0.0,
        end_padding: 0.0,
        break_mode: BoxDecorationBreak::Slice,
    }];
    let paragraph_input = ParagraphInput {
        decoration_spans: &spans,
        ..input(text, &underlined)
    };
    let ast = compile_paragraph(&paragraph_input, &FrontendOptions::default(), &*SHAPER)
        .expect("compiles");
    // "word" is 4 * 6 = 24.0; "word next" is 9 * 6 = 54.0.
    // At wrap_width 36.0, "word " wraps before "next" and the trailing space collapses.
    let p = layout_paragraph(
        &ast,
        text,
        &*SHAPER,
        &column(36.0),
        &LayoutOptions::default(),
    )
    .expect("lays out");
    ValidatedLayout::new(&p, text).expect("validates");
    assert_eq!(line_texts(&p, text), ["word ", "next"]);
    assert_eq!(p.lines[0].visible_byte_end, 4);
    assert_close(p.lines[0].slots[0].visual_width, 24.0);
    assert_eq!(p.lines[0].decorations.len(), 1);
    let underline = &p.lines[0].decorations[0];
    assert_eq!(underline.kind, DecorationKind::Underline);
    assert_eq!(underline.byte_range, TextRange::new(0, 4));
    assert_close(underline.width, 24.0);
}

#[test]
fn st5_1_nested_zero_padding_decoration_span_does_not_split_underline() {
    let text = "ab cd ef";
    let mut underlined = style(10.0);
    underlined.decoration.underline = true;
    let style_spans = [StyleSpan {
        byte_range: TextRange::new(3, 5),
        style: underlined.clone(),
    }];
    let decoration_spans = [
        InlineDecorationSpan {
            span_id: 1,
            byte_range: TextRange::new(0, 8),
            start_padding: 0.0,
            end_padding: 0.0,
            break_mode: BoxDecorationBreak::Slice,
        },
        InlineDecorationSpan {
            span_id: 2,
            byte_range: TextRange::new(3, 5),
            start_padding: 0.0,
            end_padding: 0.0,
            break_mode: BoxDecorationBreak::Slice,
        },
    ];
    let paragraph_input = ParagraphInput {
        style_spans: &style_spans,
        decoration_spans: &decoration_spans,
        ..input(text, &underlined)
    };
    let ast = compile_paragraph(&paragraph_input, &FrontendOptions::default(), &*SHAPER)
        .expect("compiles");
    let p = layout_paragraph(
        &ast,
        text,
        &*SHAPER,
        &column(200.0),
        &LayoutOptions::default(),
    )
    .expect("lays out");
    ValidatedLayout::new(&p, text).expect("validates");
    assert_eq!(p.lines.len(), 1);
    assert_eq!(
        p.lines[0].decorations.len(),
        1,
        "expected one continuous underline, got {:?}",
        p.lines[0].decorations
    );
    let underline = &p.lines[0].decorations[0];
    assert_eq!(underline.kind, DecorationKind::Underline);
    assert_eq!(underline.byte_range, TextRange::new(0, 8));
    assert_close(underline.width, 48.0);
}

#[test]
fn styled_runs_lay_out_with_the_tallest_line_box() {
    let text = "small BIG small";
    let base = style(10.0);
    let spans = [StyleSpan {
        byte_range: TextRange::new(6, 9),
        style: style(20.0),
    }];
    let input = ParagraphInput {
        style_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&input, &FrontendOptions::default(), &*SHAPER).unwrap();
    let p = layout_paragraph(
        &ast,
        text,
        &*SHAPER,
        &column(1000.0),
        &LayoutOptions::default(),
    )
    .unwrap();
    ValidatedLayout::new(&p, text).unwrap();
    assert_close(p.lines[0].ascent, 20.4);
    assert_close(p.lines[0].visual_width(), 6.0 * 12.0 + 12.0 * 3.0);
}

/// Two slots around an obstacle on every line, and an obstruction below
/// the first two lines.
#[derive(Debug)]
struct Obstacle;

impl LineGeometryProvider for Obstacle {
    fn query_line_geometry(&self, _: usize, block_start: f32, _: f32) -> LineGeometry {
        if (26.0..40.0).contains(&block_start) {
            return LineGeometry {
                slots: Default::default(),
                next_feature_block_offset: Some(40.0),
            };
        }
        LineGeometry {
            slots: [
                InlineSlotConstraint::new(0.0, 40.0).unwrap(),
                InlineSlotConstraint::new(60.0, 100.0).unwrap(),
            ]
            .into_iter()
            .collect(),
            next_feature_block_offset: None,
        }
    }
}

#[test]
fn a_provider_fills_the_first_slot_and_clears_obstructions() {
    let text = "aaa bbb ccc ddd eee";
    let constraints = LayoutConstraints {
        geometry_provider: Some(std::sync::Arc::new(Obstacle)),
        ..column(100.0)
    };
    let p = lay_out(text, &constraints, &LayoutOptions::default());
    assert_eq!(
        line_texts(&p, text),
        ["aaa ", "bbb ", "ccc ", "ddd ", "eee"]
    );
    for line in &p.lines {
        assert_eq!(line.slots.len(), 2);
        assert_eq!(line.slots[1].visual_width, 0.0);
    }
    // The third line starts below the obstruction.
    assert_close(p.lines[2].baseline_y - p.lines[2].ascent, 40.0);
}

#[test]
fn a_run_of_exclamation_marks_splits_only_once_it_starts_an_empty_line() {
    // RM-4. The run must be a unit of its own to reach an empty line: at
    // the paragraph start or after a hard break. Every glyph is 6 wide and
    // the column holds five.
    let column = column(30.0);
    let options = LayoutOptions::default();
    for (text, expected) in [
        ("!!!!!!!!", &["!!!!!", "!!!"][..]),
        ("ab\u{2028}!!!!!!!!", &["ab\u{2028}", "!!!!!", "!!!"][..]),
        // A word with the marks fused to it moves whole, then splits.
        ("ab cd!!!!!!", &["ab ", "cd!!!", "!!!"][..]),
        // UAX #14 LB13 (× EX) leaves no opportunity before `!`, after a
        // space (SP × EX) or a letter (AL × EX) alike: the paragraph is one
        // unit, already at the start of an empty line, and splits where it
        // overflows.
        ("ab !!!!!!", &["ab !!", "!!!!"][..]),
        ("ab!!!!!!", &["ab!!!", "!!!"][..]),
    ] {
        let p = lay_out(text, &column, &options);
        assert_eq!(line_texts(&p, text), expected, "{text:?}");
    }
}

#[test]
fn closing_punctuation_never_starts_a_line_while_its_unit_fits() {
    // RM-4, WN-3: CL, CP, EX, IS and SY are fused to what precedes them.
    let text = "(one) two, three. four! five; six? seven/eight";
    let longest = text.split(' ').map(str::len).max().unwrap() as f32 * 6.0;
    let mut width = longest;
    while width <= 300.0 {
        let p = lay_out(text, &column(width), &LayoutOptions::default());
        for line in line_texts(&p, text) {
            let first = line.chars().next().unwrap();
            assert!(!")]},.!;?/".contains(first), "{width}: {line:?}");
        }
        width += 6.0;
    }
}
