//! The AST that `compile_paragraph` emits for decoration spans: the order of
//! `SpanBoundary` items (AST-8), the membership of `cloned_spans` and the
//! padding folded into discretionaries (WN-8, WN-21), for nested, sequential
//! and equal-range spans of mixed `Clone` and `Slice` modes.
//!
//! The spec does not fix the order of ids inside one `cloned_spans` list, nor
//! whether `CLONED_SPAN_PENALTY` is added once or once per span; these tests
//! compare lists as sorted sets and bound the penalty from below.

use crate::common::*;
use mutil_engine::BoxDecorationBreak::{Clone as Cl, Slice as Sl};
use mutil_engine::*;

fn span(
    span_id: u64,
    start: usize,
    end: usize,
    start_padding: f32,
    end_padding: f32,
    break_mode: BoxDecorationBreak,
) -> InlineDecorationSpan {
    InlineDecorationSpan {
        span_id,
        byte_range: TextRange::new(start, end),
        start_padding,
        end_padding,
        break_mode,
    }
}

fn compile_spans(text: &str, spans: &[InlineDecorationSpan]) -> ParagraphAst {
    let base = style(10.0);
    let paragraph = ParagraphInput {
        decoration_spans: spans,
        ..input(text, &base)
    };
    compile_paragraph(&paragraph, &FrontendOptions::default(), &*SHAPER)
        .unwrap_or_else(|e| panic!("{e}"))
}

/// The item sequence: `<id` a Leading edge, `id>` a Trailing edge, `[..]` a
/// text box, `_` glue, `|` a soft break, `H` a hard break, `$` the sentinel;
/// a break with a non-empty `cloned_spans` is followed by `{id,id}` (sorted).
fn trace(ast: &ParagraphAst, text: &str) -> String {
    let tokens: Vec<String> = ast
        .items
        .iter()
        .map(|item| match item {
            WordNode::Box(BoxNode::SpanBoundary { span_id, edge, .. }) => match edge {
                SpanEdge::Leading => format!("<{span_id}"),
                SpanEdge::Trailing => format!("{span_id}>"),
            },
            WordNode::Box(BoxNode::Text { byte_range, .. }) => {
                format!("[{}]", &text[byte_range.as_range()])
            }
            WordNode::Box(BoxNode::InlineObject { .. }) => "O".into(),
            WordNode::Glue(_) => "_".into(),
            WordNode::Tab { .. } => "T".into(),
            WordNode::Discretionary(d) => {
                let mark = if d.is_paragraph_end() {
                    "$"
                } else if d.is_mandatory() {
                    "H"
                } else {
                    "|"
                };
                let mut ids = d.cloned_spans.to_vec();
                ids.sort_unstable();
                if ids.is_empty() {
                    mark.into()
                } else {
                    let ids: Vec<String> = ids.iter().map(u64::to_string).collect();
                    format!("{mark}{{{}}}", ids.join(","))
                }
            }
        })
        .collect();
    tokens.join(" ")
}

/// Checks WN-8 and WN-21 for every item against `spans`, from the spec alone.
#[track_caller]
fn assert_spec_widths(ast: &ParagraphAst, spans: &[InlineDecorationSpan]) {
    assert_eq!(ast.decoration_spans, spans, "the stream is stored as given");
    for s in spans {
        let edges: Vec<(SpanEdge, TextRange, f32)> = ast
            .items
            .iter()
            .filter_map(|item| match item {
                WordNode::Box(BoxNode::SpanBoundary {
                    span_id,
                    edge,
                    width,
                    byte_range,
                    ..
                }) if *span_id == s.span_id => Some((*edge, *byte_range, *width)),
                _ => None,
            })
            .collect();
        assert_eq!(edges.len(), 2, "span {}: one boundary per edge", s.span_id);
        for (edge, range, width) in edges {
            let (anchor, padding) = match edge {
                SpanEdge::Leading => (s.byte_range.start(), s.start_padding),
                SpanEdge::Trailing => (s.byte_range.end(), s.end_padding),
            };
            assert!(range.is_empty(), "span {}: empty range", s.span_id);
            assert_eq!(range.start(), anchor, "span {}: anchor", s.span_id);
            assert_close(width, padding);
        }
    }
    for item in &ast.items {
        let WordNode::Discretionary(d) = item else {
            continue;
        };
        let offset = d.byte_offset();
        let mut expected: Vec<u64> = spans
            .iter()
            .filter(|s| {
                s.break_mode == BoxDecorationBreak::Clone
                    && s.byte_range.start() < offset
                    && offset < s.byte_range.end()
            })
            .map(|s| s.span_id)
            .collect();
        expected.sort_unstable();
        let mut actual = d.cloned_spans.to_vec();
        actual.sort_unstable();
        if d.is_paragraph_end() {
            assert!(actual.is_empty(), "the sentinel lists no span");
            assert_eq!((d.pre_break_width, d.post_break_width), (0.0, 0.0));
            continue;
        }
        assert_eq!(actual, expected, "cloned_spans of the break at {offset:?}");
        let by_id = |id: &u64| spans.iter().find(|s| s.span_id == *id).unwrap();
        let clone_pre: f32 = expected.iter().map(|id| by_id(id).end_padding).sum();
        let clone_post: f32 = expected.iter().map(|id| by_id(id).start_padding).sum();
        assert_close(d.pre_break_width, clone_pre);
        assert_close(d.post_break_width, clone_post);
        if d.is_mandatory() {
            assert_eq!(d.penalty, DiscretionaryNode::MANDATORY_BREAK_PENALTY);
        } else if expected.is_empty() {
            assert_eq!(d.penalty, 0);
        } else if expected.len() == 1 {
            assert_eq!(d.penalty, DiscretionaryNode::CLONED_SPAN_PENALTY);
        } else {
            assert!(d.penalty >= DiscretionaryNode::CLONED_SPAN_PENALTY);
        }
    }
}

