//! FE-25, FNT-2: a style's features are canonical as built, so the order in
//! which they were pushed does not reach the compiled runs.

use crate::common::*;
use mutil_engine::*;

fn style_with(pushes: &[([u8; 4], u32)]) -> TextStyle {
    let mut style = style(10.0);
    for &(tag, value) in pushes {
        style.features.push(tag, value);
    }
    style
}

fn run_features(style: &TextStyle) -> Vec<FontFeatures> {
    let ast = compile_paragraph(
        &input("abc def", style),
        &FrontendOptions::default(),
        &*SHAPER,
    )
    .expect("the frontend accepts the text");
    assert!(!ast.runs.is_empty());
    ast.runs.iter().map(|run| run.features.clone()).collect()
}

#[test]
fn styles_differing_only_in_feature_push_order_compile_to_equal_runs() {
    let forward = style_with(&[(*b"kern", 1), (*b"liga", 0), (*b"chws", 1)]);
    let backward = style_with(&[(*b"chws", 1), (*b"liga", 0), (*b"kern", 1)]);
    let repeated = style_with(&[(*b"liga", 1), (*b"kern", 1), (*b"chws", 1), (*b"liga", 0)]);
    let expected = run_features(&forward);
    assert_eq!(run_features(&backward), expected);
    assert_eq!(run_features(&repeated), expected);
    let tags: Vec<[u8; 4]> = expected[0].as_slice().iter().map(|f| f.tag).collect();
    assert_eq!(tags, [*b"chws", *b"kern", *b"liga"]);
}

#[test]
fn a_style_span_with_unsorted_features_compiles_like_a_sorted_one() {
    let default = style(10.0);
    let unsorted = style_with(&[(*b"liga", 0), (*b"kern", 1)]);
    let sorted = style_with(&[(*b"kern", 1), (*b"liga", 0)]);
    let compile_span = |span_style: TextStyle| {
        let spans = [StyleSpan {
            byte_range: TextRange::new(0, 3),
            style: span_style,
        }];
        let paragraph = ParagraphInput {
            style_spans: &spans,
            ..input("abc def", &default)
        };
        let ast = compile_paragraph(&paragraph, &FrontendOptions::default(), &*SHAPER)
            .expect("the frontend accepts the text");
        ast.runs
            .iter()
            .map(|run| run.features.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(compile_span(unsorted), compile_span(sorted));
}
