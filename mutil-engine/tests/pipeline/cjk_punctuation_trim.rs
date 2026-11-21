//! CJK punctuation side-slot trimming in layout (FE-31, ST1-2, ST1-3, OPT-4):
//! default ragged-right compression under `CjkPunctuationTrim::Auto` for
//! `zh-Hans` and `ja`, no compression for `zh-Hant` / `zh-TW` or `SpaceAll`,
//! zero double-trimming on 3+ mark chains, line-break split restoration,
//! `SpaceFirst` vs `TrimStart` at paragraph start / hard break / soft wrap,
//! survival across `LineOnlyShaper` and emergency/seam re-shaping, and
//! `TrimStart` + `HangMode` interaction.

use crate::acceptance_phase2::glyph_at;
use crate::common::*;
use mutil_engine::*;
use std::sync::LazyLock;

/// Font size used across all tests: 1em = 10.0, 0.5em = 5.0.
const EM: f32 = 10.0;
const HALF_EM: f32 = 5.0;

/// Deterministic full-width CJK `LineShaper` with no `chws` support: every
/// character shapes to a 1.0em full-width glyph at `x_offset = 0`, `y_offset = 0`.
struct FullWidthCjkLines;

static CJK_LINES: LazyLock<LineOnlyShaper<FullWidthCjkLines>> =
    LazyLock::new(|| LineOnlyShaper::new(FullWidthCjkLines));

impl LineShaper for FullWidthCjkLines {
    fn shape_line(
        &self,
        text: &str,
        _families: &[FontFamily],
        _style: FontStyle,
        font_size: f32,
        _features: &FontFeatures,
    ) -> Vec<LineGlyph> {
        text.char_indices()
            .map(|(byte_index, ch)| LineGlyph {
                font: FontId(0),
                glyph_id: ch as u32,
                byte_index,
                x_advance: font_size,
                x_offset: 0.0,
                y_offset: 0.0,
            })
            .collect()
    }

    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics {
        SHAPER.font_metrics(instance)
    }

    fn resolve_primary_font_metrics(
        &self,
        families: &[FontFamily],
        style: FontStyle,
        script: ScriptTag,
        locale: &LocaleTag,
        font_size: f32,
        variations: &[FontVariation],
    ) -> (FontId, FontMetrics) {
        SHAPER.resolve_primary_font_metrics(families, style, script, locale, font_size, variations)
    }

    fn space_advance(&self, instance: FontInstanceRef<'_>, _orientation: GlyphOrientation) -> f32 {
        instance.font_size * 0.5
    }
}

fn cjk_style(locale: &str) -> TextStyle {
    TextStyle {
        locale: Some(LocaleTag::try_from_str_lenient(locale).expect("valid locale")),
        ..style(EM)
    }
}

#[track_caller]
fn lay_out_cjk(
    text: &str,
    locale: &str,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> PositionedParagraph {
    let st = cjk_style(locale);
    let ast = compile_paragraph(&input(text, &st), &FrontendOptions::default(), &*CJK_LINES)
        .unwrap_or_else(|e| panic!("compile {text:?} ({locale}): {e}"));
    let paragraph = layout_paragraph(&ast, text, &*CJK_LINES, constraints, options)
        .unwrap_or_else(|e| panic!("layout {text:?} ({locale}): {e}"));
    if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&paragraph, text) {
        panic!("validate {text:?} ({locale}): line {line}: {error}\n{paragraph:#?}");
    }
    paragraph
}

fn trim_opts(trim: CjkPunctuationTrim) -> LayoutOptions {
    LayoutOptions {
        cjk_punctuation_trim: trim,
        ..Default::default()
    }
}

/// Byte offset of the `n`-th char (0-indexed) in `s`.
fn byte_of(s: &str, char_index: usize) -> usize {
    s.char_indices().nth(char_index).expect("char index").0
}

