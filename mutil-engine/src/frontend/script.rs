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
use crate::types::{ScriptTag, TextRange};
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

/// Whether `c` at `range` of `text` is an apostrophe (FE-11): an ambiguous
/// quote or apostrophe with an AHLetter on both sides.
pub(crate) fn is_apostrophe(text: &str, range: TextRange) -> bool {
    let atom = &text[range.as_range()];
    let is_ambiguous = matches!(atom, "\u{27}" | "\u{2019}" | "\u{2BC}");
    is_ambiguous
        && text[..range.start()]
            .chars()
            .next_back()
            .is_some_and(is_ah_letter)
        && text[range.end()..].chars().next().is_some_and(is_ah_letter)
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
    // Closer index -> opener index, for the final pass.
    let mut paired: Vec<Option<usize>> = vec![None; n];
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
                paired[index] = Some(opener);
                pairs.push((opener, index));
                continue;
            }
        }
        let set = {
            let set = extensions(c);
            if set.is_empty() && is_wide(c) {
                SmallVec::from_slice(&HAN_LIST)
            } else {
                set
            }
        };
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
    for index in 0..n {
        if let Some(opener) = paired[index] {
            resolved[index] = resolved[opener];
        }
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
        let inner_cjk = (opener + 1..closer).any(|i| is_cjk(atoms, resolved, i));
        if !(left_cjk || right_cjk || (inner_cjk && !after_space)) {
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
    // The nearest anchor before each atom and at or after it, from one
    // forward and one backward pass, so every run finds its neighbours in
    // constant time.
    let n = atoms.len();
    let mut before: Vec<Option<usize>> = vec![None; n + 1];
    for k in 0..n {
        before[k + 1] = if anchor(k) { Some(k) } else { before[k] };
    }
    let mut from: Vec<Option<usize>> = vec![None; n + 1];
    for k in (0..n).rev() {
        from[k] = if anchor(k) { Some(k) } else { from[k + 1] };
    }
    let mut i = 0;
    while i < atoms.len() {
        let Some(c) = atoms[i]
            .text
            .chars()
            .next()
            .filter(|&c| is_dash_run_char(c) && atoms[i].explicit.is_none())
        else {
            i += 1;
            continue;
        };
        let mut end = i + 1;
        while end < atoms.len() && atoms[end].text.starts_with(c) && atoms[end].explicit.is_none() {
            end += 1;
        }
        let (left, right) = (before[i], from[end]);
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
        for s in &mut resolved[i..end] {
            *s = script;
        }
        i = end;
    }
}

/// General punctuation, for the anchors of FE-15.
fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(c, '\u{2000}'..='\u{206F}' | '\u{3000}'..='\u{303F}' | '\u{FF01}'..='\u{FF0F}')
}

/// Atoms whose dash, ellipsis or middle-dot run must not break inside
/// (FE-15): `true` for every atom of such a run but its last.
pub(crate) fn binds_to_next(atoms: &[ScriptAtom<'_>], index: usize) -> bool {
    let first = |i: usize| atoms[i].text.chars().next();
    match (first(index), atoms.get(index + 1).and(first(index + 1))) {
        (Some(a), Some(b)) => {
            a == b && is_dash_run_char(a) && atoms[index].text.chars().count() == 1
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atoms(text: &str) -> Vec<ScriptAtom<'_>> {
        text.char_indices()
            .map(|(i, c)| ScriptAtom {
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
        let a = atoms("a——b");
        assert!(binds_to_next(&a, 1));
        assert!(!binds_to_next(&a, 2));
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
