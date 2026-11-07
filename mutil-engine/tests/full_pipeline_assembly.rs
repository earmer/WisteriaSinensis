//! Zero-layout full pipeline assembly.
//!
//! One paragraph that uses every `WordNode` variant, a hyphenation opportunity
//! with both replacements (driving `ReplacePrefix` and `ReplaceSuffix`), and
//! laid-out lines including right-hanging punctuation and a trailing blank
//! line. Nothing is computed by a layout algorithm: every width and position is
//! a hand-written fixture (10-unit advances; the hyphenation example is a
//! structural fixture, not real Dutch).

use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;

// bytes: 0..7 "omaatje" | 7..8 " " | 8..11 U+FFFC | 11..12 "\t" | 12..14 "go"
//        | 14..15 "," | 15..16 "\n"
const TEXT: &str = "omaatje \u{FFFC}\tgo,\n";
const ADV: f32 = 10.0;
const BOX_W: f32 = 12.0;
const TAB_W: f32 = 8.0;
const SPACE_W: f32 = 4.0;

fn atom(offset: usize) -> GraphemeAtom {
    GraphemeAtom {
        byte_range: TextRange::new(offset, offset + 1),
        script: ScriptTag::LATIN,
        bidi_level: 0,
        linebreak_class: LineBreakClass::Al,
        flags: AtomFlags::EMPTY,
    }
}

fn cluster(base: usize, local: usize) -> ShapedCluster {
    let off = base + local;
    ShapedCluster {
        byte_range: TextRange::new(off, off + 1),
        atom_range: local..local + 1,
        font: FontRef::unscaled(FontId(1)),
        bidi_level: 0,
        flags: AtomFlags::EMPTY,
        glyphs: smallvec![RawShapedGlyph {
            glyph_id: TEXT.as_bytes()[off] as u32,
            cluster: off as u32,
            x_advance: ADV,
            ..Default::default()
        }],
        natural_width: ADV,
        uncompressed_width: ADV,
        safe_to_break_before: true,
        safe_to_concat_before: true,
    }
}

fn word(start: usize, end: usize, hyphen_breaks: Vec<HyphenOpportunity>) -> WordNode {
    let atoms: Vec<_> = (start..end).map(atom).collect();
    let clusters: Vec<_> = (0..atoms.len()).map(|i| cluster(start, i)).collect();
    let total_width = clusters.iter().map(|c| c.natural_width).sum();
    WordNode::Word {
        run_index: 0,
        atoms,
        clusters,
        hyphen_breaks,
        total_width,
    }
}

fn glyph(c: &ShapedCluster, x: f32, hanging: bool) -> PositionedGlyph {
    PositionedGlyph {
        glyph_id: c.glyphs[0].glyph_id,
        x,
        y_offset: 0.0,
        advance: c.natural_width,
        byte_range: Some(c.byte_range),
        is_synthesized: false,
        is_hanging: hanging,
    }
}

fn clusters_of(node: &WordNode) -> &[ShapedCluster] {
    match node {
        WordNode::Word { clusters, .. } | WordNode::PunctuationRun { clusters, .. } => clusters,
        _ => &[],
    }
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}

fn run() -> ShapingRun {
    ShapingRun {
        byte_range: TextRange::new(0, TEXT.len()),
        families: Arc::from([FontFamily::Generic(GenericFamily::SansSerif)]),
        font_size: 16.0,
        style: FontStyle::default(),
        script: ScriptTag::LATIN,
        locale: LocaleTag::new("nl-NL"),
        bidi_level: 0,
        decoration: TextDecoration::default(),
    }
}

