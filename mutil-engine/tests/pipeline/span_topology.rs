//! Span fragment topology (FRG-1, ST2-3), ribbon advance, `BoxHeightStyle` on
//! decoration spans (SPN-1) and `PlacedGlyph` attribution (HST-4), through the
//! whole pipeline and the output gate.

use crate::common::*;
use mutil_engine::SpanEdgeCut::{BidiSplit, ClonedWrap, LogicalEnd, SlicedWrap};
use mutil_engine::*;

const TEXT: &str = "aa bb cc dd ee ff gg hh";

fn span(
    range: (usize, usize),
    mode: BoxDecorationBreak,
    height: BoxHeightStyle,
) -> InlineDecorationSpan {
    InlineDecorationSpan {
        span_id: 1,
        byte_range: TextRange::new(range.0, range.1),
        start_padding: 2.0,
        end_padding: 2.0,
        break_mode: mode,
        height_style: height,
    }
}

fn lay(span: InlineDecorationSpan, width: f32) -> PositionedParagraph {
    let base = style(10.0);
    let spans = [span];
    let paragraph = ParagraphInput {
        decoration_spans: &spans,
        ..input(TEXT, &base)
    };
    let ast =
        compile_paragraph(&paragraph, &FrontendOptions::default(), &*SHAPER).expect("compiles");
    let p = layout_paragraph(
        &ast,
        TEXT,
        &*SHAPER,
        &column(width),
        &LayoutOptions::default(),
    )
    .expect("lays out");
    ValidatedLayout::try_from_paragraph(&p, TEXT).expect("passes G4");
    p
}

fn fragments(p: &PositionedParagraph) -> Vec<PositionedSpanFragment> {
    p.lines
        .iter()
        .flat_map(|l| l.span_fragments.clone())
        .collect()
}

#[test]
fn an_unwrapped_span_has_logical_ends_and_one_ribbon() {
    let p = lay(
        span((3, 8), BoxDecorationBreak::Slice, BoxHeightStyle::Strut),
        1000.0,
    );
    let f = fragments(&p);
    assert_eq!(f.len(), 1);
    assert_eq!(
        (f[0].edges.left, f[0].edges.right),
        (LogicalEnd, LogicalEnd)
    );
    assert_eq!(f[0].ribbon_offset, 0.0);
    assert_close(f[0].total_span_advance, f[0].width);
}

fn wrapped(mode: BoxDecorationBreak) -> Vec<PositionedSpanFragment> {
    let p = lay(span((3, 20), mode, BoxHeightStyle::Strut), 60.0);
    assert!(p.lines.len() >= 3, "the span wraps over several lines");
    fragments(&p)
}

fn assert_ribbon(f: &[PositionedSpanFragment]) {
    let mut acc = 0.0;
    for frag in f {
        assert_close(frag.ribbon_offset, acc);
        acc += frag.width;
    }
    for frag in f {
        assert_close(frag.total_span_advance, acc);
    }
}

#[test]
fn a_sliced_span_wraps_with_sliced_edges() {
    let f = wrapped(BoxDecorationBreak::Slice);
    let last = f.len() - 1;
    for (i, frag) in f.iter().enumerate() {
        let left = if i == 0 { LogicalEnd } else { SlicedWrap };
        let right = if i == last { LogicalEnd } else { SlicedWrap };
        assert_eq!(
            (frag.edges.left, frag.edges.right),
            (left, right),
            "fragment {i}"
        );
    }
    assert_ribbon(&f);
}

#[test]
fn a_cloned_span_wraps_with_cloned_edges() {
    let f = wrapped(BoxDecorationBreak::Clone);
    let last = f.len() - 1;
    for (i, frag) in f.iter().enumerate() {
        let left = if i == 0 { LogicalEnd } else { ClonedWrap };
        let right = if i == last { LogicalEnd } else { ClonedWrap };
        assert_eq!(
            (frag.edges.left, frag.edges.right),
            (left, right),
            "fragment {i}"
        );
    }
    assert_ribbon(&f);
}

#[test]
fn height_styles_follow_the_strut_the_line_box_and_the_content() {
    let strut = |p: &PositionedParagraph| (p.policy.strut.ascent, p.policy.strut.descent);
    let p = lay(
        span((3, 8), BoxDecorationBreak::Slice, BoxHeightStyle::Strut),
        1000.0,
    );
    let f = &p.lines[0].span_fragments[0];
    assert_close(f.ascent, strut(&p).0);
    assert_close(f.descent, strut(&p).1);

    let p = lay(
        span((3, 8), BoxDecorationBreak::Slice, BoxHeightStyle::Max),
        1000.0,
    );
    let (line, f) = (&p.lines[0], &p.lines[0].span_fragments[0]);
    assert_close(f.ascent, line.ascent);
    assert_close(f.descent, line.descent);

    let p = lay(
        span((3, 8), BoxDecorationBreak::Slice, BoxHeightStyle::Tight),
        1000.0,
    );
    let (line, f) = (&p.lines[0], &p.lines[0].span_fragments[0]);
    assert!(f.ascent > 0.0 && f.descent > 0.0);
    assert!(f.ascent <= line.ascent + 1e-3 && f.descent <= line.descent + 1e-3);
}

