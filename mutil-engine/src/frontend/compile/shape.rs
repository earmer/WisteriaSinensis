//! Shaping the runs into the AST's storage (spec FE-27, CLU-8, AST-4).

use super::runs::UnshapedRun;
use super::{Compiler, FrontendError, Shaped};
use crate::shaping::{ReshapeRequest, ShapeOutput};
use crate::types::{
    GraphemeAtom, LocaleTag, RawShapedGlyph, ShapedCluster, TextRange, natural_width,
};

impl Compiler<'_> {
    /// Shapes every run, splits clusters that straddle a text box boundary
    /// (CLU-8), applies letter spacing (FE-27) and stores the result (AST-4).
    pub(super) fn shape_runs(
        &self,
        all_atoms: &[GraphemeAtom],
        runs: Vec<UnshapedRun>,
        locales: &[LocaleTag],
        box_starts: &[bool],
    ) -> Result<Shaped, FrontendError> {
        let mut ast = Shaped {
            runs: Vec::with_capacity(runs.len()),
            clusters: Vec::new(),
            glyphs: Vec::new(),
        };
        let mut shaped = ShapeOutput::default();
        let mut window_out = ShapeOutput::default();
        for (run_index, unshaped) in runs.into_iter().enumerate() {
            let contract = FrontendError::ShaperContract { run: run_index };
            let style = self.styles[unshaped.style];
            let locale = locales[unshaped.atom_range.start].clone();
            let atoms = &all_atoms[unshaped.atom_range.clone()];
            let mut run = unshaped.into_shaping_run(style, locale, atoms);
            shaped.clear();
            self.shaper.shape_run(self.text, &run, atoms, &mut shaped);
            if !partitions(&shaped.clusters, &shaped.glyphs, 0..atoms.len()) {
                return Err(contract);
            }
            // Cuts are the run-local atoms that start a box, except the first.
            let is_cut = |local: usize| local > 0 && box_starts[run.atom_range.start + local];
            let push_cluster =
                |mut cluster: ShapedCluster, source_glyphs: &[RawShapedGlyph], ast: &mut Shaped| {
                    let own = &source_glyphs[cluster.glyph_range.clone()];
                    cluster.natural_width = natural_width(own, run.letter_spacing);
                    let at = ast.glyphs.len();
                    ast.glyphs.extend_from_slice(own);
                    cluster.glyph_range = at..ast.glyphs.len();
                    ast.clusters.push(cluster);
                };
            let cluster_start = ast.clusters.len();
            for cluster in shaped.clusters.drain(..) {
                let range = cluster.atom_range.clone();
                let mut cuts = (range.start + 1..range.end)
                    .filter(|&a| is_cut(a))
                    .peekable();
                if cuts.peek().is_none() {
                    push_cluster(cluster, &shaped.glyphs, &mut ast);
                    continue;
                }
                let bounds: Vec<usize> = std::iter::once(range.start)
                    .chain(cuts)
                    .chain(std::iter::once(range.end))
                    .collect();
                for (k, window) in bounds.windows(2).enumerate() {
                    let (w_start, w_end) = (window[0], window[1]);
                    window_out.clear();
                    let bytes = TextRange::new(
                        atoms[w_start].byte_range.start(),
                        atoms[w_end - 1].byte_range.end(),
                    );
                    let request = ReshapeRequest::plain(self.text, &run, atoms, bytes);
                    self.shaper.reshape_window(&request, &mut window_out);
                    if !partitions(&window_out.clusters, &window_out.glyphs, w_start..w_end) {
                        return Err(contract);
                    }
                    for (j, mut c) in window_out.clusters.drain(..).enumerate() {
                        if k > 0 && j == 0 {
                            c.safe_to_concat_before = false;
                        }
                        push_cluster(c, &window_out.glyphs, &mut ast);
                    }
                }
            }
            run.cluster_range = cluster_start..ast.clusters.len();
            ast.runs.push(run);
        }
        Ok(ast)
    }
}

/// Whether `clusters` partition `atoms` in order and their glyph ranges
/// tile `glyphs` (SHP-4, SHP-5).
fn partitions(
    clusters: &[ShapedCluster],
    glyphs: &[RawShapedGlyph],
    atoms: core::ops::Range<usize>,
) -> bool {
    let mut next_atom = atoms.start;
    let mut next_glyph = 0;
    for c in clusters {
        if c.atom_range.start != next_atom
            || c.atom_range.end <= c.atom_range.start
            || c.glyph_range.start != next_glyph
            || c.glyph_range.end < c.glyph_range.start
        {
            return false;
        }
        next_atom = c.atom_range.end;
        next_glyph = c.glyph_range.end;
    }
    next_atom == atoms.end && next_glyph == glyphs.len()
}
