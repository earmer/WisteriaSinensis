//! `compile_paragraph` (spec §3): the item sequence it builds, and that every
//! AST it returns passes the input gate (FE-26).

use crate::common::*;
use mutil_engine::*;

#[track_caller]
fn passes_the_input_gate(ast: &ParagraphAst, text: &str) {
    for wrap_width in [1.0, 37.5, f32::INFINITY] {
        for options in [
            LayoutOptions::default(),
            LayoutOptions {
                line_height: LineHeight::Fixed(1.5),
                whitespace: HangingWhitespaceMode::Preserved,
                ..Default::default()
            },
        ] {
            check_layout_inputs(ast, text, &column(wrap_width), &options)
                .unwrap_or_else(|e| panic!("{text:?}: {e}"));
        }
    }
}

#[test]
fn words_spaces_tabs_and_hard_breaks_compile_to_their_nodes() {
    for (text, expected) in [
        ("", "$"),
        ("a", "[a]$"),
        ("ab cd", "[ab]_|[cd]$"),
        ("ab  cd", "[ab]__|[cd]$"),
        ("ab ", "[ab]_$"),
        (" ab", "_|[ab]$"),
        ("a\tb", "[a]T|[b]$"),
        ("a\u{2028}b", "[a]H[b]$"),
        ("a\u{2028}", "[a]H$"),
        // UAX #14 HY: a break after the hyphen splits the word.
        ("well-known", "[well-]|[known]$"),
        // No-break spaces are text (WN-12), with no opportunity beside them.
        ("a\u{a0}b", "[a\u{a0}b]$"),
        // A space followed by a combining mark is an atom, not glue.
        ("a \u{301}b", "[a \u{301}b]$"),
        // Closing punctuation is fused to its word (WN-3).
        ("(a) b.", "[(a)]_|[b.]$"),
    ] {
        let ast = compile(text);
        assert_eq!(shape_of(&ast, text), expected, "{text:?}");
        passes_the_input_gate(&ast, text);
    }
}

#[test]
fn word_spaces_are_sized_from_the_space_advance_and_word_spacing() {
    let text = "a b";
    let ast = compile(text);
    let WordNode::Glue(glue) = &ast.items[1] else {
        panic!("glue expected");
    };
    assert_close(glue.natural_width, 6.0);
    assert_close(glue.max_stretch, 3.0);
    assert_close(glue.max_shrink, 2.0);
    let mut spaced = style(10.0);
    spaced.word_spacing = -10.0;
    let ast =
        compile_paragraph(&input(text, &spaced), &FrontendOptions::default(), &*SHAPER).unwrap();
    let WordNode::Glue(glue) = &ast.items[1] else {
        panic!("glue expected");
    };
    // Negative spacing never makes a width negative (FE-27).
    assert_eq!(glue.natural_width, 0.0);
}

#[test]
fn clusters_carry_glyph_advances_plus_letter_spacing() {
    let text = "ab";
    let mut spaced = style(10.0);
    spaced.letter_spacing = 1.5;
    let ast =
        compile_paragraph(&input(text, &spaced), &FrontendOptions::default(), &*SHAPER).unwrap();
    assert_eq!(ast.clusters.len(), 2);
    for cluster in &ast.clusters {
        assert_close(cluster.natural_width, 7.5);
        assert_eq!(cluster.font, FontRef::unscaled(FontId(0)));
    }
    passes_the_input_gate(&ast, text);
}

#[test]
fn a_combining_mark_shares_its_base_cluster_and_atom() {
    let text = "e\u{301}x";
    let ast = compile(text);
    assert_eq!(ast.atoms.len(), 2);
    assert_eq!(ast.clusters.len(), 2);
    assert_eq!(ast.clusters[0].byte_range, TextRange::new(0, 3));
    assert!(ast.cluster_glyphs(&ast.clusters[0]).len() <= 2);
    assert_close(ast.clusters[0].natural_width, 6.0);
}

#[test]
fn the_strut_is_the_default_styles_primary_font() {
    let ast = compile("");
    assert_eq!(ast.strut_font_size, 10.0);
    assert_close(ast.strut_metrics.ascent, 10.2);
    assert!(ast.items.len() == 1 && ast.runs.is_empty());
    passes_the_input_gate(&ast, "");
}

