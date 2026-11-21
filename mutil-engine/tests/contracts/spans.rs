//! Decoration, atomic and explicit bidi span ordering and nesting.

use crate::common::*;
use mutil_engine::*;
use proptest::prelude::*;
use std::collections::HashSet;

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
        clusters[0].byte_range = TextRange::new(clusters[0].byte_range.start(), 2);
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
            WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(1, 4))),
            WordNode::Box(BoxNode::InlineObject(InlineObjectBox {
                bidi_level: BidiLevel::LTR,
                baseline_align: BaselineAlignment::Auto,
                byte_range: TextRange::new(4, 5),
                box_id: 1,
                width: 1.0,
                ascent: 1.0,
                descent: 1.0,
                overflow_policy: BoxOverflowPolicy::Clip,
            })),
            end(5),
        ]
    });
    a.bidi_spans = vec![bidi_span(1, 1, 4)];
    assert!(check(&a, text).is_ok());
}

// --- AST-9 atomic spans over unsorted / overlapping concrete content -------

fn check_both(a: &ParagraphAst, t: &str) -> Result<(), LayoutInputError> {
    let via_check = check(a, t);
    let via_try =
        ValidatedInputs::try_from_ast(a, t, &constraints(), &LayoutOptions::default()).map(|_| ());
    assert_eq!(
        via_check, via_try,
        "check_layout_inputs and ValidatedInputs::try_from_ast must agree"
    );
    via_check
}

fn atomic_span(id: u64, start: usize, end: usize) -> AtomicEditSpan {
    AtomicEditSpan {
        span_id: id,
        byte_range: TextRange::new(start, end),
    }
}

fn inline_object(box_id: u64, start: usize, end: usize) -> WordNode {
    WordNode::Box(BoxNode::InlineObject(InlineObjectBox {
        bidi_level: BidiLevel::LTR,
        baseline_align: BaselineAlignment::Auto,
        byte_range: TextRange::new(start, end),
        box_id,
        width: 1.0,
        ascent: 1.0,
        descent: 1.0,
        overflow_policy: BoxOverflowPolicy::Clip,
    }))
}

fn word_space(start: usize, end: usize) -> WordNode {
    WordNode::Glue(GlueNode {
        original_bidi_class: BidiClass::Ws,
        bidi_level: BidiLevel::LTR,
        is_collapsible: true,
        byte_range: TextRange::new(start, end),
        kind: GlueKind::WordSpace,
        natural_width: 4.0,
        max_stretch: 2.0,
        max_shrink: 1.0,
    })
}

/// Builds a `TextBox` in run 0 with atoms having the exact `atom_ranges` given,
/// without requiring `atom_ranges` to be sorted or disjoint. `box_byte_range`
/// defaults to `[min_start, max_end]` of `atom_ranges`.
fn custom_text_box(
    ast: &mut ParagraphAst,
    box_byte_range: Option<(usize, usize)>,
    atom_ranges: &[(usize, usize)],
) -> WordNode {
    assert!(!atom_ranges.is_empty());
    let (min_s, max_e) = box_byte_range.unwrap_or_else(|| {
        let min_s = atom_ranges.iter().map(|&(s, _)| s).min().unwrap();
        let max_e = atom_ranges.iter().map(|&(_, e)| e).max().unwrap();
        (min_s, max_e)
    });
    // Push placeholder ordered atoms so `push_text` never panics on reversed ends,
    // then install the exact requested atom and cluster byte ranges.
    let placeholders: Vec<_> = (0..atom_ranges.len()).map(atom).collect();
    let clusters: Vec<_> = (0..atom_ranges.len())
        .map(|i| ShapedCluster {
            orientation: GlyphOrientation::Horizontal,
            byte_range: TextRange::new(i, i + 1),
            atom_range: i..i + 1,
            font: FontRef::unscaled(FontId(1)),
            bidi_level: BidiLevel::LTR,
            flags: AtomFlags::EMPTY,
            glyph_range: 0..1,
            natural_width: 1.0,
            safe_to_break_before: true,
            safe_to_concat_before: true,
        })
        .collect();
    let mut node = push_text(ast, 0, placeholders, clusters);
    if let WordNode::Box(BoxNode::Text(tb)) = &mut node {
        tb.byte_range = TextRange::new(min_s, max_e);
        for (idx, &(s, e)) in atom_ranges.iter().enumerate() {
            let r = TextRange::new(s, e);
            ast.atoms[tb.atom_range.start + idx].byte_range = r;
            ast.clusters[tb.cluster_range.start + idx].byte_range = r;
        }
    }
    node
}

