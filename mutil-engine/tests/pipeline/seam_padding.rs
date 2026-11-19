//! Cross-stream invariants of layout (ADR-0004): cloned padding charged per
//! span by its own nesting (ST2-3, WN-21), and the re-shaping of a seam that
//! is not safe to break, past a decoration span edge (ST1-5, ME-2).

use crate::common::*;
use mutil_engine::BoxDecorationBreak::{Clone as Cl, Slice as Sl};
use mutil_engine::*;

fn span(
    span_id: u64,
    range: (usize, usize),
    padding: f32,
    break_mode: BoxDecorationBreak,
) -> InlineDecorationSpan {
    InlineDecorationSpan {
        span_id,
        byte_range: TextRange::new(range.0, range.1),
        start_padding: padding,
        end_padding: padding,
        break_mode,
    }
}

/// `text` in the monospace face (6 units a character) with `spans`, laid
/// out `width` wide through the output gate.
#[track_caller]
fn lay_out_spans(text: &str, spans: &[InlineDecorationSpan], width: f32) -> PositionedParagraph {
    let base = style(10.0);
    let paragraph = ParagraphInput {
        decoration_spans: spans,
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
    p
}

const TEXT: &str = "aa bb cc dd ee ff";
const OUTER: (usize, usize) = (0, 17);
const INNER: (usize, usize) = (3, 14);

/// The lines that lie strictly inside the inner span, so that both of their
/// edges are breaks inside both spans and carry both spans' cloning.
fn inner_lines(p: &PositionedParagraph) -> Vec<&PositionedLine> {
    let inside: Vec<_> = p
        .lines
        .iter()
        .filter(|l| l.byte_range.start() > INNER.0 && l.byte_range.end() < INNER.1)
        .collect();
    assert!(!inside.is_empty(), "{:#?}", line_texts(p, TEXT));
    inside
}

fn fragments_of(line: &PositionedLine, span_id: u64) -> Vec<&PositionedSpanFragment> {
    line.span_fragments
        .iter()
        .filter(|f| f.span_id == span_id)
        .collect()
}

#[test]
fn st2_3_nested_clone_spans_lay_their_padding_out_from_the_edge_inwards() {
    let spans = [span(1, OUTER, 10.0, Cl), span(2, INNER, 4.0, Cl)];
    let p = lay_out_spans(TEXT, &spans, 50.0);
    for line in inner_lines(&p) {
        let what = &TEXT[line.byte_range.as_range()];
        let slot = &line.slots[0];
        let outer = fragments_of(line, 1);
        let inner = fragments_of(line, 2);
        // The outer span is one piece from edge to edge: no hole where the
        // inner span's padding lies.
        assert_eq!(outer.len(), 1, "{what:?}: {outer:?}");
        assert_eq!(inner.len(), 1, "{what:?}: {inner:?}");
        let (outer, inner) = (outer[0], inner[0]);
        assert_close(outer.x, slot.content_start);
        assert_close(outer.x + outer.width, slot.content_end());
        // The inner span starts at the outer padding's inner edge and ends
        // where the outer padding begins.
        assert_close(inner.x, slot.content_start + 10.0);
        assert_close(inner.x + inner.width, slot.content_end() - 10.0);
        // Each edge holds both paddings: 10 + 4 on either side of the text.
        let text_width = 6.0 * what.trim_end().chars().count() as f32;
        assert_close(slot.visual_width, text_width + 2.0 * 14.0);
    }
}

#[test]
fn st2_3_a_slice_span_covers_the_cloned_padding_inside_it_but_adds_none() {
    let spans = [span(1, OUTER, 10.0, Sl), span(2, INNER, 4.0, Cl)];
    let p = lay_out_spans(TEXT, &spans, 50.0);
    for line in inner_lines(&p) {
        let what = &TEXT[line.byte_range.as_range()];
        let slot = &line.slots[0];
        let outer = fragments_of(line, 1);
        let inner = fragments_of(line, 2);
        assert_eq!(outer.len(), 1, "{what:?}: {outer:?}");
        assert_eq!(inner.len(), 1, "{what:?}: {inner:?}");
        let (outer, inner) = (outer[0], inner[0]);
        // Only the inner span's padding is cloned at the line's edges, and
        // the outer fragment covers it.
        let text_width = 6.0 * what.trim_end().chars().count() as f32;
        assert_close(slot.visual_width, text_width + 2.0 * 4.0);
        for f in [outer, inner] {
            assert_close(f.x, slot.content_start);
            assert_close(f.x + f.width, slot.content_end());
        }
    }
}

/// The glyph ids of `line`, left to right, with the source bytes of each.
fn glyphs_of(line: &PositionedLine) -> Vec<(u32, Option<TextRange>)> {
    line.visual_runs
        .iter()
        .flat_map(|r| r.glyphs.iter().map(|g| (g.glyph_id, g.byte_range)))
        .collect()
}

fn ids(glyphs: &[(u32, Option<TextRange>)]) -> Vec<u32> {
    glyphs.iter().map(|g| g.0).collect()
}

/// `text` in the Arabic face with `spans`, laid out `width` wide.
#[track_caller]
fn lay_out_arabic(text: &str, spans: &[InlineDecorationSpan], width: f32) -> PositionedParagraph {
    let base = script_style(&[ARABIC], 10.0);
    let paragraph = ParagraphInput {
        decoration_spans: spans,
        ..input(text, &base)
    };
    let ast = compile_bidi_input(&paragraph, &FrontendOptions::default());
    lay_out_bidi_ast(&ast, text, &column(width), &LayoutOptions::default())
}

/// Every line of `p` carries the glyphs its own text shapes to alone.
#[track_caller]
fn assert_lines_shaped_alone(text: &str, p: &PositionedParagraph) {
    for line in &p.lines {
        let own = &text[line.byte_range.as_range()];
        let alone = lay_out_arabic(own, &[], 10_000.0);
        assert_eq!(alone.lines.len(), 1);
        assert_eq!(
            ids(&glyphs_of(line)),
            ids(&glyphs_of(&alone.lines[0])),
            "{own:?} in {:?}",
            line_texts(p, text)
        );
    }
}

#[test]
fn st1_5_me2_an_emergency_seam_in_a_joining_word_re_shapes_both_sides() {
    // Six dual-joining letters: in the whole word the first four take
    // initial and medial forms, alone they end in a final form.
    let text = "\u{628}\u{628}\u{628}\u{628}\u{628}\u{628}";
    let whole = lay_out_arabic(text, &[], 10_000.0);
    let width: f32 = whole.lines[0].slots[0].visual_width;
    let p = lay_out_arabic(text, &[], width * 0.7);
    assert_eq!(p.lines.len(), 2, "{:?}", line_texts(&p, text));
    let seam = p.lines[0].byte_range.end();
    // The test discriminates: the first line's glyphs in the whole word
    // differ from the first line's text shaped alone.
    let in_word: Vec<_> = glyphs_of(&whole.lines[0])
        .into_iter()
        .filter(|g| g.1.is_some_and(|r| r.end() <= seam))
        .collect();
    let alone = lay_out_arabic(&text[..seam], &[], 10_000.0);
    assert_ne!(ids(&in_word), ids(&glyphs_of(&alone.lines[0])));
    assert_lines_shaped_alone(text, &p);
}

#[test]
fn me2_a_span_edge_at_an_unsafe_seam_does_not_stop_the_re_shape() {
    let text = "\u{628}\u{628}\u{628}\u{628}\u{628}\u{628}";
    let whole = lay_out_arabic(text, &[], 10_000.0);
    let width: f32 = whole.lines[0].slots[0].visual_width;
    let plain = lay_out_arabic(text, &[], width * 0.7);
    let seam = plain.lines[0].byte_range.end();
    // A span edge exactly at the seam, without padding (so the seam stays),
    // and a span ending there, in both break modes.
    for mode in [Sl, Cl] {
        for spans in [
            vec![span(1, (seam, text.len()), 0.0, mode)],
            vec![span(1, (0, seam), 0.0, mode)],
            vec![
                span(1, (0, seam), 0.0, mode),
                span(2, (seam, text.len()), 0.0, mode),
            ],
        ] {
            let p = lay_out_arabic(text, &spans, width * 0.7);
            assert_eq!(
                p.lines[0].byte_range.end(),
                seam,
                "{mode:?} {spans:?}: {:?}",
                line_texts(&p, text)
            );
            assert_lines_shaped_alone(text, &p);
        }
    }
}
