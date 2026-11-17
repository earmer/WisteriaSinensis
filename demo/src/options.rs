//! The engine options the page sets, and how it encodes them as integers
//! across the C ABI.

use mutil_engine::{
    AutospaceProfile, BidiDirection, BoxHeightStyle, CaretAffinity, CursorPosition, HangEndMode,
    HangMode, HangStartMode, HangingWhitespaceMode, LayoutConstraints, LineBreakStrictness, Motion,
    WordBreakOption,
};

/// The options set once by `demo_set_options` and applied to every
/// paragraph laid out afterwards.
#[derive(Clone, Copy, Default)]
pub struct ExtraOptions {
    pub line_break: LineBreakStrictness,
    pub word_break: WordBreakOption,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub first_line_indent: f32,
    pub hanging_indent: f32,
}

impl ExtraOptions {
    /// The constraints of a paragraph wrapped at `wrap_width` with these
    /// indents.
    pub fn constraints(&self, wrap_width: f32) -> LayoutConstraints {
        LayoutConstraints {
            wrap_width,
            first_line_indent: self.first_line_indent,
            hanging_indent: self.hanging_indent,
            ..LayoutConstraints::default()
        }
    }
}

/// The Phase 2 options the page sets, packed into one `u32` across the C
/// ABI: bits 0-1 the base direction (0 from the text, 1 LTR, 2 RTL), bit 2
/// `justify`, bits 3-4 the start hang (0 none, 1 first line, 2 all lines),
/// bits 5-6 the end hang (0 none, 1 allow, 2 force), bit 7 the `CssCompat`
/// autospace profile, bit 8 preserved trailing whitespace and bit 9
/// `compress_last_lines`.
#[derive(Clone, Copy, Default)]
pub struct Phase2 {
    pub base_direction: Option<BidiDirection>,
    pub justify: bool,
    pub compress_last_lines: bool,
    pub hang_mode: HangMode,
    pub autospace: AutospaceProfile,
    pub whitespace: HangingWhitespaceMode,
}

impl Phase2 {
    pub fn from_bits(bits: u32) -> Self {
        let field = |shift: u32| (bits >> shift) & 0b11;
        let flag = |bit: u32| bits & (1 << bit) != 0;
        Self {
            base_direction: match field(0) {
                1 => Some(BidiDirection::Ltr),
                2 => Some(BidiDirection::Rtl),
                _ => None,
            },
            justify: flag(2),
            compress_last_lines: flag(9),
            hang_mode: HangMode {
                start: match field(3) {
                    1 => HangStartMode::FirstLine,
                    2 => HangStartMode::AllLines,
                    _ => HangStartMode::None,
                },
                end: match field(5) {
                    1 => HangEndMode::Allow,
                    2 => HangEndMode::Force,
                    _ => HangEndMode::None,
                },
            },
            autospace: if flag(7) {
                AutospaceProfile::CssCompat
            } else {
                AutospaceProfile::Jlreq
            },
            whitespace: if flag(8) {
                HangingWhitespaceMode::Preserved
            } else {
                HangingWhitespaceMode::Collapsible
            },
        }
    }
}

pub fn line_break(code: u32) -> LineBreakStrictness {
    match code {
        1 => LineBreakStrictness::Loose,
        2 => LineBreakStrictness::Strict,
        3 => LineBreakStrictness::Anywhere,
        _ => LineBreakStrictness::Normal,
    }
}

pub fn word_break(code: u32) -> WordBreakOption {
    match code {
        1 => WordBreakOption::BreakAll,
        2 => WordBreakOption::KeepAll,
        _ => WordBreakOption::Normal,
    }
}

/// A caret at `byte_offset`; a non-zero `affinity` is upstream.
pub fn cursor(byte_offset: u32, affinity: u32) -> CursorPosition {
    CursorPosition {
        byte_offset: byte_offset as usize,
        affinity: if affinity != 0 {
            CaretAffinity::Upstream
        } else {
            CaretAffinity::Downstream
        },
    }
}

pub fn motion(code: u32) -> Motion {
    match code {
        1 => Motion::Right,
        2 => Motion::Up,
        3 => Motion::Down,
        4 => Motion::LineStart,
        5 => Motion::LineEnd,
        6 => Motion::WordLeft,
        7 => Motion::WordRight,
        8 => Motion::Previous,
        9 => Motion::Next,
        _ => Motion::Left,
    }
}

pub fn box_height(code: u32) -> BoxHeightStyle {
    match code {
        1 => BoxHeightStyle::Max,
        2 => BoxHeightStyle::Strut,
        _ => BoxHeightStyle::Tight,
    }
}