/// Independently verifies every AST-9 invariant on `ast.atomic_spans`.
fn ast9_violation(ast: &ParagraphAst, text: &str) -> Option<String> {
    let mut entities: Vec<TextRange> = Vec::new();
    let mut non_entities: Vec<TextRange> = Vec::new();
    for item in &ast.items {
        match item {
            WordNode::Box(BoxNode::Text(tb)) => {
                let Some(atoms) = ast.atoms.get(tb.atom_range.clone()) else {
                    return Some("text box atom_range out of bounds".into());
                };
                entities.extend(atoms.iter().map(|a| a.byte_range));
            }
            WordNode::Box(BoxNode::InlineObject(obj)) => {
                entities.push(obj.byte_range);
            }
            WordNode::Glue(g) if !g.byte_range.is_empty() => {
                non_entities.push(g.byte_range);
            }
            WordNode::Tab { byte_range } => {
                non_entities.push(*byte_range);
            }
            WordNode::Discretionary(d) if !d.byte_range.is_empty() => {
                non_entities.push(d.byte_range);
            }
            _ => {}
        }
    }

    let mut entity_edges = HashSet::new();
    for r in &entities {
        entity_edges.insert(r.start());
        entity_edges.insert(r.end());
    }

    let mut seen_ids = HashSet::new();
    let mut prev: Option<TextRange> = None;

    for (idx, sp) in ast.atomic_spans.iter().enumerate() {
        let r = sp.byte_range;
        if r.is_empty() {
            return Some(format!("span {idx} is empty: {r:?}"));
        }
        if r.end() > text.len()
            || !text.is_char_boundary(r.start())
            || !text.is_char_boundary(r.end())
        {
            return Some(format!("span {idx} does not fit UTF-8 text: {r:?}"));
        }
        if !seen_ids.insert(sp.span_id) {
            return Some(format!("span {idx} repeats span_id {}", sp.span_id));
        }
        if let Some(p) = prev {
            if r.start() <= p.start() {
                return Some(format!(
                    "span {idx} start {} does not ascend past {}",
                    r.start(),
                    p.start()
                ));
            }
            if r.start() < p.end() {
                return Some(format!("span {idx} ({r:?}) overlaps previous ({p:?})"));
            }
        }
        prev = Some(r);

        if !entity_edges.contains(&r.start()) {
            return Some(format!(
                "span {idx} start {} is not an atom/object edge",
                r.start()
            ));
        }
        if !entity_edges.contains(&r.end()) {
            return Some(format!(
                "span {idx} end {} is not an atom/object edge",
                r.end()
            ));
        }
        for &ne in &non_entities {
            if (ne.start() < r.start() && r.start() < ne.end())
                || (ne.start() < r.end() && r.end() < ne.end())
            {
                return Some(format!(
                    "span {idx} ({r:?}) has edge inside non-entity {ne:?}"
                ));
            }
        }
        for &ent in &entities {
            if (ent.start() < r.start() && r.start() < ent.end())
                || (ent.start() < r.end() && r.end() < ent.end())
            {
                return Some(format!("span {idx} ({r:?}) straddles entity {ent:?}"));
            }
        }
        if !entities
            .iter()
            .any(|&ent| !ent.is_empty() && r.contains_range(ent))
        {
            return Some(format!("span {idx} ({r:?}) holds no atom or inline object"));
        }
    }
    None
}

fn content_order_violation(ast: &ParagraphAst) -> Option<String> {
    let mut prev_item_end = 0usize;
    for (idx, item) in ast.items.iter().enumerate() {
        let span = item.byte_span();
        if span.start() < prev_item_end {
            return Some(format!(
                "item {idx} ({span:?}) starts before previous end {prev_item_end}"
            ));
        }
        prev_item_end = span.end();
        if let WordNode::Box(BoxNode::Text(tb)) = item {
            let Some(atoms) = ast.atoms.get(tb.atom_range.clone()) else {
                return Some(format!("item {idx} atom_range out of bounds"));
            };
            let mut expected = tb.byte_range.start();
            for (a_idx, a) in atoms.iter().enumerate() {
                if a.byte_range.is_empty() || a.byte_range.start() != expected {
                    return Some(format!(
                        "item {idx} atom {a_idx} ({:?}) is not contiguous from {expected}",
                        a.byte_range
                    ));
                }
                expected = a.byte_range.end();
            }
            if expected != tb.byte_range.end() {
                return Some(format!(
                    "item {idx} atoms end at {expected}, box ends at {}",
                    tb.byte_range.end()
                ));
            }
        }
    }
    None
}

