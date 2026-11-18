//! Phase 2 spacing (spec FE-20, FE-31): the flags set after Step 4 and the
//! seams where synthetic glue goes.

use super::units::{Unit, Units};
use super::{Compiler, Shaped};
use crate::frontend::{AutospaceProfile, GlueKind, GlueNode, WordNode, spacing};
use crate::types::{AtomFlags, BidiClass, GraphemeAtom, LocaleTag, ScriptTag, TextRange};

/// Share of the run's font size each cluster of a compressible pair must
/// exceed, or the shaper has already compressed the pair (FE-31).
const COMPRESSED_PAIR_FRACTION: f32 = 0.75;

/// Synthetic glue after an atom (FE-20, FE-31).
#[derive(Clone, Copy, Debug)]
pub(super) struct Seam {
    /// The atom the glue follows.
    pub(super) after: usize,
    pub(super) kind: SeamKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SeamKind {
    /// `CjkAutospace`, with the font size of the CJK-class atom's style.
    Autospace { em: f32 },
    /// `CjkPunctCompression`, with the font size of the pair's run and its
    /// shrink share (`0.5` for full-width pairs, `0.25` for middle-dot pairs).
    Compression {
        font_size: f32,
        shrink_fraction: f32,
    },
}

/// The atoms that touch in bytes, no other unit between them: the first atom
/// of each pair.
fn touching_pairs(units: &[Unit]) -> impl Iterator<Item = usize> + '_ {
    units.windows(2).filter_map(|w| match (w[0], w[1]) {
        (Unit::Atom(a), Unit::Atom(b)) if b == a + 1 => Some(a),
        _ => None,
    })
}

/// The first code point of atom `a`.
fn first_char(text: &str, atoms: &[GraphemeAtom], a: usize) -> char {
    text[atoms[a].byte_range.start()..]
        .chars()
        .next()
        .expect("an atom is not empty")
}

