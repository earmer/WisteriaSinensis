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
            orientation: mutil_engine::types::GlyphOrientation::Horizontal,
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
            orientation: mutil_engine::types::GlyphOrientation::Horizontal,
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
        annotations: Vec::new(),
        bidi_spans: Vec::new(),
        writing_mode: mutil_engine::types::WritingMode::HorizontalTb,
        base_direction: Direction::Ltr,
        strut_metrics: FontMetrics {
            vertical_ascent: 5.0,
            vertical_descent: 5.0,
            vertical_line_gap: 0.0,
            ascent: 8.,
            descent: 2.,
            ..Default::default()
        },
        strut_font_size: 10.,
        runs: vec![ShapingRun {
            word_spacing: 0.0,
            baseline_align: mutil_engine::types::BaselineAlignment::Auto,
            letter_spacing: 0.0,
            orientation: mutil_engine::types::GlyphOrientation::Horizontal,
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
            first_line_indent: 0.0,
            geometry_provider: None,
            hanging_indent: 0.0,
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
        baseline_align: mutil_engine::types::BaselineAlignment::Auto,
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
        slots: smallvec::SmallVec::new(),
        annotations: Vec::new(),
        is_ellipsized: false,
        resolved_slot_constraints: smallvec::SmallVec::new(),
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
        strut: FontMetrics {
            vertical_descent: 5.0,
            vertical_line_gap: 0.0,
            x_height: 0.0,

            underline_thickness: 0.0,
            use_typo_metrics: false,
            vertical_ascent: 5.0,
            cap_height: 0.0,
            strikeout_position: 0.0,
            underline_position: 0.0,

            ideographic_baseline_offset: 0.0,
            mathematical_baseline_offset: 0.0,
            central_baseline_offset: 0.0,
            hanging_baseline_offset: 0.0,
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
        orientation: mutil_engine::types::GlyphOrientation::Horizontal,
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
fn malformed_atomic_fragments_are_rejected() {
    let mut l = line();
    l.atomic_fragments = vec![AtomicFragment {
        atomic_id: 1,
        full_byte_range: TextRange::new(0, 1),
        fragment_byte_range: TextRange::new(0, 1),
        x_start: 0.,
        x_end: 10.,
        bidi_level: 0,
    }];
    assert_eq!(l.validate(&ctx()), Ok(()));
    l.atomic_fragments[0].x_end = f32::NAN;
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidAtomicFragment { index: 0 })
    );
}

#[test]
fn invalid_visual_run_font_instance_is_rejected() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(0., 10., false))];
    l.visual_runs[0].font_size = 0.;
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidVisualRun { run: 0 })
    );
    l.visual_runs[0].font_size = 16.;
    l.visual_runs[0]
        .variations
        .push(FontVariation::new(*b"wght", f32::INFINITY));
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidVisualRun { run: 0 })
    );
}

#[test]
fn non_positive_line_height_is_named() {
    let mut l = line();
    l.line_height = 0.;
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::NonPositiveLineHeight {
            expected: 10.,
            actual: 0.
        })
    );
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
                first_line_indent: 0.0,
                geometry_provider: None,
                hanging_indent: 0.0,
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
            first_line_indent: 0.0,
            geometry_provider: None,
            hanging_indent: 0.0,
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
    // The box owns `0..3`, so a zero-width node at offset 1 is inside owned
    // content and breaks monotone item order.
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );
}

#[test]
fn zero_width_node_may_sit_on_a_content_boundary() {
    let mut a = ast("abc");
    a.items = vec![
        boxed(0, 1, false),
        WordNode::Discretionary(DiscretionaryNode::direct(1)),
        boxed(1, 3, false),
        end(3),
    ];
    assert!(check(&a, "abc").is_ok());
}

#[test]
fn text_box_atoms_must_tile_their_byte_range() {
    let mut a = ast("abc");
    if let WordNode::Box(BoxNode::Text {
        atoms,
        clusters,
        natural_width,
        ..
    }) = &mut a.items[0]
    {
        // Drop the middle atom (and its cluster): `0..1` + `2..3` still spans
        // `0..3`, but byte 1 is unrepresented.
        atoms.remove(1);
        clusters.retain(|c| c.byte_range != TextRange::new(1, 2));
        *natural_width = 2.;
    }
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::ItemNotSelfContained { item: 0 })
    );
}

#[test]
fn nbsp_glue_must_not_be_collapsible() {
    let mut a = ast("\u{a0}");
    a.items = vec![
        WordNode::Glue(GlueNode {
            original_bidi_class: BidiClass::Cs,
            bidi_level: 0,
            is_collapsible: true,
            byte_range: TextRange::new(0, 2),
            kind: GlueKind::WordSpace,
            tier: GlueTier::Tier0,
            natural_width: 4.,
            max_stretch: 0.,
            max_shrink: 0.,
        }),
        end(2),
    ];
    assert_eq!(
        check(&a, "\u{a0}"),
        Err(LayoutInputError::InvalidGlue { item: 0 })
    );
    if let WordNode::Glue(g) = &mut a.items[0] {
        g.is_collapsible = false;
    }
    assert!(check(&a, "\u{a0}").is_ok());
}

