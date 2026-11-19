//! Proof that data passed its gate (spec AX-3).
//!
//! [`ValidatedInputs`] exists only for inputs that passed the input gate (G1)
//! and [`ValidatedLayout`] only for output that passed the output gate (G4).
//! Code behind a gate takes these types and does not check again: their
//! lookups index directly, so a broken invariant is a bug that panics instead
//! of degrading to empty content.

use super::input_validation::validate_layout_inputs;
use super::{
    AnnotationErrorReason, LayoutConstraints, LayoutInputError, LayoutOptions,
    LayoutPolicySnapshot, LineGeometry, LineInvariantError, PositionedAnnotationRun,
    PositionedGlyph, PositionedLine, PositionedParagraph, VisualGlyphRun,
};
use crate::frontend::{BoxNode, ParagraphAst, TextBoxView, WordNode};
use crate::types::{
    FontRef, FontVariation, GlyphOrientation, GraphemeAtom, Point, RawShapedGlyph, ShapedCluster,
    WritingMode,
};

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
        let WordNode::Box(BoxNode::Text {
            run_index,
            byte_range,
            atom_range,
            cluster_range,
            natural_width,
        }) = &self.ast.items[item]
        else {
            return None;
        };
        Some(TextBoxView {
            run_index: *run_index,
            byte_range: *byte_range,
            atom_start: atom_range.start,
            atoms: &self.ast.atoms[atom_range.clone()],
            clusters: &self.ast.clusters[cluster_range.clone()],
            natural_width: *natural_width,
        })
    }

    /// The text box at `items[item]`, for an item the caller matched as a
    /// [`BoxNode::Text`].
    ///
    /// # Panics
    /// When `items[item]` is not a text box.
    #[must_use]
    pub fn text_view(&self, item: usize) -> TextBoxView<'a> {
        self.text_box(item)
            .unwrap_or_else(|| panic!("items[{item}] is not a text box"))
    }

    /// The atoms of body run `run` (spec AST-4).
    ///
    /// # Panics
    /// When `run` is not an index into `runs`.
    #[must_use]
    pub fn run_atoms(&self, run: usize) -> &'a [GraphemeAtom] {
        &self.ast.atoms[self.ast.runs[run].atom_range.clone()]
    }

    /// The clusters of body run `run` (spec AST-4).
    ///
    /// # Panics
    /// When `run` is not an index into `runs`.
    #[must_use]
    pub fn run_clusters(&self, run: usize) -> &'a [ShapedCluster] {
        &self.ast.clusters[self.ast.runs[run].cluster_range.clone()]
    }

    /// The glyphs of a body cluster of this paragraph (spec AST-4).
    ///
    /// # Panics
    /// When `cluster` is not a cluster of this paragraph.
    #[must_use]
    pub fn cluster_glyphs(&self, cluster: &ShapedCluster) -> &'a [RawShapedGlyph] {
        &self.ast.glyphs[cluster.glyph_range.clone()]
    }

    /// The geometry of line `line_index` over `[block_start, block_end)`,
    /// through the provider gate (spec GEO-3, G3). The only geometry query
    /// layout uses.
    ///
    /// # Errors
    /// `InvalidLineGeometry` or `NonMonotonicGeometry` for a provider answer
    /// that breaks its contract.
    pub fn line_geometry(
        &self,
        line_index: usize,
        block_start: f32,
        block_end: f32,
        previous: Option<&LineGeometry>,
    ) -> Result<GeometryAnswer, LayoutInputError> {
        let geometry = self.constraints.query_checked_geometry(
            line_index,
            block_start,
            block_end,
            previous,
        )?;
        match (
            geometry.slots.is_empty(),
            geometry.next_feature_block_offset,
        ) {
            (false, _) => Ok(GeometryAnswer::Open(geometry)),
            (true, Some(clear_at)) => Ok(GeometryAnswer::Obstructed { geometry, clear_at }),
            (true, None) => Err(LayoutInputError::InvalidLineGeometry),
        }
    }
}

/// A geometry answer that passed the provider gate (spec GEO-3, G3).
#[derive(Clone, Debug, PartialEq)]
pub enum GeometryAnswer {
    /// At least one slot is available.
    Open(LineGeometry),
    /// No slot is available; the gate proved the answer points ahead
    /// (spec GEO-4).
    Obstructed {
        /// The empty answer.
        geometry: LineGeometry,
        /// The next block start at which geometry may change, beyond the
        /// queried start by a measurement.
        clear_at: f32,
    },
}

impl GeometryAnswer {
    /// The answer's slots and next feature offset.
    #[must_use]
    pub fn geometry(&self) -> &LineGeometry {
        match self {
            Self::Open(geometry) | Self::Obstructed { geometry, .. } => geometry,
        }
    }
}

/// A positioned paragraph that passed the output gate (spec AX-3, G4),
/// together with the text it was validated against. The receiver of the
/// caret, hit-testing and selection queries (spec §6).
#[derive(Clone, Copy, Debug)]
pub struct ValidatedLayout<'a> {
    paragraph: &'a PositionedParagraph,
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
        Ok(Self { paragraph, text })
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
