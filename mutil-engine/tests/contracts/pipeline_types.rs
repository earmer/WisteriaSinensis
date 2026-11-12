//! The types of all three tiers fit together.
//!
//! No typesetting happens here. Every cluster, width and position below is a
//! hand-written fixture (fixed 10-unit advances) whose only purpose is to
//! exercise ownership, visibility and trait closure from *outside* the crate.

use crate::common::{
    ADV, assert_send_sync_static, atom, cluster, push_text, rebuild_items, shaping_run, single_slot,
};
use mutil_engine::*;

const TEXT: &str = "Hello world";

fn run() -> ShapingRun {
    shaping_run(TEXT, "en-GB")
}

fn word(ast: &mut ParagraphAst, range: TextRange) -> WordNode {
    let atoms: Vec<_> = range.as_range().map(atom).collect();
    let clusters: Vec<_> = (0..atoms.len()).map(cluster).collect();
    push_text(ast, 0, atoms, clusters)
}

#[test]
fn test_pipeline_types_compile() {
    // Frontend: a pseudo ParagraphAst of Box, Discretionary, Glue, Box.
    let mut ast = ParagraphAst {
        annotations: Vec::new(),
        bidi_spans: Vec::new(),
        writing_mode: WritingMode::HorizontalTb,
        base_direction: BidiDirection::Ltr,
        strut_metrics: FontMetrics {
            vertical_ascent: 5.0,
            vertical_descent: 5.0,
            vertical_line_gap: 0.0,
            ascent: 12.0,
            descent: 4.0,
            ..Default::default()
        },
        strut_font_size: 16.0,
        runs: vec![run()],
        atoms: vec![],
        clusters: vec![],
        glyphs: Vec::new(),
        items: vec![],
        decoration_spans: vec![],
        atomic_spans: vec![],
    };
    ast.items = vec![
        word(&mut ast, TextRange::new(0, 5)),
        // A breakable space: the break opportunity follows the glue.
        WordNode::Glue(GlueNode {
            original_bidi_class: BidiClass::Ws,
            bidi_level: BidiLevel::LTR,
            is_collapsible: true,

            byte_range: TextRange::new(5, 6),
            kind: GlueKind::WordSpace,
            natural_width: 4.0,
            max_stretch: 2.0,
            max_shrink: 1.0,
        }),
        WordNode::Discretionary(DiscretionaryNode::direct(6)),
        word(&mut ast, TextRange::new(6, 11)),
    ];

    // Shaping bridge: the no-op backend, through a trait object.
    let shaper: &dyn TextShaper = &NoopShaper;
    let r = &ast.runs[0];
    let (font_id, metrics) = shaper.resolve_primary_font_metrics(
        &r.families,
        r.style,
        r.script,
        &r.locale,
        r.font_size,
        r.features.variations(),
    );
    assert_eq!(font_id, FontId(0));
    assert_eq!(metrics, FontMetrics::default());

    let (mut out, mut out_glyphs) = (Vec::new(), Vec::new());
    shaper.shape_run(TEXT, r, &[], &mut out, &mut out_glyphs);
    let run_atoms = ast.run_atoms(0);
    assert_eq!(run_atoms.len(), 10); // "Hello" and "world"; the space is glue
    shaper.reshape_window(
        TEXT,
        r,
        run_atoms,
        TextRange::new(0, 5),
        LeadingEdit::None,
        TrailingEdit::AppendHyphen { hyphen: '-' },
        &mut out,
        &mut out_glyphs,
    );
    assert!(out.is_empty() && out_glyphs.is_empty());
    assert!(
        shaper
            .ligature_carets(
                FontInstanceRef {
                    font: FontRef::unscaled(font_id),
                    font_size: 16.0,
                    variations: &[]
                },
                0,
                r.direction()
            )
            .is_empty()
    );

    // Middle-end: assemble the fixture clusters into a PositionedParagraph.
    let mut line_glyphs = Vec::new();
    let mut pen = 0.0_f32; // fixture bookkeeping only, not a layout algorithm
    for item in 0..ast.items.len() {
        if let Some(view) = ast.text_box(item) {
            for c in view.clusters {
                let g = ast.cluster_glyphs(c)[0];
                line_glyphs.push(PositionedGlyph {
                    glyph_id: g.glyph_id,
                    x: pen,
                    x_offset: g.x_offset,
                    y_offset: g.y_offset,
                    advance: g.x_advance,
                    byte_range: view.cluster_byte_range(c),
                    is_hanging: false,
                });
                pen += c.natural_width;
            }
        }
    }
    // A generated hyphen: no source bytes, flagged synthesized.
    line_glyphs.push(PositionedGlyph {
        glyph_id: b'-' as u32,
        x: pen,
        x_offset: 0.0,
        y_offset: 0.0,
        advance: ADV,
        byte_range: None,
        is_hanging: false,
    });
    assert_eq!(line_glyphs.len(), 11);
    assert!(
        line_glyphs
            .last()
            .is_some_and(PositionedGlyph::is_synthesized)
    );

    let line = PositionedLine {
        slots: single_slot(pen + ADV, f32::INFINITY),
        annotations: Vec::new(),
        is_ellipsized: false,
        atomic_fragments: vec![],

        tabs: vec![],
        span_fragments: vec![],
        byte_range: TextRange::new(0, TEXT.len()),
        visible_byte_end: TEXT.len(),
        baseline_y: ast.strut_metrics.ascent,
        ascent: ast.strut_metrics.ascent,
        descent: ast.strut_metrics.descent,
        line_height: ast.strut_metrics.natural_line_height(),
        is_justified: false,
        visual_runs: vec![VisualGlyphRun {
            orientation: GlyphOrientation::Horizontal,
            font_size: 16.0,
            variations: Default::default(),

            font: FontRef::unscaled(FontId(1)),
            bidi_level: BidiLevel::LTR,
            byte_range: TextRange::new(0, TEXT.len()),
            glyphs: line_glyphs,
        }],
        caret_stops: vec![CaretStop {
            x: 0.0,
            offset: 0,
            affinity: CaretAffinity::Downstream,
            level: BidiLevel::LTR,
            line: 0,
        }],
        visual_boxes: vec![],
        decorations: vec![],
    };

    let policy = LayoutPolicySnapshot::resolve(
        &ast,
        &LayoutConstraints::default(),
        &LayoutOptions::default(),
    )
    .expect("the fixture's layout inputs are valid");
    let para = PositionedParagraph {
        policy,
        base_direction: ast.base_direction,
        truncated_range: None,
        total_block_size: 0.0,
        lines: vec![line],
    };
    assert_eq!(para.lines.len(), 1);

    // Interaction types.
    let cursor = CursorPosition {
        byte_offset: 5,
        affinity: CaretAffinity::Upstream,
    };
    let sel = SelectionRect {
        rect: Rect::new(0.0, 0.0, 50.0, 16.0),
        line: 0,
        level: BidiLevel::LTR,
        is_line_end_extension: false,
    };
    assert_eq!(cursor.affinity, CaretAffinity::Upstream);
    assert_eq!(sel.rect.right(), 50.0);

    // Ownership: both ends of the pipeline own their data outright.
    assert_send_sync_static::<ParagraphAst>();
    assert_send_sync_static::<PositionedParagraph>();
}

