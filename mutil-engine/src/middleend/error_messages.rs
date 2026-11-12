//! `Display` and `std::error::Error` for the middle-end error enums.
//!
//! Each table maps a variant (with the fields its message uses) to a format
//! string with inline captures, keeping the messages out of the domain modules
//! without a proc-macro dependency. A message names every condition its
//! variant covers, so a caller is never pointed at the wrong field.

use super::{
    AnnotationErrorReason, LayoutInputError, LineInvariantError, ParagraphErrorReason,
    SlotErrorReason,
};
use core::fmt;

/// Implements `Display` from a table of variants and messages.
macro_rules! display_messages {
    ($ty:ident { $($variant:ident $({ $($field:ident),* })? => $msg:literal,)* }) => {
        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $($ty::$variant $({ $($field,)* .. })? => write!(f, $msg),)*
                }
            }
        }
    };
}

/// [`display_messages!`] for an error type: also implements
/// `std::error::Error`.
macro_rules! error_messages {
    ($ty:ident $table:tt) => {
        display_messages! { $ty $table }

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
        InvalidConstraints => "invalid wrap width, indentation or tab stops (without a geometry provider each indent must lie in [0, wrap width))",
        ParagraphSeparatorInText => "text contains a paragraph separator; split it first",
        InvalidStrut => "invalid strut metrics, font size or line height",
        RunOutsideText { run } => "run {run} does not fit the text",
        InvalidShapingRun { run } => "run {run} has an unresolved script, an invalid font size, bidi level or variation, or bytes that start before the previous run ends",
        InvalidRunStorage { run } => "run {run} has atom or cluster ranges that do not tile the paragraph storage, clusters that do not partition its atoms, or text boxes that do not cover its clusters",
        InvalidGlyphStorage { cluster } => "cluster {cluster} has a glyph range that does not tile the paragraph's glyphs",
        RunIndexOutOfRange { item } => "item {item} refers to a run that does not exist",
        ItemOutsideText { item } => "item {item} does not fit the text",
        UncoveredSource { byte } => "no item owns byte {byte} of the text",
        ItemOutsideRun { item } => "text box {item} lies outside the bytes of the shaping run it names",
        InvalidTextWidth { item } => "text box {item} has a width that is not its clusters' total, or a cluster with an invalid width, font scale or glyph measurement",
        ItemNotSelfContained { item } => "text box {item} has no clusters, does not resolve to contiguous atoms, has inconsistent break-safety flags, or has a cluster range outside or out of order in its run",
        InvalidInlineBox { item } => "inline object {item} has a non-finite or negative size, an invalid baseline alignment or bidi level, or no source bytes",
        InvalidGlue { item } => "glue {item} has an invalid size or bidi level, a compression pool that is not shrink only, or source bytes that do not match its kind",
        InvalidDiscretionary { item } => "discretionary {item} has an invalid penalty, width, byte range or cloned-span list, or widths that do not match its action",
        InvalidHyphenOpportunity { item } => "item {item} is a hyphenation break that is not flagged, does not join two text boxes of one run at its offset, or has a cut off the atoms of its hyphen chain",
        InvalidSpanBoundary { item } => "span boundary {item} has an invalid range, width, anchor, bidi level or nesting order",
        UnknownSpan { item } => "item {item} refers to a decoration span that does not exist, or clones one the break does not cut",
        InvalidDecorationSpan { span } => "decoration span {span} is invalid, duplicated, out of order or badly nested",
        ItemsOutOfOrder { item } => "item {item} starts before the previous item ends",
        SpanEnclosure { span, item } => "item {item} is misplaced relative to the boundaries of decoration span {span}",
        MissingParagraphEnd => "the items do not end with the paragraph-end sentinel",
        MissingSpanBoundary { span } => "decoration span {span} lacks a leading or trailing boundary",
        InvalidTab { item } => "tab {item} owns no source bytes",
        InvalidAtomicSpan { span } => "atomic span {span} is invalid, duplicated, out of order or overlapping",
    }
}