#[test]
fn atomic_span_content_independent_faults_precede_unsorted_items() {
    // Variant A: two text boxes in one run built in reversed byte order so run
    // storage tiles consecutively in item order (AST-4 passes) while byte ranges
    // are out of order (AST-3 `ItemsOutOfOrder`).
    let text_a = "aabb";
    let mut sorted_a = ast(text_a);
    rebuild_items(&mut sorted_a, |a| {
        vec![boxed(a, 0, 2), boxed(a, 2, 4), end(4)]
    });
    sorted_a.atomic_spans = vec![atomic_span(1, 0, 2), atomic_span(2, 2, 4)];
    assert_eq!(check_both(&sorted_a, text_a), Ok(()));

    let mut reversed_storage_a = ast(text_a);
    rebuild_items(&mut reversed_storage_a, |a| {
        vec![boxed(a, 2, 4), boxed(a, 0, 2), end(4)]
    });
    assert!(matches!(
        check_both(&reversed_storage_a, text_a),
        Err(LayoutInputError::ItemsOutOfOrder { .. })
    ));

    // Variant B: two separate shaping runs (0..2 and 2..4) with their text boxes
    // swapped in `items` — each run's storage is still tiled consecutively, so
    // the sole non-span fault is `ItemsOutOfOrder`.
    let mut two_run_swapped = ast(text_a);
    let mut run0 = two_run_swapped.runs[0].clone();
    run0.byte_range = TextRange::new(0, 2);
    let mut run1 = two_run_swapped.runs[0].clone();
    run1.byte_range = TextRange::new(2, 4);
    two_run_swapped.runs = vec![run0, run1];
    rebuild_items(&mut two_run_swapped, |a| {
        let box0 = push_text(
            a,
            0,
            vec![atom(0), atom(1)],
            vec![cluster(0, 0), cluster(0, 1)],
        );
        let box1 = push_text(
            a,
            1,
            vec![atom(2), atom(3)],
            vec![cluster(2, 0), cluster(2, 1)],
        );
        vec![box1, box0, end(4)]
    });
    assert!(matches!(
        check_both(&two_run_swapped, text_a),
        Err(LayoutInputError::ItemsOutOfOrder { .. })
    ));

    // Variant C: mixed text boxes and inline objects with items swapped out of order.
    let text_b = "aaXXbbYY";
    let mut sorted_b = ast(text_b);
    rebuild_items(&mut sorted_b, |a| {
        vec![
            boxed(a, 0, 2),
            inline_object(1, 2, 4),
            boxed(a, 4, 6),
            inline_object(2, 6, 8),
            end(8),
        ]
    });
    sorted_b.atomic_spans = vec![atomic_span(1, 0, 4), atomic_span(2, 4, 8)];
    assert_eq!(check_both(&sorted_b, text_b), Ok(()));

    let mut swapped_objects_b = sorted_b.clone();
    swapped_objects_b.atomic_spans.clear();
    swapped_objects_b.items.swap(1, 3);
    assert!(matches!(
        check_both(&swapped_objects_b, text_b),
        Err(LayoutInputError::ItemsOutOfOrder { .. })
    ));

    let mut swapped_mixed_b = sorted_b.clone();
    swapped_mixed_b.atomic_spans.clear();
    swapped_mixed_b.items.swap(0, 1);
    assert!(matches!(
        check_both(&swapped_mixed_b, text_b),
        Err(LayoutInputError::ItemsOutOfOrder { .. })
    ));

    let faults: &[(&str, Vec<AtomicEditSpan>, usize)] = &[
        ("empty", vec![atomic_span(1, 2, 2)], 0),
        ("outside_text", vec![atomic_span(1, 0, 99)], 0),
        (
            "decreasing_start",
            vec![atomic_span(1, 2, 4), atomic_span(2, 0, 2)],
            1,
        ),
        (
            "overlapping",
            vec![atomic_span(1, 0, 2), atomic_span(2, 1, 4)],
            1,
        ),
        (
            "nested",
            vec![atomic_span(1, 0, 4), atomic_span(2, 1, 2)],
            1,
        ),
        (
            "duplicate_id",
            vec![atomic_span(7, 0, 2), atomic_span(7, 2, 4)],
            1,
        ),
    ];

    for (label, base, text) in [
        ("reversed_storage_text_boxes", &reversed_storage_a, text_a),
        ("two_run_swapped_text_boxes", &two_run_swapped, text_a),
        ("swapped_inline_objects", &swapped_objects_b, text_b),
        ("swapped_text_and_object", &swapped_mixed_b, text_b),
    ] {
        for (fault_name, spans, expected_span) in faults {
            let mut case = base.clone();
            case.atomic_spans = spans.clone();
            assert_eq!(
                check_both(&case, text),
                Err(LayoutInputError::InvalidAtomicSpan {
                    span: *expected_span
                }),
                "{label} + {fault_name}: AST-9 must precede AST-3 ItemsOutOfOrder (AST-11)"
            );
        }
    }

    // UTF-8 split inside a multi-byte character combined with swapped items.
    let utf8_text = "éaXX";
    let mut utf8_ast = ast(utf8_text);
    rebuild_items(&mut utf8_ast, |a| {
        vec![
            custom_text_box(a, Some((0, 3)), &[(0, 2), (2, 3)]),
            inline_object(1, 3, 5),
            end(5),
        ]
    });
    assert_eq!(check_both(&utf8_ast, utf8_text), Ok(()));
    utf8_ast.items.swap(0, 1);
    utf8_ast.atomic_spans = vec![atomic_span(1, 1, 3)];
    assert_eq!(
        check_both(&utf8_ast, utf8_text),
        Err(LayoutInputError::InvalidAtomicSpan { span: 0 })
    );
}