#[test]
fn fixture_clusters_index_their_own_node_atoms() {
    // Two words in one run: the second box's atoms start at run-local index 5.
    let mut ast = crate::common::ast("");
    rebuild_items(&mut ast, |ast| {
        vec![
            word(ast, TextRange::new(0, 5)),
            word(ast, TextRange::new(6, 11)),
        ]
    });
    let view = ast.text_box(1).unwrap();
    assert_eq!(view.natural_width, 5.0 * ADV);
    let run = &ast.runs[0];
    let run_atoms = ast.run_atoms(0);
    assert_eq!(run_atoms.len(), 10);
    for c in view.clusters {
        // Cluster ranges are run-local: they index the run's atom slice, and the
        // paragraph-level index adds the run's start.
        let covered = &run_atoms[c.atom_range.clone()];
        assert_eq!(covered.len(), 1);
        assert_eq!(Some(covered[0].byte_range), view.cluster_byte_range(c));
        assert_eq!(
            ast.atoms[run.atom_range.start + c.atom_range.start],
            covered[0]
        );
        assert!(c.safety_flags_consistent());
    }
    assert_eq!(view.clusters[0].atom_range, 5..6);
}

#[test]
fn every_word_node_variant_is_constructible() {
    let mut ast = crate::common::ast("");
    let nodes = [
        word(&mut ast, TextRange::new(0, 5)),
        WordNode::Box(BoxNode::InlineObject {
            bidi_level: BidiLevel::LTR,
            baseline_align: BaselineAlignment::Auto,
            byte_range: TextRange::new(7, 8),
            box_id: 7,
            width: 12.0,
            ascent: 10.0,
            descent: 2.0,
            overflow_policy: BoxOverflowPolicy::ScaleToFit,
        }),
        WordNode::Box(BoxNode::SpanBoundary {
            byte_range: TextRange::empty_at(5),
            span_id: 3,
            edge: SpanEdge::Leading,
            width: 6.0,
            bidi_level: BidiLevel::RTL,
        }),
        WordNode::Glue(GlueNode {
            original_bidi_class: BidiClass::Ws,
            bidi_level: BidiLevel::LTR,
            is_collapsible: true,

            byte_range: TextRange::empty_at(5),
            kind: GlueKind::CjkAutospace,
            natural_width: 4.0,
            max_stretch: 8.0,
            max_shrink: 2.0,
        }),
        WordNode::Discretionary(DiscretionaryNode::direct(5)),
        WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(6, 7), false)),
        WordNode::Tab {
            byte_range: TextRange::new(5, 6),
        },
    ];
    // Exhaustive: adding a variant without updating these matches fails to compile.
    for n in &nodes {
        match n {
            WordNode::Box(b) => match b {
                BoxNode::Text { .. }
                | BoxNode::InlineObject { .. }
                | BoxNode::SpanBoundary { .. } => {}
            },
            WordNode::Glue(_) | WordNode::Discretionary(_) | WordNode::Tab { .. } => {}
        }
        // Only a discretionary is a break opportunity.
        assert_eq!(
            n.is_break_opportunity(),
            matches!(n, WordNode::Discretionary(_))
        );
    }
    assert_eq!(nodes.len(), 7);
    // A soft discretionary owns no bytes: its span is empty at the break. A
    // hard break owns its newline.
    let owned = |n: &WordNode| match n {
        WordNode::Discretionary(d) => d.byte_range,
        _ => unreachable!(),
    };
    assert_eq!(owned(&nodes[4]), TextRange::empty_at(5));
    assert_eq!(owned(&nodes[5]), TextRange::new(6, 7));
    // Break actions are exhaustive too.
    let actions = [
        BreakAction::Direct,
        BreakAction::HardBreak {
            is_paragraph_end: true,
        },
        BreakAction::Hyphenate(HyphenOpportunity {
            prefix_replacement: None,
            suffix_replacement: None,
            prefix_cut_bytes: 0,
            suffix_cut_bytes: 0,
        }),
        BreakAction::CjkPunctAdjust,
        BreakAction::MathOperator {
            repeat: Some(RepeatedOperator {
                glyph_id: 9,
                advance: 6.0,
            }),
        },
    ];
    for a in &actions {
        match a {
            BreakAction::Direct
            | BreakAction::HardBreak { .. }
            | BreakAction::Hyphenate(_)
            | BreakAction::CjkPunctAdjust
            | BreakAction::MathOperator { .. } => {}
        }
    }
}