impl Compiler<'_> {
    /// FE-31's flags from each atom's first code point and resolved script.
    pub(super) fn mark_spacing_flags(&self, atoms: &mut [GraphemeAtom]) {
        for a in 0..atoms.len() {
            let first = first_char(self.text, atoms, a);
            let atom = &mut atoms[a];
            let cursive = spacing::takes_no_letter_spacing(atom.script);
            atom.flags.set(AtomFlags::NO_LETTER_SPACING, cursive);
            atom.flags
                .set(AtomFlags::KASHIDA_CAPABLE, atom.script == ScriptTag::ARABIC);
            atom.flags
                .set(AtomFlags::HANGABLE_START, spacing::is_hangable_start(first));
            atom.flags
                .set(AtomFlags::HANGABLE_END, spacing::is_hangable_end(first));
        }
    }

    /// The candidate seams, ascending: autospace between a CJK-class and a
    /// Western-class atom (FE-20), and compression between two full-width
    /// punctuation atoms of one run where the locale allows it (FE-31).
    pub(super) fn candidate_seams(&self, units: &Units, locales: &[LocaleTag]) -> Vec<Seam> {
        // The offsets where a padded span edge forbids autospace (SPN-4).
        let mut padded: Vec<usize> = self
            .input
            .decoration_spans
            .iter()
            .flat_map(|span| {
                let r = span.byte_range;
                [r.start(), r.end()]
                    .into_iter()
                    .filter(|&at| span.forbids_cjk_autospace_at(at))
            })
            .collect();
        padded.sort_unstable();
        touching_pairs(&units.units)
            .filter_map(|a| {
                let at = units.atoms[a].byte_range.end();
                let padded_edge = padded.binary_search(&at).is_ok();
                self.seam_after(units, locales, a, padded_edge)
                    .map(|kind| Seam { after: a, kind })
            })
            .collect()
    }

    /// The seam between touching atoms `a` and `a + 1`, if any.
    fn seam_after(
        &self,
        units: &Units,
        locales: &[LocaleTag],
        a: usize,
        padded_edge: bool,
    ) -> Option<SeamKind> {
        use spacing::AutospaceClass::{Cjk, Western};
        let (left, right) = (
            first_char(self.text, &units.atoms, a),
            first_char(self.text, &units.atoms, a + 1),
        );
        let cjk_atom = match (
            spacing::autospace_class(left),
            spacing::autospace_class(right),
        ) {
            (Some(Cjk), Some(Western)) => Some(a),
            (Some(Western), Some(Cjk)) => Some(a + 1),
            _ => None,
        };
        if let Some(c) = cjk_atom {
            return (!padded_edge).then(|| SeamKind::Autospace {
                em: self.styles[units.atom_styles[c]].font_size,
            });
        }
        let shrink_fraction = if ((spacing::is_compressible_close(left)
            || spacing::is_wide_opening(left))
            && spacing::is_middle_dot(right))
            || (spacing::is_middle_dot(left)
                && (spacing::is_compressible_close(right) || spacing::is_wide_opening(right)))
        {
            Some(0.25)
        } else if (spacing::is_compressible_close(left) && spacing::is_wide_punctuation(right))
            || (spacing::is_wide_opening(left) && spacing::is_wide_opening(right))
        {
            Some(0.5)
        } else {
            None
        };
        shrink_fraction
            .filter(|_| {
                self.same_run(units, locales, a, a + 1)
                    && !spacing::turns_compression_off(&locales[a])
            })
            .map(|shrink_fraction| SeamKind::Compression {
                font_size: self.styles[units.atom_styles[a]].font_size,
                shrink_fraction,
            })
    }

    /// `ALLOW_CJK_JUSTIFY_AFTER` on a CJK-class atom that another atom
    /// follows with nothing between them and that no candidate seam follows,
    /// not closing or prohibited-start punctuation (FE-31).
    pub(super) fn mark_cjk_justification(
        &self,
        units: &[Unit],
        atoms: &mut [GraphemeAtom],
        candidates: &[Seam],
    ) {
        let allowed: Vec<usize> = touching_pairs(units)
            .filter(|&a| {
                let next = first_char(self.text, atoms, a + 1);
                candidates.binary_search_by_key(&a, |s| s.after).is_err()
                    && spacing::autospace_class(first_char(self.text, atoms, a))
                        == Some(spacing::AutospaceClass::Cjk)
                    && !atoms[a + 1].flags.contains(AtomFlags::IS_CLOSE_PUNCT)
                    && !spacing::is_prohibited_start(next)
            })
            .collect();
        for a in allowed {
            atoms[a].flags.insert(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER);
        }
    }

    /// The seams without the compression ones whose pair the shaper already
    /// compressed: both clusters must be wider than 3/4 of the run's font
    /// size (FE-31).
    pub(super) fn confirmed_seams(&self, candidates: &[Seam], shaped: &Shaped) -> Vec<Seam> {
        let width_of = |global_atom: usize, ending: bool| -> f32 {
            let run = &shaped.runs[shaped
                .runs
                .partition_point(|r| r.atom_range.end <= global_atom)];
            let local = global_atom - run.atom_range.start;
            // Clusters ascend by atoms, and the seam cut the box there.
            let clusters = &shaped.clusters[run.cluster_range.clone()];
            let at = clusters.partition_point(|c| c.atom_range.end <= local);
            clusters
                .get(at)
                .filter(|c| {
                    if ending {
                        c.atom_range.end == local + 1
                    } else {
                        c.atom_range.start == local
                    }
                })
                .map_or(0.0, |c| c.natural_width)
        };
        candidates
            .iter()
            .copied()
            .filter(|seam| match seam.kind {
                SeamKind::Autospace { .. } => true,
                SeamKind::Compression { font_size, .. } => {
                    let floor = COMPRESSED_PAIR_FRACTION * font_size;
                    width_of(seam.after, true) > floor && width_of(seam.after + 1, false) > floor
                }
            })
            .collect()
    }

    /// The glue of a seam, at the level of the atom before it (WN-10).
    pub(super) fn seam_node(&self, seam: Seam, atoms: &[GraphemeAtom]) -> WordNode {
        let atom = &atoms[seam.after];
        let (kind, natural_width, max_stretch, max_shrink) = match seam.kind {
            SeamKind::Autospace { em } => match self.options.autospace_profile {
                AutospaceProfile::Jlreq => (GlueKind::CjkAutospace, em / 4.0, em / 4.0, em / 8.0),
                AutospaceProfile::CssCompat => (GlueKind::CjkAutospace, em / 8.0, 0.0, 0.0),
            },
            SeamKind::Compression {
                font_size,
                shrink_fraction,
            } => (
                GlueKind::CjkPunctCompression,
                0.0,
                0.0,
                font_size * shrink_fraction,
            ),
        };
        WordNode::Glue(GlueNode {
            original_bidi_class: BidiClass::Bn,
            bidi_level: atom.bidi_level,
            is_collapsible: true,
            byte_range: TextRange::empty_at(atom.byte_range.end()),
            kind,
            natural_width,
            max_stretch,
            max_shrink,
        })
    }
}
