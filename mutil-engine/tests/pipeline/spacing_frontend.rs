//! The Phase 2 spacing analysis of the frontend: CJK autospace (FE-20,
//! SPN-4), punctuation compression (FE-31, WN-13), synthetic glue (WN-10,
//! WN-14), the item order at a seam (AST-3, AST-8), the Phase 2 atom flags
//! (ATM-3, FE-31) and cursive letter spacing (FE-27, AST-5).

use crate::common::*;
use mutil_engine::*;

/// Latin from Noto Serif, CJK from Noto Sans JP.
const MIXED: &[&str] = &[SERIF, JAPANESE];

fn options(profile: AutospaceProfile) -> FrontendOptions {
    FrontendOptions {
        autospace_profile: profile,
        ..Default::default()
    }
}

/// Compiles `text` in `MIXED` at 10 units per em under `profile`.
#[track_caller]
fn compile_mixed(text: &str, profile: AutospaceProfile) -> ParagraphAst {
    let style = script_style(MIXED, 10.0);
    compile_bidi_input(&input(text, &style), &options(profile))
}

/// Compiles `text` in `families` at 10 units per em in `locale` with
/// [`SPACING_SHAPER`].
#[track_caller]
fn compile_localized(text: &str, families: &[&str], locale: &str) -> ParagraphAst {
    let style = localized_style(families, 10.0, locale);
    compile_paragraph(
        &input(text, &style),
        &FrontendOptions::default(),
        &*SPACING_SHAPER,
    )
    .unwrap_or_else(|e| panic!("{text:?} in {locale}: {e}"))
}

/// The offsets of the `CjkAutospace` glue of `ast`.
fn autospace_offsets(ast: &ParagraphAst) -> Vec<usize> {
    glues_of(ast, GlueKind::CjkAutospace)
        .into_iter()
        .map(|(offset, _)| offset)
        .collect()
}

/// The offsets of the `CjkPunctCompression` glue of `ast`.
fn compression_offsets(ast: &ParagraphAst) -> Vec<usize> {
    glues_of(ast, GlueKind::CjkPunctCompression)
        .into_iter()
        .map(|(offset, _)| offset)
        .collect()
}

/// The (natural width, shrink, stretch) of a glue.
fn sizes(glue: &GlueNode) -> (f32, f32, f32) {
    (glue.natural_width, glue.max_shrink, glue.max_stretch)
}

#[track_caller]
fn assert_sizes(glue: &GlueNode, expected: (f32, f32, f32)) {
    let (natural, shrink, stretch) = sizes(glue);
    assert_close(natural, expected.0);
    assert_close(shrink, expected.1);
    assert_close(stretch, expected.2);
}

/// WN-10, WN-14: synthetic glue owns no bytes, is collapsible and of class
/// `Bn`.
#[track_caller]
fn assert_synthetic(glue: &GlueNode) {
    assert!(glue.kind.is_synthetic(), "{glue:?}");
    assert!(glue.byte_range.is_empty(), "{glue:?}");
    assert!(glue.is_collapsible, "{glue:?}");
    assert_eq!(glue.original_bidi_class, BidiClass::Bn, "{glue:?}");
}

/// The items at the zero-width offset `offset`, in order: `S<` a leading and
/// `S>` a trailing span boundary, `A` an autospace, `C` a compression glue
/// and `|` a break opportunity.
fn seam(ast: &ParagraphAst, offset: usize) -> Vec<&'static str> {
    ast.items
        .iter()
        .filter(|item| {
            let range = item.byte_span();
            range.is_empty() && range.start() == offset
        })
        .map(|item| match item {
            WordNode::Box(BoxNode::SpanBoundary {
                edge: SpanEdge::Leading,
                ..
            }) => "S<",
            WordNode::Box(BoxNode::SpanBoundary {
                edge: SpanEdge::Trailing,
                ..
            }) => "S>",
            WordNode::Glue(g) if g.kind == GlueKind::CjkAutospace => "A",
            WordNode::Glue(g) if g.kind == GlueKind::CjkPunctCompression => "C",
            WordNode::Discretionary(_) => "|",
            other => panic!("unexpected zero-width item {other:?}"),
        })
        .collect()
}

