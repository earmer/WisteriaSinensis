//! Stage 3 justification priority ladder (spec ST3-2 to ST3-5).

use super::scratch::LogicalPiece;
use super::trim::is_trim_transparent;
use super::{Layout, PieceKind, Reshapes};
use crate::frontend::{GlueKind, GlueTier};
use crate::middleend::FIT_EPSILON;
use crate::types::{AtomFlags, ScriptTag};

pub(super) const CJK_GAP_STRETCH: f64 = 0.25;
const LETTER_GAP_STRETCH: f64 = 0.125;
const LEVELS: usize = 4;

#[derive(Clone, Copy, Default)]
struct Capacity {
    level: usize,
    stretch: f64,
    shrink: f64,
}

fn expands_at_words_only(script: ScriptTag) -> bool {
    matches!(&script.as_bytes(), b"Thai" | b"Laoo" | b"Khmr" | b"Mymr")
}

// Deduct static CJK punctuation trims across CjkPunctCompression seams so justification never double-shrinks.
fn effective_shrinks(
    body: &[LogicalPiece<'_>],
    lead_trims: &[f64],
    trail_trims: &[f64],
) -> Vec<f64> {
    let mut prev_trails = Vec::with_capacity(body.len());
    let mut prev_trail = 0.0_f64;
    for (p, &trail) in body.iter().zip(trail_trims) {
        prev_trails.push(prev_trail);
        if !is_trim_transparent(&p.piece) {
            prev_trail = match p.piece.kind {
                PieceKind::Cluster { .. } => trail,
                _ => 0.0,
            };
        }
    }
    let mut shrinks = prev_trails;
    let mut next_lead = 0.0_f64;
    for ((piece, &lead), slot) in body.iter().zip(lead_trims).zip(&mut shrinks).rev() {
        let prev_trail = *slot;
        *slot = match piece.piece.kind {
            PieceKind::Glue(glue) if glue.kind == GlueKind::CjkPunctCompression => {
                (f64::from(glue.max_shrink) - (prev_trail + next_lead)).max(0.0)
            }
            PieceKind::Glue(glue) => f64::from(glue.max_shrink),
            _ => 0.0,
        };
        if !is_trim_transparent(&piece.piece) {
            next_lead = match piece.piece.kind {
                PieceKind::Cluster { .. } => lead,
                _ => 0.0,
            };
        }
    }
    shrinks
}

impl Layout<'_, '_> {
    fn capacities(
        &self,
        body: &[LogicalPiece<'_>],
        lead_trims: &[f64],
        trail_trims: &[f64],
        pending: &Reshapes,
    ) -> Vec<Capacity> {
        let shrinks = effective_shrinks(body, lead_trims, trail_trims);
        let mut capacities = vec![Capacity::default(); body.len()];
        let mut next_is_cluster = false;
        for ((p, &shrink), cap) in body.iter().zip(&shrinks).zip(capacities.iter_mut()).rev() {
            *cap = match p.piece.kind {
                PieceKind::Glue(glue) => Capacity {
                    level: usize::from(glue.kind.tier() != GlueTier::Tier0),
                    stretch: f64::from(glue.max_stretch),
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
                            stretch: CJK_GAP_STRETCH * f64::from(run.font_size),
                            shrink: 0.0,
                        }
                    } else if !flags.contains(AtomFlags::NO_LETTER_SPACING)
                        && !expands_at_words_only(run.script)
                    {
                        Capacity {
                            level: 3,
                            stretch: LETTER_GAP_STRETCH * f64::from(run.font_size),
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

    fn after_last_tab(body: &[LogicalPiece<'_>]) -> usize {
        body.iter()
            .rposition(|p| matches!(p.piece.kind, PieceKind::Tab))
            .map_or(0, |tab| tab + 1)
    }

    pub(super) fn shrink_capacity(
        body: &[LogicalPiece<'_>],
        lead_trims: &[f64],
        trail_trims: &[f64],
    ) -> f64 {
        let first = Self::after_last_tab(body);
        effective_shrinks(
            body.get(first..).unwrap_or_default(),
            lead_trims.get(first..).unwrap_or_default(),
            trail_trims.get(first..).unwrap_or_default(),
        )
        .into_iter()
        .sum()
    }

    pub(super) fn justify(
        &self,
        body: &[LogicalPiece<'_>],
        widths: &mut [f64],
        expansions: &mut [f64],
        [lead_trims, trail_trims]: [&[f64]; 2],
        slack: f64,
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
        let fit_eps = f64::from(FIT_EPSILON);
        let mut remaining = slack.abs();
        let level_count = if slack < 0.0 { 2 } else { LEVELS };
        for level in 0..level_count {
            let offered = |c: &Capacity| match (c.level == level, slack < 0.0) {
                (false, _) => 0.0,
                (true, true) => c.shrink,
                (true, false) => c.stretch,
            };
            let total: f64 = capacities.iter().map(offered).sum();
            if remaining <= fit_eps || total <= 0.0 {
                continue;
            }
            let spent = remaining.min(total);
            let mut cum_cap = 0.0_f64;
            let mut cum_out = 0.0_f64;
            let columns = widths.iter_mut().zip(expansions.iter_mut());
            for ((p, c), (width, expansion)) in body.iter().zip(&capacities).zip(columns) {
                let cap = offered(c);
                if cap <= 0.0 {
                    continue;
                }
                cum_cap += cap;
                let target = spent * (cum_cap / total);
                let share = target - cum_out;
                cum_out = target;
                let delta = if slack < 0.0 { -share } else { share };
                *width += delta;
                if matches!(p.piece.kind, PieceKind::Cluster { .. }) {
                    *expansion += delta;
                }
            }
            remaining -= spent;
        }
        remaining <= fit_eps
    }
}
