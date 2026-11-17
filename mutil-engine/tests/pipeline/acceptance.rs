//! The Phase 1 acceptance criteria (spec RM-4), one test per clause. Two
//! clauses are tested where their machinery lives:
//!
//! - the `!` run, in `layout::a_run_of_exclamation_marks_splits_only_once_it_starts_an_empty_line`,
//!   and no line starting with CL, CP, EX, IS or SY while its unit fits, in
//!   `layout::closing_punctuation_never_starts_a_line_while_its_unit_fits`;
//! - Backspace removing a combining mark alone and an emoji whole, in
//!   `queries::deletion_removes_marks_alone_and_emoji_objects_and_chips_whole`.

use crate::common::*;
use mutil_engine::*;

fn offsets(ast: &ParagraphAst) -> Vec<usize> {
    ast.items
        .iter()
        .filter_map(|item| match item {
            WordNode::Discretionary(d) if !d.is_paragraph_end() => Some(d.byte_offset()),
            _ => None,
        })
        .collect()
}

#[test]
fn rm4_a_devanagari_conjunct_is_one_atom_shaped_in_one_run_and_font() {
    // Unicode 18.0 data, GB9c: क्ष is one atom.
    let text = "\u{915}\u{94D}\u{937}\u{93E}";
    let ast = compile_in(text, &["Noto Sans Devanagari"]);
    assert_eq!(ast.atoms.len(), 1);
    assert_eq!(ast.runs.len(), 1);
    assert_eq!(ast.runs[0].script, ScriptTag::try_from_str("Deva").unwrap());
    let font = ast.clusters[0].font;
    assert!(ast.clusters.iter().all(|c| c.font == font));
    assert_eq!(font.font_id, FontId(2));
    let dotted = glyph_for(SCRIPT_FONTS[1].1, '\u{25CC}');
    assert!(
        ast.glyphs
            .iter()
            .all(|g| Some(g.glyph_id) != dotted && g.glyph_id != 0)
    );
}

#[test]
fn rm4_southeast_asian_text_breaks_at_the_lstm_model_boundaries() {
    // The model is ICU4X's LSTM: `LineSegmenter::new_auto`, which FE-18's
    // segmenter uses, selects it whenever the `lstm` feature is on, as the
    // default `auto` feature makes it. The frontend's opportunities are
    // exactly that segmenter's.
    use icu_segmenter::LineSegmenter;
    use icu_segmenter::options::{LineBreakOptions, LineBreakStrictness as Strictness};
    let mut options = LineBreakOptions::default();
    options.strictness = Some(Strictness::Normal);
    let lstm = LineSegmenter::new_lstm(options);
    for (text, family) in [
        ("ภาษาไทยง่ายนิดเดียว", "Noto Sans Thai"),
        ("ພາສາລາວງ່າຍຫຼາຍ", "Noto Sans Lao"),
        ("ភាសាខ្មែរងាយស្រួលណាស់", "Noto Sans Khmer"),
        ("မြန်မာဘာသာစကားလွယ်ကူသည်", "Noto Sans Myanmar"),
    ] {
        let ast = compile_in(text, &[family]);
        let model: Vec<usize> = lstm
            .segment_str(text)
            .filter(|&b| b > 0 && b < text.len())
            .collect();
        assert!(!model.is_empty(), "{family}: the model finds word breaks");
        assert_eq!(offsets(&ast), model, "{family}");
        // A column as wide as the widest word: every line but the last
        // ends at a model boundary.
        let words: Vec<usize> = std::iter::once(0)
            .chain(model.iter().copied())
            .chain([text.len()])
            .collect();
        let widest = words
            .windows(2)
            .map(|w| {
                let word = &text[w[0]..w[1]];
                let ast = compile_in(word, &[family]);
                ast.clusters.iter().map(|c| c.natural_width).sum::<f32>()
            })
            .fold(0.0, f32::max);
        let p = lay_out_in(
            text,
            &[family],
            &column(widest + 0.01),
            &LayoutOptions::default(),
        );
        assert!(p.lines.len() > 1, "{family} wraps");
        for line in &p.lines[..p.lines.len() - 1] {
            assert!(
                model.contains(&line.byte_range.end()),
                "{family}: {:?}",
                &text[line.byte_range.as_range()]
            );
        }
    }
}

#[test]
fn rm4_cjk_quotes_ellipses_and_dashes_take_their_script_and_stay_whole() {
    let text = "東京「テスト」……新しい——終わり。";
    let ast = compile_in(text, &["Noto Sans JP"]);
    let script_at = |c: char| {
        let at = text.find(c).unwrap();
        ast.atoms
            .iter()
            .find(|a| a.byte_range.start() == at)
            .map(|a| a.script)
            .unwrap()
    };
    let cjk = ["Hani", "Hira", "Kana"].map(|s| ScriptTag::try_from_str(s).unwrap());
    for c in ['「', '」', '…', '—', '。'] {
        assert!(cjk.contains(&script_at(c)), "{c} takes {:?}", script_at(c));
    }
    // The pair takes one script, and so does each run.
    assert_eq!(script_at('「'), script_at('」'));
    let breaks = offsets(&ast);
    for c in ['…', '—'] {
        let at = text.find(c).unwrap();
        assert!(
            !breaks.contains(&(at + c.len_utf8())),
            "no break inside {c}{c}"
        );
    }
    // Laid out at every width, no line splits a run or starts with a
    // closing mark while its unit fits.
    for width in (20..=200).step_by(5) {
        let p = lay_out_in(
            text,
            &["Noto Sans JP"],
            &column(width as f32),
            &LayoutOptions::default(),
        );
        for line in &p.lines {
            let s = &text[line.byte_range.as_range()];
            assert!(!s.starts_with('」') && !s.starts_with('。'), "{width}: {s}");
            assert!(!s.starts_with("…") || s.starts_with("……"), "{width}: {s}");
            assert!(!s.starts_with("—") || s.starts_with("——"), "{width}: {s}");
        }
    }
}

