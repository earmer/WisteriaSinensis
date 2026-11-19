//! Hanging punctuation (spec ST1-3, HNG-1 to HNG-4, OUT-4, OPT-4): 0.85 of
//! the mark's advance protrudes, `Allow` hangs only when needed, `Force` always,
//! `FirstLine` only on line 0, the line end of an RTL paragraph is its left
//! side, and a mark off the base level or behind padding does not hang.
//! Every layout passes the output gate (the `lay_out*` helpers).

use crate::acceptance_phase2::{glyph_at, glyphs};
use crate::common::*;
use crate::justification::lay_out_mono;
use mutil_engine::*;

/// The part of a hanging mark's advance that protrudes (ST1-3).
const FRACTION: f32 = 0.85;
/// The monospace advance at 10 units per em, and how far a mark hangs.
const CHAR: f32 = 6.0;
const HANG: f32 = CHAR * FRACTION;

fn hang(start: HangStartMode, end: HangEndMode) -> LayoutOptions {
    LayoutOptions {
        hang_mode: HangMode { start, end },
        ..Default::default()
    }
}

fn hang_end(end: HangEndMode) -> LayoutOptions {
    hang(HangStartMode::None, end)
}

fn hang_start(start: HangStartMode) -> LayoutOptions {
    hang(start, HangEndMode::None)
}

/// The byte offsets of `line`'s hanging glyphs.
fn hanging(line: &PositionedLine) -> Vec<usize> {
    line.hanging_glyphs()
        .map(|g| g.byte_range.unwrap().start())
        .collect()
}

#[track_caller]
fn assert_no_hang(line: &PositionedLine) {
    assert_eq!(hanging(line), [] as [usize; 0]);
    assert_eq!((line.hanging_left(), line.hanging_right()), (0.0, 0.0));
}

