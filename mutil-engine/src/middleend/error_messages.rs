//! `Display` and `std::error::Error` for the middle-end error enums.
//!
//! Each table maps a variant (with the fields its message uses) to a format
//! string with inline captures, keeping the messages out of the domain modules
//! without a proc-macro dependency.

use super::{LayoutInputError, LineInvariantError};
use core::fmt;

macro_rules! error_messages {
    ($ty:ident { $($variant:ident $({ $($field:ident),* })? => $msg:literal,)* }) => {
        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $($ty::$variant $({ $($field,)* .. })? => write!(f, $msg),)*
                }
            }
        }

        impl std::error::Error for $ty {}
    };
}

error_messages! {
    LayoutInputError {
        OrientationMismatch => "orientation does not match writing mode or bidi level",
        InvalidFontMetrics => "invalid font metrics",
        InvalidSpacing => "invalid explicit spacing",
        InvalidBidiSpan { span } => "explicit bidi span {span} is invalid",
        InvalidAnnotationSpan { annotation } => "annotation {annotation} is invalid",
        InvalidLayoutOptions => "invalid layout options",
        InvalidLineGeometry => "invalid line geometry",
        NonMonotonicGeometry => "geometry grew with an expanded block envelope",
        InvalidConstraints => "invalid wrap width or tab stops",
        ParagraphSeparatorInText => "text contains a paragraph separator; split it first",
        InvalidStrut => "invalid strut metrics, font size or line height",
        RunOutsideText { run } => "run {run} does not fit the text",
        InvalidShapingRun { run } => "run {run} has an unresolved script or an invalid font size, bidi level or variation",
        ScriptMismatch { item } => "item {item} has an atom whose script differs from its run",
        InvalidRunStorage { run } => "run {run} has atom or cluster ranges that do not tile the paragraph storage",
        RunIndexOutOfRange { item } => "item {item} refers to a run that does not exist",
        ItemOutsideText { item } => "item {item} does not fit the text",
        ItemOutsideRun { item } => "item {item} lies outside the shaping run it names",
        InvalidTextWidth { item } => "item {item} has a width that is not its clusters' total",
        ItemNotSelfContained { item } => "item {item} does not partition its atoms",
        InvalidInlineBox { item } => "inline box {item} has a non-finite or negative size",
        InvalidGlue { item } => "glue {item} has a non-finite or negative size",
        InvalidDiscretionary { item } => "discretionary {item} has an invalid penalty or width",
        InvalidHyphenOpportunity { item } => "item {item} has a hyphenation opportunity off its atoms",
        InvalidSpanBoundary { item } => "span boundary {item} is not empty or has an invalid width",
        UnknownSpan { item } => "item {item} refers to a span that does not exist",
        InvalidDecorationSpan { span } => "decoration span {span} is invalid or duplicated",
        ItemsOutOfOrder { item } => "item {item} starts before the previous item ends",
        SpanEnclosure { span, item } => "item {item} is misplaced relative to the boundaries of decoration span {span}",
        MissingParagraphEnd => "the items do not end with the paragraph-end sentinel",
        MissingSpanBoundary { span } => "decoration span {span} lacks a leading or trailing boundary",
        AtomicFlagMismatch { item } => "item {item} has an atom whose atomic-span flag is wrong",
        InvalidAtomicSpan { span } => "atomic span {span} is invalid or duplicated",
    }
}

