//! Frontend options (spec §3).

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_is_the_default() {
        assert_eq!(QuotePolicy::default(), QuotePolicy::Standard);
    }
}