#[test]
fn every_paragraph_separator_is_rejected_in_paragraph_text() {
    for text in [
        "a\nb",
        "a\rb",
        "a\u{85}b",
        "a\u{2029}b",
        "a\u{1c}b",
        "a\u{1e}b",
    ] {
        let a = ast(text);
        assert_eq!(
            check(&a, text),
            Err(LayoutInputError::ParagraphSeparatorInText),
            "{text:?}"
        );
    }
}

#[test]
fn non_finite_cluster_glyph_measurement_is_rejected() {
    let mut a = ast("abc");
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].glyphs[0].x_advance = f32::NAN;
    }
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
}

#[test]
fn non_finite_variation_and_impossible_bidi_level_are_rejected() {
    let mut a = ast("abc");
    a.runs[0]
        .features
        .variations
        .push(FontVariation::new(*b"wght", f32::INFINITY));
    // `FontVariation::new` canonicalises NaN but not infinity.
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidShapingRun { run: 0 })
    );
    let mut a = ast("abc");
    a.runs[0].bidi_level = BIDI_MAX_DEPTH + 2;
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidShapingRun { run: 0 })
    );
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
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    // A non-zero-advance hanging glyph inside the block is still rejected.
    let mut l = line();
    l.visual_runs = vec![run(glyph(99., 1.00001, true))];
    assert!(l.validate(&c).is_err());
    // Zero-advance marks are exempt from containment and hanging.
    let mut l = line();
    l.visual_runs = vec![run(glyph(50., 0., true))];
    assert_eq!(l.validate(&c), Ok(()));
    // But they cannot fake a declared hanging extent.
    let mut l = line();
    l.hanging_right = 5.;
    l.visual_runs = vec![run(glyph(99., 0., true))];
    assert!(l.validate(&c).is_err());
}

// Phase 0 extension acceptance matrix.
fn orient(a: &mut ParagraphAst, mode: WritingMode, orientation: GlyphOrientation, level: u8) {
    a.writing_mode = mode;
    for r in &mut a.runs {
        r.orientation = orientation;
        r.bidi_level = level;
    }
    for node in &mut a.items {
        if let WordNode::Box(BoxNode::Text {
            atoms, clusters, ..
        }) = node
        {
            for atom in atoms {
                atom.orientation = orientation;
                atom.bidi_level = level;
            }
            for cluster in clusters {
                cluster.orientation = orientation;
                cluster.bidi_level = level;
            }
        }
    }
}
fn annotation(base: TextRange, side: AnnotationSide) -> AnnotationSpan {
    let mut a = ast("x");
    let WordNode::Box(BoxNode::Text {
        atoms, clusters, ..
    }) = a.items.remove(0)
    else {
        unreachable!()
    };
    AnnotationSpan {
        annotation_id: if side == AnnotationSide::Over { 1 } else { 2 },
        base_byte_range: base,
        side,
        kind: AnnotationKind::Ruby {
            alignment: AnnotationAlignment::Center,
            max_overhang: 0.0,
        },
        block_extent: 3.0,
        annotation_text: "x".into(),
        run: a.runs.remove(0),
        atoms,
        clusters,
    }
}
fn two_slots() -> (PositionedLine, LineContext) {
    let constraints = smallvec![
        InlineSlotConstraint {
            start: 0.0,
            end: 100.0
        },
        InlineSlotConstraint {
            start: 150.0,
            end: 250.0
        }
    ];
    let mut l = line();
    l.byte_range = TextRange::new(0, 2);
    l.visible_byte_end = 2;
    l.visual_width = 200.0;
    l.slots = smallvec![
        PositionedSlot {
            start: 0.0,
            visual_width: 100.0,
            constraint_width: 100.0
        },
        PositionedSlot {
            start: 150.0,
            visual_width: 100.0,
            constraint_width: 100.0
        }
    ];
    l.resolved_slot_constraints = constraints.clone();
    let mut right = glyph(150.0, 100.0, false);
    right.byte_range = Some(TextRange::new(1, 2));
    let mut r = run(glyph(0.0, 100.0, false));
    r.byte_range = l.byte_range;
    r.glyphs.push(right);
    l.visual_runs = vec![r];
    (l, ctx().with_slots(constraints, 250.0))
}
fn positioned_annotation() -> PositionedAnnotationRun {
    PositionedAnnotationRun {
        annotation_id: 1,
        base_byte_range: TextRange::new(0, 1),
        side: AnnotationSide::Over,
        font: FontRef::unscaled(FontId(1)),
        font_size: 5.0,
        variations: smallvec![],
        orientation: GlyphOrientation::Horizontal,
        baseline_offset: 8.0,
        max_overhang: 0.0,
        block_extent: 3.0,
        glyphs: vec![PositionedGlyph {
            byte_range: None,
            is_synthesized: true,
            ..glyph(25.0, 50.0, false)
        }],
    }
}

