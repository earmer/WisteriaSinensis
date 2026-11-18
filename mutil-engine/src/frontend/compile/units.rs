//! Steps 1 to 4 of the analysis: the source units and atoms (FE-25), their
//! scripts (FE-6, FE-11, FE-13 to FE-15) and their locales (SCO-5, FE-16).

use super::{Compiler, FrontendError, primary_language};
use crate::frontend::{
    InlineObjectInput, LocalePolicy, bidi, emoji, is_word_space, locale, script, segment,
};
use crate::types::{
    AtomFlags, GlyphOrientation, GraphemeAtom, LineBreakClass, LocaleTag, ScriptTag, TextRange,
};

/// One source unit, in text order.
#[derive(Clone, Copy, Debug)]
pub(super) enum Unit {
    /// An index into `Units::atoms`.
    Atom(usize),
    /// A word space, with the index of its style.
    Glue(TextRange, usize),
    Tab(TextRange),
    HardBreak(TextRange),
    /// An index into `ParagraphInput::inline_objects`.
    Object(usize),
}

impl Unit {
    pub(super) fn range(self, atoms: &[GraphemeAtom], objects: &[InlineObjectInput]) -> TextRange {
        match self {
            Self::Atom(i) => atoms[i].byte_range,
            Self::Glue(r, _) | Self::Tab(r) | Self::HardBreak(r) => r,
            Self::Object(i) => objects[i].byte_range,
        }
    }
}

/// The paragraph split into units, with the text atoms they index.
pub(super) struct Units {
    pub(super) units: Vec<Unit>,
    /// Text atoms, in text order; their `script` and flags are written by the
    /// named `resolve_*`/`mark_*` passes.
    pub(super) atoms: Vec<GraphemeAtom>,
    /// The style index of each atom.
    pub(super) atom_styles: Vec<usize>,
}

impl<'a> Compiler<'a> {
    /// Step 1 (FE-25): splits the text into atoms, glue, tabs, hard breaks
    /// and objects. Each atom takes the level of its first code point (FE-10,
    /// FE-30); one that L1 resets with whitespace is flagged for Stage 2.
    pub(super) fn build_units(
        &self,
        graphemes: &[usize],
        levels: &bidi::TextLevels,
    ) -> Result<Units, FrontendError> {
        let text = self.text;
        let objects = self.input.inline_objects;
        for (index, object) in objects.iter().enumerate() {
            let r = object.byte_range;
            if graphemes.binary_search(&r.start()).is_err()
                || graphemes.binary_search(&r.end()).is_err()
            {
                return Err(FrontendError::InvalidInlineObject { object: index });
            }
        }
        let mut built = Units {
            units: Vec::new(),
            atoms: Vec::new(),
            atom_styles: Vec::new(),
        };
        let mut next_object = 0;
        for pair in graphemes.windows(2) {
            let range = TextRange::new(pair[0], pair[1]);
            if let Some(object) = objects.get(next_object) {
                if range.start() >= object.byte_range.end() {
                    next_object += 1;
                } else if range.start() >= object.byte_range.start() {
                    if range.start() == object.byte_range.start() {
                        built.units.push(Unit::Object(next_object));
                    }
                    if range.end() == object.byte_range.end() {
                        next_object += 1;
                    }
                    continue;
                }
            }
            let grapheme = &text[range.as_range()];
            let mut chars = grapheme.chars();
            let first = chars.next().expect("a grapheme is not empty");
            let single = chars.next().is_none();
            let unit = match first {
                '\t' if single => Unit::Tab(range),
                '\u{2028}' if single => Unit::HardBreak(range),
                c if single && is_word_space(c) => Unit::Glue(range, self.style_at(range.start())),
                _ => {
                    let mut flags = AtomFlags::EMPTY;
                    let class = segment::line_break_class(first);
                    flags.set(AtomFlags::IS_OPEN_PUNCT, class == LineBreakClass::Op);
                    flags.set(
                        AtomFlags::IS_CLOSE_PUNCT,
                        matches!(class, LineBreakClass::Cl | LineBreakClass::Cp),
                    );
                    flags.set(
                        AtomFlags::HAS_VARIATION_SELECTOR,
                        grapheme.chars().any(|c| {
                            matches!(c, '\u{FE0E}' | '\u{FE0F}' | '\u{E0100}'..='\u{E01EF}')
                        }),
                    );
                    flags.set(
                        AtomFlags::IS_EMOJI_SEQUENCE,
                        emoji::is_emoji_sequence(grapheme),
                    );
                    flags.set(
                        AtomFlags::L1_RESETTABLE,
                        bidi::line_class(bidi::char_class(first)) != bidi::LineClass::Other,
                    );
                    flags.set(AtomFlags::IS_APOSTROPHE, script::is_apostrophe(text, range));
                    flags.set(
                        AtomFlags::IS_CJK_IDEOGRAPH,
                        script::is_wide(first)
                            && matches!(
                                &script::char_script(first).as_bytes(),
                                b"Hani" | b"Hira" | b"Kana"
                            ),
                    );
                    built.atoms.push(GraphemeAtom {
                        orientation: GlyphOrientation::Horizontal,
                        byte_range: range,
                        script: ScriptTag::COMMON,
                        bidi_level: levels.at(range.start()),
                        linebreak_class: class,
                        flags,
                    });
                    built.atom_styles.push(self.style_at(range.start()));
                    Unit::Atom(built.atoms.len() - 1)
                }
            };
            built.units.push(unit);
        }
        Ok(built)
    }