#[test]
fn zero_layout_full_pipeline_assembly() {
    // ---- Frontend: every WordNode variant -------------------------------
    let opportunity = HyphenOpportunity {
        byte_offset: 3,
        prefix_replacement: Some("a".into()),
        suffix_replacement: Some("tje".into()),
        prefix_cut_bytes: 1,
        suffix_cut_bytes: 2,
        penalty: 50,
    };
    let ast = ParagraphAst {
        base_direction: BidiDirection::Ltr,
        strut_metrics: VerticalMetrics {
            ascent: 12.0,
            descent: 4.0,
            line_gap: 0.0,
        },
        runs: vec![run()],
        items: vec![
            word(0, 7, vec![opportunity.clone()]),
            WordNode::Glue {
                byte_range: TextRange::new(7, 8),
                kind: GlueKind::WordSpace,
                natural_width: SPACE_W,
                max_stretch: 2.0,
                max_shrink: 1.0,
            },
            WordNode::InlineBox {
                byte_range: TextRange::new(8, 11),
                box_id: 42,
                width: BOX_W,
                ascent: 10.0,
                descent: 2.0,
            },
            WordNode::Tab {
                byte_range: TextRange::new(11, 12),
            },
            word(12, 14, vec![]),
            WordNode::PunctuationRun {
                run_index: 0,
                atoms: vec![atom(14)],
                clusters: vec![cluster(14, 0)],
                bind: PunctBind::WholeRun,
            },
            WordNode::HardBreak {
                byte_range: TextRange::new(15, 16),
            },
        ],
    };
    assert_eq!(ast.items.len(), 7);
    assert!(ast.items.iter().all(WordNode::is_self_contained));
    // Nodes tile the text exactly: no cluster was split or lost.
    let covered: usize = ast
        .items
        .iter()
        .flat_map(clusters_of)
        .map(|c| c.byte_range.len())
        .sum();
    assert_eq!(covered, 7 + 2 + 1); // "omaatje" + "go" + ","

    // ---- Shaping bridge: both spelling edits through the trait ----------
    let shaper: &dyn TextShaper = &NoopShaper;
    let r = &ast.runs[0];
    let run_atoms: Vec<_> = (0..TEXT.len()).map(atom).collect();
    let features = FontFeatures::new();
    // Each side reads only its own replacement and cut count.
    let prefix_edit = BoundaryEdit::prefix_for(&opportunity, '-');
    let suffix_edit = BoundaryEdit::suffix_for(&opportunity);
    assert_eq!(
        prefix_edit,
        BoundaryEdit::ReplacePrefix {
            replacement: "a".into(),
            cut_bytes: 1,
            hyphen: '-'
        }
    );
    assert_eq!(
        suffix_edit,
        BoundaryEdit::ReplaceSuffix {
            replacement: "tje".into(),
            cut_bytes: 2
        }
    );
    let mut out = Vec::new();
    shaper.reshape_window(
        TEXT,
        r,
        &run_atoms,
        TextRange::new(0, 3),
        &features,
        prefix_edit,
        &mut out,
    );
    shaper.reshape_window(
        TEXT,
        r,
        &run_atoms,
        TextRange::new(3, 7),
        &features,
        suffix_edit,
        &mut out,
    );
    assert!(out.is_empty());

    // ---- Middle-end: three lines ----------------------------------------
    let word_a = clusters_of(&ast.items[0]);
    let word_b = clusters_of(&ast.items[4]);
    let comma = &clusters_of(&ast.items[5])[0];

    // Line 0: "oma" + generated hyphen. Hyphen glyph is synthesized, no bytes.
    let mut l0_glyphs: Vec<_> = word_a[..3]
        .iter()
        .enumerate()
        .map(|(i, c)| glyph(c, i as f32 * ADV, false))
        .collect();
    l0_glyphs.push(PositionedGlyph {
        glyph_id: b'-' as u32,
        x: 3.0 * ADV,
        y_offset: 0.0,
        advance: ADV,
        byte_range: None,
        is_synthesized: true,
        is_hanging: false,
    });
    let line0 = PositionedLine {
        byte_range: TextRange::new(0, 3),
        visible_byte_end: 3,
        baseline_y: 12.0,
        ascent: 12.0,
        descent: 4.0,
        line_height: 16.0,
        visual_width: 4.0 * ADV,
        hanging_left: 0.0,
        hanging_right: 0.0,
        is_justified: false,
        visual_runs: vec![VisualGlyphRun {
            font: FontRef::unscaled(FontId(1)),
            bidi_level: 0,
            byte_range: TextRange::new(0, 3),
            glyphs: l0_glyphs,
        }],
        caret_stops: vec![],
        visual_boxes: vec![],
        decorations: vec![],
    };

    // Line 1: "atje" ␠ [box] ⇥ "go" and a right-hanging ",". The box, tab and
    // space occupy width but carry no glyphs. `visual_width` stops before ",".
    let mut pen = 0.0;
    let mut l1_glyphs = Vec::new();
    for c in word_a[3..].iter() {
        l1_glyphs.push(glyph(c, pen, false));
        pen += c.natural_width;
    }
    pen += SPACE_W;
    let box_x = pen;
    pen += BOX_W + TAB_W;
    for c in word_b {
        l1_glyphs.push(glyph(c, pen, false));
        pen += c.natural_width;
    }
    let body_width = pen;
    l1_glyphs.push(glyph(comma, body_width, true));
    let line1 = PositionedLine {
        byte_range: TextRange::new(3, 16),
        visible_byte_end: 15,
        baseline_y: 32.0,
        ascent: 12.0,
        descent: 4.0,
        line_height: 16.0,
        visual_width: body_width,
        hanging_left: 0.0,
        hanging_right: ADV,
        is_justified: false,
        visual_runs: vec![VisualGlyphRun {
            font: FontRef::unscaled(FontId(1)),
            bidi_level: 0,
            byte_range: TextRange::new(3, 15),
            glyphs: l1_glyphs,
        }],
        caret_stops: vec![],
        visual_boxes: vec![PositionedInlineBox {
            box_id: 42,
            byte_range: TextRange::new(8, 11),
            x: box_x,
            y: 0.0,
            width: BOX_W,
            ascent: 10.0,
            descent: 2.0,
        }],
        decorations: vec![],
    };

    // Line 2: the empty line after the hard break (Stage 4.2).
    let line2 = PositionedLine {
        byte_range: TextRange::empty_at(16),
        visible_byte_end: 16,
        baseline_y: 48.0,
        ascent: 12.0,
        descent: 4.0,
        line_height: ast.strut_metrics.natural_line_height(),
        visual_width: 0.0,
        hanging_left: 0.0,
        hanging_right: 0.0,
        is_justified: false,
        visual_runs: vec![],
        caret_stops: vec![CaretStop {
            x: 0.0,
            offset: 16,
            affinity: CaretAffinity::Downstream,
            level: 0,
            line: 2,
        }],
        visual_boxes: vec![],
        decorations: vec![],
    };

    // Copy: no borrow of the AST survives
    let strut = ast.strut_metrics;
    let ctx = LineContext {
        strut,
        ..Default::default()
    };
    for (i, l) in [&line0, &line1, &line2].into_iter().enumerate() {
        assert_eq!(l.validate(&ctx), Ok(()), "line {i}");
    }
    assert!(line1.hanging_right > 0.0 && line0.hanging_right == 0.0);

    let mut para = PositionedParagraph::empty(ast.base_direction);
    para.size = Size {
        width: body_width,
        height: 48.0,
    };
    para.lines = vec![line0, line1, line2];

    // ---- Ownership: the output stands alone ------------------------------
    assert_send_sync_static::<ParagraphAst>();
    assert_send_sync_static::<PositionedParagraph>();
    assert_send_sync_static::<BoundaryEdit>();
    let ast_copy = ast.clone();
    drop(ast); // the layout holds no borrow of the AST
    assert!(ast_copy.items.iter().all(WordNode::is_self_contained));
    assert_eq!(para.lines.len(), 3);
    assert_eq!(para.clone(), para);
    // Tampering is caught: counting the hanging comma in visual_width.
    let mut bad = para.lines[1].clone();
    bad.visual_width += ADV;
    bad.hanging_right = 0.0;
    assert!(matches!(
        bad.validate(&ctx),
        Err(LineInvariantError::HangingGlyphInsideBlock { .. })
    ));
}

