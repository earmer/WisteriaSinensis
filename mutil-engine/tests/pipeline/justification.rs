//! Phase 2 justification (spec ME-3, ST1-7, ST1-12, ST3-1 to ST3-5,
//! `PositionedLine::is_justified`): optimistic breaking with shrink, the
//! priority ladder, proportional sharing, the tolerance rule, the scripts
//! that expand at word separators only, tabs, and synthetic glue at line
//! edges. Every layout passes the output gate (the `lay_out*` helpers).

use crate::acceptance_phase2::{glyph_at, glyphs, x_of};
use crate::common::*;
use mutil_engine::*;

/// The monospace face: every glyph and the space 6 units at 10 per em, so a
/// word space has a stretch of 3 and a shrink of 2 (FE-27).
const CHAR: f32 = 6.0;

pub fn justified() -> LayoutOptions {
    LayoutOptions {
        justify: true,
        ..Default::default()
    }
}

/// The right edge of `line`'s content.
fn content_end(line: &PositionedLine) -> f32 {
    line.slots[0].content_end()
}

/// Lays out `paragraph` in the monospace face, through the output gate.
#[track_caller]
pub fn lay_out_mono(
    paragraph: &ParagraphInput<'_>,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> PositionedParagraph {
    let ast = compile_paragraph(paragraph, &FrontendOptions::default(), &*SHAPER).unwrap();
    let text = paragraph.text;
    let p = layout_paragraph(&ast, text, &*SHAPER, constraints, options)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    if let Err((line, error)) = ValidatedLayout::new(&p, text) {
        panic!("{text:?}: line {line}: {error}\n{p:#?}");
    }
    p
}

