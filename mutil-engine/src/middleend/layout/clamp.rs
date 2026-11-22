//! Line clamping and synthesized ellipsis suffixes (spec CLP-1, PAR-3).

use super::breaking::within;
use super::line_box::half_leaded;
use super::scratch::LayoutScratch;
use super::{Layout, LineCursor, LineEnd, Piece, PieceKind, Reshapes, Token};
use crate::middleend::shaper_gate::{checked_line_box, checked_shape_run};
use crate::middleend::validated::{RunId, TokenIdx};
use crate::middleend::{
    LayoutInputError, OpenGeometry, PositionedGlyph, PositionedLine, VisualGlyphRun,
};
use crate::types::{AtomFlags, GlyphOrientation, GraphemeAtom, ShapingRun, TextRange};

pub(super) struct SynthesizedEllipsis {
    pub(super) runs: Vec<VisualGlyphRun>,
    pub(super) width: f64,
    pub(super) ascent: f64,
    pub(super) descent: f64,
}

impl<'i, 'a> Layout<'i, 'a> {
    pub(super) fn clamp_line(
        &mut self,
        cursor: &LineCursor<'a>,
        end: LineEnd<'a>,
        ellipsis: &str,
        geometry: &OpenGeometry,
        scratch: &mut LayoutScratch<'a>,
    ) -> Result<Option<(PositionedLine, f64)>, LayoutInputError> {
        let end_token = match end {
            LineEnd::At { token, .. } | LineEnd::Before(token) => token,
        };
        let Some(mut boundary) = self.atomic_safe_boundary(cursor, end_token) else {
            return Ok(None);
        };
        let slot_width = f64::from(geometry.first_slot().width());
        let (mut suffix_run, mut suffix) = (None, None);
        loop {
            if !ellipsis.is_empty()
                && let Some(run) = self.ellipsis_run(cursor.token_start, boundary)
                && suffix_run != Some(run)
            {
                suffix = self.shape_ellipsis(run, ellipsis)?;
                suffix_run = Some(run);
            }
            let mut reshapes = Reshapes::new();
            self.repair_seam(cursor, boundary, &mut reshapes);
            let (line, height) = self.place(
                cursor,
                LineEnd::Before(boundary),
                geometry,
                &reshapes,
                scratch,
                Some(suffix.as_ref()),
            )?;
            if within(f64::from(line.visual_width()), slot_width)
                || line.byte_range.is_empty()
                || boundary == cursor.token_start
            {
                if !line.byte_range.is_empty() {
                    self.reshaped.extend(reshapes);
                }
                return Ok(Some((line, height)));
            }
            let Some(next) = boundary
                .prev()
                .filter(|&p| p >= cursor.token_start)
                .and_then(|p| self.atomic_safe_boundary(cursor, p))
            else {
                return Ok(None);
            };
            boundary = next;
        }
    }

    fn atomic_safe_boundary(
        &self,
        cursor: &LineCursor<'_>,
        boundary: TokenIdx,
    ) -> Option<TokenIdx> {
        let bytes = self.tokens.at_or_sentinel(boundary).bytes().start();
        match self.ast.atomic_span_containing(bytes) {
            Some(s) if s.byte_range.start() < cursor.byte_start => None,
            Some(s) if s.byte_range.start() < bytes => Some(
                self.tokens
                    .iter_range(cursor.token_start, boundary)
                    .find(|(_, t)| t.bytes().start() >= s.byte_range.start())
                    .map_or(boundary, |(i, _)| i),
            ),
            _ => Some(boundary),
        }
    }

    fn shape_ellipsis(
        &self,
        run_id: RunId,
        ellipsis: &str,
    ) -> Result<Option<SynthesizedEllipsis>, LayoutInputError> {
        let Some(template) = self.inputs.run_atoms_id(run_id).first() else {
            return Ok(None);
        };
        let run = self.inputs.run(run_id);
        let level = self.base.base_level();
        let orientation =
            if run.orientation == GlyphOrientation::VerticalUpright && self.base.is_rtl() {
                GlyphOrientation::VerticalSideways
            } else {
                run.orientation
            };
        let atoms: Vec<_> = ellipsis
            .char_indices()
            .map(|(offset, ch)| GraphemeAtom {
                byte_range: TextRange::new(offset, offset + ch.len_utf8()),
                bidi_level: level,
                flags: AtomFlags::EMPTY,
                ..template.clone()
            })
            .collect();
        let synthetic = ShapingRun {
            byte_range: TextRange::new(0, ellipsis.len()),
            atom_range: 0..atoms.len(),
            cluster_range: 0..0,
            bidi_level: level,
            orientation,
            ..run.clone()
        };
        let mut runs = Vec::new();
        let (mut width, mut ascent, mut descent) = (0.0, 0.0_f64, 0.0_f64);
        let variations: smallvec::SmallVec<[_; 2]> =
            synthetic.features.variations().iter().copied().collect();
        checked_shape_run(
            self.shaper,
            ellipsis,
            &synthetic,
            &atoms,
            self.ast.writing_mode,
            |cluster, glyphs| {
                if glyphs.is_empty() {
                    return Ok(());
                }
                width += glyphs.iter().map(|g| f64::from(g.x_advance)).sum::<f64>();
                let (a, d) = half_leaded(checked_line_box(
                    self.shaper,
                    synthetic.font_instance(cluster.font),
                    self.options.line_height,
                    self.ast.writing_mode,
                )?);
                (ascent, descent) = (ascent.max(a), descent.max(d));
                runs.push(VisualGlyphRun {
                    orientation: cluster.orientation,
                    font_size: synthetic.font_size,
                    x_scale: 1.0,
                    variations: variations.clone(),
                    font: cluster.font,
                    bidi_level: cluster.bidi_level,
                    byte_range: TextRange::empty_at(0),
                    glyphs: glyphs
                        .iter()
                        .map(|g| PositionedGlyph {
                            glyph_id: g.glyph_id,
                            x: 0.0,
                            x_offset: g.x_offset,
                            y_offset: g.y_offset,
                            advance: g.x_advance,
                            byte_range: None,
                        })
                        .collect(),
                });
                Ok(())
            },
        )?;
        if runs.is_empty() {
            return Ok(None);
        }
        Ok(Some(SynthesizedEllipsis {
            runs,
            width,
            ascent,
            descent,
        }))
    }

    fn ellipsis_run(&self, start: TokenIdx, end: TokenIdx) -> Option<RunId> {
        let run_of = |t: &Token<'_>| match t {
            Token::Piece(Piece {
                kind: PieceKind::Cluster { run, .. },
                ..
            }) => Some(*run),
            _ => None,
        };
        self.tokens
            .slice(start, end)
            .iter()
            .rev()
            .find_map(run_of)
            .or_else(|| self.tokens.slice_from(end).iter().find_map(run_of))
    }
}
