//! Proof that data passed its gate (spec AX-3).
//!
//! [`ValidatedInputs`] exists only for inputs that passed the input gate (G1)
//! and [`ValidatedLayout`] only for output that passed the output gate (G4).
//! Code behind a gate takes these types and does not check again: their
//! lookups index directly, so a broken invariant is a bug that panics instead
//! of degrading to empty content.

use super::input_validation::validate_layout_inputs;
use super::proofs::{LineIdx, NonEmptySlice, SortedCaretStops, StopIdx};
use super::tolerance::resolves_extent;
use super::{
    AnnotationErrorReason, CaretStop, InlineSlotConstraint, LayoutConstraints, LayoutInputError,
    LayoutOptions, LayoutPolicySnapshot, LineGeometry, LineInvariantError, PositionedAnnotationRun,
    PositionedGlyph, PositionedLine, PositionedParagraph, PositionedSlot, VisualGlyphRun,
};
use crate::frontend::{BoxNode, ParagraphAst, TextBox, TextBoxView, WordNode};
use crate::types::{
    FontRef, FontVariation, GlyphOrientation, GraphemeAtom, Point, RawShapedGlyph, ShapedCluster,
    WritingMode,
};
use smallvec::SmallVec;

/// Layout inputs that passed the input gate (spec AX-3, G1), with the policy
/// the gate resolved.
#[derive(Clone, Debug)]
pub struct ValidatedInputs<'a> {
    ast: &'a ParagraphAst,
    text: &'a str,
    constraints: &'a LayoutConstraints,
    options: &'a LayoutOptions,
    policy: LayoutPolicySnapshot,
}

impl<'a> ValidatedInputs<'a> {
    /// The input gate: runs every check of
    /// [`check_layout_inputs`](super::check_layout_inputs) once.
    ///
    /// # Errors
    /// The first violation, as [`check_layout_inputs`](super::check_layout_inputs)
    /// reports it.
    pub fn try_from_ast(
        ast: &'a ParagraphAst,
        text: &'a str,
        constraints: &'a LayoutConstraints,
        options: &'a LayoutOptions,
    ) -> Result<Self, LayoutInputError> {
        let policy = validate_layout_inputs(ast, text, constraints, options)?;
        Ok(Self {
            ast,
            text,
            constraints,
            options,
            policy,
        })
    }