#[test]
fn fe20_autospace_lies_between_touching_cjk_and_western_atoms() {
    for (text, expected) in [
        ("漢字abc漢字", vec![6, 9]),
        ("a漢", vec![1]),
        ("漢a", vec![3]),
        ("あa", vec![3]),
        ("カナabc", vec![6]),
        // A decimal digit is of the Western class.
        ("漢1", vec![3]),
        ("2024年", vec![4]),
        // Two atoms of one class take none.
        ("漢字", vec![]),
        ("abc", vec![]),
    ] {
        let ast = compile_mixed(text, AutospaceProfile::Jlreq);
        assert_eq!(autospace_offsets(&ast), expected, "{text:?}");
        for (_, glue) in glues_of(&ast, GlueKind::CjkAutospace) {
            assert_synthetic(glue);
            assert_eq!(glue.kind.tier(), GlueTier::Tier1);
            // JLReq: 1/4 em, shrinking by 1/8 em, stretching by 1/4 em.
            assert_sizes(glue, (2.5, 1.25, 2.5));
        }
    }
}

#[test]
fn fe20_autospace_is_sized_from_the_cjk_atoms_style_by_profile() {
    // The CJK atom is set at 20 units per em and the Latin one at 10, in
    // either order: `em` is the CJK atom's size.
    let style = script_style(MIXED, 10.0);
    for (text, cjk) in [("漢a", TextRange::new(0, 3)), ("a漢", TextRange::new(1, 4))] {
        let spans = [StyleSpan {
            byte_range: cjk,
            style: script_style(MIXED, 20.0),
        }];
        let paragraph = ParagraphInput {
            style_spans: &spans,
            ..input(text, &style)
        };
        for (profile, expected) in [
            (AutospaceProfile::Jlreq, (5.0, 2.5, 5.0)),
            (AutospaceProfile::CssCompat, (2.5, 0.0, 0.0)),
        ] {
            let ast = compile_bidi_input(&paragraph, &options(profile));
            let [(_, glue)] = glues_of(&ast, GlueKind::CjkAutospace)[..] else {
                panic!("{text:?} {profile:?}: {:?}", ast.items)
            };
            assert_synthetic(glue);
            assert_sizes(glue, expected);
        }
    }
    // The default profile is JLReq.
    assert_eq!(AutospaceProfile::default(), AutospaceProfile::Jlreq);
}

#[test]
fn fe20_no_autospace_beside_punctuation_or_across_separators() {
    for text in [
        // Punctuation is of neither class.
        "漢(a)",
        "a(漢)",
        "漢「a」",
        "漢、a",
        "a,漢",
        "a.漢",
        // A full-width Latin letter is East Asian Width F: not Western.
        "漢ａ",
        // A word space, a tab or a hard break lies between them.
        "漢 a",
        "a 漢",
        "漢\ta",
        "漢\u{2028}a",
    ] {
        let ast = compile_mixed(text, AutospaceProfile::Jlreq);
        assert_eq!(autospace_offsets(&ast), [] as [usize; 0], "{text:?}");
    }
    // An inline object between them.
    let text = "漢\u{FFFC}a";
    let style = script_style(MIXED, 10.0);
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(3, 6),
        box_id: 1,
        width: 4.0,
        ascent: 4.0,
        descent: 1.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    let paragraph = ParagraphInput {
        inline_objects: &objects,
        ..input(text, &style)
    };
    let ast = compile_bidi_input(&paragraph, &FrontendOptions::default());
    assert_eq!(autospace_offsets(&ast), [] as [usize; 0]);
}

fn span(id: u64, range: (usize, usize), padding: (f32, f32)) -> InlineDecorationSpan {
    InlineDecorationSpan {
        span_id: id,
        byte_range: TextRange::new(range.0, range.1),
        start_padding: padding.0,
        end_padding: padding.1,
        break_mode: BoxDecorationBreak::Slice,
    }
}

