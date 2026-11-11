//! Script and locale identifiers (spec §2.1).
//!
//! Both tags are parsed, not validated: the only constructors check the
//! syntax, so a value that exists is well formed and no later stage re-checks
//! it.

use core::fmt;
use core::str::FromStr;
use std::sync::Arc;

/// Four bytes that are not an ISO 15924 script code (four ASCII letters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InvalidScriptTag;

impl fmt::Display for InvalidScriptTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a script tag is four ASCII letters")
    }
}

impl std::error::Error for InvalidScriptTag {}

/// ISO 15924 script code: four ASCII letters in canonical case (`Latn`,
/// `Arab`).
///
/// The syntax is guaranteed by construction ([`ScriptTag::try_from_bytes`],
/// [`ScriptTag::try_from_str`]); whether the code is registered is not
/// checked, since that needs the ISO 15924 registry.
///
/// Spec §2.1: after frontend resolution no atom may remain `Zyyy` (Common) or
/// `Zinh` (Inherited) `[U4]`; see [`ScriptTag::is_unresolved`].
///
/// `Zsye` ([`ScriptTag::EMOJI`]) is **not** a Unicode `Script` property value:
/// emoji code points are `Script=Common` in the UCD. It is an
/// engine-internal font-fallback key **[Design decision, §2.1]**.
///
/// Layout: 4 bytes, alignment 1.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptTag([u8; 4]);

impl ScriptTag {
    /// `Zyyy`: Common. Must not survive frontend resolution.
    pub const COMMON: Self = Self::known(*b"Zyyy");
    /// `Zinh`: Inherited. Must not survive frontend resolution.
    pub const INHERITED: Self = Self::known(*b"Zinh");
    /// `Zzzz`: unknown / unencoded.
    pub const UNKNOWN: Self = Self::known(*b"Zzzz");
    /// `Zsye`: engine pseudo-script routing clusters to colour emoji fonts.
    pub const EMOJI: Self = Self::known(*b"Zsye");
    /// `Latn`.
    pub const LATIN: Self = Self::known(*b"Latn");
    /// `Cyrl`.
    pub const CYRILLIC: Self = Self::known(*b"Cyrl");
    /// `Grek`.
    pub const GREEK: Self = Self::known(*b"Grek");
    /// `Arab`.
    pub const ARABIC: Self = Self::known(*b"Arab");
    /// `Hebr`.
    pub const HEBREW: Self = Self::known(*b"Hebr");
    /// `Deva`.
    pub const DEVANAGARI: Self = Self::known(*b"Deva");
    /// `Thai`.
    pub const THAI: Self = Self::known(*b"Thai");
    /// `Hani`: Han ideographs.
    pub const HAN: Self = Self::known(*b"Hani");
    /// `Hira`.
    pub const HIRAGANA: Self = Self::known(*b"Hira");
    /// `Kana`.
    pub const KATAKANA: Self = Self::known(*b"Kana");
    /// `Hang`.
    pub const HANGUL: Self = Self::known(*b"Hang");
    /// `Bopo`.
    pub const BOPOMOFO: Self = Self::known(*b"Bopo");

    /// A constant of this module. An invalid literal fails the build.
    const fn known(bytes: [u8; 4]) -> Self {
        match Self::try_from_bytes(bytes) {
            Ok(tag) => tag,
            Err(InvalidScriptTag) => panic!("script constants are four ASCII letters"),
        }
    }

    /// Parses four ASCII letters in any case, normalising to the ISO 15924
    /// form (first letter upper case, the rest lower case), so that
    /// `try_from_bytes(*b"LATN") == Ok(ScriptTag::LATIN)`.
    ///
    /// # Errors
    /// A byte is not an ASCII letter.
    pub const fn try_from_bytes(bytes: [u8; 4]) -> Result<Self, InvalidScriptTag> {
        let mut canonical = bytes;
        let mut i = 0;
        while i < 4 {
            if !bytes[i].is_ascii_alphabetic() {
                return Err(InvalidScriptTag);
            }
            canonical[i] = if i == 0 {
                bytes[i].to_ascii_uppercase()
            } else {
                bytes[i].to_ascii_lowercase()
            };
            i += 1;
        }
        Ok(Self(canonical))
    }

    /// Parses a four-letter ASCII tag in any case; see
    /// [`try_from_bytes`](Self::try_from_bytes).
    ///
    /// # Errors
    /// `tag` is not exactly four ASCII letters.
    pub const fn try_from_str(tag: &str) -> Result<Self, InvalidScriptTag> {
        match *tag.as_bytes() {
            [a, b, c, d] => Self::try_from_bytes([a, b, c, d]),
            _ => Err(InvalidScriptTag),
        }
    }