    /// The validated paragraph AST.
    #[must_use]
    pub fn ast(&self) -> &'a ParagraphAst {
        self.ast
    }

    /// The paragraph text the AST was validated against.
    #[must_use]
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// The validated container geometry.
    #[must_use]
    pub fn constraints(&self) -> &'a LayoutConstraints {
        self.constraints
    }

    /// The validated layout policy.
    #[must_use]
    pub fn options(&self) -> &'a LayoutOptions {
        self.options
    }

    /// The policy the gate resolved (spec PAR-1).
    #[must_use]
    pub fn policy(&self) -> &LayoutPolicySnapshot {
        &self.policy
    }

    /// The text box at `items[item]` with its slices resolved; `None` only
    /// when that item is not a [`BoxNode::Text`]. The gate proved every text
    /// box's ranges (spec AST-4).
    ///
    /// # Panics
    /// When `item` is not an index into `items`.
    #[must_use]
    pub fn text_box(&self, item: usize) -> Option<TextBoxView<'a>> {
        // PROOF(caller): `item` indexes `items` (see # Panics).
        let WordNode::Box(BoxNode::Text(text)) = &self.ast.items[item] else {
            return None;
        };
        Some(self.text_view(text))
    }

    /// `text`, a box of this paragraph, with its slices resolved: the gate
    /// proved every text box's ranges (spec AST-4).
    #[must_use]
    pub(crate) fn text_view(&self, text: &'a TextBox) -> TextBoxView<'a> {
        self.ast.resolve(text)
    }

    /// The atoms of body run `run` (spec AST-4).
    ///
    /// # Panics
    /// When `run` is not an index into `runs`.
    #[must_use]
    pub fn run_atoms(&self, run: usize) -> &'a [GraphemeAtom] {
        // PROOF(G1::AST-4): a run's atom range lies in `atoms`; `run` is the
        // caller's index (see # Panics).
        &self.ast.atoms[self.ast.runs[run].atom_range.clone()]
    }

    /// The clusters of body run `run` (spec AST-4).
    ///
    /// # Panics
    /// When `run` is not an index into `runs`.
    #[must_use]
    pub fn run_clusters(&self, run: usize) -> &'a [ShapedCluster] {
        // PROOF(G1::AST-4): a run's cluster range lies in `clusters`; `run`
        // is the caller's index (see # Panics).
        &self.ast.clusters[self.ast.runs[run].cluster_range.clone()]
    }

    /// The glyphs of a body cluster of this paragraph (spec AST-4).
    ///
    /// # Panics
    /// When `cluster` is not a cluster of this paragraph.
    #[must_use]
    pub fn cluster_glyphs(&self, cluster: &ShapedCluster) -> &'a [RawShapedGlyph] {
        // PROOF(G1::AST-4): a cluster's glyph range lies in `glyphs`.
        &self.ast.glyphs[cluster.glyph_range.clone()]
    }

    /// The geometry of line `line_index` for a block envelope `extent` long
    /// from `block_start`: the provider gate (spec GEO-3, GEO-4, G3), the
    /// only geometry query layout uses. Without a provider the standard slot
    /// derives from constraints G1 accepted and is not checked again.
    ///
    /// # Errors
    /// `BlockPositionUnrepresentable` when `f32` cannot hold the envelope at
    /// `block_start`; `InvalidLineGeometry` for a provider answer outside the
    /// envelope, out of order or stalling; `NonMonotonicGeometry` for one not
    /// within `previous`.
    pub fn line_geometry(
        &self,
        line_index: usize,
        block_start: f32,
        extent: f32,
        previous: Option<&LineGeometry>,
    ) -> Result<GeometryAnswer, LayoutInputError> {
        let block_end = block_start + extent;
        if !(block_start.is_finite() && block_end > block_start)
            || !resolves_extent(block_end, extent)
        {
            return Err(LayoutInputError::BlockPositionUnrepresentable);
        }
        let constraints = self.constraints;
        let geometry = constraints.resolve_line_geometry(line_index, block_start, block_end);
        if constraints.geometry_provider.is_some() {
            if !geometry.is_valid(block_start, constraints.wrap_width)
                || geometry.stalls(block_start)
            {
                return Err(LayoutInputError::InvalidLineGeometry);
            }
            if previous.is_some_and(|p| !geometry.is_subset_of(p)) {
                return Err(LayoutInputError::NonMonotonicGeometry);
            }
        }
        GeometryAnswer::classify(geometry).ok_or(LayoutInputError::InvalidLineGeometry)
    }
}

/// A geometry answer that passed the provider gate (spec GEO-3, G3). Only
/// [`ValidatedInputs::line_geometry`] builds one.
#[derive(Clone, Debug, PartialEq)]
pub enum GeometryAnswer {
    /// At least one slot is available.
    Open(OpenGeometry),
    /// No slot is available; the answer points ahead (spec GEO-4).
    Obstructed(Clearance),
}

/// An open geometry answer: at least one slot (spec GEO-3).
#[derive(Clone, Debug, PartialEq)]
pub struct OpenGeometry(LineGeometry);

impl OpenGeometry {
    /// The answer as the provider gave it.
    #[must_use]
    pub fn geometry(&self) -> &LineGeometry {
        &self.0
    }

    /// The first slot, which Phase 1 layout fills (spec ME-2).
    #[must_use]
    pub fn first_slot(&self) -> InlineSlotConstraint {
        // PROOF(G3::classify): an open answer holds at least one slot.
        self.0.slots[0]
    }
}

/// An obstructed geometry answer: the next block start at which geometry may
/// change, at least the minimum clearance step ahead (spec GEO-4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clearance(f32);

impl Clearance {
    /// The block start to try next.
    #[must_use]
    pub fn clear_at(self) -> f32 {
        self.0
    }
}

