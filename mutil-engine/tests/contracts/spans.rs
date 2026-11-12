//! Decoration, atomic and explicit bidi span ordering and nesting.

use crate::common::*;
use mutil_engine::*;

fn spanned(n: usize) -> (String, ParagraphAst) {
    let t = "a".repeat(n);
    let mut a = ast(&t);
    rebuild_items(&mut a, |_| vec![]);
    for i in 0..n {
        let r = TextRange::new(i, i + 1);
        a.decoration_spans.push(InlineDecorationSpan {
            span_id: i as u64,
            byte_range: r,
            start_padding: 0.,
            end_padding: 0.,
            break_mode: BoxDecorationBreak::Slice,
        });
        a.atomic_spans.push(AtomicEditSpan {
            span_id: (n + i) as u64,
            byte_range: r,
        });
        for edge in [SpanEdge::Leading, SpanEdge::Trailing] {
            let off = if edge == SpanEdge::Leading { i } else { i + 1 };
            a.items.push(WordNode::Box(BoxNode::SpanBoundary {
                byte_range: TextRange::empty_at(off),
                span_id: i as u64,
                edge,
                width: 0.,
                bidi_level: BidiLevel::LTR,
            }));
            if edge == SpanEdge::Leading {
                let text = boxed(&mut a, i, i + 1);
                a.items.push(text);
            }
        }
    }
    a.items.push(end(n));
    (t, a)
}

#[test]
#[ignore = "manual release-mode benchmark; elapsed-time ratios are unsuitable for regular CI"]
fn five_thousand_spans_scale_linearly() {
    fn measure(n: usize) -> std::time::Duration {
        let (t, a) = spanned(n);
        assert!(check(&a, &t).is_ok());
        (0..3)
            .map(|_| {
                let start = std::time::Instant::now();
                assert!(std::hint::black_box(check(std::hint::black_box(&a), &t)).is_ok());
                start.elapsed()
            })
            .min()
            .unwrap()
    }
    let small = measure(1250);
    let large = measure(5000);
    eprintln!("1250: {small:?}; 5000: {large:?}");
    assert!(
        large.as_secs_f64() / small.as_secs_f64() < 7.,
        "4x input should approach 4x work, not 16x; operation counting needs implementation instrumentation"
    );
}

#[test]
#[ignore = "manual release-mode benchmark; elapsed-time ratios are unsuitable for regular CI"]
fn long_hyphen_chains_scale_linearly() {
    fn measure(n: usize) -> std::time::Duration {
        let (t, a) = hyphen_chain_ast(n, |offset, n| {
            ((offset - 1) as u32, (n + 1 - offset) as u32)
        });
        assert!(check(&a, &t).is_ok());
        (0..3)
            .map(|_| {
                let start = std::time::Instant::now();
                assert!(std::hint::black_box(check(std::hint::black_box(&a), &t)).is_ok());
                start.elapsed()
            })
            .min()
            .unwrap()
    }
    let small = measure(1250);
    let large = measure(5000);
    eprintln!("1250: {small:?}; 5000: {large:?}");
    assert!(
        large.as_secs_f64() / small.as_secs_f64() < 7.,
        "4x chain length should approach 4x work, not 16x"
    );
}

fn decorated(ranges: &[(usize, usize)]) -> ParagraphAst {
    let mut a = ast("abcd");
    rebuild_items(&mut a, |_| vec![]);
    a.decoration_spans = ranges
        .iter()
        .enumerate()
        .map(|(id, &(s, e))| InlineDecorationSpan {
            span_id: id as u64,
            byte_range: TextRange::new(s, e),
            start_padding: 0.,
            end_padding: 0.,
            break_mode: BoxDecorationBreak::Slice,
        })
        .collect();
    // Nodes stay in canonical geometric order even when the span array is
    // permuted: a failure must come from the array's ordering contract.
    let mut ordered: Vec<_> = a.decoration_spans.clone();
    ordered.sort_by_key(|sp| {
        (
            sp.byte_range.start(),
            std::cmp::Reverse(sp.byte_range.end()),
        )
    });
    for offset in 0..=4 {
        for sp in ordered
            .iter()
            .rev()
            .filter(|sp| sp.byte_range.end() == offset)
        {
            a.items.push(WordNode::Box(BoxNode::SpanBoundary {
                byte_range: TextRange::empty_at(offset),
                span_id: sp.span_id,
                edge: SpanEdge::Trailing,
                width: 0.,
                bidi_level: BidiLevel::LTR,
            }));
        }
        for sp in ordered.iter().filter(|sp| sp.byte_range.start() == offset) {
            a.items.push(WordNode::Box(BoxNode::SpanBoundary {
                byte_range: TextRange::empty_at(offset),
                span_id: sp.span_id,
                edge: SpanEdge::Leading,
                width: 0.,
                bidi_level: BidiLevel::LTR,
            }));
        }
        if offset < 4 {
            let text = boxed(&mut a, offset, offset + 1);
            a.items.push(text);
        }
    }
    a.items.push(end(4));
    a
}

#[test]
fn same_start_outer_span_precedes_inner() {
    assert!(check(&decorated(&[(0, 4), (0, 2)]), "abcd").is_ok());
}

