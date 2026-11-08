//! Runtime probes for the proposed single-paragraph contract; expected red until implementation.
use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;
fn end(n: usize) -> WordNode {
    WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::empty_at(n), true))
}
fn boxed(s: usize, e: usize, atomic: bool) -> WordNode {
    let flags = if atomic {
        AtomFlags::IN_ATOMIC_EDIT_SPAN
    } else {
        AtomFlags::EMPTY
    };
    let atoms: Vec<_> = (s..e)
        .map(|i| GraphemeAtom {
            byte_range: TextRange::new(i, i + 1),
            script: ScriptTag::LATIN,
            bidi_level: 0,
            linebreak_class: LineBreakClass::Al,
            flags,
        })
        .collect();
    let clusters = atoms
        .iter()
        .enumerate()
        .map(|(i, a)| ShapedCluster {
            byte_range: a.byte_range,
            atom_range: i..i + 1,
            font: FontRef::unscaled(FontId(1)),
            bidi_level: 0,
            flags,
            glyphs: smallvec![RawShapedGlyph {
                glyph_id: 1,
                cluster: a.byte_range.start as u32,
                x_advance: 1.,
                ..Default::default()
            }],
            natural_width: 1.,
            uncompressed_width: 1.,
            safe_to_break_before: true,
            safe_to_concat_before: true,
        })
        .collect();
    WordNode::Box(BoxNode::Text {
        run_index: 0,
        byte_range: TextRange::new(s, e),
        atoms,
        clusters,
        natural_width: (e - s) as f32,
    })
}
fn ast(t: &str) -> ParagraphAst {
    ParagraphAst {
        base_direction: Direction::Ltr,
        strut_metrics: FontMetrics {
            ascent: 8.,
            descent: 2.,
            ..Default::default()
        },
        strut_font_size: 10.,
        runs: vec![ShapingRun {
            features: Default::default(),

            byte_range: TextRange::new(0, t.len()),
            families: Arc::from([]),
            font_size: 10.,
            style: Default::default(),
            script: ScriptTag::LATIN,
            locale: LocaleTag::und(),
            bidi_level: 0,
            decoration: Default::default(),
        }],
        items: vec![boxed(0, t.len(), false), end(t.len())],
        decoration_spans: vec![],
        atomic_spans: vec![],
    }
}
fn check(a: &ParagraphAst, t: &str) -> Result<LineContext, LayoutInputError> {
    check_layout_inputs(
        a,
        t,
        &LayoutConstraints {
            wrap_width: 100.,
            tab_stops: TabStops { step: 8. },
        },
        &LayoutOptions::default(),
    )
}
#[test]
fn valid_control() {
    assert!(check(&ast("abc"), "abc").is_ok());
}
#[test]
fn sentinel_cannot_replace_source() {
    let mut a = ast("abc");
    a.items = vec![end(3)];
    assert!(check(&a, "abc").is_err(), "all bytes unowned");
}
#[test]
fn source_cannot_have_gaps() {
    let mut a = ast("abc");
    a.items[0] = boxed(0, 2, false);
    assert!(check(&a, "abc").is_err(), "2..3 unowned");
}
#[test]
fn source_cannot_overlap() {
    let mut a = ast("abc");
    a.items = vec![boxed(0, 2, false), boxed(1, 3, false), end(3)];
    assert!(check(&a, "abc").is_err());
}
fn reject_level_mismatch(target: usize) {
    let mut a = ast("abc");
    assert!(check(&a, "abc").is_ok());
    if target == 0 {
        a.runs[0].bidi_level = 1;
    }
    if let WordNode::Box(BoxNode::Text {
        atoms, clusters, ..
    }) = &mut a.items[0]
    {
        if target == 1 {
            atoms[0].bidi_level = 1;
        }
        if target == 2 {
            clusters[0].bidi_level = 1;
        }
    }
    assert!(check(&a, "abc").is_err());
}
#[test]
fn reject_run_level_mismatch() {
    reject_level_mismatch(0);
}
#[test]
fn reject_atom_level_mismatch() {
    reject_level_mismatch(1);
}
#[test]
fn reject_cluster_level_mismatch() {
    reject_level_mismatch(2);
}
#[test]
fn safety_flags_must_be_consistent() {
    let mut a = ast("abc");
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].safe_to_break_before = false;
        assert!(!clusters[0].safety_flags_consistent());
    }
    assert!(check(&a, "abc").is_err());
}
fn font_scale_must_be_finite(v: f32) {
    let mut a = ast("abc");
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].font.scale = v;
    }
    assert!(check(&a, "abc").is_err());
}
fn uncompressed_width_must_be_finite(v: f32) {
    let mut a = ast("abc");
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].uncompressed_width = v;
    }
    assert!(check(&a, "abc").is_err());
}