#[test]
fn phase0_orientation_and_tcy_matrix() {
    for (mode, orientation, level, valid) in [
        (
            WritingMode::HorizontalTb,
            GlyphOrientation::Horizontal,
            0,
            true,
        ),
        (
            WritingMode::HorizontalTb,
            GlyphOrientation::VerticalUpright,
            0,
            false,
        ),
        (
            WritingMode::VerticalRl,
            GlyphOrientation::Horizontal,
            0,
            false,
        ),
        (
            WritingMode::VerticalLr,
            GlyphOrientation::VerticalUpright,
            0,
            true,
        ),
        (
            WritingMode::VerticalRl,
            GlyphOrientation::VerticalUpright,
            1,
            false,
        ),
        (
            WritingMode::VerticalLr,
            GlyphOrientation::HorizontalInVertical,
            1,
            false,
        ),
        (
            WritingMode::VerticalRl,
            GlyphOrientation::VerticalSideways,
            1,
            true,
        ),
    ] {
        let mut a = ast("a");
        orient(&mut a, mode, orientation, level);
        assert_eq!(
            check(&a, "a").is_ok(),
            valid,
            "{mode:?} {orientation:?} {level}"
        );
    }
    let mut a = ast("ab");
    orient(
        &mut a,
        WritingMode::VerticalRl,
        GlyphOrientation::HorizontalInVertical,
        0,
    );
    assert_eq!(check(&a, "ab"), Err(LayoutInputError::OrientationMismatch));
    a.items = vec![
        boxed(0, 1, false),
        WordNode::Discretionary(DiscretionaryNode::direct(1)),
        boxed(1, 2, false),
        end(2),
    ];
    orient(
        &mut a,
        WritingMode::VerticalRl,
        GlyphOrientation::HorizontalInVertical,
        0,
    );
    assert_eq!(check(&a, "ab"), Err(LayoutInputError::OrientationMismatch));
    a.items.remove(1);
    assert!(check(&a, "ab").is_ok());
    if let WordNode::Box(BoxNode::Text { atoms, .. }) = &mut a.items[0] {
        atoms[0].orientation = GlyphOrientation::VerticalSideways;
    }
    assert_eq!(check(&a, "ab"), Err(LayoutInputError::OrientationMismatch));
}

#[test]
fn phase0_writing_mode_roundtrips_and_over_sides() {
    let point = Point { x: 13.0, y: 27.0 };
    let rect = Rect::new(3.0, 7.0, 11.0, 19.0);
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        assert_eq!(
            mode.physical_to_flow_point(mode.flow_to_physical_point(point, 100.0), 100.0),
            point
        );
        assert_eq!(
            mode.physical_to_flow_rect(mode.flow_to_physical_rect(rect, 100.0), 100.0),
            rect
        );
        let baseline = mode.flow_to_physical_point(point, 100.0);
        let over = mode.flow_to_physical_point(
            Point {
                y: point.y + mode.over_to_block(5.0),
                ..point
            },
            100.0,
        );
        if mode.is_vertical() {
            assert_eq!(over.x, baseline.x + 5.0);
        } else {
            assert_eq!(over.y, baseline.y - 5.0);
        }
        let (start, end) = mode.line_bounds(20.0, 8.0, 2.0);
        assert_eq!(
            (start, end),
            if mode == WritingMode::VerticalLr {
                (18.0, 28.0)
            } else {
                (12.0, 22.0)
            }
        );
    }
    use FlowMotion::*;
    assert_eq!(
        Motion::Left.flow_direction(WritingMode::VerticalRl),
        Some(BlockNext)
    );
    assert_eq!(
        Motion::Left.flow_direction(WritingMode::VerticalLr),
        Some(BlockPrev)
    );
    assert_eq!(
        Motion::Down.flow_direction(WritingMode::VerticalLr),
        Some(InlineNext)
    );
    assert_eq!(
        Motion::Previous.flow_direction(WritingMode::VerticalLr),
        None
    );
}

#[test]
fn phase0_font_metrics_all_fields_and_overflow() {
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
        assert!(check(&a, "a").is_err(), "field {i}");
    }
    let mut a = ast("a");
    a.strut_metrics.ascent = f32::MAX;
    a.strut_metrics.descent = f32::MAX;
    assert!(check(&a, "a").is_err());
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
        0,
    );
    let c = check(&a, "a").unwrap();
    assert_eq!((c.strut.ascent, c.strut.descent), (9.0, 3.0));
}

