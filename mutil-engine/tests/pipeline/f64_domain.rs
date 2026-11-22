//! Pipeline tests for the `f64` internal layout domain (spec AX-3, ST3-2,
//! ST3-4, §2.6):
//! - Zero-drift cumulative justification across prime-count gaps (LTR word
//!   spaces and CJK ideograph clusters).
//! - Exact single-rounding alignment for RTL and justified lines against
//!   `slot.content_start` and `slot.content_end()` within
//!   `LINE_GEOMETRY_ABS_EPSILON`.
//! - Boundary enforcement at `MAX_MEASUREMENT = 1_048_576.0` (`2^20`).

use crate::acceptance_phase2::glyphs;
use crate::common::*;
use mutil_engine::*;

fn justified() -> LayoutOptions {
    LayoutOptions {
        justify: true,
        ..Default::default()
    }
}

fn body_glyphs(line: &PositionedLine) -> Vec<&PositionedGlyph> {
    glyphs(line).collect()
}

#[test]
fn zero_drift_justified_ltr_prime_word_gaps() {
    // 8 two-letter words separated by 7 spaces (prime gap count = 7) on line 1,
    // followed by a long word that wraps to line 2. At 10 units/em in the
    // monospace face, each glyph and space is 6.0 wide, so line 1's natural
    // width is 23 * 6.0 = 138.0. With wrap_width = 149.0, 11.0 units of
    // stretch are distributed across 7 gaps (11/7 is non-terminating in base 2).
    let text = "aa bb cc dd ee ff gg hh zzzzzzzzzz";
    let wrap_width = 149.0;
    let p = lay_out(text, &column(wrap_width), &justified());

    assert!(p.validate(text).is_ok());
    assert!(ValidatedLayout::try_from_paragraph(&p, text).is_ok());
    assert_eq!(
        line_texts(&p, text),
        ["aa bb cc dd ee ff gg hh ", "zzzzzzzzzz"]
    );

    let line = &p.lines[0];
    assert!(line.is_justified);
    assert!(
        (line.visual_width() - wrap_width).abs() <= 1e-3,
        "justified visual_width {} drifted from wrap_width {}",
        line.visual_width(),
        wrap_width
    );
}

#[test]
fn zero_drift_justified_cjk_prime_cluster_gaps() {
    // 14 Han ideographs on line 1 (13 inter-ideograph gaps, prime = 13) plus
    // 10 more on line 2. In Noto Sans JP at 10 units/em each ideograph is 10.0
    // wide (natural width 140.0). With wrap_width = 147.0, 7.0 units of
    // stretch are distributed across 13 CJK gaps (7/13 is non-terminating).
    let text = "日本語中文漢字排版測試東京北京人大小上下山川水火";
    let wrap_width = 147.0;
    let p = lay_out_in(text, &[JAPANESE], &column(wrap_width), &justified());

    assert!(p.validate(text).is_ok());
    assert!(ValidatedLayout::try_from_paragraph(&p, text).is_ok());
    assert_eq!(p.lines.len(), 2);

    let line = &p.lines[0];
    assert!(line.is_justified);
    assert!(
        (line.visual_width() - wrap_width).abs() <= 1e-3,
        "justified CJK visual_width {} drifted from wrap_width {}",
        line.visual_width(),
        wrap_width
    );
}

