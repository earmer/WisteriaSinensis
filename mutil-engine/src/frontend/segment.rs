//! Segmentation (spec §3 Steps 1 and 6, FE-5, FE-18): grapheme boundaries,
//! line-break opportunities and per-character properties, all from the
//! compiled ICU4X data the lock file pins (spec SCO-8).
//!
//! The segmenters and property maps are built once per process: the
//! grapheme segmenter and the property maps are `const` borrows of compiled
//! data, and the line segmenters, one per option set, are built on first use.

use super::{LineBreakStrictness, WordBreakOption};
use crate::types::LineBreakClass;
use icu_properties::PropertyNamesShortBorrowed;
use icu_properties::props::{LineBreak, Script};
use icu_properties::{CodePointMapData, CodePointMapDataBorrowed, PropertyNamesShort};
use icu_segmenter::options::LineBreakWordOption;
use icu_segmenter::options::WordBreakInvariantOptions;
use icu_segmenter::options::{LineBreakOptions, LineBreakStrictness as IcuStrictness};
use icu_segmenter::{GraphemeClusterSegmenter, GraphemeClusterSegmenterBorrowed};
use icu_segmenter::{LineSegmenter, LineSegmenterBorrowed, WordSegmenter, WordSegmenterBorrowed};
use std::sync::LazyLock;

static GRAPHEMES: GraphemeClusterSegmenterBorrowed<'static> = GraphemeClusterSegmenter::new();
pub(crate) static LINE_BREAK: CodePointMapDataBorrowed<'static, LineBreak> =
    CodePointMapData::<LineBreak>::new();
static LINE_BREAK_NAMES: PropertyNamesShortBorrowed<'static, LineBreak> =
    PropertyNamesShort::<LineBreak>::new();
pub(crate) static SCRIPT: CodePointMapDataBorrowed<'static, Script> =
    CodePointMapData::<Script>::new();
pub(crate) static SCRIPT_NAMES: PropertyNamesShortBorrowed<'static, Script> =
    PropertyNamesShort::<Script>::new();

/// One line segmenter per `(strictness, word option)` pair; `Anywhere` needs
/// none (see [`line_opportunities`]).
static LINE_SEGMENTERS: LazyLock<[[LineSegmenterBorrowed<'static>; 3]; 3]> = LazyLock::new(|| {
    let strictness = [
        IcuStrictness::Loose,
        IcuStrictness::Normal,
        IcuStrictness::Strict,
    ];
    let words = [
        LineBreakWordOption::Normal,
        LineBreakWordOption::BreakAll,
        LineBreakWordOption::KeepAll,
    ];
    strictness.map(|s| {
        words.map(|w| {
            let mut options = LineBreakOptions::default();
            options.strictness = Some(s);
            options.word_option = Some(w);
            LineSegmenter::new_auto(options)
        })
    })
});

/// The word segmenter of the word motions (spec FE-24, INT-14).
static WORDS: LazyLock<WordSegmenterBorrowed<'static>> =
    LazyLock::new(|| WordSegmenter::new_auto(WordBreakInvariantOptions::default()));

/// The UAX #29 word boundaries of `text`, ascending, `0` and `text.len()`
/// included.
pub(crate) fn word_boundaries(text: &str) -> Vec<usize> {
    WORDS.segment_str(text).collect()
}

/// The extended grapheme cluster boundaries of `text` (UAX #29, GB9c
/// included), ascending, from `0` to `text.len()` inclusive.
pub(crate) fn grapheme_boundaries(text: &str) -> Vec<usize> {
    GRAPHEMES.segment_str(text).collect()
}

/// The UAX #14 break opportunities of `text` under the given options,
/// ascending, excluding `0`. `Anywhere` allows a break at every grapheme
/// boundary.
pub(crate) fn line_opportunities(
    text: &str,
    strictness: LineBreakStrictness,
    word_option: WordBreakOption,
    graphemes: &[usize],
) -> Vec<usize> {
    let s = match strictness {
        LineBreakStrictness::Loose => 0,
        LineBreakStrictness::Normal => 1,
        LineBreakStrictness::Strict => 2,
        LineBreakStrictness::Anywhere => {
            return graphemes.get(1..).unwrap_or_default().to_vec();
        }
    };
    let w = match word_option {
        WordBreakOption::Normal => 0,
        WordBreakOption::BreakAll => 1,
        WordBreakOption::KeepAll => 2,
    };
    LINE_SEGMENTERS[s][w]
        .segment_str(text)
        .filter(|&b| b > 0)
        .collect()
}

/// The UAX #14 class of `c` (spec ATM-2).
pub(crate) fn line_break_class(c: char) -> LineBreakClass {
    use LineBreakClass as L;
    match LINE_BREAK_NAMES.get(LINE_BREAK.get(c)).unwrap_or("XX") {
        "BK" => L::Bk,
        "CR" => L::Cr,
        "LF" => L::Lf,
        "CM" => L::Cm,
        "NL" => L::Nl,
        "SG" => L::Sg,
        "WJ" => L::Wj,
        "ZW" => L::Zw,
        "GL" => L::Gl,
        "SP" => L::Sp,
        "ZWJ" => L::Zwj,
        "B2" => L::B2,
        "BA" => L::Ba,
        "BB" => L::Bb,
        "HY" => L::Hy,
        "HH" => L::Hh,
        "CB" => L::Cb,
        "CL" => L::Cl,
        "CP" => L::Cp,
        "EX" => L::Ex,
        "IN" => L::In,
        "NS" => L::Ns,
        "OP" => L::Op,
        "QU" => L::Qu,
        "IS" => L::Is,
        "NU" => L::Nu,
        "PO" => L::Po,
        "PR" => L::Pr,
        "SY" => L::Sy,
        "AI" => L::Ai,
        "AK" => L::Ak,
        "AL" => L::Al,
        "AP" => L::Ap,
        "AS" => L::As,
        "CJ" => L::Cj,
        "EB" => L::Eb,
        "EM" => L::Em,
        "H2" => L::H2,
        "H3" => L::H3,
        "HL" => L::Hl,
        "ID" => L::Id,
        "JL" => L::Jl,
        "JT" => L::Jt,
        "JV" => L::Jv,
        "RI" => L::Ri,
        "SA" => L::Sa,
        "VF" => L::Vf,
        "VI" => L::Vi,
        _ => L::Xx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_devanagari_conjunct_is_one_grapheme() {
        // क्ष: KA, VIRAMA, SSA (GB9c).
        let text = "\u{915}\u{94D}\u{937}";
        assert_eq!(grapheme_boundaries(text), [0, text.len()]);
        assert_eq!(grapheme_boundaries(""), [0]);
    }

    #[test]
    fn spaces_offer_a_break_after_the_last_one() {
        let text = "a  b";
        let g = grapheme_boundaries(text);
        assert_eq!(
            line_opportunities(
                text,
                LineBreakStrictness::Normal,
                WordBreakOption::Normal,
                &g
            ),
            [3, 4]
        );
        assert_eq!(
            line_opportunities(
                text,
                LineBreakStrictness::Anywhere,
                WordBreakOption::Normal,
                &g
            ),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn classes_map_by_their_short_names() {
        assert_eq!(line_break_class('a'), LineBreakClass::Al);
        assert_eq!(line_break_class(' '), LineBreakClass::Sp);
        assert_eq!(line_break_class('\u{2028}'), LineBreakClass::Bk);
        assert_eq!(line_break_class('('), LineBreakClass::Op);
        assert_eq!(line_break_class('\u{a0}'), LineBreakClass::Gl);
    }
}
