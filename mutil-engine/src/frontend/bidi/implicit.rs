//! UAX #9 weak types, neutrals, paired brackets and implicit levels (W1 to
//! W7, N0 to N2, I1 and I2) on one isolating run sequence (spec FE-30).

use super::Bracket;
use super::explicit::{RunSequence, direction_class, is_isolate_initiator};
use crate::types::BidiClass;

/// BD16's bracket stack depth.
const MAX_BRACKET_DEPTH: usize = 63;

/// What the implicit rules read of the paragraph.
pub(super) struct Context<'a> {
    /// The classes after X1 to X8, overrides applied.
    pub(super) classes: &'a [BidiClass],
    /// The classes of the text, before any rule.
    pub(super) original: &'a [BidiClass],
    pub(super) brackets: &'a [Bracket],
}

/// Resolves one sequence and writes its final levels into `levels`.
pub(super) fn resolve_sequence(ctx: &Context<'_>, seq: &RunSequence, levels: &mut [u8]) {
    let mut types: Vec<BidiClass> = seq.indices.iter().map(|&i| ctx.classes[i]).collect();
    resolve_weak(&mut types, seq);
    resolve_brackets(ctx, seq, &mut types);
    resolve_neutrals(&mut types, seq);
    for (&i, &t) in seq.indices.iter().zip(&types) {
        let odd = seq.level % 2 == 1;
        levels[i] = seq.level
            + match (odd, t) {
                (false, BidiClass::R) => 1,
                (false, BidiClass::An | BidiClass::En) => 2,
                (true, BidiClass::L | BidiClass::En | BidiClass::An) => 1,
                _ => 0,
            };
    }
}

/// The strong class of a type for the neutral rules: EN and AN count as R.
fn strong_for_neutrals(t: BidiClass) -> Option<BidiClass> {
    match t {
        BidiClass::L => Some(BidiClass::L),
        BidiClass::R | BidiClass::Al | BidiClass::En | BidiClass::An => Some(BidiClass::R),
        _ => None,
    }
}

/// W1 to W7.
fn resolve_weak(t: &mut [BidiClass], seq: &RunSequence) {
    // W1: NSM takes the previous type, ON after an isolate initiator or PDI.
    for k in 0..t.len() {
        if t[k] == BidiClass::Nsm {
            t[k] = match k.checked_sub(1).map(|p| t[p]) {
                None => seq.sos,
                Some(p) if is_isolate_initiator(p) || p == BidiClass::Pdi => BidiClass::On,
                Some(p) => p,
            };
        }
    }
    // W2 and W3, with the last strong type seen.
    let mut last_strong = seq.sos;
    for x in t.iter_mut() {
        match *x {
            BidiClass::L | BidiClass::R => last_strong = *x,
            BidiClass::Al => {
                last_strong = BidiClass::Al;
                *x = BidiClass::R;
            }
            BidiClass::En if last_strong == BidiClass::Al => *x = BidiClass::An,
            _ => {}
        }
    }
    // W4: a single separator between two numbers of one type.
    for k in 1..t.len().saturating_sub(1) {
        let (before, after) = (t[k - 1], t[k + 1]);
        t[k] = match (t[k], before, after) {
            (BidiClass::Es, BidiClass::En, BidiClass::En)
            | (BidiClass::Cs, BidiClass::En, BidiClass::En) => BidiClass::En,
            (BidiClass::Cs, BidiClass::An, BidiClass::An) => BidiClass::An,
            (x, ..) => x,
        };
    }
    // W5: terminators next to a European number.
    let mut k = 0;
    while k < t.len() {
        if t[k] != BidiClass::Et {
            k += 1;
            continue;
        }
        let end = (k..t.len())
            .find(|&e| t[e] != BidiClass::Et)
            .unwrap_or(t.len());
        let touches_en = (k > 0 && t[k - 1] == BidiClass::En) || t.get(end) == Some(&BidiClass::En);
        if touches_en {
            t[k..end].fill(BidiClass::En);
        }
        k = end;
    }
    // W6 and W7.
    let mut last_strong = seq.sos;
    for x in t.iter_mut() {
        match *x {
            BidiClass::Es | BidiClass::Et | BidiClass::Cs => *x = BidiClass::On,
            BidiClass::L | BidiClass::R => last_strong = *x,
            BidiClass::En if last_strong == BidiClass::L => *x = BidiClass::L,
            _ => {}
        }
    }
}

/// BD16: the bracket pairs of the sequence, by position in it, in order of
/// their openers.
fn bracket_pairs(ctx: &Context<'_>, seq: &RunSequence, t: &[BidiClass]) -> Vec<(usize, usize)> {
    let mut openers: Vec<(u32, usize)> = Vec::new();
    let mut pairs = Vec::new();
    for (k, &i) in seq.indices.iter().enumerate() {
        if t[k] != BidiClass::On {
            continue;
        }
        match ctx.brackets[i] {
            Bracket::Open(key) => {
                if openers.len() == MAX_BRACKET_DEPTH {
                    break;
                }
                openers.push((key, k));
            }
            Bracket::Close(key) => {
                if let Some(depth) = openers.iter().rposition(|&(open, _)| open == key) {
                    pairs.push((openers[depth].1, k));
                    openers.truncate(depth);
                }
            }
            Bracket::None => {}
        }
    }
    pairs.sort_unstable();
    pairs
}

/// N0: paired brackets take the direction their content and context give.
fn resolve_brackets(ctx: &Context<'_>, seq: &RunSequence, t: &mut [BidiClass]) {
    let embedding = direction_class(seq.level);
    for (open, close) in bracket_pairs(ctx, seq, t) {
        let inside = || {
            t[open + 1..close]
                .iter()
                .filter_map(|&x| strong_for_neutrals(x))
        };
        // N0 c: opposite strong types only take the direction of the context.
        let direction = if inside().any(|strong| strong == embedding) {
            embedding
        } else if inside().next().is_some() {
            t[..open]
                .iter()
                .rev()
                .find_map(|&x| strong_for_neutrals(x))
                .unwrap_or(seq.sos)
        } else {
            continue;
        };
        for bracket in [open, close] {
            t[bracket] = direction;
            // Marks that followed the bracket in the text follow it now.
            let marks = seq.indices[bracket + 1..]
                .iter()
                .take_while(|&&i| ctx.original[i] == BidiClass::Nsm)
                .count();
            t[bracket + 1..bracket + 1 + marks].fill(direction);
        }
    }
}

/// Whether a type is a neutral or isolate formatting character (NI).
fn is_neutral(t: BidiClass) -> bool {
    matches!(
        t,
        BidiClass::B
            | BidiClass::S
            | BidiClass::Ws
            | BidiClass::On
            | BidiClass::Fsi
            | BidiClass::Lri
            | BidiClass::Rli
            | BidiClass::Pdi
    )
}

/// N1 and N2.
fn resolve_neutrals(t: &mut [BidiClass], seq: &RunSequence) {
    let embedding = direction_class(seq.level);
    let mut k = 0;
    while k < t.len() {
        if !is_neutral(t[k]) {
            k += 1;
            continue;
        }
        let end = (k..t.len()).find(|&e| !is_neutral(t[e])).unwrap_or(t.len());
        let before = k
            .checked_sub(1)
            .map_or(Some(seq.sos), |p| strong_for_neutrals(t[p]));
        let after = t
            .get(end)
            .map_or(Some(seq.eos), |&x| strong_for_neutrals(x));
        let direction = match (before, after) {
            (Some(b), Some(a)) if a == b => b,
            _ => embedding,
        };
        t[k..end].fill(direction);
        k = end;
    }
}
