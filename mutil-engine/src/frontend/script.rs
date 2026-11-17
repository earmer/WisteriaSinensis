//! Script resolution (spec §3 Step 3, FE-11, FE-13 to FE-15).
//!
//! Explicit scripts stand, and an atom with a specific script of its own
//! keeps it, so combining marks follow their base; emoji atoms take `Zsye`
//! (FE-6). A Common atom resolves, in order: a closer takes the script of its
//! opener on the bounded Table S stack; a character with `Script_Extensions`
//! takes the preceding strong script when the extensions hold it, else the
//! first following one when they hold that, else their preferred member; a
//! wide character (East Asian Width W, F or H) with no extensions does the
//! same with the Han list; anything else takes the preceding strong script,
//! at the paragraph start the first following one, else `Latn` (FE-25).
//! Under `QuotePolicy::CjkAware` a quote pair beside CJK text takes a CJK
//! script (FE-14), and a run of dashes, ellipses or middle dots resolves as
//! one unit from its two neighbours (FE-15).

use super::QuotePolicy;
use super::segment::{SCRIPT, SCRIPT_NAMES};
use crate::types::{GraphemeAtom, ScriptTag, TextRange};
use icu_properties::props::{
    BidiMirroringGlyph, BidiPairedBracketType, EastAsianWidth, Script, WordBreak,
};
use icu_properties::script::{ScriptWithExtensions, ScriptWithExtensionsBorrowed};
use icu_properties::{CodePointMapData, CodePointMapDataBorrowed};
use smallvec::SmallVec;
use std::sync::LazyLock;

static WIDTH: CodePointMapDataBorrowed<'static, EastAsianWidth> =
    CodePointMapData::<EastAsianWidth>::new();
static MIRRORING: CodePointMapDataBorrowed<'static, BidiMirroringGlyph> =
    CodePointMapData::<BidiMirroringGlyph>::new();
static WORD_BREAK: CodePointMapDataBorrowed<'static, WordBreak> =
    CodePointMapData::<WordBreak>::new();
static EXTENSIONS: LazyLock<ScriptWithExtensionsBorrowed<'static>> =
    LazyLock::new(ScriptWithExtensions::new);

/// The depth of the Table S stack (FE-4).
const STACK_DEPTH: usize = 63;

/// A script tag known to be well formed.
const fn tag(bytes: [u8; 4]) -> ScriptTag {
    match ScriptTag::try_from_bytes(bytes) {
        Ok(tag) => tag,
        Err(_) => panic!("a well-formed script tag"),
    }
}

const HAN: ScriptTag = tag(*b"Hani");
/// The Han script list of FE-13 and the CJK scripts of FE-14.
const HAN_LIST: [ScriptTag; 5] = [
    HAN,
    tag(*b"Hira"),
    tag(*b"Kana"),
    tag(*b"Hang"),
    tag(*b"Bopo"),
];

/// Table S's quotation pairs beyond `BidiBrackets.txt` (FE-9): opener and
/// closer by code point. U+201E and U+201A open German-style pairs that close
/// with U+201C and U+2018.
const QUOTE_PAIRS: [(char, char); 6] = [
    ('\u{201C}', '\u{201D}'),
    ('\u{2018}', '\u{2019}'),
    ('\u{00AB}', '\u{00BB}'),
    ('\u{2039}', '\u{203A}'),
    ('\u{201E}', '\u{201C}'),
    ('\u{201A}', '\u{2018}'),
];

/// Table S's locale-aware override (FE-9), after CLDR's quotation
/// delimiters, keyed by the primary language subtag: the pairs a locale
/// opens and closes its own way. A pair whose opener and closer are one
/// character toggles: it closes when its opener is on the stack, else opens.
/// The override's role for a character wins over the default table's.
pub(crate) fn locale_quote_pairs(language: &str) -> &'static [(char, char)] {
    match language {
        "de" => &[('\u{201E}', '\u{201C}'), ('\u{201A}', '\u{2018}')],
        "pl" => &[('\u{201E}', '\u{201D}')],
        "fr" | "ru" => &[('\u{00AB}', '\u{00BB}')],
        "ja" | "zh" => &[('\u{300C}', '\u{300D}'), ('\u{300E}', '\u{300F}')],
        "sv" | "fi" => &[('\u{201D}', '\u{201D}'), ('\u{2019}', '\u{2019}')],
        _ => &[],
    }
}