    /// The atoms as script resolution sees them.
    fn script_atoms(
        &self,
        atoms: &[GraphemeAtom],
        atom_styles: &[usize],
    ) -> Vec<script::ScriptAtom<'a>> {
        let text = self.text;
        atoms
            .iter()
            .zip(atom_styles)
            .map(|(atom, &style)| script::ScriptAtom {
                byte_range: atom.byte_range,
                text: &text[atom.byte_range.as_range()],
                explicit: self.styles[style].script,
                is_emoji: atom.flags.contains(AtomFlags::IS_EMOJI_SEQUENCE),
                is_apostrophe: atom.flags.contains(AtomFlags::IS_APOSTROPHE),
                after_space: text[..atom.byte_range.start()].ends_with(' '),
                quotes: script::locale_quote_pairs(primary_language(&self.locale_of(style))),
            })
            .collect()
    }

    /// Step 3 (FE-6, FE-11, FE-13 to FE-15): a style's script stands. Writes
    /// `GraphemeAtom::script`.
    pub(super) fn resolve_scripts(&self, atoms: &mut [GraphemeAtom], atom_styles: &[usize]) {
        let resolved = script::resolve(
            &self.script_atoms(atoms, atom_styles),
            self.options.quote_policy,
        );
        for (atom, script) in atoms.iter_mut().zip(resolved) {
            atom.script = script;
        }
    }

    /// Step 4 (SCO-5, FE-16): a style's locale stands; untagged atoms take
    /// `default_locale`, or under `ExperimentalHeuristics` the locale their
    /// sentence's scripts imply. One locale per atom.
    pub(super) fn resolve_locales(&self, units: &Units) -> Vec<LocaleTag> {
        if self.options.locale_policy == LocalePolicy::ExplicitOnly {
            return units
                .atom_styles
                .iter()
                .map(|&style| self.locale_of(style))
                .collect();
        }
        let text = self.text;
        let atoms: Vec<locale::LocaleAtom<'_>> = units
            .atoms
            .iter()
            .zip(&units.atom_styles)
            .map(|(atom, &style)| locale::LocaleAtom {
                byte_range: atom.byte_range,
                text: &text[atom.byte_range.as_range()],
                script: atom.script,
                explicit: self.styles[style].locale.as_ref(),
            })
            .collect();
        locale::resolve(
            text,
            &atoms,
            &self.options.default_locale,
            self.options.locale_policy,
        )
    }
}
