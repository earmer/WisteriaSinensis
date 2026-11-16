//! Phase 2 spacing (spec FE-20, FE-31): the flags set after Step 4 and the
//! seams where synthetic glue goes.

use super::super::{AutospaceProfile, GlueKind, GlueNode, ParagraphAst, WordNode, spacing};
use super::{Compiler, Unit};
use crate::types::{AtomFlags, BidiClass, ScriptTag, TextRange};

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
    /// `CjkPunctCompression`, with the font size of the pair's run.
    Compression { font_size: f32 },
}

impl Compiler<'_> {
    /// The first code point of atom `a`.
    fn first_char(&self, a: usize) -> char {
        self.text[self.atoms[a].byte_range.start()..]
            .chars()
            .next()
            .expect("an atom is not empty")
    }

    /// FE-31's flags from each atom's first code point and resolved script,
    /// then the seams, then `ALLOW_CJK_JUSTIFY_AFTER`, which reads them.
    pub(super) fn resolve_spacing(&mut self) {
        for a in 0..self.atoms.len() {
            let first = self.first_char(a);
            let atom = &mut self.atoms[a];
            let cursive = spacing::takes_no_letter_spacing(atom.script);
            atom.flags.set(AtomFlags::NO_LETTER_SPACING, cursive);
            atom.flags
                .set(AtomFlags::KASHIDA_CAPABLE, atom.script == ScriptTag::ARABIC);
            atom.flags
                .set(AtomFlags::HANGABLE_START, spacing::is_hangable_start(first));
            atom.flags
                .set(AtomFlags::HANGABLE_END, spacing::is_hangable_end(first));
        }
        self.seams = self.find_seams();
        self.allow_cjk_justification();
    }

    /// The pairs of atoms that touch in bytes, no other unit between them.
    fn touching_pairs(&self) -> impl Iterator<Item = usize> + '_ {
        self.units.windows(2).filter_map(|w| match (w[0], w[1]) {
            (Unit::Atom(a), Unit::Atom(b)) if b == a + 1 => Some(a),
            _ => None,
        })
    }

    /// The candidate seams, ascending: autospace between a CJK-class and a
    /// Western-class atom (FE-20), and compression between two full-width
    /// punctuation atoms of one run where the locale allows it (FE-31).
    fn find_seams(&self) -> Vec<Seam> {
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
        self.touching_pairs()
            .filter_map(|a| {
                let at = self.atoms[a].byte_range.end();
                let padded_edge = padded.binary_search(&at).is_ok();
                self.seam_after(a, padded_edge)
                    .map(|kind| Seam { after: a, kind })
            })
            .collect()
    }

    /// The seam between touching atoms `a` and `a + 1`, if any.
    fn seam_after(&self, a: usize, padded_edge: bool) -> Option<SeamKind> {
        use spacing::AutospaceClass::{Cjk, Western};
        let (left, right) = (self.first_char(a), self.first_char(a + 1));
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
                em: self.styles[self.atom_styles[c]].font_size,
            });
        }
        let compressible = (spacing::is_compressible_close(left)
            && spacing::is_wide_punctuation(right))
            || (spacing::is_wide_opening(left) && spacing::is_wide_opening(right));
        (compressible
            && self.same_run(a, a + 1)
            && !spacing::turns_compression_off(&self.atom_locales[a]))
        .then(|| SeamKind::Compression {
            font_size: self.styles[self.atom_styles[a]].font_size,
        })
    }

    /// `ALLOW_CJK_JUSTIFY_AFTER` on a CJK-class atom that another atom
    /// follows with nothing between them, not closing or prohibited-start
    /// punctuation (FE-31).
    fn allow_cjk_justification(&mut self) {
        let mut seams = self.seams.iter().map(|s| s.after).peekable();
        let allowed: Vec<usize> = self
            .touching_pairs()
            .filter(|&a| {
                while seams.next_if(|&s| s < a).is_some() {}
                let next = self.first_char(a + 1);
                seams.peek() != Some(&a)
                    && spacing::autospace_class(self.first_char(a))
                        == Some(spacing::AutospaceClass::Cjk)
                    && !self.atoms[a + 1].flags.contains(AtomFlags::IS_CLOSE_PUNCT)
                    && !spacing::is_prohibited_start(next)
            })
            .collect();
        for a in allowed {
            self.atoms[a]
                .flags
                .insert(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER);
        }
    }

    /// Drops the compression seams whose pair the shaper already compressed:
    /// both clusters must be wider than 3/4 of the run's font size (FE-31).
    pub(super) fn confirm_compression(&mut self, ast: &ParagraphAst) {
        let width_of = |global_atom: usize, ending: bool| -> f32 {
            let run = &ast.runs[ast
                .runs
                .partition_point(|r| r.atom_range.end <= global_atom)];
            let local = global_atom - run.atom_range.start;
            let clusters = &ast.clusters[run.cluster_range.clone()];
            clusters
                .iter()
                .find(|c| {
                    if ending {
                        c.atom_range.end == local + 1
                    } else {
                        c.atom_range.start == local
                    }
                })
                .map_or(0.0, |c| c.natural_width)
        };
        self.seams.retain(|seam| match seam.kind {
            SeamKind::Autospace { .. } => true,
            SeamKind::Compression { font_size } => {
                let floor = COMPRESSED_PAIR_FRACTION * font_size;
                width_of(seam.after, true) > floor && width_of(seam.after + 1, false) > floor
            }
        });
    }

    /// The glue of a seam, at the level of the atom before it (WN-10).
    pub(super) fn seam_node(&self, seam: Seam) -> WordNode {
        let atom = &self.atoms[seam.after];
        let (kind, natural_width, max_stretch, max_shrink) = match seam.kind {
            SeamKind::Autospace { em } => match self.options.autospace_profile {
                AutospaceProfile::Jlreq => (GlueKind::CjkAutospace, em / 4.0, em / 4.0, em / 8.0),
                AutospaceProfile::CssCompat => (GlueKind::CjkAutospace, em / 8.0, 0.0, 0.0),
            },
            SeamKind::Compression { font_size } => {
                (GlueKind::CjkPunctCompression, 0.0, 0.0, font_size / 2.0)
            }
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
