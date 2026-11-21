//! Stage 3 justification (spec ST3-2 to ST3-5): the priority ladder that
//! spends a line's slack on its glue and inter-cluster gaps.

use super::scratch::LogicalPiece;
use super::trim::is_trim_transparent;
use super::{Layout, PieceKind, Reshapes};
use crate::frontend::{GlueKind, GlueTier};
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

/// Effective shrink capacity per piece in `body`, deducting static CJK
/// punctuation trims already applied across `GlueKind::CjkPunctCompression`
/// seams so justification never double-shrinks a statically trimmed pair.
fn effective_shrinks(
    body: &[LogicalPiece<'_>],
    lead_trims: &[f32],
    trail_trims: &[f32],
) -> Vec<f32> {
    let mut prev_trails = Vec::with_capacity(body.len());
    let mut prev_trail = 0.0_f32;
    for (p, &trail) in body.iter().zip(trail_trims) {
        prev_trails.push(prev_trail);
        if !is_trim_transparent(&p.piece) {
            prev_trail = match p.piece.kind {
                PieceKind::Cluster { .. } => trail,
                _ => 0.0,
            };
        }
    }
    let mut shrinks = vec![0.0_f32; body.len()];
    let mut next_lead = 0.0_f32;
    for (((p, &lead), &prev_t), out) in body
        .iter()
        .zip(lead_trims)
        .zip(&prev_trails)
        .zip(shrinks.iter_mut())
        .rev()
    {
        if let PieceKind::Glue(glue) = p.piece.kind {
            *out = if glue.kind == GlueKind::CjkPunctCompression {
                (glue.max_shrink - (prev_t + next_lead)).max(0.0)
            } else {
                glue.max_shrink
            };
        }
        if !is_trim_transparent(&p.piece) {
            next_lead = match p.piece.kind {
                PieceKind::Cluster { .. } => lead,
                _ => 0.0,
            };
        }
    }
    shrinks
}

impl Layout<'_, '_> {
    /// The capacities of the pieces after the line's last tab (ST3-1): glue
    /// by its tier, and the gap after each cluster that another cluster
    /// follows, with no inline object between them (ST1-7, ST3-2).
    fn capacities(
        &self,
        body: &[LogicalPiece<'_>],
        lead_trims: &[f32],
        trail_trims: &[f32],
        pending: &Reshapes,
    ) -> Vec<Capacity> {
        let shrinks = effective_shrinks(body, lead_trims, trail_trims);
        let mut capacities = vec![Capacity::default(); body.len()];
        let mut next_is_cluster = false;
        for ((p, &shrink), cap) in body.iter().zip(&shrinks).zip(capacities.iter_mut()).rev() {
            *cap = match p.piece.kind {
                PieceKind::Glue(glue) => Capacity {
                    level: usize::from(glue.kind.tier() != GlueTier::Tier0),
                    stretch: glue.max_stretch,
                    shrink,
                },
                PieceKind::Cluster { run, cluster }
                    if next_is_cluster && !self.cluster_glyphs(pending, cluster).0.is_empty() =>
                {
                    let run = self.inputs.run(run);
                    let flags = self.inputs.cluster(cluster).flags;
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
            };
            match p.piece.kind {
                PieceKind::Cluster { .. } => next_is_cluster = true,
                PieceKind::Object(_) => next_is_cluster = false,
                _ => {}
            }
        }
        capacities
    }

    /// The index of the first piece of `body` after its last tab; only
    /// those take part in justification (ST3-1).
    fn after_last_tab(body: &[LogicalPiece<'_>]) -> usize {
        body.iter()
            .rposition(|p| matches!(p.piece.kind, PieceKind::Tab))
            .map_or(0, |tab| tab + 1)
    }

    /// `Z`: how much the glue after the line's last tab can shrink (ST1-7).
    pub(super) fn shrink_capacity(
        body: &[LogicalPiece<'_>],
        lead_trims: &[f32],
        trail_trims: &[f32],
    ) -> f32 {
        let first = Self::after_last_tab(body);
        effective_shrinks(
            body.get(first..).unwrap_or_default(),
            lead_trims.get(first..).unwrap_or_default(),
            trail_trims.get(first..).unwrap_or_default(),
        )
        .into_iter()
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
        [lead_trims, trail_trims]: [&[f32]; 2],
        slack: f32,
        pending: &Reshapes,
    ) -> bool {
        let first = Self::after_last_tab(body);
        let (body, widths, expansions, lead_trims, trail_trims) = (
            body.get(first..).unwrap_or_default(),
            widths.get_mut(first..).unwrap_or_default(),
            expansions.get_mut(first..).unwrap_or_default(),
            lead_trims.get(first..).unwrap_or_default(),
            trail_trims.get(first..).unwrap_or_default(),
        );
        let capacities = self.capacities(body, lead_trims, trail_trims, pending);
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