#[test]
fn st1_7_a_justified_greedy_line_breaks_optimistically_and_compresses_flush() {
    // "aaa bbb ccc" is 66 wide, 2 over a 63-unit column, and its two word
    // spaces can shrink by 4 together: under `justify` it fits, compressed
    // (ST3-3); at natural width it does not.
    let text = "aaa bbb ccc dddddddddd";
    let p = lay_out(text, &column(63.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["aaa bbb ", "ccc ", "dddddddddd"]);
    assert!(p.lines.iter().all(|l| !l.is_justified));
    let p = lay_out(text, &column(63.0), &justified());
    assert_eq!(line_texts(&p, text), ["aaa bbb ccc ", "dddddddddd"]);
    let line = &p.lines[0];
    assert!(line.is_justified);
    assert_close(line.visual_width(), 63.0);
    // The 3 units are taken equally from the two spaces (equal max_shrink).
    assert_close(x_of(line, 4), 3.0 * CHAR + CHAR - 1.5);
    assert_close(x_of(line, 8), 6.0 * CHAR + 2.0 * CHAR - 3.0);
    // The last line keeps its natural width.
    assert!(!p.lines[1].is_justified);
    assert_close(p.lines[1].visual_width(), 10.0 * CHAR);
}

#[test]
fn st3_2_word_spaces_share_the_stretch_in_proportion_to_their_capacity() {
    // "aa bb cc dd": the second space has 6 units of word spacing, so it is
    // 12 wide with a stretch of 6, against 3 for the first. In a 58.5-unit
    // column line 1 is "aa bb cc " (54) and the 4.5 units go 1.5 and 3.
    let text = "aa bb cc dd";
    let style = style(10.0);
    let spans = [StyleSpan {
        byte_range: TextRange::new(5, 6),
        style: TextStyle {
            word_spacing: CHAR,
            ..style.clone()
        },
    }];
    let paragraph = ParagraphInput {
        style_spans: &spans,
        ..input(text, &style)
    };
    let p = lay_out_mono(&paragraph, &column(58.5), &justified());
    assert_eq!(line_texts(&p, text), ["aa bb cc ", "dd"]);
    let line = &p.lines[0];
    assert!(line.is_justified);
    assert_close(line.visual_width(), 58.5);
    assert_close(x_of(line, 3), 2.0 * CHAR + CHAR + 1.5);
    assert_close(x_of(line, 6), 4.0 * CHAR + 3.0 * CHAR + 4.5);
    // The glyphs themselves keep their advances (Level 3 is not reached).
    assert!(glyphs(line).all(|g| (g.advance - CHAR).abs() < 1e-3));
}

#[test]
fn st3_2_last_lines_and_lines_ending_at_a_hard_break_are_never_justified() {
    let text = "aa bb\u{2028}cc dd";
    let p = lay_out(text, &column(40.0), &justified());
    assert_eq!(line_texts(&p, text), ["aa bb\u{2028}", "cc dd"]);
    for line in &p.lines {
        assert!(!line.is_justified);
        assert_close(line.visual_width(), 5.0 * CHAR);
    }
    assert_close(x_of(&p.lines[0], 3), 3.0 * CHAR);
    assert_close(x_of(&p.lines[1], 11), 3.0 * CHAR);
}

#[test]
fn st3_1_glue_before_a_tab_takes_no_part_in_justification() {
    // Tab stops every 40 units: "aa bb" (30) tabs to 40, then "cc dd" ends
    // at 70 in a 72-unit column. Only the space after the tab stretches.
    let text = "aa bb\tcc dd eeeeeeee";
    let constraints = LayoutConstraints {
        tab_stops: TabStops::new(40.0).unwrap(),
        ..column(72.0)
    };
    let ragged = lay_out(text, &constraints, &LayoutOptions::default());
    let p = lay_out(text, &constraints, &justified());
    assert_eq!(line_texts(&p, text), ["aa bb\tcc dd ", "eeeeeeee"]);
    assert_eq!(line_texts(&ragged, text), line_texts(&p, text));
    let line = &p.lines[0];
    assert!(line.is_justified);
    assert_close(line.visual_width(), 72.0);
    // Before the tab: unchanged.
    assert_close(x_of(line, 3), 3.0 * CHAR);
    let [tab] = line.tabs.as_slice() else {
        panic!("{:?}", line.tabs)
    };
    assert_close(tab.x, 5.0 * CHAR);
    assert_close(tab.x + tab.width, 40.0);
    assert_close(x_of(line, 6), 40.0);
    // After it: the space takes the 2 units.
    assert_close(x_of(&ragged.lines[0], 9), 40.0 + 3.0 * CHAR);
    assert_close(x_of(line, 9), 40.0 + 3.0 * CHAR + 2.0);
}

/// "ab 漢字漢字ab abcdefghijkl": line 1 holds a word space (Level 0), three
/// `ALLOW_CJK_JUSTIFY_AFTER` gaps after 漢, 字 and 漢 (Level 2) and an
/// autospace before the second "ab" (Level 1); the last word cannot join
/// it.
pub const LADDER: &str = "ab 漢字漢字ab abcdefghijkl";

/// The ladder's text under `profile`, its ragged layout in a column `slack`
/// wider than its first line, and its justified layout.
pub struct Ladder {
    pub ast: ParagraphAst,
    /// The natural width of line 1.
    pub natural: f32,
    /// The stretch of the word space and of the autospace.
    pub word_stretch: f32,
    pub autospace_stretch: f32,
    pub ragged: PositionedParagraph,
    pub justified: PositionedParagraph,
}

pub fn ladder(profile: AutospaceProfile, slack: f32) -> Ladder {
    let style = script_style(&[SERIF, JAPANESE], 10.0);
    let options = FrontendOptions {
        autospace_profile: profile,
        ..Default::default()
    };
    let ast = compile_bidi_input(&input(LADDER, &style), &options);
    let [(2, space)] = glues_of(&ast, GlueKind::WordSpace)[..1] else {
        panic!("{:?}", ast.items)
    };
    let [(15, autospace)] = glues_of(&ast, GlueKind::CjkAutospace)[..] else {
        panic!("{:?}", ast.items)
    };
    let (word_stretch, autospace_stretch) = (space.max_stretch, autospace.max_stretch);
    let probe = lay_out_bidi_ast(&ast, LADDER, &column(1000.0), &LayoutOptions::default());
    // Line 1 ends before the space at 17.
    let natural = x_of(&probe.lines[0], 16) + glyph_at(&probe.lines[0], 16).advance;
    let constraints = column(natural + slack);
    let ragged = lay_out_bidi_ast(&ast, LADDER, &constraints, &LayoutOptions::default());
    let justified = lay_out_bidi_ast(&ast, LADDER, &constraints, &justified());
    for p in [&ragged, &justified] {
        assert_eq!(
            line_texts(p, LADDER),
            ["ab 漢字漢字ab ", "abcdefghijkl"],
            "{slack}"
        );
    }
    Ladder {
        ast,
        natural,
        word_stretch,
        autospace_stretch,
        ragged,
        justified,
    }
}

/// The 0.25 em CJK gap at 10 units per em (ST1-7).
pub const CJK_GAP: f32 = 2.5;

impl Ladder {
    /// How far the glyph at `offset` moved under justification.
    pub fn shift(&self, offset: usize) -> f32 {
        x_of(&self.justified.lines[0], offset) - x_of(&self.ragged.lines[0], offset)
    }

    /// How much the advance of the glyph at `offset` grew.
    pub fn growth(&self, offset: usize) -> f32 {
        glyph_at(&self.justified.lines[0], offset).advance
            - glyph_at(&self.ragged.lines[0], offset).advance
    }

    /// Asserts the expansion of each level for a slack the first three
    /// levels cover: (Level 0, Level 1, each Level 2 gap).
    #[track_caller]
    pub fn assert_levels(&self, level0: f32, level1: f32, gap: f32) {
        let line = &self.justified.lines[0];
        assert!(line.is_justified);
        assert_close(
            line.visual_width(),
            self.natural + level0 + level1 + 3.0 * gap,
        );
        assert_close(self.shift(0), 0.0);
        assert_close(self.shift(1), 0.0);
        assert_close(self.shift(3), level0);
        assert_close(self.shift(6), level0 + gap);
        assert_close(self.shift(9), level0 + 2.0 * gap);
        assert_close(self.shift(12), level0 + 3.0 * gap);
        assert_close(self.shift(15), level0 + 3.0 * gap + level1);
        assert_close(self.shift(16), level0 + 3.0 * gap + level1);
        // A Level 2 gap lies on its cluster's logical end: the advance of
        // its last glyph (LTR).
        for offset in [3, 6, 9] {
            assert_close(self.growth(offset), gap);
        }
        for offset in [0, 1, 12, 15, 16] {
            assert_close(self.growth(offset), 0.0);
        }
        // The ragged line is untouched.
        assert!(!self.ragged.lines[0].is_justified);
        assert_close(self.ragged.lines[0].visual_width(), self.natural);
    }
}

#[test]
fn st3_2_the_ladder_fills_word_spaces_then_autospace_then_cjk_gaps() {
    let probe = ladder(AutospaceProfile::Jlreq, 0.0);
    let (s0, s1) = (probe.word_stretch, probe.autospace_stretch);
    assert_close(s1, 2.5);
    // Level 0 only.
    ladder(AutospaceProfile::Jlreq, s0 / 2.0).assert_levels(s0 / 2.0, 0.0, 0.0);
    // Level 0 full, Level 1 half.
    ladder(AutospaceProfile::Jlreq, s0 + s1 / 2.0).assert_levels(s0, s1 / 2.0, 0.0);
    // Levels 0 and 1 full, Level 2 shared equally by its three gaps.
    ladder(AutospaceProfile::Jlreq, s0 + s1 + 1.5).assert_levels(s0, s1, 0.5);
    // Exactly the tolerance: every gap at 0.25 em.
    ladder(AutospaceProfile::Jlreq, s0 + s1 + 3.0 * CJK_GAP).assert_levels(s0, s1, CJK_GAP);
}

#[test]
fn st3_2_a_rigid_autospace_is_skipped_by_the_ladder() {
    // CssCompat autospace has no stretch: Level 1 is empty, so the slack
    // after the word space goes to the CJK gaps.
    let probe = ladder(AutospaceProfile::CssCompat, 0.0);
    assert_eq!(probe.autospace_stretch, 0.0);
    let s0 = probe.word_stretch;
    ladder(AutospaceProfile::CssCompat, s0 + 3.0).assert_levels(s0, 0.0, 1.0);
}

#[test]
fn st3_4_an_over_loose_line_spends_every_level_and_stays_ragged() {
    let l = ladder(AutospaceProfile::Jlreq, 40.0);
    let line = &l.justified.lines[0];
    assert!(!line.is_justified);
    let column = l.natural + 40.0;
    // The line starts at the slot's start and ends short of its end.
    assert_close(line.slots[0].content_start, 0.0);
    assert!(content_end(line) < column - 1.0, "{}", content_end(line));
    // Level 0 and Level 1 at their caps: the gaps after "b" and before the
    // second "a" are the natural glue plus its full stretch.
    let ragged = &l.ragged.lines[0];
    let gap = |p: &PositionedLine, from: usize, to: usize| {
        x_of(p, to) - (x_of(p, from) + glyph_at(p, from).advance)
    };
    assert_close(gap(line, 1, 3) - gap(ragged, 1, 3), l.word_stretch);
    assert_close(gap(line, 12, 15) - gap(ragged, 12, 15), l.autospace_stretch);
    // Level 2 at its cap: 0.25 em after 漢, 字 and 漢, and no Level 3 there.
    for offset in [3, 6, 9] {
        assert_close(l.growth(offset), CJK_GAP);
    }
    // Level 3 at its cap: 0.125 em after every other visible cluster but
    // the last (after "a", "b", the second 字 and the second "a").
    for offset in [0, 1, 12, 15] {
        assert_close(l.growth(offset), 1.25);
    }
    assert_close(l.growth(16), 0.0);
    assert_close(
        line.visual_width(),
        l.natural + l.word_stretch + l.autospace_stretch + 3.0 * CJK_GAP + 4.0 * 1.25,
    );
    assert!(!l.ast.items.is_empty());
}

/// Lays out `words` (the last one too long to join the first line) in
/// `families` in a column `slack` wider than the first line's natural
/// width, ragged and justified. Returns the AST, the natural width and the
/// two layouts.
fn two_word_line(
    text: &str,
    families: &[&str],
    first_line_end: usize,
    slack: f32,
) -> (ParagraphAst, f32, PositionedParagraph, PositionedParagraph) {
    let ast = compile_bidi(text, families);
    let natural: f32 = ast
        .items
        .iter()
        // Everything before the trailing space.
        .filter(|item| item.byte_span().end() < first_line_end)
        .map(|item| match item {
            WordNode::Box(BoxNode::Text { natural_width, .. }) => *natural_width,
            WordNode::Glue(g) => g.natural_width,
            _ => 0.0,
        })
        .sum();
    let constraints = column(natural + slack);
    let ragged = lay_out_bidi_ast(&ast, text, &constraints, &LayoutOptions::default());
    let justified = lay_out_bidi_ast(&ast, text, &constraints, &justified());
    (ast, natural, ragged, justified)
}

/// The sum of the advances of `line`'s glyphs.
fn ink(line: &PositionedLine) -> f32 {
    glyphs(line).map(|g| g.advance).sum()
}

#[test]
fn st3_5_arabic_lines_justify_at_word_spaces_only() {
    // "سلام عليكم" then a long word: the first line has one word space.
    let text = "سلام عليكم مرحبامرحبامرحبامرحبا";
    let first_line_end = text.find("مرحبا").unwrap();
    let ast = compile_bidi(text, &[ARABIC]);
    let [(_, space), ..] = glues_of(&ast, GlueKind::WordSpace)[..] else {
        panic!()
    };
    let s0 = space.max_stretch;
    // Within the word space's stretch: flush, and no glyph grows.
    let (_, natural, ragged, p) = two_word_line(text, &[ARABIC], first_line_end, s0 / 2.0);
    assert_eq!(line_texts(&p, text)[0], &text[..first_line_end]);
    let line = &p.lines[0];
    assert!(line.is_justified);
    assert_close(line.visual_width(), natural + s0 / 2.0);
    assert_close(ink(line), ink(&ragged.lines[0]));
    // Beyond it: no other level applies to Arabic, so the line is over-loose
    // with the space at its cap, ragged at the line end, the left side of
    // an RTL line.
    let (_, natural, ragged, p) = two_word_line(text, &[ARABIC], first_line_end, s0 + 5.0);
    let line = &p.lines[0];
    assert!(!line.is_justified);
    assert_close(line.visual_width(), natural + s0);
    assert_close(ink(line), ink(&ragged.lines[0]));
    assert_close(content_end(line), natural + s0 + 5.0);
}

#[test]
fn st3_5_thai_expands_at_word_separators_only() {
    let text = "สวัสดี ครับ ขอบคุณมาก";
    let first_line_end = text.find("ขอบ").unwrap();
    let ast = compile_bidi(text, &[THAI]);
    let [(_, space), ..] = glues_of(&ast, GlueKind::WordSpace)[..] else {
        panic!()
    };
    let s0 = space.max_stretch;
    let (_, natural, ragged, p) = two_word_line(text, &[THAI], first_line_end, s0 / 2.0);
    assert_eq!(line_texts(&p, text)[0], &text[..first_line_end]);
    let line = &p.lines[0];
    assert!(line.is_justified);
    assert_close(line.visual_width(), natural + s0 / 2.0);
    assert_close(ink(line), ink(&ragged.lines[0]));
    // Over-loose: the clusters never spread (no Level 3 after Thai).
    let (_, natural, ragged, p) = two_word_line(text, &[THAI], first_line_end, 10.0);
    let line = &p.lines[0];
    assert!(!line.is_justified);
    assert_close(line.visual_width(), natural + s0);
    assert_close(ink(line), ink(&ragged.lines[0]));
}

#[test]
fn st1_12_synthetic_glue_collapses_at_line_edges_under_both_modes() {
    // "漢字漢字abcdef" breaks at the autospace before "a": the glue stays on
    // line 1 (it precedes the break) and has no width there, under either
    // whitespace mode; line 2 starts flush with "a".
    for (text, first) in [("漢字漢字abcdef", "漢字漢字"), ("abcdef漢字漢字", "abcdef")]
    {
        let ast = compile_bidi(text, &[SERIF, JAPANESE]);
        let probe = lay_out_bidi_ast(&ast, text, &column(1000.0), &LayoutOptions::default());
        let line = &probe.lines[0];
        let first_width = glyphs(line)
            .filter(|g| g.byte_range.is_some_and(|r| r.end() <= first.len()))
            .map(|g| g.advance)
            .sum::<f32>();
        for whitespace in [
            HangingWhitespaceMode::Collapsible,
            HangingWhitespaceMode::Preserved,
        ] {
            let options = LayoutOptions {
                whitespace,
                ..Default::default()
            };
            let p = lay_out_bidi_ast(&ast, text, &column(first_width + 4.0), &options);
            assert_eq!(line_texts(&p, text)[0], first, "{whitespace:?}");
            assert_close(p.lines[0].visual_width(), first_width);
            assert_eq!(p.lines[0].visible_byte_end, first.len());
            let second = &p.lines[1];
            assert_close(x_of(second, first.len()), 0.0);
            assert_close(second.slots[0].content_start, 0.0);
            // The collapsed glue leaves no gap before the line end either:
            // under `Preserved` it is no trailing whitespace.
            let last = glyphs(&p.lines[0])
                .map(|g| g.x + g.advance)
                .fold(f32::NEG_INFINITY, f32::max);
            assert_close(last, first_width);
        }
    }
}

#[test]
fn st3_3_compression_spends_tier_0_before_tier_1() {
    // "漢、「漢漢b漢漢abcdefghijkl": line 1 (bytes 0..22) holds a
    // compression glue after 、 (Tier 0, shrink 5) and autospaces before and
    // after "b" (Tier 1, shrink 1.25 each); the Latin word cannot join it.
    let text = "漢、「漢漢b漢漢abcdefghijkl";
    let style = localized_style(&[SERIF, JAPANESE], 10.0, "ja");
    let ast = compile_bidi_input(&input(text, &style), &FrontendOptions::default());
    let compression: Vec<usize> = glues_of(&ast, GlueKind::CjkPunctCompression)
        .iter()
        .map(|(offset, _)| *offset)
        .collect();
    assert_eq!(compression, [6]);
    let probe = lay_out_bidi_ast(&ast, text, &column(1000.0), &LayoutOptions::default());
    let wide = &probe.lines[0];
    let natural = x_of(wide, 19) + glyph_at(wide, 19).advance;
    // At natural width the last ideograph does not fit; under `justify` the
    // line takes it and compresses.
    let ragged = lay_out_bidi_ast(
        &ast,
        text,
        &column(natural - 4.0),
        &LayoutOptions::default(),
    );
    assert_eq!(line_texts(&ragged, text)[0], "漢、「漢漢b漢");
    for (shortfall, tier0, tier1) in [(4.0, 4.0, 0.0), (6.0, 5.0, 0.5)] {
        let p = lay_out_bidi_ast(&ast, text, &column(natural - shortfall), &justified());
        assert_eq!(line_texts(&p, text)[0], "漢、「漢漢b漢漢", "{shortfall}");
        let line = &p.lines[0];
        assert!(line.is_justified, "{shortfall}");
        assert_close(line.visual_width(), natural - shortfall);
        assert_close(x_of(line, 6), x_of(wide, 6) - tier0);
        assert_close(x_of(line, 15), x_of(wide, 15) - tier0 - tier1);
        assert_close(x_of(line, 16), x_of(wide, 16) - tier0 - 2.0 * tier1);
        // The glyphs keep their advances.
        for g in glyphs(line) {
            let offset = g.byte_range.unwrap().start();
            assert_close(g.advance, glyph_at(wide, offset).advance);
        }
    }
}
