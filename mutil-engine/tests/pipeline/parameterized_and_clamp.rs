use crate::common::*;
use mutil_engine::LineBreakStrategy::*;
use mutil_engine::LineClampPolicy as Clamp;
use mutil_engine::SpanEdgeCut::{ClonedWrap, SlicedWrap};
use mutil_engine::*;

#[test]
fn three_strategies_validate_tile_and_a_break_differs() {
    let text = "a bb ccc dddd eeeee ffffff https://example.com/segmented-path ggggggg hhhhhhhh a";
    let ast = compile(text);
    let mut opts = LayoutOptions::default();
    let diverged = (0..=38).any(|step| {
        let w = column(42.0 + 6.0 * step as f32);
        let [g, b, p] = [Greedy, Balanced, Publication].map(|strategy| {
            opts.strategy = strategy;
            let out = layout_paragraph(&ast, text, &*SHAPER, &w, &opts).unwrap();
            ValidatedLayout::try_from_paragraph(&out, text).unwrap();
            line_texts(&out, text)
        });
        g != p || g != b
    });
    assert!(diverged);
}

#[test]
fn clamp_variants_keep_one_ellipsized_line_and_respect_boundaries() {
    let text = "one two three four five six seven eight nine ten eleven twelve";
    let base = style(10.0);
    let deco = [InlineDecorationSpan {
        span_id: 1,
        byte_range: TextRange::new(4, 55),
        start_padding: 0.0,
        end_padding: 0.0,
        break_mode: BoxDecorationBreak::Slice,
        height_style: BoxHeightStyle::Strut,
    }];
    let atom = [AtomicEditSpan {
        span_id: 2,
        byte_range: TextRange::new(14, 18),
    }];
    let mut para = input(text, &base);
    (para.decoration_spans, para.atomic_spans) = (&deco, &atom);
    let ast = compile_paragraph(&para, &FrontendOptions::default(), &*SHAPER).unwrap();
    let (w, mut opts) = (column(60.0), LayoutOptions::default());
    let h = layout_paragraph(&ast, text, &*SHAPER, &w, &opts)
        .unwrap()
        .lines[0]
        .line_height;
    let custom = |e: &str| Clamp {
        ellipsis: e.into(),
        ..Clamp::max_lines(2)
    };
    for (what, clamp, expect_n) in [
        ("max_lines(1)", Clamp::max_lines(1), Some(1)),
        ("max_lines(2)", Clamp::max_lines(2), Some(2)),
        ("max_block_size", Clamp::max_block_size(2.5 * h), None),
        ("custom …", custom("…"), Some(2)),
        ("custom empty", custom(""), Some(2)),
        ("line 0", Clamp::max_block_size(0.5 * h), Some(1)),
    ] {
        opts.clamp = Some(clamp);
        let p = layout_paragraph(&ast, text, &*SHAPER, &w, &opts).unwrap();
        ValidatedLayout::try_from_paragraph(&p, text).unwrap();
        let last = p.lines.last().unwrap();
        assert_eq!(
            p.truncated_range,
            Some(TextRange::new(last.byte_range.end(), text.len()))
        );
        assert!(expect_n.is_none_or(|n| p.lines.len() == n), "{what}");
        let lf = last.span_fragments.last().unwrap();
        assert!(matches!(lf.edges.right, SlicedWrap | ClonedWrap), "{what}");
    }
}
