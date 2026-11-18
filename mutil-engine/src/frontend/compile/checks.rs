//! The input checks of `compile_paragraph` (spec FE-25, FE-27, FE-28).

use super::units::{Unit, Units};
use super::{Compiler, FrontendError, FrontendFeature};
use crate::frontend::{
    FrontendOptions, InlineObjectInput, MAX_BIDI_SPAN_DEPTH, MAX_DECORATION_NESTING_DEPTH,
    ParagraphInput, TextStyle, is_paragraph_separator,
};
use crate::types::{TextRange, is_extent, is_measurement};
use smallvec::SmallVec;

pub(super) fn unsupported(feature: FrontendFeature) -> FrontendError {
    FrontendError::Unsupported { feature }
}

/// The checks of FE-28 that need no segmentation.
pub(super) fn check_input(
    input: &ParagraphInput<'_>,
    options: &FrontendOptions,
) -> Result<(), FrontendError> {
    let text = input.text;
    if u32::try_from(text.len()).is_err() {
        return Err(FrontendError::TextTooLong);
    }
    if options.writing_mode.is_vertical() {
        return Err(unsupported(FrontendFeature::VerticalWritingMode));
    }
    check_decoration_spans(input)?;
    if !input.annotations.is_empty() {
        return Err(unsupported(FrontendFeature::Annotations));
    }
    check_style(input.default_style, None)?;
    let mut previous_end = 0;
    for (index, span) in input.style_spans.iter().enumerate() {
        let r = span.byte_range;
        let well_formed = r.start() < r.end()
            && r.end() <= text.len()
            && text.is_char_boundary(r.start())
            && text.is_char_boundary(r.end())
            && r.start() >= previous_end;
        if !well_formed {
            return Err(FrontendError::InvalidStyleSpan { span: index });
        }
        previous_end = r.end();
        check_style(&span.style, Some(index))?;
    }
    let mut previous_end = 0;
    for (index, object) in input.inline_objects.iter().enumerate() {
        let r = object.byte_range;
        let sizes = [object.width, object.ascent, object.descent];
        let well_formed = r.start() < r.end()
            && r.end() <= text.len()
            && r.start() >= previous_end
            && sizes.into_iter().all(is_extent)
            && object.baseline_align.is_valid();
        if !well_formed {
            return Err(FrontendError::InvalidInlineObject { object: index });
        }
        previous_end = r.end();
    }
    if let Some((byte, _)) = text
        .char_indices()
        .find(|&(_, c)| is_paragraph_separator(c))
    {
        return Err(FrontendError::ParagraphSeparatorInText { byte });
    }
    check_bidi_spans(input)
}

/// BID-2 without segmentation: unique ids, non-empty ranges on character
/// boundaries inside the text, ascending by start and, for equal starts,
/// descending by end with equal ranges rejected, proper nesting and the
/// depth bound. Grapheme and object edges are checked once units exist.
fn check_bidi_spans(input: &ParagraphInput<'_>) -> Result<(), FrontendError> {
    let text = input.text;
    let mut ids = std::collections::HashSet::new();
    let mut open: Vec<TextRange> = Vec::new();
    for (index, span) in input.bidi_spans.iter().enumerate() {
        let r = span.byte_range;
        let is_ordered = index.checked_sub(1).is_none_or(|p| {
            let prev = input.bidi_spans[p].byte_range;
            r.start() > prev.start() || (r.start() == prev.start() && r.end() < prev.end())
        });
        while open.last().is_some_and(|outer| outer.end() <= r.start()) {
            open.pop();
        }
        if !(ids.insert(span.span_id)
            && !r.is_empty()
            && r.end() <= text.len()
            && text.is_char_boundary(r.start())
            && text.is_char_boundary(r.end())
            && is_ordered
            && open.last().is_none_or(|outer| r.end() <= outer.end())
            && open.len() < MAX_BIDI_SPAN_DEPTH)
        {
            return Err(FrontendError::InvalidBidiSpan { span: index });
        }
        open.push(r);
    }
    Ok(())
}

