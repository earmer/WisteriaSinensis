//! Break opportunities and text box edges (spec §3 Step 6, FE-15, FE-19,
//! FE-22, FE-23, FE-30).

use super::spacing::Seam;
use super::units::{Unit, Units};
use super::{Compiler, primary_language};
use crate::frontend::{DiscretionaryNode, LineBreakStrictness, script, segment, url};
use crate::types::{AtomFlags, GraphemeAtom, LocaleTag};

/// Flags the atoms in `bound` as binding to their predecessor (FE-15). Runs
/// before shaping, which reads the flag.
pub(super) fn mark_bound_atoms(atoms: &mut [GraphemeAtom], bound: &[usize]) {
    for &a in bound {
        atoms[a].flags.insert(AtomFlags::BINDS_TO_PREVIOUS);
    }
}

impl Compiler<'_> {
    /// The atom that unit `i` binds to its predecessor (FE-15): unit `i` is
    /// an atom, the unit after it is an atom that touches it, shares its run
    /// and continues a dash run.
    fn atom_bound_by(&self, units: &Units, locales: &[LocaleTag], i: usize) -> Option<usize> {
        let (Unit::Atom(a), Some(Unit::Atom(b))) =
            (units.units[i], units.units.get(i + 1).copied())
        else {
            return None;
        };
        (b == a + 1
            && script::binds_to_next(self.text, &units.atoms, a)
            && self.same_run(units, locales, a, b))
        .then_some(b)
    }

    /// Every atom that binds to its predecessor (FE-15), for
    /// [`mark_bound_atoms`].
    pub(super) fn bound_atoms(&self, units: &Units, locales: &[LocaleTag]) -> Vec<usize> {
        (0..units.units.len())
            .filter_map(|i| self.atom_bound_by(units, locales, i))
            .collect()
    }

    /// Whether unit `i` is the word space after a listed one-letter word of
    /// its locale, which binds the word to the next (FE-23).
    fn binds_one_letter_word(&self, units: &Units, locales: &[LocaleTag], i: usize) -> bool {
        let (Some(Unit::Glue(..)), Some(&Unit::Atom(a))) = (
            units.units.get(i),
            i.checked_sub(1).and_then(|p| units.units.get(p)),
        ) else {
            return false;
        };
        let list = url::no_line_end_words(primary_language(&locales[a]));
        if list.is_empty() {
            return false;
        }
        let letter = &self.text[units.atoms[a].byte_range.as_range()];
        let mut chars = letter.chars();
        let Some(c) = chars.next().filter(|_| chars.next().is_none()) else {
            return false;
        };
        if !list.contains(c) {
            return false;
        }
        units.units[..i - 1]
            .iter()
            .rev()
            .find(|u| {
                !matches!(
                    u,
                    Unit::Atom(p) if units.atoms[*p].flags.contains(AtomFlags::IS_OPEN_PUNCT)
                )
            })
            .is_none_or(|u| !matches!(u, Unit::Atom(_)))
    }

    /// The penalty of the discretionary after each unit, if any (FE-19,
    /// WN-2, WN-3), with URL tailoring (FE-22), unbreakable dash runs and fused CJK middle
    /// dots (FE-15) and the no-line-end words (FE-23).
    pub(super) fn breaks_after(
        &self,
        units: &Units,
        locales: &[LocaleTag],
        graphemes: &[usize],
    ) -> Vec<Option<i32>> {
        let text = self.text;
        let opportunities = segment::line_opportunities(
            text,
            self.options.line_break_strictness,
            self.options.word_break_option,
            graphemes,
        );
        let urls = url::url_ranges(text);
        let added: Vec<(usize, i32)> = urls
            .iter()
            .flat_map(|&u| url::url_opportunities(text, u, graphemes))
            .collect();
        let mut breaks_after = Vec::with_capacity(units.units.len());
        let mut opp_at = 0;
        let has_urls = !urls.is_empty();
        for i in 0..units.units.len() {
            let unit = units.units[i];
            let end = unit.range(&units.atoms, self.input.inline_objects).end();
            if end >= text.len() || matches!(unit, Unit::HardBreak(_)) {
                breaks_after.push(None);
                continue;
            }
            // Only directly adjacent `Atom` units within the same `ShapingRun`
            // may bind under FE-15 (marked earlier by `mark_bound_atoms`).
            if let (Unit::Atom(a), Some(Unit::Atom(b))) = (unit, units.units.get(i + 1).copied())
                && b == a + 1
                && (units.atoms[b].flags.contains(AtomFlags::BINDS_TO_PREVIOUS)
                    || (units.atoms[a].byte_range.end() == units.atoms[b].byte_range.start()
                        && script::is_cjk_middle_dot(self.text, &units.atoms, b)
                        && self.options.line_break_strictness != LineBreakStrictness::Anywhere))
            {
                breaks_after.push(None);
                continue;
            }
            if self.binds_one_letter_word(units, locales, i) {
                breaks_after.push(None);
                continue;
            }
            while opp_at < opportunities.len() && opportunities[opp_at] < end {
                opp_at += 1;
            }
            let natural = opportunities.get(opp_at) == Some(&end);
            let penalty = if !has_urls {
                natural.then_some(DiscretionaryNode::NATURAL_PENALTY)
            } else {
                let tailored = added
                    .get(added.partition_point(|&(at, _)| at < end))
                    .filter(|&&(at, _)| at == end)
                    .map(|&(_, p)| p);
                let inside_url = urls
                    .get(urls.partition_point(|u| u.end() <= end))
                    .is_some_and(|u| u.start() < end && end < u.end());
                match (natural, tailored) {
                    (_, Some(p)) => Some(p),
                    (true, None) if inside_url => Some(DiscretionaryNode::URL_PENALTY_MIN),
                    (true, None) => Some(DiscretionaryNode::NATURAL_PENALTY),
                    (false, None) => None,
                }
            };
            breaks_after.push(penalty);
        }
        breaks_after
    }

    /// Whether each atom starts a text box: after any other unit, after a
    /// discretionary, at a style, script, locale, level or decoration edge,
    /// and at every cut the spacing and bidi analyses ask for
    /// ([`Self::box_cuts`]).
    pub(super) fn box_starts(
        &self,
        units: &Units,
        locales: &[LocaleTag],
        breaks_after: &[Option<i32>],
        candidates: &[Seam],
    ) -> Vec<bool> {
        let cuts = self.box_cuts(&units.atoms, candidates);
        let mut box_starts = vec![false; units.atoms.len()];
        let mut previous: Option<(Unit, bool)> = None;
        for (i, unit) in units.units.iter().enumerate() {
            if let Unit::Atom(a) = *unit {
                let at = units.atoms[a].byte_range.start();
                box_starts[a] = match previous {
                    Some((Unit::Atom(p), broke)) => {
                        broke
                            || !self.same_run(units, locales, p, a)
                            || cuts.binary_search(&at).is_ok()
                    }
                    _ => true,
                };
            }
            previous = Some((*unit, breaks_after[i].is_some()));
        }
        box_starts
    }

    /// Offsets where a text box must start besides the run edges, ascending:
    /// the edges of decoration spans, those of explicit bidi spans (FE-30) and the
    /// seams of synthetic glue, compression candidates included (FE-31).
    fn box_cuts(&self, atoms: &[GraphemeAtom], candidates: &[Seam]) -> Vec<usize> {
        let input = self.input;
        let mut cuts: Vec<usize> = (input.decoration_spans.iter().map(|s| s.byte_range))
            .chain(input.bidi_spans.iter().map(|s| s.byte_range))
            .flat_map(|r| [r.start(), r.end()])
            .chain(
                candidates
                    .iter()
                    .map(|seam| atoms[seam.after].byte_range.end()),
            )
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        cuts
    }
}