fn compile_with_spans(text: &str, spans: &[InlineDecorationSpan], locale: &str) -> ParagraphAst {
    let style = localized_style(MIXED, 10.0, locale);
    let paragraph = ParagraphInput {
        decoration_spans: spans,
        ..input(text, &style)
    };
    compile_bidi_input(&paragraph, &FrontendOptions::default())
}

#[test]
fn spn4_a_padded_span_edge_forbids_autospace_on_both_sides() {
    // "漢字abc": A over "漢字" (0..6), B over "abc" (6..9), meeting at 6.
    let text = "漢字abc";
    for (a, b, expected) in [
        // Unpadded spans keep the glue: [A Trailing, Glue, Discretionary, B
        // Leading] (AST-8, FE-31).
        ((0.0, 0.0), (0.0, 0.0), vec!["S>", "A", "|", "S<"]),
        // Padding on the far edges does not reach offset 6.
        ((2.0, 0.0), (0.0, 2.0), vec!["S>", "A", "|", "S<"]),
        // A's end padding or B's start padding sits at 6: no glue.
        ((0.0, 2.0), (0.0, 0.0), vec!["S>", "|", "S<"]),
        ((0.0, 0.0), (2.0, 0.0), vec!["S>", "|", "S<"]),
    ] {
        let spans = [span(1, (0, 6), a), span(2, (6, 9), b)];
        let ast = compile_with_spans(text, &spans, "ja");
        assert_eq!(seam(&ast, 6), expected, "A {a:?}, B {b:?}");
    }
    // A single span over the Latin word: its padded start forbids the glue
    // before it, its unpadded end does not forbid the one after it.
    let text = "漢abc字";
    let spans = [span(1, (3, 6), (1.5, 0.0))];
    let ast = compile_with_spans(text, &spans, "ja");
    assert_eq!(autospace_offsets(&ast), [6]);
    assert_eq!(seam(&ast, 3), ["|", "S<"]);
    assert_eq!(seam(&ast, 6), ["S>", "A", "|"]);
}

#[test]
fn fe31_synthetic_glue_precedes_the_break_and_leading_boundaries_at_its_offset() {
    // Compression at a seam between two spans: "漢、" (0..6) and "「字"
    // (6..12) meet where 、 is followed by 「.
    let text = "漢、「字」";
    let spans = [span(1, (0, 6), (0.0, 0.0)), span(2, (6, 12), (0.0, 0.0))];
    let ast = compile_with_spans(text, &spans, "ja");
    assert_eq!(seam(&ast, 6), ["S>", "C", "|", "S<"]);
    // Without spans: the glue, then the break (AST-3).
    let ast = compile_mixed("漢字abc", AutospaceProfile::Jlreq);
    assert_eq!(seam(&ast, 6), ["A", "|"]);
    let ast = compile_localized("漢、「字」", &[JAPANESE], "ja");
    assert_eq!(seam(&ast, 6), ["C", "|"]);
}

#[test]
fn wn10_synthetic_glue_takes_the_level_of_the_entity_before_it() {
    // "١漢 漢١": the Arabic-Indic digits resolve to level 2 in an LTR
    // paragraph (class AN, I2) and the ideographs to 0.
    let text = "١漢 漢١";
    let style = script_style(&[ARABIC, JAPANESE], 10.0);
    let ast = compile_bidi_input(&input(text, &style), &FrontendOptions::default());
    assert_eq!(ast.base_direction, Direction::Ltr);
    let glue = glues_of(&ast, GlueKind::CjkAutospace);
    let levels: Vec<(usize, u8)> = glue
        .iter()
        .map(|(offset, g)| (*offset, g.bidi_level.get()))
        .collect();
    assert_eq!(levels, [(2, 2), (9, 0)]);
    for (_, g) in glue {
        assert_synthetic(g);
    }
    // In an RTL paragraph a Latin-and-CJK stretch is level 2, and so is the
    // glue between them.
    let text = "אב 漢a";
    let style = script_style(&[HEBREW, SERIF, JAPANESE], 10.0);
    let ast = compile_bidi_input(&input(text, &style), &FrontendOptions::default());
    assert_eq!(ast.base_direction, Direction::Rtl);
    let [(8, glue)] = glues_of(&ast, GlueKind::CjkAutospace)[..] else {
        panic!("{:?}", ast.items)
    };
    assert_eq!(glue.bidi_level.get(), 2);
}

