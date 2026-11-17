//! The vendored script fonts shape the samples the acceptance tests use
//! (`tests/assets/README.md`): each sample takes its own face, with no
//! `.notdef` and no dotted circle.

use crate::common::*;
use mutil_engine::*;

/// The clusters of `text` as `(text, font id, glyph count)`.
fn clusters<'t>(ast: &ParagraphAst, text: &'t str) -> Vec<(&'t str, u32, usize)> {
    ast.clusters
        .iter()
        .map(|c| {
            (
                &text[c.byte_range.as_range()],
                c.font.font_id.0,
                c.glyph_range.len(),
            )
        })
        .collect()
}

#[track_caller]
fn shapes_cleanly_in(text: &str, family: &str, face: u32) {
    let ast = compile_in(text, &[family]);
    let bytes = SCRIPT_FONTS[face as usize - 1].1;
    let dotted_circle = glyph_for(bytes, '\u{25CC}');
    for cluster in &ast.clusters {
        let piece = &text[cluster.byte_range.as_range()];
        if piece.chars().all(char::is_whitespace) {
            continue;
        }
        assert_eq!(cluster.font.font_id, FontId(face), "{piece:?} in {text:?}");
        for glyph in &ast.glyphs[cluster.glyph_range.clone()] {
            assert_ne!(glyph.glyph_id, 0, ".notdef for {piece:?} in {text:?}");
            assert_ne!(
                Some(glyph.glyph_id),
                dotted_circle,
                "dotted circle in {piece:?} of {text:?}"
            );
        }
    }
}

#[test]
fn the_latin_face_forms_fi_and_ffi_ligatures() {
    let text = "office fit";
    let ast = compile_in(text, &["Noto Serif"]);
    let found = clusters(&ast, text);
    assert!(found.contains(&("ffi", 1, 1)), "{found:?}");
    assert!(found.contains(&("fi", 1, 1)), "{found:?}");
    let ffi = ast
        .clusters
        .iter()
        .find(|c| &text[c.byte_range.as_range()] == "ffi")
        .expect("the ffi cluster");
    assert_eq!(ffi.atom_count(), 3);
    shapes_cleanly_in(text, "Noto Serif", 1);
}

#[test]
fn a_devanagari_conjunct_is_one_atom_and_one_cluster() {
    let text = "\u{915}\u{94D}\u{937}";
    let ast = compile_in(text, &["Noto Sans Devanagari"]);
    assert_eq!(ast.atoms.len(), 1);
    assert_eq!(clusters(&ast, text), [(text, 2, 1)]);
    shapes_cleanly_in(text, "Noto Sans Devanagari", 2);
}

#[test]
fn every_script_sample_takes_its_own_face() {
    for (text, family, face) in [
        ("日本語の「テスト」、東京……", "Noto Sans JP", 3),
        ("ภาษาไทยง่ายนิดเดียว", "Noto Sans Thai", 4),
        ("ພາສາລາວງ່າຍ", "Noto Sans Lao", 5),
        ("ភាសាខ្មែរងាយស្រួល", "Noto Sans Khmer", 6),
        ("မြန်မာဘာသာစကား", "Noto Sans Myanmar", 7),
        ("नमस्ते क्षत्रिय", "Noto Sans Devanagari", 2),
    ] {
        shapes_cleanly_in(text, family, face);
    }
}

#[test]
fn ligature_carets_come_from_gdef_scaled_to_the_instance() {
    let text = "office";
    let ast = compile_in(text, &["Noto Serif"]);
    let ffi = ast
        .clusters
        .iter()
        .find(|c| &text[c.byte_range.as_range()] == "ffi")
        .expect("the ffi cluster");
    let glyph = ast.glyphs[ffi.glyph_range.start].glyph_id;
    let instance = FontInstanceRef {
        font: ffi.font,
        font_size: 10.0,
        variations: &[],
    };
    let carets = SCRIPT_SHAPER.ligature_carets(instance, glyph, Direction::Ltr);
    assert_eq!(carets.len(), 2);
    assert_close(carets[0], 3.34);
    assert_close(carets[1], 6.68);
    // A glyph that is no ligature has none.
    let o = ast.glyphs[ast.clusters[0].glyph_range.start].glyph_id;
    assert!(
        SCRIPT_SHAPER
            .ligature_carets(instance, o, Direction::Ltr)
            .is_empty()
    );

    // The caret stops inside the ligature sit at the GDEF carets (INT-5).
    let p = lay_out_in(
        text,
        &["Noto Serif"],
        &column(200.0),
        &LayoutOptions::default(),
    );
    let o_width = ast.clusters[0].natural_width;
    let inner: Vec<(usize, f32)> = p.lines[0]
        .caret_stops
        .iter()
        .filter(|s| (2..=3).contains(&s.offset))
        .map(|s| (s.offset, s.x))
        .collect();
    assert_eq!(inner.len(), 2, "{inner:?}");
    assert_close(inner[0].1, o_width + 3.34);
    assert_close(inner[1].1, o_width + 6.68);
}