/// AST-7 without segmentation: order, nesting, ids, paddings and ranges on
/// character boundaries. Grapheme and object edges are checked once units
/// exist.
fn check_decoration_spans(input: &ParagraphInput<'_>) -> Result<(), FrontendError> {
    let text = input.text;
    let mut ids = std::collections::HashSet::new();
    let mut stack: SmallVec<[TextRange; MAX_DECORATION_NESTING_DEPTH]> = SmallVec::new();
    for (index, span) in input.decoration_spans.iter().enumerate() {
        let r = span.byte_range;
        let invalid = Err(FrontendError::InvalidDecorationSpan { span: index });
        if index > 0 {
            let prev = input.decoration_spans[index - 1].byte_range;
            if r.start() < prev.start() || (r.start() == prev.start() && r.end() > prev.end()) {
                return invalid;
            }
        }
        while stack.last().is_some_and(|top| top.end() <= r.start()) {
            stack.pop();
        }
        if !ids.insert(span.span_id)
            || !span.is_valid()
            || r.end() > text.len()
            || !text.is_char_boundary(r.start())
            || !text.is_char_boundary(r.end())
            || stack.last().is_some_and(|top| !top.contains_range(r))
            || stack.len() >= MAX_DECORATION_NESTING_DEPTH
        {
            return invalid;
        }
        stack.push(r);
    }
    let mut ids = std::collections::HashSet::new();
    for (index, span) in input.atomic_spans.iter().enumerate() {
        let r = span.byte_range;
        let ordered = index == 0 || input.atomic_spans[index - 1].byte_range.end() <= r.start();
        if !ids.insert(span.span_id) || r.is_empty() || r.end() > text.len() || !ordered {
            return Err(FrontendError::InvalidAtomicSpan { span: index });
        }
    }
    Ok(())
}

fn check_style(style: &TextStyle, span: Option<usize>) -> Result<(), FrontendError> {
    let valid = is_extent(style.font_size)
        && style.font_size > 0.0
        && is_measurement(style.letter_spacing)
        && is_measurement(style.word_spacing)
        && style.baseline_align.is_valid()
        && style.script.is_none_or(|script| !script.is_unresolved());
    if !valid {
        return Err(FrontendError::InvalidStyle { span });
    }
    Ok(())
}

impl Units {
    /// The unit that starts at `offset`, if one does.
    fn starting_at(&self, offset: usize, objects: &[InlineObjectInput]) -> Option<Unit> {
        let at = self
            .units
            .binary_search_by_key(&offset, |u| u.range(&self.atoms, objects).start())
            .ok()?;
        Some(self.units[at])
    }

    /// The unit that ends at `offset`, if one does.
    fn ending_at(&self, offset: usize, objects: &[InlineObjectInput]) -> Option<Unit> {
        let at = self
            .units
            .binary_search_by_key(&offset, |u| u.range(&self.atoms, objects).end())
            .ok()?;
        Some(self.units[at])
    }
}

impl Compiler<'_> {
    /// Whether `offset` lies between units, so on a grapheme boundary and
    /// outside every inline object.
    fn is_unit_edge(&self, units: &Units, offset: usize) -> bool {
        offset == self.text.len()
            || units
                .starting_at(offset, self.input.inline_objects)
                .is_some()
    }

    /// The edge checks of AST-7, AST-9 and BID-2 that need units: a
    /// decoration or bidi edge lies between units, an atomic edge at the
    /// start or end of a text atom or object, and an atomic span holds one.
    pub(super) fn check_span_edges(&self, units: &Units) -> Result<(), FrontendError> {
        let input = self.input;
        let objects = input.inline_objects;
        let off_edge = |r: TextRange| {
            !self.is_unit_edge(units, r.start()) || !self.is_unit_edge(units, r.end())
        };
        if let Some(span) = (input.decoration_spans.iter()).position(|s| off_edge(s.byte_range)) {
            return Err(FrontendError::InvalidDecorationSpan { span });
        }
        let is_text_or_object =
            |unit: Option<Unit>| matches!(unit, Some(Unit::Atom(_) | Unit::Object(_)));
        if let Some(span) = input.atomic_spans.iter().position(|span| {
            let r = span.byte_range;
            !(is_text_or_object(units.starting_at(r.start(), objects))
                && is_text_or_object(units.ending_at(r.end(), objects)))
        }) {
            return Err(FrontendError::InvalidAtomicSpan { span });
        }
        match input.bidi_spans.iter().position(|s| off_edge(s.byte_range)) {
            Some(span) => Err(FrontendError::InvalidBidiSpan { span }),
            None => Ok(()),
        }
    }
}