#[test]
fn fe31_compression_lies_after_closing_or_stop_marks_and_between_openers() {
    for (text, expected) in [
        // 、 and 」 and 。 before full-width punctuation.
        ("漢、「字」。（あ）", vec![6, 15, 18]),
        // Two openers, and two closers.
        ("「（字）」", vec![3, 12]),
        // A closer before a closer.
        ("「字）」", vec![9]),
        // The full-width comma, full stop, colon and semicolon are stops.
        ("字，「字」", vec![6]),
        ("字．「字」", vec![6]),
        ("字：「字」", vec![6]),
        ("字；「字」", vec![6]),
        // ！ (class EX) is neither a closing nor a stop mark.
        ("字！「字」", vec![]),
        // A mark beside an ideograph, or an opener before a closer.
        ("漢」字", vec![]),
        ("「」", vec![]),
    ] {
        let ast = compile_localized(text, &[JAPANESE], "ja");
        assert_eq!(compression_offsets(&ast), expected, "{text:?}");
        for (_, glue) in glues_of(&ast, GlueKind::CjkPunctCompression) {
            assert_synthetic(glue);
            assert_eq!(glue.kind.tier(), GlueTier::Tier0);
            // WN-13: shrink only, half the font size.
            assert_sizes(glue, (0.0, 5.0, 0.0));
        }
    }
    // `max_shrink` is half the run's font size.
    let style = localized_style(&[JAPANESE], 16.0, "ja");
    let ast = compile_paragraph(
        &input("漢、「字」", &style),
        &FrontendOptions::default(),
        &*SPACING_SHAPER,
    )
    .unwrap();
    let [(6, glue)] = glues_of(&ast, GlueKind::CjkPunctCompression)[..] else {
        panic!("{:?}", ast.items)
    };
    assert_sizes(glue, (0.0, 8.0, 0.0));
}

#[test]
fn fe31_compression_is_off_for_traditional_chinese() {
    let text = "漢、「字」";
    for (locale, family, on) in [
        ("zh-Hans", SIMPLIFIED, true),
        ("zh-CN", SIMPLIFIED, true),
        ("zh", SIMPLIFIED, true),
        ("ja-JP", JAPANESE, true),
        // A script other than Hant, or a region other than TW.
        ("zh-Hans-TW", SIMPLIFIED, true),
        ("zh-HK", TRADITIONAL, true),
        // Hant, or TW with no script.
        ("zh-Hant", TRADITIONAL, false),
        ("zh-Hant-HK", TRADITIONAL, false),
        ("zh-TW", TRADITIONAL, false),
    ] {
        let ast = compile_localized(text, &[family], locale);
        let expected: Vec<usize> = if on { vec![6] } else { vec![] };
        assert_eq!(compression_offsets(&ast), expected, "{locale}");
    }
    // The default locale counts for untagged text.
    let style = script_style(&[TRADITIONAL], 10.0);
    let options = FrontendOptions {
        default_locale: LocaleTag::parse_lenient("zh-TW").unwrap(),
        ..Default::default()
    };
    let ast = compile_paragraph(&input(text, &style), &options, &*SPACING_SHAPER).unwrap();
    assert_eq!(compression_offsets(&ast), [] as [usize; 0]);
}

