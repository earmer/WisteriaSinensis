//! Shaping the runs into the AST's storage (spec FE-27, CLU-8, AST-4).

use super::{Compiler, FrontendError, Shaped};
use crate::shaping::{LeadingEdit, TrailingEdit};
use crate::types::{GraphemeAtom, RawShapedGlyph, ShapedCluster, ShapingRun, TextRange};

impl Compiler<'_> {
    /// Shapes every run, splits clusters that straddle a text box boundary
    /// (CLU-8), applies letter spacing (FE-27) and stores the result (AST-4).
    pub(super) fn shape_runs(
        &self,
        all_atoms: &[GraphemeAtom],
        runs: &[ShapingRun],
        box_starts: &[bool],
    ) -> Result<Shaped, FrontendError> {
        let mut ast = Shaped {
            runs: Vec::new(),
            clusters: Vec::new(),
            glyphs: Vec::new(),
        };
        let mut clusters: Vec<ShapedCluster> = Vec::new();
        let mut glyphs: Vec<RawShapedGlyph> = Vec::new();
        let mut out_clusters: Vec<ShapedCluster> = Vec::new();
        let mut out_glyphs: Vec<RawShapedGlyph> = Vec::new();
        for (run_index, run) in runs.iter().enumerate() {
            let contract = FrontendError::ShaperContract { run: run_index };
            let atoms = &all_atoms[run.atom_range.clone()];
            clusters.clear();
            glyphs.clear();
            self.shaper
                .shape_run(self.text, run, atoms, &mut clusters, &mut glyphs);
            if !partitions(&clusters, &glyphs, 0..atoms.len()) {
                return Err(contract);
            }
            // Cuts are the run-local atoms that start a box, except the first.
            let is_cut = |local: usize| local > 0 && box_starts[run.atom_range.start + local];
            let push_cluster =
                |mut cluster: ShapedCluster, source_glyphs: &[RawShapedGlyph], ast: &mut Shaped| {
                    let own = &source_glyphs[cluster.glyph_range.clone()];
                    let advance: f32 = own.iter().map(|g| g.x_advance).sum();
                    let raw = advance + run.letter_spacing;
                    cluster.natural_width = if raw.is_finite() { raw.max(0.0) } else { raw };
                    let at = ast.glyphs.len();
                    ast.glyphs.extend_from_slice(own);
                    cluster.glyph_range = at..ast.glyphs.len();
                    ast.clusters.push(cluster);
                };
            let cluster_start = ast.clusters.len();
            for cluster in clusters.drain(..) {
                let range = cluster.atom_range.clone();
                let mut bounds = vec![range.start];
                bounds.extend((range.start + 1..range.end).filter(|&a| is_cut(a)));
                bounds.push(range.end);
                if bounds.len() == 2 {
                    push_cluster(cluster, &glyphs, &mut ast);
                    continue;
                }
                for (k, window) in bounds.windows(2).enumerate() {
                    let (w_start, w_end) = (window[0], window[1]);
                    out_clusters.clear();
                    out_glyphs.clear();
                    let bytes = TextRange::new(
                        atoms[w_start].byte_range.start(),
                        atoms[w_end - 1].byte_range.end(),
                    );
                    self.shaper.reshape_window(
                        self.text,
                        run,
                        atoms,
                        bytes,
                        LeadingEdit::None,
                        TrailingEdit::None,
                        &mut out_clusters,
                        &mut out_glyphs,
                    );
                    if !partitions(&out_clusters, &out_glyphs, w_start..w_end) {
                        return Err(contract);
                    }
                    for (j, mut c) in out_clusters.drain(..).enumerate() {
                        if k > 0 && j == 0 {
                            c.safe_to_concat_before = false;
                        }
                        push_cluster(c, &out_glyphs, &mut ast);
                    }
                }
            }
            ast.runs.push(ShapingRun {
                cluster_range: cluster_start..ast.clusters.len(),
                ..run.clone()
            });
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
