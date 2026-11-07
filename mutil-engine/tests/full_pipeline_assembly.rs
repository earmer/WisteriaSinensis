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
        // Raw primary-font metrics: the frontend never resolves a line height.
        strut_metrics: FontMetrics {
            ascent: 12.0,
            descent: 4.0,
            ..Default::default()
        },
        strut_font_size: 16.0,
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

    // Layout policy and the validation context. The strut is resolved here, in
    // the middle-end, from the AST's raw metrics and the line-height policy.
    let options = LayoutOptions {
        hang_mode: HangMode {
            start: HangStartMode::None,
            end: HangEndMode::Allow,
        },
        ..Default::default()
    };
    let ctx = LineContext::from_options(
        &ast.strut_metrics,
        ast.strut_font_size,
        ast.base_direction,
        &options,
    );
    assert_eq!(ctx.strut.natural_line_height(), 16.0);
    let constraints = LayoutConstraints {
        wrap_width: 200.0,
        tab_stops: TabStops { step: 8.0 },
    };
    assert_eq!(
        check_layout_inputs(&ast, TEXT, &constraints, &options),
        Ok(ctx)
    );

    // ---- Shaping bridge: both spelling edits through the trait ----------
    let shaper: &dyn TextShaper = &NoopShaper;
    let r = &ast.runs[0];
    let run_atoms: Vec<_> = (0..TEXT.len()).map(atom).collect();
    let features = FontFeatures::new();
    // Each side reads only its own replacement and cut count.
    let trailing_edit = TrailingEdit::for_opportunity(&opportunity, '-');
    let leading_edit = LeadingEdit::for_opportunity(&opportunity);
    assert_eq!(
        trailing_edit,
        TrailingEdit::ReplacePrefix {
            replacement: "a".into(),
            cut_bytes: 1,
            hyphen: '-'
        }
    );
    assert_eq!(
        leading_edit,
        LeadingEdit::ReplaceSuffix {
            replacement: "tje".into(),
            cut_bytes: 2
        }
    );
    let mut out = Vec::new();
    // Regular break: line 0 ends at the break, line 1 starts after it.
    shaper.reshape_window(
        TEXT,
        r,
        &run_atoms,
        TextRange::new(0, 3),
        &features,
        LeadingEdit::None,
        trailing_edit.clone(),
        &mut out,
    );
    shaper.reshape_window(
        TEXT,
        r,
        &run_atoms,
        TextRange::new(3, 7),
        &features,
        leading_edit.clone(),
        TrailingEdit::None,
        &mut out,
    );
    // Extreme middle fragment: both edges edited in one call.
    shaper.reshape_window(
        TEXT,
        r,
        &run_atoms,
        TextRange::new(3, 7),
        &features,
        leading_edit,
        trailing_edit,
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
        tabs: vec![],
        decorations: vec![],
    };

    // Line 1: "atje" ␠ [box] ⇥ "go" and a right-hanging ",". The space occupies
    // width without any output entity (glue); the box and the tab are
    // `PositionedInlineBox` and `PositionedTab`. `visual_width` stops before ",".
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
        tabs: vec![PositionedTab {
            byte_range: TextRange::new(11, 12),
            x: box_x + BOX_W,
            width: TAB_W,
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
        line_height: ctx.strut.natural_line_height(),
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
        tabs: vec![],
        decorations: vec![],
    };

    for (i, l) in [&line0, &line1, &line2].into_iter().enumerate() {
        assert_eq!(l.validate(&ctx), Ok(()), "line {i}");
    }
    assert!(line1.hanging_right > 0.0 && line0.hanging_right == 0.0);
    assert_eq!((line1.visual_boxes.len(), line1.tabs.len()), (1, 1));
    // The tab follows the box exactly and ends where "go" begins.
    let (b, t) = (line1.visual_boxes[0], line1.tabs[0]);
    assert_eq!(b.x + b.width, t.x);
    assert_eq!(t.x + t.width, line1.visual_runs[0].glyphs[4].x);

    let mut para = PositionedParagraph::empty(ast.base_direction);
    para.size = Size {
        width: body_width,
        height: 48.0,
    };
    para.lines = vec![line0, line1, line2];

    // ---- Ownership: the output stands alone ------------------------------
    assert_send_sync_static::<ParagraphAst>();
    assert_send_sync_static::<PositionedParagraph>();
    assert_send_sync_static::<LeadingEdit>();
    assert_send_sync_static::<TrailingEdit>();
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

// ---- Counter-examples on the assembled line --------------------------------

/// A one-line fixture whose body is `[0, 40]`: glyph, inline box, tab, glyph.
fn small_line() -> PositionedLine {
    let g = |x: f32, bytes: (usize, usize)| PositionedGlyph {
        glyph_id: 1,
        x,
        y_offset: 0.0,
        advance: 10.0,
        byte_range: Some(TextRange::new(bytes.0, bytes.1)),
        is_synthesized: false,
        is_hanging: false,
    };
    PositionedLine {
        byte_range: TextRange::new(0, 4),
        visible_byte_end: 4,
        baseline_y: 12.0,
        ascent: 12.0,
        descent: 4.0,
        line_height: 16.0,
        visual_width: 40.0,
        hanging_left: 0.0,
        hanging_right: 0.0,
        is_justified: false,
        visual_runs: vec![VisualGlyphRun {
            font: FontRef::unscaled(FontId(1)),
            bidi_level: 0,
            byte_range: TextRange::new(0, 4),
            glyphs: vec![g(0.0, (0, 1)), g(30.0, (3, 4))],
        }],
        visual_boxes: vec![PositionedInlineBox {
            box_id: 7,
            byte_range: TextRange::new(1, 2),
            x: 10.0,
            y: 0.0,
            width: 10.0,
            ascent: 10.0,
            descent: 2.0,
        }],
        tabs: vec![PositionedTab {
            byte_range: TextRange::new(2, 3),
            x: 20.0,
            width: 10.0,
        }],
        caret_stops: vec![],
        decorations: vec![],
    }
}

fn small_ctx() -> LineContext {
    LineContext {
        strut: VerticalMetrics {
            ascent: 12.0,
            descent: 4.0,
            line_gap: 0.0,
        },
        ..Default::default()
    }
}

#[test]
fn glyph_box_and_tab_tile_the_body_and_validate() {
    let l = small_line();
    assert_eq!(l.validate(&small_ctx()), Ok(()));
    // Entities abut: glyph | box | tab | glyph = [0, 40].
    assert_eq!(l.visual_runs[0].glyphs[0].advance, l.visual_boxes[0].x);
    assert_eq!(l.visual_boxes[0].x + l.visual_boxes[0].width, l.tabs[0].x);
    assert_eq!(l.tabs[0].x + l.tabs[0].width, l.visual_runs[0].glyphs[1].x);
}

#[test]
fn inline_box_with_nan_dimensions_is_intercepted() {
    for field in 0..5 {
        let mut l = small_line();
        let b = &mut l.visual_boxes[0];
        match field {
            0 => b.x = f32::NAN,
            1 => b.y = f32::NAN,
            2 => b.width = f32::NAN,
            3 => b.ascent = f32::NAN,
            _ => b.descent = f32::NAN,
        }
        assert_eq!(
            l.validate(&small_ctx()),
            Err(LineInvariantError::InvalidInlineBox { index: 0 }),
            "field {field}"
        );
    }
}

#[test]
fn tab_beyond_visual_width_is_intercepted() {
    let mut l = small_line();
    l.tabs[0].x = 35.0; // ends at 45 > 40
    assert_eq!(
        l.validate(&small_ctx()),
        Err(LineInvariantError::TabOutsideBlock { index: 0 })
    );
    let mut l = small_line();
    l.tabs[0].x = -5.0;
    assert_eq!(
        l.validate(&small_ctx()),
        Err(LineInvariantError::TabOutsideBlock { index: 0 })
    );
}

#[test]
fn non_finite_or_negative_tab_is_intercepted_before_other_checks() {
    for (x, width) in [(f32::NAN, 10.0), (20.0, f32::INFINITY), (20.0, -1.0)] {
        let mut l = small_line();
        l.tabs[0] = PositionedTab {
            x,
            width,
            ..l.tabs[0]
        };
        // Also break containment elsewhere: finiteness is judged first.
        l.visual_runs[0].glyphs[1].x = 500.0;
        assert_eq!(
            l.validate(&small_ctx()),
            Err(LineInvariantError::InvalidTab { index: 0 })
        );
    }
}

#[test]
fn empty_line_rejects_tabs() {
    let mut l = small_line();
    l.byte_range = TextRange::empty_at(0);
    l.visible_byte_end = 0;
    l.visual_runs.clear();
    l.visual_boxes.clear();
    l.visual_width = 0.0;
    l.line_height = 16.0;
    assert_eq!(
        l.validate(&small_ctx()),
        Err(LineInvariantError::EmptyLineHasTabs)
    );
    l.tabs.clear();
    assert_eq!(l.validate(&small_ctx()), Ok(()));
}

#[test]
fn hang_mode_struct_gates_each_physical_edge() {
    // A comma hangs right of the body of an LTR line.
    let mut l = small_line();
    l.visual_runs[0].glyphs.push(PositionedGlyph {
        glyph_id: 2,
        x: 40.0,
        y_offset: 0.0,
        advance: 10.0,
        byte_range: Some(TextRange::new(4, 5)),
        is_synthesized: false,
        is_hanging: true,
    });
    l.byte_range = TextRange::new(0, 5);
    l.visible_byte_end = 5;
    l.hanging_right = 10.0;
    let with = |hang_mode, base_direction| LineContext {
        hang_mode,
        base_direction,
        ..small_ctx()
    };
    let end = |end| HangMode {
        start: HangStartMode::None,
        end,
    };
    let start_only = HangMode {
        start: HangStartMode::AllLines,
        end: HangEndMode::None,
    };
    for mode in [end(HangEndMode::Allow), end(HangEndMode::Force)] {
        assert_eq!(l.validate(&with(mode, BidiDirection::Ltr)), Ok(()));
    }
    for mode in [HangMode::NONE, start_only] {
        assert_eq!(
            l.validate(&with(mode, BidiDirection::Ltr)),
            Err(LineInvariantError::HangingNotAllowed {
                side: HangSide::Right
            })
        );
    }
    // In an RTL paragraph the right edge is the line *start*, so the same
    // geometry needs `start`, and `end` no longer permits it.
    assert_eq!(l.validate(&with(start_only, BidiDirection::Rtl)), Ok(()));
    assert_eq!(
        l.validate(&with(end(HangEndMode::Force), BidiDirection::Rtl)),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Right
        })
    );
    // Both edges together are expressible.
    assert_eq!(
        l.validate(&with(
            HangMode {
                start: HangStartMode::AllLines,
                end: HangEndMode::Force
            },
            BidiDirection::Rtl
        )),
        Ok(())
    );
}