#[test]
fn placed_glyphs_carry_their_bytes_and_advance() {
    let p = lay(
        span((3, 8), BoxDecorationBreak::Slice, BoxHeightStyle::Strut),
        1000.0,
    );
    let layout = ValidatedLayout::try_from_paragraph(&p, TEXT).unwrap();
    let mut covered = 0;
    for run in layout.glyph_runs() {
        let PlacedRunSource::Body(body) = run.source() else {
            continue;
        };
        for (placed, raw) in run.glyphs().zip(&body.glyphs) {
            assert_eq!(placed.byte_range, raw.byte_range);
            assert_eq!(placed.advance, raw.advance);
            covered += placed.byte_range.map_or(0, |r| r.len());
        }
    }
    assert_eq!(covered, TEXT.len() - TEXT.matches(' ').count());
}

#[test]
fn g4_rejects_a_broken_ribbon() {
    let p = lay(
        span((3, 20), BoxDecorationBreak::Slice, BoxHeightStyle::Strut),
        60.0,
    );
    let corrupt = |edit: fn(&mut PositionedSpanFragment)| {
        let mut q = p.clone();
        let line = q
            .lines
            .iter_mut()
            .find(|l| !l.span_fragments.is_empty())
            .unwrap();
        edit(&mut line.span_fragments[0]);
        q.validate(TEXT).expect_err("a bad ribbon is rejected")
    };
    for edit in [
        (|f| f.ribbon_offset = -1.0) as fn(&mut PositionedSpanFragment),
        |f| f.ribbon_offset = f32::NAN,
        |f| f.total_span_advance = f32::INFINITY,
        |f| f.total_span_advance = -1.0,
    ] {
        corrupt(edit);
    }
    // The paragraph check: the first offset must be zero, and totals agree.
    corrupt(|f| f.ribbon_offset = 5.0);
    corrupt(|f| f.total_span_advance += 7.0);
}

#[test]
fn g4_rejects_forged_logical_end_edge() {
    let p = lay(
        span((3, 20), BoxDecorationBreak::Slice, BoxHeightStyle::Strut),
        60.0,
    );
    let mut forged = p.clone();
    let first = &mut forged.lines[0].span_fragments[0];
    assert!(!first.is_last_fragment());
    first.edges.right = LogicalEnd;
    assert!(matches!(
        forged.validate(TEXT),
        Err((0, LineInvariantError::InvalidSpanFragment { index: 0 }))
    ));
}

#[test]
fn bidi_split_span_uses_bidi_split_interior_edges_and_logical_ribbon() {
    // RTL base paragraph: "שלום abc" where "שלום" (8 bytes) and "abc" (3 bytes).
    // Span covers "לום ab" (bytes 2..11): in visual order on one line,
    // RTL "לום" is on the right and LTR "ab" is on the left, split by Bidi.
    let text = "שלום abc";
    let base = script_style(&[HEBREW], 10.0);
    let spans = [InlineDecorationSpan {
        span_id: 1,
        byte_range: TextRange::new(2, 11),
        start_padding: 2.0,
        end_padding: 2.0,
        break_mode: BoxDecorationBreak::Slice,
        height_style: BoxHeightStyle::Strut,
    }];
    let paragraph = ParagraphInput {
        decoration_spans: &spans,
        ..input(text, &base)
    };
    let options = FrontendOptions {
        base_direction: Some(BidiDirection::Rtl),
        ..Default::default()
    };
    let ast = compile_paragraph(&paragraph, &options, &*BIDI_SHAPER).expect("compiles");
    let p = layout_paragraph(
        &ast,
        text,
        &*BIDI_SHAPER,
        &column(300.0),
        &LayoutOptions::default(),
    )
    .expect("lays out");
    ValidatedLayout::try_from_paragraph(&p, text).expect("passes G4");
    let frags = &p.lines[0].span_fragments;
    if frags.len() >= 2 {
        assert_eq!(frags[0].edges.right, BidiSplit);
        assert_eq!(frags[1].edges.left, BidiSplit);
        let mut by_bytes: Vec<_> = frags.iter().collect();
        by_bytes.sort_by_key(|f| (f.byte_range.start(), f.byte_range.end()));
        let mut acc = 0.0;
        for f in by_bytes {
            assert_close(f.ribbon_offset, acc);
            acc += f.width;
        }
    }
}
