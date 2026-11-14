//! The host contract (§8): font resolution without copying, and glyph runs
//! with physical drawing origins in every writing mode.

use super::common::*;
use mutil_engine::*;
use std::collections::HashMap;
use std::sync::Arc;

/// A font registry shared by a shaper and a renderer (HST-2).
struct Registry(HashMap<FontId, FontFace>);

impl FontSource for Registry {
    fn resolve_font(&self, font: FontId) -> Option<FontFace> {
        self.0.get(&font).cloned()
    }
}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn a_font_source_resolves_faces_without_copying() {
    assert_send_sync::<FontBytes>();
    assert_send_sync::<FontFace>();
    static EMBEDDED: [u8; 4] = *b"true";
    let file = vec![0_u8; 64];
    let file_address = file.as_ptr();
    let registry = Registry(HashMap::from([
        (
            FontId(1),
            FontFace::new(FontBytes::from_static(&EMBEDDED), 0),
        ),
        (FontId(2), FontFace::new(FontBytes::new(file), 3)),
    ]));
    let source: Arc<dyn FontSource> = Arc::new(registry);
    let embedded = source.resolve_font(FontId(1)).expect("issued id");
    assert_eq!(embedded.bytes().as_bytes().as_ptr(), EMBEDDED.as_ptr());
    let collection = source.resolve_font(FontId(2)).expect("issued id");
    assert_eq!(collection.bytes().as_bytes().as_ptr(), file_address);
    assert_eq!(collection.face_index(), 3);
    assert!(source.resolve_font(FontId(9)).is_none());
}

/// One line with one glyph at pen 0, offset 3 along the inline axis and 2
/// towards line-over, in `mode`.
fn placed(mode: WritingMode, orientation: GlyphOrientation, baseline_y: f32) -> Point {
    let mut l = line();
    l.baseline_y = baseline_y;
    let mut g = glyph(0.0, 100.0, false);
    g.x_offset = 3.0;
    g.y_offset = 2.0;
    let mut r = run(g);
    r.orientation = orientation;
    l.visual_runs = vec![r];
    let mut p = paragraph(&ctx(), vec![l]);
    p.policy.writing_mode = mode;
    p.total_block_size = 10.0;
    let layout = ValidatedLayout::new(&p, "a").expect("valid output");
    let runs: Vec<_> = layout.glyph_runs().collect();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].line(), 0);
    assert_eq!(runs[0].run().orientation, orientation);
    let glyphs: Vec<_> = runs[0].glyphs().collect();
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].glyph_id, 1);
    glyphs[0].origin
}

/// HST-5, ORI-2: the over offset moves the origin towards line-over in every
/// mode: up in `HorizontalTb`, right in both vertical modes.
#[test]
fn drawing_origins_are_physical_in_every_writing_mode() {
    use GlyphOrientation::*;
    use WritingMode::*;
    assert_eq!(
        placed(HorizontalTb, Horizontal, 8.0),
        Point { x: 3.0, y: 6.0 }
    );
    for orientation in [VerticalUpright, VerticalSideways] {
        // Block progresses rightwards; the baseline is 2 from the left edge.
        assert_eq!(
            placed(VerticalLr, orientation, 2.0),
            Point { x: 4.0, y: 3.0 }
        );
        // Block progresses leftwards; the baseline is 8 from the right edge.
        assert_eq!(
            placed(VerticalRl, orientation, 8.0),
            Point { x: 4.0, y: 3.0 }
        );
    }
}

/// ORI-7: the output gate rejects a tate-chu-yoko run until its placement is
/// defined, so `glyph_runs` never meets one.
#[test]
fn the_output_gate_rejects_unplaced_tate_chu_yoko() {
    let mut l = line();
    l.baseline_y = 8.0;
    let mut r = run(glyph(0.0, 100.0, false));
    r.orientation = GlyphOrientation::HorizontalInVertical;
    l.visual_runs = vec![r];
    let mut p = paragraph(&ctx(), vec![l]);
    p.policy.writing_mode = WritingMode::VerticalRl;
    p.total_block_size = 10.0;
    assert_eq!(
        ValidatedLayout::new(&p, "a").map(drop),
        Err((0, LineInvariantError::UnplacedTateChuYoko { run: 0 }))
    );
}