#[test]
fn auto_trims_adjacent_pairs_ragged_right_for_zh_hans_and_ja() {
    let opts = LayoutOptions::default();
    assert_eq!(opts.cjk_punctuation_trim, CjkPunctuationTrim::Auto);
    assert!(!opts.justify);

    for locale in ["zh-Hans", "ja"] {
        // Adjacent close+close: "中”。文" -> 4 chars, `”` trimmed by 0.5em on trail.
        let text = "中”。文";
        let p = lay_out_cjk(text, locale, &column(200.0), &opts);
        assert_eq!(p.policy.cjk_punctuation_trim, CjkPunctuationTrim::Auto);
        assert_eq!(p.lines.len(), 1);
        assert!(!p.lines[0].is_justified);
        assert_close(p.lines[0].visual_width(), 4.0 * EM - HALF_EM);
        assert_close(glyph_at(&p.lines[0], byte_of(text, 1)).advance, HALF_EM);
        assert_close(glyph_at(&p.lines[0], byte_of(text, 2)).advance, EM);

        // Adjacent close+open: "中。“文" -> 4 chars, `。` trimmed by 0.5em on trail.
        let text = "中。“文";
        let p = lay_out_cjk(text, locale, &column(200.0), &opts);
        assert_eq!(p.lines.len(), 1);
        assert_close(p.lines[0].visual_width(), 4.0 * EM - HALF_EM);
        assert_close(glyph_at(&p.lines[0], byte_of(text, 1)).advance, HALF_EM);
        assert_close(glyph_at(&p.lines[0], byte_of(text, 2)).advance, EM);

        // Adjacent open+open: "中《（文" -> 4 chars, `（` trimmed by 0.5em on lead.
        let text = "中《（文";
        let p = lay_out_cjk(text, locale, &column(200.0), &opts);
        assert_eq!(p.lines.len(), 1);
        assert_close(p.lines[0].visual_width(), 4.0 * EM - HALF_EM);
        assert_close(glyph_at(&p.lines[0], byte_of(text, 1)).advance, EM);
        assert_close(glyph_at(&p.lines[0], byte_of(text, 2)).advance, HALF_EM);
    }
}

#[test]
fn auto_does_not_compress_traditional_chinese() {
    let opts = LayoutOptions::default();

    for locale in ["zh-Hant", "zh-TW"] {
        for text in ["中”。文", "中。“文", "中《（文"] {
            let p = lay_out_cjk(text, locale, &column(200.0), &opts);
            assert_eq!(p.lines.len(), 1, "{locale} {text}");
            assert_close(p.lines[0].visual_width(), 4.0 * EM);
            assert_close(glyph_at(&p.lines[0], byte_of(text, 1)).advance, EM);
            assert_close(glyph_at(&p.lines[0], byte_of(text, 2)).advance, EM);
        }
    }
}

#[test]
fn explicit_space_all_disables_trimming_for_zh_hans_and_ja() {
    let opts = trim_opts(CjkPunctuationTrim::SpaceAll);

    for locale in ["zh-Hans", "ja"] {
        for text in ["中”。文", "中。“文", "中《（文"] {
            let p = lay_out_cjk(text, locale, &column(200.0), &opts);
            assert_eq!(p.policy.cjk_punctuation_trim, CjkPunctuationTrim::SpaceAll);
            assert_eq!(p.lines.len(), 1, "{locale} {text}");
            assert_close(p.lines[0].visual_width(), 4.0 * EM);
            assert_close(glyph_at(&p.lines[0], byte_of(text, 1)).advance, EM);
            assert_close(glyph_at(&p.lines[0], byte_of(text, 2)).advance, EM);
        }
    }
}

