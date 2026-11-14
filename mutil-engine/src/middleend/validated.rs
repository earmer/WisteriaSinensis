//! Proof that data passed its gate (spec AX-3).
//!
//! [`ValidatedInputs`] exists only for inputs that passed the input gate (G1)
//! and [`ValidatedLayout`] only for output that passed the output gate (G4).
//! Code behind a gate takes these types and does not check again: their
//! lookups index directly, so a broken invariant is a bug that panics instead
//! of degrading to empty content.

use super::input_validation::validate_layout_inputs;
use super::{
    LayoutConstraints, LayoutInputError, LayoutOptions, LayoutPolicySnapshot, LineGeometry,
    LineInvariantError, PositionedParagraph, VisualGlyphRun,
};
use crate::frontend::{BoxNode, ParagraphAst, TextBoxView, WordNode};
use crate::types::{GraphemeAtom, Point, RawShapedGlyph, ShapedCluster, WritingMode};

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
    pub fn new(
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
    ) -> Result<LineGeometry, LayoutInputError> {
        self.constraints
            .query_checked_geometry(line_index, block_start, block_end, previous)
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
    pub fn new(
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

    /// Every visual glyph run of the paragraph, line by line in screen order,
    /// with its glyphs' drawing origins in paragraph-local physical
    /// coordinates (spec HST-4, HST-5). A run keeps its font instance,
    /// orientation and level, so a renderer submits it as one batch.
    pub fn glyph_runs(&self) -> impl Iterator<Item = PlacedRun<'a>> + 'a {
        let paragraph = self.paragraph;
        let mode = paragraph.policy.writing_mode;
        let total_block_size = paragraph.total_block_size;
        paragraph
            .lines
            .iter()
            .enumerate()
            .flat_map(move |(line, positioned)| {
                positioned.visual_runs.iter().map(move |run| PlacedRun {
                    line,
                    run,
                    baseline_y: positioned.baseline_y,
                    mode,
                    total_block_size,
                })
            })
    }
}

/// One visual glyph run of a validated paragraph, ready to draw
/// (spec HST-4, HST-5).
#[derive(Clone, Copy, Debug)]
pub struct PlacedRun<'a> {
    line: usize,
    run: &'a VisualGlyphRun,
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

    /// The run: font instance, size, variations, orientation and level shared
    /// by all its glyphs.
    #[must_use]
    pub fn run(&self) -> &'a VisualGlyphRun {
        self.run
    }

    /// The run's glyphs in screen order, each with the paragraph-local
    /// physical point where its outline's origin is drawn: the flow-space
    /// drawing origin of OUT-1 (pen position plus the inline offset, baseline
    /// moved towards line-over by the over offset) mapped by the writing mode
    /// (spec ORI-1, ORI-2). A `VerticalSideways` glyph is then rotated 90°
    /// clockwise about that point (spec HST-5).
    pub fn glyphs(&self) -> impl Iterator<Item = PlacedGlyph> + 'a {
        let Self {
            baseline_y,
            mode,
            total_block_size,
            ..
        } = *self;
        self.run.glyphs.iter().map(move |glyph| {
            let flow = Point {
                x: glyph.x + glyph.x_offset,
                y: baseline_y + mode.over_to_block(glyph.y_offset),
            };
            PlacedGlyph {
                glyph_id: glyph.glyph_id,
                origin: mode.flow_to_physical_point(flow, total_block_size),
            }
        })
    }
}

/// One glyph to draw: its id in the run's font and its paragraph-local
/// physical drawing origin (spec HST-5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedGlyph {
    /// The glyph id in the run's font.
    pub glyph_id: u32,
    /// Where the glyph outline's origin is drawn, in paragraph-local physical
    /// coordinates.
    pub origin: Point,
}
