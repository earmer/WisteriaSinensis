//! A very long unbreakable word (WN-6, AX-8, G1 `InvalidTextWidth`).
//!
//! The font size is chosen so the 600-unit advance of the test font is not
//! a short binary fraction; a dyadic advance would sum exactly in f32 and
//! hide any difference in summation order.

use crate::common::*;
use mutil_engine::*;

/// 0.6 em at this size is 9.12336, which is not exactly representable.
const FONT_SIZE: f32 = 15.2056;

fn compile_at(text: &str, font_size: f32) -> ParagraphAst {
    let style = style(font_size);
    compile_paragraph(&input(text, &style), &FrontendOptions::default(), &*SHAPER)
        .expect("the frontend accepts the text")
}

#[test]
fn a_long_unbreakable_word_lays_out_and_passes_the_output_gate() {
    for n in [2000usize, 5000] {
        let text = "a".repeat(n);
        let ast = compile_at(&text, FONT_SIZE);
        let paragraph = layout_paragraph(
            &ast,
            &text,
            &*SHAPER,
            &column(100.0),
            &LayoutOptions::default(),
        )
        .unwrap_or_else(|e| panic!("n = {n}: {e}"));
        if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&paragraph, &text) {
            panic!("n = {n}: line {line}: {error}");
        }
    }
}

#[test]
fn wn6_text_width_is_the_left_to_right_f32_sum_of_its_clusters() {
    for (n, font_size) in [2000usize, 5000]
        .into_iter()
        .flat_map(|n| [FONT_SIZE, 14.5, 13.37, 7.77].map(|size| (n, size)))
    {
        let text = "a".repeat(n);
        let ast = compile_at(&text, font_size);
        let mut texts = 0;
        for item in &ast.items {
            let WordNode::Box(BoxNode::Text {
                cluster_range,
                natural_width,
                ..
            }) = item
            else {
                continue;
            };
            texts += 1;
            let mut sum = 0.0f32;
            for cluster in &ast.clusters[cluster_range.clone()] {
                sum += cluster.natural_width;
            }
            assert_eq!(*natural_width, sum, "n = {n}, font size {font_size}");
        }
        assert!(texts > 0);
    }
}