impl GeometryAnswer {
    /// Classifies an answer the gate accepted: open when it has a slot,
    /// obstructed when it has none but a next offset, and `None` for an empty
    /// answer without one.
    fn classify(geometry: LineGeometry) -> Option<Self> {
        if !geometry.slots.is_empty() {
            return Some(Self::Open(OpenGeometry(geometry)));
        }
        geometry
            .next_feature_block_offset
            .map(|clear_at| Self::Obstructed(Clearance(clear_at)))
    }

    /// The answer as the provider gave it: its slots and next feature offset.
    #[must_use]
    pub fn geometry(&self) -> LineGeometry {
        match self {
            Self::Open(open) => open.0.clone(),
            Self::Obstructed(clearance) => LineGeometry {
                slots: SmallVec::new(),
                next_feature_block_offset: Some(clearance.0),
            },
        }
    }
}

/// A positioned paragraph that passed the output gate (spec AX-3, G4),
/// together with the text it was validated against. The receiver of the
/// caret, hit-testing and selection queries (spec §6).
#[derive(Clone, Copy, Debug)]
pub struct ValidatedLayout<'a> {
    paragraph: &'a PositionedParagraph,
    lines: NonEmptySlice<'a, PositionedLine>,
    text: &'a str,
}

impl<'a> ValidatedLayout<'a> {
    /// The output gate: runs [`PositionedParagraph::validate`] once.
    ///
    /// # Errors
    /// The first violation, with the index of its line.
    pub fn try_from_paragraph(
        paragraph: &'a PositionedParagraph,
        text: &'a str,
    ) -> Result<Self, (usize, LineInvariantError)> {
        paragraph.validate(text)?;
        // PROOF(G4::NoLines): `validate` rejects a paragraph without lines.
        let lines = NonEmptySlice::try_from_slice(&paragraph.lines).expect("G4 proved lines");
        Ok(Self {
            paragraph,
            lines,
            text,
        })
    }

    /// The validated paragraph.
    #[must_use]
    pub fn paragraph(&self) -> &'a PositionedParagraph {
        self.paragraph
    }

    /// The paragraph text the output was validated against.
    #[must_use]
    pub fn text(&self) -> &'a str {
        self.text
    }

    /// The paragraph's lines, which G4 proved non-empty (`NoLines`).
    pub(crate) fn lines(&self) -> NonEmptySlice<'a, PositionedLine> {
        self.lines
    }

    /// Line `index`, or the last line past the end.
    pub(crate) fn line(&self, index: usize) -> ValidatedLine<'a> {
        let index = index.min(self.lines.last_index());
        self.view(index, self.lines.get_clamped(index))
    }

    /// The lines from `from` onwards (`forward`) or backwards, `from`
    /// included; none when `from` is past the last line.
    pub(crate) fn walk_lines(
        &self,
        from: usize,
        forward: bool,
    ) -> impl Iterator<Item = ValidatedLine<'a>> + '_ {
        let step = move |&i: &usize| {
            if forward {
                i.checked_add(1)
            } else {
                i.checked_sub(1)
            }
        };
        std::iter::successors(Some(from), step)
            .map_while(|i| self.lines.get(i).map(|line| self.view(i, line)))
    }

    /// The nearest line after (`forward`) or before `line` that has caret
    /// stops: a line inside an atomic span has none and is crossed whole
    /// (CRT-1, INT-13).
    pub(crate) fn adjacent_line_with_stops(
        &self,
        line: LineIdx,
        forward: bool,
    ) -> Option<LineWithStops<'a>> {
        self.walk_lines(line.get(), forward)
            .skip(1)
            .find_map(ValidatedLine::with_stops)
    }

    /// Line `index`, which `line` is: its stops are those of a line G4
    /// accepted.
    fn view(&self, index: usize, line: &'a PositionedLine) -> ValidatedLine<'a> {
        ValidatedLine {
            index: LineIdx(index),
            line,
            stops: SortedCaretStops::try_from_validated(&line.caret_stops),
        }
    }

    /// Every glyph run of the paragraph with its glyphs' drawing origins in
    /// paragraph-local physical coordinates (spec HST-3, HST-4): line by
    /// line, each line's visual runs in screen order and then its annotation
    /// runs in storage order. A run keeps its font instance and orientation,
    /// so a renderer submits it as one batch.
    pub fn glyph_runs(&self) -> impl Iterator<Item = PlacedRun<'a>> + 'a {
        let paragraph = self.paragraph;
        let mode = paragraph.policy.writing_mode;
        let total_block_size = paragraph.total_block_size;
        paragraph
            .lines
            .iter()
            .enumerate()
            .flat_map(move |(line, positioned)| {
                let body = positioned.visual_runs.iter().map(PlacedRunSource::Body);
                let annotations = positioned
                    .annotations
                    .iter()
                    .map(PlacedRunSource::Annotation);
                body.chain(annotations).map(move |source| PlacedRun {
                    line,
                    source,
                    baseline_y: source.baseline_y(positioned.baseline_y, mode),
                    mode,
                    total_block_size,
                })
            })
    }
}

