//! The `LineOnly` profile (spec SHP-18, SHP-24) over a line backend that
//! shapes whole strings with HarfRust, the way a platform text system does:
//! one face per stretch, no context, no language, no break-safety flags.

use crate::common::*;
use mutil_engine::*;
use read_fonts::model::Font;
use std::sync::{Arc, LazyLock};

/// A line backend over the faces of [`SCRIPT_SHAPER`], under the same ids,
/// whose metrics it answers.
struct HarfRustLines {
    faces: Vec<(&'static str, Font)>,
}

static LINES: LazyLock<LineOnlyShaper<HarfRustLines>> = LazyLock::new(|| {
    let faces = std::iter::once((FAMILY, FONT_BYTES))
        .chain(SCRIPT_FONTS.iter().copied())
        .map(|(family, bytes)| {
            let data: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(bytes);
            (family, Font::new(data, 0).expect("the test font parses"))
        })
        .collect();
    LineOnlyShaper::new(HarfRustLines { faces })
});

impl HarfRustLines {
    fn maps(&self, id: usize, c: char) -> bool {
        self.faces[id].1.charmap().map_unicode(c).is_some()
    }
}

impl LineShaper for HarfRustLines {
    fn shape_line(
        &self,
        text: &str,
        families: &[FontFamily],
        _style: FontStyle,
        font_size: f32,
        features: &FontFeatures,
    ) -> Vec<LineGlyph> {
        let mut order: Vec<usize> = families
            .iter()
            .filter_map(|f| match f {
                FontFamily::Named(name) => self.faces.iter().position(|(n, _)| **n == **name),
                FontFamily::Generic(_) => None,
            })
            .collect();
        let rest: Vec<usize> = (0..self.faces.len())
            .filter(|i| !order.contains(i))
            .collect();
        order.extend(rest);
        // One face per stretch: a character keeps the face before it when
        // that face maps it.
        let mut stretches: Vec<(usize, usize)> = Vec::new();
        for (offset, c) in text.char_indices() {
            let previous = stretches.last().map(|&(_, id)| id);
            let face = previous
                .filter(|&id| self.maps(id, c))
                .or_else(|| order.iter().copied().find(|&id| self.maps(id, c)))
                .or(previous)
                .unwrap_or(order[0]);
            if previous != Some(face) {
                stretches.push((offset, face));
            }
        }
        let tags: Vec<harfrust::Feature> = features
            .as_slice()
            .iter()
            .map(|f| harfrust::Feature::new(harfrust::Tag::new(&f.tag), f.value, ..))
            .collect();
        let mut out = Vec::new();
        for (k, &(start, id)) in stretches.iter().enumerate() {
            let end = stretches.get(k + 1).map_or(text.len(), |&(s, _)| s);
            let font = &self.faces[id].1;
            let mut buffer = harfrust::Buffer::new();
            for (offset, c) in text[start..end].char_indices() {
                buffer.push(c as u32, offset as u32);
            }
            buffer.set_direction(harfrust::Direction::LeftToRight);
            buffer.guess_segment_properties();
            let options = harfrust::ShapeOptions::new().features(&tags);
            harfrust::shape(&harfrust::ShaperFont::new(font), &mut buffer, options)
                .expect("the line shapes");
            let scale = font_size / f32::from(font.metrics().units_per_em);
            out.extend(
                buffer
                    .glyph_infos()
                    .iter()
                    .zip(buffer.glyph_positions())
                    .map(|(info, pos)| LineGlyph {
                        font: FontId(id as u32),
                        glyph_id: info.glyph_id,
                        byte_index: start + info.cluster as usize,
                        x_advance: pos.x_advance as f32 * scale,
                        x_offset: pos.x_offset as f32 * scale,
                        y_offset: pos.y_offset as f32 * scale,
                    }),
            );
        }
        out
    }

    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics {
        SCRIPT_SHAPER.font_metrics(instance)
    }

