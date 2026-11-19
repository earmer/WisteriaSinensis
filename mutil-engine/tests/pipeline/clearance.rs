//! An obstruction that clears far from the origin (GEO-3): G3 refuses an
//! envelope `f32` cannot hold as `BlockPositionUnrepresentable`, so layout
//! fails before any line box collapses; a moderate clearance lays out and
//! passes the output gate (`LineBoxCollapsed` never fires on engine output).

use crate::common::*;
use mutil_engine::*;
use smallvec::smallvec;
use std::sync::Arc;

/// Obstructed, pointing at `clear_at`, while the block start is before it;
/// one slot 200 wide from there on.
#[derive(Debug)]
struct ClearsAt(f32);

impl LineGeometryProvider for ClearsAt {
    fn query_line_geometry(&self, _: usize, block_start: f32, _: f32) -> LineGeometry {
        if block_start < self.0 {
            LineGeometry {
                slots: smallvec![],
                next_feature_block_offset: Some(self.0),
            }
        } else {
            LineGeometry {
                slots: smallvec![InlineSlotConstraint::try_new(0.0, 200.0).unwrap()],
                next_feature_block_offset: None,
            }
        }
    }
}

fn lay_out_clearing_at(clear_at: f32, text: &str) -> Result<PositionedParagraph, LayoutInputError> {
    let constraints = LayoutConstraints {
        geometry_provider: Some(Arc::new(ClearsAt(clear_at))),
        ..column(200.0)
    };
    layout_paragraph(
        &compile(text),
        text,
        &*SHAPER,
        &constraints,
        &LayoutOptions::default(),
    )
}

#[test]
fn geo3_a_clearance_far_from_the_origin_is_block_position_unrepresentable() {
    for clear_at in [3e8_f32, 5e7, 1e12] {
        assert_eq!(
            lay_out_clearing_at(clear_at, "ab cd").map(drop),
            Err(LayoutInputError::BlockPositionUnrepresentable),
            "{clear_at}"
        );
    }
}

#[test]
fn geo3_a_moderate_clearance_lays_out_and_passes_the_output_gate() {
    let text = "ab cd";
    for clear_at in [1000.0_f32, 1e5] {
        let p = lay_out_clearing_at(clear_at, text).unwrap_or_else(|e| panic!("{clear_at}: {e}"));
        if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&p, text) {
            panic!("{clear_at}: line {line}: {error}\n{p:#?}");
        }
        let first = &p.lines[0];
        let top = first.baseline_y - first.ascent;
        assert!(
            top >= clear_at - 1e-3 * clear_at.max(1.0),
            "{clear_at}: {top}"
        );
        assert!(p.total_block_size >= first.baseline_y + first.descent);
    }
}