#[test]
fn phase0_explicit_spacing_and_discretionary_equations() {
    let mut a = ast("a");
    a.runs[0].letter_spacing = 1.0;
    if let WordNode::Box(BoxNode::Text {
        atoms, clusters, ..
    }) = &mut a.items[0]
    {
        atoms[0].flags |= AtomFlags::NO_LETTER_SPACING;
        clusters[0].flags = atoms[0].flags;
    }
    assert_eq!(check(&a, "a"), Err(LayoutInputError::InvalidSpacing));
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut a = ast("a");
        a.runs[0].word_spacing = value;
        assert_eq!(check(&a, "a"), Err(LayoutInputError::InvalidSpacing));
    }
    let mut a = ast("a");
    if let WordNode::Box(BoxNode::Text {
        clusters,
        natural_width,
        ..
    }) = &mut a.items[0]
    {
        clusters[0].natural_width = -1.0;
        *natural_width = -1.0;
    }
    assert!(check(&a, "a").is_err());
    for spacing in [-0.25, 0.25] {
        let mut a = ast("ab");
        let mut disc = DiscretionaryNode::direct(1);
        disc.no_break_width = spacing;
        disc.no_break_spacing = spacing;
        a.items = vec![
            boxed(0, 1, false),
            WordNode::Discretionary(disc),
            boxed(1, 2, false),
            end(2),
        ];
        assert!(check(&a, "ab").is_ok());
        if let WordNode::Discretionary(d) = &mut a.items[1] {
            d.no_break_spacing = 0.0;
        }
        assert!(check(&a, "ab").is_err());
    }
    let mut a = ast("a");
    if let WordNode::Discretionary(d) = &mut a.items[1] {
        d.no_break_width = 1.0;
        d.no_break_spacing = 1.0;
    }
    assert!(check(&a, "a").is_err());
}
fn bidi_span(id: u64, start: usize, end: usize) -> ExplicitBidiSpan {
    ExplicitBidiSpan {
        span_id: id,
        byte_range: TextRange::new(start, end),
        control: BidiControlKind::Isolate(None),
    }
}
#[test]
fn phase0_bidi_spans_order_ids_boundaries_and_depth() {
    let mut a = ast("abcd");
    a.bidi_spans = vec![bidi_span(1, 0, 4), bidi_span(2, 1, 3)];
    assert!(check(&a, "abcd").is_ok());
    for spans in [
        vec![bidi_span(1, 0, 3), bidi_span(2, 1, 4)],
        vec![bidi_span(1, 0, 2), bidi_span(1, 2, 4)],
        vec![bidi_span(1, 0, 4), bidi_span(2, 0, 4)],
    ] {
        a.bidi_spans = spans;
        assert_eq!(check(&a, "abcd"), Err(LayoutInputError::InvalidBidiSpan));
    }
    let text = "a".repeat(128);
    let mut a = ast(&text);
    a.bidi_spans = (0..63).map(|i| bidi_span(i as u64, i, 128 - i)).collect();
    assert!(check(&a, &text).is_ok());
    a.bidi_spans.push(bidi_span(63, 63, 65));
    assert_eq!(check(&a, &text), Err(LayoutInputError::InvalidBidiSpan));
    let mut a = ast("ab");
    if let WordNode::Box(BoxNode::Text {
        atoms, clusters, ..
    }) = &mut a.items[0]
    {
        atoms.truncate(1);
        atoms[0].byte_range.end = 2;
        clusters.truncate(1);
        clusters[0].byte_range.end = 2;
        clusters[0].natural_width = 2.0;
    }
    a.bidi_spans = vec![bidi_span(1, 1, 2)];
    assert_eq!(check(&a, "ab"), Err(LayoutInputError::InvalidBidiSpan));
}

#[test]
fn phase0_annotation_input_boundaries_and_local_partition() {
    let mut a = ast("ab");
    a.annotations = vec![
        annotation(TextRange::new(0, 2), AnnotationSide::Over),
        annotation(TextRange::new(0, 2), AnnotationSide::Under),
    ];
    assert!(check(&a, "ab").is_ok());
    a.annotations[1].side = AnnotationSide::Over;
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
    a.annotations.truncate(1);
    a.annotations[0].clusters.clear();
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
    a.annotations[0] = annotation(TextRange::new(0, 2), AnnotationSide::Over);
    a.items = vec![
        boxed(0, 1, false),
        WordNode::Discretionary(DiscretionaryNode::direct(1)),
        boxed(1, 2, false),
        end(2),
    ];
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
    let mut a = ast("ab");
    a.annotations = vec![annotation(TextRange::new(0, 2), AnnotationSide::Over)];
    a.annotations[0].kind = AnnotationKind::Emphasis;
    assert!(check(&a, "ab").is_ok());
    a.annotations[0].run.letter_spacing = f32::NAN;
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
    a.annotations[0] = annotation(TextRange::new(0, 2), AnnotationSide::Over);
    a.items = vec![
        boxed(0, 1, false),
        WordNode::Tab {
            byte_range: TextRange::new(1, 2),
        },
        end(2),
    ];
    assert!(check(&a, "ab").is_err());
}