/// A line "ab" + one preserved trailing space (bytes 2..3). `base` is the
/// paragraph direction; the space is placed at `x`.
fn trailing_space_line(x: f32) -> PositionedLine {
    let mut l = small_line();
    l.tabs.clear();
    l.visual_boxes.clear();
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 2;
    l.visual_width = 20.0;
    l.visual_runs[0].glyphs = vec![
        PositionedGlyph {
            glyph_id: 1,
            x: 0.0,
            y_offset: 0.0,
            advance: 20.0,
            byte_range: Some(TextRange::new(0, 2)),
            is_synthesized: false,
            is_hanging: false,
        },
        PositionedGlyph {
            glyph_id: 3,
            x,
            y_offset: 0.0,
            advance: 5.0,
            byte_range: Some(TextRange::new(2, 3)),
            is_synthesized: false,
            is_hanging: false,
        },
    ];
    l
}

#[test]
fn l1_trailing_whitespace_sits_on_the_far_edge_of_the_base_direction() {
    let ctx = |base_direction| LineContext {
        whitespace: HangingWhitespaceMode::Preserved,
        base_direction,
        ..small_ctx()
    };
    let right_of_body = trailing_space_line(20.0);
    let left_of_body = trailing_space_line(-5.0);
    // LTR paragraph: right of the body is right; left of it is wrong.
    assert_eq!(right_of_body.validate(&ctx(BidiDirection::Ltr)), Ok(()));
    assert_eq!(
        left_of_body.validate(&ctx(BidiDirection::Ltr)),
        Err(LineInvariantError::TrailingWhitespaceOnWrongSide { run: 0, glyph: 1 })
    );
    // RTL paragraph: mirrored.
    assert_eq!(left_of_body.validate(&ctx(BidiDirection::Rtl)), Ok(()));
    assert_eq!(
        right_of_body.validate(&ctx(BidiDirection::Rtl)),
        Err(LineInvariantError::TrailingWhitespaceOnWrongSide { run: 0, glyph: 1 })
    );
    // Whitespace in the middle of the body is wrong in both directions.
    let inside = trailing_space_line(10.0);
    for d in [BidiDirection::Ltr, BidiDirection::Rtl] {
        assert!(inside.validate(&ctx(d)).is_err());
    }
    // It never counts as hanging punctuation, on either side.
    assert_eq!(
        (left_of_body.hanging_left, right_of_body.hanging_right),
        (0.0, 0.0)
    );
}