/// A line of a [`ValidatedLayout`] with the facts G4 proved about it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidatedLine<'a> {
    pub(crate) index: LineIdx,
    pub(crate) line: &'a PositionedLine,
    /// `None` for a line without stops, as one inside an atomic span (CRT-1).
    pub(crate) stops: Option<SortedCaretStops<'a>>,
}

impl<'a> ValidatedLine<'a> {
    /// The line, when it has caret stops.
    pub(crate) fn with_stops(self) -> Option<LineWithStops<'a>> {
        Some(LineWithStops {
            index: self.index,
            line: self.line,
            stops: self.stops?,
        })
    }

    /// The line's first slot.
    pub(crate) fn first_slot(self) -> &'a PositionedSlot {
        // PROOF(G4::NoSlots): `check_slots` rejects a line without slots.
        self.line.slots.first().expect("G4 proved slots")
    }
}

/// A line of a [`ValidatedLayout`] that has caret stops.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineWithStops<'a> {
    pub(crate) index: LineIdx,
    pub(crate) line: &'a PositionedLine,
    pub(crate) stops: SortedCaretStops<'a>,
}

impl<'a> LineWithStops<'a> {
    /// The same line as a [`ValidatedLine`].
    pub(crate) fn as_line(self) -> ValidatedLine<'a> {
        ValidatedLine {
            index: self.index,
            line: self.line,
            stops: Some(self.stops),
        }
    }
}

/// The caret stop that shows a cursor position, on its line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocatedCaret<'a> {
    pub(crate) line: LineWithStops<'a>,
    pub(crate) stop: StopIdx,
}

impl<'a> LocatedCaret<'a> {
    /// The located stop.
    pub(crate) fn caret(self) -> &'a CaretStop {
        self.line.stops.stop(self.stop)
    }
}

/// Where a [`PlacedRun`]'s glyphs come from.
#[derive(Clone, Copy, Debug)]
pub enum PlacedRunSource<'a> {
    /// A visual glyph run of the line body.
    Body(&'a VisualGlyphRun),
    /// A ruby or emphasis annotation run of the line (spec ANO-1).
    Annotation(&'a PositionedAnnotationRun),
}

impl<'a> PlacedRunSource<'a> {
    /// The run's own baseline: the line's, moved towards line-over by an
    /// annotation's `baseline_offset` (spec ANO-3, ORI-2).
    fn baseline_y(self, line_baseline_y: f32, mode: WritingMode) -> f32 {
        match self {
            Self::Body(_) => line_baseline_y,
            Self::Annotation(run) => line_baseline_y + mode.over_to_block(run.baseline_offset),
        }
    }

    fn glyphs(self) -> &'a [PositionedGlyph] {
        match self {
            Self::Body(run) => &run.glyphs,
            Self::Annotation(run) => &run.glyphs,
        }
    }
}

/// One glyph run of a validated paragraph, ready to draw
/// (spec HST-3, HST-4).
#[derive(Clone, Copy, Debug)]
pub struct PlacedRun<'a> {
    line: usize,
    source: PlacedRunSource<'a>,
    baseline_y: f32,
    mode: WritingMode,
    total_block_size: f32,
}

impl<'a> PlacedRun<'a> {
    /// The index of the run's line.
    #[must_use]
    pub fn line(&self) -> usize {
        self.line
    }