/// The run characters of FE-15.
fn is_dash_run_char(c: char) -> bool {
    matches!(c, '\u{2014}' | '\u{2026}' | '\u{00B7}')
}

/// The script of `c`, `Zyyy` or `Zinh` included.
pub(crate) fn char_script(c: char) -> ScriptTag {
    tag_of(SCRIPT.get(c))
}

fn tag_of(script: Script) -> ScriptTag {
    SCRIPT_NAMES
        .get(script)
        .and_then(|name| ScriptTag::try_from_str(name).ok())
        .unwrap_or(ScriptTag::UNKNOWN)
}

/// The first specific script of `atom`'s characters, if any.
pub(crate) fn atom_script(atom: &str) -> Option<ScriptTag> {
    atom.chars()
        .map(char_script)
        .find(|script| !script.is_unresolved())
}

/// East Asian Width W, F or H.
pub(crate) fn is_wide(c: char) -> bool {
    matches!(
        WIDTH.get(c),
        EastAsianWidth::Wide | EastAsianWidth::Fullwidth | EastAsianWidth::Halfwidth
    )
}

/// Word_Break AHLetter (ALetter or Hebrew_Letter), for FE-11.
fn is_ah_letter(c: char) -> bool {
    matches!(
        WORD_BREAK.get(c),
        WordBreak::ALetter | WordBreak::HebrewLetter
    )
}

/// Word_Break Extend, Format, or ZWJ (ignored by UAX #29 WB4).
fn is_wb_transparent(c: char) -> bool {
    matches!(
        WORD_BREAK.get(c),
        WordBreak::Extend | WordBreak::Format | WordBreak::ZWJ
    )
}

/// Whether `c` at `range` of `text` is an apostrophe (FE-11): an ambiguous
/// quote or apostrophe with an AHLetter on both sides (ignoring WB4 Extend/Format/ZWJ).
pub(crate) fn is_apostrophe(text: &str, range: TextRange) -> bool {
    let atom = &text[range.as_range()];
    let is_ambiguous = matches!(atom, "\u{27}" | "\u{2019}" | "\u{2BC}");
    is_ambiguous
        && text[..range.start()]
            .chars()
            .rev()
            .find(|&c| !is_wb_transparent(c))
            .is_some_and(is_ah_letter)
        && text[range.end()..]
            .chars()
            .find(|&c| !is_wb_transparent(c))
            .is_some_and(is_ah_letter)
}

/// The non-trivial `Script_Extensions` of `c`: none for a character whose
/// extensions are just its own Common or Inherited script.
fn extensions(c: char) -> SmallVec<[ScriptTag; 8]> {
    EXTENSIONS
        .get_script_extensions_val(c)
        .iter()
        .map(tag_of)
        .filter(|s| !s.is_unresolved())
        .collect()
}

/// The preferred member of a script set: Han when it holds Han, else its
/// first member.
fn preferred(set: &[ScriptTag]) -> ScriptTag {
    if set.contains(&HAN) {
        HAN
    } else {
        set.first().copied().unwrap_or(ScriptTag::LATIN)
    }
}

/// What a Common atom waits for.
#[derive(Clone)]
enum Pending {
    /// Any strong script.
    Any,
    /// A strong script in the set, else the set's preferred member.
    In(SmallVec<[ScriptTag; 8]>),
}

/// A Table S pairing role.
enum Role {
    Opener(char),
    Closer(char),
    /// U+201C and U+2018 open English-style pairs and close German-style
    /// ones, and a toggling pair of the override closes itself: a closer
    /// when an opener `closes_for` waiting for `closes` is on the stack,
    /// else an opener waiting for `opens`.
    Either {
        closes: char,
        closes_for: char,
        opens: char,
    },
    None,
}

