//! One root cause per error variant (AST-12): the strut, the raw metrics of
//! the input and the metrics a shaper answers at layout time (G1 versus G2)
//! stay distinct.
//!
//! `InvalidFontMetrics` from raw strut metrics is covered by
//! `metrics::font_metrics_all_fields_and_overflow` (non-strut fields such as
//! `x_height` beyond `MAX_MEASUREMENT`, NaN in every field).

use crate::common::*;
use mutil_engine::*;

fn layout(
    a: &ParagraphAst,
    shaper: &dyn TextShaper,
    options: &LayoutOptions,
) -> Result<(), LayoutInputError> {
    layout_paragraph(a, "abc", shaper, &constraints(), options).map(drop)
}

/// SHP-9, FNT-4: a shaper whose metrics are not valid `FontMetrics` is
/// stopped at the shaper gate, not at the input gate.
#[test]
fn shp9_fnt4_shaper_metrics_invalid_raises_invalid_shaper_metrics() {
    let a = ast("abc");
    assert_eq!(check(&a, "abc"), Ok(()));
    let options = LayoutOptions::default();

    // NoopShaper's zero metrics are not valid (SHP-9).
    assert_eq!(
        layout(&a, &NoopShaper, &options),
        Err(LayoutInputError::InvalidShaperMetrics)
    );

    // A stub whose metrics break FNT-4.
    for broken in [
        FontMetrics {
            x_height: -1.0,
            ..TEST_SHAPER.0
        },
        FontMetrics {
            cap_height: f32::NAN,
            ..TEST_SHAPER.0
        },
        FontMetrics {
            ascent: f32::INFINITY,
            ..TEST_SHAPER.0
        },
    ] {
        assert!(!broken.is_valid());
        assert_eq!(
            layout(&a, &MetricsShaper(broken), &options),
            Err(LayoutInputError::InvalidShaperMetrics),
            "{broken:?}"
        );
    }
    assert_eq!(layout(&a, &TEST_SHAPER, &options), Ok(()));
}

/// SHP-9, MET-1: metrics that are each valid but resolve, under a valid line
/// height, to an invalid line box also raise `InvalidShaperMetrics`.
#[test]
fn shp9_met1_shaper_line_box_invalid_raises_invalid_shaper_metrics() {
    let a = ast("abc");

    // Normal: ascent + descent + line gap exceeds MAX_MEASUREMENT.
    let big = MAX_MEASUREMENT * 0.4;
    let metrics = FontMetrics {
        ascent: big,
        descent: big,
        line_gap: big,
        ..TEST_SHAPER.0
    };
    assert!(metrics.is_valid(), "each number is a valid measurement");
    assert!(
        !LineHeight::Normal
            .resolve_strut(&metrics, 10.0, WritingMode::HorizontalTb)
            .is_valid()
    );
    assert_eq!(
        layout(&a, &MetricsShaper(metrics), &LayoutOptions::default()),
        Err(LayoutInputError::InvalidShaperMetrics)
    );
}

/// AST-6, MET-1: a strut that resolves to an invalid line box is
/// `InvalidStrut` at the input gate, whichever shaper is used.
#[test]
fn ast6_met1_strut_line_box_invalid_raises_invalid_strut() {
    let a = ast("abc");
    let options = LayoutOptions {
        line_height: LineHeight::Fixed(MAX_MEASUREMENT),
        ..Default::default()
    };
    assert!(options.line_height.is_valid() && a.strut_metrics.is_valid());
    assert_eq!(
        check_layout_inputs(&a, "abc", &constraints(), &options),
        Err(LayoutInputError::InvalidStrut)
    );
    for shaper in [&TEST_SHAPER as &dyn TextShaper, &NoopShaper] {
        assert_eq!(
            layout(&a, shaper, &options),
            Err(LayoutInputError::InvalidStrut)
        );
    }
}

/// AST-12: the causes stay distinct variants with distinct messages, and each
/// gate fires only for its own cause.
#[test]
fn ast12_strut_font_metrics_and_shaper_metrics_are_distinct() {
    assert_ne!(
        LayoutInputError::InvalidStrut,
        LayoutInputError::InvalidShaperMetrics
    );
    assert_ne!(
        LayoutInputError::InvalidFontMetrics,
        LayoutInputError::InvalidShaperMetrics
    );
    assert_ne!(
        LayoutInputError::InvalidFontMetrics.to_string(),
        LayoutInputError::InvalidShaperMetrics.to_string()
    );

    // Valid input, broken shaper: only the shaper gate fires.
    let a = ast("abc");
    assert_eq!(check(&a, "abc"), Ok(()));
    assert_eq!(
        layout(&a, &NoopShaper, &LayoutOptions::default()),
        Err(LayoutInputError::InvalidShaperMetrics)
    );
    // Broken input metric, valid shaper: only the input gate fires.
    let mut bad = ast("abc");
    bad.strut_metrics.x_height = 2.0 * MAX_MEASUREMENT;
    assert_eq!(
        layout(&bad, &TEST_SHAPER, &LayoutOptions::default()),
        Err(LayoutInputError::InvalidFontMetrics)
    );
}
