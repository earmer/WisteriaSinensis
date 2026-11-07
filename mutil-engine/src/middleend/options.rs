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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_spec_baselines() {
        assert_eq!(HangMode::default(), HangMode::None);
        assert_eq!(LineHeight::default(), LineHeight::Normal);
    }
}