#[test]
fn canonical_penalty_scale_is_ordered_and_tiers_are_ranked() {
    use DiscretionaryNode as D;
    let scale = [
        D::MANDATORY_BREAK_PENALTY,
        D::NATURAL_PENALTY,
        D::URL_PENALTY_MIN,
        D::URL_PENALTY_MAX,
        D::HYPHEN_PENALTY,
        D::SPELLING_CHANGE_HYPHEN_PENALTY,
        D::CLONED_SPAN_PENALTY,
        D::MATH_RELATION_PENALTY,
        D::MATH_BINARY_OPERATOR_PENALTY,
        D::FORBIDDEN_BREAK_PENALTY,
    ];
    assert!(scale.windows(2).all(|w| w[0] < w[1]));
    assert!(D::hard_break(TextRange::EMPTY, true).is_mandatory());
    assert!(!D::direct(0).is_mandatory());
    assert!(GlueTier::Tier0 < GlueTier::Tier1 && GlueTier::Tier1 < GlueTier::Tier2);
    assert_eq!(GlueKind::WordSpace.tier(), GlueTier::Tier0);
    assert_eq!(GlueKind::CjkAutospace.tier(), GlueTier::Tier1);
    assert_eq!(GlueKind::CjkPunctCompression.tier(), GlueTier::Tier0);
    assert_eq!(GlueTier::Tier2 as u8, 2);
}

#[test]
fn hyphen_opportunity_carries_prefix_and_suffix_replacements() {
    let h = HyphenOpportunity {
        prefix_replacement: Some("a".into()),
        suffix_replacement: Some("tje".into()),
        prefix_cut_bytes: 1,
        suffix_cut_bytes: 2,
    };
    assert_eq!(h.prefix_replacement.as_deref(), Some("a"));
    assert_eq!(h.suffix_replacement.as_deref(), Some("tje"));
    assert_eq!((h.prefix_cut_bytes, h.suffix_cut_bytes), (1, 2));
}