#[test]
fn atomic_span_edge_faults_with_permuted_items_never_bypass_g1() {
    // 8 bytes: 2-byte atom `0..2`, 2-byte space glue `2..4`,
    // 2-byte inline object `4..6`, 2-byte atom `6..8`.
    let text = "aa  XXbb";
    let mut base = ast(text);
    rebuild_items(&mut base, |a| {
        vec![
            custom_text_box(a, Some((0, 2)), &[(0, 2)]),
            word_space(2, 4),
            inline_object(1, 4, 6),
            custom_text_box(a, Some((6, 8)), &[(6, 8)]),
            end(8),
        ]
    });
    base.atomic_spans = vec![atomic_span(1, 0, 2), atomic_span(2, 4, 8)];
    assert_eq!(check_both(&base, text), Ok(()));
    assert_eq!(ast9_violation(&base, text), None);

    // Every one of these spans is invalid by AST-9's edge-alignment / non-empty-content rules.
    let edge_faults: &[(&str, Vec<AtomicEditSpan>, usize)] = &[
        ("end_inside_first_atom", vec![atomic_span(1, 0, 1)], 0),
        ("start_inside_first_atom", vec![atomic_span(1, 1, 2)], 0),
        ("end_inside_whitespace_glue", vec![atomic_span(1, 0, 3)], 0),
        (
            "start_inside_whitespace_glue",
            vec![atomic_span(1, 3, 6)],
            0,
        ),
        ("glue_only_no_entity", vec![atomic_span(1, 2, 4)], 0),
        ("end_inside_inline_object", vec![atomic_span(1, 4, 5)], 0),
        ("start_inside_inline_object", vec![atomic_span(1, 5, 8)], 0),
        ("end_inside_second_atom", vec![atomic_span(1, 6, 7)], 0),
        ("start_inside_second_atom", vec![atomic_span(1, 7, 8)], 0),
        (
            "valid_then_invalid_edge",
            vec![atomic_span(1, 0, 2), atomic_span(2, 5, 8)],
            1,
        ),
        (
            "invalid_edge_then_valid",
            vec![atomic_span(1, 0, 1), atomic_span(2, 4, 6)],
            0,
        ),
    ];

    // Confirm every fault fails with InvalidAtomicSpan on the sorted baseline.
    for (name, spans, expected_span) in edge_faults {
        let mut sorted = base.clone();
        sorted.atomic_spans = spans.clone();
        assert_eq!(
            check_both(&sorted, text),
            Err(LayoutInputError::InvalidAtomicSpan {
                span: *expected_span
            }),
            "sorted baseline must reject {name}"
        );
    }

    // Build several out-of-order item permutations, including one where text
    // boxes are pushed in reversed byte order so run storage stays consecutive.
    let mut permutations: Vec<(&str, ParagraphAst)> = Vec::new();
    for (label, order) in [
        ("swap_text_boxes", [3, 1, 2, 0]),
        ("swap_box_and_object", [2, 1, 0, 3]),
        ("reverse_all_items", [3, 2, 1, 0]),
        ("glue_first", [1, 0, 2, 3]),
        ("rotate_left", [1, 2, 3, 0]),
    ] {
        let mut p = base.clone();
        p.items = vec![
            base.items[order[0]].clone(),
            base.items[order[1]].clone(),
            base.items[order[2]].clone(),
            base.items[order[3]].clone(),
            end(8),
        ];
        permutations.push((label, p));
    }

    let mut reversed_storage = ast(text);
    rebuild_items(&mut reversed_storage, |a| {
        vec![
            custom_text_box(a, Some((6, 8)), &[(6, 8)]),
            inline_object(1, 4, 6),
            word_space(2, 4),
            custom_text_box(a, Some((0, 2)), &[(0, 2)]),
            end(8),
        ]
    });
    permutations.push(("reversed_consecutive_storage", reversed_storage));

    for (perm_name, perm_ast) in &permutations {
        // Even with valid atomic spans, unsorted items must never pass G1.
        let mut valid_spans = perm_ast.clone();
        valid_spans.atomic_spans = vec![atomic_span(1, 0, 2), atomic_span(2, 4, 8)];
        assert!(
            check_both(&valid_spans, text).is_err(),
            "{perm_name} + valid_spans must be rejected"
        );

        // With every edge-alignment / glue-only invalid atomic span, G1 must reject.
        for (fault_name, spans, _) in edge_faults {
            let mut case = perm_ast.clone();
            case.atomic_spans = spans.clone();
            assert!(
                check_both(&case, text).is_err(),
                "{perm_name} + {fault_name} must never bypass G1"
            );
        }
    }
}