#[test]
fn phase0_multi_slot_containment_and_conserved_width() {
    let (valid, c) = two_slots();
    assert_eq!(valid.validate(&c), Ok(()));
    let mut bad = valid.clone();
    bad.visual_runs[0].glyphs[0].advance = 151.0;
    assert!(bad.validate(&c).is_err());
    let mut bad = valid.clone();
    bad.slots[0].visual_width = 151.0;
    bad.visual_width = 251.0;
    assert!(bad.validate(&c).is_err());
    let mut bad = valid.clone();
    bad.visual_width += 1.0;
    assert_eq!(bad.validate(&c), Err(LineInvariantError::InvalidSlots));
    let mut bad = valid.clone();
    bad.visual_runs.clear();
    bad.visual_boxes.push(PositionedInlineBox {
        baseline_align: BaselineAlignment::Auto,
        box_id: 1,
        byte_range: TextRange::new(0, 1),
        x: 90.0,
        y: 0.0,
        width: 70.0,
        ascent: 8.0,
        descent: 2.0,
        overflow_policy: BoxOverflowPolicy::Clip,
    });
    assert!(bad.validate(&c).is_err());
    let mut bad = valid;
    bad.span_fragments.push(PositionedSpanFragment {
        span_id: 1,
        byte_range: TextRange::new(0, 2),
        x: 90.0,
        width: 70.0,
        ascent: 8.0,
        descent: 2.0,
        bidi_level: 0,
        is_first_fragment: true,
        is_last_fragment: true,
    });
    assert!(bad.validate(&c).is_err());
}

#[test]
fn phase0_empty_slot_hanging_and_trailing_whitespace() {
    let (mut l, mut c) = two_slots();
    l.visual_runs[0].glyphs.remove(0);
    l.slots[0].visual_width = 0.0;
    l.visual_width = 100.0;
    c.hang_mode.start = HangStartMode::AllLines;
    assert_eq!(l.validate(&c), Ok(()));
    let mut hanging = glyph(145.0, 5.0, true);
    hanging.byte_range = Some(TextRange::new(0, 1));
    l.visual_runs[0].glyphs.push(hanging);
    l.hanging_left = 5.0;
    assert!(l.validate(&c).is_err());
    l.visual_runs[0].glyphs.pop();
    l.hanging_left = 0.0;
    let trailing = PositionedGlyph {
        byte_range: Some(TextRange::new(2, 3)),
        ..glyph(250.0, 5.0, false)
    };
    l.byte_range.end = 3;
    l.visual_runs[0].glyphs.push(trailing);
    c.whitespace = HangingWhitespaceMode::Preserved;
    assert_eq!(l.validate(&c), Ok(()));
    l.visual_runs[0].glyphs.last_mut().unwrap().x = 145.0;
    assert!(l.validate(&c).is_err());
}

#[test]
fn phase0_annotation_output_and_synthesized_ownership() {
    let (mut l, c) = two_slots();
    l.ascent += 3.0;
    l.line_height += 3.0;
    l.baseline_y += 3.0;
    l.annotations.push(positioned_annotation());
    assert_eq!(l.validate(&c), Ok(()));
    let mut bad = l.clone();
    bad.annotations[0].glyphs[0].byte_range = Some(TextRange::new(0, 1));
    assert!(bad.validate(&c).is_err());
    let mut bad = l.clone();
    bad.annotations[0].base_byte_range.end = 2;
    assert!(bad.validate(&c).is_err());
    let mut bad = l.clone();
    bad.ascent -= 3.0;
    assert!(bad.validate(&c).is_err());
    let mut bad = l.clone();
    bad.annotations[0].glyphs[0].x = f32::NAN;
    assert!(bad.validate(&c).is_err());
    let mut bad = l;
    bad.visual_runs[0].glyphs[0].is_synthesized = true;
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::InvalidGlyphSource)
    );
}