#[test]
fn fe31_compression_needs_two_wide_clusters_of_one_run() {
    // `halt` sets the marks half width: the pair is already narrow (both
    // clusters must be wider than 3/4 em).
    let mut style = localized_style(&[JAPANESE], 10.0, "ja");
    style.features.push(*b"halt", 1);
    let text = "漢、「字」";
    let ast = compile_paragraph(
        &input(text, &style),
        &FrontendOptions::default(),
        &*SPACING_SHAPER,
    )
    .unwrap();
    let marks: Vec<f32> = ast
        .clusters
        .iter()
        .filter(|c| !c.flags.contains(AtomFlags::IS_CJK_IDEOGRAPH))
        .map(|c| c.natural_width)
        .collect();
    assert!(marks.iter().all(|&w| w <= 7.5), "{marks:?}");
    assert_eq!(compression_offsets(&ast), [] as [usize; 0]);
    // 、 and 「 in two runs (another size for 「字」): no compression.
    let style = localized_style(&[JAPANESE], 10.0, "ja");
    let spans = [StyleSpan {
        byte_range: TextRange::new(6, 15),
        style: localized_style(&[JAPANESE], 12.0, "ja"),
    }];
    let paragraph = ParagraphInput {
        style_spans: &spans,
        ..input(text, &style)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SPACING_SHAPER).unwrap();
    assert!(ast.runs.len() >= 2);
    assert_eq!(compression_offsets(&ast), [] as [usize; 0]);
}

#[test]
fn fe31_wn13_middle_dot_compression_is_quarter_em() {
    // U+00B7 in Noto Sans SC is full-width (1000 units), so it compresses at 0.25 em.
    for (text, expected) in [("「·」", vec![3, 5]), ("漢。·字", vec![6])] {
        let ast = compile_localized(text, &[SIMPLIFIED], "zh-Hans");
        assert_eq!(compression_offsets(&ast), expected, "{text:?}");
        for (_, glue) in glues_of(&ast, GlueKind::CjkPunctCompression) {
            assert_synthetic(glue);
            assert_eq!(glue.kind.tier(), GlueTier::Tier0);
            assert_sizes(glue, (0.0, 2.5, 0.0));
        }
    }

    // Middle dots (U+30FB in Noto Sans JP, U+00B7 in Noto Sans SC) compress
    // against adjacent closing/stop marks or opening brackets/quotes, with
    // max_shrink == 0.25 * font_size (WN-13).
    for (text, expected) in [
        ("あ「・」", vec![6, 9]),
        ("あ「・い」", vec![6]),
        ("あ。・い", vec![6]),
        ("あ・「い」", vec![6]),
        ("あ「い・」", vec![12]),
        // Middle dot beside a Kana character does not compress.
        ("あ・い", vec![]),
    ] {
        let ast = compile_localized(text, &[JAPANESE], "ja");
        assert_eq!(compression_offsets(&ast), expected, "{text:?}");
        for (_, glue) in glues_of(&ast, GlueKind::CjkPunctCompression) {
            assert_synthetic(glue);
            assert_eq!(glue.kind.tier(), GlueTier::Tier0);
            assert_sizes(glue, (0.0, 2.5, 0.0));
        }
    }

    // Scaling with font_size: 0.25 * 16.0 == 4.0.
    let style = localized_style(&[JAPANESE], 16.0, "ja");
    let ast = compile_paragraph(
        &input("あ。・い", &style),
        &FrontendOptions::default(),
        &*SPACING_SHAPER,
    )
    .unwrap();
    let [(6, glue)] = glues_of(&ast, GlueKind::CjkPunctCompression)[..] else {
        panic!("{:?}", ast.items)
    };
    assert_synthetic(glue);
    assert_sizes(glue, (0.0, 4.0, 0.0));

    // U+00B7 in Noto Sans JP is proportional (<= 0.75 em), so it does not compress.
    let ast = compile_localized("漢。·字", &[JAPANESE], "ja");
    assert!(
        ast.clusters
            .iter()
            .any(|c| c.byte_range == TextRange::new(6, 8) && c.natural_width <= 7.5),
        "{:?}",
        ast.clusters
    );
    assert_eq!(compression_offsets(&ast), [] as [usize; 0]);
}