#[test]
fn atomic_span_faults_with_unsorted_atoms_in_single_text_box_never_bypass_g1() {
    let text = "aabbcc";
    let mut sorted = ast(text);
    rebuild_items(&mut sorted, |a| {
        vec![
            custom_text_box(a, Some((0, 6)), &[(0, 2), (2, 4), (4, 6)]),
            end(6),
        ]
    });
    sorted.atomic_spans = vec![atomic_span(1, 0, 2), atomic_span(2, 4, 6)];
    assert_eq!(check_both(&sorted, text), Ok(()));

    let edge_faults: &[(&str, Vec<AtomicEditSpan>, usize)] = &[
        ("end_inside_atom_0", vec![atomic_span(1, 0, 1)], 0),
        ("start_inside_atom_0", vec![atomic_span(1, 1, 4)], 0),
        ("both_edges_inside_atoms", vec![atomic_span(1, 1, 5)], 0),
        ("end_inside_atom_2", vec![atomic_span(1, 2, 5)], 0),
        ("start_inside_atom_2", vec![atomic_span(1, 5, 6)], 0),
        (
            "valid_then_invalid_edge",
            vec![atomic_span(1, 0, 2), atomic_span(2, 3, 6)],
            1,
        ),
    ];

    for (name, spans, expected_span) in edge_faults {
        let mut s = sorted.clone();
        s.atomic_spans = spans.clone();
        assert_eq!(
            check_both(&s, text),
            Err(LayoutInputError::InvalidAtomicSpan {
                span: *expected_span
            }),
            "sorted single-box baseline must reject {name}"
        );
    }

    let content_independent_faults: &[(&str, Vec<AtomicEditSpan>, usize)] = &[
        ("empty", vec![atomic_span(1, 2, 2)], 0),
        ("outside_text", vec![atomic_span(1, 0, 7)], 0),
        (
            "out_of_order",
            vec![atomic_span(1, 4, 6), atomic_span(2, 0, 2)],
            1,
        ),
        (
            "overlapping",
            vec![atomic_span(1, 0, 4), atomic_span(2, 2, 6)],
            1,
        ),
        (
            "duplicate_id",
            vec![atomic_span(9, 0, 2), atomic_span(9, 4, 6)],
            1,
        ),
    ];

    let unsorted_atom_orders: &[(&str, &[(usize, usize)])] = &[
        ("reversed_atoms", &[(4, 6), (2, 4), (0, 2)]),
        ("swapped_first_two", &[(2, 4), (0, 2), (4, 6)]),
        ("rotated_atoms", &[(4, 6), (0, 2), (2, 4)]),
    ];

    for &(order_name, ranges) in unsorted_atom_orders {
        let mut unsorted = ast(text);
        rebuild_items(&mut unsorted, |a| {
            vec![custom_text_box(a, Some((0, 6)), ranges), end(6)]
        });

        // Valid atomic spans still fail because the box's atoms are unsorted.
        let mut with_valid = unsorted.clone();
        with_valid.atomic_spans = vec![atomic_span(1, 0, 2), atomic_span(2, 4, 6)];
        assert!(
            check_both(&with_valid, text).is_err(),
            "{order_name} + valid spans must be rejected"
        );

        // Content-independent faults still precede per-item checks (AST-11).
        for (fault_name, spans, expected_span) in content_independent_faults {
            let mut case = unsorted.clone();
            case.atomic_spans = spans.clone();
            assert_eq!(
                check_both(&case, text),
                Err(LayoutInputError::InvalidAtomicSpan {
                    span: *expected_span
                }),
                "{order_name} + {fault_name}: AST-9 must precede per-item validation"
            );
        }

        // Edge-alignment faults must never bypass G1.
        for (fault_name, spans, _) in edge_faults {
            let mut case = unsorted.clone();
            case.atomic_spans = spans.clone();
            assert!(
                check_both(&case, text).is_err(),
                "{order_name} + {fault_name} must never bypass G1"
            );
        }
    }
}

#[test]
fn atomic_span_faults_with_overlapping_content_ranges_never_bypass_g1() {
    let text = "abcdef";

    // Overlapping text boxes: 0..4 and 2..6.
    let mut overlap_boxes = ast(text);
    rebuild_items(&mut overlap_boxes, |a| {
        vec![
            custom_text_box(a, Some((0, 4)), &[(0, 2), (2, 4)]),
            custom_text_box(a, Some((2, 6)), &[(2, 4), (4, 6)]),
            end(6),
        ]
    });

    // Overlapping inline objects: 0..4 and 2..6.
    let mut overlap_objects = ast(text);
    rebuild_items(&mut overlap_objects, |_| {
        vec![inline_object(1, 0, 4), inline_object(2, 2, 6), end(6)]
    });
    overlap_objects.runs.clear();

    // Overlapping atoms inside a single text box: 0..4 and 2..6.
    let mut overlap_atoms = ast(text);
    rebuild_items(&mut overlap_atoms, |a| {
        vec![custom_text_box(a, Some((0, 6)), &[(0, 4), (2, 6)]), end(6)]
    });

    for (label, base) in [
        ("overlapping_text_boxes", &overlap_boxes),
        ("overlapping_inline_objects", &overlap_objects),
        ("overlapping_atoms_in_box", &overlap_atoms),
    ] {
        // Content-independent atomic span fault reports InvalidAtomicSpan before item checks.
        let mut ci_fault = base.clone();
        ci_fault.atomic_spans = vec![atomic_span(1, 0, 4), atomic_span(1, 4, 6)];
        assert_eq!(
            check_both(&ci_fault, text),
            Err(LayoutInputError::InvalidAtomicSpan { span: 1 }),
            "{label} + duplicate_id"
        );

        // Edge-alignment faults and valid-looking spans must all be rejected.
        for spans in [
            vec![atomic_span(1, 1, 3)],
            vec![atomic_span(1, 0, 5)],
            vec![atomic_span(1, 0, 2), atomic_span(2, 3, 6)],
            vec![atomic_span(1, 0, 6)],
        ] {
            let mut case = base.clone();
            case.atomic_spans = spans;
            assert!(
                check_both(&case, text).is_err(),
                "{label} must never bypass G1"
            );
        }
    }
}

