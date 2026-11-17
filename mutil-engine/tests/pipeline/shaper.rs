//! The HarfRust backend against the test font (spec §4, SHP-20, FNT-4).

use crate::common::*;
use mutil_engine::*;

#[test]
fn metrics_follow_the_font_tables_scaled_to_the_instance() {
    let m = SHAPER.font_metrics(instance(10.0));
    assert!(m.is_valid());
    // USE_TYPO_METRICS: typo ascender 1020, descender -300, gap 0.
    assert!(m.use_typo_metrics);
    for (actual, expected) in [
        (m.ascent, 10.2),
        (m.descent, 3.0),
        (m.line_gap, 0.0),
        // `post` and OS/2 are already positive towards line-over (FNT-4).
        (m.underline_position, -1.55),
        (m.strikeout_position, 3.2),
        // No vhea: half an em each side; no BASE: the fallbacks.
        (m.vertical_ascent, 5.0),
        (m.vertical_descent, 5.0),
        (m.ideographic_baseline_offset, -3.0),
    ] {
        assert_close(actual, expected);
    }
    let (id, primary) = SHAPER.resolve_primary_font_metrics(
        &[FontFamily::Generic(GenericFamily::Monospace)],
        FontStyle::default(),
        ScriptTag::LATIN,
        &LocaleTag::und(),
        10.0,
        &[],
    );
    assert_eq!((id, primary), (FontId(0), m));
}

#[test]
fn the_space_advance_is_the_space_glyph_scaled() {
    assert_eq!(
        SHAPER.space_advance(instance(10.0), GlyphOrientation::Horizontal),
        6.0
    );
    let scaled = FontInstanceRef {
        font: FontRef {
            font_id: FontId(0),
            scale: 2.0,
        },
        ..instance(10.0)
    };
    assert_eq!(
        SHAPER.space_advance(scaled, GlyphOrientation::Horizontal),
        12.0
    );
}

#[test]
fn shp5_horizontal_in_vertical_shapes_into_one_valid_cluster() {
    let text = "12";
    let atoms = vec![
        GraphemeAtom {
            orientation: GlyphOrientation::HorizontalInVertical,
            byte_range: TextRange::new(0, 1),
            script: ScriptTag::LATIN,
            bidi_level: BidiLevel::LTR,
            linebreak_class: LineBreakClass::Nu,
            flags: AtomFlags::EMPTY,
        },
        GraphemeAtom {
            orientation: GlyphOrientation::HorizontalInVertical,
            byte_range: TextRange::new(1, 2),
            script: ScriptTag::LATIN,
            bidi_level: BidiLevel::LTR,
            linebreak_class: LineBreakClass::Nu,
            flags: AtomFlags::EMPTY,
        },
    ];
    let run = ShapingRun {
        baseline_align: BaselineAlignment::Auto,
        letter_spacing: 0.0,
        word_spacing: 0.0,
        orientation: GlyphOrientation::HorizontalInVertical,
        features: FontFeatures::default(),
        byte_range: TextRange::new(0, 2),
        families: std::sync::Arc::from([FontFamily::Named(FAMILY.into())]),
        font_size: 10.0,
        style: FontStyle::default(),
        script: ScriptTag::LATIN,
        locale: LocaleTag::und(),
        bidi_level: BidiLevel::LTR,
        decoration: TextDecoration::default(),
        atom_range: 0..2,
        cluster_range: 0..1,
    };
    let mut clusters = Vec::new();
    let mut glyphs = Vec::new();
    SHAPER.shape_run(text, &run, &atoms, &mut clusters, &mut glyphs);

    assert_eq!(clusters.len(), 1, "{clusters:?}");
    let cluster = &clusters[0];
    assert_eq!(cluster.atom_range, 0..atoms.len());
    assert_eq!(cluster.byte_range, TextRange::new(0, 2));
    assert_eq!(cluster.orientation, GlyphOrientation::HorizontalInVertical);
    assert_eq!(cluster.glyph_range, 0..glyphs.len());
    assert_eq!(glyphs.len(), 2);
    assert!(cluster.safe_to_break_before);
    assert!(cluster.safe_to_concat_before);

    let natural_width = cluster.natural_width;
    let ast = ParagraphAst {
        writing_mode: WritingMode::VerticalRl,
        bidi_spans: Vec::new(),
        annotations: Vec::new(),
        base_direction: BidiDirection::Ltr,
        strut_metrics: SHAPER.font_metrics(instance(10.0)),
        strut_font_size: 10.0,
        runs: vec![run],
        atoms,
        clusters,
        glyphs,
        items: vec![
            WordNode::Box(BoxNode::Text {
                run_index: 0,
                byte_range: TextRange::new(0, 2),
                atom_range: 0..2,
                cluster_range: 0..1,
                natural_width,
            }),
            WordNode::Discretionary(DiscretionaryNode::paragraph_end(2)),
        ],
        decoration_spans: Vec::new(),
        atomic_spans: Vec::new(),
    };
    check_layout_inputs(&ast, text, &column(100.0), &LayoutOptions::default())
        .expect("G1 accepts HorizontalInVertical cluster");
}
