//! Regressions for hanging punctuation under glue and emergency breaks
//! (spec ST1-3, HNG-2, ST1-1, SLT-5) and for URL boundaries (FE-22).
//! Every layout passes the output gate (`lay_out`).

use crate::common::*;
use mutil_engine::*;

fn all_lines() -> LayoutOptions {
    LayoutOptions {
        hang_mode: HangMode {
            start: HangStartMode::AllLines,
            end: HangEndMode::None,
        },
        ..Default::default()
    }
}

/// The number of hanging glyphs on `line`.
fn hanging_glyphs(line: &PositionedLine) -> usize {
    crate::acceptance_phase2::glyphs(line)
        .filter(|g| g.is_hanging)
        .count()
}

#[test]
fn st1_3_a_mark_behind_glue_on_its_line_does_not_hang() {
    // Only zero-width span boundaries may sit between a hanging mark and the
    // edge, so the leading spaces keep "(" from hanging, whatever the width:
    // the layout succeeds and validates, and the lines are as follows.
    for (text, expected) in [
        ("  (aaaa bb", &["  ", "(aaaa ", "bb"][..]),
        (
            "x\u{2028}  (aaaa bb",
            &["x\u{2028}", "  ", "(aaaa ", "bb"][..],
        ),
    ] {
        for width in [38.0, 40.0] {
            let p = lay_out(text, &column(width), &all_lines());
            assert_eq!(line_texts(&p, text), expected, "{text:?} at {width}");
            // Only the opening mark that starts its own line hangs, never glue.
            assert!(p.lines.iter().map(hanging_glyphs).sum::<usize>() <= 1);
        }
        // Only laying out and validating is required at these widths.
        for width in [36.0, 42.0] {
            lay_out(text, &column(width), &all_lines());
        }
    }
}

#[test]
fn st1_5_an_emergency_break_counts_the_hang_of_an_opening_mark() {
    // "(" hangs 5.1 of its 6 units at the start of the unbreakable run, so
    // the first line holds `n` clusters when 6n - 5.1 fits: 6 clusters need
    // 30.9, 7 need 36.9.
    let text = "(aaaaaaaaaaaaaaaa";
    for (width, clusters) in [(30.0, 5), (31.0, 6), (36.0, 6), (37.0, 7)] {
        let p = lay_out(text, &column(width), &all_lines());
        let first = p.lines[0].byte_range.as_range().len();
        assert_eq!(first, clusters, "width {width}: {:?}", line_texts(&p, text));
        assert_eq!(hanging_glyphs(&p.lines[0]), 1, "width {width}");
    }
}

fn url_band_offsets(text: &str) -> Vec<usize> {
    let band = DiscretionaryNode::URL_PENALTY_MIN..=DiscretionaryNode::URL_PENALTY_MAX;
    compile(text)
        .items
        .iter()
        .filter_map(|item| match item {
            WordNode::Discretionary(d) if band.contains(&d.penalty) => Some(d.byte_offset()),
            _ => None,
        })
        .collect()
}

#[test]
fn fe22_a_bare_www_dot_is_not_a_url_but_www_dot_a_is() {
    // A bare "www." has no URL opportunity before its dot.
    assert_eq!(url_band_offsets("see www. now"), [] as [usize; 0]);
    // "www.a" is a URL: an opportunity before the dot at offset 7.
    assert_eq!(url_band_offsets("see www.a now"), [7]);
}

#[test]
fn fe22_a_scheme_url_needs_no_host() {
    // The URL begins at "http://" and "a://" even with nothing after them:
    // what follows, here a dot, is still inside the URL.
    assert_eq!(url_band_offsets("see http://.a now"), [11]);
    assert_eq!(url_band_offsets("see a://.a now"), [8]);
    // Nothing is strictly inside a bare scheme.
    for text in ["see http:// now", "see a:// now", "http://", "a://"] {
        assert_eq!(url_band_offsets(text), [] as [usize; 0], "{text:?}");
    }
}