#[test]
fn phase0_annotation_base_allows_glyph_less_source_gaps() {
    let mut l = line();
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 3;
    l.visual_width = 100.0;
    l.ascent += 3.0;
    l.line_height += 3.0;
    l.baseline_y += 3.0;
    let mut first = glyph(0.0, 10.0, false);
    first.byte_range = Some(TextRange::new(0, 1));
    let mut last = glyph(70.0, 10.0, false);
    last.byte_range = Some(TextRange::new(2, 3));
    l.visual_runs = vec![run(first), run(last)];
    let mut ann = positioned_annotation();
    ann.base_byte_range = TextRange::new(0, 3);
    ann.glyphs[0] = PositionedGlyph {
        byte_range: None,
        is_synthesized: true,
        ..glyph(25.0, 50.0, false)
    };
    l.annotations.push(ann);
    assert_eq!(l.validate(&ctx()), Ok(()));

    // A partially intersecting body glyph is still an ownership error even
    // when the base envelope would contain its painted extent.
    let mut bad = l.clone();
    bad.annotations[0].base_byte_range = TextRange::new(0, 2);
    bad.visual_runs[1].glyphs[0].byte_range = Some(TextRange::new(1, 3));
    assert_eq!(
        bad.validate(&ctx()),
        Err(LineInvariantError::InvalidAnnotation)
    );

    // A base with no intersecting body glyph is not a visible base.
    let mut bad = l;
    bad.annotations[0].base_byte_range = TextRange::new(1, 2);
    assert_eq!(
        bad.validate(&ctx()),
        Err(LineInvariantError::InvalidAnnotation)
    );
}

#[test]
fn phase0_unbounded_width_indent_and_clamp_options() {
    let a = ast("a");
    let mut constraints = LayoutConstraints::default();
    let c = check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()).unwrap();
    assert!(c.wrap_width.is_infinite());
    assert_eq!(line().validate(&c), Ok(()));
    constraints.wrap_width = 10.0;
    constraints.first_line_indent = 10.0;
    assert_eq!(
        check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()),
        Err(LayoutInputError::InvalidConstraints)
    );
    constraints.first_line_indent = -2.0;
    let c = check_layout_inputs(&a, "a", &constraints, &LayoutOptions::default()).unwrap();
    assert_eq!(c.slot_constraints[0].start, -2.0);
    assert_eq!(c.wrap_width, 10.0);
    for policy in [
        LineClampPolicy::default(),
        LineClampPolicy {
            max_lines: Some(0),
            ..Default::default()
        },
        LineClampPolicy {
            max_block_size: Some(f32::INFINITY),
            ..Default::default()
        },
        LineClampPolicy {
            max_lines: Some(1),
            ellipsis: "\u{2028}".into(),
            ..Default::default()
        },
    ] {
        let options = LayoutOptions {
            clamp: Some(policy),
            ..Default::default()
        };
        assert_eq!(
            check_layout_inputs(&a, "a", &constraints, &options),
            Err(LayoutInputError::InvalidLayoutOptions)
        );
    }
}