fn role(c: char, overrides: &[(char, char)]) -> Role {
    if let Some(&(opener, closer)) = overrides.iter().find(|&&(o, k)| o == c || k == c) {
        return if opener == closer {
            Role::Either {
                closes: c,
                closes_for: c,
                opens: c,
            }
        } else if opener == c {
            Role::Opener(closer)
        } else {
            Role::Closer(c)
        };
    }
    match c {
        '\u{201C}' => Role::Either {
            closes: '\u{201C}',
            closes_for: '\u{201E}',
            opens: '\u{201D}',
        },
        '\u{2018}' => Role::Either {
            closes: '\u{2018}',
            closes_for: '\u{201A}',
            opens: '\u{2019}',
        },
        _ => {
            if let Some(&(_, closer)) = QUOTE_PAIRS.iter().find(|(o, _)| *o == c) {
                return Role::Opener(closer);
            }
            if QUOTE_PAIRS.iter().any(|(_, k)| *k == c) {
                return Role::Closer(c);
            }
            let glyph = MIRRORING.get(c);
            match glyph.paired_bracket_type {
                BidiPairedBracketType::Open => {
                    glyph.mirroring_glyph.map_or(Role::None, Role::Opener)
                }
                BidiPairedBracketType::Close => Role::Closer(c),
                _ => Role::None,
            }
        }
    }
}

/// One atom as script resolution sees it.
pub(crate) struct ScriptAtom<'a> {
    /// The atom's byte range in the paragraph text.
    pub byte_range: TextRange,
    /// The atom's text.
    pub text: &'a str,
    /// A script its style states, which stands.
    pub explicit: Option<ScriptTag>,
    /// An emoji sequence (FE-6), which takes `Zsye`.
    pub is_emoji: bool,
    /// An apostrophe (FE-11), which stays off the stack.
    pub is_apostrophe: bool,
    /// Directly preceded by an ASCII space (FE-14).
    pub after_space: bool,
    /// The Table S override of the atom's stated locale (FE-9).
    pub quotes: &'static [(char, char)],
}