error_messages! {
    LineInvariantError {
        InvalidSlots { reason } => "invalid slots: {reason}",
        OrientationMismatch => "invalid visual orientation",
        InvalidGlyphSource { run, glyph } => "glyph {glyph} of run {run} has an invalid source range",
        InvalidAnnotation { index, reason } => "annotation {index} is invalid: {reason}",
        InvalidParagraph { reason } => "invalid paragraph: {reason}",
        NonFiniteLineMetrics => "baseline_y, ascent, descent or line_height is not finite",
        NonPositiveLineHeight { expected, actual } => "line height {actual} or strut height {expected} is not positive",
        InvalidVisualRun { run } => "visual run {run} has no glyphs, bytes outside the line, or an invalid font instance",
        GlyphsOutOfVisualOrder { run, glyph } => "glyph {glyph} of run {run} lies left of the glyph before it",
        InvalidAtomicFragment { index } => "atomic fragment {index} has invalid geometry or bytes, or the fragments of its span do not cover the span's part of the line exactly",
        VisibleEndOutsideRange => "visible_byte_end lies outside byte_range",
        VisibleEndNotOnBoundary => "visible_byte_end is not a line end or the start or end of a glyph, inline box or tab",
        EmptyLineHasVisualRuns => "an empty line has visual runs",
        EmptyLineHasBoxes => "an empty line has inline boxes",
        EmptyLineHeightMismatch { expected, actual } => "empty line height {actual} differs from the strut height {expected}",
        EmptyLineHasExtent => "an empty line's slots hold a non-zero width",
        LineHeightMismatch { expected, actual } => "line height {actual} differs from the fixed strut height {expected}",
        LineMetricsMismatch { expected_ascent, expected_descent, actual_ascent, actual_descent } => "ascent {actual_ascent} and descent {actual_descent} differ from the fixed strut's {expected_ascent} and {expected_descent}",
        LineHeightBelowStrut { minimum, actual } => "line height {actual} is below the strut height {minimum}",
        NonFiniteGlyph { run, glyph } => "glyph {glyph} of run {run} has a non-finite coordinate",
        TrailingWhitespaceNotCollapsed { run, glyph } => "trailing whitespace glyph {glyph} of run {run} has width in Collapsible mode",
        HangingGlyphInsideBlock { run, glyph } => "hanging glyph {glyph} of run {run} overlaps the text block",
        BodyGlyphOutsideBlock { run, glyph } => "body glyph {glyph} of run {run} lies outside the content of every slot",
        InvalidInlineBox { index } => "inline box {index} has a non-finite coordinate, a negative size, an invalid baseline alignment or bidi level, or bytes that are empty or outside the line",
        InlineBoxOutsideBlock { index } => "inline box {index} lies outside the content of every slot",
        EmptyLineHasTabs => "an empty line has tabs",
        EmptyLineHasSpanFragments => "an empty line has span fragments",
        EmptyLineHasAtomicFragments => "an empty line has atomic fragments",
        InvalidSpanFragment { index } => "span fragment {index} has a non-finite coordinate, a negative size or an unrepresentable bidi level",
        SpanFragmentOutsideBlock { index } => "span fragment {index} lies outside the content of every slot",
        SpanFragmentOutsideLine { index } => "span fragment {index} lies outside the line's bytes",
        SpanFragmentOutsideSpan { index } => "span fragment {index} covers bytes outside its span",
        GlyphOutsideSpanFragment { span_id, run, glyph } => "glyph {glyph} of run {run} lies outside every fragment of span {span_id}",
        InlineBoxOutsideSpanFragment { span_id, index } => "inline box {index} lies outside every fragment of span {span_id}",
        InlineBoxWiderThanWrapWidth { index } => "inline box {index} exceeds the wrap width",
        LineWiderThanWrapWidth { visual_width, wrap_width } => "line width {visual_width} exceeds the wrap width {wrap_width} and is not a single overflowing entity",
        SlotOverflow { slot, extent, capacity } => "the content of slot {slot} reaches {extent} into a capacity of {capacity} and is not a single overflowing entity",
        InvalidTab { index } => "tab {index} has a non-finite coordinate, a negative width, or empty or out-of-line bytes",
        TabOutsideBlock { index } => "tab {index} lies outside the content of every slot",
        TrailingTabNotCollapsed { index } => "trailing tab {index} has width in Collapsible mode",
        TrailingWhitespaceOnWrongSide { run, glyph } => "trailing whitespace glyph {glyph} of run {run} is not on the far edge of the base direction",
        TrailingTabOnWrongSide { index } => "trailing tab {index} is not on the far edge of the base direction",
        InvalidCaretStop { index } => "caret stop {index} is non-finite, out of visual order, outside the line, or at a bidi level that no run owning its offset has",
        InvalidDecoration { index } => "decoration {index} has a non-finite or negative extent or bytes outside the line",
        TrailingWhitespaceMarkedHanging { run, glyph } => "trailing whitespace glyph {glyph} of run {run} is flagged as hanging",
        HangingNotAllowed { side } => "{side} hanging punctuation is not allowed by the hang mode",
        HangingWithoutOverflow { side } => "{side} hanging punctuation would have fitted the line, which the allow-end mode does not hang",
    }
}

