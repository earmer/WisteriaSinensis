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
            "invalid wrap width or tab stops",
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
            "run 1 has an unresolved script or an invalid font size, bidi level or variation",
        ),
        (
            LayoutInputError::InvalidRunStorage { run: 1 },
            "run 1 has atom or cluster ranges that do not tile the paragraph storage",
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
            "item 2 lies outside the shaping run it names",
        ),
        (
            LayoutInputError::InvalidTextWidth { item: 2 },
            "item 2 has a width that is not its clusters' total",
        ),
        (
            LayoutInputError::ItemNotSelfContained { item: 2 },
            "item 2 does not partition its atoms",
        ),
        (
            LayoutInputError::InvalidInlineBox { item: 2 },
            "inline box 2 has a non-finite or negative size",
        ),
        (
            LayoutInputError::InvalidGlue { item: 2 },
            "glue 2 has a non-finite or negative size",
        ),
        (
            LayoutInputError::InvalidDiscretionary { item: 2 },
            "discretionary 2 has an invalid penalty or width",
        ),
        (
            LayoutInputError::InvalidHyphenOpportunity { item: 2 },
            "item 2 has a hyphenation opportunity off its atoms",
        ),
        (
            LayoutInputError::InvalidSpanBoundary { item: 2 },
            "span boundary 2 is not empty or has an invalid width",
        ),
        (
            LayoutInputError::UnknownSpan { item: 2 },
            "item 2 refers to a span that does not exist",
        ),
        (
            LayoutInputError::InvalidDecorationSpan { span: 3 },
            "decoration span 3 is invalid or duplicated",
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
            LayoutInputError::InvalidAtomicSpan { span: 3 },
            "atomic span 3 is invalid or duplicated",
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
            LineInvariantError::InvalidSlots,
            "invalid slot geometry or capacity",
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
            LineInvariantError::InvalidAnnotation { index: 4 },
            "annotation 4 is invalid, has no glyphs, repeats an id, overlaps another on its side, has an invalid base extent, sits closer to the baseline than its base, or does not fit the line box",
        ),
        (
            LineInvariantError::InvalidParagraph,
            "invalid paragraph coverage or block geometry",
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
            "atomic fragment 4 has invalid geometry or ranges",
        ),
        (
            LineInvariantError::VisibleEndOutsideRange,
            "visible_byte_end lies outside byte_range",
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
            "body glyph 5 of run 1 lies outside the text block",
        ),
        (
            LineInvariantError::HangingExtentMismatch {
                side: HangSide::Left,
                expected: 1.5,
                actual: 2.5,
            },
            "Left hanging extent is 2.5 but the glyphs occupy 1.5",
        ),
        (
            LineInvariantError::InvalidInlineBox { index: 4 },
            "inline box 4 has a non-finite coordinate, a negative size, an invalid baseline alignment or bytes outside the line",
        ),
        (
            LineInvariantError::InlineBoxOutsideBlock { index: 4 },
            "inline box 4 lies outside the text block",
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
            "span fragment 4 has a non-finite coordinate or a negative size",
        ),
        (
            LineInvariantError::SpanFragmentOutsideBlock { index: 4 },
            "span fragment 4 lies outside the text block",
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
            LineInvariantError::InvalidTab { index: 4 },
            "tab 4 has a non-finite coordinate, a negative width, or empty or out-of-line bytes",
        ),
        (
            LineInvariantError::TabOutsideBlock { index: 4 },
            "tab 4 lies outside the text block",
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
            "caret stop 4 is non-finite, out of visual order, outside the line or at an invalid level",
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
            "Left hanging punctuation is not allowed by the hang mode",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
        let _: &dyn std::error::Error = &error;
    }
}
