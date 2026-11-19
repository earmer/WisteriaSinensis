//! Face selection (spec SHP-5, SHP-12, SHP-25, SHP-26): which registered
//! face shapes an atom, or a tate-chu-yoko group, of a run.

use super::registry::{CjkRegion, FontRegistry, ResolvedCandidate, preferred_regions};
use crate::types::{FontId, GraphemeAtom, ScriptTag, ShapingRun};
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use smallvec::SmallVec;

/// The normalisations under which a face covers an atom (spec SHP-26).
static NFC: ComposingNormalizerBorrowed<'static> = ComposingNormalizerBorrowed::new_nfc();
static NFD: DecomposingNormalizerBorrowed<'static> = DecomposingNormalizerBorrowed::new_nfd();

/// Pre-resolved candidate faces and CJK routing state for one shaping run.
///
/// Built once per run so per-atom face selection works directly on in-memory
/// references with zero registry lookups and zero locale parsing in the loop.
pub(super) struct RunFaceMatcher<'a> {
    candidates: SmallVec<[ResolvedCandidate<'a>; 8]>,
    /// `Some(regions)` for a CJK script run (with the run's preferred region
    /// tiers, possibly empty); `None` for non-CJK runs.
    cjk_preferred: Option<&'static [CjkRegion]>,
}

impl<'a> RunFaceMatcher<'a> {
    pub(super) fn new(registry: &'a FontRegistry, run: &ShapingRun) -> Self {
        Self {
            candidates: registry.resolve_candidates(&run.families, run.style),
            cjk_preferred: is_cjk_script(run.script).then(|| preferred_regions(run)),
        }
    }

    pub(super) fn first_id(&self) -> FontId {
        self.candidates.first().map_or(FontId::default(), |c| c.id)
    }

    /// The face that shapes `atom` of the run, keyed by script and locale
    /// (spec SHP-12, SHP-25).
    ///
    /// In a CJK run (`Hani`, `Hira`, `Kana`, `Hang`, `Bopo`) an atom whose
    /// first code point is ASCII keeps stack order; for any other atom the
    /// CJK faces come first: the first candidate whose region the run prefers
    /// and whose character map covers the whole atom, else the first CJK
    /// candidate that covers it; a face that is not CJK is reached only when
    /// no CJK face covers the atom. Every run then falls back to the first
    /// candidate that covers the whole atom, else the first that covers its
    /// first code point, else the first candidate, CJK faces first in a CJK
    /// run. A face covers an atom when it covers the atom as written or its
    /// NFC or NFD form (SHP-26).
    pub(super) fn face_for(&self, text: &str, atom: &GraphemeAtom) -> Option<FontId> {
        let written = &text[atom.byte_range.as_range()];
        // Fast path: a single ASCII byte is already NFC and NFD, always keeps
        // stack order (even in a CJK run), and its whole-atom and lead-char
        // coverage predicates are identical.
        if written.len() == 1
            && let Some(c) = written.chars().next()
        {
            return self
                .candidates
                .iter()
                .find(|cand| covers_char(cand, c))
                .or_else(|| self.candidates.first())
                .map(|cand| cand.id);
        }
        let mut atom_text = AtomText::new(written);
        self.choose_index(&mut atom_text)
            .map(|idx| self.candidates[idx].id)
    }

