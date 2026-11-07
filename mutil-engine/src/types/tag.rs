//! Script and locale identifiers (spec §2.1).

use core::fmt;
use std::sync::Arc;

/// ISO 15924 script tag, four ASCII bytes in canonical case (`Latn`, `Arab`).
///
/// Spec §2.1: after frontend resolution no atom may remain `Zyyy` (Common) or
/// `Zinh` (Inherited) `[U4]`; see [`ScriptTag::is_unresolved`].
///
/// `Zsye` ([`ScriptTag::EMOJI`]) is **not** a Unicode `Script` property value:
/// emoji code points are `Script=Common` in the UCD. It is an
/// engine-internal font-fallback key **[Design decision, §2.1]**.
///
/// The inner array is private, as in the spec's `pub struct ScriptTag([u8; 4])`,
/// so construction goes through [`ScriptTag::from_bytes`] / [`ScriptTag::parse`].
/// Layout: 4 bytes, alignment 1.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptTag([u8; 4]);

impl ScriptTag {
    /// `Zyyy`: Common. Must not survive frontend resolution.
    pub const COMMON: Self = Self(*b"Zyyy");
    /// `Zinh`: Inherited. Must not survive frontend resolution.
    pub const INHERITED: Self = Self(*b"Zinh");
    /// `Zzzz`: unknown / unencoded.
    pub const UNKNOWN: Self = Self(*b"Zzzz");
    /// `Zsye`: engine pseudo-script routing clusters to colour emoji fonts.
    pub const EMOJI: Self = Self(*b"Zsye");
    /// `Latn`.
    pub const LATIN: Self = Self(*b"Latn");
    /// `Cyrl`.
    pub const CYRILLIC: Self = Self(*b"Cyrl");
    /// `Grek`.
    pub const GREEK: Self = Self(*b"Grek");
    /// `Arab`.
    pub const ARABIC: Self = Self(*b"Arab");
    /// `Hebr`.
    pub const HEBREW: Self = Self(*b"Hebr");
    /// `Deva`.
    pub const DEVANAGARI: Self = Self(*b"Deva");
    /// `Thai`.
    pub const THAI: Self = Self(*b"Thai");
    /// `Hani`: Han ideographs.
    pub const HAN: Self = Self(*b"Hani");
    /// `Hira`.
    pub const HIRAGANA: Self = Self(*b"Hira");
    /// `Kana`.
    pub const KATAKANA: Self = Self(*b"Kana");
    /// `Hang`.
    pub const HANGUL: Self = Self(*b"Hang");
    /// `Bopo`.
    pub const BOPOMOFO: Self = Self(*b"Bopo");

    /// Wraps four bytes in ISO 15924 canonical case: ASCII letters are
    /// normalised (first upper case, the rest lower case) so that
    /// `from_bytes(*b"LATN") == ScriptTag::LATIN` and `is_unresolved` matches
    /// `ZYYY`. Other bytes are kept as given; no further validation is done.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 4]) -> Self {
        let mut b = bytes;
        let mut i = 0;
        while i < 4 {
            b[i] = if i == 0 {
                b[i].to_ascii_uppercase()
            } else {
                b[i].to_ascii_lowercase()
            };
            i += 1;
        }
        Self(b)
    }

    /// Parses a four-letter ASCII tag in any case, normalising to the ISO 15924
    /// form (first letter upper case, the rest lower case).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 4 || !b.iter().all(u8::is_ascii_alphabetic) {
            return None;
        }
        Some(Self([
            b[0].to_ascii_uppercase(),
            b[1].to_ascii_lowercase(),
            b[2].to_ascii_lowercase(),
            b[3].to_ascii_lowercase(),
        ]))
    }

    /// The raw tag bytes.
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

impl fmt::Debug for ScriptTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ScriptTag({self})")
    }
}

impl fmt::Display for ScriptTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Tags built via `parse` are ASCII; `from_bytes` may carry anything, so
        // render non-ASCII bytes as U+FFFD rather than assuming UTF-8.
        for &b in &self.0 {
            let c = if b.is_ascii() {
                b as char
            } else {
                char::REPLACEMENT_CHARACTER
            };
            fmt::Write::write_char(f, c)?;
        }
        Ok(())
    }
}

/// BCP 47 language tag, for example `ja-JP` or `sr-Cyrl` (spec §2.1).
///
/// Stored as a shared string: locales are per `ShapingRun`, not per atom, so
/// one allocation is shared by every run and cache key that references it, and
/// cloning is a reference-count bump. The tag is **not validated** in Phase 0;
/// canonicalisation and the BCP 47 to OpenType language-system mapping arrive
/// with Frontend Step 4 and §4.4.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct LocaleTag(Arc<str>);

impl LocaleTag {
    /// Wraps a BCP 47 string as given.
    #[must_use]
    pub fn new(tag: impl Into<Arc<str>>) -> Self {
        Self(tag.into())
    }

    /// The undetermined locale, `und` (no explicit tag; spec §1.2 fallback
    /// rules apply).
    #[must_use]
    pub fn und() -> Self {
        Self::new("und")
    }

    /// The tag text.
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

impl fmt::Display for LocaleTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_parse_normalises_case() {
        assert_eq!(ScriptTag::parse("latn"), Some(ScriptTag::LATIN));
        assert_eq!(ScriptTag::parse("ZSYE"), Some(ScriptTag::EMOJI));
        assert_eq!(ScriptTag::parse("Lat"), None);
        assert_eq!(ScriptTag::parse("La1n"), None);
    }

    #[test]
    fn from_bytes_canonicalises_case() {
        assert_eq!(ScriptTag::from_bytes(*b"LATN"), ScriptTag::LATIN);
        assert_eq!(ScriptTag::from_bytes(*b"zyyy"), ScriptTag::COMMON);
        assert!(ScriptTag::from_bytes(*b"ZINH").is_unresolved());
        assert_eq!(ScriptTag::from_bytes(*b"Zsye"), ScriptTag::EMOJI);
        assert_eq!(
            ScriptTag::from_bytes([b'1', b'a', 0xFF, b'Z']).as_bytes(),
            [b'1', b'a', 0xFF, b'z']
        );
    }

    #[test]
    fn unresolved_means_common_or_inherited() {
        assert!(ScriptTag::COMMON.is_unresolved());
        assert!(ScriptTag::INHERITED.is_unresolved());
        assert!(!ScriptTag::EMOJI.is_unresolved());
        assert!(!ScriptTag::HAN.is_unresolved());
    }

    #[test]
    fn display_is_the_tag_text() {
        assert_eq!(ScriptTag::ARABIC.to_string(), "Arab");
        assert_eq!(LocaleTag::new("ja-JP").to_string(), "ja-JP");
        assert_eq!(LocaleTag::default().as_str(), "und");
    }
}