#[test]
fn st1_3_force_hangs_a_stop_at_the_line_end() {
    // "aaaa bbb. " is exactly 54 wide in a 54-unit column.
    let text = "aaaa bbb. cccc";
    let p = lay_out(text, &column(54.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["aaaa bbb. ", "cccc"]);
    assert_no_hang(&p.lines[0]);
    assert_close(p.lines[0].visual_width(), 54.0);
    let p = lay_out(text, &column(54.0), &hang_end(HangEndMode::Force));
    assert_eq!(line_texts(&p, text), ["aaaa bbb. ", "cccc"]);
    let line = &p.lines[0];
    assert_eq!(hanging(line), [8]);
    // The body is measured with the inside part only.
    assert_close(line.visual_width(), 54.0 - HANG);
    assert_close(line.hanging_right(), HANG);
    assert_close(line.hanging_left(), 0.0);
    let stop = glyph_at(line, 8);
    assert_close(stop.x, 8.0 * CHAR);
    assert_close(stop.x + stop.advance - content_end(line), HANG);
    // `Force` hangs even where the line has room, the last line included,
    // and a comma like a full stop.
    for text in ["aaa bbb.", "aaa bbb,"] {
        let p = lay_out(text, &column(100.0), &hang_end(HangEndMode::Force));
        let line = &p.lines[0];
        assert_eq!(hanging(line), [7], "{text}");
        assert_close(line.hanging_right(), HANG);
        assert_close(line.visual_width(), 8.0 * CHAR - HANG);
    }
    // Nothing else hangs at the end.
    for text in ["aaa bbb!", "aaa bbb)", "aaa bbb;"] {
        let p = lay_out(text, &column(100.0), &hang_end(HangEndMode::Force));
        assert_no_hang(&p.lines[0]);
    }
}

fn content_end(line: &PositionedLine) -> f32 {
    line.slots[0].content_end()
}

#[test]
fn st1_3_allow_hangs_a_stop_only_when_the_line_would_not_fit_otherwise() {
    let text = "aaaa bbb. cccc";
    // 54 fits 60: nothing hangs.
    let p = lay_out(text, &column(60.0), &hang_end(HangEndMode::Allow));
    assert_eq!(line_texts(&p, text), ["aaaa bbb. ", "cccc"]);
    assert_no_hang(&p.lines[0]);
    assert_close(p.lines[0].visual_width(), 54.0);
    // In 52 units "bbb." fits only by hanging its stop.
    let p = lay_out(text, &column(52.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["aaaa ", "bbb. ", "cccc"]);
    let p = lay_out(text, &column(52.0), &hang_end(HangEndMode::Allow));
    assert_eq!(line_texts(&p, text), ["aaaa bbb. ", "cccc"]);
    let line = &p.lines[0];
    assert_eq!(hanging(line), [8]);
    assert_close(line.hanging_right(), HANG);
    assert_close(line.visual_width(), 54.0 - HANG);
    // A start mode never hangs a stop.
    let p = lay_out(text, &column(52.0), &hang_start(HangStartMode::AllLines));
    assert_eq!(line_texts(&p, text), ["aaaa ", "bbb. ", "cccc"]);
    assert!(p.lines.iter().all(|l| hanging(l).is_empty()));
}

#[test]
fn st1_3_an_opening_mark_hangs_at_the_line_start_by_start_mode() {
    // "(aa bbb " and "(cc ddd": each line opens with a parenthesis.
    let text = "(aa bbb (cc ddd";
    for (mode, hangs) in [
        (HangStartMode::None, [false, false]),
        (HangStartMode::FirstLine, [true, false]),
        (HangStartMode::AllLines, [true, true]),
    ] {
        let p = lay_out(text, &column(45.0), &hang_start(mode));
        assert_eq!(line_texts(&p, text), ["(aa bbb ", "(cc ddd"], "{mode:?}");
        for (line, (hangs, opener)) in p.lines.iter().zip(hangs.into_iter().zip([0, 8])) {
            let paren = glyph_at(line, opener);
            if hangs {
                assert_eq!(hanging(line), [opener], "{mode:?}");
                assert_close(line.hanging_left(), HANG);
                assert_close(line.slots[0].content_start - paren.x, HANG);
                assert_close(line.visual_width(), 7.0 * CHAR - HANG);
            } else {
                assert_no_hang(line);
                assert_close(paren.x, line.slots[0].content_start);
                assert_close(line.visual_width(), 7.0 * CHAR);
            }
            assert_close(line.hanging_right(), 0.0);
        }
    }
    // A line after a hard break is not a first line.
    let text = "(aa\u{2028}(bb";
    let p = lay_out(text, &column(100.0), &hang_start(HangStartMode::FirstLine));
    assert_eq!(hanging(&p.lines[0]), [0]);
    assert_no_hang(&p.lines[1]);
    // An end mode never hangs an opener.
    let p = lay_out(text, &column(100.0), &hang_end(HangEndMode::Force));
    assert!(p.lines.iter().all(|l| hanging(l).is_empty()));
}

#[test]
fn st1_3_the_line_end_of_an_rtl_paragraph_is_its_left_side() {
    let text = "אבג דהו.";
    let p = lay_out_bidi(
        text,
        &[HEBREW],
        &column(100.0),
        &hang_end(HangEndMode::Force),
    );
    assert_eq!(p.base_direction, Direction::Rtl);
    let line = &p.lines[0];
    let stop = text.find('.').unwrap();
    assert_eq!(hanging(line), [stop]);
    let mark = glyph_at(line, stop);
    assert_close(line.hanging_left(), mark.advance * FRACTION);
    assert_close(line.hanging_right(), 0.0);
    // The mark is the leftmost glyph and straddles the content's left edge.
    assert!(glyphs(line).all(|g| g.x >= mark.x - 1e-3));
    assert_close(
        line.slots[0].content_start - mark.x,
        mark.advance * FRACTION,
    );
    // The start edge is the right side: an opener hangs there.
    let text = "(אבג) דהו";
    let p = lay_out_bidi(
        text,
        &[HEBREW],
        &column(100.0),
        &hang_start(HangStartMode::AllLines),
    );
    let line = &p.lines[0];
    assert_eq!(hanging(line), [0]);
    let opener = glyph_at(line, 0);
    assert_close(line.hanging_right(), opener.advance * FRACTION);
    assert_close(line.hanging_left(), 0.0);
    assert_close(
        opener.x + opener.advance - content_end(line),
        opener.advance * FRACTION,
    );
}

/// The natural width of the text boxes and glue of `ast` that end by `end`.
fn natural_before(ast: &ParagraphAst, end: usize) -> f32 {
    ast.items
        .iter()
        .filter(|item| item.byte_span().end() <= end)
        .map(|item| match item {
            WordNode::Box(BoxNode::Text { natural_width, .. }) => *natural_width,
            WordNode::Glue(g) => g.natural_width,
            _ => 0.0,
        })
        .sum()
}

#[test]
fn st1_3_a_mark_off_the_base_level_does_not_hang() {
    // In an LTR paragraph, "." between two Hebrew words is R (level 1); before
    // a Latin word it takes the embedding direction (level 0).
    for (text, base_level) in [("abc אבג. דהו", false), ("abc אבג. def", true)] {
        let ast = compile_bidi(text, &[HEBREW]);
        let stop = text.find('.').unwrap();
        assert_eq!(
            crate::acceptance_phase2::level_at(&ast, stop) == 0,
            base_level,
            "{text}"
        );
        let width = natural_before(&ast, stop + 1);
        let p = lay_out_bidi_ast(
            &ast,
            text,
            &column(width + 1.0),
            &hang_end(HangEndMode::Force),
        );
        assert_eq!(line_texts(&p, text)[0], &text[..stop + 2], "{text}");
        let line = &p.lines[0];
        if base_level {
            assert_eq!(hanging(line), [stop], "{text}");
            assert!(line.hanging_right() > 0.0);
        } else {
            assert_no_hang(line);
        }
    }
}

/// Lays out "aaaa bbb. cccc" in a 60-unit column under `Force` with one
/// decoration span over `range`, and checks that line 1 is "aaaa bbb. ".
fn lay_out_span(
    range: (usize, usize),
    end_padding: f32,
    mode: BoxDecorationBreak,
) -> PositionedParagraph {
    let text = "aaaa bbb. cccc";
    let style = style(10.0);
    let spans = [InlineDecorationSpan {
        span_id: 1,
        byte_range: TextRange::new(range.0, range.1),
        start_padding: 0.0,
        end_padding,
        break_mode: mode,
    }];
    let paragraph = ParagraphInput {
        decoration_spans: &spans,
        ..input(text, &style)
    };
    let p = lay_out_mono(&paragraph, &column(60.0), &hang_end(HangEndMode::Force));
    assert_eq!(line_texts(&p, text), ["aaaa bbb. ", "cccc"]);
    p
}

#[test]
fn st1_3_a_padded_span_boundary_after_the_mark_prevents_hanging() {
    // A span ending right after the stop: its trailing boundary, 2 wide,
    // lies between the mark and the line end.
    let p = lay_out_span((5, 9), 2.0, BoxDecorationBreak::Slice);
    assert_no_hang(&p.lines[0]);
    assert_close(p.lines[0].visual_width(), 9.0 * CHAR + 2.0);
}

#[test]
fn st1_3_a_zero_width_span_boundary_after_the_mark_does_not_prevent_hanging() {
    // The same span without padding: the mark hangs, and the fragment stops
    // at the content block's edge (ST5-1).
    let p = lay_out_span((5, 9), 0.0, BoxDecorationBreak::Slice);
    let line = &p.lines[0];
    assert_eq!(hanging(line), [8]);
    let [fragment] = line.span_fragments.as_slice() else {
        panic!("{:?}", line.span_fragments)
    };
    assert_close(fragment.x + fragment.width, content_end(line));
}

#[test]
fn st1_3_cloned_padding_at_the_line_end_prevents_hanging() {
    // A `Clone` span that wraps after the stop repeats its end padding at
    // the end of line 1, after the mark.
    let p = lay_out_span((5, 14), 2.0, BoxDecorationBreak::Clone);
    assert_no_hang(&p.lines[0]);
    assert_close(p.lines[0].visual_width(), 9.0 * CHAR + 2.0);
}

#[test]
fn st1_3_a_sliced_span_across_the_line_end_does_not_prevent_hanging() {
    // `Slice` puts no padding at the wrap: the mark hangs, and the
    // fragment stops at the content block's edge (ST5-1).
    let p = lay_out_span((5, 14), 2.0, BoxDecorationBreak::Slice);
    let line = &p.lines[0];
    assert_eq!(hanging(line), [8]);
    assert_close(line.hanging_right(), HANG);
    let [fragment] = line.span_fragments.as_slice() else {
        panic!("{:?}", line.span_fragments)
    };
    assert_close(fragment.x + fragment.width, content_end(line));
}

#[test]
fn st1_3_cjk_stops_and_openers_hang_by_the_hang_fraction() {
    // "「漢字漢字。" with a full-width opener and stop, both hanging on line 0
    // under `AllLines` and `Force`.
    let text = "「漢字漢字。";
    let options = hang(HangStartMode::AllLines, HangEndMode::Force);
    let p = lay_out_bidi(text, &[JAPANESE], &column(100.0), &options);
    let line = &p.lines[0];
    assert_eq!(hanging(line), [0, 15]);
    assert_close(line.hanging_left(), 10.0 * FRACTION);
    assert_close(line.hanging_right(), 10.0 * FRACTION);
    assert_close(line.visual_width(), 60.0 - 20.0 * FRACTION);
}

#[test]
fn st5_1_an_underline_stops_at_the_content_edge_before_a_hanging_mark() {
    let text = "aaaa bbb.";
    let style = TextStyle {
        decoration: TextDecoration {
            underline: true,
            strikethrough: false,
        },
        ..style(10.0)
    };
    let p = lay_out_mono(
        &input(text, &style),
        &column(100.0),
        &hang_end(HangEndMode::Force),
    );
    let line = &p.lines[0];
    assert_eq!(hanging(line), [8]);
    assert!(!line.decorations.is_empty());
    let right = line
        .decorations
        .iter()
        .map(|d| d.x + d.width)
        .fold(f32::NEG_INFINITY, f32::max);
    assert_close(right, content_end(line));
}

/// Compiles `text` in Simplified Chinese at 10 units per em with
/// [`SPACING_SHAPER`].
#[track_caller]
fn compile_zh(text: &str) -> ParagraphAst {
    let style = localized_style(&[SIMPLIFIED], 10.0, "zh-CN");
    compile_paragraph(
        &input(text, &style),
        &FrontendOptions::default(),
        &*SPACING_SHAPER,
    )
    .unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

/// Lays out `ast` with [`SPACING_SHAPER`] through the output gate.
#[track_caller]
fn lay_out_zh(
    ast: &ParagraphAst,
    text: &str,
    wrap_width: f32,
    options: &LayoutOptions,
) -> PositionedParagraph {
    let p = layout_paragraph(ast, text, &*SPACING_SHAPER, &column(wrap_width), options)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&p, text) {
        panic!("{text:?} in {wrap_width}, {options:?}: line {line}: {error}\n{p:#?}");
    }
    p
}

#[test]
fn st1_3_hng2_allow_hangs_a_fused_cjk_stop_only_when_its_protrusion_overflows() {
    for (text, mark) in [("都在这里。总结", '。'), ("时间校验，确保", '，')] {
        let ast = compile_zh(text);
        let stop = text.find(mark).unwrap();
        let end = stop + mark.len_utf8();
        let natural = crate::acceptance_phase2::width_of(&ast, TextRange::new(0, end));
        let mark_width = crate::acceptance_phase2::width_of(&ast, TextRange::new(stop, end));
        assert!(mark_width > 0.0, "{text}");
        // The column holds the line's body plus the inside part of the
        // mark, but not the whole mark.
        let width = natural - mark_width * FRACTION / 2.0;
        let wrapped = lay_out_zh(&ast, text, width, &LayoutOptions::default());
        let lines = line_texts(&wrapped, text);
        assert!(
            !lines[0].ends_with(mark),
            "{text}: without hanging the stop wraps: {lines:?}"
        );
        // The stop is fused to its ideograph: it never starts a line alone.
        assert!(lines.iter().all(|l| !l.starts_with(mark)), "{lines:?}");
        assert!(wrapped.lines.iter().all(|l| hanging(l).is_empty()));
        let p = lay_out_zh(&ast, text, width, &hang_end(HangEndMode::Allow));
        let lines = line_texts(&p, text);
        assert_eq!(lines[0], &text[..end], "{text}: {lines:?}");
        let line = &p.lines[0];
        assert_eq!(hanging(line), [stop], "{text}");
        let glyph = glyph_at(line, stop);
        assert_close(line.hanging_right(), glyph.advance * FRACTION);
        // HNG-1: 0.85 protrudes, 0.15 lies inside the content block.
        assert_close(
            glyph.x + glyph.advance - content_end(line),
            glyph.advance * FRACTION,
        );
        assert_close(
            content_end(line) - glyph.x,
            glyph.advance * (1.0 - FRACTION),
        );
        assert_close(line.visual_width(), natural - glyph.advance * FRACTION);
        // `Allow` in a column that holds the whole mark hangs nothing.
        let p = lay_out_zh(&ast, text, natural + 1.0, &hang_end(HangEndMode::Allow));
        assert_eq!(line_texts(&p, text)[0], &text[..end], "{text}");
        assert_no_hang(&p.lines[0]);
    }
}

#[test]
fn st1_3_the_default_hang_mode_hangs_nothing() {
    assert_eq!(
        HangMode::default(),
        HangMode {
            start: HangStartMode::None,
            end: HangEndMode::None
        }
    );
    let text = "「漢字漢字。」「漢字，";
    for width in [35.0, 45.0, 1000.0] {
        let p = lay_out_bidi(text, &[JAPANESE], &column(width), &LayoutOptions::default());
        for line in &p.lines {
            assert_no_hang(line);
            let (start, end) = (line.slots[0].content_start, content_end(line));
            for g in glyphs(line) {
                let (a, b) = (g.x.min(g.x + g.advance), g.x.max(g.x + g.advance));
                assert!(
                    a >= start - 1e-3 && b <= end + 1e-3,
                    "{width}: {g:?} outside [{start}, {end}]"
                );
            }
        }
    }
}

#[test]
fn st1_3_full_width_cjk_marks_hang_their_ink_half_outside_the_column() {
    let text = "「漢字漢字。";
    let options = hang(HangStartMode::AllLines, HangEndMode::Force);
    let p = lay_out_bidi(text, &[JAPANESE], &column(100.0), &options);
    let line = &p.lines[0];
    assert_eq!(hanging(line), [0, 15]);
    let (start, end) = (line.slots[0].content_start, content_end(line));
    let opener = glyph_at(line, 0);
    let stop = glyph_at(line, 15);
    // More than half of each mark lies outside: its ink half with it.
    assert!(line.hanging_left() > opener.advance / 2.0);
    assert!(line.hanging_right() > stop.advance / 2.0);
    assert_close(start - opener.x, opener.advance * FRACTION);
    assert_close(
        opener.x + opener.advance - start,
        opener.advance * (1.0 - FRACTION),
    );
    assert_close(stop.x + stop.advance - end, stop.advance * FRACTION);
    assert_close(end - stop.x, stop.advance * (1.0 - FRACTION));
    // The neighbouring ideographs sit 0.15 em inside the edges.
    let em = 10.0;
    let first = glyph_at(line, 3);
    let last = glyph_at(line, 12);
    assert_close(first.x - start, em * (1.0 - FRACTION));
    assert_close(end - (last.x + last.advance), em * (1.0 - FRACTION));
}
