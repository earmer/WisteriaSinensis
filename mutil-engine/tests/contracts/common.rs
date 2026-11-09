//! Shared fixture builders: atoms, clusters, shaping runs, ASTs, contexts and
//! positioned lines.

use core::ops::Range;
use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;

pub fn end(n: usize) -> WordNode {
    WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::empty_at(n), true))
}

/// Appends a text box over `atoms` and `clusters` to body run `run` of `ast`
/// and returns its node. The one place that computes storage ranges: the atoms
/// and clusters go at the end of the run's slice of `ast.atoms` / `ast.clusters`,
/// the run's ranges grow, later runs and their boxes shift, and the node gets
/// the paragraph-level `atom_range` / `cluster_range`.
///
/// `clusters` carry `atom_range`s relative to `atoms` (the first cluster
/// starts at 0); they are stored run-local. The caller places the node in
/// `ast.items`, in the order the run's boxes were pushed.
pub fn push_text(
    ast: &mut ParagraphAst,
    run: usize,
    atoms: Vec<GraphemeAtom>,
    clusters: Vec<ShapedCluster>,
) -> WordNode {
    let (atom_at, cluster_at) = (
        ast.runs[run].atom_range.end,
        ast.runs[run].cluster_range.end,
    );
    let run_local = atom_at - ast.runs[run].atom_range.start;
    let (n_atoms, n_clusters) = (atoms.len(), clusters.len());
    let byte_range = match (atoms.first(), atoms.last()) {
        (Some(first), Some(last)) => TextRange::new(first.byte_range.start, last.byte_range.end),
        _ => TextRange::EMPTY,
    };
    let natural_width = clusters.iter().map(|c| c.natural_width).sum();
    ast.atoms.splice(atom_at..atom_at, atoms);
    ast.clusters.splice(
        cluster_at..cluster_at,
        clusters.into_iter().map(|mut c| {
            c.atom_range = c.atom_range.start + run_local..c.atom_range.end + run_local;
            c
        }),
    );
    ast.runs[run].atom_range.end += n_atoms;
    ast.runs[run].cluster_range.end += n_clusters;
    for later in ast.runs.iter_mut().skip(run + 1) {
        later.atom_range = later.atom_range.start + n_atoms..later.atom_range.end + n_atoms;
        later.cluster_range =
            later.cluster_range.start + n_clusters..later.cluster_range.end + n_clusters;
    }
    for item in &mut ast.items {
        if let WordNode::Box(BoxNode::Text {
            run_index,
            atom_range,
            cluster_range,
            ..
        }) = item
        {
            if *run_index > run {
                *atom_range = atom_range.start + n_atoms..atom_range.end + n_atoms;
                *cluster_range = cluster_range.start + n_clusters..cluster_range.end + n_clusters;
            }
        }
    }
    WordNode::Box(BoxNode::Text {
        run_index: run,
        byte_range,
        atom_range: atom_at..atom_at + n_atoms,
        cluster_range: cluster_at..cluster_at + n_clusters,
        natural_width,
    })
}

/// A text box over the one-byte atoms `s..e` of run 0, one unit-wide cluster
/// per atom, appended to `ast`'s storage.
pub fn boxed(ast: &mut ParagraphAst, s: usize, e: usize, atomic: bool) -> WordNode {
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
    push_text(ast, 0, atoms, clusters)
}

/// "x" followed by `n` one-byte boxes `"a"` joined by `Hyphenate`
/// discretionaries, all in run 0: the hyphen chain spans bytes `1..n + 1`, and a
/// `Direct` break keeps the `x` box out of it. The opportunity between box `k`
/// and box `k + 1` (offset `k + 2`) takes the cuts `cut(offset, n)` returns as
/// `(prefix_cut_bytes, suffix_cut_bytes)`.
pub fn hyphen_chain_ast(
    n: usize,
    cut: impl Fn(usize, usize) -> (u32, u32),
) -> (String, ParagraphAst) {
    let t = format!("x{}", "a".repeat(n));
    let mut a = ast(&t);
    rebuild_items(&mut a, |a| {
        let mut items = vec![boxed(a, 0, 1, false)];
        items.push(WordNode::Discretionary(DiscretionaryNode::direct(1)));
        for k in 0..n {
            if k > 0 {
                let offset = k + 1;
                let (prefix_cut_bytes, suffix_cut_bytes) = cut(offset, n);
                items.push(WordNode::Discretionary(DiscretionaryNode {
                    byte_range: TextRange::empty_at(offset),
                    penalty: DiscretionaryNode::HYPHEN_PENALTY,
                    flagged: true,
                    pre_break_width: 5.0,
                    post_break_width: 0.0,
                    no_break_width: 0.0,
                    action: BreakAction::Hyphenate(HyphenOpportunity {
                        byte_offset: offset,
                        prefix_replacement: None,
                        suffix_replacement: None,
                        prefix_cut_bytes,
                        suffix_cut_bytes,
                    }),
                    cloned_spans: Default::default(),
                }));
            }
            items.push(boxed(a, k + 1, k + 2, false));
        }
        items.push(end(t.len()));
        items
    });
    (t, a)
}