fn reject_separator_in_source(sep: &str) {
    let t = format!("a{sep}b");
    let mut a = ast(&t);
    a.items[0] = WordNode::Box(BoxNode::InlineObject {
        byte_range: TextRange::new(0, t.len()),
        box_id: 1,
        width: 1.,
        ascent: 8.,
        descent: 2.,
        overflow_policy: BoxOverflowPolicy::Clip,
    });
    assert!(check(&a, &t).is_err(), "separator {sep:?}");
}
fn reject_hard_break(sep: &str) {
    let mut a = ast(sep);
    a.items = vec![
        WordNode::Discretionary(DiscretionaryNode::hard_break(
            TextRange::new(0, sep.len()),
            false,
        )),
        end(sep.len()),
    ];
    assert!(check(&a, sep).is_err(), "hard break {sep:?}");
}
#[test]
fn line_separator_remains_allowed() {
    let t = "a\u{2028}b";
    let mut a = ast(t);
    a.items = vec![
        boxed(0, 1, false),
        WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(1, 4), false)),
        boxed(4, 5, false),
        end(5),
    ];
    assert!(check(&a, t).is_ok());
}
#[test]
fn reject_negative_strut() {
    let mut a = ast("abc");
    a.strut_metrics.ascent = -8.;
    a.strut_metrics.descent = -2.;
    assert!(check(&a, "abc").is_err());
}
fn line() -> PositionedLine {
    PositionedLine {
        atomic_fragments: vec![],

        byte_range: TextRange::new(0, 1),
        visible_byte_end: 1,
        baseline_y: 8.,
        ascent: 8.,
        descent: 2.,
        line_height: 10.,
        visual_width: 100.,
        hanging_left: 0.,
        hanging_right: 0.,
        is_justified: false,
        visual_runs: vec![],
        visual_boxes: vec![],
        tabs: vec![],
        span_fragments: vec![],
        caret_stops: vec![],
        decorations: vec![],
    }
}
fn ctx() -> LineContext {
    LineContext {
        strut: VerticalMetrics {
            ascent: 8.,
            descent: 2.,
            line_gap: 0.,
        },
        wrap_width: 100.,
        ..Default::default()
    }
}
fn glyph(x: f32, advance: f32, hanging: bool) -> PositionedGlyph {
    PositionedGlyph {
        glyph_id: 1,
        x,
        x_offset: 0.,
        y_offset: 0.,
        advance,
        byte_range: Some(TextRange::new(0, 1)),
        is_synthesized: false,
        is_hanging: hanging,
    }
}
fn run(g: PositionedGlyph) -> VisualGlyphRun {
    VisualGlyphRun {
        font_size: 16.0,
        variations: Default::default(),

        font: FontRef::unscaled(FontId(1)),
        bidi_level: 0,
        byte_range: g.byte_range.unwrap(),
        glyphs: vec![g],
    }
}
#[test]
fn reject_negative_line_height() {
    let mut l = line();
    l.ascent = -8.;
    l.descent = -2.;
    l.line_height = -10.;
    let mut c = ctx();
    c.strut.ascent = -8.;
    c.strut.descent = -2.;
    assert!(l.validate(&c).is_err());
}
#[test]
fn tab_stop_advances_at_large_coordinates() {
    let x = 1e10_f32;
    let next = TabStops { step: 8. }.next_stop(x);
    assert!(
        next.is_none_or(|next| next > x),
        "{x} -> {next:?}; unchanged input cannot signal termination"
    );
    assert!(next.is_none_or(f32::is_finite));
}
#[test]
fn partial_hanging_counts_only_overflow() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(96., 10., true))];
    l.hanging_right = 6.;
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    assert_eq!(l.validate(&c), Ok(()));
    l.hanging_right = 10.;
    assert!(l.validate(&c).is_err());
}
#[test]
fn nbsp_remains_visible_in_collapsible_output() {
    // Output-contract control only: no layout algorithm exists to test actual folding yet.
    let mut l = line();
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 3;
    l.visual_width = 14.;
    let mut nbsp = glyph(10., 4., false);
    nbsp.byte_range = Some(TextRange::new(1, 3));
    l.visual_runs = vec![run(glyph(0., 10., false)), run(nbsp)];
    assert_eq!(l.validate(&ctx()), Ok(()));
    assert_eq!(l.visual_runs[1].glyphs[0].advance, 4.);
}
#[test]
fn badness_has_no_primary_only_capacity_cliff() {
    let c = TierCapacities {
        stretch: [0., 10., 0.],
        shrink: [0., 0.],
    };
    let a = line_fit(10., &c, 16.).badness;
    let b = line_fit(10.001, &c, 16.).badness;
    assert!(b >= a && b - a < 0.1, "{a} -> {b}");
}
fn spanned(n: usize) -> (String, ParagraphAst) {
    let t = "a".repeat(n);
    let mut a = ast(&t);
    a.items.clear();
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
                bidi_level: 0,
            }));
            if edge == SpanEdge::Leading {
                a.items.push(boxed(i, i + 1, true));
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

macro_rules! nonfinite_cases {
    ($nan:ident, $pos:ident, $neg:ident, $probe:ident) => {
        #[test]
        fn $nan() {
            $probe(f32::NAN);
        }
        #[test]
        fn $pos() {
            $probe(f32::INFINITY);
        }
        #[test]
        fn $neg() {
            $probe(f32::NEG_INFINITY);
        }
    };
}
nonfinite_cases!(
    reject_nan_scale,
    reject_infinite_scale,
    reject_negative_infinite_scale,
    font_scale_must_be_finite
);
nonfinite_cases!(
    reject_nan_uncompressed_width,
    reject_infinite_uncompressed_width,
    reject_negative_infinite_uncompressed_width,
    uncompressed_width_must_be_finite
);

macro_rules! separator_cases {
    ($source:ident, $node:ident, $separator:expr) => {
        #[test]
        fn $source() {
            reject_separator_in_source($separator);
        }
        #[test]
        fn $node() {
            reject_hard_break($separator);
        }
    };
}
separator_cases!(reject_lf_source, reject_lf_node, "\n");
separator_cases!(reject_cr_source, reject_cr_node, "\r");
separator_cases!(reject_crlf_source, reject_crlf_node, "\r\n");
separator_cases!(reject_nel_source, reject_nel_node, "\u{85}");
separator_cases!(reject_ps_source, reject_ps_node, "\u{2029}");
separator_cases!(reject_vt_source, reject_vt_node, "\u{b}");
separator_cases!(reject_ff_source, reject_ff_node, "\u{c}");

#[test]
fn concat_unsafety_alone_is_valid() {
    let mut a = ast("abc");
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].safe_to_concat_before = false;
        assert!(clusters[0].safety_flags_consistent());
    }
    assert!(check(&a, "abc").is_ok());
}
fn capacity_endpoint(stretch: [f32; 3], expected: f32) {
    let cap = TierCapacities {
        stretch,
        shrink: [0., 0.],
    };
    let seam = cap.total_stretch();
    let at = line_fit(seam, &cap, 16.).badness;
    let after = line_fit(seam + 0.001, &cap, 16.).badness;
    assert!(
        (at - expected).abs() < 0.01,
        "endpoint {at}, expected {expected}"
    );
    assert!(after >= at && after - at < 0.1, "cliff {at} -> {after}");
}
#[test]
fn tier_zero_only_excess_starts_at_100() {
    capacity_endpoint([10., 0., 0.], 100.);
}
#[test]
fn tier_two_excess_starts_at_400() {
    capacity_endpoint([6., 4., 10.], 400.);
}
#[test]
fn zero_capacity_excess_starts_at_zero() {
    let cap = TierCapacities::default();
    let near = line_fit(0.002, &cap, 16.).badness;
    assert!(near > 0. && near < 0.01, "zero-capacity origin: {near}");
}