#[test]
fn three_mark_chains_have_zero_double_trimming() {
    let opts = LayoutOptions::default();

    // Closing chain: "中。）”文" (5 chars)
    // `。` trimmed 0.5em on trail, `）` trimmed 0.5em on trail, `”` untrimmed.
    let close_chain = "中。）”文";
    let p = lay_out_cjk(close_chain, "zh-Hans", &column(200.0), &opts);
    assert_eq!(p.lines.len(), 1);
    assert_close(p.lines[0].visual_width(), 5.0 * EM - 2.0 * HALF_EM);
    assert_close(
        glyph_at(&p.lines[0], byte_of(close_chain, 1)).advance,
        HALF_EM,
    );
    assert_close(
        glyph_at(&p.lines[0], byte_of(close_chain, 2)).advance,
        HALF_EM,
    );
    assert_close(glyph_at(&p.lines[0], byte_of(close_chain, 3)).advance, EM);

    // Opening chain: "中《（「文" (5 chars)
    // `《` untrimmed, `（` trimmed 0.5em on lead, `「` trimmed 0.5em on lead.
    let open_chain = "中《（「文";
    let p = lay_out_cjk(open_chain, "zh-Hans", &column(200.0), &opts);
    assert_eq!(p.lines.len(), 1);
    assert_close(p.lines[0].visual_width(), 5.0 * EM - 2.0 * HALF_EM);
    assert_close(glyph_at(&p.lines[0], byte_of(open_chain, 1)).advance, EM);
    assert_close(
        glyph_at(&p.lines[0], byte_of(open_chain, 2)).advance,
        HALF_EM,
    );
    assert_close(
        glyph_at(&p.lines[0], byte_of(open_chain, 3)).advance,
        HALF_EM,
    );
}

#[test]
fn line_break_split_restores_trailing_stop_and_trims_soft_wrapped_opener() {
    // "中文。《文学"
    // Line 1: "中文。" (3 chars = 30.0)
    // Line 2: "《文学" (soft wrap: `《` trimmed by 0.5em -> 25.0)
    let text = "中文。《文学";
    for mode in [
        CjkPunctuationTrim::SpaceFirst,
        CjkPunctuationTrim::TrimStart,
    ] {
        let p = lay_out_cjk(text, "zh-Hans", &column(30.0), &trim_opts(mode));
        assert_eq!(line_texts(&p, text), ["中文。", "《文学"], "{mode:?}");

        // `。` at the end of line 1 is restored to full width (1.0em).
        let stop = glyph_at(&p.lines[0], byte_of(text, 2));
        assert_close(stop.advance, EM);
        assert_close(p.lines[0].visual_width(), 3.0 * EM);

        // `《` at the start of line 2 (soft wrap) is trimmed by 0.5em.
        let open = glyph_at(&p.lines[1], byte_of(text, 3));
        assert_close(open.advance, HALF_EM);
        assert_close(p.lines[1].visual_width(), 3.0 * EM - HALF_EM);
    }
}

#[test]
fn space_first_vs_trim_start_at_paragraph_start_hard_break_and_soft_wrap() {
    // Line 0 (paragraph start): "《中文"
    // Line 1 (soft wrap): "《语文"
    // Hard break (\u{2028})
    // Line 2 (post-hard-break): "《诗"
    let text = "《中文《语文\u{2028}《诗";

    // SpaceFirst: paragraph-start (line 0) and post-hard-break (line 2) stay full-width;
    // soft-wrapped line 1 trims its leading `《` by 0.5em.
    let p_first = lay_out_cjk(
        text,
        "zh-Hans",
        &column(30.0),
        &trim_opts(CjkPunctuationTrim::SpaceFirst),
    );
    assert_eq!(
        line_texts(&p_first, text),
        ["《中文", "《语文\u{2028}", "《诗"]
    );
    assert_close(glyph_at(&p_first.lines[0], byte_of(text, 0)).advance, EM);
    assert_close(p_first.lines[0].visual_width(), 3.0 * EM);

    assert_close(
        glyph_at(&p_first.lines[1], byte_of(text, 3)).advance,
        HALF_EM,
    );
    assert_close(p_first.lines[1].visual_width(), 3.0 * EM - HALF_EM);

    assert_close(glyph_at(&p_first.lines[2], byte_of(text, 7)).advance, EM);
    assert_close(p_first.lines[2].visual_width(), 2.0 * EM);

    // TrimStart: trims opening marks at every line start (line 0, line 1, line 2).
    let p_trim = lay_out_cjk(
        text,
        "zh-Hans",
        &column(30.0),
        &trim_opts(CjkPunctuationTrim::TrimStart),
    );
    assert_eq!(
        line_texts(&p_trim, text),
        ["《中文", "《语文\u{2028}", "《诗"]
    );
    assert_close(
        glyph_at(&p_trim.lines[0], byte_of(text, 0)).advance,
        HALF_EM,
    );
    assert_close(p_trim.lines[0].visual_width(), 3.0 * EM - HALF_EM);

    assert_close(
        glyph_at(&p_trim.lines[1], byte_of(text, 3)).advance,
        HALF_EM,
    );
    assert_close(p_trim.lines[1].visual_width(), 3.0 * EM - HALF_EM);

    assert_close(
        glyph_at(&p_trim.lines[2], byte_of(text, 7)).advance,
        HALF_EM,
    );
    assert_close(p_trim.lines[2].visual_width(), 2.0 * EM - HALF_EM);
}

