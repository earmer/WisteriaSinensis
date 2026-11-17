//! The layout policy a paragraph records for its own validation (spec §2.6,
//! §5 GEO-2).

use crate::frontend::ParagraphAst;
use crate::middleend::{
    HangMode, HangingWhitespaceMode, InlineSlotConstraint, LayoutConstraints, LayoutInputError,
    LayoutOptions, LineClampPolicy, LineHeight, StrutMetrics,
};
use crate::types::{BidiDirection, WritingMode, is_extent, is_measurement};

/// The policy a paragraph was laid out under, recorded in the output so that
/// validating it needs nothing from the caller: a paragraph cannot be checked
/// against a more permissive policy than the one that produced it.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutPolicySnapshot {
    /// Writing mode of the paragraph (`ParagraphAst::writing_mode`).
    pub writing_mode: WritingMode,
    /// The **resolved** strut: `ParagraphAst::strut_metrics` after
    /// `LineHeight::resolve_strut`, half-leading included.
    pub strut: StrutMetrics,
    /// The line-height policy.
    pub line_height: LineHeight,
    /// The trailing-whitespace mode.
    pub whitespace: HangingWhitespaceMode,
    /// The hanging-punctuation mode.
    pub hang_mode: HangMode,
    /// `LayoutConstraints::wrap_width`: positive, possibly infinite. A line
    /// wider than this is valid only in the single-entity overflow state (see
    /// [`PositionedLine::validate`](super::PositionedLine::validate)).
    pub wrap_width: f32,
    /// Where the lines' slots came from.
    pub slot_source: SlotSource,
    /// The truncation policy, if any (`LayoutOptions::clamp`).
    pub clamp: Option<LineClampPolicy>,
}

/// Where a paragraph's slots came from, recorded so that validation can
/// rebuild the standard column (spec §5, GEO-2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SlotSource {
    /// No geometry provider: every line has exactly the one slot
    /// `[indent, wrap_width]`, with `first_line_indent` on line 0 and
    /// `hanging_indent` after it. Both indents lie in `[0, wrap_width)`.
    Standard {
        /// `LayoutConstraints::first_line_indent`.
        first_line_indent: f32,
        /// `LayoutConstraints::hanging_indent`.
        hanging_indent: f32,
    },
    /// A `LineGeometryProvider` answered for every line. Its answers are not
    /// recomputable after layout, so only their shape is validated; the
    /// constraints' indents do not apply.
    Provider,
}

impl SlotSource {
    /// The one slot line `line_index` must have under
    /// [`Standard`](Self::Standard) in a container `wrap_width` wide; `None`
    /// for [`Provider`](Self::Provider), and for indents that leave no
    /// well-formed slot (a policy [`LayoutPolicySnapshot::is_valid`] rejects).
    #[must_use]
    pub fn standard_slot(self, line_index: usize, wrap_width: f32) -> Option<InlineSlotConstraint> {
        let Self::Standard {
            first_line_indent,
            hanging_indent,
        } = self
        else {
            return None;
        };
        let indent = if line_index == 0 {
            first_line_indent
        } else {
            hanging_indent
        };
        InlineSlotConstraint::new(indent, wrap_width)
    }

    /// The slot source `constraints` lay lines out from.
    fn of(constraints: &LayoutConstraints) -> Self {
        if constraints.geometry_provider.is_some() {
            Self::Provider
        } else {
            Self::Standard {
                first_line_indent: constraints.first_line_indent,
                hanging_indent: constraints.hanging_indent,
            }
        }
    }
}

impl LayoutPolicySnapshot {
    /// Resolves the policy of one paragraph from the layout inputs. The strut
    /// is resolved here, once, from the AST's **raw** strut metrics,
    /// `ParagraphAst::strut_font_size`, the writing mode and
    /// `options.line_height`.
    ///
    /// # Errors
    /// `InvalidConstraints`, `InvalidLayoutOptions`, `InvalidStrut` or
    /// `InvalidFontMetrics` for inputs no policy can be resolved from.
    pub fn resolve(
        ast: &ParagraphAst,
        constraints: &LayoutConstraints,
        options: &LayoutOptions,
    ) -> Result<Self, LayoutInputError> {
        if !constraints.is_valid() {
            return Err(LayoutInputError::InvalidConstraints);
        }
        if options.clamp.as_ref().is_some_and(|c| !c.is_valid()) {
            return Err(LayoutInputError::InvalidLayoutOptions);
        }
        Ok(Self {
            writing_mode: ast.writing_mode,
            strut: resolve_strut(ast, options.line_height)?,
            line_height: options.line_height,
            whitespace: options.whitespace,
            hang_mode: options.hang_mode,
            wrap_width: constraints.wrap_width,
            slot_source: SlotSource::of(constraints),
            clamp: options.clamp.clone(),
        })
    }