#[test]
fn a_box_boundary_inside_a_ligature_cuts_it_and_marks_the_seam() {
    // Under `Anywhere` every grapheme boundary is an opportunity, so the
    // boxes of "office" are single letters and the ffi ligature is cut
    // (spec CLU-8): each letter is re-shaped alone, and the cluster after
    // each cut is not safe to concatenate.
    let text = "office";
    let style = script_style(&["Noto Serif"], 10.0);
    let options = FrontendOptions {
        line_break_strictness: LineBreakStrictness::Anywhere,
        ..Default::default()
    };
    let ast = compile_paragraph(&input(text, &style), &options, &*SCRIPT_SHAPER)
        .expect("the frontend accepts the text");
    assert_eq!(
        clusters(&ast, text),
        [
            ("o", 1, 1),
            ("f", 1, 1),
            ("f", 1, 1),
            ("i", 1, 1),
            ("c", 1, 1),
            ("e", 1, 1)
        ]
    );
    let seams: Vec<bool> = ast
        .clusters
        .iter()
        .map(|c| c.safe_to_concat_before)
        .collect();
    assert_eq!(seams[2..4], [false, false], "{seams:?}");
    check_layout_inputs(&ast, text, &column(20.0), &LayoutOptions::default())
        .expect("the input gate accepts the cut");
}

#[test]
fn an_emergency_seam_inside_kerned_text_re_shapes_and_still_fits() {
    // "AV" kerns in Noto Serif; HarfRust marks the seam unsafe to break, and
    // the emergency split re-shapes both sides (spec ST1-5). The widened
    // sides must still fit, else the split moves back.
    let text = "AVAVAVAVAVAVAV";
    let ast = compile_in(text, &["Noto Serif"]);
    assert!(
        ast.clusters.iter().skip(1).any(|c| !c.safe_to_break_before),
        "the kerned seams are unsafe"
    );
    for width in [18.0, 25.0, 31.0, 47.0] {
        let p = lay_out_in(
            text,
            &["Noto Serif"],
            &column(width),
            &LayoutOptions::default(),
        );
        assert!(p.lines.len() > 1, "wraps at {width}");
        // The output gate (in `lay_out_in`) rejects a line wider than the
        // column that is not one overflowing entity.
        for line in &p.lines {
            let end = line.caret_stops.iter().map(|s| s.x).fold(0.0, f32::max);
            assert!(end <= width + 1e-3, "{width}: line ends at {end}");
        }
    }
}

#[test]
fn window_edits_append_and_replace_with_synthesized_glyphs() {
    let text = "office";
    let ast = compile_in(text, &["Noto Serif"]);
    let (run, atoms) = (&ast.runs[0], ast.run_atoms(0));
    let reshape = |window: TextRange, leading: LeadingEdit, trailing: TrailingEdit| {
        let mut clusters = Vec::new();
        let mut glyphs = Vec::new();
        SCRIPT_SHAPER.reshape_window(
            text,
            run,
            atoms,
            window,
            leading,
            trailing,
            &mut clusters,
            &mut glyphs,
        );
        let covered: Vec<String> = clusters
            .iter()
            .map(|c| text[c.byte_range.as_range()].to_owned())
            .collect();
        let synthesized: Vec<bool> = glyphs.iter().map(|g| g.is_synthesized).collect();
        (clusters, glyphs.len(), covered, synthesized)
    };
    // "off" with a hyphen: the window keeps the ff ligature, and the hyphen
    // is the last cluster's last glyph (SHP-6, CLU-7).
    let (clusters, count, covered, synthesized) = reshape(
        TextRange::new(0, 3),
        LeadingEdit::None,
        TrailingEdit::AppendHyphen { hyphen: '-' },
    );
    assert_eq!(covered, ["o", "ff"]);
    assert_eq!(synthesized, [false, false, true]);
    assert_eq!(clusters[1].glyph_range.end, count);
    assert!(!clusters[0].safe_to_concat_before);
    // The whole word with its last two letters cut and replaced by "k" and
    // a hyphen: the cut atoms keep empty clusters, so the window's atoms
    // stay partitioned, and the replacement goes on the window's last one.
    let (clusters, count, covered, synthesized) = reshape(
        TextRange::new(0, 6),
        LeadingEdit::None,
        TrailingEdit::ReplacePrefix {
            replacement: "k".into(),
            cut_bytes: 2,
            hyphen: '-',
        },
    );
    assert_eq!(covered, ["o", "ffi", "c", "e"]);
    assert_eq!(clusters[2].glyph_range.len(), 0);
    assert_eq!(clusters[3].glyph_range.end, count);
    assert_eq!(synthesized.iter().filter(|&&s| s).count(), 2);
    // A suffix replacement on the window's start goes on its first cluster.
    let (clusters, _, covered, synthesized) = reshape(
        TextRange::new(3, 6),
        LeadingEdit::ReplaceSuffix {
            replacement: "x".into(),
            cut_bytes: 0,
        },
        TrailingEdit::None,
    );
    assert_eq!(covered, ["i", "c", "e"]);
    assert!(synthesized[0]);
    assert_eq!(clusters[0].glyph_range.len(), 2);
}