#[test]
fn trailing_tab_follows_the_same_l1_rule() {
    let mut l = small_line();
    l.visual_boxes.clear();
    l.visual_runs[0].glyphs.truncate(1);
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 2;
    l.visual_width = 10.0;
    l.tabs = vec![PositionedTab {
        byte_range: TextRange::new(2, 3),
        x: 10.0,
        width: 8.0,
    }];
    let ctx = |whitespace, base_direction| LineContext {
        whitespace,
        base_direction,
        ..small_ctx()
    };
    use HangingWhitespaceMode::{Collapsible, Preserved};
    assert_eq!(l.validate(&ctx(Preserved, BidiDirection::Ltr)), Ok(()));
    assert_eq!(
        l.validate(&ctx(Preserved, BidiDirection::Rtl)),
        Err(LineInvariantError::TrailingTabOnWrongSide { index: 0 })
    );
    // Collapsible: a trailing tab must have collapsed to nothing.
    assert_eq!(
        l.validate(&ctx(Collapsible, BidiDirection::Ltr)),
        Err(LineInvariantError::TrailingTabNotCollapsed { index: 0 })
    );
    l.tabs[0].width = 0.0;
    assert_eq!(l.validate(&ctx(Collapsible, BidiDirection::Ltr)), Ok(()));
}