    /// `true` when the snapshot could have come from
    /// [`resolve`](Self::resolve): a valid strut, line height and clamp, a
    /// positive wrap width, and standard indents inside `[0, wrap_width)`.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let indents_are_valid = match self.slot_source {
            SlotSource::Standard {
                first_line_indent,
                hanging_indent,
            } => [first_line_indent, hanging_indent]
                .into_iter()
                .all(|indent| (0.0..self.wrap_width).contains(&indent)),
            SlotSource::Provider => true,
        };
        self.strut.is_valid()
            && self.line_height.is_valid()
            && self.wrap_width > 0.0
            && indents_are_valid
            && self.clamp.as_ref().is_none_or(LineClampPolicy::is_valid)
    }

    /// The context line `line_index` of a paragraph of base `base_direction`
    /// is validated against.
    #[must_use]
    pub fn line_context(&self, base_direction: BidiDirection, line_index: usize) -> LineContext {
        LineContext {
            writing_mode: self.writing_mode,
            base_direction,
            strut: self.strut,
            line_height: self.line_height,
            whitespace: self.whitespace,
            hang_mode: self.hang_mode,
            wrap_width: self.wrap_width,
            line_index,
        }
    }
}

/// Resolves the AST's **raw** strut under `line_height`, checking the raw
/// extents and font size, then the raw metrics, then the resolved strut.
fn resolve_strut(
    ast: &ParagraphAst,
    line_height: LineHeight,
) -> Result<StrutMetrics, LayoutInputError> {
    let raw = &ast.strut_metrics;
    let extents_are_valid = is_extent(raw.ascent)
        && is_extent(raw.descent)
        && is_measurement(raw.line_gap)
        && raw.ascent + raw.descent > 0.0;
    let font_size_is_valid = is_extent(ast.strut_font_size) && ast.strut_font_size > 0.0;
    if !(extents_are_valid && font_size_is_valid && line_height.is_valid()) {
        return Err(LayoutInputError::InvalidStrut);
    }
    if !raw.is_valid() {
        return Err(LayoutInputError::InvalidFontMetrics);
    }
    let strut = line_height.resolve_strut(raw, ast.strut_font_size, ast.writing_mode);
    if strut.is_valid() {
        Ok(strut)
    } else {
        Err(LayoutInputError::InvalidStrut)
    }
}

/// What one line is validated against: the paragraph's policy
/// ([`LayoutPolicySnapshot`]), its base direction and the line's index.
///
/// There is no default context: an all-zero strut validates no line. Build one
/// with [`LineContext::for_paragraph`] or
/// [`LayoutPolicySnapshot::line_context`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineContext {
    /// Active writing mode.
    pub writing_mode: WritingMode,
    /// The paragraph base direction. It decides which physical edge is the
    /// line start (hanging, leading glue) and where trailing whitespace lies
    /// (UAX #9 L1).
    pub base_direction: BidiDirection,
    /// The **resolved** strut.
    pub strut: StrutMetrics,
    /// The active line-height policy.
    pub line_height: LineHeight,
    /// The active trailing-whitespace mode.
    pub whitespace: HangingWhitespaceMode,
    /// The active hanging-punctuation mode.
    pub hang_mode: HangMode,
    /// The paragraph's `LayoutConstraints::wrap_width`.
    pub wrap_width: f32,
    /// Index of the line within the paragraph; `0` is the first line, which
    /// `HangStartMode::FirstLine` refers to.
    pub line_index: usize,
}

impl LineContext {
    /// The context of line `line_index` of the paragraph `ast` laid out under
    /// `constraints` and `options`.
    ///
    /// # Errors
    /// As [`LayoutPolicySnapshot::resolve`].
    pub fn for_paragraph(
        ast: &ParagraphAst,
        constraints: &LayoutConstraints,
        options: &LayoutOptions,
        line_index: usize,
    ) -> Result<Self, LayoutInputError> {
        LayoutPolicySnapshot::resolve(ast, constraints, options)
            .map(|policy| policy.line_context(ast.base_direction, line_index))
    }
}