    /// The one face that shapes a tate-chu-yoko group `atoms` (SHP-5): the
    /// face of its first atom when that covers every atom, else the first
    /// face, among those chosen for its atoms and then the candidates, that
    /// does, else the first atom's face.
    pub(super) fn face_for_group(&self, text: &str, atoms: &[GraphemeAtom]) -> Option<FontId> {
        // Each atom's NFC/NFD fallback forms are computed lazily at most once
        // across both selection and group coverage.
        let mut group: SmallVec<[AtomText<'_>; 4]> = atoms
            .iter()
            .map(|atom| AtomText::new(&text[atom.byte_range.as_range()]))
            .collect();
        let first_idx = self.choose_index(group.first_mut()?)?;
        if self.candidate_covers_all(first_idx, &mut group) {
            return Some(self.candidates[first_idx].id);
        }
        for i in 1..group.len() {
            if let Some(idx) = self.choose_index(&mut group[i])
                && idx != first_idx
                && self.candidate_covers_all(idx, &mut group)
            {
                return Some(self.candidates[idx].id);
            }
        }
        for idx in 0..self.candidates.len() {
            if self.candidate_covers_all(idx, &mut group) {
                return Some(self.candidates[idx].id);
            }
        }
        Some(self.candidates[first_idx].id)
    }

    fn candidate_covers_all(&self, idx: usize, group: &mut [AtomText<'_>]) -> bool {
        let cand = &self.candidates[idx];
        group.iter_mut().all(|atom| atom.is_covered_by(cand))
    }

    fn choose_index(&self, atom: &mut AtomText<'_>) -> Option<usize> {
        // ASCII keeps stack order even in a CJK run: FE-13 folds digits and
        // ASCII punctuation into a neighbouring Han run, and they belong to
        // the stack's leading Latin face.
        let ascii = atom.lead().is_some_and(|c| c.is_ascii());
        match self.cjk_preferred {
            Some(preferred) if !ascii => self.choose_cjk(atom, preferred),
            _ => self.choose_stack_order(atom),
        }
    }

    fn choose_stack_order(&self, atom: &mut AtomText<'_>) -> Option<usize> {
        self.candidates
            .iter()
            .position(|c| atom.is_covered_by(c))
            .or_else(|| {
                let lead = atom.lead()?;
                self.candidates.iter().position(|c| covers_char(c, lead))
            })
            .or_else(|| (!self.candidates.is_empty()).then_some(0))
    }

    fn choose_cjk(&self, atom: &mut AtomText<'_>, preferred: &[CjkRegion]) -> Option<usize> {
        for (tier, &region) in preferred.iter().enumerate() {
            if let Some(idx) = self.candidates.iter().position(|c| {
                let profile = c.profile();
                profile.cjk
                    && (profile.region == Some(region)
                        || (tier == 0 && profile.region == Some(CjkRegion::Pan)))
                    && atom.is_covered_by(c)
            }) {
                return Some(idx);
            }
        }
        self.candidates
            .iter()
            .position(|c| c.profile().cjk && atom.is_covered_by(c))
            .or_else(|| self.candidates.iter().position(|c| atom.is_covered_by(c)))
            .or_else(|| {
                let lead = atom.lead()?;
                self.candidates
                    .iter()
                    .position(|c| c.profile().cjk && covers_char(c, lead))
                    .or_else(|| self.candidates.iter().position(|c| covers_char(c, lead)))
            })
            .or_else(|| self.candidates.iter().position(|c| c.profile().cjk))
            .or_else(|| (!self.candidates.is_empty()).then_some(0))
    }
}

/// Whether `cand` maps `c`, or `c` needs no glyph of its own.
fn covers_char(cand: &ResolvedCandidate<'_>, c: char) -> bool {
    is_coverage_exempt(c) || cand.face.font.charmap().map_unicode(c).is_some()
}

/// Whether `cand` covers every scalar of `s`.
fn covers_str(cand: &ResolvedCandidate<'_>, s: &str) -> bool {
    s.chars().all(|c| covers_char(cand, c))
}

/// An atom's written slice and lazily computed NFC/NFD scalar forms (SHP-26).
struct AtomText<'t> {
    written: &'t str,
    /// Populated only when a candidate fails to cover `written` directly.
    /// Holds distinct NFC/NFD scalar sequences that differ from `written`.
    extra: Option<SmallVec<[SmallVec<[char; 8]>; 2]>>,
}

impl<'t> AtomText<'t> {
    fn new(written: &'t str) -> Self {
        Self {
            written,
            extra: None,
        }
    }

    fn lead(&self) -> Option<char> {
        self.written.chars().next()
    }

    /// Whether `cand` covers `self` as written, or in NFC or NFD (SHP-26).
    fn is_covered_by(&mut self, cand: &ResolvedCandidate<'_>) -> bool {
        if covers_str(cand, self.written) {
            return true;
        }
        self.extra_forms()
            .iter()
            .any(|form| form.iter().all(|&c| covers_char(cand, c)))
    }

    fn extra_forms(&mut self) -> &[SmallVec<[char; 8]>] {
        self.extra.get_or_insert_with(|| {
            let mut forms: SmallVec<[SmallVec<[char; 8]>; 2]> = SmallVec::new();
            if self.written.is_ascii() {
                return forms;
            }
            if !NFC.is_normalized(self.written) {
                forms.push(NFC.normalize_iter(self.written.chars()).collect());
            }
            if !NFD.is_normalized(self.written) {
                let nfd: SmallVec<[char; 8]> = NFD.normalize_iter(self.written.chars()).collect();
                if forms.first() != Some(&nfd) {
                    forms.push(nfd);
                }
            }
            forms
        })
    }
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