/// Empties the paragraph's items and text storage (atoms, clusters and every
/// body run's ranges), runs `build` to refill them through [`push_text`] /
/// [`boxed`], and installs the items it returns.
pub fn rebuild_items(
    ast: &mut ParagraphAst,
    build: impl FnOnce(&mut ParagraphAst) -> Vec<WordNode>,
) {
    ast.items.clear();
    clear_text(ast);
    ast.items = build(ast);
}

/// Drops all atoms and clusters and empties every body run's ranges. The items
/// are left alone: use after replacing text boxes with other nodes.
pub fn clear_text(ast: &mut ParagraphAst) {
    ast.atoms.clear();
    ast.clusters.clear();
    for run in &mut ast.runs {
        run.atom_range = 0..0;
        run.cluster_range = 0..0;
    }
}

/// Lets `edit` change the atoms and clusters of the text box at `ast.items[item]`
/// (add, drop or replace entries; cluster `atom_range`s are run-local, as
/// stored) and repairs the ranges of the box, its run, later runs and later
/// boxes so the storage still tiles.
pub fn edit_text_box(
    ast: &mut ParagraphAst,
    item: usize,
    edit: impl FnOnce(&mut Vec<GraphemeAtom>, &mut Vec<ShapedCluster>),
) {
    let WordNode::Box(BoxNode::Text {
        run_index,
        atom_range,
        cluster_range,
        ..
    }) = &ast.items[item]
    else {
        panic!("items[{item}] is not a text box");
    };
    let (run, atom_range, cluster_range) = (*run_index, atom_range.clone(), cluster_range.clone());
    let mut atoms = ast.atoms[atom_range.clone()].to_vec();
    let mut clusters = ast.clusters[cluster_range.clone()].to_vec();
    edit(&mut atoms, &mut clusters);
    let delta_atoms = atoms.len() as isize - atom_range.len() as isize;
    let delta_clusters = clusters.len() as isize - cluster_range.len() as isize;
    ast.atoms.splice(atom_range.clone(), atoms);
    ast.clusters.splice(cluster_range.clone(), clusters);
    let shift = |r: &mut Range<usize>, by: isize| {
        *r = r.start.wrapping_add_signed(by)..r.end.wrapping_add_signed(by);
    };
    ast.runs[run].atom_range.end = ast.runs[run]
        .atom_range
        .end
        .wrapping_add_signed(delta_atoms);
    ast.runs[run].cluster_range.end = ast.runs[run]
        .cluster_range
        .end
        .wrapping_add_signed(delta_clusters);
    for later in ast.runs.iter_mut().skip(run + 1) {
        shift(&mut later.atom_range, delta_atoms);
        shift(&mut later.cluster_range, delta_clusters);
    }
    for (i, node) in ast.items.iter_mut().enumerate() {
        if let WordNode::Box(BoxNode::Text {
            atom_range: a,
            cluster_range: c,
            ..
        }) = node
        {
            if i == item {
                a.end = a.end.wrapping_add_signed(delta_atoms);
                c.end = c.end.wrapping_add_signed(delta_clusters);
            } else if a.start >= atom_range.end && c.start >= cluster_range.end {
                shift(a, delta_atoms);
                shift(c, delta_clusters);
            }
        }
    }
}

pub fn ast(t: &str) -> ParagraphAst {
    let mut a = ParagraphAst {
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
            atom_range: 0..0,
            cluster_range: 0..0,
        }],
        atoms: vec![],
        clusters: vec![],
        items: vec![],
        decoration_spans: vec![],
        atomic_spans: vec![],
    };
    let text = boxed(&mut a, 0, t.len(), false);
    a.items = vec![text, end(t.len())];
    a
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

/// The one slot of a line laid out in a context whose wrap width is
/// `wrap_width`: it starts at 0 and holds the whole `visual_width`.
pub fn single_slot(visual_width: f32, wrap_width: f32) -> smallvec::SmallVec<[PositionedSlot; 1]> {
    smallvec![PositionedSlot {
        start: 0.0,
        visual_width,
        constraint_width: wrap_width,
    }]
}

/// Sets the width of a single-slot line, keeping its slot in step.
pub fn set_visual_width(line: &mut PositionedLine, width: f32) {
    line.visual_width = width;
    line.slots[0].visual_width = width;
}

pub fn line() -> PositionedLine {
    PositionedLine {
        slots: single_slot(100., f32::INFINITY),
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
    for atom in &mut a.atoms {
        atom.orientation = orientation;
        atom.bidi_level = level;
    }
    for cluster in &mut a.clusters {
        cluster.orientation = orientation;
        cluster.bidi_level = level;
    }
}

pub fn annotation(base: TextRange, side: AnnotationSide) -> AnnotationSpan {
    let mut a = ast("x");
    let atoms = std::mem::take(&mut a.atoms);
    let clusters = std::mem::take(&mut a.clusters);
    let mut run = a.runs.remove(0);
    run.atom_range = 0..atoms.len();
    run.cluster_range = 0..clusters.len();
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
        run,
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
        base_over: 8.0,
        base_under: 2.0,
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
/// `base` of `text`; `atom_range` is relative to the node's own atoms (what
/// [`push_text`] expects; it stores them run-local).
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
        atom_range: 0..0,
        cluster_range: 0..0,
    }
}

pub fn assert_send_sync_static<T: Send + Sync + 'static>() {}