#[test]
fn shp6_a_cut_cluster_takes_the_face_of_the_nearest_kept_cluster() {
    // A Latin face first and a CJK face second: the ideographs are shaped
    // in the second face.
    let text = "漢字漢字";
    let ast = compile_in(text, &[FAMILY, JAPANESE]);
    assert_eq!(ast.runs.len(), 1);
    let (run, atoms) = (&ast.runs[0], ast.run_atoms(0));
    let japanese = SCRIPT_FONTS
        .iter()
        .find(|(family, _)| *family == JAPANESE)
        .unwrap()
        .1;
    let reshape = |window: TextRange, leading: LeadingEdit, trailing: TrailingEdit| {
        let mut clusters = Vec::new();
        let mut glyphs = Vec::new();
        SCRIPT_SHAPER.reshape_window(
            text,
            run,
            atoms,
            window,
            leading,
            trailing,
            &mut clusters,
            &mut glyphs,
        );
        (clusters, glyphs)
    };
    let cjk_face = ast.clusters[0].font.font_id;
    assert_ne!(cjk_face, FontId(0), "the ideographs take the CJK face");

    // A trailing cut of the last ideograph, replaced by another: the cut
    // cluster takes the face of the last kept one, and the replacement is
    // shaped in it.
    let (clusters, glyphs) = reshape(
        TextRange::new(0, 12),
        LeadingEdit::None,
        TrailingEdit::ReplacePrefix {
            replacement: "漢".into(),
            cut_bytes: 3,
            hyphen: '-',
        },
    );
    assert_eq!(clusters.len(), 4);
    let (kept, cut) = (&clusters[2], &clusters[3]);
    assert_eq!(cut.byte_range, TextRange::new(9, 12));
    assert_eq!(kept.font.font_id, cjk_face);
    assert_eq!(cut.font.font_id, kept.font.font_id);
    let first = &glyphs[cut.glyph_range.start];
    assert!(first.is_synthesized);
    assert_eq!(Some(first.glyph_id), glyph_for(japanese, '漢'));

    // A leading cut of the first ideograph: the face of the first kept one.
    let (clusters, glyphs) = reshape(
        TextRange::new(0, 12),
        LeadingEdit::ReplaceSuffix {
            replacement: "字".into(),
            cut_bytes: 3,
        },
        TrailingEdit::None,
    );
    assert_eq!(clusters.len(), 4);
    let (cut, kept) = (&clusters[0], &clusters[1]);
    assert_eq!(cut.byte_range, TextRange::new(0, 3));
    assert_eq!(kept.font.font_id, cjk_face);
    assert_eq!(cut.font.font_id, kept.font.font_id);
    let first = &glyphs[cut.glyph_range.start];
    assert!(first.is_synthesized);
    assert_eq!(Some(first.glyph_id), glyph_for(japanese, '字'));

    // Both edges cut in one window: each cut follows its own side.
    let (clusters, _) = reshape(
        TextRange::new(0, 12),
        LeadingEdit::ReplaceSuffix {
            replacement: "字".into(),
            cut_bytes: 3,
        },
        TrailingEdit::ReplacePrefix {
            replacement: "漢".into(),
            cut_bytes: 3,
            hyphen: '-',
        },
    );
    assert_eq!(clusters.len(), 4);
    assert!(
        clusters.iter().all(|c| c.font.font_id == cjk_face),
        "{clusters:?}"
    );

    // A window that keeps no atom falls back to the stack's first face.
    let (clusters, _) = reshape(
        TextRange::new(3, 6),
        LeadingEdit::ReplaceSuffix {
            replacement: "漢".into(),
            cut_bytes: 3,
        },
        TrailingEdit::None,
    );
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].font.font_id, FontId(0));
}
