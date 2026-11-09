//! Frontend options (spec §3).

use crate::types::LocaleTag;

/// How quotation marks and paired punctuation resolve their script
/// (spec §3 Step 3).
///
/// The names are this crate's: the spec calls only the second one
/// `QuotePolicy::CjkAware` and describes the first as "the default algorithm".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum QuotePolicy {
    /// §3.1 default: `Script_Extensions` set intersection with a bounded stack;
    /// paired punctuation takes the enclosing text's script `[U4][S1]`.
    #[default]
    Standard,
    /// §3.2 opt-in: pairs adjacent to CJK text take a CJK script so the quotation
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
    pub writing_mode: crate::types::WritingMode,
    /// Vertical orientation policy.
    pub text_orientation: crate::types::TextOrientation,
    /// Whether untagged text can trigger experimental locale inference.
    pub locale_policy: LocalePolicy,
    /// Profile for `GlueKind::CjkAutospace` sizes.
    pub autospace_profile: AutospaceProfile,
    /// How quotation marks resolve their script.
    pub quote_policy: QuotePolicy,
    /// Locale for runs that carry none (spec §1.2 fallback rules).
    pub default_locale: LocaleTag,
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
