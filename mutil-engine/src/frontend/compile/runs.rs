//! Step 5 (spec FE-17, FE-27): the shaping runs.

use super::Compiler;
use super::units::Units;
use crate::frontend::TextStyle;
use crate::types::{AtomFlags, GlyphOrientation, GraphemeAtom, LocaleTag, ShapingRun, TextRange};
use core::ops::Range;

/// One maximal run of atoms before shaping has assigned its `cluster_range`.
pub(super) struct UnshapedRun {
    pub(super) style: usize,
    pub(super) atom_range: Range<usize>,
}

impl UnshapedRun {
    /// Materialises a [`ShapingRun`] for `TextShaper::shape_run` (which ignores
    /// `cluster_range`, SHP-4); the shaper phase sets `cluster_range` once the
    /// run's clusters are recorded, without cloning the run.
    pub(super) fn into_shaping_run(
        self,
        style: &TextStyle,
        locale: LocaleTag,
        atoms: &[GraphemeAtom],
    ) -> ShapingRun {
        let first = &atoms[0];
        let last = &atoms[atoms.len() - 1];
        // A cursive run takes no letter spacing (FE-27).
        let cursive = first.flags.contains(AtomFlags::NO_LETTER_SPACING);
        ShapingRun {
            baseline_align: style.baseline_align,
            letter_spacing: if cursive { 0.0 } else { style.letter_spacing },
            word_spacing: style.word_spacing,
            orientation: GlyphOrientation::Horizontal,
            features: style.features.clone(),
            byte_range: TextRange::new(first.byte_range.start(), last.byte_range.end()),
            families: style.families.clone(),
            font_size: style.font_size,
            style: style.style,
            script: first.script,
            locale,
            bidi_level: first.bidi_level,
            decoration: style.decoration,
            atom_range: self.atom_range,
            cluster_range: Range::default(),
        }
    }
}

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
    pub(super) fn build_runs(&self, units: &Units, locales: &[LocaleTag]) -> Vec<UnshapedRun> {
        let atoms = &units.atoms;
        let mut runs: Vec<UnshapedRun> = Vec::new();
        let mut start = 0;
        for end in 1..=atoms.len() {
            if end < atoms.len() && self.same_run(units, locales, start, end) {
                continue;
            }
            runs.push(UnshapedRun {
                style: units.atom_styles[start],
                atom_range: start..end,
            });
            start = end;
        }
        runs
    }
}
