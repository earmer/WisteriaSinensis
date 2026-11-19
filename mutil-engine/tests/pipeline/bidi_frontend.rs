//! The Phase 2 frontend: base direction, atom levels, explicit bidi spans,
//! span-boundary and glue levels (spec FE-8 to FE-12, FE-28, FE-30, WN-8,
//! WN-14), and face coverage under normalisation (SHP-26).

use crate::acceptance_phase2::{level_at, x_of};
use crate::common::*;
use mutil_engine::*;

fn options(base: Option<Direction>) -> FrontendOptions {
    FrontendOptions {
        base_direction: base,
        ..Default::default()
    }
}

fn hebrew_style() -> TextStyle {
    script_style(&[HEBREW], 10.0)
}

fn compile_with(
    text: &str,
    base: Option<Direction>,
    bidi_spans: &[ExplicitBidiSpan],
) -> Result<ParagraphAst, FrontendError> {
    let style = hebrew_style();
    let paragraph = ParagraphInput {
        bidi_spans,
        ..input(text, &style)
    };
    compile_paragraph(&paragraph, &options(base), &*BIDI_SHAPER)
}

fn span(id: u64, start: usize, end: usize, control: BidiControlKind) -> ExplicitBidiSpan {
    ExplicitBidiSpan {
        span_id: id,
        byte_range: TextRange::new(start, end),
        control,
    }
}

fn isolate(id: u64, start: usize, end: usize) -> ExplicitBidiSpan {
    span(id, start, end, BidiControlKind::Isolate(None))
}

#[test]
fn fe8_the_first_strong_character_outside_isolates_sets_the_base_direction() {
    for (text, expected) in [
        ("אבג abc", Direction::Rtl),
        ("abc אבג", Direction::Ltr),
        ("مرحبا abc", Direction::Rtl),
        // Digits and neutrals are not strong.
        ("123 (אבג)", Direction::Rtl),
        ("123 !", Direction::Ltr),
        ("", Direction::Ltr),
        // P2 skips the content of an in-text isolate.
        ("\u{2068}abc\u{2069}אבג", Direction::Rtl),
        ("\u{2067}אבג\u{2069}abc", Direction::Ltr),
    ] {
        let ast = compile_with(text, None, &[]).unwrap();
        assert_eq!(ast.base_direction, expected, "{text:?}");
    }
    // ... and of an isolate span (BID-3).
    let ast = compile_with("abcאבג", None, &[isolate(1, 0, 3)]).unwrap();
    assert_eq!(ast.base_direction, Direction::Rtl);
    // HL1: the caller's direction wins.
    let ast = compile_with("אבג abc", Some(Direction::Ltr), &[]).unwrap();
    assert_eq!(ast.base_direction, Direction::Ltr);
    assert_eq!((level_at(&ast, 0), level_at(&ast, 7)), (1, 0));
    let ast = compile_with("abc", Some(Direction::Rtl), &[]).unwrap();
    assert_eq!(ast.base_direction, Direction::Rtl);
    assert_eq!(level_at(&ast, 0), 2);
}

#[test]
fn fe10_removed_controls_take_the_level_before_them_and_are_l1_resettable() {
    // RLO b c PDF in an LTR paragraph: b and c are overridden to level 1;
    // RLO takes the level of "a", PDF that of "c".
    let text = "a\u{202E}bc\u{202C}d";
    let ast = compile_with(text, None, &[]).unwrap();
    for (offset, level) in [(0, 0), (1, 0), (4, 1), (5, 1), (6, 1), (9, 0)] {
        assert_eq!(level_at(&ast, offset), level, "atom at {offset}");
    }
    let flagged = |ast: &ParagraphAst, offset: usize| {
        ast.atoms
            .iter()
            .find(|a| a.byte_range.start() == offset)
            .unwrap()
            .flags
            .contains(AtomFlags::L1_RESETTABLE)
    };
    assert!(flagged(&ast, 1) && flagged(&ast, 6));
    assert!(!flagged(&ast, 0) && !flagged(&ast, 4) && !flagged(&ast, 9));
    let p = lay_out_bidi_ast(&ast, text, &column(100.0), &LayoutOptions::default());
    let line = &p.lines[0];
    assert!(x_of(line, 0) < x_of(line, 5) && x_of(line, 5) < x_of(line, 4));
    assert!(x_of(line, 4) < x_of(line, 9));
    // A word joiner (BN) after Hebrew in an LTR paragraph takes ב's level;
    // at the paragraph start, the base level.
    let ast = compile_with("אב\u{2060}cd", Some(Direction::Ltr), &[]).unwrap();
    assert_eq!(level_at(&ast, 4), 1);
    assert!(flagged(&ast, 4));
    let ast = compile_with("\u{2060}אב", Some(Direction::Ltr), &[]).unwrap();
    assert_eq!(level_at(&ast, 0), 0);
    // An ideographic space (WS) is text, flagged; isolate initiators and PDI
    // keep the outer level, their content rises.
    let text = "a\u{3000}b \u{2067}cd\u{2069} e";
    let ast = compile_with(text, None, &[]).unwrap();
    assert!(flagged(&ast, 1));
    let rli = text.find('\u{2067}').unwrap();
    let pdi = text.find('\u{2069}').unwrap();
    assert!(flagged(&ast, rli) && flagged(&ast, pdi));
    assert_eq!((level_at(&ast, rli), level_at(&ast, pdi)), (0, 0));
    assert_eq!(level_at(&ast, rli + 3), 2);
    assert!(!flagged(&ast, 0) && !flagged(&ast, rli + 3));
}

