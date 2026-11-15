//! Emoji sequences (spec FE-6): which grapheme atoms are emoji, read from
//! the emoji properties of the pinned data, after segmentation and with no
//! second state machine. UAX #29 has already fixed the atom's boundaries.

use icu_properties::props::{
    Emoji, EmojiModifier, EmojiModifierBase, EmojiPresentation, ExtendedPictographic,
    RegionalIndicator,
};
use icu_properties::{CodePointSetData, CodePointSetDataBorrowed};

static EMOJI: CodePointSetDataBorrowed<'static> = CodePointSetData::new::<Emoji>();
static PRESENTATION: CodePointSetDataBorrowed<'static> =
    CodePointSetData::new::<EmojiPresentation>();
static MODIFIER: CodePointSetDataBorrowed<'static> = CodePointSetData::new::<EmojiModifier>();
static MODIFIER_BASE: CodePointSetDataBorrowed<'static> =
    CodePointSetData::new::<EmojiModifierBase>();
static PICTOGRAPHIC: CodePointSetDataBorrowed<'static> =
    CodePointSetData::new::<ExtendedPictographic>();
static REGIONAL: CodePointSetDataBorrowed<'static> = CodePointSetData::new::<RegionalIndicator>();

const ZWJ: char = '\u{200D}';
const TEXT_STYLE: char = '\u{FE0E}';
const EMOJI_STYLE: char = '\u{FE0F}';
const KEYCAP: char = '\u{20E3}';
const CANCEL_TAG: char = '\u{E007F}';

fn is_tag(c: char) -> bool {
    ('\u{E0020}'..='\u{E007E}').contains(&c)
}

/// Whether the atom `atom` is an emoji sequence (FE-6): a flag, a keycap, a
/// tag sequence, a modifier sequence, a ZWJ sequence of emoji, or a core
/// emoji in emoji style. A ZWJ between non-emoji (Indic or Arabic joining)
/// makes nothing an emoji, and a base whose default is text style is emoji
/// only with U+FE0F (a modifier sequence needs none), never with U+FE0E.
pub(crate) fn is_emoji_sequence(atom: &str) -> bool {
    if atom.is_ascii() {
        return false;
    }
    let mut chars = atom.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if is_keycap_base(first) {
        return matches!(
            (chars.next(), chars.next(), chars.next()),
            (Some(KEYCAP), None, None) | (Some(EMOJI_STYLE), Some(KEYCAP), None)
        );
    }
    if REGIONAL.contains(first) {
        return matches!(
            (chars.next(), chars.next()),
            (Some(second), None) if REGIONAL.contains(second)
        );
    }
    if atom.contains(ZWJ) {
        atom.split(ZWJ).all(|piece| is_element(piece, true))
    } else {
        is_element(atom, false)
    }
}

fn is_keycap_base(c: char) -> bool {
    c.is_ascii_digit() || c == '#' || c == '*'
}

/// One element of a sequence: a base with what may follow it.
fn is_element(element: &str, in_zwj_sequence: bool) -> bool {
    let mut chars = element.chars();
    let Some(base) = chars.next() else {
        return false;
    };
    if !(EMOJI.contains(base) || PICTOGRAPHIC.contains(base)) || is_keycap_base(base) {
        return false;
    }
    match chars.next() {
        // A modifier sequence needs no variation selector.
        Some(m) if MODIFIER.contains(m) => MODIFIER_BASE.contains(base),
        Some(TEXT_STYLE) => false,
        Some(EMOJI_STYLE) => true,
        // A tag sequence: tags ended by the cancel tag.
        Some(t) if is_tag(t) => {
            let Some(last) = chars.next_back() else {
                return false;
            };
            last == CANCEL_TAG && chars.all(is_tag)
        }
        // A core emoji in its default emoji style, possibly with marks that
        // UAX #29 keeps in its atom; inside a ZWJ sequence any pictograph.
        _ => PRESENTATION.contains(base) || (in_zwj_sequence && PICTOGRAPHIC.contains(base)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_of_sequence_is_an_emoji() {
        for emoji in [
            "\u{1F600}",                                                       // grinning face
            "\u{1F1EF}\u{1F1F5}",                                              // flag JP
            "1\u{FE0F}\u{20E3}",                                               // keycap 1
            "#\u{20E3}",                                                       // keycap #, no FE0F
            "\u{1F44B}\u{1F3FD}", // waving hand, modifier
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", // family
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}", // Scotland
            "\u{2764}\u{FE0F}",   // red heart, emoji style
            "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}", // rainbow flag
        ] {
            assert!(is_emoji_sequence(emoji), "{emoji:?}");
        }
    }

    #[test]
    fn text_style_and_joining_text_are_not_emoji() {
        for text in [
            "a",
            "1",
            "#",
            "\u{2764}",               // red heart, text style by default
            "\u{2764}\u{FE0E}",       // explicit text style
            "\u{1F600}\u{FE0E}",      // emoji base forced to text style
            "\u{915}\u{94D}\u{200D}", // Devanagari half form with ZWJ
            "\u{644}\u{200D}",        // Arabic joiner
            "\u{1F1EF}",              // a lone regional indicator
        ] {
            assert!(!is_emoji_sequence(text), "{text:?}");
        }
    }
}