#[test]
fn runs_split_at_style_and_script_changes() {
    let text = "ab cd\u{3b1}";
    let big = style(20.0);
    let base = style(10.0);
    let spans = [StyleSpan {
        byte_range: TextRange::new(3, 5),
        style: big.clone(),
    }];
    let input = ParagraphInput {
        style_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&input, &FrontendOptions::default(), &*SHAPER).unwrap();
    let summary: Vec<_> = ast
        .runs
        .iter()
        .map(|r| (r.byte_range.as_range(), r.font_size, r.script))
        .collect();
    assert_eq!(
        summary,
        [
            (0..2, 10.0, ScriptTag::LATIN),
            (3..5, 20.0, ScriptTag::LATIN),
            (5..7, 10.0, ScriptTag::GREEK),
        ]
    );
    passes_the_input_gate(&ast, text);
}

#[test]
fn inline_objects_own_their_bytes() {
    let text = "a\u{fffc}b";
    let base = style(10.0);
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(1, 4),
        box_id: 7,
        width: 12.0,
        ascent: 9.0,
        descent: 1.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    let input = ParagraphInput {
        inline_objects: &objects,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&input, &FrontendOptions::default(), &*SHAPER).unwrap();
    // UAX #14 CB: breaks on both sides of U+FFFC.
    assert_eq!(shape_of(&ast, text), "[a]|O|[b]$");
    passes_the_input_gate(&ast, text);
}

#[test]
fn what_phase_1_does_not_compile_is_refused() {
    let base = style(10.0);
    let options = FrontendOptions::default();
    let refused = |input: &ParagraphInput<'_>, options: &FrontendOptions| {
        compile_paragraph(input, options, &*SHAPER).unwrap_err()
    };
    let unsupported = |feature| FrontendError::Unsupported { feature };
    assert_eq!(
        refused(&input("\u{5d0}", &base), &options),
        unsupported(FrontendFeature::RtlBaseDirection)
    );
    let ltr = FrontendOptions {
        base_direction: Some(BidiDirection::Ltr),
        ..Default::default()
    };
    assert_eq!(
        refused(&input("a \u{5d0}", &base), &ltr),
        unsupported(FrontendFeature::BidiText)
    );
    let vertical = FrontendOptions {
        writing_mode: WritingMode::VerticalRl,
        ..Default::default()
    };
    assert_eq!(
        refused(&input("a", &base), &vertical),
        unsupported(FrontendFeature::VerticalWritingMode)
    );
    let spans = [AtomicEditSpan {
        span_id: 1,
        byte_range: TextRange::new(0, 1),
    }];
    let with_span = ParagraphInput {
        atomic_spans: &spans,
        ..input("a", &base)
    };
    assert_eq!(
        refused(&with_span, &options),
        unsupported(FrontendFeature::AtomicSpans)
    );
    let mut hanging = style(10.0);
    hanging.baseline_align = BaselineAlignment::Hanging;
    assert_eq!(
        refused(&input("a", &hanging), &options),
        unsupported(FrontendFeature::BaselineAlignment)
    );
}

#[test]
fn malformed_input_is_reported() {
    let base = style(10.0);
    let options = FrontendOptions::default();
    let error = |input: &ParagraphInput<'_>| compile_paragraph(input, &options, &*SHAPER);
    assert_eq!(
        error(&input("a\nb", &base)).unwrap_err(),
        FrontendError::ParagraphSeparatorInText { byte: 1 }
    );
    let mut bad = style(10.0);
    bad.font_size = 0.0;
    assert_eq!(
        error(&input("a", &bad)).unwrap_err(),
        FrontendError::InvalidStyle { span: None }
    );
    let spans = [StyleSpan {
        byte_range: TextRange::new(1, 2),
        style: base.clone(),
    }];
    assert_eq!(
        error(&ParagraphInput {
            style_spans: &spans,
            ..input("\u{e9}", &base)
        })
        .unwrap_err(),
        FrontendError::InvalidStyleSpan { span: 0 }
    );
    // An object edge inside a grapheme (between e and its accent).
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(0, 1),
        box_id: 1,
        width: 1.0,
        ascent: 1.0,
        descent: 0.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    assert_eq!(
        error(&ParagraphInput {
            inline_objects: &objects,
            ..input("e\u{301}", &base)
        })
        .unwrap_err(),
        FrontendError::InvalidInlineObject { object: 0 }
    );
}
