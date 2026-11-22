//! Spatial compression: G1 validation, Tier 3 horizontal scale, and shape-fit.

use crate::common::*;
use mutil_engine::*;

#[test]
fn g1_rejects_invalid_compression_and_shape_fit_options() {
    let text = "hello world";
    let ast = compile(text);
    let base = ShapeFitPolicy {
        target_aspect_ratio: Some(2.0),
        max_width: 100.0,
        max_height: None,
        max_lines: 4,
        min_font_scale: 0.5,
        max_font_scale: 2.0,
    };
    let cases = [
        LayoutOptions {
            min_horizontal_scale: 0.0,
            ..Default::default()
        },
        LayoutOptions {
            min_horizontal_scale: -0.5,
            ..Default::default()
        },
        LayoutOptions {
            min_horizontal_scale: 1.5,
            ..Default::default()
        },
        LayoutOptions {
            min_horizontal_scale: f32::NAN,
            ..Default::default()
        },
        LayoutOptions {
            shape_fit: Some(ShapeFitPolicy {
                max_lines: 0,
                ..base
            }),
            ..Default::default()
        },
        LayoutOptions {
            shape_fit: Some(ShapeFitPolicy {
                max_lines: 17,
                ..base
            }),
            ..Default::default()
        },
        LayoutOptions {
            shape_fit: Some(ShapeFitPolicy {
                max_width: 0.0,
                ..base
            }),
            ..Default::default()
        },
        LayoutOptions {
            shape_fit: Some(ShapeFitPolicy {
                min_font_scale: 2.5,
                ..base
            }),
            ..Default::default()
        },
    ];
    for opts in &cases {
        assert!(
            check_layout_inputs(&ast, text, &column(100.0), opts).is_err(),
            "{opts:?}"
        );
    }
}

#[test]
fn tier3_horizontal_compression_fits_narrow_slot_on_fewer_lines() {
    let text = "hello world";
    let base = lay_out(text, &column(50.0), &LayoutOptions::default());
    let compressed = lay_out(
        text,
        &column(50.0),
        &LayoutOptions {
            min_horizontal_scale: 0.7,
            ..Default::default()
        },
    );
    assert!(compressed.lines.len() < base.lines.len());
    assert!(
        compressed.lines[0]
            .visual_runs
            .iter()
            .any(|r| r.x_scale < 1.0)
    );
}

#[test]
fn shape_fit_aspect_ratio_and_bounding_box_converge_validly() {
    let text = "The quick brown fox jumps over the lazy dog";
    let policies = [
        ShapeFitPolicy {
            target_aspect_ratio: Some(3.0),
            max_width: 140.0,
            max_height: None,
            max_lines: 4,
            min_font_scale: 0.5,
            max_font_scale: 2.0,
        },
        ShapeFitPolicy {
            target_aspect_ratio: None,
            max_width: 100.0,
            max_height: Some(36.0),
            max_lines: 4,
            min_font_scale: 0.5,
            max_font_scale: 2.0,
        },
    ];
    for policy in policies {
        let p = lay_out(
            text,
            &column(policy.max_width),
            &LayoutOptions {
                shape_fit: Some(policy),
                ..Default::default()
            },
        );
        assert!(!p.lines.is_empty() && p.lines.len() <= policy.max_lines);
    }
}

#[test]
fn shape_fit_keeps_inline_object_width_fixed_while_text_scales() {
    let text = "ab\u{fffc}cd";
    let base = style(10.0);
    let obj = [InlineObjectInput {
        byte_range: TextRange::new(2, 5),
        box_id: 1,
        width: 20.0,
        ascent: 8.0,
        descent: 2.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    let ast = compile_paragraph(
        &ParagraphInput {
            inline_objects: &obj,
            ..input(text, &base)
        },
        &FrontendOptions::default(),
        &*SHAPER,
    )
    .unwrap();
    let policy = ShapeFitPolicy {
        target_aspect_ratio: None,
        max_width: 120.0,
        max_height: Some(40.0),
        max_lines: 2,
        min_font_scale: 0.5,
        max_font_scale: 2.0,
    };
    let p = layout_paragraph(
        &ast,
        text,
        &*SHAPER,
        &column(120.0),
        &LayoutOptions {
            shape_fit: Some(policy),
            ..Default::default()
        },
    )
    .unwrap();
    ValidatedLayout::try_from_paragraph(&p, text).expect("G4 valid");
    let bx = p.lines.iter().flat_map(|l| &l.visual_boxes).next().unwrap();
    assert_close(bx.width, 20.0);
}
