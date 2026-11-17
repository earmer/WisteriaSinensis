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
fn fe22_www_url_after_opening_punctuation_gains_url_opportunities() {
    let text = "(www.example.com/a/b)";
    let ast = compile(text);
    // "www.example.com/a/b" is bytes 1..20.
    let url = TextRange::new(1, 20);
    assert_eq!(&text[url.as_range()], "www.example.com/a/b");
    let at: Vec<(usize, i32)> = discretionaries(&ast)
        .iter()
        .map(|d| (d.byte_offset(), d.penalty))
        .collect();
    // Never beside `(` (FE-22).
    assert!(!at.iter().any(|&(offset, _)| offset == 1), "{at:?}");
    let inside: Vec<(usize, i32)> = at
        .iter()
        .copied()
        .filter(|&(offset, _)| url.start() < offset && offset < url.end())
        .collect();
    assert!(
        inside
            .iter()
            .any(|&(offset, _)| text[offset..].starts_with(".example")
                || text[offset..].starts_with("example")),
        "{inside:?}"
    );
    assert!(
        inside
            .iter()
            .any(|&(offset, _)| text[offset..].starts_with("a/b")),
        "{inside:?}"
    );
    for (_, penalty) in &inside {
        assert!(
            (DiscretionaryNode::URL_PENALTY_MIN..=DiscretionaryNode::URL_PENALTY_MAX)
                .contains(penalty),
            "unexpected penalty {penalty} in {inside:?}"
        );
    }
    passes_the_input_gate(&ast, text);
}

