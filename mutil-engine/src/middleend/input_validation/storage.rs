//! Storage, presentation, text payload, and source-coverage checks for input validation.

use super::{LayoutInputError, fits};
use crate::frontend::{
    BoxNode, ParagraphAst, ShapingRun, TextBoxView, WordNode, text_is_self_contained,
};
use crate::middleend::shaper_gate::glyph_is_measured;
use crate::types::{
    AtomFlags, GlyphOrientation, GraphemeAtom, RawShapedGlyph, ShapedCluster, WritingMode,
    is_extent, is_measurement,
};

/// Checks that the `glyph_range`s of `clusters` tile a glyph buffer of
/// `glyph_count` entries, in cluster order.
///
/// # Errors
/// The index of the first cluster whose range is out of place, or of the last
/// cluster when glyphs are left over after it.
pub(super) fn check_glyph_ranges(
    clusters: &[ShapedCluster],
    glyph_count: usize,
) -> Result<(), usize> {
    let mut next = 0;
    for (cluster, c) in clusters.iter().enumerate() {
        let r = &c.glyph_range;
        if r.start != next || r.end < r.start || r.end > glyph_count {
            return Err(cluster);
        }
        next = r.end;
    }
    if next == glyph_count {
        Ok(())
    } else {
        Err(clusters.len().saturating_sub(1))
    }
}

/// The storage invariants of the paragraph-wide arrays, checked before any
/// check that resolves a slice of them:
///
/// * the runs' `atom_range`s tile `ast.atoms` and their `cluster_range`s tile
///   `ast.clusters`, in run order, in bounds;
/// * the clusters' `glyph_range`s tile `ast.glyphs`, in cluster order;
/// * in item order, the text boxes of one run take consecutive ranges inside
///   the run's own, and together cover them exactly.
pub(super) fn check_storage(ast: &ParagraphAst) -> Result<TextBoxes<'_>, LayoutInputError> {
    use LayoutInputError as E;
    let (mut atom_at, mut cluster_at) = (0, 0);
    for (run, r) in ast.runs.iter().enumerate() {
        let (a, c) = (&r.atom_range, &r.cluster_range);
        if a.start != atom_at
            || a.end < a.start
            || a.end > ast.atoms.len()
            || c.start != cluster_at
            || c.end < c.start
            || c.end > ast.clusters.len()
        {
            return Err(E::InvalidRunStorage { run });
        }
        (atom_at, cluster_at) = (a.end, c.end);
    }
    if atom_at != ast.atoms.len() || cluster_at != ast.clusters.len() {
        return Err(E::InvalidRunStorage {
            run: ast.runs.len().saturating_sub(1),
        });
    }
    check_glyph_ranges(&ast.clusters, ast.glyphs.len())
        .map_err(|cluster| E::InvalidGlyphStorage { cluster })?;

    // The next unclaimed atom and cluster of each run.
    let mut next: Vec<(usize, usize)> = ast
        .runs
        .iter()
        .map(|r| (r.atom_range.start, r.cluster_range.start))
        .collect();
    let mut boxes = Vec::with_capacity(ast.items.len());
    for (item, node) in ast.items.iter().enumerate() {
        let WordNode::Box(BoxNode::Text {
            run_index,
            byte_range,
            atom_range,
            cluster_range,
            natural_width,
        }) = node
        else {
            boxes.push(None);
            continue;
        };
        let run = ast
            .runs
            .get(*run_index)
            .ok_or(E::RunIndexOutOfRange { item })?;
        let (next_atom, next_cluster) = &mut next[*run_index];
        if atom_range.start != *next_atom
            || atom_range.end < atom_range.start
            || atom_range.end > run.atom_range.end
            || cluster_range.start != *next_cluster
            || cluster_range.end < cluster_range.start
            || cluster_range.end > run.cluster_range.end
        {
            return Err(E::ItemNotSelfContained { item });
        }
        (*next_atom, *next_cluster) = (atom_range.end, cluster_range.end);
        // The ranges lie inside the run's, which the run checks above put
        // inside the storage.
        boxes.push(Some(TextBoxView {
            run_index: *run_index,
            byte_range: *byte_range,
            atom_start: atom_range.start,
            atoms: &ast.atoms[atom_range.clone()],
            clusters: &ast.clusters[cluster_range.clone()],
            natural_width: *natural_width,
        }));
    }
    for (run, (r, (atom_end, cluster_end))) in ast.runs.iter().zip(next).enumerate() {
        if atom_end != r.atom_range.end || cluster_end != r.cluster_range.end {
            return Err(E::InvalidRunStorage { run });
        }
    }
    Ok(TextBoxes(boxes))
}

