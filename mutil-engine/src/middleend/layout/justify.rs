//! Stage 3 justification (spec ST3-2 to ST3-5): the priority ladder that
//! spends a line's slack on its glue and inter-cluster gaps.

use super::scratch::LogicalPiece;
use super::{Layout, PieceKind, Reshapes};
use crate::frontend::GlueTier;
use crate::middleend::FIT_EPSILON;
use crate::types::{AtomFlags, ScriptTag};

/// The inter-character stretch of an `ALLOW_CJK_JUSTIFY_AFTER` gap, per
/// unit of the run's font size (ST1-7, Tier 2).
pub(super) const CJK_GAP_STRETCH: f32 = 0.25;
/// The residual inter-cluster expansion of Level 3, per unit of the run's
/// font size (ST3-2).
const LETTER_GAP_STRETCH: f32 = 0.125;
/// The ladder's levels: glue Tiers 0 and 1, then the gaps of Tier 2 and
/// Level 3.
const LEVELS: usize = 4;

/// What one piece offers: it takes part in exactly one level. Only glue
/// (Levels 0 and 1) shrinks.
#[derive(Clone, Copy, Default)]
struct Capacity {
    level: usize,
    stretch: f32,
    shrink: f32,
}

/// Scripts that expand at word separators only (ST3-5).
fn expands_at_words_only(script: ScriptTag) -> bool {
    matches!(&script.as_bytes(), b"Thai" | b"Laoo" | b"Khmr" | b"Mymr")
}

impl Layout<'_, '_> {
    /// The capacities of the pieces after the line's last tab (ST3-1): glue
    /// by its tier, and the gap after each cluster that another cluster
    /// follows, with no inline object between them (ST1-7, ST3-2).
    fn capacities(&self, body: &[LogicalPiece<'_>], pending: &Reshapes) -> Vec<Capacity> {
        // Whether the next cluster or object after each piece is a cluster.
        let mut cluster_follows = vec![false; body.len()];
        let mut next_is_cluster = false;
        for (k, p) in body.iter().enumerate().rev() {
            cluster_follows[k] = next_is_cluster;
            match p.piece.kind {
                PieceKind::Cluster { .. } => next_is_cluster = true,
                PieceKind::Object { .. } => next_is_cluster = false,
                _ => {}
            }
        }
        body.iter()
            .zip(cluster_follows)
            .map(|(p, cluster_follows)| match p.piece.kind {
                PieceKind::Glue(glue) => Capacity {
                    level: usize::from(glue.kind.tier() != GlueTier::Tier0),
                    stretch: glue.max_stretch,
                    shrink: glue.max_shrink,
                },
                PieceKind::Cluster { run, cluster } if cluster_follows => {
                    if self.cluster_glyphs(pending, cluster).0.is_empty() {
                        return Capacity::default();
                    }
                    let run = &self.ast.runs[run];
                    let flags = self.ast.clusters[cluster].flags;
                    if flags.contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER) {
                        Capacity {
                            level: 2,
                            stretch: CJK_GAP_STRETCH * run.font_size,
                            shrink: 0.0,
                        }
                    } else if !flags.contains(AtomFlags::NO_LETTER_SPACING)
                        && !expands_at_words_only(run.script)
                    {
                        Capacity {
                            level: 3,
                            stretch: LETTER_GAP_STRETCH * run.font_size,
                            shrink: 0.0,
                        }
                    } else {
                        Capacity::default()
                    }
                }
                _ => Capacity::default(),
            })
            .collect()
    }

    /// The index of the first piece of `body` after its last tab; only
    /// those take part in justification (ST3-1).
    fn after_last_tab(body: &[LogicalPiece<'_>]) -> usize {
        body.iter()
            .rposition(|p| matches!(p.piece.kind, PieceKind::Tab))
            .map_or(0, |tab| tab + 1)
    }

    /// `Z`: how much the glue after the line's last tab can shrink (ST1-7).
    pub(super) fn shrink_capacity(body: &[LogicalPiece<'_>]) -> f32 {
        body[Self::after_last_tab(body)..]
            .iter()
            .map(|p| match p.piece.kind {
                PieceKind::Glue(glue) => glue.max_shrink,
                _ => 0.0,
            })
            .sum()
    }

    /// Spends `slack` on the body pieces after the line's last tab, level by
    /// level, each level shared in proportion to its capacities (ST3-2,
    /// ST3-3), adding to their `widths` and, for a cluster, its
    /// `expansions`. Expansion after a cluster widens it; the cluster's
    /// glyphs place it by direction. Returns whether the line came out flush
    /// (ST3-4).
    pub(super) fn justify(
        &self,
        body: &[LogicalPiece<'_>],
        widths: &mut [f32],
        expansions: &mut [f32],
        slack: f32,
        pending: &Reshapes,
    ) -> bool {
        let first = Self::after_last_tab(body);
        let (body, widths, expansions) = (
            &body[first..],
            &mut widths[first..],
            &mut expansions[first..],
        );
        let capacities = self.capacities(body, pending);
        let mut remaining = slack.abs();
        let level_count = if slack < 0.0 { 2 } else { LEVELS };
        for level in 0..level_count {
            let offered = |c: &Capacity| match (c.level == level, slack < 0.0) {
                (false, _) => 0.0,
                (true, true) => c.shrink,
                (true, false) => c.stretch,
            };
            let total: f32 = capacities.iter().map(offered).sum();
            if remaining <= FIT_EPSILON || total <= 0.0 {
                continue;
            }
            let spent = remaining.min(total);
            let columns = widths.iter_mut().zip(expansions.iter_mut());
            for ((p, c), (width, expansion)) in body.iter().zip(&capacities).zip(columns) {
                let share = spent * offered(c) / total;
                let delta = if slack < 0.0 { -share } else { share };
                *width += delta;
                if matches!(p.piece.kind, PieceKind::Cluster { .. }) {
                    *expansion += delta;
                }
            }
            remaining -= spent;
        }
        remaining <= FIT_EPSILON
    }
}