fn decorated(ranges: &[(usize, usize)]) -> ParagraphAst {
    let mut a = ast("abcd");
    a.items.clear();
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
    ordered.sort_by_key(|sp| (sp.byte_range.start, std::cmp::Reverse(sp.byte_range.end)));
    for offset in 0..=4 {
        for sp in ordered
            .iter()
            .rev()
            .filter(|sp| sp.byte_range.end == offset)
        {
            a.items.push(WordNode::Box(BoxNode::SpanBoundary {
                byte_range: TextRange::empty_at(offset),
                span_id: sp.span_id,
                edge: SpanEdge::Trailing,
                width: 0.,
                bidi_level: 0,
            }));
        }
        for sp in ordered.iter().filter(|sp| sp.byte_range.start == offset) {
            a.items.push(WordNode::Box(BoxNode::SpanBoundary {
                byte_range: TextRange::empty_at(offset),
                span_id: sp.span_id,
                edge: SpanEdge::Leading,
                width: 0.,
                bidi_level: 0,
            }));
        }
        if offset < 4 {
            a.items.push(boxed(offset, offset + 1, false));
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
    assert!(check(&decorated(&[(0, 2), (0, 4)]), "abcd").is_err());
}
#[test]
fn decreasing_span_starts_are_rejected() {
    assert!(check(&decorated(&[(2, 4), (0, 2)]), "abcd").is_err());
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
    assert!(check(&a, &t).is_err());
}
#[test]
fn zero_strut_font_size_is_rejected() {
    let mut a = ast("abc");
    a.strut_font_size = 0.;
    assert!(check(&a, "abc").is_err());
}
#[test]
fn zero_run_font_size_is_rejected() {
    let mut a = ast("abc");
    a.runs[0].font_size = 0.;
    assert!(check(&a, "abc").is_err());
}
#[test]
fn zero_fixed_multiplier_is_rejected() {
    let a = ast("abc");
    let options = LayoutOptions {
        line_height: LineHeight::Fixed(0.),
        ..Default::default()
    };
    assert!(
        check_layout_inputs(
            &a,
            "abc",
            &LayoutConstraints {
                wrap_width: 100.,
                tab_stops: TabStops { step: 8. }
            },
            &options
        )
        .is_err()
    );
}
#[test]
fn negative_half_leading_with_positive_total_is_allowed() {
    let a = ast("abc");
    let options = LayoutOptions {
        line_height: LineHeight::Fixed(0.5),
        ..Default::default()
    };
    let c = check_layout_inputs(
        &a,
        "abc",
        &LayoutConstraints {
            wrap_width: 100.,
            tab_stops: TabStops { step: 8. },
        },
        &options,
    )
    .unwrap();
    assert!(c.strut.descent < 0.);
    assert_eq!(c.strut.natural_line_height(), 5.);
}
#[test]
fn false_hanging_flag_inside_body_is_rejected() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(90., 5., true))];
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    assert!(l.validate(&c).is_err());
}
#[test]
fn negative_advance_partial_hang_uses_normalized_envelope() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(106., -10., true))];
    l.hanging_right = 6.;
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    assert_eq!(l.validate(&c), Ok(()));
}