#[test]
fn fe31_curly_quotes_compress_only_when_wide() {
    // In a Latin/narrow font (Noto Serif / "en"), curly quotes are <= 0.75 em
    // and must not receive CjkPunctCompression.
    for text in ["‘“a”’", "”‘a’“", "“（a）”"] {
        let ast = compile_localized(text, &[SERIF, JAPANESE], "en");
        let quote_widths: Vec<f32> = ast
            .clusters
            .iter()
            .filter(|c| matches!(&text[c.byte_range.as_range()], "‘" | "’" | "“" | "”"))
            .map(|c| c.natural_width)
            .collect();
        assert!(!quote_widths.is_empty(), "{text:?}");
        assert!(
            quote_widths.iter().all(|&w| w <= 7.5),
            "{text:?}: {quote_widths:?}"
        );
        assert_eq!(compression_offsets(&ast), [] as [usize; 0], "{text:?}");
    }

    // In Noto Sans JP, curly quotes are proportional (<= 0.75 em), so no compression.
    let ast = compile_localized("“（中文）”", &[JAPANESE], "ja");
    assert_eq!(compression_offsets(&ast), [] as [usize; 0]);

    // In Noto Sans SC ("zh-Hans"), curly quotes are full-width (> 0.75 em),
    // so pairs with curly quotes receive CjkPunctCompression.
    for (text, expected) in [
        ("人：“（中文）”", vec![6, 9, 21]),
        ("“中文。”「日本」", vec![12, 15]),
        ("“‘中文’”", vec![3, 15]),
    ] {
        let ast = compile_localized(text, &[SIMPLIFIED], "zh-Hans");
        let punct_widths: Vec<(&str, f32)> = ast
            .clusters
            .iter()
            .filter(|c| !c.flags.contains(AtomFlags::IS_CJK_IDEOGRAPH))
            .map(|c| (&text[c.byte_range.as_range()], c.natural_width))
            .collect();
        assert!(
            punct_widths.iter().all(|&(_, w)| w > 7.5),
            "{text:?}: {punct_widths:?}"
        );
        assert_eq!(compression_offsets(&ast), expected, "{text:?}");
        for (_, glue) in glues_of(&ast, GlueKind::CjkPunctCompression) {
            assert_synthetic(glue);
            assert_eq!(glue.kind.tier(), GlueTier::Tier0);
            assert_sizes(glue, (0.0, 5.0, 0.0));
        }
    }
}

#[test]
fn fe31_cursive_scripts_are_flagged_for_no_letter_spacing() {
    let text = "abc אבג مرحبا";
    let style = script_style(&[HEBREW, ARABIC], 10.0);
    let ast = compile_bidi_input(&input(text, &style), &FrontendOptions::default());
    for atom in &ast.atoms {
        let arabic = atom.script == ScriptTag::ARABIC;
        let flags = atom.flags;
        assert_eq!(
            flags.contains(AtomFlags::NO_LETTER_SPACING),
            arabic,
            "{:?}",
            &text[atom.byte_range.as_range()]
        );
        assert_eq!(
            flags.contains(AtomFlags::KASHIDA_CAPABLE),
            arabic,
            "{:?}",
            &text[atom.byte_range.as_range()]
        );
    }
    assert!(ast.atoms.iter().any(|a| a.script == ScriptTag::ARABIC));
}

#[test]
fn fe31_hangable_marks_are_flagged() {
    let text = "(a) [b] “c” ‘d’ «e» 「漢」 f, g. 漢、漢。漢，漢． h! i; j: k?";
    let style = script_style(&[SERIF, HEBREW, JAPANESE], 10.0);
    let ast = compile_bidi_input(&input(text, &style), &FrontendOptions::default());
    let start: Vec<&str> = ast
        .atoms
        .iter()
        .filter(|a| a.flags.contains(AtomFlags::HANGABLE_START))
        .map(|a| &text[a.byte_range.as_range()])
        .collect();
    let end: Vec<&str> = ast
        .atoms
        .iter()
        .filter(|a| a.flags.contains(AtomFlags::HANGABLE_END))
        .map(|a| &text[a.byte_range.as_range()])
        .collect();
    // Class OP, or General Category Ps or Pi.
    assert_eq!(start, ["(", "[", "“", "‘", "«", "「"]);
    // U+002C, U+002E, U+3001, U+3002, U+FF0C and U+FF0E only.
    assert_eq!(end, [",", ".", "、", "。", "，", "．"]);
}