    /// The tag bytes, in canonical case.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> [u8; 4] {
        self.0
    }

    /// `true` for `Zyyy` and `Zinh`, the two values the frontend must have
    /// eliminated from every atom (spec §2.1).
    #[inline]
    #[must_use]
    pub const fn is_unresolved(&self) -> bool {
        matches!(&self.0, b"Zyyy" | b"Zinh")
    }
}

impl FromStr for ScriptTag {
    type Err = InvalidScriptTag;

    fn from_str(tag: &str) -> Result<Self, Self::Err> {
        Self::try_from_str(tag)
    }
}

impl fmt::Debug for ScriptTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ScriptTag({self})")
    }
}

impl fmt::Display for ScriptTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|&byte| fmt::Write::write_char(f, char::from(byte)))
    }
}

/// A string that is not a well-formed BCP 47 language tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InvalidLocaleTag;

impl fmt::Display for InvalidLocaleTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a well-formed BCP 47 language tag")
    }
}

impl std::error::Error for InvalidLocaleTag {}

/// BCP 47 language tag, for example `ja-JP` or `sr-Cyrl` (spec §2.1).
///
/// **Well formed by construction.** [`LocaleTag::try_from_str`] accepts exactly
/// the `Language-Tag` grammar of RFC 5646 §2.1 (a `langtag`, a private-use tag
/// or a grandfathered tag) and stores the tag in the letter case RFC 5646
/// §2.1.1 recommends (`ja-JP`, `sr-Cyrl`, `zh-Hant-TW`), so equal tags compare
/// and hash equal. That is the RFC's "well-formed" class: no subtag is looked
/// up in the IANA registry, and repeated variants or extension singletons are
/// not rejected. The BCP 47 to OpenType language-system mapping arrives with
/// §4.4.
///
/// Stored as a shared string: locales are per `ShapingRun`, not per atom, so
/// one allocation is shared by every run and cache key that references it, and
/// cloning is a reference-count bump.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct LocaleTag(Arc<str>);

impl LocaleTag {
    /// Parses a BCP 47 tag in any letter case.
    ///
    /// # Errors
    /// `tag` does not match the RFC 5646 `Language-Tag` grammar.
    pub fn try_from_str(tag: &str) -> Result<Self, InvalidLocaleTag> {
        canonical_language_tag(tag)
            .map(|canonical| Self(canonical.into()))
            .ok_or(InvalidLocaleTag)
    }

    /// The undetermined locale, `und` (no explicit tag; spec §1.2 fallback
    /// rules apply).
    #[must_use]
    pub fn und() -> Self {
        Self("und".into())
    }

    /// The tag text, in canonical case.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for LocaleTag {
    fn default() -> Self {
        Self::und()
    }
}

impl FromStr for LocaleTag {
    type Err = InvalidLocaleTag;

    fn from_str(tag: &str) -> Result<Self, Self::Err> {
        Self::try_from_str(tag)
    }
}

impl fmt::Display for LocaleTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The `irregular` grandfathered tags of RFC 5646 §2.1, which the `langtag`
/// production does not derive. The `regular` ones (`art-lojban`, `zh-min-nan`,
/// ...) are ordinary `langtag`s.
const IRREGULAR_GRANDFATHERED: [&str; 17] = [
    "en-GB-oed",
    "i-ami",
    "i-bnn",
    "i-default",
    "i-enochian",
    "i-hak",
    "i-klingon",
    "i-lux",
    "i-mingo",
    "i-navajo",
    "i-pwn",
    "i-tao",
    "i-tay",
    "i-tsu",
    "sgn-BE-FR",
    "sgn-BE-NL",
    "sgn-CH-DE",
];

/// `tag` in RFC 5646 §2.1.1 case if it matches the `Language-Tag` grammar.
fn canonical_language_tag(tag: &str) -> Option<String> {
    if let Some(grandfathered) = IRREGULAR_GRANDFATHERED
        .iter()
        .find(|known| known.eq_ignore_ascii_case(tag))
    {
        return Some((*grandfathered).to_owned());
    }
    let mut subtags = Subtags {
        rest: tag.split('-').peekable(),
        canonical: String::with_capacity(tag.len()),
    };
    if !subtags.private_use()? {
        subtags.langtag()?;
    }
    // Every subtag must have been consumed by a production.
    subtags.rest.next().is_none().then_some(subtags.canonical)
}