error_messages! {
    LineInvariantError {
        InvalidSlots => "invalid slot geometry or capacity",
        OrientationMismatch => "invalid visual orientation",
        InvalidGlyphSource { run, glyph } => "glyph {glyph} of run {run} has an invalid synthesized flag or source range",
        InvalidAnnotation { index } => "annotation {index} is invalid, has no glyphs, repeats an id, overlaps another on its side, or does not fit its base",
        InvalidParagraph => "invalid paragraph coverage or block geometry",
        NonFiniteOrNegativeExtent => "visual_width or a hanging extent is negative or not finite",
        NonFiniteLineMetrics => "baseline_y, ascent, descent or line_height is not finite",
        NonPositiveLineHeight { expected, actual } => "line height {actual} or strut height {expected} is not positive",
        InvalidVisualRun { run } => "visual run {run} has no glyphs, bytes outside the line, or an invalid font instance",
        InvalidAtomicFragment { index } => "atomic fragment {index} has invalid geometry or ranges",
        VisibleEndOutsideRange => "visible_byte_end lies outside byte_range",
        EmptyLineHasVisualRuns => "an empty line has visual runs",
        EmptyLineHasBoxes => "an empty line has inline boxes",
        EmptyLineHeightMismatch { expected, actual } => "empty line height {actual} differs from the strut height {expected}",
        EmptyLineHasExtent => "an empty line has a non-zero width or hanging extent",
        LineHeightMismatch { expected, actual } => "line height {actual} differs from the fixed strut height {expected}",
        LineHeightBelowStrut { minimum, actual } => "line height {actual} is below the strut height {minimum}",
        NonFiniteGlyph { run, glyph } => "glyph {glyph} of run {run} has a non-finite coordinate",
        TrailingWhitespaceNotCollapsed { run, glyph } => "trailing whitespace glyph {glyph} of run {run} has width in Collapsible mode",
        HangingGlyphInsideBlock { run, glyph } => "hanging glyph {glyph} of run {run} overlaps the text block",
        BodyGlyphOutsideBlock { run, glyph } => "body glyph {glyph} of run {run} lies outside the text block",
        HangingExtentMismatch { side, expected, actual } => "{side:?} hanging extent is {actual} but the glyphs occupy {expected}",
        InvalidInlineBox { index } => "inline box {index} has a non-finite coordinate, a negative size, an invalid baseline alignment or bytes outside the line",
        InlineBoxOutsideBlock { index } => "inline box {index} lies outside the text block",
        EmptyLineHasTabs => "an empty line has tabs",
        EmptyLineHasSpanFragments => "an empty line has span fragments",
        EmptyLineHasAtomicFragments => "an empty line has atomic fragments",
        InvalidSpanFragment { index } => "span fragment {index} has a non-finite coordinate or a negative size",
        SpanFragmentOutsideBlock { index } => "span fragment {index} lies outside the text block",
        SpanFragmentOutsideLine { index } => "span fragment {index} lies outside the line's bytes",
        GlyphOutsideSpanFragment { span_id, run, glyph } => "glyph {glyph} of run {run} lies outside every fragment of span {span_id}",
        InlineBoxOutsideSpanFragment { span_id, index } => "inline box {index} lies outside every fragment of span {span_id}",
        InlineBoxWiderThanWrapWidth { index } => "inline box {index} exceeds the wrap width",
        LineWiderThanWrapWidth { visual_width, wrap_width } => "line width {visual_width} exceeds the wrap width {wrap_width} and is not a single overflowing entity",
        InvalidTab { index } => "tab {index} has a non-finite coordinate, a negative width, or empty or out-of-line bytes",
        TabOutsideBlock { index } => "tab {index} lies outside the text block",
        TrailingTabNotCollapsed { index } => "trailing tab {index} has width in Collapsible mode",
        TrailingWhitespaceOnWrongSide { run, glyph } => "trailing whitespace glyph {glyph} of run {run} is not on the far edge of the base direction",
        TrailingTabOnWrongSide { index } => "trailing tab {index} is not on the far edge of the base direction",
        InvalidCaretStop { index } => "caret stop {index} is non-finite, out of visual order, outside the line or at an invalid level",
        InvalidDecoration { index } => "decoration {index} has a non-finite or negative extent or bytes outside the line",
        TrailingWhitespaceMarkedHanging { run, glyph } => "trailing whitespace glyph {glyph} of run {run} is flagged as hanging",
        HangingNotAllowed { side } => "{side:?} hanging punctuation is not allowed by the hang mode",
    }
}
