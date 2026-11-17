//! Selection geometry (spec INT-16, INT-18): the rectangles a logical range
//! covers, merged over visual order, at the requested block height.

use super::super::ValidatedLayout;
use super::super::output::PositionedParagraph;
use super::super::tolerance::{approx_eq, tolerance};
use super::{BoxHeightStyle, SelectionRect, line_block};
use crate::types::{BidiLevel, Rect, TextRange};

/// Calls `emit(line, x0, x1, level)` for each inline interval that `range`
/// selects (spec INT-16), line by line from the first line that holds the
/// range: visually adjacent selected slots (the stretches between two
/// neighbouring caret stops) merge, a zero-width slot between two selected
/// ones bridges them, and an unselected slot breaks the merge. An empty
/// range selects nothing.
fn for_each_selected_interval(
    paragraph: &PositionedParagraph,
    range: TextRange,
    mut emit: impl FnMut(usize, f32, f32, BidiLevel),
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
        for pair in line.caret_stops.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let bytes = TextRange::new(a.offset.min(b.offset), a.offset.max(b.offset));
            if bytes.is_empty() {
                continue;
            }
            let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
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
                emit(index, lo, hi, level);
            }
        }
        if let Some((lo, hi, level)) = open {
            emit(index, lo, hi, level);
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
        for_each_selected_interval(p, range, |index, x0, x1, level| {
            let (top, bottom) = match height {
                BoxHeightStyle::Max => line_block(p, index),
                BoxHeightStyle::Strut => mode.line_bounds(
                    p.lines[index].baseline_y,
                    p.policy.strut.ascent,
                    p.policy.strut.descent,
                ),
                BoxHeightStyle::Tight => self.tight_block(shaper, index, x0, x1, range),
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
        for_each_selected_interval(p, range, |index, x0, x1, _| {
            let (top, bottom) = line_block(p, index);
            out.push(p.policy.writing_mode.flow_to_physical_rect(
                Rect::new(x0, top, x1 - x0, bottom - top),
                p.total_block_size,
            ));
        });
        out
    }

    /// The block interval of a `Tight` piece `[x0, x1]` of line `line`: the
    /// union of the ascents and descents of the selected glyph runs' font
    /// instances (through the shaper gate) and inline boxes, else the strut.
    fn tight_block(
        &self,
        shaper: &dyn crate::shaping::TextShaper,
        line: usize,
        x0: f32,
        x1: f32,
        range: TextRange,
    ) -> (f32, f32) {
        let p = self.paragraph();
        let l = &p.lines[line];
        let (mut above, mut below) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
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
            let Ok(metrics) = super::super::shaper_gate::checked_font_metrics(
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
            above = above.max(ascent);
            below = below.max(descent);
        }
        for b in &l.visual_boxes {
            if meets(b.x, b.x + b.width) && range.contains_range(b.byte_range) {
                above = above.max(b.ascent - b.y);
                below = below.max(b.descent + b.y);
            }
        }
        if !above.is_finite() || !below.is_finite() {
            above = p.policy.strut.ascent;
            below = p.policy.strut.descent;
        }
        // Raw font metrics may sum to a negative height; the line box and
        // the strut never do.
        let (top, bottom) = p
            .policy
            .writing_mode
            .line_bounds(l.baseline_y, above, below);
        (top, bottom.max(top))
    }
}
