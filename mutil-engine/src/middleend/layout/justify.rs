//! Stage 3 justification (spec ST3-2 to ST3-5): the priority ladder that
//! spends a line's slack on its glue and inter-cluster gaps.

use super::{FIT_EPSILON, Layout, PieceKind, PlacedPiece};
use crate::frontend::{GlueNode, GlueTier, WordNode};
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

/// What one piece offers each level.
#[derive(Clone, Copy, Default)]
struct Capacity {
    stretch: [f32; LEVELS],
    /// Shrink of glue Tiers 0 and 1.
    shrink: [f32; 2],
}

/// Scripts that expand at word separators only (ST3-5).
fn expands_at_words_only(script: ScriptTag) -> bool {
    matches!(&script.as_bytes(), b"Thai" | b"Laoo" | b"Khmr" | b"Mymr")
}

impl Layout<'_, '_> {
    /// The glue node a glue piece names.
    pub(super) fn glue(&self, item: usize) -> &GlueNode {
        let WordNode::Glue(glue) = &self.ast.items[item] else {
            unreachable!("a glue piece names a glue item")
        };
        glue
    }

    /// The capacities of the pieces after the line's last tab (ST3-1): glue
    /// by its tier, and the gap after each cluster but the last (ST1-7,
    /// ST3-2).
    fn capacities(&self, body: &[PlacedPiece]) -> Vec<Capacity> {
        let last_cluster = body
            .iter()
            .rposition(|p| matches!(p.piece.kind, PieceKind::Cluster { .. }));
        body.iter()
            .enumerate()
            .map(|(k, p)| match p.piece.kind {
                PieceKind::Glue { item } => {
                    let glue = self.glue(item);
                    let tier = match glue.kind.tier() {
                        GlueTier::Tier0 => 0,
                        _ => 1,
                    };
                    let mut capacity = Capacity::default();
                    capacity.stretch[tier] = glue.max_stretch;
                    capacity.shrink[tier] = glue.max_shrink;
                    capacity
                }
                PieceKind::Cluster { run, cluster } if Some(k) != last_cluster => {
                    let run = &self.ast.runs[run];
                    let flags = self.ast.clusters[cluster].flags;
                    let mut capacity = Capacity::default();
                    if flags.contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER) {
                        capacity.stretch[2] = CJK_GAP_STRETCH * run.font_size;
                    } else if !flags.contains(AtomFlags::NO_LETTER_SPACING)
                        && !expands_at_words_only(run.script)
                    {
                        capacity.stretch[3] = LETTER_GAP_STRETCH * run.font_size;
                    }
                    capacity
                }
                _ => Capacity::default(),
            })
            .collect()
    }

    /// The pieces of `body` after its last tab, which alone take part in
    /// justification (ST3-1).
    fn after_last_tab(body: &mut [PlacedPiece]) -> &mut [PlacedPiece] {
        let first = body
            .iter()
            .rposition(|p| matches!(p.piece.kind, PieceKind::Tab))
            .map_or(0, |tab| tab + 1);
        &mut body[first..]
    }

    /// `Z`: how much the glue after the line's last tab can shrink (ST1-7).
    pub(super) fn shrink_capacity(&self, body: &mut [PlacedPiece]) -> f32 {
        self.capacities(Self::after_last_tab(body))
            .iter()
            .map(|c| c.shrink.iter().sum::<f32>())
            .sum()
    }

    /// Spends `slack` on the body pieces after the line's last tab, level by
    /// level, each level shared in proportion to its capacities (ST3-2,
    /// ST3-3). Expansion after a cluster widens it; the cluster's glyphs
    /// place it by direction. Returns whether the line came out flush
    /// (ST3-4).
    pub(super) fn justify(&self, body: &mut [PlacedPiece], slack: f32) -> bool {
        let body = Self::after_last_tab(body);
        let capacities = self.capacities(body);
        let mut remaining = slack.abs();
        let level_count = if slack < 0.0 { 2 } else { LEVELS };
        for level in 0..level_count {
            let offered = |c: &Capacity| {
                if slack < 0.0 {
                    c.shrink[level]
                } else {
                    c.stretch[level]
                }
            };
            let total: f32 = capacities.iter().map(offered).sum();
            if remaining <= FIT_EPSILON || total <= 0.0 {
                continue;
            }
            let spent = remaining.min(total);
            for (p, c) in body.iter_mut().zip(&capacities) {
                let share = spent * offered(c) / total;
                let delta = if slack < 0.0 { -share } else { share };
                p.width += delta;
                if matches!(p.piece.kind, PieceKind::Cluster { .. }) {
                    p.expansion += delta;
                }
            }
            remaining -= spent;
        }
        remaining <= FIT_EPSILON
    }
}
