//! Break opportunities and text box edges (spec §3 Step 6, FE-15, FE-19,
//! FE-22, FE-23, FE-30).

use super::super::{DiscretionaryNode, MAX_DECORATION_NESTING_DEPTH, script, segment, url};
use super::{Compiler, Unit};
use crate::types::AtomFlags;
use smallvec::SmallVec;

impl Compiler<'_> {
    /// The penalty of the discretionary after each unit, if any (FE-19,
    /// WN-2, WN-3), with URL tailoring (FE-22), unbreakable dash runs (FE-15)
    /// and the no-line-end words (FE-23).
    pub(super) fn breaks_after(&mut self, graphemes: &[usize]) -> Vec<Option<i32>> {
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
        let mut breaks_after = Vec::with_capacity(self.units.len());
        let mut opp_idx = 0usize;
        let mut add_idx = 0usize;
        let mut url_idx = 0usize;
        for i in 0..self.units.len() {
            let unit = self.units[i];
            let end = unit.range(&self.atoms, self.input.inline_objects).end();
            if end >= text.len() || matches!(unit, Unit::HardBreak(_)) {
                breaks_after.push(None);
                continue;
            }
            // Only directly adjacent `Atom` units within the same `ShapingRun`
            // may bind under FE-15 (never across `Glue`, `Tab`, `HardBreak`,
            // or `Object`).
            if let (Unit::Atom(a), Some(Unit::Atom(b))) = (unit, self.units.get(i + 1).copied())
                && b == a + 1
                && self.same_run(a, b)
                && script::binds_to_next(self.text, &self.atoms, a)
            {
                self.atoms[b].flags.insert(AtomFlags::BINDS_TO_PREVIOUS);
                breaks_after.push(None);
                continue;
            }
            if self.binds_one_letter_word(i) {
                breaks_after.push(None);
                continue;
            }
            while opp_idx < opportunities.len() && opportunities[opp_idx] < end {
                opp_idx += 1;
            }
            let natural = opportunities.get(opp_idx).copied() == Some(end);
            while add_idx < added.len() && added[add_idx].0 < end {
                add_idx += 1;
            }
            let tailored = added
                .get(add_idx)
                .filter(|&&(at, _)| at == end)
                .map(|&(_, p)| p);
            while url_idx < urls.len() && urls[url_idx].end() <= end {
                url_idx += 1;
            }
            let penalty = match (natural, tailored) {
                (_, Some(p)) => Some(p),
                (true, None)
                    if urls
                        .get(url_idx)
                        .is_some_and(|u| u.start() < end && end < u.end()) =>
                {
                    Some(DiscretionaryNode::URL_PENALTY_MIN)
                }
                (true, None) => Some(DiscretionaryNode::NATURAL_PENALTY),
                (false, None) => None,
            };
            breaks_after.push(penalty);
        }
        breaks_after
    }

    /// Whether each atom starts a text box: after any other unit, after a
    /// discretionary, at a style, script, locale, level or decoration edge,
    /// and at every cut the spacing and bidi analyses ask for
    /// ([`Self::box_cuts`]).
    pub(super) fn box_starts(&self, breaks_after: &[Option<i32>]) -> Vec<bool> {
        let cuts = self.box_cuts();
        let mut box_starts = vec![false; self.atoms.len()];
        let dec_spans = self.input.decoration_spans;
        let mut dec_next = 0usize;
        let mut dec_ends: SmallVec<[usize; MAX_DECORATION_NESTING_DEPTH]> = SmallVec::new();
        let mut has_decoration_edge_at = |at: usize| {
            let mut edge = false;
            while let Some(&end) = dec_ends.last() {
                if end > at {
                    break;
                }
                if end == at {
                    edge = true;
                }
                dec_ends.pop();
            }
            while dec_next < dec_spans.len() && dec_spans[dec_next].byte_range.start() <= at {
                let r = dec_spans[dec_next].byte_range;
                if r.start() == at || r.end() == at {
                    edge = true;
                }
                if r.end() > at {
                    while dec_ends.last().is_some_and(|&end| end <= r.start()) {
                        dec_ends.pop();
                    }
                    dec_ends.push(r.end());
                }
                dec_next += 1;
            }
            edge
        };
        let mut previous: Option<(Unit, bool)> = None;
        for (i, unit) in self.units.iter().enumerate() {
            if let Unit::Atom(a) = *unit {
                let at = self.atoms[a].byte_range.start();
                box_starts[a] = match previous {
                    Some((Unit::Atom(p), broke)) => {
                        broke
                            || !self.same_run(p, a)
                            || has_decoration_edge_at(at)
                            || cuts.binary_search(&at).is_ok()
                    }
                    _ => true,
                };
            }
            previous = Some((*unit, breaks_after[i].is_some()));
        }
        box_starts
    }

    /// Offsets where a text box must start besides the run and decoration
    /// edges, ascending: the edges of explicit bidi spans (FE-30) and the
    /// seams of synthetic glue, compression candidates included (FE-31).
    fn box_cuts(&self) -> Vec<usize> {
        let mut cuts: Vec<usize> = self
            .input
            .bidi_spans
            .iter()
            .flat_map(|span| [span.byte_range.start(), span.byte_range.end()])
            .chain(
                self.seams
                    .iter()
                    .map(|seam| self.atoms[seam.after].byte_range.end()),
            )
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        cuts
    }
}
