//! Step 5 (spec FE-17, FE-27): the shaping runs.

use super::Compiler;
use super::units::Units;
use crate::types::{AtomFlags, GlyphOrientation, LocaleTag, ShapingRun, TextRange};

impl Compiler<'_> {
    /// Whether atoms `a` and `b` can share a run (FE-17).
    pub(super) fn same_run(
        &self,
        units: &Units,
        locales: &[LocaleTag],
        a: usize,
        b: usize,
    ) -> bool {
        let (sa, sb) = (units.atom_styles[a], units.atom_styles[b]);
        units.atoms[a].bidi_level == units.atoms[b].bidi_level
            && units.atoms[a].script == units.atoms[b].script
            && (sa == sb || self.styles[sa] == self.styles[sb])
            && locales[a] == locales[b]
    }

    /// Step 5: maximal sequences of atoms that agree on style, script and
    /// locale. Glue, tabs and objects between them stay inside the run's
    /// bytes (AST-5).
    pub(super) fn build_runs(&self, units: &Units, locales: &[LocaleTag]) -> Vec<ShapingRun> {
        let atoms = &units.atoms;
        let mut runs: Vec<ShapingRun> = Vec::new();
        let mut start = 0;
        for end in 1..=atoms.len() {
            if end < atoms.len() && self.same_run(units, locales, start, end) {
                continue;
            }
            let style = &self.styles[units.atom_styles[start]];
            // A cursive run takes no letter spacing (FE-27).
            let cursive = atoms[start].flags.contains(AtomFlags::NO_LETTER_SPACING);
            runs.push(ShapingRun {
                baseline_align: style.baseline_align,
                letter_spacing: if cursive { 0.0 } else { style.letter_spacing },
                word_spacing: style.word_spacing,
                orientation: GlyphOrientation::Horizontal,
                features: style.features.clone(),
                byte_range: TextRange::new(
                    atoms[start].byte_range.start(),
                    atoms[end - 1].byte_range.end(),
                ),
                families: style.families.clone(),
                font_size: style.font_size,
                style: style.style,
                script: atoms[start].script,
                locale: locales[start].clone(),
                bidi_level: atoms[start].bidi_level,
                decoration: style.decoration,
                atom_range: start..end,
                cluster_range: 0..0,
            });
            start = end;
        }
        runs
    }
}