#[test]
fn phase0_truncation_and_multiple_synthesized_ellipsis_glyphs() {
    let mut l = line();
    l.visual_width = 120.0;
    l.is_ellipsized = true;
    l.visual_runs = vec![run(glyph(0.0, 100.0, false))];
    for x in [100.0, 110.0] {
        l.visual_runs[0].glyphs.push(PositionedGlyph {
            byte_range: None,
            is_synthesized: true,
            ..glyph(x, 10.0, false)
        });
    }
    let p = PositionedParagraph {
        writing_mode: WritingMode::HorizontalTb,
        base_direction: Direction::Ltr,
        size: Size {
            width: 120.0,
            height: 10.0,
        },
        total_block_size: 10.0,
        lines: vec![l],
        truncated_range: Some(TextRange::new(1, 3)),
    };
    let options = LayoutOptions {
        clamp: Some(LineClampPolicy {
            max_lines: Some(1),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(p.validate("abc", &options, &ctx()), Ok(()));
    let mut bad = p.clone();
    bad.truncated_range = Some(TextRange::new(2, 3));
    assert!(bad.validate("abc", &options, &ctx()).is_err());
    let mut bad = p.clone();
    let mut next = bad.lines[0].clone();
    next.baseline_y += 10.0;
    next.byte_range = TextRange::new(1, 2);
    bad.lines.push(next);
    bad.total_block_size = 20.0;
    assert!(bad.validate("abc", &options, &ctx()).is_err());
    assert!(
        p.validate("abc", &LayoutOptions::default(), &ctx())
            .is_err()
    );
}

#[derive(Debug)]
struct GrowingGeometry;
impl LineGeometryProvider for GrowingGeometry {
    fn query_line_geometry(&self, _: usize, _: f32, end: f32) -> LineGeometry {
        LineGeometry {
            slots: smallvec![InlineSlotConstraint {
                start: 0.0,
                end: if end > 10.0 { 200.0 } else { 100.0 }
            }],
            next_feature_block_offset: None,
            max_unobstructed_width: 200.0,
        }
    }
}
#[test]
fn phase0_provider_monotonicity_clearance_and_snapshots() {
    let constraints = LayoutConstraints {
        geometry_provider: Some(Arc::new(GrowingGeometry)),
        ..Default::default()
    };
    let mut a = ast("a");
    a.items[0] = WordNode::Box(BoxNode::InlineObject {
        baseline_align: BaselineAlignment::Auto,
        byte_range: TextRange::new(0, 1),
        box_id: 1,
        width: 10.0,
        ascent: 20.0,
        descent: 2.0,
        overflow_policy: BoxOverflowPolicy::Clip,
    });
    assert_eq!(
        layout_paragraph(
            &a,
            "a",
            &NoopShaper,
            &constraints,
            &LayoutOptions::default()
        ),
        Err(LayoutInputError::NonMonotonicGeometry)
    );
    let previous = LineGeometry {
        slots: smallvec![InlineSlotConstraint {
            start: 0.0,
            end: 100.0
        }],
        next_feature_block_offset: None,
        max_unobstructed_width: 100.0,
    };
    let split = LineGeometry {
        slots: smallvec![
            InlineSlotConstraint {
                start: 0.0,
                end: 20.0
            },
            InlineSlotConstraint {
                start: 40.0,
                end: 100.0
            }
        ],
        ..previous.clone()
    };
    assert!(split.is_subset_of(&previous));
    assert!(!previous.is_subset_of(&split));
    let (l, c) = two_slots();
    let p = PositionedParagraph {
        writing_mode: WritingMode::HorizontalTb,
        base_direction: Direction::Ltr,
        size: Size::default(),
        total_block_size: 10.0,
        truncated_range: None,
        lines: vec![l],
    };
    assert_eq!(p.validate("ab", &LayoutOptions::default(), &c), Ok(()));
}

#[test]
fn phase0_probe_ignores_tall_input_after_first_break() {
    let constraints = LayoutConstraints {
        geometry_provider: Some(Arc::new(GrowingGeometry)),
        ..Default::default()
    };
    let text = "a\u{2028}b";
    let mut a = ast(text);
    a.items = vec![
        boxed(0, 1, false),
        WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(1, 4), false)),
        WordNode::Box(BoxNode::InlineObject {
            baseline_align: BaselineAlignment::Auto,
            byte_range: TextRange::new(4, 5),
            box_id: 1,
            width: 10.0,
            ascent: 20.0,
            descent: 2.0,
            overflow_policy: BoxOverflowPolicy::Clip,
        }),
        end(5),
    ];
    assert_eq!(
        layout_paragraph(
            &a,
            text,
            &NoopShaper,
            &constraints,
            &LayoutOptions::default()
        ),
        Ok(PositionedParagraph::empty(Direction::Ltr))
    );
}

#[test]
fn phase0_infinity_does_not_equal_finite_slot_capacity() {
    let (mut l, c) = two_slots();
    l.slots[0].constraint_width = f32::INFINITY;
    assert_eq!(l.validate(&c), Err(LineInvariantError::InvalidSlots));
    let mut c = c;
    c.slot_constraints[0].start = -f32::MAX;
    c.slot_constraints[0].end = f32::MAX;
    assert!(l.validate(&c).is_err());
}

#[test]
fn phase0_ellipsis_only_empty_and_empty_paragraph_contracts() {
    let mut l = line();
    l.byte_range = TextRange::empty_at(0);
    l.visible_byte_end = 0;
    l.is_ellipsized = true;
    l.visual_width = 120.0;
    let mut r = run(glyph(0.0, 120.0, false));
    r.byte_range = TextRange::empty_at(0);
    r.glyphs[0].is_synthesized = true;
    r.glyphs[0].byte_range = None;
    l.visual_runs = vec![r];
    let mut p = PositionedParagraph::empty(Direction::Ltr);
    p.total_block_size = 10.0;
    p.lines = vec![l];
    p.truncated_range = Some(TextRange::new(0, 1));
    let mut options = LayoutOptions {
        clamp: Some(LineClampPolicy {
            max_lines: Some(1),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(p.validate("a", &options, &ctx()), Ok(()));
    p.lines[0].visual_runs.clear();
    p.lines[0].visual_width = 0.0;
    options.clamp.as_mut().unwrap().ellipsis = "".into();
    assert_eq!(p.validate("a", &options, &ctx()), Ok(()));
    p.truncated_range = None;
    p.lines[0].is_ellipsized = false;
    assert_eq!(p.validate("", &LayoutOptions::default(), &ctx()), Ok(()));
    p.lines.clear();
    assert!(p.validate("", &LayoutOptions::default(), &ctx()).is_err());
}

#[test]
fn phase0_ruby_rejects_glue_and_cluster_internal_edges() {
    let mut a = ast("a b");
    a.items = vec![
        boxed(0, 1, false),
        WordNode::Glue(GlueNode {
            original_bidi_class: BidiClass::Ws,
            bidi_level: 0,
            is_collapsible: true,
            byte_range: TextRange::new(1, 2),
            kind: GlueKind::WordSpace,
            tier: GlueTier::Tier0,
            natural_width: 1.0,
            max_stretch: 0.0,
            max_shrink: 0.0,
        }),
        boxed(2, 3, false),
        end(3),
    ];
    assert!(check(&a, "a b").is_ok());
    a.annotations = vec![annotation(TextRange::new(0, 3), AnnotationSide::Over)];
    assert_eq!(
        check(&a, "a b"),
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
    let mut a = ast("ab");
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters.truncate(1);
        clusters[0].byte_range.end = 2;
        clusters[0].atom_range.end = 2;
        clusters[0].natural_width = 2.0;
    }
    assert!(check(&a, "ab").is_ok());
    a.annotations = vec![annotation(TextRange::new(0, 1), AnnotationSide::Over)];
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan)
    );
}

#[derive(Debug)]
struct EmptyGeometry {
    next: Option<f32>,
}
impl LineGeometryProvider for EmptyGeometry {
    fn query_line_geometry(&self, _: usize, _: f32, _: f32) -> LineGeometry {
        LineGeometry {
            slots: smallvec![],
            next_feature_block_offset: self.next,
            max_unobstructed_width: 100.0,
        }
    }
}
#[test]
fn phase0_geometry_rejects_empty_without_progress_and_invalid_intervals() {
    for next in [
        None,
        Some(0.0),
        Some(1e-6),
        Some(f32::NAN),
        Some(f32::INFINITY),
    ] {
        let constraints = LayoutConstraints {
            geometry_provider: Some(Arc::new(EmptyGeometry { next })),
            ..Default::default()
        };
        assert_eq!(
            layout_paragraph(
                &ast("a"),
                "a",
                &NoopShaper,
                &constraints,
                &LayoutOptions::default()
            ),
            Err(LayoutInputError::InvalidLineGeometry)
        );
    }
    let mut geometry = LineGeometry {
        slots: smallvec![
            InlineSlotConstraint {
                start: 0.0,
                end: 100.0
            },
            InlineSlotConstraint {
                start: 90.0,
                end: 200.0
            }
        ],
        next_feature_block_offset: None,
        max_unobstructed_width: 200.0,
    };
    assert!(!geometry.is_valid(0.0));
    geometry.slots[1].start = 100.0;
    assert!(geometry.is_valid(0.0));
    geometry.slots[1].end = f32::INFINITY;
    assert!(geometry.is_valid(0.0));
    geometry.slots[0].start = -f32::MAX;
    geometry.slots[0].end = f32::MAX;
    assert!(!geometry.is_valid(0.0));
}

#[test]
fn phase0_vertical_lr_paragraph_uses_line_under_for_block_start() {
    let mut l = line();
    l.baseline_y = 2.0;
    let mut r = run(glyph(0.0, 100.0, false));
    r.orientation = GlyphOrientation::VerticalUpright;
    l.visual_runs = vec![r];
    let mut p = PositionedParagraph::empty(Direction::Ltr);
    p.writing_mode = WritingMode::VerticalLr;
    p.total_block_size = 10.0;
    p.lines = vec![l];
    assert_eq!(p.validate("a", &LayoutOptions::default(), &ctx()), Ok(()));
    p.lines[0].baseline_y = 1.0;
    assert!(p.validate("a", &LayoutOptions::default(), &ctx()).is_err());
}

#[test]
fn phase0_vertical_lr_under_annotation_moves_first_baseline() {
    let mut l = line();
    l.baseline_y = 3.0;
    l.descent = 3.0;
    l.line_height = 11.0;
    let mut r = run(glyph(0.0, 100.0, false));
    r.orientation = GlyphOrientation::VerticalUpright;
    l.visual_runs = vec![r];
    let mut ann = positioned_annotation();
    ann.side = AnnotationSide::Under;
    ann.orientation = GlyphOrientation::VerticalUpright;
    ann.block_extent = 1.0;
    ann.baseline_offset = -2.0;
    ann.glyphs[0].x = 0.0;
    ann.glyphs[0].advance = 10.0;
    l.annotations.push(ann);
    let mut p = PositionedParagraph::empty(Direction::Ltr);
    p.writing_mode = WritingMode::VerticalLr;
    p.total_block_size = 11.0;
    p.lines = vec![l];
    assert_eq!(p.validate("a", &LayoutOptions::default(), &ctx()), Ok(()));
    // The line-under side is the block start in VerticalLr, so shrinking the
    // baseline to the strut descent alone pushes the line into negative block
    // space once the annotation has extended descent.
    p.lines[0].baseline_y = 2.0;
    assert!(p.validate("a", &LayoutOptions::default(), &ctx()).is_err());
}
