//! Frontend Steps 3 and 6 beyond the Phase 1 core: emoji (FE-6), URL
//! tailoring (FE-22), the no-line-end words (FE-23), unbreakable dash runs
//! (FE-15), and the decoration and atomic span streams (WN-8, WN-21,
//! AST-7 to AST-9).

use crate::common::*;
use crate::frontend::passes_the_input_gate;
use mutil_engine::*;

fn compile_with(input: &ParagraphInput<'_>, options: &FrontendOptions) -> ParagraphAst {
    compile_paragraph(input, options, &*SHAPER).unwrap_or_else(|e| panic!("{e}"))
}

fn discretionaries(ast: &ParagraphAst) -> Vec<&DiscretionaryNode> {
    ast.items
        .iter()
        .filter_map(|item| match item {
            WordNode::Discretionary(d) if !d.is_paragraph_end() => Some(d),
            _ => None,
        })
        .collect()
}

#[test]
fn decoration_spans_compile_to_nested_boundaries_with_cloned_padding() {
    let text = "ab cd ef";
    let base = style(10.0);
    let spans = [
        InlineDecorationSpan {
            span_id: 7,
            byte_range: TextRange::new(0, 5),
            start_padding: 2.0,
            end_padding: 3.0,
            break_mode: BoxDecorationBreak::Clone,
        },
        InlineDecorationSpan {
            span_id: 9,
            byte_range: TextRange::new(3, 5),
            start_padding: 1.0,
            end_padding: 1.0,
            break_mode: BoxDecorationBreak::Slice,
        },
    ];
    let paragraph = ParagraphInput {
        decoration_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_with(&paragraph, &FrontendOptions::default());
    // At 3 the break precedes the inner Leading; at 5 the inner Trailing
    // precedes the outer one (AST-8).
    assert_eq!(shape_of(&ast, text), "S[ab]_|S[cd]SS_|[ef]$");
    let edges: Vec<(u64, SpanEdge, f32)> = ast
        .items
        .iter()
        .filter_map(|item| match item {
            WordNode::Box(BoxNode::SpanBoundary {
                span_id,
                edge,
                width,
                ..
            }) => Some((*span_id, *edge, *width)),
            _ => None,
        })
        .collect();
    assert_eq!(
        edges,
        [
            (7, SpanEdge::Leading, 2.0),
            (9, SpanEdge::Leading, 1.0),
            (9, SpanEdge::Trailing, 1.0),
            (7, SpanEdge::Trailing, 3.0),
        ]
    );
    // The break inside the Clone span carries its padding (WN-21); the one
    // after it does not.
    let breaks = discretionaries(&ast);
    assert_eq!(breaks[0].cloned_spans.as_slice(), [7]);
    assert_eq!(
        (breaks[0].pre_break_width, breaks[0].post_break_width),
        (3.0, 2.0)
    );
    assert_eq!(breaks[0].penalty, DiscretionaryNode::CLONED_SPAN_PENALTY);
    assert!(breaks[1].cloned_spans.is_empty());
    assert_eq!(ast.decoration_spans, spans);
    passes_the_input_gate(&ast, text);
    for width in [12.0, 20.0, 30.0, 100.0] {
        let p = layout_paragraph(
            &ast,
            text,
            &*SHAPER,
            &column(width),
            &LayoutOptions::default(),
        )
        .expect("the paragraph lays out");
        ValidatedLayout::new(&p, text).expect("the output gate accepts it");
    }
}

#[test]
fn an_atomic_span_has_no_inner_caret_stop() {
    let text = "see @alice now";
    let base = style(10.0);
    let spans = [AtomicEditSpan {
        span_id: 3,
        byte_range: TextRange::new(4, 10),
    }];
    let paragraph = ParagraphInput {
        atomic_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_with(&paragraph, &FrontendOptions::default());
    assert_eq!(ast.atomic_spans, spans);
    passes_the_input_gate(&ast, text);
    let p = layout_paragraph(
        &ast,
        text,
        &*SHAPER,
        &column(200.0),
        &LayoutOptions::default(),
    )
    .expect("the paragraph lays out");
    ValidatedLayout::new(&p, text).expect("the output gate accepts it");
    assert!(
        p.lines[0]
            .caret_stops
            .iter()
            .all(|s| s.offset <= 4 || s.offset >= 10)
    );
}

#[test]
fn malformed_spans_are_reported() {
    let base = style(10.0);
    let options = FrontendOptions::default();
    let span = |id, start, end| InlineDecorationSpan {
        span_id: id,
        byte_range: TextRange::new(start, end),
        start_padding: 0.0,
        end_padding: 0.0,
        break_mode: BoxDecorationBreak::Slice,
    };
    let error = |spans: &[InlineDecorationSpan], text: &str| {
        compile_paragraph(
            &ParagraphInput {
                decoration_spans: spans,
                ..input(text, &base)
            },
            &options,
            &*SHAPER,
        )
        .unwrap_err()
    };
    let invalid = |span| FrontendError::InvalidDecorationSpan { span };
    // Overlapping without nesting, a repeated id, an edge inside a grapheme.
    assert_eq!(error(&[span(1, 0, 2), span(2, 1, 3)], "abc"), invalid(1));
    assert_eq!(error(&[span(1, 0, 1), span(1, 1, 2)], "abc"), invalid(1));
    assert_eq!(error(&[span(1, 0, 1)], "e\u{301}"), invalid(0));
    let atomic = |start, end| AtomicEditSpan {
        span_id: 1,
        byte_range: TextRange::new(start, end),
    };
    let atomic_error = |spans: &[AtomicEditSpan], text: &str| {
        compile_paragraph(
            &ParagraphInput {
                atomic_spans: spans,
                ..input(text, &base)
            },
            &options,
            &*SHAPER,
        )
        .unwrap_err()
    };
    // An edge inside the word space, and a span holding only a space.
    assert_eq!(
        atomic_error(&[atomic(0, 2)], "a b"),
        FrontendError::InvalidAtomicSpan { span: 0 }
    );
    assert_eq!(
        atomic_error(&[atomic(1, 2)], "a b"),
        FrontendError::InvalidAtomicSpan { span: 0 }
    );
}

#[test]
fn an_emoji_sequence_is_one_atom_in_the_emoji_script() {
    let text = "a\u{1F44B}\u{1F3FD}b";
    let ast = compile(text);
    assert_eq!(ast.atoms.len(), 3);
    let emoji = &ast.atoms[1];
    assert!(emoji.flags.contains(AtomFlags::IS_EMOJI_SEQUENCE));
    assert_eq!(emoji.script, ScriptTag::EMOJI);
    assert!(!ast.atoms[0].flags.contains(AtomFlags::IS_EMOJI_SEQUENCE));
    // The emoji runs on its own between the Latin letters.
    assert_eq!(ast.runs.len(), 3);
    passes_the_input_gate(&ast, text);
}

#[test]
fn urls_gain_opportunities_in_the_url_band() {
    let text = "see https://example.com/a/b?q=1 now";
    let ast = compile(text);
    let url = TextRange::new(4, 31);
    assert_eq!(&text[url.as_range()], "https://example.com/a/b?q=1");
    let inside: Vec<(usize, i32)> = discretionaries(&ast)
        .iter()
        .map(|d| (d.byte_offset(), d.penalty))
        .filter(|&(at, _)| url.start() < at && at < url.end())
        .collect();
    assert!(inside.iter().any(|&(at, _)| &text[at..at + 7] == "example"));
    for (_, penalty) in &inside {
        assert!(
            (DiscretionaryNode::URL_PENALTY_MIN..=DiscretionaryNode::URL_PENALTY_MAX)
                .contains(penalty)
        );
    }
    passes_the_input_gate(&ast, text);
}

#[test]
fn czech_one_letter_words_bind_to_the_next_word() {
    let text = "jdu v les a k nám";
    let base = style(10.0);
    let czech = FrontendOptions {
        default_locale: LocaleTag::try_from_str("cs-CZ").expect("a locale"),
        ..Default::default()
    };
    let ast = compile_with(&input(text, &base), &czech);
    let after: Vec<&str> = discretionaries(&ast)
        .iter()
        .map(|d| &text[d.byte_offset()..])
        .collect();
    // No break after "v " or "k " ("a" is not listed); English binds none.
    assert_eq!(after, ["v les a k nám", "a k nám", "k nám"]);
    let english = compile(text);
    assert_eq!(discretionaries(&english).len(), 5);
}

#[test]
fn dash_and_ellipsis_runs_never_break_inside() {
    let text = "a——b……c";
    let base = style(10.0);
    let anywhere = FrontendOptions {
        line_break_strictness: LineBreakStrictness::Anywhere,
        ..Default::default()
    };
    let ast = compile_with(&input(text, &base), &anywhere);
    let at: Vec<usize> = discretionaries(&ast)
        .iter()
        .map(|d| d.byte_offset())
        .collect();
    let (dash, ellipsis) = (text.find('—').unwrap(), text.find('…').unwrap());
    assert!(!at.contains(&(dash + 3)), "{at:?}");
    assert!(!at.contains(&(ellipsis + 3)), "{at:?}");
}

/// The vertical offset of the first glyph of the cluster at `byte`.
fn glyph_y(p: &PositionedParagraph, byte: usize) -> f32 {
    p.lines[0]
        .visual_runs
        .iter()
        .flat_map(|r| r.glyphs.iter())
        .find(|g| g.byte_range.is_some_and(|b| b.start() == byte))
        .map(|g| g.y_offset)
        .expect("a glyph at the byte")
}

#[test]
fn named_baselines_align_runs_and_objects_to_the_strut() {
    // The strut is the monospace face, which has no BASE: its ideographic
    // baseline is the fallback -descent (-3.0 at 10). Noto Sans JP's BASE
    // puts `ideo` 120 units below `romn`: -1.2 at 10.
    let text = "ab日本x";
    let base = style(10.0);
    let mut jp = script_style(&["Noto Sans JP"], 10.0);
    jp.baseline_align = BaselineAlignment::Ideographic;
    let spans = [StyleSpan {
        byte_range: TextRange::new(2, 8),
        style: jp,
    }];
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(8, 9),
        box_id: 1,
        width: 5.0,
        ascent: 4.0,
        descent: 1.0,
        baseline_align: BaselineAlignment::Ideographic,
        overflow_policy: BoxOverflowPolicy::AllowOverflow,
    }];
    let paragraph = ParagraphInput {
        style_spans: &spans,
        inline_objects: &objects,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SCRIPT_SHAPER)
        .expect("the frontend accepts named baselines");
    let p = layout_paragraph(
        &ast,
        text,
        &*SCRIPT_SHAPER,
        &column(200.0),
        &LayoutOptions::default(),
    )
    .expect("the paragraph lays out");
    ValidatedLayout::new(&p, text).expect("the output gate accepts it");
    assert_close(glyph_y(&p, 0), 0.0);
    // The run's ideographic baseline moves onto the strut's: -3.0 - -1.2.
    assert_close(glyph_y(&p, 2), -1.8);
    // The object's baseline sits on the strut's ideographic baseline, 3.0
    // below the alphabetic one (y grows downwards).
    assert_close(p.lines[0].visual_boxes[0].y, 3.0);
}

