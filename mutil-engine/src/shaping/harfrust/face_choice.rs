//! Face selection (spec SHP-5, SHP-12, SHP-25, SHP-26): which registered
//! face shapes an atom, or a tate-chu-yoko group, of a run.

use super::HarfRustShaper;
use super::registry::{CjkRegion, preferred_regions};
use crate::types::{FontId, GraphemeAtom, ScriptTag, ShapingRun};
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use smallvec::SmallVec;
use std::borrow::Cow;

/// The normalisations under which a face covers an atom (spec SHP-26).
static NFC: ComposingNormalizerBorrowed<'static> = ComposingNormalizerBorrowed::new_nfc();
static NFD: DecomposingNormalizerBorrowed<'static> = DecomposingNormalizerBorrowed::new_nfd();

impl HarfRustShaper {
    /// The face that shapes `atom` of `run`, keyed by the run's script and
    /// locale (spec SHP-12, SHP-25).
    ///
    /// In a CJK run (`Hani`, `Hira`, `Kana`, `Hang`, `Bopo`) an atom whose
    /// first code point is ASCII keeps stack order; for any other atom the
    /// CJK faces come first: the first candidate whose region the run prefers and
    /// whose character map covers the whole atom, else the first CJK
    /// candidate that covers it; a face that is not CJK is reached only when
    /// no CJK face covers the atom. Every run then falls back to the first
    /// candidate that covers the whole atom, else the first that covers its
    /// first code point, else the first candidate, CJK faces first in a CJK
    /// run. A face covers an atom when it covers the atom as written or its
    /// NFC or NFD form (SHP-26).
    pub(super) fn face_for(
        &self,
        text: &str,
        run: &ShapingRun,
        atom: &GraphemeAtom,
        candidates: &[FontId],
    ) -> Option<FontId> {
        self.face_for_forms(run, &AtomText::new(text, atom), candidates)
    }

    /// [`Self::face_for`] for an atom whose normal forms are already known.
    fn face_for_forms(
        &self,
        run: &ShapingRun,
        atom: &AtomText<'_>,
        candidates: &[FontId],
    ) -> Option<FontId> {
        let covers_atom = |id: FontId| self.covers_forms(id, &atom.forms);
        let covers_lead = |id: FontId| atom.lead.is_some_and(|c| self.covers_char(id, c));
        let profile = |id: FontId| self.registry.profile(id).unwrap_or_default();
        let first = |pass: &dyn Fn(FontId) -> bool| candidates.iter().copied().find(|&id| pass(id));
        // ASCII keeps stack order even in a CJK run: FE-13 folds digits and
        // ASCII punctuation into a neighbouring Han run, and they belong to
        // the stack's leading Latin face.
        let ascii = atom.lead.is_some_and(|c| c.is_ascii());
        if !is_cjk_script(run.script) || ascii {
            return first(&covers_atom)
                .or_else(|| first(&covers_lead))
                .or_else(|| candidates.first().copied());
        }
        let cjk = |id: FontId| profile(id).cjk;
        for (tier, &region) in preferred_regions(run).iter().enumerate() {
            let found = first(&|id| {
                let face = profile(id);
                face.cjk
                    && (face.region == Some(region)
                        || (tier == 0 && face.region == Some(CjkRegion::Pan)))
                    && covers_atom(id)
            });
            if found.is_some() {
                return found;
            }
        }
        first(&|id| cjk(id) && covers_atom(id))
            .or_else(|| first(&covers_atom))
            .or_else(|| first(&|id| cjk(id) && covers_lead(id)))
            .or_else(|| first(&covers_lead))
            .or_else(|| first(&cjk))
            .or_else(|| candidates.first().copied())
    }

    /// Whether face `id` maps `c`, or `c` needs no glyph of its own.
    fn covers_char(&self, id: FontId, c: char) -> bool {
        is_coverage_exempt(c)
            || self
                .registry
                .face(id)
                .is_some_and(|face| face.font.charmap().map_unicode(c).is_some())
    }

    /// Whether face `id` covers one of an atom's `forms` (SHP-26).
    fn covers_forms(&self, id: FontId, forms: &[Cow<'_, str>]) -> bool {
        forms
            .iter()
            .any(|form| form.chars().all(|c| self.covers_char(id, c)))
    }

    /// The one face that shapes a tate-chu-yoko group `atoms` (SHP-5): the
    /// face of its first atom when that covers every atom, else the first
    /// face, among those chosen for its atoms and then the candidates, that
    /// does, else the first atom's face.
    pub(super) fn face_for_group(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        candidates: &[FontId],
    ) -> Option<FontId> {
        // Each atom is decoded and normalised once, for the choice of its
        // face and for the coverage of the whole group.
        let group: SmallVec<[AtomText<'_>; 4]> =
            atoms.iter().map(|atom| AtomText::new(text, atom)).collect();
        let lead = self.face_for_forms(run, group.first()?, candidates)?;
        let covers_all = |id: FontId| group.iter().all(|a| self.covers_forms(id, &a.forms));
        core::iter::once(lead)
            .chain(
                group[1..]
                    .iter()
                    .filter_map(|atom| self.face_for_forms(run, atom, candidates)),
            )
            .chain(candidates.iter().copied())
            .find(|&id| covers_all(id))
            .or(Some(lead))
    }
}

/// An atom's first code point and its forms as written, NFC and NFD
/// (SHP-26), decoded once for face selection.
struct AtomText<'t> {
    lead: Option<char>,
    forms: SmallVec<[Cow<'t, str>; 3]>,
}

impl<'t> AtomText<'t> {
    fn new(text: &'t str, atom: &GraphemeAtom) -> Self {
        let written = &text[atom.byte_range.as_range()];
        Self {
            lead: written.chars().next(),
            forms: atom_forms(written),
        }
    }
}

/// An atom's text as written, then its NFC and NFD forms where they differ
/// (SHP-26).
fn atom_forms(written: &str) -> SmallVec<[Cow<'_, str>; 3]> {
    let mut forms: SmallVec<[Cow<'_, str>; 3]> = SmallVec::new();
    for form in [
        Cow::Borrowed(written),
        NFC.normalize(written),
        NFD.normalize(written),
    ] {
        if !forms.contains(&form) {
            forms.push(form);
        }
    }
    forms
}

/// Code points shaped by whatever face shapes the rest of their atom:
/// joiners and variation selectors, which faces rarely map.
fn is_coverage_exempt(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}')
}

/// The scripts whose runs route to CJK faces first (spec SHP-12, SHP-25).
fn is_cjk_script(script: ScriptTag) -> bool {
    [
        ScriptTag::HAN,
        ScriptTag::HIRAGANA,
        ScriptTag::KATAKANA,
        ScriptTag::HANGUL,
        ScriptTag::BOPOMOFO,
    ]
    .contains(&script)
}