#[test]
fn exact_single_rounding_rtl_and_justified_alignment() {
    // 1. RTL paragraph aligned to the slot's right edge (slot.end() - width).
    let rtl_text = "שלום עולם";
    let rtl_wrap = 200.0;
    let rtl = lay_out_bidi(
        rtl_text,
        &[HEBREW],
        &column(rtl_wrap),
        &LayoutOptions::default(),
    );

    assert!(rtl.validate(rtl_text).is_ok());
    assert!(ValidatedLayout::try_from_paragraph(&rtl, rtl_text).is_ok());
    assert_eq!(rtl.lines.len(), 1);

    let rtl_line = &rtl.lines[0];
    let rtl_slot = &rtl_line.slots[0];
    let rtl_glyphs = body_glyphs(rtl_line);
    assert!(!rtl_glyphs.is_empty());
    let rtl_first = rtl_glyphs.first().unwrap();
    let rtl_last = rtl_glyphs.last().unwrap();
    assert!(
        (rtl_first.x - rtl_slot.content_start).abs() <= LINE_GEOMETRY_ABS_EPSILON,
        "RTL first glyph x {} != content_start {}",
        rtl_first.x,
        rtl_slot.content_start
    );
    assert!(
        ((rtl_last.x + rtl_last.advance) - rtl_slot.content_end()).abs()
            <= LINE_GEOMETRY_ABS_EPSILON,
        "RTL last glyph end {} != content_end {}",
        rtl_last.x + rtl_last.advance,
        rtl_slot.content_end()
    );

    // 2. Justified LTR paragraph spanning the full slot.
    let ltr_text = "aa bb cc dd ee ff gg hh zzzzzzzzzz";
    let ltr_wrap = 149.0;
    let ltr = lay_out(ltr_text, &column(ltr_wrap), &justified());

    assert!(ltr.validate(ltr_text).is_ok());
    assert!(ValidatedLayout::try_from_paragraph(&ltr, ltr_text).is_ok());

    let ltr_line = &ltr.lines[0];
    let ltr_slot = &ltr_line.slots[0];
    let ltr_glyphs = body_glyphs(ltr_line);
    assert!(!ltr_glyphs.is_empty());
    let ltr_first = ltr_glyphs.first().unwrap();
    let ltr_last = ltr_glyphs.last().unwrap();
    assert!(
        (ltr_first.x - ltr_slot.content_start).abs() <= LINE_GEOMETRY_ABS_EPSILON,
        "justified first glyph x {} != content_start {}",
        ltr_first.x,
        ltr_slot.content_start
    );
    assert!(
        ((ltr_last.x + ltr_last.advance) - ltr_slot.content_end()).abs()
            <= LINE_GEOMETRY_ABS_EPSILON,
        "justified last glyph end {} != content_end {}",
        ltr_last.x + ltr_last.advance,
        ltr_slot.content_end()
    );
}

#[test]
fn max_measurement_constants_and_predicates() {
    assert_eq!(MAX_MEASUREMENT, 1_048_576.0);
    assert!(is_measurement(MAX_MEASUREMENT));
    assert!(is_extent(MAX_MEASUREMENT));
    assert!(!is_measurement(MAX_MEASUREMENT + 1.0));
    assert!(!is_measurement(-MAX_MEASUREMENT - 1.0));
    assert!(!is_extent(MAX_MEASUREMENT + 1.0));
}

#[test]
fn check_layout_inputs_enforces_max_measurement_boundary() {
    let text = "a b";
    let base = compile(text);
    let constraints = column(100.0);
    let options = LayoutOptions::default();

    // Baseline AST is accepted.
    assert!(check_layout_inputs(&base, text, &constraints, &options).is_ok());

    // font_size at MAX_MEASUREMENT is accepted; above MAX_MEASUREMENT is rejected.
    let mut at_max_font = base.clone();
    at_max_font.runs[0].font_size = MAX_MEASUREMENT;
    assert!(check_layout_inputs(&at_max_font, text, &constraints, &options).is_ok());

    let mut over_max_font = base.clone();
    over_max_font.runs[0].font_size = MAX_MEASUREMENT + 1.0;
    assert!(check_layout_inputs(&over_max_font, text, &constraints, &options).is_err());

    // letter_spacing at MAX_MEASUREMENT is accepted; above MAX_MEASUREMENT is rejected.
    let mut at_max_ls = base.clone();
    at_max_ls.runs[0].letter_spacing = MAX_MEASUREMENT;
    assert!(check_layout_inputs(&at_max_ls, text, &constraints, &options).is_ok());

    let mut over_max_ls = base.clone();
    over_max_ls.runs[0].letter_spacing = MAX_MEASUREMENT + 1.0;
    assert!(check_layout_inputs(&over_max_ls, text, &constraints, &options).is_err());

    // glue.natural_width at MAX_MEASUREMENT is accepted; above MAX_MEASUREMENT is rejected.
    let mut at_max_glue = base.clone();
    let glue = at_max_glue
        .items
        .iter_mut()
        .find_map(|item| match item {
            WordNode::Glue(g) => Some(g),
            _ => None,
        })
        .expect("a space produces a glue node");
    glue.natural_width = MAX_MEASUREMENT;
    assert!(check_layout_inputs(&at_max_glue, text, &constraints, &options).is_ok());

    let mut over_max_glue = base.clone();
    let glue = over_max_glue
        .items
        .iter_mut()
        .find_map(|item| match item {
            WordNode::Glue(g) => Some(g),
            _ => None,
        })
        .expect("a space produces a glue node");
    glue.natural_width = MAX_MEASUREMENT + 1.0;
    assert!(check_layout_inputs(&over_max_glue, text, &constraints, &options).is_err());
}
