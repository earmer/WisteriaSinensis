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
