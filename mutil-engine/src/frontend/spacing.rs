//! Character classes of Phase 2 spacing (spec FE-20, FE-31, ATM-3): the
//! autospace classes, the punctuation of compression and hanging, and the
//! locales that turn compression off. Each answer reads one code point.

use super::segment::{LINE_BREAK, SCRIPT, SCRIPT_NAMES};
use crate::types::{LocaleTag, ScriptTag};
use icu_properties::props::{EastAsianWidth, GeneralCategory, LineBreak};
use icu_properties::{CodePointMapData, CodePointMapDataBorrowed};

static WIDTH: CodePointMapDataBorrowed<'static, EastAsianWidth> =
    CodePointMapData::<EastAsianWidth>::new();
static CATEGORY: CodePointMapDataBorrowed<'static, GeneralCategory> =
    CodePointMapData::<GeneralCategory>::new();

/// The autospace class of a code point (FE-20).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutospaceClass {
    Cjk,
    Western,
}

/// The `Script` property of `c` by its short name, without extensions.
fn script_name(c: char) -> &'static str {
    SCRIPT_NAMES.get(SCRIPT.get(c)).unwrap_or_default()
}

/// East Asian Width W or F.
fn is_wide_or_full(c: char) -> bool {
    matches!(
        WIDTH.get(c),
        EastAsianWidth::Wide | EastAsianWidth::Fullwidth
    )
}

/// General Category P*.
fn is_punctuation(c: char) -> bool {
    matches!(
        CATEGORY.get(c),
        GeneralCategory::ConnectorPunctuation
            | GeneralCategory::DashPunctuation
            | GeneralCategory::OpenPunctuation
            | GeneralCategory::ClosePunctuation
            | GeneralCategory::InitialPunctuation
            | GeneralCategory::FinalPunctuation
            | GeneralCategory::OtherPunctuation
    )
}

/// The autospace class of `c`, if any (FE-20). Punctuation is of neither.
pub(crate) fn autospace_class(c: char) -> Option<AutospaceClass> {
    let category = CATEGORY.get(c);
    if is_wide_or_full(c) {
        let cjk = matches!(script_name(c), "Hani" | "Hira" | "Kana" | "Hang" | "Bopo")
            && !is_punctuation(c)
            && category != GeneralCategory::Format
            && !matches!(
                LINE_BREAK.get(c),
                LineBreak::OpenPunctuation
                    | LineBreak::ClosePunctuation
                    | LineBreak::CloseParenthesis
                    | LineBreak::Quotation
                    | LineBreak::Inseparable
            );
        return cjk.then_some(AutospaceClass::Cjk);
    }
    let letter = matches!(
        category,
        GeneralCategory::UppercaseLetter
            | GeneralCategory::LowercaseLetter
            | GeneralCategory::TitlecaseLetter
            | GeneralCategory::ModifierLetter
            | GeneralCategory::OtherLetter
    ) && matches!(script_name(c), "Latn" | "Grek" | "Cyrl");
    (letter || category == GeneralCategory::DecimalNumber).then_some(AutospaceClass::Western)
}

/// A closing or stop mark after which compression may follow (FE-31):
/// U+3001, U+3002, U+FF0C, U+FF0E, U+FF1A, U+FF1B, closing quotes ’ / ”,
/// or a wide or full-width mark of UAX #14 class CL or CP.
pub(crate) fn is_compressible_close(c: char) -> bool {
    matches!(
        c,
        '\u{3001}'
            | '\u{3002}'
            | '\u{FF0C}'
            | '\u{FF0E}'
            | '\u{FF1A}'
            | '\u{FF1B}'
            | '\u{2019}'
            | '\u{201D}'
    ) || is_wide_or_full(c)
        && matches!(
            LINE_BREAK.get(c),
            LineBreak::ClosePunctuation | LineBreak::CloseParenthesis
        )
}

/// A wide or full-width punctuation mark or curly quote (FE-31).
pub(crate) fn is_wide_punctuation(c: char) -> bool {
    (is_wide_or_full(c) || matches!(c, '\u{2018}' | '\u{2019}' | '\u{201C}' | '\u{201D}'))
        && is_punctuation(c)
}

/// A wide or full-width opening bracket (UAX #14 class OP) or opening curly
/// quote (FE-31).
pub(crate) fn is_wide_opening(c: char) -> bool {
    (is_wide_or_full(c) && LINE_BREAK.get(c) == LineBreak::OpenPunctuation)
        || matches!(c, '\u{2018}' | '\u{201C}')
}

/// A CJK middle dot (FE-31): U+00B7, U+30FB, or U+2027.
pub(crate) const fn is_middle_dot(c: char) -> bool {
    matches!(c, '\u{00B7}' | '\u{30FB}' | '\u{2027}')
}

/// Closing or prohibited-start punctuation, after which no inter-character
/// gap opens before it (FE-31): UAX #14 class CL, CP, EX, IS or SY, or
/// U+3001, U+3002, U+FF0C or U+FF0E.
pub(crate) fn is_prohibited_start(c: char) -> bool {
    matches!(c, '\u{3001}' | '\u{3002}' | '\u{FF0C}' | '\u{FF0E}')
        || matches!(
            LINE_BREAK.get(c),
            LineBreak::ClosePunctuation
                | LineBreak::CloseParenthesis
                | LineBreak::Exclamation
                | LineBreak::InfixNumeric
                | LineBreak::BreakSymbols
        )
}

/// An opening mark that may hang at a line start (ATM-3): UAX #14 class OP,
/// or General Category Ps or Pi.
pub(crate) fn is_hangable_start(c: char) -> bool {
    LINE_BREAK.get(c) == LineBreak::OpenPunctuation
        || matches!(
            CATEGORY.get(c),
            GeneralCategory::OpenPunctuation | GeneralCategory::InitialPunctuation
        )
}

/// A stop or comma that may hang at a line end (ATM-3).
pub(crate) const fn is_hangable_end(c: char) -> bool {
    matches!(
        c,
        ',' | '.' | '\u{3001}' | '\u{3002}' | '\u{FF0C}' | '\u{FF0E}'
    )
}

/// The cursive scripts that take no letter spacing (FE-31).
pub(crate) fn takes_no_letter_spacing(script: ScriptTag) -> bool {
    matches!(
        &script.as_bytes(),
        b"Arab" | b"Rohg" | b"Mand" | b"Mong" | b"Nkoo" | b"Phag" | b"Syrc"
    )
}

/// Whether a locale turns punctuation compression off (FE-31): Traditional
/// Chinese, a `zh` tag with the script `Hant`, or with no script and the
/// region TW.
pub(crate) fn turns_compression_off(locale: &LocaleTag) -> bool {
    let mut subtags = locale.as_str().split('-');
    if !subtags
        .next()
        .is_some_and(|language| language.eq_ignore_ascii_case("zh"))
    {
        return false;
    }
    let mut script = None;
    let mut region = None;
    // Extensions and private use follow a singleton; they name no region.
    for subtag in subtags.take_while(|subtag| subtag.len() > 1) {
        match subtag.len() {
            4 if script.is_none() && subtag.bytes().all(|b| b.is_ascii_alphabetic()) => {
                script = Some(subtag);
            }
            2 if region.is_none() && subtag.bytes().all(|b| b.is_ascii_alphabetic()) => {
                region = Some(subtag);
            }
            _ => {}
        }
    }
    match script {
        Some(script) => script.eq_ignore_ascii_case("Hant"),
        None => region.is_some_and(|region| region.eq_ignore_ascii_case("TW")),
    }
}