#[test]
fn fe31_cjk_justification_opportunities_follow_cjk_atoms() {
    let allowed = |text: &str| -> Vec<usize> {
        let ast = compile_mixed(text, AutospaceProfile::Jlreq);
        ast.atoms
            .iter()
            .filter(|a| a.flags.contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER))
            .map(|a| a.byte_range.start())
            .collect()
    };
    // Every CJK atom but the last of the paragraph.
    assert_eq!(allowed("漢字漢"), [0, 3]);
    assert_eq!(allowed("あいう"), [0, 3]);
    // Nor the last one before a hard break.
    assert_eq!(allowed("漢字\u{2028}漢字"), [0, 9]);
    // Nor one before closing or prohibited-start punctuation.
    for text in [
        "漢」", "漢）", "漢、", "漢。", "漢，", "漢．", "漢！", "漢？",
    ] {
        assert_eq!(allowed(text), [] as [usize; 0], "{text:?}");
    }
    // An opening mark does not prohibit it; punctuation itself is not CJK.
    assert_eq!(allowed("漢「字」"), [0]);
    // Nor one before glue (autospace or a word space), a tab or an object.
    assert_eq!(allowed("漢a"), [] as [usize; 0]);
    assert_eq!(allowed("漢 字"), [] as [usize; 0]);
    assert_eq!(allowed("漢\t字"), [] as [usize; 0]);
    let text = "漢\u{FFFC}字";
    let style = script_style(MIXED, 10.0);
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(3, 6),
        box_id: 1,
        width: 4.0,
        ascent: 4.0,
        descent: 1.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    let paragraph = ParagraphInput {
        inline_objects: &objects,
        ..input(text, &style)
    };
    let ast = compile_bidi_input(&paragraph, &FrontendOptions::default());
    assert!(!flags_at(&ast, 0).contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER));
    // Latin letters never have it.
    assert_eq!(allowed("abc"), [] as [usize; 0]);
}

#[test]
fn fe27_a_cursive_run_takes_no_letter_spacing() {
    // The paragraph asks for 2 units of letter spacing throughout.
    let text = "abc مرحبا אבג";
    let spaced = TextStyle {
        letter_spacing: 2.0,
        ..script_style(&[SERIF, ARABIC, HEBREW], 10.0)
    };
    let plain = script_style(&[SERIF, ARABIC, HEBREW], 10.0);
    let options = FrontendOptions {
        base_direction: Some(Direction::Ltr),
        ..Default::default()
    };
    let ast = compile_bidi_input(&input(text, &spaced), &options);
    let reference = compile_bidi_input(&input(text, &plain), &options);
    for run in &ast.runs {
        let cursive = run.script == ScriptTag::ARABIC;
        assert_eq!(
            run.letter_spacing,
            if cursive { 0.0 } else { 2.0 },
            "{:?}",
            run.script
        );
    }
    assert!(ast.runs.iter().any(|r| r.script == ScriptTag::ARABIC));
    // The Arabic clusters keep their natural widths; the others gain 2.
    assert_eq!(ast.clusters.len(), reference.clusters.len());
    for (c, r) in ast.clusters.iter().zip(&reference.clusters) {
        let extra = if c.flags.contains(AtomFlags::NO_LETTER_SPACING) {
            0.0
        } else {
            2.0
        };
        assert_close(c.natural_width, r.natural_width + extra);
    }
    // The AST passes the input gate (AST-5 forbids letter spacing on a run
    // with a `NO_LETTER_SPACING` atom).
    lay_out_bidi_ast(&ast, text, &column(300.0), &LayoutOptions::default());
}