#[test]
fn extreme_assembly_every_cluster_belongs_to_one_node() {
    // Words of one atom each, glue between them, 200 nodes: the partition check
    // holds for every node and the cluster byte ranges tile the text without
    // overlap.
    let text_len = 200usize;
    let items: Vec<_> = (0..text_len)
        .map(|i| {
            let atoms = vec![atom(i)];
            WordNode::Word {
                run_index: 0,
                clusters: vec![ShapedCluster {
                    byte_range: atoms[0].byte_range,
                    atom_range: 0..1,
                    font: FontRef::unscaled(FontId(1)),
                    bidi_level: 0,
                    flags: AtomFlags::EMPTY,
                    glyphs: smallvec![],
                    natural_width: 0.0,
                    uncompressed_width: 0.0,
                    safe_to_break_before: true,
                    safe_to_concat_before: true,
                }],
                atoms,
                hyphen_breaks: vec![],
                total_width: 0.0,
            }
        })
        .collect();
    assert!(items.iter().all(WordNode::is_self_contained));
    let mut next = 0;
    for c in items.iter().flat_map(clusters_of) {
        assert_eq!(c.byte_range.start, next);
        next = c.byte_range.end;
    }
    assert_eq!(next, text_len);
}

#[test]
fn ligature_cluster_takes_justify_after_from_its_last_atom_only() {
    let atom_with = |off: usize, flags: AtomFlags| GraphemeAtom { flags, ..atom(off) };
    let node = |atoms: Vec<GraphemeAtom>, flags: AtomFlags| {
        let mut c = cluster(0, 0);
        c.atom_range = 0..atoms.len();
        c.byte_range = TextRange::new(0, atoms.len());
        c.flags = flags;
        WordNode::Word {
            run_index: 0,
            atoms,
            clusters: vec![c],
            hyphen_breaks: vec![],
            total_width: 0.0,
        }
    };
    let allow = AtomFlags::ALLOW_CJK_JUSTIFY_AFTER;

    // Only the last atom has the opportunity: the cluster has it.
    let atoms = vec![atom_with(0, AtomFlags::EMPTY), atom_with(1, allow)];
    let aggregated = AtomFlags::for_cluster(&atoms);
    assert!(aggregated.contains(allow));
    assert!(node(atoms.clone(), aggregated).is_self_contained());
    // A naive OR is the same here, but the cluster flags must equal the aggregate.
    assert!(!node(atoms, AtomFlags::EMPTY).is_self_contained());

    // Only the first atom has it: the opportunity lies *inside* the ligature,
    // so the cluster must not have it, and a naive OR is rejected.
    let atoms = vec![atom_with(0, allow), atom_with(1, AtomFlags::EMPTY)];
    let aggregated = AtomFlags::for_cluster(&atoms);
    assert!(!aggregated.contains(allow));
    assert!(node(atoms.clone(), aggregated).is_self_contained());
    assert!(!node(atoms.clone(), allow).is_self_contained());

    // Open punctuation from the first atom, close punctuation from the last.
    let atoms = vec![
        atom_with(0, AtomFlags::IS_OPEN_PUNCT),
        atom_with(1, AtomFlags::IS_CLOSE_PUNCT),
    ];
    let f = AtomFlags::for_cluster(&atoms);
    assert!(f.contains(AtomFlags::IS_OPEN_PUNCT | AtomFlags::IS_CLOSE_PUNCT));
    assert!(node(atoms, f).is_self_contained());
}