display_messages! {
    SlotErrorReason {
        InvalidWrapWidth => "the wrap width is not positive",
        NoSlots => "the line has no slot",
        InvalidConstraint => "a constraint is malformed, outside the wrap envelope or out of order",
        InvalidContent => "a content block is not finite, is negative or starts outside its constraint",
        WidthNotContentSpan => "a content block is not the span of the content placed in it",
        ContentOutsideSlots => "the line has content but no slot holds any of it",
    }
}

display_messages! {
    AnnotationErrorReason {
        OnEmptyLine => "the line has no source bytes",
        InvalidMetrics => "a size, scale, offset, extent or variation is invalid",
        OrientationMismatch => "the orientation does not match the writing mode",
        BaseOutsideLine => "the base is empty or outside the visible part of the line",
        NoGlyphs => "it has no glyphs",
        InvalidGlyph => "a glyph is not synthesized, has a byte range or is not finite",
        DuplicateId => "its id repeats on the line",
        OverlapsOnSide => "it shares base bytes with another annotation on its side",
        GlyphStraddlesBase => "a body glyph straddles an edge of the base",
        NoBaseGlyphs => "no body glyph lies in the base",
        BaseSpansSlots => "the base does not lie in one slot",
        OutsideOverhang => "a glyph lies beyond the overhang allowance or the slot",
        InsideBase => "it sits closer to the baseline than its base",
        DoesNotFitLineBox => "the line box is too small for the base and the annotation",
    }
}

display_messages! {
    ParagraphErrorReason {
        InvalidPolicy => "the recorded policy is invalid",
        NoLines => "it has no line",
        InvalidBlockSize => "the total block size is not finite, is negative or ends above the last line",
        LineBoxesOverlap => "a line box is not finite, is inverted or overlaps the previous line",
        LineBoxExceedsLineHeight => "a line's ascent plus descent exceeds its line height",
        NotStandardSlots => "a line's slots are not the standard column [indent, wrap width]",
        SourceNotCovered => "the lines do not tile the rendered text on character boundaries",
        InvalidTruncation => "the ellipsized line, the truncated range and the clamp policy disagree",
        DuplicateAnnotation => "an annotation id repeats across lines",
        InconsistentSpanFragments => "the fragments of a decoration span disagree about its range or do not hold exactly one first and one last fragment",
        InconsistentAtomicFragments => "the fragments of an atomic span disagree about its range or do not cover its rendered part",
    }
}
