//! Shared fixture builders: atoms, clusters, shaping runs, ASTs, contexts and
//! positioned lines.

use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;

pub fn end(n: usize) -> WordNode {
    WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::empty_at(n), true))
}

pub fn boxed(s: usize, e: usize, atomic: bool) -> WordNode {
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

pub fn ast(t: &str) -> ParagraphAst {
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

pub fn check(a: &ParagraphAst, t: &str) -> Result<LineContext, LayoutInputError> {
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

pub fn line() -> PositionedLine {
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

pub fn ctx() -> LineContext {
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

pub fn glyph(x: f32, advance: f32, hanging: bool) -> PositionedGlyph {
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

pub fn run(g: PositionedGlyph) -> VisualGlyphRun {
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

pub fn orient(a: &mut ParagraphAst, mode: WritingMode, orientation: GlyphOrientation, level: u8) {
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

pub fn annotation(base: TextRange, side: AnnotationSide) -> AnnotationSpan {
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

pub fn two_slots() -> (PositionedLine, LineContext) {
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

pub fn positioned_annotation() -> PositionedAnnotationRun {
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

/// Fixed advance of every fixture glyph.
pub const ADV: f32 = 10.0;

/// One Latin atom covering the single byte at `offset`.
pub fn atom(offset: usize) -> GraphemeAtom {
    GraphemeAtom {
        orientation: GlyphOrientation::Horizontal,
        byte_range: TextRange::new(offset, offset + 1),
        script: ScriptTag::LATIN,
        bidi_level: 0,
        linebreak_class: LineBreakClass::Al,
        flags: AtomFlags::EMPTY,
    }
}

/// One-glyph cluster for the atom at `local` index of a node starting at byte
/// `base` of `text`; `atom_range` is relative to the node's own atoms.
pub fn cluster(text: &str, base: usize, local: usize) -> ShapedCluster {
    let off = base + local;
    ShapedCluster {
        orientation: GlyphOrientation::Horizontal,
        byte_range: TextRange::new(off, off + 1),
        atom_range: local..local + 1,
        font: FontRef::unscaled(FontId(1)),
        bidi_level: 0,
        flags: AtomFlags::EMPTY,
        glyphs: smallvec![RawShapedGlyph {
            glyph_id: u32::from(text.as_bytes()[off]),
            cluster: off as u32,
            x_advance: ADV,
            ..Default::default()
        }],
        natural_width: ADV,
        safe_to_break_before: true,
        safe_to_concat_before: true,
    }
}

/// A 16-unit sans-serif Latin run covering all of `text`.
pub fn shaping_run(text: &str, locale: &str) -> ShapingRun {
    ShapingRun {
        word_spacing: 0.0,
        baseline_align: BaselineAlignment::Auto,
        letter_spacing: 0.0,
        orientation: GlyphOrientation::Horizontal,
        features: Default::default(),
        byte_range: TextRange::new(0, text.len()),
        families: Arc::from([FontFamily::Generic(GenericFamily::SansSerif)]),
        font_size: 16.0,
        style: FontStyle::default(),
        script: ScriptTag::LATIN,
        locale: LocaleTag::new(locale),
        bidi_level: 0,
        decoration: TextDecoration::default(),
    }
}

pub fn assert_send_sync_static<T: Send + Sync + 'static>() {}