#[test]
fn fe12_explicit_bidi_spans_act_as_isolates_and_overrides() {
    let rtl = Some(Direction::Rtl);
    let ltr = Some(Direction::Ltr);
    // An RTL override over "bc" in "abcd": level 1, drawn c then b.
    let spans = [span(9, 1, 3, BidiControlKind::Override(Direction::Rtl))];
    let ast = compile_with("abcd", None, &spans).unwrap();
    assert_eq!(ast.bidi_spans, spans);
    assert_eq!(
        (0..4).map(|o| level_at(&ast, o)).collect::<Vec<_>>(),
        [0, 1, 1, 0]
    );
    let p = lay_out_bidi_ast(&ast, "abcd", &column(100.0), &LayoutOptions::default());
    let line = &p.lines[0];
    assert!(x_of(line, 2) < x_of(line, 1));
    // FE-30: a text box ends at each edge of the span.
    let box_edges: Vec<(usize, usize)> = ast
        .items
        .iter()
        .filter_map(|n| match n {
            WordNode::Box(BoxNode::Text(TextBox { byte_range, .. })) => {
                Some((byte_range.start(), byte_range.end()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(box_edges, [(0, 1), (1, 3), (3, 4)]);
    // An RTL isolate over Latin: L at an odd level rises to 2 (I2).
    let spans = [span(1, 2, 5, BidiControlKind::Isolate(rtl))];
    let ast = compile_with("x abc y", None, &spans).unwrap();
    assert_eq!(ast.base_direction, Direction::Ltr);
    assert_eq!((level_at(&ast, 0), level_at(&ast, 2)), (0, 2));
    assert_eq!(level_at(&ast, 6), 0);
    // An LTR isolate over Latin in an LTR paragraph: level 2.
    let spans = [span(1, 2, 5, BidiControlKind::Isolate(ltr))];
    let ast = compile_with("x abc y", None, &spans).unwrap();
    assert_eq!(level_at(&ast, 2), 2);
    // A first-strong isolate over Hebrew: RLI, level 1.
    let spans = [isolate(1, 2, 8)];
    let ast = compile_with("x אבג y", None, &spans).unwrap();
    assert_eq!(level_at(&ast, 2), 1);
    // An LTR override over Hebrew in an RTL paragraph: level 2, left to
    // right.
    let text = "אב גד הו";
    let spans = [span(1, 5, 9, BidiControlKind::Override(Direction::Ltr))];
    let ast = compile_with(text, None, &spans).unwrap();
    assert_eq!(ast.base_direction, Direction::Rtl);
    assert_eq!((level_at(&ast, 5), level_at(&ast, 7)), (2, 2));
    assert_eq!(level_at(&ast, 0), 1);
    let p = lay_out_bidi_ast(&ast, text, &column(100.0), &LayoutOptions::default());
    let line = &p.lines[0];
    assert!(x_of(line, 5) < x_of(line, 7));
    assert!(x_of(line, 7) < x_of(line, 0));
}

#[test]
fn fe12_objects_are_neutrals_and_word_spaces_take_their_resolved_level() {
    // An object (class ON) between two R letters in an LTR paragraph is R;
    // between L and R it takes the embedding direction.
    let style = hebrew_style();
    for (text, level) in [("א\u{FFFC}ב", 1), ("a\u{FFFC}ב", 0)] {
        let start = text.find('\u{FFFC}').unwrap();
        let objects = [InlineObjectInput {
            byte_range: TextRange::new(start, start + 3),
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
        let ast = compile_bidi_input(&paragraph, &options(Some(Direction::Ltr)));
        let object = ast
            .items
            .iter()
            .find_map(|n| match n {
                WordNode::Box(BoxNode::InlineObject(InlineObjectBox { bidi_level, .. })) => {
                    Some(*bidi_level)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(object.get(), level, "{text:?}");
    }
    // WN-14: a word space is class WS at its resolved level: 2 between two
    // Latin words of an RTL paragraph, 1 between Hebrew and Latin.
    let text = "אב cd ef";
    let ast = compile_with(text, None, &[]).unwrap();
    let glue: Vec<(usize, u8, BidiClass, GlueKind)> = ast
        .items
        .iter()
        .filter_map(|n| match n {
            WordNode::Glue(g) => Some((
                g.byte_range.start(),
                g.bidi_level.get(),
                g.original_bidi_class,
                g.kind,
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        glue,
        [
            (4, 1, BidiClass::Ws, GlueKind::WordSpace),
            (7, 2, BidiClass::Ws, GlueKind::WordSpace)
        ]
    );
}

#[test]
fn wn8_span_boundaries_take_the_level_of_the_first_and_last_inline_entity() {
    let style = hebrew_style();
    let compile = |text: &str, range: TextRange, objects: &[InlineObjectInput]| {
        let spans = [InlineDecorationSpan {
            span_id: 1,
            byte_range: range,
            start_padding: 1.0,
            end_padding: 1.0,
            break_mode: BoxDecorationBreak::Slice,
        }];
        let paragraph = ParagraphInput {
            decoration_spans: &spans,
            inline_objects: objects,
            ..input(text, &style)
        };
        let ast = compile_bidi_input(&paragraph, &FrontendOptions::default());
        let levels: Vec<(SpanEdge, u8)> = ast
            .items
            .iter()
            .filter_map(|n| match n {
                WordNode::Box(BoxNode::SpanBoundary {
                    edge, bidi_level, ..
                }) => Some((*edge, bidi_level.get())),
                _ => None,
            })
            .collect();
        (ast, levels)
    };
    use SpanEdge::{Leading, Trailing};
    // Hebrew inside an LTR paragraph.
    let (_, levels) = compile("ab אבג cd", TextRange::new(3, 9), &[]);
    assert_eq!(levels, [(Leading, 1), (Trailing, 1)]);
    // Latin first, Hebrew last.
    let (_, levels) = compile("ab אבג cd", TextRange::new(0, 9), &[]);
    assert_eq!(levels, [(Leading, 0), (Trailing, 1)]);
    // A span that holds only a word space between two Latin words of an RTL
    // paragraph: the base level, not the space's level 2.
    let (ast, levels) = compile("אב cd ef", TextRange::new(7, 8), &[]);
    assert_eq!(ast.base_direction, Direction::Rtl);
    assert_eq!(levels, [(Leading, 1), (Trailing, 1)]);
    // A span that starts with an object: the object's level (0, between L
    // and R), not that of its first atom (1).
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(1, 4),
        box_id: 1,
        width: 4.0,
        ascent: 4.0,
        descent: 1.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    let (_, levels) = compile("a\u{FFFC}אב", TextRange::new(1, 8), &objects);
    assert_eq!(levels, [(Leading, 0), (Trailing, 1)]);
}

#[test]
fn fe28_invalid_bidi_spans_are_reported_by_index() {
    let iso = |id, s, e| isolate(id, s, e);
    let text = "abcd e\u{301}f";
    for (spans, bad) in [
        // A repeated id.
        (vec![iso(1, 0, 2), iso(1, 2, 4)], 1),
        // Empty.
        (vec![iso(1, 2, 2)], 0),
        // Outside the text.
        (vec![iso(1, 0, 40)], 0),
        // An edge inside the atom "e\u{301}" (5..8).
        (vec![iso(1, 0, 6)], 0),
        // Out of order.
        (vec![iso(1, 2, 4), iso(2, 0, 2)], 1),
        // Overlapping without nesting.
        (vec![iso(1, 0, 3), iso(2, 1, 4)], 1),
        // The same range as its predecessor.
        (vec![iso(1, 0, 4), iso(2, 0, 4)], 1),
    ] {
        assert_eq!(
            compile_with(text, None, &spans).map(|_| ()),
            Err(FrontendError::InvalidBidiSpan { span: bad }),
            "{spans:?}"
        );
    }
    // Nested properly, equal starts descending by end: accepted.
    let spans = [iso(1, 0, 4), iso(2, 0, 2), iso(3, 1, 2)];
    assert!(compile_with(text, None, &spans).is_ok());
    // Depth: 63 nested spans pass, a 64th is too deep.
    let long = "a".repeat(140);
    let nested: Vec<ExplicitBidiSpan> = (0..64).map(|i| iso(i as u64, i, 140 - i)).collect();
    assert!(compile_with(&long, None, &nested[..63]).is_ok());
    assert_eq!(
        compile_with(&long, None, &nested).map(|_| ()),
        Err(FrontendError::InvalidBidiSpan { span: 63 })
    );
    // An edge inside an inline object.
    let style = hebrew_style();
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(1, 4),
        box_id: 1,
        width: 4.0,
        ascent: 4.0,
        descent: 1.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    let object_text = "a\u{FFFC}b";
    for (range, ok) in [((0, 2), false), ((0, 4), true), ((1, 4), true)] {
        let spans = [iso(1, range.0, range.1)];
        let paragraph = ParagraphInput {
            inline_objects: &objects,
            bidi_spans: &spans,
            ..input(object_text, &style)
        };
        let result = compile_paragraph(&paragraph, &FrontendOptions::default(), &*BIDI_SHAPER);
        if ok {
            assert!(result.is_ok(), "{range:?}: {result:?}");
        } else {
            assert_eq!(
                result.map(|_| ()),
                Err(FrontendError::InvalidBidiSpan { span: 0 })
            );
        }
    }
}

#[test]
fn fe28_bidi_input_is_no_longer_refused() {
    // RTL paragraphs, bidi text in an LTR paragraph and bidi spans compile
    // (FE-28: those `Unsupported` variants are frozen and no longer
    // returned), and the result passes the input gate (FE-26).
    let spans = [span(1, 0, 3, BidiControlKind::Override(Direction::Rtl))];
    for (text, base, spans) in [
        ("אבג", Some(Direction::Rtl), &[][..]),
        ("abc אבג", None, &[][..]),
        ("abc", None, &spans[..]),
    ] {
        let ast = compile_with(text, base, spans).unwrap();
        check_layout_inputs(&ast, text, &column(100.0), &LayoutOptions::default()).unwrap();
    }
}

#[test]
fn shp26_a_face_covers_an_atom_through_its_normal_forms() {
    // U+212A KELVIN SIGN and U+037E GREEK QUESTION MARK decompose canonically
    // to "K" and ";", which no face maps as written. Noto Sans Thai maps
    // neither form; Noto Sans Hebrew maps "K" and ";", so it covers both atoms
    // and shapes them with real glyphs.
    let text = "\u{212A}\u{37E}";
    let ast = compile_bidi(text, &["Noto Sans Thai", HEBREW]);
    assert_eq!(ast.clusters.len(), 2);
    let hebrew = RTL_FONTS[0].1;
    for (cluster, c) in ast.clusters.iter().zip(['K', ';']) {
        assert_eq!(cluster.font.font_id, HEBREW_ID, "{c}");
        assert_eq!(
            ast.cluster_glyphs(cluster)
                .iter()
                .map(|g| g.glyph_id)
                .collect::<Vec<_>>(),
            [glyph_for(hebrew, c).unwrap()],
            "{c}"
        );
    }
    // か + U+3099 (one atom): Noto Sans JP maps its NFC form が but not
    // U+3099; Noto Serif maps neither.
    let text = "\u{304B}\u{3099}";
    let ast = compile_bidi(text, &["Noto Serif", "Noto Sans JP"]);
    assert_eq!(ast.atoms.len(), 1);
    assert!(ast.clusters.iter().all(|c| c.font.font_id == FontId(3)));
    assert!(ast.glyphs.iter().all(|g| g.glyph_id != 0));
}
