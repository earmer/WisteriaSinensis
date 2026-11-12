//! Font metrics, struts and line-height resolution.

use crate::common::*;
use mutil_engine::*;

#[test]
fn reject_negative_strut() {
    let mut a = ast("abc");
    a.strut_metrics.ascent = -8.;
    a.strut_metrics.descent = -2.;
    assert_eq!(check(&a, "abc"), Err(LayoutInputError::InvalidStrut));
}

#[test]
fn zero_strut_font_size_is_rejected() {
    let mut a = ast("abc");
    a.strut_font_size = 0.;
    assert_eq!(check(&a, "abc"), Err(LayoutInputError::InvalidStrut));
}

#[test]
fn zero_run_font_size_is_rejected() {
    let mut a = ast("abc");
    a.runs[0].font_size = 0.;
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidShapingRun { run: 0 })
    );
}

#[test]
fn zero_fixed_multiplier_is_rejected() {
    let a = ast("abc");
    let options = LayoutOptions {
        line_height: LineHeight::Fixed(0.),
        ..Default::default()
    };
    assert_eq!(
        check_layout_inputs(
            &a,
            "abc",
            &LayoutConstraints {
                first_line_indent: 0.0,
                geometry_provider: None,
                hanging_indent: 0.0,
                wrap_width: 100.,
                tab_stops: TabStops::DEFAULT
            },
            &options
        ),
        Err(LayoutInputError::InvalidStrut)
    );
}

#[test]
fn negative_half_leading_with_positive_total_is_allowed() {
    let a = ast("abc");
    let options = LayoutOptions {
        line_height: LineHeight::Fixed(0.5),
        ..Default::default()
    };
    assert_eq!(
        check_layout_inputs(&a, "abc", &constraints(), &options),
        Ok(())
    );
    let strut = LayoutPolicySnapshot::resolve(&a, &constraints(), &options)
        .unwrap()
        .strut;
    assert!(strut.descent < 0.);
    assert_eq!(strut.line_height(), 5.);
}

#[test]
fn raw_metrics_with_one_negative_depth_are_rejected() {
    let mut a = ast("abc");
    a.strut_metrics.descent = -1.;
    assert_eq!(check(&a, "abc"), Err(LayoutInputError::InvalidStrut));
}

#[test]
fn zero_total_raw_metrics_are_rejected() {
    let mut a = ast("abc");
    a.strut_metrics.ascent = 0.;
    a.strut_metrics.descent = 0.;
    assert_eq!(check(&a, "abc"), Err(LayoutInputError::InvalidStrut));
}

#[test]
fn font_metrics_all_fields_and_overflow() {
    let valid = ast("a").strut_metrics;
    for i in 0..15 {
        let mut a = ast("a");
        let m = &mut a.strut_metrics;
        let fields = [
            &mut m.ascent,
            &mut m.descent,
            &mut m.line_gap,
            &mut m.vertical_ascent,
            &mut m.vertical_descent,
            &mut m.vertical_line_gap,
            &mut m.underline_position,
            &mut m.underline_thickness,
            &mut m.strikeout_position,
            &mut m.x_height,
            &mut m.cap_height,
            &mut m.ideographic_baseline_offset,
            &mut m.central_baseline_offset,
            &mut m.hanging_baseline_offset,
            &mut m.mathematical_baseline_offset,
        ];
        *fields.into_iter().nth(i).unwrap() = f32::NAN;
        // The first three fields are the horizontal strut itself; the rest are
        // metrics the strut does not use.
        let expected = if i < 3 {
            LayoutInputError::InvalidStrut
        } else {
            LayoutInputError::InvalidFontMetrics
        };
        assert_eq!(check(&a, "a"), Err(expected), "field {i}");
    }
    let mut a = ast("a");
    a.strut_metrics.ascent = f32::MAX;
    a.strut_metrics.descent = f32::MAX;
    assert_eq!(check(&a, "a"), Err(LayoutInputError::InvalidFontMetrics));
    a.strut_metrics = valid;
    a.strut_metrics.vertical_ascent = -1.0;
    assert_eq!(check(&a, "a"), Err(LayoutInputError::InvalidFontMetrics));
    a.strut_metrics = valid;
    a.strut_metrics.vertical_ascent = 9.0;
    a.strut_metrics.vertical_descent = 3.0;
    orient(
        &mut a,
        WritingMode::VerticalLr,
        GlyphOrientation::VerticalUpright,
        BidiLevel::LTR,
    );
    assert_eq!(check(&a, "a"), Ok(()));
    let strut = LayoutPolicySnapshot::resolve(&a, &constraints(), &LayoutOptions::default())
        .unwrap()
        .strut;
    assert_eq!((strut.ascent, strut.descent), (9.0, 3.0));
}