#[test]
fn atomic_span_tab_hardbreak_and_outer_glue_faults_with_permuted_items() {
    // 10 bytes: leading space `0..1`, atom `1..2`, tab `2..3`, atom `3..4`,
    // hard break `4..7` (U+2028), inline object `7..9`, trailing space `9..10`.
    let text = " a\tb\u{2028}XX ";
    let mut base = ast(text);
    rebuild_items(&mut base, |a| {
        vec![
            word_space(0, 1),
            boxed(a, 1, 2),
            WordNode::Tab {
                byte_range: TextRange::new(2, 3),
            },
            boxed(a, 3, 4),
            WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(4, 7))),
            inline_object(1, 7, 9),
            word_space(9, 10),
            end(10),
        ]
    });
    base.atomic_spans = vec![atomic_span(1, 1, 2), atomic_span(2, 7, 9)];
    assert_eq!(check_both(&base, text), Ok(()));

    let faults: &[(&str, Vec<AtomicEditSpan>)] = &[
        ("starts_at_leading_glue", vec![atomic_span(1, 0, 2)]),
        ("ends_at_trailing_glue", vec![atomic_span(1, 7, 10)]),
        ("tab_only", vec![atomic_span(1, 2, 3)]),
        ("hard_break_only", vec![atomic_span(1, 4, 7)]),
        ("edge_inside_hard_break", vec![atomic_span(1, 3, 5)]),
        ("edge_inside_inline_object", vec![atomic_span(1, 7, 8)]),
    ];

    for (name, spans) in faults {
        let mut sorted = base.clone();
        sorted.atomic_spans = spans.clone();
        assert_eq!(
            check_both(&sorted, text),
            Err(LayoutInputError::InvalidAtomicSpan { span: 0 }),
            "sorted baseline must reject {name}"
        );
    }

    // Permute items out of byte order while preserving text-box run storage order
    // (swapping only glue, tab and hard break around the text boxes).
    let mut permuted = base.clone();
    permuted.items.swap(0, 6);
    permuted.items.swap(2, 4);
    assert!(
        matches!(
            check_both(&permuted, text),
            Err(LayoutInputError::ItemsOutOfOrder { .. })
        ),
        "permuted fixture with valid atomic spans must reach ItemsOutOfOrder"
    );

    for (name, spans) in faults {
        let mut case = permuted.clone();
        case.atomic_spans = spans.clone();
        assert!(
            check_both(&case, text).is_err(),
            "permuted items + {name} must never bypass G1"
        );
    }
}

#[test]
fn atomic_span_edge_fault_precedes_unsorted_inline_objects() {
    let text = "XXXX";
    let mut a = ast(text);
    rebuild_items(&mut a, |_| {
        vec![inline_object(1, 2, 4), inline_object(2, 0, 2), end(4)]
    });
    a.runs.clear();

    // Without atomic spans, or with a valid atomic span `0..2`, AST-3 reports
    // the out-of-order second inline object.
    assert_eq!(
        check_both(&a, text),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );
    a.atomic_spans = vec![atomic_span(1, 0, 2)];
    assert_eq!(
        check_both(&a, text),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );

    // With an atomic span `0..1` whose end splits the inline object `0..2`,
    // AST-11 checks AST-9 before AST-3 item order.
    a.atomic_spans = vec![atomic_span(1, 0, 1)];
    assert_eq!(
        check_both(&a, text),
        Err(LayoutInputError::InvalidAtomicSpan { span: 0 })
    );
}

#[derive(Clone, Debug)]
enum SegmentSpec {
    Text { atom_lens: Vec<usize> },
    InlineObject { len: usize },
    WordSpace { len: usize },
    Tab,
    HardBreak,
}

fn segment_spec_strategy() -> impl Strategy<Value = SegmentSpec> {
    prop_oneof![
        4 => prop::collection::vec(1usize..=2, 1..=3)
            .prop_map(|atom_lens| SegmentSpec::Text { atom_lens }),
        2 => (1usize..=2).prop_map(|len| SegmentSpec::InlineObject { len }),
        2 => (1usize..=2).prop_map(|len| SegmentSpec::WordSpace { len }),
        1 => Just(SegmentSpec::Tab),
        1 => Just(SegmentSpec::HardBreak),
    ]
}

#[derive(Clone, Debug)]
struct CandidateSpanSpec {
    id: u64,
    a: usize,
    b: usize,
}

#[derive(Clone, Debug)]
enum ContentMutation {
    None,
    /// Permute `ast.items[..len-1]` after building storage in canonical order.
    PermuteItems(Vec<usize>),
    /// Build text boxes in permuted order so run storage tiles in item order
    /// even though byte ranges are out of order.
    PermuteAndBuildInOrder(Vec<usize>),
    /// Permute atoms inside a text box (preserving `tb.byte_range` and storage tiling).
    PermuteAtoms {
        box_idx: usize,
        seed: usize,
    },
    /// Perturb one atom's byte range inside a text box (can create overlaps/gaps/unsorted atoms).
    PerturbAtom {
        box_idx: usize,
        atom_idx: usize,
        start: usize,
        end: usize,
    },
}