#[test]
fn left_partial_hanging_counts_only_overflow() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(-6., 10., true))];
    l.hanging_left = 6.;
    let mut c = ctx();
    c.hang_mode.start = HangStartMode::AllLines;
    assert_eq!(l.validate(&c), Ok(()));
}

#[test]
fn zero_width_node_cannot_be_anchored_inside_owned_content() {
    let mut a = ast("abc");
    a.items
        .insert(1, WordNode::Discretionary(DiscretionaryNode::direct(1)));
    assert!(check(&a, "abc").is_err());
}

#[test]
fn source_coverage_cannot_skip_an_internal_byte() {
    let mut a = ast("abc");
    a.items = vec![boxed(0, 1, false), boxed(2, 3, false), end(3)];
    assert!(check(&a, "abc").is_err());
}

#[test]
fn raw_metrics_with_one_negative_depth_are_rejected() {
    let mut a = ast("abc");
    a.strut_metrics.descent = -1.;
    assert!(check(&a, "abc").is_err());
}

#[test]
fn zero_total_raw_metrics_are_rejected() {
    let mut a = ast("abc");
    a.strut_metrics.ascent = 0.;
    a.strut_metrics.descent = 0.;
    assert!(check(&a, "abc").is_err());
}

#[test]
fn glue_only_source_must_cover_every_byte() {
    let mut a = ast("   ");
    a.items = vec![
        WordNode::Glue(GlueNode {
            original_bidi_class: BidiClass::Ws,
            bidi_level: 0,
            is_collapsible: true,
            byte_range: TextRange::new(0, 1),
            kind: GlueKind::WordSpace,
            tier: GlueTier::Tier0,
            natural_width: 1.,
            max_stretch: 0.,
            max_shrink: 0.,
        }),
        end(3),
    ];
    assert!(check(&a, "   ").is_err());
    if let WordNode::Glue(g) = &mut a.items[0] {
        g.byte_range.end = 3;
    }
    assert!(check(&a, "   ").is_ok());
}
#[test]
fn hanging_requires_overflow_beyond_tolerance() {
    let mut l = line();
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    for (x, advance) in [(99., 1.00001), (50., 0.)] {
        l.visual_runs = vec![run(glyph(x, advance, true))];
        assert!(l.validate(&c).is_err());
    }
}