#[test]
fn context_follows_the_layout_options() {
    let options = LayoutOptions {
        strategy: LineBreakStrategy::GlobalOptimal,
        hang_mode: HangMode {
            start: HangStartMode::FirstLine,
            end: HangEndMode::Force,
        },
        line_height: LineHeight::Fixed(1.5),
        whitespace: HangingWhitespaceMode::Preserved,
        copy_soft_hyphen: SoftHyphenPolicy::Strip,
    };
    let raw = FontMetrics {
        ascent: 12.0,
        descent: 4.0,
        ..Default::default()
    };
    let c = LineContext::from_options(&raw, 10.0, BidiDirection::Rtl, &options);
    assert_eq!(c.hang_mode, options.hang_mode);
    assert_eq!(c.line_height, LineHeight::Fixed(1.5));
    assert_eq!(c.whitespace, HangingWhitespaceMode::Preserved);
    assert_eq!(c.base_direction, BidiDirection::Rtl);
    // Fixed(1.5) at size 10 resolves to a 15-unit strut with 12/4 raw: leading -1.
    assert_eq!(c.strut.natural_line_height(), 15.0);
    assert_eq!(c.line_index, 0);
    assert_eq!(c.for_line(3).line_index, 3);
    // FrontendOptions is the frontend's counterpart, with its own defaults.
    let f = FrontendOptions {
        default_locale: LocaleTag::new("ja-JP"),
        ..Default::default()
    };
    assert_eq!(f.default_locale.as_str(), "ja-JP");
}

#[test]
fn first_line_start_hanging_is_gated_by_line_index_and_direction() {
    // An opening bracket hangs at the physical left of line 0 (LTR).
    let mut l = small_line();
    l.byte_range = TextRange::new(0, 5);
    l.visible_byte_end = 5;
    l.visual_runs[0].glyphs.insert(
        0,
        PositionedGlyph {
            glyph_id: 9,
            x: -10.0,
            y_offset: 0.0,
            advance: 10.0,
            byte_range: Some(TextRange::new(4, 5)),
            is_synthesized: false,
            is_hanging: true,
        },
    );
    l.hanging_left = 10.0;
    let ctx = |start, dir, index| {
        LineContext {
            hang_mode: HangMode {
                start,
                end: HangEndMode::None,
            },
            base_direction: dir,
            ..small_ctx()
        }
        .for_line(index)
    };
    use HangStartMode::{AllLines, FirstLine, None as NoStart};
    assert_eq!(l.validate(&ctx(FirstLine, BidiDirection::Ltr, 0)), Ok(()));
    let denied = Err(LineInvariantError::HangingNotAllowed {
        side: HangSide::Left,
    });
    assert_eq!(l.validate(&ctx(FirstLine, BidiDirection::Ltr, 1)), denied);
    assert_eq!(l.validate(&ctx(NoStart, BidiDirection::Ltr, 0)), denied);
    assert_eq!(l.validate(&ctx(AllLines, BidiDirection::Ltr, 5)), Ok(()));
    // In an RTL paragraph the left edge is the line *end*: `start` never allows it.
    assert_eq!(l.validate(&ctx(AllLines, BidiDirection::Rtl, 0)), denied);

    // The paragraph-level check supplies the index per line.
    let para = PositionedParagraph {
        base_direction: BidiDirection::Ltr,
        size: Size::default(),
        lines: vec![l.clone(), l],
    };
    let c = ctx(FirstLine, BidiDirection::Ltr, 0);
    assert_eq!(
        para.validate(&c),
        Err((
            1,
            LineInvariantError::HangingNotAllowed {
                side: HangSide::Left
            }
        ))
    );
}