fn content_mutation_strategy() -> impl Strategy<Value = ContentMutation> {
    prop_oneof![
        3 => Just(ContentMutation::None),
        3 => prop::collection::vec(0usize..12, 1..=6).prop_map(ContentMutation::PermuteItems),
        3 => prop::collection::vec(0usize..12, 1..=6)
            .prop_map(ContentMutation::PermuteAndBuildInOrder),
        2 => (0usize..6, 0usize..24)
            .prop_map(|(box_idx, seed)| ContentMutation::PermuteAtoms { box_idx, seed }),
        2 => (0usize..6, 0usize..4, 0usize..18, 0usize..18).prop_map(
            |(box_idx, atom_idx, a, b)| {
                let (start, end) = if a <= b { (a, b) } else { (b, a) };
                ContentMutation::PerturbAtom {
                    box_idx,
                    atom_idx,
                    start,
                    end,
                }
            }
        ),
    ]
}

fn apply_swaps<T>(slice: &mut [T], swaps: &[usize]) {
    let n = slice.len();
    if n < 2 {
        return;
    }
    for (i, &target) in swaps.iter().enumerate() {
        slice.swap(i % n, target % n);
    }
}

fn build_fuzz_ast(
    segments: &[SegmentSpec],
    mutation: &ContentMutation,
    raw_spans: &[CandidateSpanSpec],
) -> (String, ParagraphAst) {
    let mut text = String::new();
    // Precompute each segment's canonical byte range and sub-atom byte ranges.
    #[derive(Clone)]
    enum Placed {
        Text(Vec<(usize, usize)>),
        InlineObject(u64, usize, usize),
        WordSpace(usize, usize),
        Tab(usize, usize),
        HardBreak(usize, usize),
    }

    let mut placed = Vec::with_capacity(segments.len());
    let mut next_box_id = 1u64;
    for seg in segments {
        let start = text.len();
        match seg {
            SegmentSpec::Text { atom_lens } => {
                let mut ranges = Vec::with_capacity(atom_lens.len());
                let mut cur = start;
                for &len in atom_lens {
                    for _ in 0..len {
                        text.push('a');
                    }
                    ranges.push((cur, cur + len));
                    cur += len;
                }
                placed.push(Placed::Text(ranges));
            }
            SegmentSpec::InlineObject { len } => {
                for _ in 0..*len {
                    text.push('X');
                }
                placed.push(Placed::InlineObject(next_box_id, start, text.len()));
                next_box_id += 1;
            }
            SegmentSpec::WordSpace { len } => {
                for _ in 0..*len {
                    text.push(' ');
                }
                placed.push(Placed::WordSpace(start, text.len()));
            }
            SegmentSpec::Tab => {
                text.push('\t');
                placed.push(Placed::Tab(start, text.len()));
            }
            SegmentSpec::HardBreak => {
                text.push('\u{2028}');
                placed.push(Placed::HardBreak(start, text.len()));
            }
        }
    }

    if let ContentMutation::PermuteAndBuildInOrder(swaps) = mutation {
        apply_swaps(&mut placed, swaps);
    }

    let has_text = placed.iter().any(|p| matches!(p, Placed::Text(_)));
    let mut a = ast(&text);
    rebuild_items(&mut a, |a| {
        let mut items = Vec::with_capacity(placed.len() + 1);
        for p in &placed {
            match p {
                Placed::Text(ranges) => {
                    items.push(custom_text_box(a, None, ranges));
                }
                Placed::InlineObject(id, s, e) => items.push(inline_object(*id, *s, *e)),
                Placed::WordSpace(s, e) => items.push(word_space(*s, *e)),
                Placed::Tab(s, e) => items.push(WordNode::Tab {
                    byte_range: TextRange::new(*s, *e),
                }),
                Placed::HardBreak(s, e) => items.push(WordNode::Discretionary(
                    DiscretionaryNode::hard_break(TextRange::new(*s, *e)),
                )),
            }
        }
        items.push(end(text.len()));
        items
    });
    if !has_text {
        a.runs.clear();
    }

    match mutation {
        ContentMutation::None | ContentMutation::PermuteAndBuildInOrder(_) => {}
        ContentMutation::PermuteItems(swaps) => {
            let content_len = a.items.len() - 1;
            apply_swaps(&mut a.items[..content_len], swaps);
        }
        ContentMutation::PermuteAtoms { box_idx, seed } => {
            let text_boxes: Vec<usize> = a
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, n)| matches!(n, WordNode::Box(BoxNode::Text(_))).then_some(i))
                .collect();
            if !text_boxes.is_empty() {
                let item_idx = text_boxes[box_idx % text_boxes.len()];
                if let WordNode::Box(BoxNode::Text(tb)) = &a.items[item_idx] {
                    let ar = tb.atom_range.clone();
                    let cr = tb.cluster_range.clone();
                    let len = ar.len();
                    if len >= 2 {
                        let i = seed % len;
                        let j = (i + 1 + (seed / len) % (len - 1)) % len;
                        let bi = a.atoms[ar.start + i].byte_range;
                        let bj = a.atoms[ar.start + j].byte_range;
                        a.atoms[ar.start + i].byte_range = bj;
                        a.atoms[ar.start + j].byte_range = bi;
                        a.clusters[cr.start + i].byte_range = bj;
                        a.clusters[cr.start + j].byte_range = bi;
                    }
                }
            }
        }
        ContentMutation::PerturbAtom {
            box_idx,
            atom_idx,
            start,
            end,
        } => {
            let text_boxes: Vec<usize> = a
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, n)| matches!(n, WordNode::Box(BoxNode::Text(_))).then_some(i))
                .collect();
            if !text_boxes.is_empty() && !text.is_empty() {
                let item_idx = text_boxes[box_idx % text_boxes.len()];
                if let WordNode::Box(BoxNode::Text(tb)) = &a.items[item_idx] {
                    let ar = tb.atom_range.clone();
                    let cr = tb.cluster_range.clone();
                    let idx = atom_idx % ar.len();
                    let s = (*start).min(text.len());
                    let e = (*end).min(text.len());
                    let (s, e) = if s <= e { (s, e) } else { (e, s) };
                    let r = TextRange::new(s, e);
                    a.atoms[ar.start + idx].byte_range = r;
                    a.clusters[cr.start + idx].byte_range = r;
                }
            }
        }
    }

    let bound = text.len() + 2;
    a.atomic_spans = raw_spans
        .iter()
        .map(|spec| {
            let x = spec.a % (bound + 1);
            let y = spec.b % (bound + 1);
            let (s, e) = if x <= y { (x, y) } else { (y, x) };
            atomic_span(spec.id, s, e)
        })
        .collect();

    (text, a)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn no_invalid_atomic_span_bypasses_g1_under_permuted_or_perturbed_content(
        segments in prop::collection::vec(segment_spec_strategy(), 1..=6),
        mutation in content_mutation_strategy(),
        raw_spans in prop::collection::vec(
            (0u64..4, 0usize..24, 0usize..24)
                .prop_map(|(id, a, b)| CandidateSpanSpec { id, a, b }),
            0..=5,
        ),
    ) {
        let (text, ast) = build_fuzz_ast(&segments, &mutation, &raw_spans);
        let result = check_both(&ast, &text);
        if result.is_ok() {
            prop_assert_eq!(
                ast9_violation(&ast, &text),
                None,
                "G1 accepted AST violating AST-9: text={:?}, spans={:?}, items={:?}",
                text,
                ast.atomic_spans,
                ast.items,
            );
            prop_assert_eq!(
                content_order_violation(&ast),
                None,
                "G1 accepted AST with unsorted/non-contiguous content: text={:?}, items={:?}, atoms={:?}",
                text,
                ast.items,
                ast.atoms,
            );
        } else if matches!(mutation, ContentMutation::None) && ast9_violation(&ast, &text).is_some() {
            prop_assert!(
                matches!(result, Err(LayoutInputError::InvalidAtomicSpan { .. })),
                "unmutated valid AST with AST-9 violation must report InvalidAtomicSpan, got {:?}",
                result
            );
        }
    }

    #[test]
    fn no_well_ordered_invalid_atomic_span_bypasses_g1_under_unsorted_content(
        segments in prop::collection::vec(segment_spec_strategy(), 1..=6),
        mutation in content_mutation_strategy(),
        raw_cuts in prop::collection::vec(0usize..32, 2..=10),
        keep_flags in prop::collection::vec(any::<bool>(), 1..=8),
    ) {
        let (text, mut ast) = build_fuzz_ast(&segments, &mutation, &[]);

        // Build strictly ascending, pairwise-disjoint, non-empty, unique-id,
        // UTF-8-aligned spans so every case passes content-independent AST-9 checks
        // and directly stresses edge-alignment / entity-containment over unsorted content.
        let char_bounds: Vec<usize> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect();
        let mut cuts: Vec<usize> = raw_cuts
            .iter()
            .map(|&c| char_bounds[c % char_bounds.len()])
            .collect();
        cuts.sort_unstable();
        cuts.dedup();

        let mut spans = Vec::new();
        let mut id = 1u64;
        let mut cursor = 0usize;
        for (k, &keep) in keep_flags.iter().enumerate() {
            if cursor + 1 >= cuts.len() {
                break;
            }
            if keep {
                let start = cuts[cursor];
                let end = cuts[cursor + 1];
                spans.push(atomic_span(id, start, end));
                id += 1;
                // Advance by 1 on even steps (allows adjacent spans) or 2 on odd steps (allows gaps).
                cursor += if k % 2 == 0 { 1 } else { 2 };
            } else {
                cursor += 1;
            }
        }
        ast.atomic_spans = spans;

        let result = check_both(&ast, &text);
        if result.is_ok() {
            prop_assert_eq!(
                ast9_violation(&ast, &text),
                None,
                "G1 accepted AST violating AST-9: text={:?}, spans={:?}, items={:?}",
                text,
                ast.atomic_spans,
                ast.items,
            );
            prop_assert_eq!(
                content_order_violation(&ast),
                None,
                "G1 accepted AST with unsorted/non-contiguous content: text={:?}, items={:?}, atoms={:?}",
                text,
                ast.items,
                ast.atoms,
            );
        } else if matches!(mutation, ContentMutation::None) && ast9_violation(&ast, &text).is_some() {
            prop_assert!(
                matches!(result, Err(LayoutInputError::InvalidAtomicSpan { .. })),
                "unmutated valid AST with AST-9 violation must report InvalidAtomicSpan, got {:?}",
                result
            );
        }
    }
}