#[test]
fn survives_line_only_shaper_and_emergency_break_reshaping() {
    // Column narrower than the fused unit "中。）" (10 + 5 + 10 = 25.0) forces
    // an emergency break and seam re-shaping under `LineOnlyShaper`.
    let text = "中。）文";
    let p = lay_out_cjk(text, "zh-Hans", &column(16.0), &LayoutOptions::default());
    assert!(p.lines.len() >= 2, "narrow column must force a break");
    // Every line passes ValidatedLayout (checked inside lay_out_cjk).
    // Check that trimming survives on an adjacent pair that stays on one line
    // after an emergency break earlier in the paragraph.
    let text2 = "中。）《（文";
    let p2 = lay_out_cjk(text2, "zh-Hans", &column(25.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p2, text2), ["中。）", "《（文"]);
    // Line 0: "中。）" -> `。` trimmed to 0.5em, `）` full-width.
    assert_close(glyph_at(&p2.lines[0], byte_of(text2, 1)).advance, HALF_EM);
    assert_close(glyph_at(&p2.lines[0], byte_of(text2, 2)).advance, EM);
    // Line 1 (soft wrap): "《（文" -> both `《` (line-start) and `（` (after open) trimmed to 0.5em.
    assert_close(glyph_at(&p2.lines[1], byte_of(text2, 3)).advance, HALF_EM);
    assert_close(glyph_at(&p2.lines[1], byte_of(text2, 4)).advance, HALF_EM);
}

#[test]
fn trim_start_with_hang_start_mode_does_not_double_shift() {
    let text = "《中文《语文";
    let expected_protrusion = 0.85 * EM - HALF_EM; // 0.35 * EM = 3.5

    // HangStartMode::AllLines + TrimStart: both line 0 and line 1 hang by 0.35em.
    let opts_all = LayoutOptions {
        cjk_punctuation_trim: CjkPunctuationTrim::TrimStart,
        hang_mode: HangMode {
            start: HangStartMode::AllLines,
            end: HangEndMode::None,
        },
        ..Default::default()
    };
    let p_all = lay_out_cjk(text, "zh-Hans", &column(30.0), &opts_all);
    assert_eq!(line_texts(&p_all, text), ["《中文", "《语文"]);
    for line in &p_all.lines {
        assert_close(line.hanging_left(), expected_protrusion);
        assert_close(line.hanging_right(), 0.0);
        // Body width: 2 full chars (20.0) + inside portion of trimmed opener (0.15 * EM = 1.5).
        assert_close(line.visual_width(), 2.0 * EM + 0.15 * EM);
    }

    // HangStartMode::FirstLine + TrimStart: line 0 hangs by 0.35em; line 1 does not hang
    // but still trims its leading opener by 0.5em.
    let opts_first = LayoutOptions {
        cjk_punctuation_trim: CjkPunctuationTrim::TrimStart,
        hang_mode: HangMode {
            start: HangStartMode::FirstLine,
            end: HangEndMode::None,
        },
        ..Default::default()
    };
    let p_first = lay_out_cjk(text, "zh-Hans", &column(30.0), &opts_first);
    assert_eq!(line_texts(&p_first, text), ["《中文", "《语文"]);
    assert_close(p_first.lines[0].hanging_left(), expected_protrusion);
    assert_close(p_first.lines[0].visual_width(), 2.0 * EM + 0.15 * EM);

    assert_close(p_first.lines[1].hanging_left(), 0.0);
    assert_close(
        glyph_at(&p_first.lines[1], byte_of(text, 3)).advance,
        HALF_EM,
    );
    assert_close(p_first.lines[1].visual_width(), 3.0 * EM - HALF_EM);
}
