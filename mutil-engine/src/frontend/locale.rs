//! Locale resolution (spec §3 Step 4, SCO-5, FE-16).
//!
//! An atom whose style states a locale keeps it. Under
//! `LocalePolicy::ExplicitOnly` every other atom takes `default_locale`.
//! Under `LocalePolicy::ExperimentalHeuristics` an untagged atom looks at
//! its UAX #29 sentence, tagged atoms included: when the sentence holds kana,
//! its Han and kana atoms take `ja-JP`; else, when it holds Hangul, its Han
//! and Hangul atoms take `ko-KR`; when it holds U+0402, U+040B, U+0452 or
//! U+045B (Ђ, Ћ, ђ, ћ), its
//! Cyrillic atoms take `sr-Cyrl`. Script is the one resolved in Step 3, so
//! punctuation that joined CJK text follows it. The frequency of ideographs
//! decides nothing.

use super::LocalePolicy;
use crate::types::{LocaleTag, ScriptTag, TextRange};
use icu_segmenter::options::SentenceBreakInvariantOptions;
use icu_segmenter::{SentenceSegmenter, SentenceSegmenterBorrowed};
use std::sync::LazyLock;

static SENTENCES: LazyLock<SentenceSegmenterBorrowed<'static>> =
    LazyLock::new(|| SentenceSegmenter::new(SentenceBreakInvariantOptions::default()));

static JA: LazyLock<LocaleTag> = LazyLock::new(|| known("ja-JP"));
static KO: LazyLock<LocaleTag> = LazyLock::new(|| known("ko-KR"));
static SR: LazyLock<LocaleTag> = LazyLock::new(|| known("sr-Cyrl"));

fn known(tag: &str) -> LocaleTag {
    LocaleTag::try_from_str(tag).expect("a well-formed locale tag")
}

/// One atom as locale resolution sees it.
pub(crate) struct LocaleAtom<'a> {
    pub byte_range: TextRange,
    /// The atom's text.
    pub text: &'a str,
    /// Its script after Step 3.
    pub script: ScriptTag,
    /// The locale its style states, which stands.
    pub explicit: Option<&'a LocaleTag>,
}

/// The locale of every atom of the paragraph `text`, in order.
pub(crate) fn resolve(
    text: &str,
    atoms: &[LocaleAtom<'_>],
    default: &LocaleTag,
    policy: LocalePolicy,
) -> Vec<LocaleTag> {
    let mut out: Vec<LocaleTag> = atoms
        .iter()
        .map(|a| a.explicit.cloned().unwrap_or_else(|| default.clone()))
        .collect();
    if policy != LocalePolicy::ExperimentalHeuristics {
        return out;
    }
    let mut ends = SENTENCES.segment_str(text).filter(|&b| b > 0).peekable();
    let mut first = 0;
    while let Some(end) = ends.next() {
        let last = if ends.peek().is_none() {
            atoms.len()
        } else {
            first + atoms[first..].partition_point(|a| a.byte_range.start() < end)
        };
        let sentence = &atoms[first..last];
        let (mut has_kana, mut has_hangul, mut serbian) = (false, false, false);
        for a in sentence {
            match a.script {
                ScriptTag::HIRAGANA | ScriptTag::KATAKANA => has_kana = true,
                ScriptTag::HANGUL => has_hangul = true,
                _ => {}
            }
            if !serbian
                && a.text
                    .contains(['\u{0402}', '\u{040B}', '\u{0452}', '\u{045B}'])
            {
                serbian = true;
            }
        }
        if has_kana || has_hangul || serbian {
            for (k, atom) in sentence.iter().enumerate() {
                if atom.explicit.is_some() {
                    continue;
                }
                let s = atom.script;
                let locale = if has_kana
                    && matches!(
                        s,
                        ScriptTag::HAN | ScriptTag::HIRAGANA | ScriptTag::KATAKANA
                    ) {
                    Some(&*JA)
                } else if !has_kana && has_hangul && matches!(s, ScriptTag::HAN | ScriptTag::HANGUL)
                {
                    Some(&*KO)
                } else if serbian && s == ScriptTag::CYRILLIC {
                    Some(&*SR)
                } else {
                    None
                };
                if let Some(locale) = locale {
                    out[first + k] = locale.clone();
                }
            }
        }
        first = last;
        if first == atoms.len() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::script::char_script;

    fn locales(text: &str, tagged: &[(usize, &LocaleTag)], policy: LocalePolicy) -> Vec<String> {
        let atoms: Vec<LocaleAtom<'_>> = text
            .char_indices()
            .map(|(i, c)| {
                let s = char_script(c);
                LocaleAtom {
                    byte_range: TextRange::new(i, i + c.len_utf8()),
                    text: &text[i..i + c.len_utf8()],
                    // A stand-in for Step 3: Common takes Latin.
                    script: if s.is_unresolved() {
                        ScriptTag::LATIN
                    } else {
                        s
                    },
                    explicit: tagged.iter().find(|(at, _)| *at == i).map(|(_, l)| *l),
                }
            })
            .collect();
        resolve(text, &atoms, &known("en-US"), policy)
            .iter()
            .map(|l| l.as_str().to_owned())
            .collect()
    }

    #[test]
    fn explicit_only_gives_untagged_text_the_default() {
        let got = locales("漢字かな", &[], LocalePolicy::ExplicitOnly);
        assert!(got.iter().all(|l| l == "en-US"), "{got:?}");
    }

    #[test]
    fn kana_makes_han_japanese_and_hangul_makes_it_korean() {
        let h = LocalePolicy::ExperimentalHeuristics;
        // Kana in the sentence: Han and kana take ja-JP, Latin keeps the
        // default.
        assert_eq!(
            locales("漢字かなab", &[], h),
            ["ja-JP", "ja-JP", "ja-JP", "ja-JP", "en-US", "en-US"]
        );
        // Hangul: Han and Hangul take ko-KR.
        assert_eq!(locales("韓國한국", &[], h), ["ko-KR"; 4]);
        // Han alone takes the default, however much of it there is.
        assert_eq!(locales("漢字漢字", &[], h), ["en-US"; 4]);
        // The scope is the sentence: the kana of the first does not reach
        // the Han of the second.
        let got = locales("かな。 漢字。", &[], h);
        assert_eq!(got[0], "ja-JP");
        assert_eq!(got[got.len() - 2], "en-US");
    }

    #[test]
    fn serbian_letters_make_cyrillic_serbian_and_tags_win() {
        let h = LocalePolicy::ExperimentalHeuristics;
        assert_eq!(locales("ђак", &[], h), ["sr-Cyrl"; 3]);
        assert_eq!(locales("ЋАК", &[], h), ["sr-Cyrl"; 3]);
        assert_eq!(locales("ЂАК", &[], h), ["sr-Cyrl"; 3]);
        assert_eq!(locales("дак", &[], h), ["en-US"; 3]);
        // An explicit tag stands, and still counts as evidence.
        let zh = known("zh-Hant");
        assert_eq!(
            locales("漢かな", &[(0, &zh)], h),
            ["zh-Hant", "ja-JP", "ja-JP"]
        );
    }
}