#[test]
fn rm4_emoji_sequences_move_the_caret_atomically() {
    let text = "a\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{1F1EF}\u{1F1F5}b";
    let family = TextRange::new(1, 19);
    let flag = TextRange::new(19, 27);
    let p = lay_out(text, &column(200.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    let next = |from: usize| match l.move_cursor(
        CursorPosition {
            byte_offset: from,
            affinity: CaretAffinity::Downstream,
        },
        Motion::Next,
        None,
    ) {
        CursorMove::Moved(p) => p.byte_offset,
        other => panic!("{other:?}"),
    };
    assert_eq!(next(1), family.end());
    assert_eq!(next(family.end()), flag.end());
    // No caret stop inside either sequence.
    for stop in &p.lines[0].caret_stops {
        assert!(!(family.start() < stop.offset && stop.offset < family.end()));
        assert!(!(flag.start() < stop.offset && stop.offset < flag.end()));
    }
    let ast = compile(text);
    let (constraints, options) = (column(200.0), LayoutOptions::default());
    let inputs = ValidatedInputs::new(&ast, text, &constraints, &options).unwrap();
    assert_eq!(inputs.delete_backward_range(family.end()), family);
    assert_eq!(inputs.delete_forward_range(flag.start()), flag);
}

#[test]
fn rm4_long_urls_wrap_without_overflow() {
    let text = "Read https://www.example.com/documentation/typesetting/line-breaking/\
                abcdefghijklmnopqrstuvwxyz0123456789?section=urls&lang=en today.";
    for width in [60.0, 90.0, 130.0] {
        let p = lay_out(text, &column(width), &LayoutOptions::default());
        assert!(p.lines.len() > 2, "{width}");
        for line in &p.lines {
            let end = line.caret_stops.iter().map(|s| s.x).fold(0.0, f32::max);
            assert!(
                end <= width + 1e-3,
                "{width}: {:?} ends at {end}",
                &text[line.byte_range.as_range()]
            );
        }
    }
}

#[test]
fn rm4_the_conformance_report_names_the_data_actually_used() {
    let r = conformance_report();
    let pinned = Some(UNICODE_DATA_VERSION);
    assert_eq!(
        (r.grapheme, r.bidi, r.line_break, r.word, r.script, r.emoji),
        (pinned, pinned, pinned, pinned, pinned, pinned)
    );
    // The conformance suites run against the same version (RM-1).
    for file in [
        "GraphemeBreakTest",
        "WordBreakTest",
        "LineBreakTest",
        "BidiTest",
        "BidiCharacterTest",
    ] {
        let path = format!("{}/tests/assets/ucd/{file}.txt", env!("CARGO_MANIFEST_DIR"));
        let data = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            data.lines().next(),
            Some(format!("# {file}-{UNICODE_DATA_VERSION}.txt").as_str())
        );
    }
}

#[test]
fn rm4_a_mention_chip_is_one_caret_unit_wrapped_or_not() {
    let text = "hi @alice smith ok";
    let chip = TextRange::new(3, 15);
    let base = style(10.0);
    let spans = [AtomicEditSpan {
        span_id: 1,
        byte_range: chip,
    }];
    let paragraph = ParagraphInput {
        atomic_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SHAPER).unwrap();
    // Unwrapped (one line) and wrapped across two lines.
    for (width, wrapped) in [(200.0, false), (70.0, true)] {
        let constraints = column(width);
        let options = LayoutOptions::default();
        let p = layout_paragraph(&ast, text, &*SHAPER, &constraints, &options).unwrap();
        let l = ValidatedLayout::new(&p, text).unwrap();
        let lines_holding: Vec<usize> = p
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.atomic_fragments.is_empty())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(lines_holding.len() > 1, wrapped, "{width}");
        for (i, line) in p.lines.iter().enumerate() {
            for stop in &line.caret_stops {
                assert!(!spans[0].is_interior(stop.offset), "{width}: stop {stop:?}");
            }
            // A click inside any fragment snaps to one of the chip's ends.
            for fragment in &line.atomic_fragments {
                let hit = l.hit_test_on_line(i, fragment.x + fragment.width * 0.4);
                assert!(
                    hit.position.byte_offset == chip.start()
                        || hit.position.byte_offset == chip.end(),
                    "{width}: {hit:?}"
                );
            }
        }
        let inputs = ValidatedInputs::new(&ast, text, &constraints, &options).unwrap();
        assert_eq!(inputs.delete_backward_range(chip.end()), chip);
        assert_eq!(inputs.delete_forward_range(chip.start()), chip);
    }
}