    fn resolve_primary_font_metrics(
        &self,
        families: &[FontFamily],
        style: FontStyle,
        script: ScriptTag,
        locale: &LocaleTag,
        font_size: f32,
        variations: &[FontVariation],
    ) -> (FontId, FontMetrics) {
        SCRIPT_SHAPER
            .resolve_primary_font_metrics(families, style, script, locale, font_size, variations)
    }

    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32 {
        SCRIPT_SHAPER.space_advance(instance, orientation)
    }
}

fn compile_lines(text: &str, families: &[&str]) -> ParagraphAst {
    let style = script_style(families, 10.0);
    compile_paragraph(&input(text, &style), &FrontendOptions::default(), &*LINES)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

#[test]
fn clusters_are_rebuilt_from_byte_indices_with_conservative_seams() {
    let text = "office fit";
    let lines = compile_lines(text, &["Noto Serif"]);
    let full = compile_in(text, &["Noto Serif"]);
    let ranges = |ast: &ParagraphAst| -> Vec<(usize, usize)> {
        ast.clusters
            .iter()
            .map(|c| (c.byte_range.start(), c.byte_range.end()))
            .collect()
    };
    // The same partition as the full contract, the ffi ligature included.
    assert_eq!(ranges(&lines), ranges(&full));
    assert!(
        lines
            .clusters
            .iter()
            .any(|c| &text[c.byte_range.as_range()] == "ffi" && c.atom_count() == 3)
    );
    // Safe only at the start of the shaped text and after the space.
    let safe: Vec<&str> = lines
        .clusters
        .iter()
        .filter(|c| c.safe_to_break_before)
        .map(|c| &text[c.byte_range.as_range()])
        .collect();
    assert_eq!(safe, ["o", "fi"]);
    for c in &lines.clusters {
        assert_eq!(c.safe_to_concat_before, c.safe_to_break_before);
    }
}

#[test]
fn the_profile_has_no_ligature_carets_and_divides_ligatures_equally() {
    assert_eq!(
        LineOnlyShaper::<HarfRustLines>::PROFILE,
        ShaperProfile::LineOnly
    );
    let text = "office";
    let ast = compile_lines(text, &["Noto Serif"]);
    let ffi = &ast.clusters[1];
    assert_eq!(&text[ffi.byte_range.as_range()], "ffi");
    let glyph = ast.glyphs[ffi.glyph_range.start].glyph_id;
    let instance = FontInstanceRef {
        font: ffi.font,
        font_size: 10.0,
        variations: &[],
    };
    assert!(
        LINES
            .ligature_carets(instance, glyph, Direction::Ltr)
            .is_empty()
    );
    let p = layout_paragraph(
        &ast,
        text,
        &*LINES,
        &column(200.0),
        &LayoutOptions::default(),
    )
    .expect("the paragraph lays out");
    ValidatedLayout::new(&p, text).expect("the output gate accepts it");
    let start = ast.clusters[0].natural_width;
    let third = ffi.natural_width / 3.0;
    let inner: Vec<f32> = p.lines[0]
        .caret_stops
        .iter()
        .filter(|s| (2..=3).contains(&s.offset))
        .map(|s| s.x)
        .collect();
    assert_eq!(inner.len(), 2);
    assert_close(inner[0], start + third);
    assert_close(inner[1], start + 2.0 * third);
}

#[test]
fn mixed_scripts_lay_out_and_wrap_through_the_profile() {
    for (text, families, width) in [
        ("office fit and affine", &["Noto Serif"][..], 40.0),
        ("日本語の「テスト」、東京", &["Noto Sans JP"][..], 35.0),
        ("नमस्ते क्षत्रिय", &["Noto Sans Devanagari"][..], 30.0),
        ("AVAVAVAVAVAVAV", &["Noto Serif"][..], 25.0),
    ] {
        let ast = compile_lines(text, families);
        let p = layout_paragraph(
            &ast,
            text,
            &*LINES,
            &column(width),
            &LayoutOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{text:?}: {e}"));
        if let Err((line, error)) = ValidatedLayout::new(&p, text) {
            panic!("{text:?}: line {line}: {error}");
        }
        assert!(p.lines.len() > 1, "{text:?} wraps at {width}");
    }
}

#[test]
fn a_window_takes_its_edits_as_synthesized_glyphs() {
    let text = "office";
    let ast = compile_lines(text, &["Noto Serif"]);
    let run = &ast.runs[0];
    let atoms = ast.run_atoms(0);
    let mut clusters = Vec::new();
    let mut glyphs = Vec::new();
    // "off" + hyphen: the window's atoms stay partitioned, the hyphen is the
    // last cluster's last glyph.
    LINES.reshape_window(
        text,
        run,
        atoms,
        TextRange::new(0, 3),
        LeadingEdit::None,
        TrailingEdit::AppendHyphen { hyphen: '-' },
        &mut clusters,
        &mut glyphs,
    );
    let covered: Vec<&str> = clusters
        .iter()
        .map(|c| &text[c.byte_range.as_range()])
        .collect();
    assert_eq!(covered, ["o", "ff"]);
    assert!(!clusters[0].safe_to_concat_before);
    let last = clusters.last().expect("clusters");
    assert_eq!(last.glyph_range.end, glyphs.len());
    assert!(glyphs.last().expect("glyphs").is_synthesized);
    assert_eq!(glyphs.iter().filter(|g| g.is_synthesized).count(), 1);
}

#[test]
fn word_spaces_are_sized_by_the_frontend_not_by_a_cluster() {
    // The line backend shapes the whole run, spaces included; a space's
    // glyph belongs to no atom, so no cluster may carry its advance.
    for (text, family) in [
        ("office fit", "Noto Serif"),
        ("a b\tc", "Noto Serif"),
        ("日本 語", "Noto Sans JP"),
    ] {
        let lines = compile_lines(text, &[family]);
        let full = compile_in(text, &[family]);
        assert_eq!(lines.clusters.len(), full.clusters.len(), "{text:?}");
        for (a, b) in lines.clusters.iter().zip(&full.clusters) {
            let s = &text[a.byte_range.as_range()];
            assert_close(a.natural_width, b.natural_width);
            assert_eq!(a.glyph_range.len(), b.glyph_range.len(), "{text:?}: {s:?}");
        }
    }
}

#[test]
fn shp5_line_only_horizontal_in_vertical_shapes_into_one_valid_cluster() {
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
        families: Arc::from([FontFamily::Named(FAMILY.into())]),
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
    LINES.shape_run(text, &run, &atoms, &mut clusters, &mut glyphs);

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
        strut_metrics: LINES.font_metrics(instance(10.0)),
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
        .expect("G1 accepts LineOnly HorizontalInVertical cluster");
}

#[test]
fn shp5_line_only_horizontal_in_vertical_across_faces_keeps_one_face() {
    // The backend sets "1" in the monospace face and "漢" in Noto Sans JP; the
    // one cluster takes the first face, and the other face's glyph becomes
    // that face's .notdef rather than an id read in the wrong face.
    let text = "1漢";
    let (atoms, run) = crate::shaper::tate_chu_yoko(text, &[FAMILY]);
    let mut clusters = Vec::new();
    let mut glyphs = Vec::new();
    LINES.shape_run(text, &run, &atoms, &mut clusters, &mut glyphs);

    assert_eq!(clusters.len(), 1, "{clusters:?}");
    assert_eq!(clusters[0].font.font_id, FontId(0));
    assert_eq!(clusters[0].glyph_range, 0..glyphs.len());
    assert_eq!(
        glyphs.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
        [glyph_for(FONT_BYTES, '1').unwrap(), 0]
    );
    assert!(glyphs[1].x_advance > 0.0, "{glyphs:?}");
}
