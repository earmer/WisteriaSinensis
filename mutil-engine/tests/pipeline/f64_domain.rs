//! Pipeline tests for the `f64` internal layout domain (MSR-1, ST3-2, ST3-4).

use crate::acceptance_phase2::glyphs;
use crate::common::*;
use mutil_engine::*;

fn justify_opts() -> LayoutOptions {
    LayoutOptions {
        justify: true,
        ..Default::default()
    }
}

fn assert_aligned_edges(line: &PositionedLine) {
    let slot = &line.slots[0];
    let gs: Vec<_> = glyphs(line).collect();
    let (first, last) = (gs.first().unwrap(), gs.last().unwrap());
    assert!((first.x - slot.content_start).abs() <= LINE_GEOMETRY_ABS_EPSILON);
    assert!((last.x + last.advance - slot.content_end()).abs() <= LINE_GEOMETRY_ABS_EPSILON);
}

#[test]
fn zero_drift_justified_prime_gaps() {
    // 7 word spaces (prime) with 11.0 stretch across 7 gaps; 13 CJK gaps with 7.0 stretch.
    let cases: [(&str, Option<&[&str]>, f32); 2] = [
        ("aa bb cc dd ee ff gg hh zzzzzzzzzz", None, 149.0),
        (
            "日本語中文漢字排版測試東京北京人大小上下山川水火",
            Some(&[JAPANESE]),
            147.0,
        ),
    ];
    for (text, fonts, wrap_width) in cases {
        let p = match fonts {
            Some(f) => lay_out_in(text, f, &column(wrap_width), &justify_opts()),
            None => lay_out(text, &column(wrap_width), &justify_opts()),
        };
        assert!(ValidatedLayout::try_from_paragraph(&p, text).is_ok());
        assert_eq!(p.lines.len(), 2);
        let line = &p.lines[0];
        assert!(line.is_justified);
        assert!((line.visual_width() - wrap_width).abs() <= LINE_GEOMETRY_ABS_EPSILON);
        assert_aligned_edges(line);
    }
}

#[test]
fn exact_single_rounding_rtl_alignment() {
    let text = "שלום עולם";
    let p = lay_out_bidi(text, &[HEBREW], &column(200.0), &LayoutOptions::default());
    assert!(ValidatedLayout::try_from_paragraph(&p, text).is_ok());
    assert_eq!(p.lines.len(), 1);
    assert_aligned_edges(&p.lines[0]);
}

#[test]
fn max_measurement_boundary_is_2_pow_20() {
    assert_eq!(MAX_MEASUREMENT, 1_048_576.0);
    assert!(is_measurement(MAX_MEASUREMENT) && is_extent(MAX_MEASUREMENT));
    assert!(!is_measurement(MAX_MEASUREMENT + 1.0) && !is_measurement(-MAX_MEASUREMENT - 1.0));

    let text = "a b";
    let base = compile(text);
    let (c, o) = (column(100.0), LayoutOptions::default());
    for (val, ok) in [(MAX_MEASUREMENT, true), (MAX_MEASUREMENT + 1.0, false)] {
        let mut font = base.clone();
        font.runs[0].font_size = val;
        assert_eq!(check_layout_inputs(&font, text, &c, &o).is_ok(), ok);

        let mut spacing = base.clone();
        spacing.runs[0].letter_spacing = val;
        assert_eq!(check_layout_inputs(&spacing, text, &c, &o).is_ok(), ok);

        let mut glue = base.clone();
        for item in &mut glue.items {
            if let WordNode::Glue(g) = item {
                g.natural_width = val;
            }
        }
        assert_eq!(check_layout_inputs(&glue, text, &c, &o).is_ok(), ok);
    }
}