/// Resolves the script of every atom of a paragraph, in order.
pub(crate) fn resolve(atoms: &[ScriptAtom<'_>], policy: QuotePolicy) -> Vec<ScriptTag> {
    let n = atoms.len();
    let mut scripts: Vec<Option<ScriptTag>> = vec![None; n];
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    // Table S: (closer it waits for, opener atom).
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut pending: Vec<(usize, Pending)> = Vec::new();
    let mut strong: Option<ScriptTag> = None;
    let settle = |pending: &mut Vec<(usize, Pending)>,
                  scripts: &mut Vec<Option<ScriptTag>>,
                  next: Option<ScriptTag>| {
        for (atom, wants) in pending.drain(..) {
            scripts[atom] = Some(match (wants, next) {
                (Pending::Any, Some(s)) => s,
                (Pending::Any, None) => ScriptTag::LATIN,
                (Pending::In(set), Some(s)) if set.contains(&s) => s,
                (Pending::In(set), _) => preferred(&set),
            });
        }
    };
    for (index, atom) in atoms.iter().enumerate() {
        if atom.is_emoji {
            scripts[index] = Some(atom.explicit.unwrap_or(ScriptTag::EMOJI));
            continue;
        }
        let own = atom.explicit.or_else(|| atom_script(atom.text));
        if let Some(script) = own {
            scripts[index] = Some(script);
            settle(&mut pending, &mut scripts, Some(script));
            strong = Some(script);
            continue;
        }
        let Some(c) = atom.text.chars().next() else {
            continue;
        };
        // Table S pairing.
        let mut is_opener = None;
        if !atom.is_apostrophe {
            let closes = match role(c, atom.quotes) {
                Role::Opener(closer) => {
                    is_opener = Some(closer);
                    None
                }
                Role::Closer(closer) => Some(closer),
                Role::Either {
                    closes,
                    closes_for,
                    opens,
                } => {
                    if stack
                        .iter()
                        .any(|&(k, o)| k == closes && atoms[o].text.starts_with(closes_for))
                    {
                        Some(closes)
                    } else {
                        is_opener = Some(opens);
                        None
                    }
                }
                Role::None => None,
            };
            if let Some(closer) = closes
                && let Some(at) = stack.iter().rposition(|&(k, _)| k == closer)
            {
                let opener = stack[at].1;
                stack.truncate(at);
                pairs.push((opener, index));
                continue;
            }
        }
        let mut set = extensions(c);
        if set.is_empty() && is_wide(c) {
            set = SmallVec::from_slice(&HAN_LIST);
        }
        if set.is_empty() {
            match strong {
                Some(s) => scripts[index] = Some(s),
                None => pending.push((index, Pending::Any)),
            }
        } else if let Some(s) = strong.filter(|s| set.contains(s)) {
            scripts[index] = Some(s);
        } else {
            pending.push((index, Pending::In(set)));
        }
        if let Some(closer) = is_opener {
            if stack.len() == STACK_DEPTH {
                stack.remove(0);
            }
            stack.push((closer, index));
        }
    }
    settle(&mut pending, &mut scripts, None);
    let mut resolved: Vec<ScriptTag> = scripts
        .iter()
        .map(|s| s.unwrap_or(ScriptTag::LATIN))
        .collect();
    for &(opener, closer) in &pairs {
        resolved[closer] = resolved[opener];
    }
    if policy == QuotePolicy::CjkAware {
        cjk_quotes(atoms, &mut resolved, &pairs);
    }
    dash_runs(atoms, &mut resolved);
    resolved
}

/// Whether atom `i` counts as CJK (FE-14).
fn is_cjk(atoms: &[ScriptAtom<'_>], resolved: &[ScriptTag], i: usize) -> bool {
    let text = atoms[i].text;
    !text.chars().all(char::is_whitespace)
        && (text.chars().next().is_some_and(is_wide) || HAN_LIST.contains(&resolved[i]))
}

/// FE-14: a quotation pair is CJK when its left anchor is, else its right
/// anchor, else when its interior holds CJK and the opener follows no ASCII
/// space; it then takes its neighbours' common CJK script, or Han.
fn cjk_quotes(atoms: &[ScriptAtom<'_>], resolved: &mut [ScriptTag], pairs: &[(usize, usize)]) {
    for &(opener, closer) in pairs {
        // A quotation opener of either table; the override's wide corner
        // brackets are CJK by width already.
        let is_quote = atoms[opener].text.chars().next().is_some_and(|c| {
            QUOTE_PAIRS.iter().any(|(o, _)| *o == c)
                || (!is_wide(c) && atoms[opener].quotes.iter().any(|(o, _)| *o == c))
        });
        if !is_quote || atoms[opener].explicit.is_some() {
            continue;
        }
        let left = opener.checked_sub(1);
        let right = (closer + 1 < atoms.len()).then_some(closer + 1);
        let left_cjk = left.is_some_and(|i| is_cjk(atoms, resolved, i));
        let right_cjk = right.is_some_and(|i| is_cjk(atoms, resolved, i));
        let after_space = atoms[opener].after_space;
        if !(left_cjk
            || right_cjk
            || (!after_space && (opener + 1..closer).any(|i| is_cjk(atoms, resolved, i))))
        {
            continue;
        }
        let script = match (left, right) {
            (Some(l), Some(r)) if resolved[l] == resolved[r] && HAN_LIST.contains(&resolved[l]) => {
                resolved[l]
            }
            _ => HAN,
        };
        resolved[opener] = script;
        resolved[closer] = script;
    }
}

/// FE-15: a maximal run of one of U+2014, U+2026 and U+00B7 takes one
/// script from its nearest non-punctuation neighbours.
fn dash_runs(atoms: &[ScriptAtom<'_>], resolved: &mut [ScriptTag]) {
    let anchor = |i: usize| {
        let text = atoms[i].text;
        !text
            .chars()
            .all(|c| is_dash_run_char(c) || c.is_whitespace() || is_punctuation(c))
    };
    let mut last_anchor: Option<usize> = None;
    let mut next_anchor: Option<usize> = None;
    let mut next_scanned = 0usize;
    let mut i = 0;
    while i < atoms.len() {
        let Some(c) = atoms[i]
            .text
            .chars()
            .next()
            .filter(|&c| is_dash_run_char(c) && atoms[i].explicit.is_none())
        else {
            if anchor(i) {
                last_anchor = Some(i);
            }
            i += 1;
            continue;
        };
        let mut end = i + 1;
        while end < atoms.len()
            && atoms[end - 1].byte_range.end() == atoms[end].byte_range.start()
            && atoms[end].text.starts_with(c)
            && atoms[end].explicit.is_none()
        {
            end += 1;
        }
        if next_anchor.is_none_or(|k| k < end) {
            next_anchor = (next_scanned.max(end)..atoms.len()).find(|&k| anchor(k));
            next_scanned = next_anchor.map_or(atoms.len(), |k| k + 1);
        }
        let (left, right) = (last_anchor, next_anchor);
        let cjk = |k: Option<usize>| k.is_some_and(|k| is_cjk(atoms, resolved, k));
        let host = resolved[i];
        let script = match (cjk(left), cjk(right)) {
            (true, true) => HAN,
            (true, false) => left.map_or(host, |k| resolved[k]),
            (false, true) => right.map_or(host, |k| resolved[k]),
            (false, false) => match (left, right) {
                (Some(l), Some(r)) if resolved[l] != resolved[r] => resolved[l],
                _ => host,
            },
        };
        resolved[i..end].fill(script);
        last_anchor = (i..end).rev().find(|&k| anchor(k)).or(last_anchor);
        i = end;
    }
}

use super::spacing::is_punctuation;

/// Whether atom `index` is a middle dot (U+00B7, U+2027) resolved to a CJK
/// script (FE-15): such a dot never begins a line after the atom before it.
/// UAX #14 leaves U+00B7 ambiguous (AI), and the segmenter resolves it to AL,
/// which allows a break after an ideograph (`号·`).
pub(crate) fn is_cjk_middle_dot(text: &str, atoms: &[GraphemeAtom], index: usize) -> bool {
    atoms.get(index).is_some_and(|a| {
        HAN_LIST.contains(&a.script)
            && text
                .get(a.byte_range.as_range())
                .and_then(|s| s.chars().next())
                .is_some_and(|c| matches!(c, '\u{00B7}' | '\u{2027}'))
    })
}

/// Atoms whose dash, ellipsis or middle-dot run must not break inside
/// (FE-15): `true` for every atom of such a run but its last.
pub(crate) fn binds_to_next(text: &str, atoms: &[GraphemeAtom], index: usize) -> bool {
    let (Some(a), Some(b)) = (
        atoms.get(index),
        index.checked_add(1).and_then(|i| atoms.get(i)),
    ) else {
        return false;
    };
    if a.byte_range.end() != b.byte_range.start() || a.script != b.script {
        return false;
    }
    let (Some(sa), Some(sb)) = (
        text.get(a.byte_range.as_range()),
        text.get(b.byte_range.as_range()),
    ) else {
        return false;
    };
    matches!((sa.chars().next(), sb.chars().next()),
        (Some(ch_a), Some(ch_b)) if ch_a == ch_b && is_dash_run_char(ch_a))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atoms(text: &str) -> Vec<ScriptAtom<'_>> {
        text.char_indices()
            .map(|(i, c)| ScriptAtom {
                byte_range: TextRange::new(i, i + c.len_utf8()),
                text: &text[i..i + c.len_utf8()],
                explicit: None,
                is_emoji: false,
                is_apostrophe: is_apostrophe(text, TextRange::new(i, i + c.len_utf8())),
                after_space: text[..i].ends_with(' '),
                quotes: &[],
            })
            .collect()
    }

    /// [`scripts`] with every atom under `language`'s Table S override.
    fn scripts_in(text: &str, language: &str) -> String {
        let atoms: Vec<ScriptAtom<'_>> = atoms(text)
            .into_iter()
            .map(|a| ScriptAtom {
                quotes: locale_quote_pairs(language),
                ..a
            })
            .collect();
        resolve(&atoms, QuotePolicy::Standard)
            .iter()
            .map(|s| char::from(s.as_bytes()[0]))
            .collect()
    }

    #[test]
    fn the_locale_override_decides_which_quotes_pair() {
        // A quotation opens after Greek and holds Latin: a closer that pairs
        // takes Greek (G), one that does not takes Latin (L).
        // German: “ only closes „, so “…” is no pair.
        assert_eq!(scripts_in("α“ab”", "en"), "GGLLG");
        assert_eq!(scripts_in("α“ab”", "de"), "GGLLL");
        assert_eq!(scripts_in("α„ab“", "de"), "GGLLG");
        assert_eq!(scripts_in("α‚ab‘", "de"), "GGLLG");
        // Polish: „ closes with ”.
        assert_eq!(scripts_in("α„ab”", "en"), "GGLLL");
        assert_eq!(scripts_in("α„ab”", "pl"), "GGLLG");
        // Swedish and Finnish: ” and ’ close themselves.
        for language in ["sv", "fi"] {
            assert_eq!(scripts_in("α”ab”", language), "GGLLG");
            // After a full stop, so the first ’ is no apostrophe (FE-11).
            assert_eq!(scripts_in("α.’ab’", language), "GGGLLG");
        }
        assert_eq!(scripts_in("α”ab”", "en"), "GGLLL");
        // French, Russian, Japanese and Chinese name pairs the default
        // table already makes.
        for language in ["fr", "ru"] {
            assert_eq!(scripts_in("α«ab»", language), "GGLLG");
        }
        // A wide corner bracket takes Han (FE-13), and its closer with it.
        for language in ["ja", "zh"] {
            assert_eq!(scripts_in("α「ab」", language), "GHLLH");
            assert_eq!(scripts_in("α『ab』", language), "GHLLH");
        }
    }

    fn scripts(text: &str, policy: QuotePolicy) -> String {
        resolve(&atoms(text), policy)
            .iter()
            .map(|s| char::from(s.as_bytes()[0]))
            .collect()
    }

    #[test]
    fn common_atoms_take_the_preceding_script_or_the_first_following() {
        // L: Latn, G: Grek.
        assert_eq!(scripts("1α2b3", QuotePolicy::Standard), "GGGLL");
        assert_eq!(scripts("12", QuotePolicy::Standard), "LL");
    }

    #[test]
    fn marks_follow_their_base() {
        assert_eq!(atom_script("e\u{301}"), Some(ScriptTag::LATIN));
        assert_eq!(atom_script("\u{301}"), None);
        assert_eq!(atom_script("1"), None);
        assert_eq!(atom_script("\u{3b1}"), Some(ScriptTag::GREEK));
    }

    #[test]
    fn a_closer_takes_its_opener_script() {
        // The parenthesis opens in Greek; its closer follows, after Latin.
        assert_eq!(scripts("α(ab)", QuotePolicy::Standard), "GGLLG");
        // An unmatched closer takes the preceding strong script.
        assert_eq!(scripts("ab)", QuotePolicy::Standard), "LLL");
    }

    #[test]
    fn extensions_and_wide_punctuation_join_cjk_text() {
        // The ideographic comma's extensions hold Hira: it joins the kana.
        // H: Hani or Hira (both start with H); check the tags directly.
        let text = "かな、abc";
        let r = resolve(&atoms(text), QuotePolicy::Standard);
        assert_eq!(r[2], tag(*b"Hira"));
        // A corner bracket after Latin takes the following kana's script,
        // and its closer the opener's.
        let text = "ab「カナ」";
        let r = resolve(&atoms(text), QuotePolicy::Standard);
        assert_eq!(r[2], tag(*b"Kana"));
        assert_eq!(r[5], tag(*b"Kana"));
    }

    #[test]
    fn apostrophes_stay_off_the_stack() {
        let text = "‘it’s’";
        let a = atoms(text);
        assert!(a[3].is_apostrophe);
        assert!(!a[5].is_apostrophe);
    }

    #[test]
    fn cjk_aware_quotes_take_a_cjk_script_beside_cjk_text() {
        let text = "漢“ab”字";
        let standard = resolve(&atoms(text), QuotePolicy::Standard);
        assert_eq!(standard[1], HAN);
        assert_eq!(standard[4], HAN);
        let text = "ab “漢字” cd";
        let standard = resolve(&atoms(text), QuotePolicy::Standard);
        assert_eq!(standard[3], ScriptTag::LATIN);
        // Interior CJK, but the opener follows an ASCII space: no change.
        let aware = resolve(&atoms(text), QuotePolicy::CjkAware);
        assert_eq!(aware[3], ScriptTag::LATIN);
        let text = "ab“漢字”cd";
        let aware = resolve(&atoms(text), QuotePolicy::CjkAware);
        assert_eq!((aware[2], aware[5]), (HAN, HAN));
    }

    #[test]
    fn dash_and_ellipsis_runs_resolve_as_one_unit() {
        let text = "漢——ab";
        let r = resolve(&atoms(text), QuotePolicy::Standard);
        assert_eq!((r[1], r[2]), (HAN, HAN));
        let text = "ab……漢";
        let r = resolve(&atoms(text), QuotePolicy::Standard);
        assert_eq!((r[2], r[3]), (HAN, HAN));
        let text = "ab……cd";
        let r = resolve(&atoms(text), QuotePolicy::Standard);
        assert_eq!((r[2], r[3]), (ScriptTag::LATIN, ScriptTag::LATIN));
        let text = "a——b";
        let g: Vec<GraphemeAtom> = text
            .char_indices()
            .map(|(i, c)| GraphemeAtom {
                orientation: crate::types::GlyphOrientation::Horizontal,
                byte_range: TextRange::new(i, i + c.len_utf8()),
                script: ScriptTag::LATIN,
                bidi_level: crate::types::BidiLevel::LTR,
                linebreak_class: crate::types::LineBreakClass::Al,
                flags: crate::types::AtomFlags::EMPTY,
            })
            .collect();
        assert!(binds_to_next(text, &g, 1));
        assert!(!binds_to_next(text, &g, 2));

        // Extending marks on a run character keep the FE-15 binding intact.
        let decorated = "—\u{0300}—";
        let dg = [
            GraphemeAtom {
                orientation: crate::types::GlyphOrientation::Horizontal,
                byte_range: TextRange::new(0, 5),
                script: ScriptTag::LATIN,
                bidi_level: crate::types::BidiLevel::LTR,
                linebreak_class: crate::types::LineBreakClass::Al,
                flags: crate::types::AtomFlags::EMPTY,
            },
            GraphemeAtom {
                orientation: crate::types::GlyphOrientation::Horizontal,
                byte_range: TextRange::new(5, 8),
                script: ScriptTag::LATIN,
                bidi_level: crate::types::BidiLevel::LTR,
                linebreak_class: crate::types::LineBreakClass::Al,
                flags: crate::types::AtomFlags::EMPTY,
            },
        ];
        assert!(binds_to_next(decorated, &dg, 0));
    }

    #[test]
    fn many_runs_without_anchors_resolve_in_linear_time() {
        // 100,000 runs separated by commas, with no anchor anywhere: each
        // run's neighbour search is constant, not a scan of the paragraph.
        let text = "—,".repeat(100_000);
        let resolved = resolve(&atoms(&text), QuotePolicy::Standard);
        assert_eq!(resolved.len(), 200_000);
        assert!(resolved.iter().all(|&s| s == ScriptTag::LATIN));
    }
}
