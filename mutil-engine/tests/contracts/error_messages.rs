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
            "invalid wrap width or indentation (without a geometry provider each indent must lie in [0, wrap width))",
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
            "run 1 has an unresolved script, an invalid font size, or bytes that start before the previous run ends",
        ),
        (
            LayoutInputError::InvalidRunStorage { run: 1 },
            "run 1 has atom or cluster ranges that do not tile the paragraph storage, clusters that do not partition its atoms, or text boxes that do not cover its clusters",
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
            "text box 2 lies outside the bytes of the shaping run it names",
        ),
        (
            LayoutInputError::InvalidTextWidth { item: 2 },
            "text box 2 has a width that is not its clusters' total, or a cluster with an invalid width, font scale or glyph measurement",
        ),
        (
            LayoutInputError::ItemNotSelfContained { item: 2 },
            "text box 2 has no clusters, does not resolve to contiguous atoms, has inconsistent break-safety flags, or has a cluster range outside or out of order in its run",
        ),
        (
            LayoutInputError::InvalidInlineBox { item: 2 },
            "inline object 2 has a non-finite or negative size, an invalid baseline alignment, or no source bytes",
        ),
        (
            LayoutInputError::InvalidGlue { item: 2 },
            "glue 2 has an invalid size, a compression pool that is not shrink only, or source bytes that do not match its kind",
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
            "glyph 5 of run 1 has an invalid source range",
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
            "an empty line's slots hold a non-zero width",
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
            "span fragment 4 has a non-finite coordinate or a negative size",
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
            "caret stop 4 is non-finite, out of visual order, outside the line, or at a bidi level that nothing owning its offset has",
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

#[test]
fn storage_coverage_and_ordering_error_messages() {
    assert_eq!(
        LayoutInputError::InvalidGlyphStorage { cluster: 3 }.to_string(),
        "cluster 3 has a glyph range that does not tile the paragraph's glyphs"
    );
    assert_eq!(
        LayoutInputError::UncoveredSource { byte: 7 }.to_string(),
        "no item owns byte 7 of the text"
    );
    for (error, expected) in [
        (
            LineInvariantError::GlyphsOutOfVisualOrder { run: 1, glyph: 5 },
            "glyph 5 of run 1 lies left of the glyph before it",
        ),
        (
            LineInvariantError::SpanFragmentOutsideSpan { index: 4 },
            "span fragment 4 covers bytes outside its span",
        ),
        (
            LineInvariantError::HangingWithoutOverflow {
                side: HangSide::Right,
            },
            "right hanging punctuation would have fitted the line, which the allow-end mode does not hang",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn every_reason_completes_its_error_message() {
    use AnnotationErrorReason as A;
    use ParagraphErrorReason as P;
    use SlotErrorReason as S;
    let slots = [
        (S::InvalidWrapWidth, "the wrap width is not positive"),
        (S::NoSlots, "the line has no slot"),
        (
            S::InvalidConstraint,
            "a constraint lies outside the wrap envelope or out of order",
        ),
        (
            S::InvalidContent,
            "a content block is not finite, is negative or starts outside its constraint",
        ),
        (
            S::WidthNotContentSpan,
            "a content block is not the span of the content placed in it",
        ),
        (
            S::ContentOutsideSlots,
            "the line has content but no slot holds any of it",
        ),
    ];
    for (reason, expected) in slots {
        assert_eq!(
            LineInvariantError::InvalidSlots { reason }.to_string(),
            format!("invalid slots: {expected}")
        );
    }
    let annotations = [
        (A::OnEmptyLine, "the line has no source bytes"),
        (
            A::InvalidMetrics,
            "a size, scale, offset or extent is invalid",
        ),
        (
            A::OrientationMismatch,
            "the orientation does not match the writing mode",
        ),
        (
            A::BaseOutsideLine,
            "the base is empty or outside the visible part of the line",
        ),
        (A::NoGlyphs, "it has no glyphs"),
        (
            A::InvalidGlyph,
            "a glyph is not synthesized, has a byte range or is not finite",
        ),
        (A::DuplicateId, "its id repeats on the line"),
        (
            A::OverlapsOnSide,
            "it shares base bytes with another annotation on its side",
        ),
        (
            A::GlyphStraddlesBase,
            "a body glyph straddles an edge of the base",
        ),
        (A::NoBaseGlyphs, "no body glyph lies in the base"),
        (A::BaseSpansSlots, "the base does not lie in one slot"),
        (
            A::OutsideOverhang,
            "a glyph lies beyond the overhang allowance or the slot",
        ),
        (
            A::InsideBase,
            "it sits closer to the baseline than its base",
        ),
        (
            A::DoesNotFitLineBox,
            "the line box is too small for the base and the annotation",
        ),
    ];
    for (reason, expected) in annotations {
        assert_eq!(
            LineInvariantError::InvalidAnnotation { index: 2, reason }.to_string(),
            format!("annotation 2 is invalid: {expected}")
        );
    }
    let paragraphs = [
        (P::InvalidPolicy, "the recorded policy is invalid"),
        (P::NoLines, "it has no line"),
        (
            P::InvalidBlockSize,
            "the total block size is not finite, is negative or ends above the last line",
        ),
        (
            P::LineBoxesOverlap,
            "a line box is not finite, is inverted or overlaps the previous line",
        ),
        (
            P::LineBoxExceedsLineHeight,
            "a line's ascent plus descent exceeds its line height",
        ),
        (
            P::NotStandardSlots,
            "a line's slots are not the standard column [indent, wrap width]",
        ),
        (
            P::SourceNotCovered,
            "the lines do not tile the rendered text on character boundaries",
        ),
        (
            P::InvalidTruncation,
            "the ellipsized line, the truncated range and the clamp policy disagree",
        ),
        (
            P::DuplicateAnnotation,
            "an annotation id repeats across lines",
        ),
        (
            P::InconsistentSpanFragments,
            "the fragments of a decoration span disagree about its range or do not hold exactly one first and one last fragment",
        ),
        (
            P::InconsistentAtomicFragments,
            "the fragments of an atomic span disagree about its range or do not cover its rendered part",
        ),
    ];
    for (reason, expected) in paragraphs {
        assert_eq!(
            LineInvariantError::InvalidParagraph { reason }.to_string(),
            format!("invalid paragraph: {expected}")
        );
    }
}