#[test]
fn line_top_and_bottom_objects_sit_on_the_line_edges() {
    let text = "ab cd";
    let base = style(10.0);
    let object = |start, align, ascent| InlineObjectInput {
        byte_range: TextRange::new(start, start + 1),
        box_id: start as u64,
        width: 4.0,
        ascent,
        descent: 0.0,
        baseline_align: align,
        overflow_policy: BoxOverflowPolicy::AllowOverflow,
    };
    // A short top object and a tall bottom one.
    let objects = [
        object(0, BaselineAlignment::LineTop, 2.0),
        object(4, BaselineAlignment::LineBottom, 30.0),
    ];
    let paragraph = ParagraphInput {
        inline_objects: &objects,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SHAPER)
        .expect("the frontend accepts line-relative objects");
    let p = layout_paragraph(
        &ast,
        text,
        &*SHAPER,
        &column(200.0),
        &LayoutOptions::default(),
    )
    .expect("the paragraph lays out");
    ValidatedLayout::new(&p, text).expect("the output gate accepts it");
    let line = &p.lines[0];
    // The strut is 10.2 + 3.0; the 30-unit bottom object deepens the line
    // above the baseline to 27.0.
    assert_close(line.descent, 3.0);
    assert_close(line.ascent, 27.0);
    let (top, bottom) = (&line.visual_boxes[0], &line.visual_boxes[1]);
    // Top flush with the over edge: its top is `ascent` above the baseline.
    assert_close(-top.y + top.ascent, line.ascent);
    // Bottom flush with the under edge.
    assert_close(-bottom.y - bottom.descent, -line.descent);
}