/// A run's own shaping settings: its bytes fit `text`, its size is positive,
/// its script resolved.
pub(super) fn check_run(
    run: &ShapingRun,
    text: &str,
    index: usize,
) -> Result<(), LayoutInputError> {
    use LayoutInputError as E;
    if !fits(text, run.byte_range) {
        return Err(E::RunOutsideText { run: index });
    }
    if !(is_extent(run.font_size) && run.font_size > 0.0 && !run.script.is_unresolved()) {
        return Err(E::InvalidShapingRun { run: index });
    }
    Ok(())
}

/// Every run on its own ([`check_run`]), then the body runs progress through
/// the text (`prev.end <= next.start`); `check_run` proves each range ordered
/// and inside `text`.
pub(super) fn check_runs(ast: &ParagraphAst, text: &str) -> Result<(), LayoutInputError> {
    let mut previous_end = 0;
    for (run, r) in ast.runs.iter().enumerate() {
        check_run(r, text, run)?;
        if r.byte_range.start() < previous_end {
            return Err(LayoutInputError::InvalidShapingRun { run });
        }
        previous_end = r.byte_range.end();
    }
    Ok(())
}

/// A run's orientation suits the writing mode and its level, and its explicit
/// spacing and baseline shift are measurements (MSR-1).
pub(super) fn check_run_presentation(
    run: &ShapingRun,
    mode: WritingMode,
) -> Result<(), LayoutInputError> {
    if !run.orientation.is_valid(mode, run.bidi_level) {
        return Err(LayoutInputError::OrientationMismatch);
    }
    if !(is_measurement(run.letter_spacing)
        && is_measurement(run.word_spacing)
        && run.baseline_align.is_valid())
    {
        return Err(LayoutInputError::InvalidSpacing);
    }
    Ok(())
}

/// The atoms and clusters of one text payload present the way their run says:
/// they share its orientation, which suits the writing mode and their level,
/// and no atom that forbids letter spacing sits in a letter-spaced run.
pub(super) fn check_text_presentation(
    run: &ShapingRun,
    atoms: &[GraphemeAtom],
    clusters: &[ShapedCluster],
    mode: WritingMode,
) -> Result<(), LayoutInputError> {
    let orientation_is_valid = |orientation: GlyphOrientation, level| {
        orientation == run.orientation && orientation.is_valid(mode, level)
    };
    if !atoms
        .iter()
        .all(|a| orientation_is_valid(a.orientation, a.bidi_level))
        || !clusters
            .iter()
            .all(|c| orientation_is_valid(c.orientation, c.bidi_level))
    {
        return Err(LayoutInputError::OrientationMismatch);
    }
    if run.letter_spacing != 0.0
        && atoms
            .iter()
            .any(|a| a.flags.contains(AtomFlags::NO_LETTER_SPACING))
    {
        return Err(LayoutInputError::InvalidSpacing);
    }
    Ok(())
}

/// Writing-mode rules of the body: every run and every text box presents
/// consistently, and a tate-chu-yoko run is one cluster in one text box that
/// spans the whole run (so no break can fall inside it).
pub(super) fn check_presentation(
    ast: &ParagraphAst,
    boxes: &TextBoxes<'_>,
) -> Result<(), LayoutInputError> {
    for run in &ast.runs {
        check_run_presentation(run, ast.writing_mode)?;
    }
    for view in boxes.iter() {
        let run = &ast.runs[view.run_index];
        // The box lies inside its run (`check_storage`), so equal lengths
        // mean it spans the whole run.
        if run.orientation == GlyphOrientation::HorizontalInVertical
            && (view.clusters.len() != 1
                || view.atoms.len() != run.atom_range.len()
                || view.clusters.len() != run.cluster_range.len())
        {
            return Err(LayoutInputError::OrientationMismatch);
        }
        check_text_presentation(run, view.atoms, view.clusters, ast.writing_mode)?;
    }
    Ok(())
}

