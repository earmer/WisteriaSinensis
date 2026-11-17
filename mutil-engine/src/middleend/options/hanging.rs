//! Hanging punctuation configuration (spec §5 Stage 1.3).

use crate::middleend::HangSide;
use crate::types::Direction;

/// Hanging at the trailing edge of a line (spec §5 Stage 1.3; CSS Text 4
/// `hanging-punctuation: force-end | allow-end` `[W2]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangEndMode {
    /// Nothing hangs at the line end.
    #[default]
    None,
    /// A stop or comma hangs only if it would otherwise have to move to the
    /// next line (JLReq 2.5.1 `[W4]`; CSS `allow-end`).
    Allow,
    /// A stop or comma at the line end always hangs outside the text block,
    /// even if the line could have fitted it (CSS `force-end`).
    Force,
}

/// Hanging at the leading edge of a line (spec §5 Stage 1.3; CSS Text 4
/// `hanging-punctuation: first` `[W2]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HangStartMode {
    /// Nothing hangs at the line start.
    #[default]
    None,
    /// An opening mark may hang at the start of the **first line of the
    /// paragraph** only (CSS `first`). "First line" is line index 0; a line
    /// after a hard break is not a first line.
    FirstLine,
    /// An opening mark may hang at the start of **every** line.
    AllLines,
}

/// Hanging punctuation configuration (spec §5 Stage 1.3; JLReq 2.5.1, CSS Text
/// 4 `hanging-punctuation` `[W4][W2]`). At most one character hangs at each
/// edge.
///
/// The two edges are independent. They are **logical** (start and end of the
/// line in the paragraph's base direction), whereas
/// `PositionedLine::hanging_left()` and `hanging_right()` are **physical**: the
/// start edge is the left one for an LTR paragraph and the right one for an RTL
/// paragraph, the end edge the opposite. [`HangMode::allows`] is that mapping
/// (plus the first-line rule), and `PositionedLine::validate` enforces it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HangMode {
    /// Hanging at the start of a line.
    pub start: HangStartMode,
    /// Hanging at the end of a line.
    pub end: HangEndMode,
}

impl HangMode {
    /// Nothing hangs anywhere (the default).
    pub const NONE: Self = Self {
        start: HangStartMode::None,
        end: HangEndMode::None,
    };

    /// Whether punctuation may hang at the physical `side` of a line in a
    /// paragraph of the given base `direction`; `is_first_line` is whether the
    /// line is the paragraph's first (line index 0).
    #[must_use]
    pub const fn allows(self, side: HangSide, direction: Direction, is_first_line: bool) -> bool {
        if !side.is_line_start(direction) {
            return !matches!(self.end, HangEndMode::None);
        }
        match self.start {
            HangStartMode::None => false,
            HangStartMode::FirstLine => is_first_line,
            HangStartMode::AllLines => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hang_mode_grades_start_and_end_independently() {
        use Direction::{Ltr, Rtl};
        const L: HangSide = HangSide::Left;
        const R: HangSide = HangSide::Right;
        let mode = |start, end| HangMode { start, end };
        use HangEndMode as E;
        use HangStartMode as S;
        // (mode, direction, first line?) -> (left allowed, right allowed)
        let cases = [
            (HangMode::NONE, Ltr, true, (false, false)),
            (mode(S::FirstLine, E::None), Ltr, true, (true, false)),
            (mode(S::FirstLine, E::None), Ltr, false, (false, false)),
            (mode(S::FirstLine, E::None), Rtl, true, (false, true)),
            (mode(S::FirstLine, E::None), Rtl, false, (false, false)),
            (mode(S::AllLines, E::None), Ltr, false, (true, false)),
            (mode(S::AllLines, E::None), Rtl, false, (false, true)),
            // The end edge ignores the line index.
            (mode(S::None, E::Allow), Ltr, false, (false, true)),
            (mode(S::None, E::Force), Rtl, false, (true, false)),
            // Both edges at once.
            (mode(S::FirstLine, E::Force), Ltr, true, (true, true)),
            (mode(S::FirstLine, E::Force), Ltr, false, (false, true)),
            (mode(S::AllLines, E::Allow), Rtl, false, (true, true)),
        ];
        for (m, d, first, (left, right)) in cases {
            assert_eq!(
                m.allows(L, d, first),
                left,
                "{m:?} {d:?} first={first} left"
            );
            assert_eq!(
                m.allows(R, d, first),
                right,
                "{m:?} {d:?} first={first} right"
            );
        }
        assert_ne!(HangEndMode::Allow, HangEndMode::Force);
        assert_eq!(HangEndMode::default(), HangEndMode::None);
        assert_eq!(HangStartMode::default(), HangStartMode::None);
    }
}
