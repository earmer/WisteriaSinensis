//! Frontend options (spec §3).

use crate::types::{BidiDirection, LocaleTag, TextOrientation, WritingMode};

/// How quotation marks and paired punctuation resolve their script
/// (spec §3 Step 3).
///
/// The names are this crate's: the spec calls only the second one
/// `QuotePolicy::CjkAware` and describes the first as "the default algorithm".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum QuotePolicy {
    /// Spec §3 Step 3, FE-13 default: `Script_Extensions` set intersection with a bounded stack;
    /// paired punctuation takes the enclosing text's script `[U4][S1]`.
    #[default]
    Standard,
    /// Spec §3 Step 3, FE-14 opt-in: pairs adjacent to CJK text take a CJK script so the quotation
    /// glyphs come from a CJK font. Deterministic for a given text, but **not
    /// stable under edits** (an unclosed opener re-resolves once its closer is
    /// typed).
    CjkAware,
}

/// Size profile for `GlueKind::CjkAutospace` (spec §2.4 table).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AutospaceProfile {
    /// JLReq 3.8.3/3.8.4 `[W4]`: natural 1/4 em, shrinking to 1/8 em, stretching
    /// to 1/2 em (1/3 em in some styles). Default for `ja`.
    #[default]
    Jlreq,
    /// CSS Text 4 `text-autospace` `[W2]`: 0.125 ic (1/8 of the CJK advance);
    /// shrink and stretch bounds are not specified.
    CssCompat,
}

/// Everything the frontend needs besides the text and its styled runs
/// (spec §3): the single input carrier for analysis policy.
///
/// Deliberately has no line-height policy: the frontend records raw strut
/// metrics and never applies `LineHeight` (that is the middle-end's job).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FrontendOptions {
    /// Paragraph writing mode.
    pub writing_mode: WritingMode,
    /// Vertical orientation policy.
    pub text_orientation: TextOrientation,
    /// Whether untagged text can trigger experimental locale inference.
    pub locale_policy: LocalePolicy,
    /// Profile for `GlueKind::CjkAutospace` sizes.
    pub autospace_profile: AutospaceProfile,
    /// How quotation marks resolve their script.
    pub quote_policy: QuotePolicy,
    /// Locale for runs that carry none (spec §1.2 fallback rules).
    pub default_locale: LocaleTag,
    /// The caller's paragraph direction, which wins over UAX #9 P2 and P3
    /// (rule HL1, spec FE-8); `None` lets the text decide.
    pub base_direction: Option<BidiDirection>,
    /// Line-breaking strictness (spec FE-18).
    pub line_break_strictness: LineBreakStrictness,
    /// Word-breaking behaviour inside words (spec FE-18).
    pub word_break_option: WordBreakOption,
}

/// Line-breaking strictness, after CSS Text `line-break` (spec FE-18).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineBreakStrictness {
    /// The least restrictive rules, for short lines.
    Loose,
    /// The common rules.
    #[default]
    Normal,
    /// The most restrictive rules.
    Strict,
    /// A soft wrap opportunity around every typographic character unit.
    Anywhere,
}

/// Word-breaking behaviour, after CSS Text `word-break` (spec FE-18).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WordBreakOption {
    /// Breaks follow the line-breaking rules.
    #[default]
    Normal,
    /// Breaks are allowed between any two letters of non-CJK text.
    BreakAll,
    /// No break inside CJK words, which wrap like other words.
    KeepAll,
}

/// Language inference policy. Heuristics require explicit opt-in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LocalePolicy {
    /// Use only explicit tags and the host default locale.
    #[default]
    ExplicitOnly,
    /// Experimental script-based inference.
    ///
    /// **Forward contract.** No locale-resolution pass exists yet, so
    /// selecting this policy currently has no effect; it is
    /// reserved for the script-based rules in spec §3 Step 4.
    ExperimentalHeuristics,
}
