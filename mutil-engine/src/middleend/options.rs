//! Layout options (spec §5 Stage 1.3 and Stage 4).

/// Hanging punctuation mode (spec §5 Stage 1.3; JLReq 2.5.1, CSS Text 4
/// `hanging-punctuation` `[W4][W2]`). At most one character hangs at each edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangMode {
    /// No hanging.
    #[default]
    None,
    /// A stop or comma hangs only if it would otherwise have to move to the
    /// next line.
    AllowEnd,
    /// A stop or comma hangs even if the line could have fitted it.
    ForceEnd,
}

/// Line height policy (spec §5 Stage 4.1; CSS Inline Layout 3 `[B7]`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LineHeight {
    /// `line-height: normal`: the union of the glyph bounds of every font used,
    /// plus the font line gap.
    #[default]
    Normal,
    /// A multiple of the font size taken from the first available font only;
    /// fallback glyphs may overflow the line box (standard CSS behaviour).
    Fixed(f32),
}

/// Trailing whitespace handling at a soft wrap (spec §5 Stage 1.7).
/// **[Design decision]** in the spec; CSS Text 3 `white-space` is the model `[W1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangingWhitespaceMode {
    /// CSS `normal`-like: trailing `WordSpace` and `CjkAutospace` collapse to
    /// zero visual width. They stay in `PositionedLine::byte_range`;
    /// `visible_byte_end` marks the last visible cluster.
    #[default]
    Collapsible,
    /// CSS `pre-wrap`-like: trailing spaces hang outside the line width but keep
    /// selectable, caret-reachable slots. An editor normally uses this on the
    /// line containing the caret.
    Preserved,
}

/// Line-breaking strategy (spec §5 Stage 1.6). The pair is the standard
/// dichotomy (Typst `Simple`/`Optimized`, Android `SIMPLE`/`HIGH_QUALITY`) `[L3][L6]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineBreakStrategy {
    /// Strategy A: first-fit greedy with the overflow ladder. Each line depends
    /// only on its start, which allows resynchronising incremental relayout.
    #[default]
    Greedy,
    /// Strategy B: global optimisation over the whole paragraph or a bounded
    /// window (Knuth-Plass demerits, §5 Stage 1.6).
    GlobalOptimal,
}

/// Clipboard policy for an actual U+00AD in the source text (spec §6.6, the
/// `copy_soft_hyphen` flag). The *generated* hyphen is never copied regardless.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SoftHyphenPolicy {
    /// Copy U+00AD as it is in the source (Firefox's behaviour `[C4]`).
    #[default]
    Keep,
    /// Drop U+00AD from copied text.
    Strip,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_spec_baselines() {
        assert_eq!(HangMode::default(), HangMode::None);
        assert_eq!(LineHeight::default(), LineHeight::Normal);
    }
}