/// Letter case of one subtag in the canonical form (RFC 5646 §2.1.1).
#[derive(Clone, Copy)]
enum SubtagCase {
    Lower,
    Upper,
    Title,
}

fn is_alpha(subtag: &str) -> bool {
    subtag.bytes().all(|b| b.is_ascii_alphabetic())
}

fn is_digit(subtag: &str) -> bool {
    subtag.bytes().all(|b| b.is_ascii_digit())
}

fn is_alphanum(subtag: &str) -> bool {
    subtag.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn is_private_use_prefix(subtag: &str) -> bool {
    subtag.eq_ignore_ascii_case("x")
}

/// Recursive-descent parser over the `-`-separated subtags of a language tag,
/// accumulating the canonical spelling of what it accepts.
struct Subtags<'a> {
    rest: core::iter::Peekable<core::str::Split<'a, char>>,
    canonical: String,
}

impl<'a> Subtags<'a> {
    /// Consumes the next subtag if `accepts` it, appending it in `case`.
    fn take(&mut self, case: SubtagCase, accepts: impl Fn(&str) -> bool) -> Option<&'a str> {
        let subtag = self.rest.next_if(|subtag| accepts(subtag))?;
        if !self.canonical.is_empty() {
            self.canonical.push('-');
        }
        // An accepted subtag is ASCII and non-empty, so byte 1 is a boundary.
        let (head, tail) = subtag.split_at(1);
        match case {
            SubtagCase::Lower => self.canonical.push_str(&subtag.to_ascii_lowercase()),
            SubtagCase::Upper => self.canonical.push_str(&subtag.to_ascii_uppercase()),
            SubtagCase::Title => {
                self.canonical.push_str(&head.to_ascii_uppercase());
                self.canonical.push_str(&tail.to_ascii_lowercase());
            }
        }
        Some(subtag)
    }

    /// Consumes lower-case subtags while `accepts` holds; `None` if there is
    /// not at least one.
    fn take_some(&mut self, accepts: impl Fn(&str) -> bool) -> Option<()> {
        self.take(SubtagCase::Lower, &accepts)?;
        while self.take(SubtagCase::Lower, &accepts).is_some() {}
        Some(())
    }

    /// `privateuse = "x" 1*("-" (1*8alphanum))`. `Some(false)` when the next
    /// subtag does not open a private-use sequence, `None` when it opens an
    /// empty one.
    fn private_use(&mut self) -> Option<bool> {
        if self
            .take(SubtagCase::Lower, is_private_use_prefix)
            .is_none()
        {
            return Some(false);
        }
        self.take_some(|s| (1..=8).contains(&s.len()) && is_alphanum(s))?;
        Some(true)
    }

    /// `langtag = language ["-" script] ["-" region] *("-" variant)
    /// *("-" extension) ["-" privateuse]`.
    fn langtag(&mut self) -> Option<()> {
        use SubtagCase::{Lower, Title, Upper};
        // language = 2*3ALPHA ["-" extlang] / 4ALPHA / 5*8ALPHA
        let language = self.take(Lower, |s| (2..=8).contains(&s.len()) && is_alpha(s))?;
        if language.len() <= 3 {
            // extlang = 3ALPHA *2("-" 3ALPHA)
            for _ in 0..3 {
                if self.take(Lower, |s| s.len() == 3 && is_alpha(s)).is_none() {
                    break;
                }
            }
        }
        // script = 4ALPHA
        self.take(Title, |s| s.len() == 4 && is_alpha(s));
        // region = 2ALPHA / 3DIGIT
        self.take(Upper, |s| {
            (s.len() == 2 && is_alpha(s)) || (s.len() == 3 && is_digit(s))
        });
        // variant = 5*8alphanum / (DIGIT 3alphanum)
        let is_variant = |s: &str| {
            is_alphanum(s)
                && ((5..=8).contains(&s.len())
                    || (s.len() == 4 && s.as_bytes()[0].is_ascii_digit()))
        };
        while self.take(Lower, is_variant).is_some() {}
        // extension = singleton 1*("-" (2*8alphanum)); a singleton is any
        // alphanumeric but "x".
        let is_singleton = |s: &str| s.len() == 1 && is_alphanum(s) && !is_private_use_prefix(s);
        while self.take(Lower, is_singleton).is_some() {
            self.take_some(|s| (2..=8).contains(&s.len()) && is_alphanum(s))?;
        }
        self.private_use()?;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_tags_parse_and_normalise_case() {
        assert_eq!(ScriptTag::try_from_str("latn"), Ok(ScriptTag::LATIN));
        assert_eq!("ZSYE".parse(), Ok(ScriptTag::EMOJI));
        assert_eq!(ScriptTag::try_from_bytes(*b"LATN"), Ok(ScriptTag::LATIN));
        assert_eq!(ScriptTag::try_from_bytes(*b"zyyy"), Ok(ScriptTag::COMMON));
        assert_eq!(ScriptTag::LATIN.as_bytes(), *b"Latn");
    }

    #[test]
    fn a_script_tag_is_exactly_four_ascii_letters() {
        for bad in ["Lat", "Latin", "La1n", "Lat ", "", "Latñ"] {
            assert_eq!(ScriptTag::try_from_str(bad), Err(InvalidScriptTag), "{bad}");
        }
        assert_eq!(
            ScriptTag::try_from_bytes([b'1', b'a', 0xFF, b'Z']),
            Err(InvalidScriptTag)
        );
    }

    #[test]
    fn unresolved_means_common_or_inherited() {
        assert!(ScriptTag::COMMON.is_unresolved());
        assert!(ScriptTag::INHERITED.is_unresolved());
        assert!(ScriptTag::try_from_bytes(*b"ZINH").unwrap().is_unresolved());
        assert!(!ScriptTag::EMOJI.is_unresolved());
        assert!(!ScriptTag::HAN.is_unresolved());
    }

    #[test]
    fn display_is_the_tag_text() {
        assert_eq!(ScriptTag::ARABIC.to_string(), "Arab");
        assert_eq!(format!("{:?}", ScriptTag::ARABIC), "ScriptTag(Arab)");
        assert_eq!(
            LocaleTag::try_from_str("ja-JP").unwrap().to_string(),
            "ja-JP"
        );
        assert_eq!(LocaleTag::default().as_str(), "und");
    }

    fn locale(tag: &str) -> Option<String> {
        LocaleTag::try_from_str(tag)
            .ok()
            .map(|tag| tag.as_str().to_owned())
    }

    #[test]
    fn locale_tags_follow_the_rfc_5646_grammar() {
        // RFC 5646 Appendix A, in canonical case.
        for tag in [
            "de",
            "fr",
            "ja",
            "zh-Hant",
            "zh-Hans-CN",
            "sr-Latn-RS",
            "zh-cmn-Hans-CN",
            "zh-yue-HK",
            "sl-rozaj",
            "sl-rozaj-biske",
            "de-CH-1901",
            "sl-IT-nedis",
            "hy-Latn-IT-arevela",
            "es-419",
            "de-CH-x-phonebk",
            "az-Arab-x-aze-derbend",
            "x-whatever",
            "qaa-Qaaa-QM-x-southern",
            "en-US-u-islamcal",
            "zh-CN-a-myext-x-private",
            "en-a-myext-b-another",
            "art-lojban",
            "zh-min-nan",
            "i-klingon",
            "en-GB-oed",
            "und",
        ] {
            assert_eq!(locale(tag).as_deref(), Some(tag), "{tag}");
        }
    }

    #[test]
    fn locale_tags_are_stored_in_canonical_case() {
        assert_eq!(locale("JA-jp").as_deref(), Some("ja-JP"));
        assert_eq!(locale("SR-CYRL-rs").as_deref(), Some("sr-Cyrl-RS"));
        assert_eq!(
            locale("EN-us-U-CA-Gregory").as_deref(),
            Some("en-US-u-ca-gregory")
        );
        assert_eq!(locale("X-Private").as_deref(), Some("x-private"));
        assert_eq!(locale("I-KLINGON").as_deref(), Some("i-klingon"));
        assert_eq!(locale("sgn-be-fr").as_deref(), Some("sgn-BE-FR"));
        assert_eq!(
            LocaleTag::try_from_str("ja-jp"),
            LocaleTag::try_from_str("JA-JP")
        );
    }

    #[test]
    fn malformed_locale_tags_are_rejected() {
        for bad in [
            "",
            "j",
            "ja_JP",
            "ja-",
            "-ja",
            "ja--JP",
            "ja JP",
            "abcdefghi",
            "de-419-DE",
            "a-DE",
            "ar-a-aaa-b-bbb-a",
            "en-a",
            "x",
            "x-",
            "en-x",
            "x-abcdefghi",
            "en-US-abc",
            "日本語",
            "i-nonexistent",
        ] {
            // `ar-a-aaa-b-bbb-a` ends in a singleton without a subtag.
            assert_eq!(locale(bad), None, "{bad:?}");
        }
    }
}