#[track_caller]
fn check(text: &str, spans: &[InlineDecorationSpan], expected: &str) -> ParagraphAst {
    let ast = compile_spans(text, spans);
    assert_eq!(trace(&ast, text), expected);
    assert_spec_widths(&ast, spans);
    ast
}

fn soft_breaks(ast: &ParagraphAst) -> Vec<&DiscretionaryNode> {
    ast.items
        .iter()
        .filter_map(|item| match item {
            WordNode::Discretionary(d) if !d.is_paragraph_end() && !d.is_mandatory() => Some(d),
            _ => None,
        })
        .collect()
}

#[test]
fn sequential_clone_spans_meet_as_trailing_glue_break_leading() {
    // "ab cd ef gh": words at 0..2, 3..5, 6..8, 9..11; breaks at 3, 6, 9.
    let spans = [span(1, 0, 5, 1.0, 2.0, Cl), span(2, 6, 11, 4.0, 8.0, Cl)];
    let ast = check(
        "ab cd ef gh",
        &spans,
        "<1 [ab] _ |{1} [cd] 1> _ | <2 [ef] _ |{2} [gh] 2> $",
    );
    let breaks = soft_breaks(&ast);
    assert_eq!(
        (breaks[0].pre_break_width, breaks[0].post_break_width),
        (2.0, 1.0)
    );
    assert_eq!(
        (breaks[2].pre_break_width, breaks[2].post_break_width),
        (8.0, 4.0)
    );
}

#[test]
fn sequential_mixed_modes_only_clone_pads_the_break() {
    let spans = [span(1, 0, 5, 1.0, 2.0, Sl), span(2, 6, 11, 4.0, 8.0, Cl)];
    let ast = check(
        "ab cd ef gh",
        &spans,
        "<1 [ab] _ | [cd] 1> _ | <2 [ef] _ |{2} [gh] 2> $",
    );
    let breaks = soft_breaks(&ast);
    assert!(breaks[0].cloned_spans.is_empty());
    assert_eq!(breaks[0].penalty, 0);
}

#[test]
fn three_deep_mixed_nesting_with_distinct_ends() {
    // 1 Clone [0,14) > 2 Slice [3,11) > 3 Clone [6,11); breaks at 3, 6, 9, 12.
    let spans = [
        span(1, 0, 14, 1.0, 2.0, Cl),
        span(2, 3, 11, 10.0, 20.0, Sl),
        span(3, 6, 11, 100.0, 200.0, Cl),
    ];
    let ast = check(
        "ab cd ef gh ij",
        &spans,
        "<1 [ab] _ |{1} <2 [cd] _ |{1} <3 [ef] _ |{1,3} [gh] 3> 2> _ |{1} [ij] 1> $",
    );
    let breaks = soft_breaks(&ast);
    // Slice paddings never reach a break; the two Clone spans sum.
    assert_eq!(
        (breaks[2].pre_break_width, breaks[2].post_break_width),
        (202.0, 101.0)
    );
}

#[test]
fn three_deep_nesting_with_a_shared_end_closes_inner_first() {
    let spans = [
        span(4, 0, 11, 1.0, 2.0, Cl),
        span(5, 3, 11, 10.0, 20.0, Sl),
        span(6, 6, 11, 100.0, 200.0, Cl),
    ];
    check(
        "ab cd ef gh",
        &spans,
        "<4 [ab] _ |{4} <5 [cd] _ |{4} <6 [ef] _ |{4,6} [gh] 6> 5> 4> $",
    );
}

#[test]
fn three_deep_nesting_with_a_shared_start_opens_outer_first() {
    let spans = [
        span(1, 0, 8, 1.0, 2.0, Cl),
        span(2, 0, 5, 10.0, 20.0, Sl),
        span(3, 0, 2, 100.0, 200.0, Cl),
    ];
    check(
        "ab cd ef gh",
        &spans,
        "<1 <2 <3 [ab] 3> _ |{1} [cd] 2> _ |{1} [ef] 1> _ | [gh] $",
    );
}