#[test]
fn same_start_inner_first_is_rejected() {
    assert_eq!(
        check(&decorated(&[(0, 2), (0, 4)]), "abcd"),
        Err(LayoutInputError::InvalidDecorationSpan { span: 1 })
    );
}

#[test]
fn decreasing_span_starts_are_rejected() {
    for ranges in [[(2, 4), (0, 2)], [(2, 4), (0, 4)]] {
        assert_eq!(
            check(&decorated(&ranges), "abcd"),
            Err(LayoutInputError::InvalidDecorationSpan { span: 1 }),
            "{ranges:?}"
        );
    }
}

#[test]
fn equal_ranges_keep_input_order() {
    assert!(check(&decorated(&[(0, 4), (0, 4)]), "abcd").is_ok());
}

#[test]
fn atomic_spans_require_increasing_starts() {
    let (t, mut a) = spanned(2);
    assert!(check(&a, &t).is_ok());
    a.atomic_spans.reverse();
    assert_eq!(
        check(&a, &t),
        Err(LayoutInputError::InvalidAtomicSpan { span: 1 })
    );
}

#[test]
fn atomic_spans_are_ascending_and_disjoint() {
    let (t, mut a) = spanned(4);
    // Later start first, then a span reaching back over it.
    a.atomic_spans = vec![
        AtomicEditSpan {
            span_id: 1,
            byte_range: TextRange::new(2, 4),
        },
        AtomicEditSpan {
            span_id: 2,
            byte_range: TextRange::new(0, 4),
        },
    ];
    assert_eq!(
        check(&a, &t),
        Err(LayoutInputError::InvalidAtomicSpan { span: 1 })
    );
    // Ascending starts but overlapping, or nested in the first.
    for second in [(1, 3), (1, 2)] {
        a.atomic_spans = vec![
            AtomicEditSpan {
                span_id: 1,
                byte_range: TextRange::new(0, 2),
            },
            AtomicEditSpan {
                span_id: 2,
                byte_range: TextRange::new(second.0, second.1),
            },
        ];
        assert_eq!(
            check(&a, &t),
            Err(LayoutInputError::InvalidAtomicSpan { span: 1 }),
            "{second:?}"
        );
    }
}

#[test]
fn thousands_of_spans_validate() {
    // Deterministic, untimed counterpart of `five_thousand_spans_scale_linearly`.
    let (t, a) = spanned(3000);
    assert_eq!(a.decoration_spans.len(), 3000);
    assert_eq!(a.atomic_spans.len(), 3000);
    assert!(check(&a, &t).is_ok());
}

fn bidi_span(id: u64, start: usize, end: usize) -> ExplicitBidiSpan {
    ExplicitBidiSpan {
        span_id: id,
        byte_range: TextRange::new(start, end),
        control: BidiControlKind::Isolate(None),
    }
}

#[test]
fn bidi_spans_order_ids_boundaries_and_depth() {
    let mut a = ast("abcd");
    a.bidi_spans = vec![bidi_span(1, 0, 4), bidi_span(2, 1, 3)];
    assert!(check(&a, "abcd").is_ok());
    for spans in [
        vec![bidi_span(1, 0, 3), bidi_span(2, 1, 4)],
        vec![bidi_span(1, 0, 2), bidi_span(1, 2, 4)],
        vec![bidi_span(1, 0, 4), bidi_span(2, 0, 4)],
    ] {
        a.bidi_spans = spans;
        assert_eq!(
            check(&a, "abcd"),
            Err(LayoutInputError::InvalidBidiSpan { span: 1 })
        );
    }
    let text = "a".repeat(128);
    let mut a = ast(&text);
    a.bidi_spans = (0..63).map(|i| bidi_span(i as u64, i, 128 - i)).collect();
    assert!(check(&a, &text).is_ok());
    a.bidi_spans.push(bidi_span(63, 63, 65));
    assert_eq!(
        check(&a, &text),
        Err(LayoutInputError::InvalidBidiSpan { span: 63 })
    );
    let mut a = ast("ab");
    edit_text_box(&mut a, 0, |atoms, clusters| {
        atoms.truncate(1);
        atoms[0].byte_range = TextRange::new(atoms[0].byte_range.start(), 2);
        clusters.truncate(1);
        clusters[0].natural_width = 2.0;
    });
    a.bidi_spans = vec![bidi_span(1, 1, 2)];
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidBidiSpan { span: 0 })
    );
}

#[test]
fn an_isolate_may_cover_only_a_hard_break() {
    let text = "a\u{2028}b";
    let mut a = ast(text);
    rebuild_items(&mut a, |b| {
        vec![
            boxed(b, 0, 1),
            WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(1, 4), false)),
            WordNode::Box(BoxNode::InlineObject {
                bidi_level: BidiLevel::LTR,
                baseline_align: BaselineAlignment::Auto,
                byte_range: TextRange::new(4, 5),
                box_id: 1,
                width: 1.0,
                ascent: 1.0,
                descent: 1.0,
                overflow_policy: BoxOverflowPolicy::Clip,
            }),
            end(5),
        ]
    });
    a.bidi_spans = vec![bidi_span(1, 1, 4)];
    assert!(check(&a, text).is_ok());
}
