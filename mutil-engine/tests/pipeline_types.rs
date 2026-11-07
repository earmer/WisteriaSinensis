//! Phase 0 validation: the types of all three tiers fit together.
//!
//! No typesetting happens here. Every cluster, width and position below is a
//! hand-written fixture (fixed 10-unit advances) whose only purpose is to
//! exercise ownership, visibility and trait closure from *outside* the crate.

use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;

const TEXT: &str = "Hello world";
const ADVANCE: f32 = 10.0;

fn run() -> ShapingRun {
    ShapingRun {
        byte_range: TextRange::new(0, TEXT.len()),
        families: Arc::from([FontFamily::Generic(GenericFamily::SansSerif)]),
        font_size: 16.0,
        style: FontStyle::default(),
        script: ScriptTag::LATIN,
        locale: LocaleTag::new("en-GB"),
        bidi_level: 0,
        decoration: TextDecoration::default(),
    }
}

/// One atom per ASCII letter, at its absolute byte offset.
fn atom(offset: usize) -> GraphemeAtom {
    GraphemeAtom {
        byte_range: TextRange::new(offset, offset + 1),
        script: ScriptTag::LATIN,
        bidi_level: 0,
        linebreak_class: LineBreakClass::Al,
        flags: AtomFlags::EMPTY,
    }
}

/// Fixture cluster for the atom at `local` index of a node starting at byte `base`.
fn cluster(base: usize, local: usize) -> ShapedCluster {
    let off = base + local;
    ShapedCluster {
        byte_range: TextRange::new(off, off + 1),
        atom_range: local..local + 1, // relative to the node's own atoms
        font: FontRef::unscaled(FontId(1)),
        bidi_level: 0,
        flags: AtomFlags::EMPTY,
        glyphs: smallvec![RawShapedGlyph {
            glyph_id: TEXT.as_bytes()[off] as u32,
            cluster: off as u32,
            x_advance: ADVANCE,
            x_offset: 0.0,
            y_offset: 0.0,
        }],
        natural_width: ADVANCE,
        uncompressed_width: ADVANCE,
        safe_to_break_before: true,
        safe_to_concat_before: true,
    }
}

fn word(range: TextRange) -> WordNode {
    let atoms: Vec<_> = range.as_range().map(atom).collect();
    let clusters: Vec<_> = (0..atoms.len()).map(|i| cluster(range.start, i)).collect();
    let total_width = clusters.iter().map(|c| c.natural_width).sum();
    WordNode::Word {
        run_index: 0,
        atoms,
        clusters,
        hyphen_breaks: Vec::new(),
        total_width,
    }
}

fn assert_send_sync_static<T: Send + Sync + 'static>() {}