#[test]
fn clusters_partition_atoms_and_split_clusters_are_detected() {
    // Fixture words satisfy both invariants.
    let mut ast = crate::common::ast("");
    rebuild_items(&mut ast, |ast| vec![word(ast, TextRange::new(0, 5))]);
    assert!(ast.is_self_contained(0));

    // A two-atom ligature cluster is fine while it stays inside one node...
    let lig = |range: std::ops::Range<usize>| {
        let mut c = cluster(0);
        c.atom_range = range;
        c
    };
    let atoms: Vec<_> = (0..3).map(atom).collect();
    // A paragraph whose only text box holds `atoms` and `clusters`.
    let build = |atoms: Vec<GraphemeAtom>, clusters: Vec<ShapedCluster>| {
        let mut ast = crate::common::ast("");
        rebuild_items(&mut ast, |ast| vec![push_text(ast, 0, atoms, clusters)]);
        ast
    };
    let contained = |atoms, clusters| build(atoms, clusters).is_self_contained(0);
    assert!(contained(atoms.clone(), vec![lig(0..2), lig(2..3)]));

    // ...but a cluster reaching past the box's atoms does not resolve
    // (invariant 1).
    assert!(!contained(atoms[..1].to_vec(), vec![lig(0..2)]));

    // A cluster that skips the run's first atom leaves it to no cluster: the
    // box itself is consistent, the run's storage is not (invariant 2).
    let gap = build(atoms.clone(), vec![lig(1..3)]);
    assert!(gap.is_self_contained(0));
    assert_eq!(
        crate::common::check(&gap, "abc"),
        Err(LayoutInputError::InvalidRunStorage { run: 0 })
    );
    // Atoms that run backwards or overlap are not a partition of the text,
    // even when every cluster covers its own atoms.
    assert!(!contained(
        vec![atom(1), atom(0)],
        vec![lig(0..1), lig(1..2)]
    ));
    let overlapping = vec![
        GraphemeAtom {
            byte_range: TextRange::new(0, 2),
            ..atom(0)
        },
        GraphemeAtom {
            byte_range: TextRange::new(1, 3),
            ..atom(1)
        },
    ];
    assert!(!contained(overlapping, vec![lig(0..1), lig(1..2)]));
    // Every other node is trivially self-contained.
    let mut tab = crate::common::ast("");
    rebuild_items(&mut tab, |_| {
        vec![WordNode::Tab {
            byte_range: TextRange::new(0, 1),
        }]
    });
    assert!(tab.is_self_contained(0));
}

#[test]
fn full_spec_type_surface_is_nameable() {
    let pos = CursorPosition {
        byte_offset: 3,
        affinity: CaretAffinity::Downstream,
    };
    let hit = HitResult {
        position: pos,
        inside: false,
    };
    let sel = Selection {
        anchor: pos,
        focus: CursorPosition {
            byte_offset: 7,
            ..pos
        },
    };
    let caret = CaretVisual {
        primary_rect: Rect::new(10.0, 0.0, 1.0, 16.0),
        secondary_rect: None,
        slant_angle: 0.0,
    };
    assert!(!hit.inside && sel.anchor != sel.focus && caret.secondary_rect.is_none());

    let motions = [
        Motion::Left,
        Motion::Right,
        Motion::Previous,
        Motion::Next,
        Motion::WordLeft,
        Motion::WordRight,
        Motion::WordPrevious,
        Motion::WordNext,
        Motion::LineStart,
        Motion::LineEnd,
        Motion::Up,
        Motion::Down,
    ];
    assert_eq!(motions.len(), 12);
    assert_ne!(
        HangMode {
            start: HangStartMode::FirstLine,
            end: HangEndMode::None
        },
        HangMode {
            start: HangStartMode::None,
            end: HangEndMode::Allow
        }
    );
    assert_ne!(QuotePolicy::CjkAware, QuotePolicy::Standard);
}

#[test]
fn hanging_punctuation_fits_negative_x_and_synthesized_flag_round_trips() {
    // A hanging opening bracket sits left of the line edge.
    let hanging = PositionedGlyph {
        glyph_id: 1,
        x: -ADV,
        x_offset: 0.0,
        y_offset: 0.0,
        advance: ADV,
        byte_range: Some(TextRange::new(0, 1)),
        is_hanging: true,
    };
    assert!(hanging.x < 0.0);

    // The shaper marks a glyph it inserts (the hyphen of a `TrailingEdit`),
    // since a raw glyph has no source range of its own; once positioned, the
    // glyph carries `byte_range: None` instead.
    let g = RawShapedGlyph {
        is_synthesized: true,
        ..Default::default()
    };
    assert!(g.is_synthesized);
}

#[test]
fn font_variation_is_nameable_and_canonical() {
    let mut f = FontFeatures::new();
    f.push_variation(*b"wght", 650.0);
    f.push_variation(*b"slnt", -0.0);
    let v: &[FontVariation] = f.variations();
    assert_eq!(v[0], FontVariation::new(*b"wght", 650.0));
    assert_eq!(v[1].value().to_bits(), 0.0_f32.to_bits());
}
