//! Selection geometry (spec INT-16, INT-18): the rectangles a logical range
//! covers, merged over visual order, at the requested block height.

use super::{BoxHeightStyle, SelectionRect, line_box};
use crate::middleend::ValidatedLayout;
use crate::middleend::output::{PositionedLine, PositionedParagraph};
use crate::middleend::tolerance::{approx_eq, tolerance};
use crate::types::{BidiLevel, Rect, TextRange};

/// Calls `emit(index, line, x0, x1, level)` for each inline interval that
/// `range` selects (spec INT-16), line by line from the first line that holds the
/// range: visually adjacent selected slots (the stretches between two
/// neighbouring caret stops) merge, a zero-width slot between two selected
/// ones bridges them, and an unselected slot breaks the merge. An empty
/// range selects nothing.
fn for_each_selected_interval(
    paragraph: &PositionedParagraph,
    range: TextRange,
    mut emit: impl FnMut(usize, &PositionedLine, f32, f32, BidiLevel),
) {
    if range.is_empty() {
        return;
    }
    for (index, line) in paragraph.lines.iter().enumerate() {
        if line.byte_range.end() <= range.start() && !line.byte_range.is_empty() {
            continue;
        }
        if line.byte_range.start() >= range.end() {
            break;
        }
        let mut open: Option<(f32, f32, BidiLevel)> = None;
        let stops = &line.caret_stops;
        for (a, b) in stops.iter().zip(stops.iter().skip(1)) {
            let bytes = TextRange::new(a.offset.min(b.offset), a.offset.max(b.offset));
            if bytes.is_empty() {
                continue;
            }
            // G4 proved the stops in non-decreasing `x` (InvalidCaretStop).
            let (x0, x1) = (a.x, b.x);
            if range.contains_range(bytes) {
                open = Some(match open {
                    Some((lo, hi, level)) => (
                        lo.min(x0),
                        hi.max(x1),
                        if approx_eq(lo, hi) { a.level } else { level },
                    ),
                    None => (x0, x1, a.level),
                });
            } else if let Some((lo, hi, level)) = open.take() {
                emit(index, line, lo, hi, level);
            }
        }
        if let Some((lo, hi, level)) = open {
            emit(index, line, lo, hi, level);
        }
    }
}

impl ValidatedLayout<'_> {
    /// The selection rectangles of `range` (spec INT-16), physical and
    /// paragraph-local, line by line in visual order. `Tight` spans the
    /// ascent and descent of the font instances and inline boxes the piece
    /// crosses, asked of `shaper` through the shaper gate; `Max` the line
    /// box, which holds the line gap and the tallest inline box; `Strut` the
    /// resolved strut. An empty range gives none, and no line-end extension
    /// is drawn.
    #[must_use]
    pub fn selection_geometry(
        &self,
        shaper: &dyn crate::shaping::TextShaper,
        range: TextRange,
        height: BoxHeightStyle,
    ) -> Vec<SelectionRect> {
        let p = self.paragraph();
        let mode = p.policy.writing_mode;
        let mut out = Vec::new();
        for_each_selected_interval(p, range, |index, line, x0, x1, level| {
            let (top, bottom) = match height {
                BoxHeightStyle::Max => line_box(p, line),
                BoxHeightStyle::Strut => mode.line_bounds(
                    line.baseline_y,
                    p.policy.strut.ascent,
                    p.policy.strut.descent,
                ),
                BoxHeightStyle::Tight => self.tight_block(shaper, line, x0, x1, range),
            };
            out.push(SelectionRect {
                rect: mode.flow_to_physical_rect(
                    Rect::new(x0, top, x1 - x0, bottom - top),
                    p.total_block_size,
                ),
                line: index,
                level,
                is_line_end_extension: false,
            });
        });
        out
    }

    /// The rectangles of `range` over full line boxes, for placing an IME
    /// candidate window (spec INT-18).
    #[must_use]
    pub fn range_rects(&self, range: TextRange) -> Vec<Rect> {
        let p = self.paragraph();
        let mut out = Vec::new();
        for_each_selected_interval(p, range, |_, line, x0, x1, _| {
            let (top, bottom) = line_box(p, line);
            out.push(p.policy.writing_mode.flow_to_physical_rect(
                Rect::new(x0, top, x1 - x0, bottom - top),
                p.total_block_size,
            ));
        });
        out
    }

    /// The block interval of a `Tight` piece `[x0, x1]` of `line`: the
    /// union of the ascents and descents of the selected glyph runs' font
    /// instances (through the shaper gate) and inline boxes, else the strut.
    fn tight_block(
        &self,
        shaper: &dyn crate::shaping::TextShaper,
        l: &PositionedLine,
        x0: f32,
        x1: f32,
        range: TextRange,
    ) -> (f32, f32) {
        let p = self.paragraph();
        // The union of the selected extents: above and below the baseline.
        let mut extent: Option<(f32, f32)> = None;
        let mut include = |above: f32, below: f32| {
            extent = Some(extent.map_or((above, below), |(a, b)| (a.max(above), b.max(below))));
        };
        let eps = tolerance(x0, x1);
        // An entity counts when its extent meets the piece.
        let meets = |lo: f32, hi: f32| lo < x1 - eps && hi > x0 + eps;
        for run in &l.visual_runs {
            // A glyph counts when its extent meets the piece and its bytes
            // meet the selection: part of a ligature selects its font.
            let selected = run.glyphs.iter().any(|g| {
                let (lo, hi) = g.pen_interval();
                meets(lo, hi)
                    && g.byte_range
                        .is_some_and(|b| b.start() < range.end() && range.start() < b.end())
            });
            if !selected {
                continue;
            }
            let Ok(metrics) = crate::middleend::shaper_gate::checked_font_metrics(
                shaper,
                crate::types::FontInstanceRef {
                    font: run.font,
                    font_size: run.font_size,
                    variations: &run.variations,
                },
            ) else {
                continue;
            };
            let (ascent, descent) = if p.policy.writing_mode.is_vertical() {
                (metrics.vertical_ascent, metrics.vertical_descent)
            } else {
                (metrics.ascent, metrics.descent)
            };
            include(ascent, descent);
        }
        for b in &l.visual_boxes {
            if meets(b.x, b.x + b.width) && range.contains_range(b.byte_range) {
                include(b.ascent - b.y, b.descent + b.y);
            }
        }
        let (above, below) = extent.unwrap_or((p.policy.strut.ascent, p.policy.strut.descent));
        // Both are finite, and so are the writing-mode edges and block span
        // they reach from the baseline (including in `VerticalRl` physical
        // space): the shaper gate proves font metrics measurements, and G4
        // proves an inline box's `ascent - y`, `descent + y`, and their
        // writing-mode bounds and union span (`check_finite`,
        // `check_drawing_origins`).
        // Their sum is not negative: font ascents and descents are
        // (FontMetrics::is_valid), an inline box's are (`check_finite`) and
        // every box adds to both sides, its `y` cancelling; G4 proves the
        // strut's box non-negative (`LayoutPolicySnapshot::is_valid`).
        p.policy
            .writing_mode
            .line_bounds(l.baseline_y, above, below)
    }
}