    /// The body or annotation run the glyphs come from.
    #[must_use]
    pub fn source(&self) -> PlacedRunSource<'a> {
        self.source
    }

    /// The font and fallback scale shared by every glyph of the run.
    #[must_use]
    pub fn font(&self) -> FontRef {
        match self.source {
            PlacedRunSource::Body(run) => run.font,
            PlacedRunSource::Annotation(run) => run.font,
        }
    }

    /// The requested font size before fallback scaling.
    #[must_use]
    pub fn font_size(&self) -> f32 {
        match self.source {
            PlacedRunSource::Body(run) => run.font_size,
            PlacedRunSource::Annotation(run) => run.font_size,
        }
    }

    /// The variation coordinates of the run's font instance.
    #[must_use]
    pub fn variations(&self) -> &'a [FontVariation] {
        match self.source {
            PlacedRunSource::Body(run) => &run.variations,
            PlacedRunSource::Annotation(run) => &run.variations,
        }
    }

    /// The orientation every glyph of the run is drawn in (spec HST-4).
    #[must_use]
    pub fn orientation(&self) -> GlyphOrientation {
        match self.source {
            PlacedRunSource::Body(run) => run.orientation,
            PlacedRunSource::Annotation(run) => run.orientation,
        }
    }

    /// The run's glyphs in storage order, each with the paragraph-local
    /// physical point where its outline's origin is drawn: the flow-space
    /// drawing origin of OUT-1 (pen position plus the inline offset, the
    /// run's baseline moved towards line-over by the over offset) mapped by
    /// the writing mode (spec ORI-1, ORI-2). A `VerticalSideways` glyph is
    /// then rotated 90° clockwise about that point (spec HST-4).
    pub fn glyphs(&self) -> impl Iterator<Item = PlacedGlyph> + 'a {
        let Self {
            baseline_y,
            mode,
            total_block_size,
            ..
        } = *self;
        self.source.glyphs().iter().map(move |glyph| PlacedGlyph {
            glyph_id: glyph.glyph_id,
            origin: drawing_origin(glyph, baseline_y, mode, total_block_size),
        })
    }
}

/// The paragraph-local physical drawing origin of `glyph` on a run whose
/// baseline is `baseline_y` (spec OUT-1, ORI-1, ORI-2). The output gate
/// checks the same arithmetic for every glyph (spec HST-4).
pub(super) fn drawing_origin(
    glyph: &PositionedGlyph,
    baseline_y: f32,
    mode: WritingMode,
    total_block_size: f32,
) -> Point {
    let flow = Point {
        x: glyph.x + glyph.x_offset,
        y: baseline_y + mode.over_to_block(glyph.y_offset),
    };
    mode.flow_to_physical_point(flow, total_block_size)
}

/// Every placed glyph's origin is finite: the check behind
/// [`PlacedRun::glyphs`], run by the output gate (spec AX-3, HST-4).
pub(super) fn check_drawing_origins(
    line: &PositionedLine,
    mode: WritingMode,
    total_block_size: f32,
) -> Result<(), LineInvariantError> {
    let finite = |source: PlacedRunSource<'_>, glyph: &PositionedGlyph| {
        let origin = drawing_origin(
            glyph,
            source.baseline_y(line.baseline_y, mode),
            mode,
            total_block_size,
        );
        origin.x.is_finite() && origin.y.is_finite()
    };
    for (run, r) in line.visual_runs.iter().enumerate() {
        if let Some(glyph) = r
            .glyphs
            .iter()
            .position(|g| !finite(PlacedRunSource::Body(r), g))
        {
            return Err(LineInvariantError::NonFiniteGlyph { run, glyph });
        }
    }
    for (index, a) in line.annotations.iter().enumerate() {
        if a.glyphs
            .iter()
            .any(|g| !finite(PlacedRunSource::Annotation(a), g))
        {
            return Err(LineInvariantError::InvalidAnnotation {
                index,
                reason: AnnotationErrorReason::InvalidGlyph,
            });
        }
    }
    Ok(())
}

/// One glyph to draw: its id in the run's font and its paragraph-local
/// physical drawing origin (spec HST-4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    /// The glyph id in the run's font.
    pub glyph_id: u32,
    /// Where the glyph outline's origin is drawn, in paragraph-local physical
    /// coordinates.
    pub origin: Point,
}