#[test]
fn fe22_an_email_domain_is_not_a_www_url() {
    for text in ["mail name@www.example.com now", "see a/www.example.com now"] {
        let ast = compile(text);
        let url_band = DiscretionaryNode::URL_PENALTY_MIN..=DiscretionaryNode::URL_PENALTY_MAX;
        let in_band: Vec<usize> = discretionaries(&ast)
            .iter()
            .filter(|d| url_band.contains(&d.penalty))
            .map(|d| d.byte_offset())
            .collect();
        assert!(in_band.is_empty(), "{text:?}: URL breaks at {in_band:?}");
        passes_the_input_gate(&ast, text);
    }
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
fn fe23_czech_one_letter_word_after_opening_punctuation_binds() {
    let text = "(v lese)";
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
    assert!(
        !after.contains(&"lese)"),
        "expected no break after \"(v \": {after:?}"
    );
    passes_the_input_gate(&ast, text);

    let english = compile(text);
    let en_after: Vec<&str> = discretionaries(&english)
        .iter()
        .map(|d| &text[d.byte_offset()..])
        .collect();
    assert!(en_after.contains(&"lese)"), "{en_after:?}");
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
    assert!(!ast.atoms[1].flags.contains(AtomFlags::BINDS_TO_PREVIOUS));
    assert!(ast.atoms[2].flags.contains(AtomFlags::BINDS_TO_PREVIOUS));
    assert!(!ast.atoms[4].flags.contains(AtomFlags::BINDS_TO_PREVIOUS));
    assert!(ast.atoms[5].flags.contains(AtomFlags::BINDS_TO_PREVIOUS));

    // A ShapingRun boundary (here a style span) prevents binding across runs.
    let split_spans = [StyleSpan {
        byte_range: TextRange::new(3, 6),
        style: style(20.0),
    }];
    let split = ParagraphInput {
        style_spans: &split_spans,
        ..input("——", &base)
    };
    let split_ast = compile_with(&split, &anywhere);
    assert_eq!(split_ast.runs.len(), 2);
    assert!(
        !split_ast.atoms[0]
            .flags
            .contains(AtomFlags::BINDS_TO_PREVIOUS)
    );
    assert!(
        !split_ast.atoms[1]
            .flags
            .contains(AtomFlags::BINDS_TO_PREVIOUS)
    );
}

#[test]
fn fe15_space_separated_dashes_are_separate_runs_and_break_at_space() {
    let text = "字 — — a";
    let jp = script_style(&["Noto Sans JP"], 10.0);
    let anywhere = FrontendOptions {
        line_break_strictness: LineBreakStrictness::Anywhere,
        ..Default::default()
    };
    let ast = compile_paragraph(&input(text, &jp), &anywhere, &*SCRIPT_SHAPER).expect("compiles");
    passes_the_input_gate(&ast, text);

    // Neither dash binds to the previous atom across the space.
    for (i, atom) in ast.atoms.iter().enumerate() {
        assert!(
            !atom.flags.contains(AtomFlags::BINDS_TO_PREVIOUS),
            "atom {i} unexpectedly bound: {atom:?}"
        );
    }

    // There is a break opportunity between the space and the second dash.
    let second_dash = text.rfind('—').expect("second dash");
    let breaks: Vec<usize> = discretionaries(&ast)
        .iter()
        .map(|d| d.byte_offset())
        .collect();
    assert!(
        breaks.contains(&second_dash),
        "expected break before second dash at {second_dash}: {breaks:?}"
    );

    // Laying out the `Anywhere` AST at a width that fits "字 — " breaks
    // between the space and the second dash (under `Normal`, UAX #14 LB17
    // suppresses B2 SP* ÷ B2).
    let p = layout_paragraph(
        &ast,
        text,
        &*SCRIPT_SHAPER,
        &column(25.0),
        &LayoutOptions::default(),
    )
    .expect("lays out");
    ValidatedLayout::new(&p, text).expect("validates");
    assert_eq!(line_texts(&p, text), ["字 — ", "— a"]);

    // Each space-separated dash resolves its script independently rather than
    // merging across the space.
    let mixed = "字 — a — b";
    let mixed_ast = compile_in(mixed, &["Noto Sans JP"]);
    passes_the_input_gate(&mixed_ast, mixed);
    assert_eq!(mixed_ast.atoms[1].script, ScriptTag::HAN);
    assert_eq!(mixed_ast.atoms[3].script, ScriptTag::LATIN);
}

#[test]
fn fe11_decomposed_apostrophe_resolves_in_word_script() {
    // Greek alpha + combining acute (U+0301) + apostrophe + beta:
    // U+0301 is Extend (ignored per UAX #29 WB4), so AHLetter lies on both sides.
    for text in ["α\u{0301}'β", "α\u{0301}\u{2019}β"] {
        let ast = compile(text);
        passes_the_input_gate(&ast, text);
        assert_eq!(ast.atoms.len(), 3, "{text:?}: {:?}", ast.atoms);
        let apos = &ast.atoms[1];
        assert!(
            apos.flags.contains(AtomFlags::IS_APOSTROPHE),
            "{text:?}: expected IS_APOSTROPHE on {apos:?}"
        );
        assert_eq!(
            apos.script,
            ScriptTag::GREEK,
            "{text:?}: expected Greek script on {apos:?}"
        );
    }

    let latin = "cafe\u{0301}'s";
    let ast = compile(latin);
    passes_the_input_gate(&ast, latin);
    assert_eq!(ast.atoms.len(), 6, "{:?}", ast.atoms);
    let apos = &ast.atoms[4];
    assert!(apos.flags.contains(AtomFlags::IS_APOSTROPHE), "{apos:?}");
    assert_eq!(apos.script, ScriptTag::LATIN, "{apos:?}");
}

fn locale(tag: &str) -> LocaleTag {
    LocaleTag::try_from_str(tag).expect("a locale")
}

/// The (text, locale) of each run of `text` compiled with `options`.
fn run_locales(text: &str, style: &TextStyle, options: &FrontendOptions) -> Vec<(String, String)> {
    let ast = compile_paragraph(&input(text, style), options, &*SCRIPT_SHAPER)
        .unwrap_or_else(|e| panic!("{e}"));
    ast.runs
        .iter()
        .map(|r| {
            (
                text[r.byte_range.as_range()].to_owned(),
                r.locale.as_str().to_owned(),
            )
        })
        .collect()
}

#[test]
fn experimental_locale_heuristics_tag_han_by_its_sentence() {
    let jp = script_style(&["Noto Sans JP"], 10.0);
    let explicit = FrontendOptions {
        default_locale: locale("en-US"),
        ..Default::default()
    };
    let heuristic = FrontendOptions {
        locale_policy: LocalePolicy::ExperimentalHeuristics,
        ..explicit.clone()
    };
    // Kana in the sentence: the Han, the kana and the punctuation that
    // joined them take ja-JP and share runs by script alone.
    let text = "東京へ行く。";
    assert!(
        run_locales(text, &jp, &explicit)
            .iter()
            .all(|(_, l)| l == "en-US")
    );
    let runs = run_locales(text, &jp, &heuristic);
    assert!(runs.iter().all(|(_, l)| l == "ja-JP"), "{runs:?}");
    // Han alone keeps the default, and an explicit tag stands.
    assert!(
        run_locales("東京", &jp, &heuristic)
            .iter()
            .all(|(_, l)| l == "en-US")
    );
    let tagged = TextStyle {
        locale: Some(locale("zh-Hans")),
        ..jp.clone()
    };
    assert!(
        run_locales(text, &tagged, &heuristic)
            .iter()
            .all(|(_, l)| l == "zh-Hans")
    );
    // Serbian letters make Cyrillic sr-Cyrl; Latin in the same sentence
    // keeps the default.
    let text = "ћирилица and latin";
    let runs = run_locales(text, &style(10.0), &heuristic);
    assert_eq!(runs[0], ("ћирилица".to_owned(), "sr-Cyrl".to_owned()));
    assert!(runs[1..].iter().all(|(_, l)| l == "en-US"), "{runs:?}");
}

#[test]
fn table_s_reads_the_override_of_the_stated_locale() {
    // A German “ closes „ only, so in “…” the closer pairs with nothing
    // and takes the preceding script; under English it takes the opener's.
    let text = "α“ab”";
    let closer_script = |tag: &str| {
        let style = TextStyle {
            locale: Some(locale(tag)),
            ..style(10.0)
        };
        let ast = compile_paragraph(&input(text, &style), &FrontendOptions::default(), &*SHAPER)
            .unwrap_or_else(|e| panic!("{e}"));
        ast.atoms.last().map(|a| a.script).expect("atoms")
    };
    assert_eq!(closer_script("en-GB"), ScriptTag::GREEK);
    assert_eq!(closer_script("de-DE"), ScriptTag::LATIN);
}

#[test]
fn many_spans_compile_in_linear_time() {
    // 20,000 one-letter decoration spans, every other one nested in a
    // two-letter Clone span, and 20,000 atomic spans over the other letters:
    // checking and placing them is one sweep, not atoms × spans.
    let n = 40_000;
    let text = "a".repeat(n);
    let base = style(10.0);
    let mut decoration = Vec::new();
    for i in (0..n).step_by(2) {
        decoration.push(InlineDecorationSpan {
            span_id: i as u64,
            byte_range: TextRange::new(i, i + 2),
            break_mode: BoxDecorationBreak::Clone,
            start_padding: 1.0,
            end_padding: 1.0,
        });
        decoration.push(InlineDecorationSpan {
            span_id: i as u64 + 1,
            byte_range: TextRange::new(i, i + 1),
            break_mode: BoxDecorationBreak::Slice,
            start_padding: 0.0,
            end_padding: 0.0,
        });
    }
    let atomic: Vec<AtomicEditSpan> = (1..n)
        .step_by(2)
        .map(|i| AtomicEditSpan {
            span_id: i as u64,
            byte_range: TextRange::new(i, i + 1),
        })
        .collect();
    let paragraph = ParagraphInput {
        decoration_spans: &decoration,
        atomic_spans: &atomic,
        ..input(&text, &base)
    };
    let anywhere = FrontendOptions {
        line_break_strictness: LineBreakStrictness::Anywhere,
        ..Default::default()
    };
    let ast = compile_with(&paragraph, &anywhere);
    let boundaries = ast
        .items
        .iter()
        .filter(|i| matches!(i, WordNode::Box(BoxNode::SpanBoundary { .. })))
        .count();
    assert_eq!(boundaries, 2 * decoration.len());
    // The break inside each Clone span (after its first letter) carries it.
    let cloned = discretionaries(&ast)
        .iter()
        .filter(|d| d.cloned_spans.len() == 1)
        .count();
    assert_eq!(cloned, n / 2);
}

#[test]
fn greedy_breaking_prefers_the_lowest_tier_of_the_overflow_ladder() {
    // 6 units a character. At 156 "https://example.com/abcdef" fits, and
    // the long piece "abcdefghijklmnop" may break anywhere, but the break at
    // the slash (URL punctuation, tier 1) wins over one inside the piece
    // (tier 2).
    let text = "https://example.com/abcdefghijklmnop";
    let p = lay_out(text, &column(156.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text)[0], "https://example.com/");
    // A natural break (tier 0) wins over URL punctuation, and a long piece
    // breaks inside only when it holds the line alone.
    let text = "see https://example.com/abcdefghijklmnopqrstuvwxyz";
    let p = lay_out(text, &column(96.0), &LayoutOptions::default());
    assert_eq!(
        line_texts(&p, text),
        [
            "see ",
            "https://example",
            ".com/",
            "abcdefghijklmnop",
            "qrstuvwxyz"
        ]
    );
}

#[test]
fn a_dash_run_too_wide_for_an_empty_line_splits_one_cluster_at_a_time() {
    // JetBrains Mono at 10 px: every cluster is 6 wide, so "……" is 12 and
    // "————" 24. In a column of 5 no run fits an empty line: ST1-5 splits it,
    // one overflowing cluster per line (SLT-5), and the output gate (G4)
    // accepts every line.
    for text in ["……", "————", "a……", "ab——cd"] {
        for width in [5.0, 7.0, 11.0, 15.0] {
            let p = lay_out(text, &column(width), &LayoutOptions::default());
            for line in &p.lines {
                let s = &text[line.byte_range.as_range()];
                let units = s.chars().count();
                let end = line.caret_stops.iter().map(|c| c.x).fold(0.0, f32::max);
                assert!(
                    end <= width + 1e-3 || units == 1,
                    "{text:?} at {width}: {s:?} ends at {end}"
                );
            }
        }
    }
    // With room for two ellipses after "a", the run moves whole: "a" alone,
    // then "……".
    let p = lay_out("a……", &column(15.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, "a……"), ["a", "……"]);
    // A run that starts the line keeps as many clusters as fit.
    let p = lay_out("————", &column(15.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, "————"), ["——", "——"]);
    // Full-width ellipses in Noto Sans JP, 10 each: "……" is 20 against a
    // slot of 15, so each line holds one, and the output gate accepts it.
    for text in ["……", "東……京"] {
        let p = lay_out_in(
            text,
            &["Noto Sans JP"],
            &column(15.0),
            &LayoutOptions::default(),
        );
        assert!(
            line_texts(&p, text).iter().all(|s| s.chars().count() == 1),
            "{:?}",
            line_texts(&p, text)
        );
    }
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