#[test]
fn test_pipeline_types_compile() {
    // Frontend: a pseudo ParagraphAst of Word, Glue, Word.
    let ast = ParagraphAst {
        base_direction: BidiDirection::Ltr,
        strut_metrics: VerticalMetrics {
            ascent: 12.0,
            descent: 4.0,
            line_gap: 0.0,
        },
        runs: vec![run()],
        items: vec![
            word(TextRange::new(0, 5)),
            WordNode::Glue {
                byte_range: TextRange::new(5, 6),
                kind: GlueKind::WordSpace,
                natural_width: 4.0,
                max_stretch: 2.0,
                max_shrink: 1.0,
            },
            word(TextRange::new(6, 11)),
        ],
    };

    // Shaping bridge: the no-op backend, through a trait object.
    let shaper: &dyn TextShaper = &NoopShaper;
    let r = &ast.runs[0];
    let (font_id, metrics) =
        shaper.resolve_primary_font_metrics(&r.families, r.style, r.script, &r.locale, r.font_size);
    assert_eq!(font_id, FontId(0));
    assert_eq!(metrics, FontMetrics::default());

    let mut out = Vec::new();
    shaper.shape_run(
        TEXT,
        r,
        &[],
        &r.families,
        r.font_size,
        &FontFeatures::new(),
        &mut out,
    );
    shaper.reshape_window(
        TEXT,
        r,
        TextRange::new(0, 5),
        BoundaryEdit::AppendHyphen { hyphen: '-' },
        &mut out,
    );
    assert!(out.is_empty());
    assert!(
        shaper
            .ligature_carets(FontRef::unscaled(font_id), 0, r.direction())
            .is_empty()
    );

    // Middle-end: assemble the fixture clusters into a PositionedParagraph.
    let mut line_glyphs = Vec::new();
    let mut pen = 0.0_f32; // fixture bookkeeping only, not a layout algorithm
    for item in &ast.items {
        if let WordNode::Word { clusters, .. } = item {
            for c in clusters {
                let g = c.glyphs[0];
                line_glyphs.push(PositionedGlyph {
                    glyph_id: g.glyph_id,
                    x: pen,
                    y_offset: g.y_offset,
                    advance: g.x_advance,
                    byte_range: Some(c.byte_range),
                    is_synthesized: false,
                });
                pen += c.natural_width;
            }
        }
    }
    // A generated hyphen: no source bytes, flagged synthesized.
    line_glyphs.push(PositionedGlyph {
        glyph_id: b'-' as u32,
        x: pen,
        y_offset: 0.0,
        advance: ADVANCE,
        byte_range: None,
        is_synthesized: true,
    });
    assert_eq!(line_glyphs.len(), 11);
    assert!(
        line_glyphs
            .last()
            .is_some_and(|g| g.is_synthesized && g.byte_range.is_none())
    );

    let line = PositionedLine {
        byte_range: TextRange::new(0, TEXT.len()),
        visible_byte_end: TEXT.len(),
        baseline_y: ast.strut_metrics.ascent,
        ascent: ast.strut_metrics.ascent,
        descent: ast.strut_metrics.descent,
        line_height: ast.strut_metrics.natural_line_height(),
        visual_width: pen + ADVANCE,
        is_justified: false,
        visual_runs: vec![VisualGlyphRun {
            font: FontRef::unscaled(FontId(1)),
            bidi_level: 0,
            byte_range: TextRange::new(0, TEXT.len()),
            glyphs: line_glyphs,
        }],
        caret_stops: vec![CaretStop {
            x: 0.0,
            offset: 0,
            affinity: CaretAffinity::Downstream,
            level: 0,
            line: 0,
        }],
        decorations: vec![],
    };

    let mut para = PositionedParagraph::empty(ast.base_direction);
    assert!(para.lines.is_empty());
    para.size = Size {
        width: line.visual_width,
        height: line.line_height,
    };
    para.lines.push(line);
    assert_eq!(para.lines.len(), 1);

    // Interaction types.
    let cursor = CursorPosition {
        byte_offset: 5,
        affinity: CaretAffinity::Upstream,
    };
    let sel = SelectionRect {
        rect: Rect::new(0.0, 0.0, 50.0, 16.0),
        line: 0,
        level: 0,
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
    let w = word(TextRange::new(6, 11));
    let WordNode::Word {
        atoms,
        clusters,
        total_width,
        ..
    } = &w
    else {
        unreachable!()
    };
    assert_eq!(*total_width, 5.0 * ADVANCE);
    for c in clusters {
        let covered = &atoms[c.atom_range.clone()];
        assert_eq!(covered.len(), 1);
        assert_eq!(covered[0].byte_range, c.byte_range);
        assert!(c.safety_flags_consistent());
    }
}

#[test]
fn every_word_node_variant_is_constructible() {
    let nodes = [
        word(TextRange::new(0, 5)),
        WordNode::PunctuationRun {
            run_index: 0,
            atoms: vec![atom(0)],
            clusters: vec![cluster(0, 0)],
            bind: PunctBind::default(),
        },
        WordNode::Glue {
            byte_range: TextRange::empty_at(5),
            kind: GlueKind::CjkAutospace,
            natural_width: 4.0,
            max_stretch: 8.0,
            max_shrink: 2.0,
        },
        WordNode::Tab {
            byte_range: TextRange::new(5, 6),
        },
        WordNode::HardBreak {
            byte_range: TextRange::new(6, 7),
        },
        WordNode::InlineBox {
            byte_range: TextRange::new(7, 8),
            box_id: 7,
            width: 12.0,
            ascent: 10.0,
            descent: 2.0,
        },
    ];
    // Exhaustive: adding a variant without updating this match fails to compile.
    for n in &nodes {
        match n {
            WordNode::Word { .. }
            | WordNode::PunctuationRun { .. }
            | WordNode::Glue { .. }
            | WordNode::Tab { .. }
            | WordNode::HardBreak { .. }
            | WordNode::InlineBox { .. } => {}
        }
    }
    assert_eq!(nodes.len(), 6);
}

#[test]
fn hyphen_opportunity_carries_prefix_and_suffix_replacements() {
    let h = HyphenOpportunity {
        byte_offset: 3,
        prefix_replacement: Some("a".into()),
        suffix_replacement: Some("tje".into()),
        cut_bytes: 1,
        penalty: 50,
    };
    assert_eq!(h.prefix_replacement.as_deref(), Some("a"));
    assert_eq!(h.suffix_replacement.as_deref(), Some("tje"));
}
