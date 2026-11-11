//! Every error variant's `Display` message, pinned verbatim.

use mutil_engine::*;

#[test]
fn layout_input_error_messages() {
    for (error, expected) in [
        (
            LayoutInputError::OrientationMismatch,
            "orientation does not match writing mode or bidi level",
        ),
        (LayoutInputError::InvalidFontMetrics, "invalid font metrics"),
        (LayoutInputError::InvalidSpacing, "invalid explicit spacing"),
        (
            LayoutInputError::InvalidBidiSpan { span: 3 },
            "explicit bidi span 3 is invalid",
        ),
        (
            LayoutInputError::InvalidAnnotationSpan { annotation: 2 },
            "annotation 2 is invalid",
        ),
        (
            LayoutInputError::ScriptMismatch { item: 4 },
            "item 4 has an atom whose script differs from its run",
        ),
        (
            LayoutInputError::InvalidLayoutOptions,
            "invalid layout options",
        ),
        (
            LayoutInputError::InvalidLineGeometry,
            "invalid line geometry",
        ),
        (
            LayoutInputError::NonMonotonicGeometry,
            "geometry grew with an expanded block envelope",
        ),
        (
            LayoutInputError::InvalidConstraints,
            "invalid wrap width, indentation or tab stops (without a geometry provider each indent must lie in [0, wrap width))",
        ),
        (
            LayoutInputError::ParagraphSeparatorInText,
            "text contains a paragraph separator; split it first",
        ),
        (
            LayoutInputError::InvalidStrut,
            "invalid strut metrics, font size or line height",
        ),
        (
            LayoutInputError::RunOutsideText { run: 1 },
            "run 1 does not fit the text",
        ),
        (
            LayoutInputError::InvalidShapingRun { run: 1 },
            "run 1 has an unresolved script, an invalid font size, bidi level or variation, or bytes that start before the previous run ends",
        ),
        (
            LayoutInputError::InvalidRunStorage { run: 1 },
            "run 1 has atom or cluster ranges that do not tile the paragraph storage, or its text boxes do not cover them",
        ),
        (
            LayoutInputError::RunIndexOutOfRange { item: 2 },
            "item 2 refers to a run that does not exist",
        ),
        (
            LayoutInputError::ItemOutsideText { item: 2 },
            "item 2 does not fit the text",
        ),
        (
            LayoutInputError::ItemOutsideRun { item: 2 },
            "text box 2 lies outside the bytes of the shaping run it names, or has an atom or cluster at another bidi level",
        ),
        (
            LayoutInputError::InvalidTextWidth { item: 2 },
            "text box 2 has a width that is not its clusters' total, or a cluster with an invalid width, font scale or glyph measurement",
        ),
        (
            LayoutInputError::ItemNotSelfContained { item: 2 },
            "text box 2 has no atoms, does not partition them into clusters, has inconsistent break-safety flags, or has ranges outside or out of order in its run",
        ),
        (
            LayoutInputError::InvalidInlineBox { item: 2 },
            "inline object 2 has a non-finite or negative size, an invalid baseline alignment, or no source bytes",
        ),
        (
            LayoutInputError::InvalidGlue { item: 2 },
            "glue 2 has an invalid size or bidi level, a compression pool that is not shrink only, or source bytes that do not match its kind",
        ),
        (
            LayoutInputError::InvalidDiscretionary { item: 2 },
            "discretionary 2 has an invalid penalty, width, byte range or cloned-span list, or widths that do not match its action",
        ),
        (
            LayoutInputError::InvalidHyphenOpportunity { item: 2 },
            "item 2 is a hyphenation break that is not flagged, does not join two text boxes of one run at its offset, or has a cut off the atoms of its hyphen chain",
        ),
        (
            LayoutInputError::InvalidSpanBoundary { item: 2 },
            "span boundary 2 has an invalid range, width, anchor, bidi level or nesting order",
        ),
        (
            LayoutInputError::UnknownSpan { item: 2 },
            "item 2 refers to a decoration span that does not exist, or clones one the break does not cut",
        ),
        (
            LayoutInputError::InvalidDecorationSpan { span: 3 },
            "decoration span 3 is invalid, duplicated, out of order or badly nested",
        ),
        (
            LayoutInputError::ItemsOutOfOrder { item: 2 },
            "item 2 starts before the previous item ends",
        ),
        (
            LayoutInputError::SpanEnclosure { span: 3, item: 2 },
            "item 2 is misplaced relative to the boundaries of decoration span 3",
        ),
        (
            LayoutInputError::MissingParagraphEnd,
            "the items do not end with the paragraph-end sentinel",
        ),
        (
            LayoutInputError::MissingSpanBoundary { span: 3 },
            "decoration span 3 lacks a leading or trailing boundary",
        ),
        (
            LayoutInputError::AtomicFlagMismatch { item: 2 },
            "item 2 has an atom whose atomic-span flag is wrong",
        ),
        (
            LayoutInputError::InvalidTab { item: 2 },
            "tab 2 owns no source bytes",
        ),
        (
            LayoutInputError::InvalidAtomicSpan { span: 3 },
            "atomic span 3 is invalid, duplicated, out of order or overlapping",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
        let _: &dyn std::error::Error = &error;
    }
}

#[test]
fn line_invariant_error_messages() {
    for (error, expected) in [
        (
            LineInvariantError::InvalidSlots {
                reason: SlotErrorReason::NoSlots,
            },
            "invalid slots: the line has no slot",
        ),
        (
            LineInvariantError::OrientationMismatch,
            "invalid visual orientation",
        ),
        (
            LineInvariantError::InvalidGlyphSource { run: 1, glyph: 5 },
            "glyph 5 of run 1 has an invalid synthesized flag or source range",
        ),
        (
            LineInvariantError::InvalidAnnotation {
                index: 4,
                reason: AnnotationErrorReason::NoGlyphs,
            },
            "annotation 4 is invalid: it has no glyphs",
        ),
        (
            LineInvariantError::InvalidParagraph {
                reason: ParagraphErrorReason::NoLines,
            },
            "invalid paragraph: it has no line",
        ),
        (
            LineInvariantError::NonFiniteOrNegativeExtent,
            "visual_width or a hanging extent is negative or not finite",
        ),
        (
            LineInvariantError::NonFiniteLineMetrics,
            "baseline_y, ascent, descent or line_height is not finite",
        ),
        (
            LineInvariantError::NonPositiveLineHeight {
                expected: 1.5,
                actual: 2.5,
            },
            "line height 2.5 or strut height 1.5 is not positive",
        ),
        (
            LineInvariantError::InvalidVisualRun { run: 1 },
            "visual run 1 has no glyphs, bytes outside the line, or an invalid font instance",
        ),
        (
            LineInvariantError::InvalidAtomicFragment { index: 4 },
            "atomic fragment 4 has invalid geometry or bytes, or the fragments of its span do not cover the span's part of the line exactly",
        ),
        (
            LineInvariantError::VisibleEndOutsideRange,
            "visible_byte_end lies outside byte_range",
        ),
        (
            LineInvariantError::VisibleEndNotOnBoundary,
            "visible_byte_end is not a line end or the start or end of a glyph, inline box or tab",
        ),
        (
            LineInvariantError::EmptyLineHasVisualRuns,
            "an empty line has visual runs",
        ),
        (
            LineInvariantError::EmptyLineHasBoxes,
            "an empty line has inline boxes",
        ),
        (
            LineInvariantError::EmptyLineHeightMismatch {
                expected: 1.5,
                actual: 2.5,
            },
            "empty line height 2.5 differs from the strut height 1.5",
        ),
        (
            LineInvariantError::EmptyLineHasExtent,
            "an empty line has a non-zero width or hanging extent",
        ),
        (
            LineInvariantError::LineHeightMismatch {
                expected: 1.5,
                actual: 2.5,
            },
            "line height 2.5 differs from the fixed strut height 1.5",
        ),
        (
            LineInvariantError::LineMetricsMismatch {
                expected_ascent: 1.5,
                expected_descent: 0.5,
                actual_ascent: 2.5,
                actual_descent: 3.5,
            },
            "ascent 2.5 and descent 3.5 differ from the fixed strut's 1.5 and 0.5",
        ),
        (
            LineInvariantError::LineHeightBelowStrut {
                minimum: 3.5,
                actual: 2.5,
            },
            "line height 2.5 is below the strut height 3.5",
        ),
        (
            LineInvariantError::NonFiniteGlyph { run: 1, glyph: 5 },
            "glyph 5 of run 1 has a non-finite coordinate",
        ),
        (
            LineInvariantError::TrailingWhitespaceNotCollapsed { run: 1, glyph: 5 },
            "trailing whitespace glyph 5 of run 1 has width in Collapsible mode",
        ),
        (
            LineInvariantError::HangingGlyphInsideBlock { run: 1, glyph: 5 },
            "hanging glyph 5 of run 1 overlaps the text block",
        ),
        (
            LineInvariantError::BodyGlyphOutsideBlock { run: 1, glyph: 5 },
            "body glyph 5 of run 1 lies outside the content of every slot",
        ),
        (
            LineInvariantError::HangingExtentMismatch {
                side: HangSide::Left,
                expected: 1.5,
                actual: 2.5,
            },
            "left hanging extent is 2.5 but the glyphs occupy 1.5",
        ),
        (
            LineInvariantError::InvalidInlineBox { index: 4 },
            "inline box 4 has a non-finite coordinate, a negative size, an invalid baseline alignment, or bytes that are empty or outside the line",
        ),
        (
            LineInvariantError::InlineBoxOutsideBlock { index: 4 },
            "inline box 4 lies outside the content of every slot",
        ),
        (
            LineInvariantError::EmptyLineHasTabs,
            "an empty line has tabs",
        ),
        (
            LineInvariantError::EmptyLineHasSpanFragments,
            "an empty line has span fragments",
        ),
        (
            LineInvariantError::EmptyLineHasAtomicFragments,
            "an empty line has atomic fragments",
        ),
        (
            LineInvariantError::InvalidSpanFragment { index: 4 },
            "span fragment 4 has a non-finite coordinate, a negative size or an unrepresentable bidi level",
        ),
        (
            LineInvariantError::SpanFragmentOutsideBlock { index: 4 },
            "span fragment 4 lies outside the content of every slot",
        ),
        (
            LineInvariantError::SpanFragmentOutsideLine { index: 4 },
            "span fragment 4 lies outside the line's bytes",
        ),
        (
            LineInvariantError::GlyphOutsideSpanFragment {
                span_id: 6,
                run: 1,
                glyph: 5,
            },
            "glyph 5 of run 1 lies outside every fragment of span 6",
        ),
        (
            LineInvariantError::InlineBoxOutsideSpanFragment {
                span_id: 6,
                index: 4,
            },
            "inline box 4 lies outside every fragment of span 6",
        ),
        (
            LineInvariantError::InlineBoxWiderThanWrapWidth { index: 4 },
            "inline box 4 exceeds the wrap width",
        ),
        (
            LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 4.5,
                wrap_width: 5.5,
            },
            "line width 4.5 exceeds the wrap width 5.5 and is not a single overflowing entity",
        ),
        (
            LineInvariantError::SlotOverflow {
                slot: 1,
                extent: 4.5,
                capacity: 3.5,
            },
            "the content of slot 1 reaches 4.5 into a capacity of 3.5 and is not a single overflowing entity",
        ),
        (
            LineInvariantError::InvalidTab { index: 4 },
            "tab 4 has a non-finite coordinate, a negative width, or empty or out-of-line bytes",
        ),
        (
            LineInvariantError::TabOutsideBlock { index: 4 },
            "tab 4 lies outside the content of every slot",
        ),
        (
            LineInvariantError::TrailingTabNotCollapsed { index: 4 },
            "trailing tab 4 has width in Collapsible mode",
        ),
        (
            LineInvariantError::TrailingWhitespaceOnWrongSide { run: 1, glyph: 5 },
            "trailing whitespace glyph 5 of run 1 is not on the far edge of the base direction",
        ),
        (
            LineInvariantError::TrailingTabOnWrongSide { index: 4 },
            "trailing tab 4 is not on the far edge of the base direction",
        ),
        (
            LineInvariantError::InvalidCaretStop { index: 4 },
            "caret stop 4 is non-finite, out of visual order, outside the line, or at a bidi level that no run owning its offset has",
        ),
        (
            LineInvariantError::InvalidDecoration { index: 4 },
            "decoration 4 has a non-finite or negative extent or bytes outside the line",
        ),
        (
            LineInvariantError::TrailingWhitespaceMarkedHanging { run: 1, glyph: 5 },
            "trailing whitespace glyph 5 of run 1 is flagged as hanging",
        ),
        (
            LineInvariantError::HangingNotAllowed {
                side: HangSide::Left,
            },
            "left hanging punctuation is not allowed by the hang mode",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
        let _: &dyn std::error::Error = &error;
    }
}