#[test]
fn out_of_bounds_clicks_converge_on_the_physical_extreme_stop() {
    let stop = |x: f32, offset: usize, level: u8, affinity| CaretStop {
        x,
        offset,
        affinity,
        level,
        line: 0,
    };
    use CaretAffinity::{Downstream, Upstream};
    // LTR paragraph "ab" + RTL run "אב" (bytes 2..6, two bytes per letter).
    // Visually: a | b | ב | א. The right edge is the *start* of the RTL run
    // (offset 2); visible_byte_end (6) is at the run's left edge, mid-line.
    let mut mixed = small_line();
    mixed.byte_range = TextRange::new(0, 6);
    mixed.visible_byte_end = 6;
    mixed.caret_stops = vec![
        stop(0.0, 0, 0, Downstream),
        stop(10.0, 1, 0, Downstream),
        stop(20.0, 2, 0, Downstream),
        stop(20.0, 6, 1, Upstream), // two stops share x = 20
        stop(30.0, 4, 1, Downstream),
        stop(40.0, 2, 1, Upstream),
    ];
    let right = mixed.extreme_caret_stop(HangSide::Right).unwrap();
    assert_eq!((right.x, right.offset), (40.0, 2));
    assert_ne!(right.offset, mixed.visible_byte_end);
    let left = mixed.extreme_caret_stop(HangSide::Left).unwrap();
    assert_eq!((left.x, left.offset), (0.0, 0));

    // Pure LTR line: the right extreme is (visible_byte_end, Upstream).
    let mut ltr = small_line();
    ltr.caret_stops = vec![
        stop(0.0, 0, 0, Downstream),
        stop(20.0, 2, 0, Downstream),
        stop(40.0, 4, 0, Upstream),
    ];
    let end = ltr.extreme_caret_stop(HangSide::Right).unwrap();
    assert_eq!((end.offset, end.affinity), (ltr.visible_byte_end, Upstream));

    // Pure RTL line in an RTL paragraph: logical end is the *left* extreme.
    let mut rtl = small_line();
    rtl.visual_runs[0].bidi_level = 1;
    rtl.caret_stops = vec![
        stop(0.0, 4, 1, Upstream),
        stop(20.0, 2, 1, Downstream),
        stop(40.0, 0, 1, Downstream),
    ];
    let end = rtl.extreme_caret_stop(HangSide::Left).unwrap();
    assert_eq!((end.offset, end.affinity), (rtl.visible_byte_end, Upstream));
    let start = rtl.extreme_caret_stop(HangSide::Right).unwrap();
    assert_eq!(start.offset, 0);

    // Tie-break by visual order: equal x picks the last stop on the right.
    let tie = [stop(5.0, 1, 0, Downstream), stop(5.0, 9, 1, Upstream)];
    let mut t = small_line();
    t.caret_stops = tie.to_vec();
    assert_eq!(t.extreme_caret_stop(HangSide::Right).unwrap().offset, 9);
    assert_eq!(t.extreme_caret_stop(HangSide::Left).unwrap().offset, 1);
    assert!(small_line().extreme_caret_stop(HangSide::Left).is_none());
}

#[test]
fn layout_paragraph_skeleton_assembles_and_returns_an_empty_paragraph() {
    let ast = ParagraphAst {
        base_direction: BidiDirection::Rtl,
        strut_metrics: FontMetrics {
            ascent: 8.0,
            descent: 2.0,
            ..Default::default()
        },
        strut_font_size: 10.0,
        runs: vec![],
        items: vec![],
    };
    let constraints = LayoutConstraints {
        wrap_width: 120.0,
        tab_stops: TabStops { step: 16.0 },
    };
    let options = LayoutOptions::default();
    let shaper: &dyn TextShaper = &NoopShaper;
    let p = layout_paragraph(&ast, "", shaper, &constraints, &options);
    assert_eq!(p, PositionedParagraph::empty(BidiDirection::Rtl));
    assert!(check_layout_inputs(&ast, "", &constraints, &options).is_ok());
}
