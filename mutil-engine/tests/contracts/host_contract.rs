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
    assert_eq!(runs[0].orientation(), orientation);
    assert!(matches!(runs[0].source(), PlacedRunSource::Body(_)));
    let glyphs: Vec<_> = runs[0].glyphs().collect();
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].glyph_id, 1);
    glyphs[0].origin
}

/// HST-4, ORI-2: the over offset moves the origin towards line-over in every
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

/// One line with a body glyph at pen 0 and an `Over` annotation glyph at pen
/// 25, `baseline_offset` 8 towards line-over, in `mode`.
fn annotated(mode: WritingMode, orientation: GlyphOrientation, baseline_y: f32) -> Vec<Point> {
    let mut l = line();
    let mut r = run(glyph(0.0, 100.0, false));
    r.orientation = orientation;
    l.visual_runs = vec![r];
    let mut ann = positioned_annotation();
    ann.orientation = orientation;
    l.ascent += 3.0;
    l.line_height += 3.0;
    l.baseline_y = baseline_y;
    l.annotations = vec![ann];
    let mut p = paragraph(&ctx(), vec![l]);
    p.policy.writing_mode = mode;
    p.total_block_size = 13.0;
    let layout = ValidatedLayout::new(&p, "a").expect("valid output");
    let runs: Vec<_> = layout.glyph_runs().collect();
    assert_eq!(runs.len(), 2);
    assert!(matches!(runs[0].source(), PlacedRunSource::Body(_)));
    assert!(matches!(runs[1].source(), PlacedRunSource::Annotation(_)));
    assert_eq!(runs[1].font_size(), 5.0);
    runs.iter()
        .flat_map(|run| run.glyphs())
        .map(|g| g.origin)
        .collect()
}

/// HST-3, HST-4: annotation glyphs reach the renderer, on a baseline moved
/// towards line-over by `baseline_offset`.
#[test]
fn annotation_runs_are_placed_beyond_the_base() {
    use GlyphOrientation::*;
    use WritingMode::*;
    assert_eq!(
        annotated(HorizontalTb, Horizontal, 11.0),
        [Point { x: 0.0, y: 11.0 }, Point { x: 25.0, y: 3.0 }]
    );
    // Line-over is rightwards: the baseline is 2 from the left edge.
    assert_eq!(
        annotated(VerticalLr, VerticalUpright, 2.0),
        [Point { x: 2.0, y: 0.0 }, Point { x: 10.0, y: 25.0 }]
    );
    // Line-over is still rightwards: the baseline is 11 from the right edge.
    assert_eq!(
        annotated(VerticalRl, VerticalUpright, 11.0),
        [Point { x: 2.0, y: 0.0 }, Point { x: 10.0, y: 25.0 }]
    );
}

/// HST-4, AX-3: the output gate rejects a glyph whose drawing origin
/// overflows, so `PlacedRun::glyphs` only yields finite origins.
#[test]
fn the_output_gate_rejects_overflowing_drawing_origins() {
    let mut l = line();
    l.baseline_y = 3.0e38;
    let mut g = glyph(0.0, 100.0, false);
    g.y_offset = -3.0e38;
    l.visual_runs = vec![run(g)];
    let mut p = paragraph(&ctx(), vec![l]);
    p.total_block_size = 3.1e38;
    assert_eq!(
        ValidatedLayout::new(&p, "a").map(drop),
        Err((0, LineInvariantError::NonFiniteGlyph { run: 0, glyph: 0 }))
    );
}