/// `true` when every measurement a cluster hands downstream is a measurement
/// (MSR-1) and the fallback scale is positive. `glyphs` is the buffer the cluster's
/// `glyph_range` indexes, already proved in bounds by `check_glyph_ranges`.
fn cluster_measurements_are_valid(cluster: &ShapedCluster, glyphs: &[RawShapedGlyph]) -> bool {
    let glyphs = &glyphs[cluster.glyph_range.clone()];
    is_measurement(cluster.font.scale)
        && cluster.font.scale > 0.0
        && is_extent(cluster.natural_width)
        && glyphs.iter().all(glyph_is_measured)
}

/// The text boxes of an AST whose storage `check_storage` proved: one entry per
/// item, `Some` exactly for the items that are text boxes. Later phases read
/// boxes from here and never resolve them again.
pub(super) struct TextBoxes<'a>(Vec<Option<TextBoxView<'a>>>);

impl<'a> TextBoxes<'a> {
    /// The text box at `items[item]`, or `None` if that item is not one.
    pub(super) fn get(&self, item: usize) -> Option<&TextBoxView<'a>> {
        self.0.get(item)?.as_ref()
    }

    /// The text box at `items[item]`, for an item the caller matched as a
    /// [`BoxNode::Text`]: `check_storage` produced one for each.
    ///
    /// # Panics
    /// When `items[item]` is not a text box.
    pub(super) fn view(&self, item: usize) -> &TextBoxView<'a> {
        self.get(item)
            .unwrap_or_else(|| panic!("items[{item}] is not a text box"))
    }

    /// Every text box, in item order.
    pub(super) fn iter(&self) -> impl Iterator<Item = &TextBoxView<'a>> {
        self.0.iter().flatten()
    }
}

/// Checks one text payload (a text box, or an annotation's text) against its
/// run. `glyphs` is the glyph buffer its clusters index; `base` is the
/// run-local index of the payload's first atom (0 for an annotation).
pub(super) fn check_text_payload(
    text: &str,
    run: &ShapingRun,
    payload: &TextBoxView<'_>,
    glyphs: &[RawShapedGlyph],
    base: usize,
    item: usize,
) -> Result<(), LayoutInputError> {
    use LayoutInputError as E;
    let TextBoxView {
        byte_range,
        atoms,
        clusters,
        natural_width,
        ..
    } = *payload;
    if !atoms.iter().all(|a| fits(text, a.byte_range)) {
        return Err(E::ItemOutsideText { item });
    }
    if !text_is_self_contained(byte_range, atoms, clusters, base)
        || !clusters.iter().all(ShapedCluster::safety_flags_consistent)
    {
        return Err(E::ItemNotSelfContained { item });
    }
    // Atoms tile the payload's bytes, so the payload is inside the run's bytes
    // exactly when its range is.
    if !run.byte_range.contains_range(byte_range)
        || atoms.iter().any(|a| a.bidi_level != run.bidi_level)
        || clusters.iter().any(|c| c.bidi_level != run.bidi_level)
    {
        return Err(E::ItemOutsideRun { item });
    }
    if !clusters
        .iter()
        .all(|c| cluster_measurements_are_valid(c, glyphs))
        || !natural_width.is_finite()
        || natural_width != ShapedCluster::total_width(clusters)
    {
        return Err(E::InvalidTextWidth { item });
    }
    if atoms.iter().any(|a| a.script != run.script) {
        return Err(E::ScriptMismatch { item });
    }
    Ok(())
}

/// Source coverage: the non-empty item ranges concatenate to exactly
/// `0..text.len()`. Item order makes each empty range a boundary of the
/// coverage cursor.
pub(super) fn check_coverage(ast: &ParagraphAst, text: &str) -> Result<(), LayoutInputError> {
    let mut covered = 0;
    for node in &ast.items {
        let range = node.byte_span();
        if range.start() != covered {
            break;
        }
        covered = range.end();
    }
    if covered == text.len() {
        Ok(())
    } else {
        Err(LayoutInputError::UncoveredSource { byte: covered })
    }
}