#[test]
fn equal_ranges_open_outer_first_and_close_inner_first_by_index_not_id() {
    // Ids deliberately out of numeric order: the lower index is the outer.
    let spans = [
        span(50, 3, 8, 1.0, 2.0, Cl),
        span(20, 3, 8, 10.0, 20.0, Sl),
        span(30, 3, 8, 100.0, 200.0, Cl),
    ];
    let ast = check(
        "ab cd ef gh",
        &spans,
        "[ab] _ | <50 <20 <30 [cd] _ |{30,50} [ef] 30> 20> 50> _ | [gh] $",
    );
    // The break at the shared start precedes all three Leading edges and the
    // break after the shared end follows all three Trailing edges, unpadded.
    let breaks = soft_breaks(&ast);
    assert!(breaks[0].cloned_spans.is_empty());
    assert!(breaks[2].cloned_spans.is_empty());
}

#[test]
fn equal_ranges_ending_exactly_at_a_soft_break() {
    // [3,6) holds "cd " and ends where the break at 6 sits; it starts at the
    // break at 3. Neither break is strictly inside.
    let spans = [
        span(50, 3, 6, 1.0, 2.0, Cl),
        span(20, 3, 6, 10.0, 20.0, Cl),
        span(30, 3, 6, 100.0, 200.0, Sl),
    ];
    check(
        "ab cd ef gh",
        &spans,
        "[ab] _ | <50 <20 <30 [cd] _ 30> 20> 50> | [ef] _ | [gh] $",
    );
}

#[test]
fn equal_whole_text_ranges_nest_around_the_sentinel() {
    let spans = [span(1, 0, 5, 1.0, 2.0, Cl), span(2, 0, 5, 3.0, 4.0, Cl)];
    let ast = check("ab cd", &spans, "<1 <2 [ab] _ |{1,2} [cd] 2> 1> $");
    let breaks = soft_breaks(&ast);
    assert_eq!(
        (breaks[0].pre_break_width, breaks[0].post_break_width),
        (6.0, 4.0)
    );
}

#[test]
fn a_zero_padding_clone_span_still_has_edges_and_is_listed() {
    let spans = [span(9, 0, 5, 0.0, 0.0, Cl)];
    let ast = check("ab cd", &spans, "<9 [ab] _ |{9} [cd] 9> $");
    assert_eq!(
        soft_breaks(&ast)[0].penalty,
        DiscretionaryNode::CLONED_SPAN_PENALTY
    );
}

#[test]
fn a_slice_span_adds_nothing_to_the_break_inside_it() {
    let spans = [span(9, 0, 5, 3.0, 4.0, Sl)];
    let ast = check("ab cd", &spans, "<9 [ab] _ | [cd] 9> $");
    let b = soft_breaks(&ast)[0];
    assert_eq!(
        (b.pre_break_width, b.post_break_width, b.penalty),
        (0.0, 0.0, 0)
    );
}

#[test]
fn clone_nested_in_slice_with_a_slice_child() {
    // 1 Slice [0,11) > 2 Clone [3,8) > 3 Slice [3,5); the break at 6 lies in
    // 1 and 2 only and the one at 3 precedes the shared Leading edges.
    let spans = [
        span(1, 0, 11, 1.0, 2.0, Sl),
        span(2, 3, 8, 10.0, 20.0, Cl),
        span(3, 3, 5, 100.0, 200.0, Sl),
    ];
    check(
        "ab cd ef gh",
        &spans,
        "<1 [ab] _ | <2 <3 [cd] 3> _ |{2} [ef] 2> _ | [gh] 1> $",
    );
}

#[test]
fn spans_touching_a_hard_break_and_one_containing_it() {
    // "ab" 0..2, U+2028 owns 2..5, "cd" 5..7, space 7..8, "ef" 8..10.
    let text = "ab\u{2028}cd ef";
    let spans = [
        span(1, 0, 10, 1.0, 2.0, Cl),
        span(2, 0, 2, 4.0, 8.0, Cl),
        span(3, 5, 10, 16.0, 32.0, Cl),
    ];
    let ast = check(
        text,
        &spans,
        "<1 <2 [ab] 2> H{1} <3 [cd] _ |{1,3} [ef] 3> 1> $",
    );
    // A mandatory break lists the Clone spans and folds their padding but
    // takes no CLONED_SPAN_PENALTY (WN-21, WN-18).
    let hard = ast
        .items
        .iter()
        .find_map(|i| match i {
            WordNode::Discretionary(d) if d.is_mandatory() && !d.is_paragraph_end() => Some(d),
            _ => None,
        })
        .unwrap();
    assert_eq!(hard.penalty, DiscretionaryNode::MANDATORY_BREAK_PENALTY);
    assert_eq!((hard.pre_break_width, hard.post_break_width), (2.0, 1.0));
}
