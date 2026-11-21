//! Span, bidi-span, annotation, and boundary-enclosure checks for input validation.

use super::items::{ItemsChecked, ItemsInOrder};
use super::storage::{
    TextBoxes, check_glyph_ranges, check_run, check_run_presentation, check_text_payload,
    check_text_presentation,
};
use super::{LayoutInputError, fits, in_byte_order, is_range_edge};
use crate::frontend::{
    AnnotationKind, AnnotationSide, AnnotationSpan, BoxNode, GlueKind, InlineObjectBox,
    MAX_BIDI_SPAN_DEPTH, MAX_DECORATION_NESTING_DEPTH, ParagraphAst, SpanEdge, TextBoxView,
    WordNode, is_line_or_paragraph_separator,
};
use crate::types::{BidiLevel, GlyphOrientation, TextRange, WritingMode, is_extent};
use std::collections::{HashMap, HashSet};

/// The offsets at which the content of a paragraph starts and ends, for the
/// span streams that must align with it.
struct ContentEdges {
    /// Every start and end of an atom, inline object, glue, tab or
    /// discretionary.
    node_edges: HashSet<usize>,
    /// Every start and end of a cluster.
    cluster_edges: HashSet<usize>,
    /// Atom starts and atom ends, each with the index of its **text stretch**:
    /// a maximal sequence of text boxes with no glue, tab, inline object or
    /// discretionary between them.
    atom_starts: HashMap<usize, usize>,
    atom_ends: HashMap<usize, usize>,
}

impl ContentEdges {
    fn new(ast: &ParagraphAst, boxes: &TextBoxes<'_>) -> Self {
        let mut edges = Self {
            node_edges: HashSet::new(),
            cluster_edges: HashSet::new(),
            atom_starts: HashMap::new(),
            atom_ends: HashMap::new(),
        };
        let mut stretch = 0;
        for (item, node) in ast.items.iter().enumerate() {
            if let Some(view) = boxes.get(item) {
                for atom in view.atoms {
                    let range = atom.byte_range;
                    edges.node_edges.extend([range.start(), range.end()]);
                    edges.atom_starts.insert(range.start(), stretch);
                    edges.atom_ends.insert(range.end(), stretch);
                }
                for cluster in view.clusters {
                    let range = cluster.byte_range;
                    edges.cluster_edges.extend([range.start(), range.end()]);
                }
            } else if !matches!(node, WordNode::Box(BoxNode::SpanBoundary { .. })) {
                let range = node.byte_span();
                edges.node_edges.extend([range.start(), range.end()]);
                stretch += 1;
            }
        }
        edges
    }

    /// Whether `range` starts on an atom and cluster start and ends on an atom
    /// and cluster end of the same text stretch, so that it covers whole
    /// clusters and nothing but text.
    fn is_text_only(&self, range: TextRange) -> bool {
        self.cluster_edges.contains(&range.start())
            && self.cluster_edges.contains(&range.end())
            && self
                .atom_starts
                .get(&range.start())
                .zip(self.atom_ends.get(&range.end()))
                .is_some_and(|(start, end)| start == end)
    }
}

/// The two span streams that must align with content edges. Skipped when
/// both are empty, so the edge index is not built for plain paragraphs.
pub(super) fn check_bidi_and_annotations(
    ast: &ParagraphAst,
    text: &str,
    boxes: &TextBoxes<'_>,
) -> Result<(), LayoutInputError> {
    if ast.bidi_spans.is_empty() && ast.annotations.is_empty() {
        return Ok(());
    }
    let edges = ContentEdges::new(ast, boxes);
    check_bidi_spans(ast, text, &edges)?;
    check_annotations(ast, text, &edges)
}

/// Explicit bidi spans: unique ids; non-empty ranges that fit the text and
/// start and end on content edges; ordered by start, an outer span before the
/// spans it contains; properly nested no deeper than [`MAX_BIDI_SPAN_DEPTH`].
fn check_bidi_spans(
    ast: &ParagraphAst,
    text: &str,
    edges: &ContentEdges,
) -> Result<(), LayoutInputError> {
    let mut ids = HashSet::with_capacity(ast.bidi_spans.len());
    let mut open: Vec<TextRange> = Vec::new();
    let mut previous: Option<TextRange> = None;
    for (span, s) in ast.bidi_spans.iter().enumerate() {
        let r = s.byte_range;
        let is_ordered = previous
            .is_none_or(|p| r.start() > p.start() || (r.start() == p.start() && r.end() < p.end()));
        while open.last().is_some_and(|outer| outer.end() <= r.start()) {
            open.pop();
        }
        if !(ids.insert(s.span_id)
            && !r.is_empty()
            && fits(text, r)
            && edges.node_edges.contains(&r.start())
            && edges.node_edges.contains(&r.end())
            && is_ordered
            && open.last().is_none_or(|outer| r.end() <= outer.end())
            && open.len() < MAX_BIDI_SPAN_DEPTH)
        {
            return Err(LayoutInputError::InvalidBidiSpan { span });
        }
        open.push(r);
        previous = Some(r);
    }
    Ok(())
}

/// Annotations: unique ids; a non-empty base over whole clusters of one text
/// stretch, disjoint from the other bases on its side and in start order; a
/// finite extent and overhang; and a self-contained, independently shaped
/// payload in annotation-local coordinates.
fn check_annotations(
    ast: &ParagraphAst,
    text: &str,
    edges: &ContentEdges,
) -> Result<(), LayoutInputError> {
    let mut ids = HashSet::with_capacity(ast.annotations.len());
    // The end of the last base seen on each side.
    let mut reach = [0, 0];
    for (annotation, ann) in ast.annotations.iter().enumerate() {
        let base = ann.base_byte_range;
        let side = usize::from(ann.side == AnnotationSide::Under);
        let base_is_valid = !base.is_empty()
            && fits(text, base)
            && base.start() >= reach[side]
            && edges.is_text_only(base);
        if !(ids.insert(ann.annotation_id)
            && base_is_valid
            && is_extent(ann.block_extent)
            && annotation_payload_is_valid(ann, ast.writing_mode))
        {
            return Err(LayoutInputError::InvalidAnnotationSpan { annotation });
        }
        reach[side] = base.end();
    }
    Ok(())
}

/// The independently shaped payload of one annotation.
fn annotation_payload_is_valid(ann: &AnnotationSpan, mode: WritingMode) -> bool {
    let text = &*ann.annotation_text;
    let run = &ann.run;
    let storage_is_valid = !text.is_empty()
        && !text.chars().any(is_line_or_paragraph_separator)
        && run.byte_range == TextRange::new(0, text.len())
        && run.atom_range == (0..ann.atoms.len())
        && run.cluster_range == (0..ann.clusters.len())
        && check_glyph_ranges(&ann.clusters, ann.glyphs.len()).is_ok();
    let kind_is_valid = match ann.kind {
        AnnotationKind::Ruby { max_overhang, .. } => is_extent(max_overhang),
        // One shaped mark, repeated over the base.
        AnnotationKind::Emphasis => ann.atoms.len() == 1 && ann.clusters.len() == 1,
    };
    let payload = TextBoxView {
        run_index: 0,
        byte_range: run.byte_range,
        atom_start: 0,
        atoms: &ann.atoms,
        clusters: &ann.clusters,
        natural_width: ann.clusters.iter().map(|c| c.natural_width).sum(),
    };
    storage_is_valid
        && kind_is_valid
        && check_run_presentation(run, mode).is_ok()
        && check_run(run, text, 0).is_ok()
        && check_text_payload(text, run, &payload, &ann.glyphs, 0, 0).is_ok()
        && check_text_presentation(run, &ann.atoms, &ann.clusters, mode).is_ok()
        && (run.orientation != GlyphOrientation::HorizontalInVertical || ann.clusters.len() == 1)
}

/// Zero-allocation iterator over the byte ranges of concrete content: every
/// text atom and inline object, in item order.
fn concrete_content_ranges<'a>(
    ast: &'a ParagraphAst,
    boxes: &'a TextBoxes<'a>,
) -> impl Iterator<Item = TextRange> + 'a {
    ast.items.iter().enumerate().flat_map(|(item, node)| {
        let (atoms, object) = if let Some(view) = boxes.get(item) {
            (view.atoms, None)
        } else if let WordNode::Box(BoxNode::InlineObject(InlineObjectBox { byte_range, .. })) =
            node
        {
            (&[][..], Some(*byte_range))
        } else {
            (&[][..], None)
        };
        atoms.iter().map(|a| a.byte_range).chain(object)
    })
}

/// Concrete content ranges (atoms and inline objects) proved in non-overlapping
/// byte order, so binary search over edges is sound.
struct SortedContent(Vec<TextRange>);

impl SortedContent {
    /// Probes concrete content order with zero allocations; collects the ranges
    /// only when they are in byte order.
    fn try_from_ast(ast: &ParagraphAst, boxes: &TextBoxes<'_>) -> Option<Self> {
        if !in_byte_order(concrete_content_ranges(ast, boxes)) {
            return None;
        }
        Some(Self(concrete_content_ranges(ast, boxes).collect()))
    }
}

/// Proof obligation returned by [`check_atomic_spans`] and discharged by
/// [`discharge_atomic_spans`] after item order and self-containment run.
#[must_use]
pub(super) enum AtomicSpanCheck {
    Verified,
    Deferred,
}

/// Validates the decoration spans: ranges fit the text, ids are unique,
/// paddings are finite and non-negative, and the spans **nest properly**
/// (two spans are disjoint or one contains the other, never a partial
/// overlap) no deeper than [`MAX_DECORATION_NESTING_DEPTH`].
pub(super) fn check_decoration_spans(
    ast: &ParagraphAst,
    text: &str,
) -> Result<DecorationIds, LayoutInputError> {
    use LayoutInputError as E;
    if u32::try_from(ast.decoration_spans.len()).is_err() {
        return Err(E::InvalidDecorationSpan {
            span: u32::MAX as usize,
        });
    }
    let mut ids = HashMap::with_capacity(ast.decoration_spans.len());
    let mut stack: Vec<TextRange> = Vec::new();
    for (span, s) in ast.decoration_spans.iter().enumerate() {
        let r = s.byte_range;
        if span > 0 {
            let prev = ast.decoration_spans[span - 1].byte_range;
            if r.start() < prev.start() || (r.start() == prev.start() && r.end() > prev.end()) {
                return Err(E::InvalidDecorationSpan { span });
            }
        }
        while stack.last().is_some_and(|top| top.end() <= r.start()) {
            stack.pop();
        }
        if ids.insert(s.span_id, span).is_some()
            || !s.is_valid()
            || !fits(text, r)
            || stack.last().is_some_and(|top| !top.contains_range(r))
            || stack.len() >= MAX_DECORATION_NESTING_DEPTH
        {
            return Err(E::InvalidDecorationSpan { span });
        }
        stack.push(r);
    }
    Ok(DecorationIds(ids))
}

/// Validates the atomic spans: ranges fit the text, ids are unique, and each
/// span is non-empty, **strictly disjoint** from the others, has both edges
/// at the start or end of a text atom or an inline object, and wraps at
/// least one atom or object (a span of whitespace alone is not atomic
/// content).
///
/// Runs before the item checks to honor AST-11 error precedence. When
/// concrete content is not yet in byte order, span-intrinsic invariants still
/// run here and the content-edge check is returned as
/// [`AtomicSpanCheck::Deferred`], which [`discharge_atomic_spans`] must
/// discharge after `check_item_order` and `check_each_item`.
pub(super) fn check_atomic_spans(
    ast: &ParagraphAst,
    text: &str,
    boxes: &TextBoxes<'_>,
) -> Result<AtomicSpanCheck, LayoutInputError> {
    if ast.atomic_spans.is_empty() {
        return Ok(AtomicSpanCheck::Verified);
    }
    if let Some(sorted) = SortedContent::try_from_ast(ast, boxes) {
        validate_atomic_spans(ast, text, &sorted)?;
        return Ok(AtomicSpanCheck::Verified);
    }
    check_atomic_span_basics(ast, text)?;
    Ok(AtomicSpanCheck::Deferred)
}

/// Discharges an [`AtomicSpanCheck`] after item order and per-item
/// self-containment have been proved, ensuring no AST can bypass atomic-span
/// edge validation via unsorted content.
pub(super) fn discharge_atomic_spans(
    ast: &ParagraphAst,
    text: &str,
    boxes: &TextBoxes<'_>,
    check: AtomicSpanCheck,
    _: &ItemsInOrder,
    _: &ItemsChecked,
) -> Result<(), LayoutInputError> {
    match check {
        AtomicSpanCheck::Verified => Ok(()),
        AtomicSpanCheck::Deferred => {
            let Some(sorted) = SortedContent::try_from_ast(ast, boxes) else {
                return Err(LayoutInputError::InvalidAtomicSpan { span: 0 });
            };
            validate_atomic_spans(ast, text, &sorted)
        }
    }
}

/// Content-independent atomic-span checks: unique ids, non-empty ranges inside
/// `text`, and strictly disjoint ascending order.
fn check_atomic_span_basics(ast: &ParagraphAst, text: &str) -> Result<(), LayoutInputError> {
    let mut atomic_ids = HashSet::with_capacity(ast.atomic_spans.len());
    for (span, s) in ast.atomic_spans.iter().enumerate() {
        if !atomic_ids.insert(s.span_id)
            || s.byte_range.is_empty()
            || !fits(text, s.byte_range)
            || (span > 0 && ast.atomic_spans[span - 1].byte_range.end() > s.byte_range.start())
        {
            return Err(LayoutInputError::InvalidAtomicSpan { span });
        }
    }
    Ok(())
}

/// Full atomic-span validation against concrete content proved in byte order.
fn validate_atomic_spans(
    ast: &ParagraphAst,
    text: &str,
    SortedContent(content): &SortedContent,
) -> Result<(), LayoutInputError> {
    use LayoutInputError as E;
    // The offsets an atomic span may start or end at: the start or end of a
    // text atom or of an inline object. Glue, tabs and line terminators
    // contribute none, so an edge inside whitespace is rejected.
    let is_edge = |offset: usize| is_range_edge(content, |r| *r, offset);
    let mut content_at = 0;
    let mut atomic_ids = HashSet::with_capacity(ast.atomic_spans.len());
    for (span, s) in ast.atomic_spans.iter().enumerate() {
        while content_at < content.len() && content[content_at].end() <= s.byte_range.start() {
            content_at += 1;
        }
        if !atomic_ids.insert(s.span_id)
            || s.byte_range.is_empty()
            || !fits(text, s.byte_range)
            || (span > 0 && ast.atomic_spans[span - 1].byte_range.end() > s.byte_range.start())
            || !is_edge(s.byte_range.start())
            || !is_edge(s.byte_range.end())
            || !content
                .get(content_at)
                .is_some_and(|r| !r.is_empty() && s.byte_range.contains_range(*r))
        {
            return Err(E::InvalidAtomicSpan { span });
        }
    }
    Ok(())
}

/// The index into `decoration_spans` of each span id, proved unique by
/// `check_decoration_spans`.
#[derive(Clone, Debug)]
pub(in crate::middleend) struct DecorationIds(HashMap<u64, usize>);

impl DecorationIds {
    /// The index of the decoration span with `span_id`, if there is one.
    pub(in crate::middleend) fn index_of(&self, span_id: u64) -> Option<usize> {
        self.0.get(&span_id).copied()
    }
}

/// Indexed span membership for item checks; nesting is bounded by eight.
pub(super) struct SpanIndex {
    /// Span indices touching item `i`: `touching_flat[touching_start[i]..touching_start[i + 1]]`.
    touching_flat: Vec<usize>,
    touching_start: Vec<usize>,
    /// Per span, the bidi levels of the first and of the last inline entity
    /// (text atom or inline object) inside it.
    pub(super) levels: Vec<(Option<BidiLevel>, Option<BidiLevel>)>,
}

impl SpanIndex {
    /// Indexes the spans over items [`ItemsInOrder`] proved in byte order,
    /// which the sweep over both relies on.
    pub(super) fn new(ast: &ParagraphAst, boxes: &TextBoxes<'_>, _: &ItemsInOrder) -> Self {
        if ast.decoration_spans.is_empty() {
            return Self {
                touching_flat: Vec::new(),
                touching_start: Vec::new(),
                levels: Vec::new(),
            };
        }
        let mut touching_flat = Vec::new();
        let mut touching_start = Vec::with_capacity(ast.items.len() + 1);
        let mut levels = vec![(None, None); ast.decoration_spans.len()];
        let (mut next, mut active) = (0, Vec::<usize>::new());
        for (item, node) in ast.items.iter().enumerate() {
            let r = node.byte_span();
            active.retain(|&i| ast.decoration_spans[i].byte_range.end() >= r.start());
            while next < ast.decoration_spans.len()
                && ast.decoration_spans[next].byte_range.start() <= r.end()
            {
                if ast.decoration_spans[next].byte_range.end() >= r.start() {
                    active.push(next);
                }
                next += 1;
            }
            if !active.is_empty() {
                // The first and last inline entity inside each span: text
                // atoms and inline objects (WN-8).
                if let Some(view) = boxes.get(item) {
                    for &i in &active {
                        let span = ast.decoration_spans[i].byte_range;
                        for atom in view.atoms {
                            if span.contains_range(atom.byte_range) {
                                levels[i].0.get_or_insert(atom.bidi_level);
                                levels[i].1 = Some(atom.bidi_level);
                            }
                        }
                    }
                } else if let WordNode::Box(BoxNode::InlineObject(InlineObjectBox {
                    byte_range,
                    bidi_level,
                    ..
                })) = node
                {
                    for &i in &active {
                        if ast.decoration_spans[i]
                            .byte_range
                            .contains_range(*byte_range)
                        {
                            levels[i].0.get_or_insert(*bidi_level);
                            levels[i].1 = Some(*bidi_level);
                        }
                    }
                }
            }
            touching_start.push(touching_flat.len());
            touching_flat.extend_from_slice(&active);
        }
        touching_start.push(touching_flat.len());
        Self {
            touching_flat,
            touching_start,
            levels,
        }
    }

    /// Indices into `decoration_spans` of the spans touching `items[item]`.
    pub(super) fn touching(&self, item: usize) -> &[usize] {
        if self.touching_flat.is_empty() {
            return &[];
        }
        &self.touching_flat[self.touching_start[item]..self.touching_start[item + 1]]
    }
}

/// Indices into `items` of the `Leading` and `Trailing` nodes of one span.
#[derive(Clone, Copy)]
struct BoundaryItems {
    leading: usize,
    trailing: usize,
}

/// The boundary nodes of every decoration span, indexed by span: the proof
/// [`check_boundary_order`] returns that both exist and nest.
pub(super) struct SpanBoundaries(Vec<BoundaryItems>);

/// Enclosure of the boundaries [`check_boundary_order`] found.
pub(super) fn check_span_enclosure(
    ast: &ParagraphAst,
    index: &SpanIndex,
    SpanBoundaries(boundary_positions): &SpanBoundaries,
) -> Result<(), LayoutInputError> {
    if ast.decoration_spans.is_empty() {
        return Ok(());
    }
    for (item, node) in ast.items.iter().enumerate() {
        let item_range = node.byte_span();
        for &span_index in index.touching(item) {
            let span = &ast.decoration_spans[span_index];
            if matches!(node, WordNode::Box(BoxNode::SpanBoundary { span_id, .. }) if *span_id == span.span_id)
            {
                continue;
            }
            if !span_enclosure_is_allowed(
                node,
                item_range,
                span.byte_range,
                item,
                boundary_positions[span_index],
            ) {
                return Err(LayoutInputError::SpanEnclosure {
                    span: span_index,
                    item,
                });
            }
        }
    }
    Ok(())
}

fn span_enclosure_is_allowed(
    node: &WordNode,
    item_range: TextRange,
    span_range: TextRange,
    item: usize,
    BoundaryItems { leading, trailing }: BoundaryItems,
) -> bool {
    let is_strictly_between_boundaries = leading < item && item < trailing;
    let is_zero_width = item_range.is_empty();
    let is_cjk_autospace =
        matches!(node, WordNode::Glue(glue) if glue.kind == GlueKind::CjkAutospace);
    let is_zero_width_break = is_zero_width && (node.is_break_opportunity() || is_cjk_autospace);
    let sits_on_span_edge =
        item_range.start() == span_range.start() || item_range.start() == span_range.end();

    if is_zero_width_break && item_range.start() == span_range.start() {
        return item < leading;
    }
    if is_zero_width_break && item_range.start() == span_range.end() {
        return item > trailing;
    }
    if is_zero_width && sits_on_span_edge {
        return true;
    }
    if item_range.start() >= span_range.start() && item_range.end() <= span_range.end() {
        return is_strictly_between_boundaries;
    }
    if item_range.end() <= span_range.start() || item_range.start() >= span_range.end() {
        return !is_strictly_between_boundaries;
    }
    false
}

/// The boundary nodes nest: a `Trailing` edge closes the innermost open span,
/// no `Leading` repeats, two spans with equal ranges nest in slice order, and
/// every decoration span has both of its boundaries.
pub(super) fn check_boundary_order(
    ast: &ParagraphAst,
    ids: &DecorationIds,
) -> Result<SpanBoundaries, LayoutInputError> {
    use LayoutInputError as E;
    let mut open_spans: Vec<usize> = Vec::new();
    let mut seen_leading_spans = HashSet::new();
    let mut positions = vec![(None, None); ast.decoration_spans.len()];
    for (item, node) in ast.items.iter().enumerate() {
        let WordNode::Box(BoxNode::SpanBoundary { span_id, edge, .. }) = node else {
            continue;
        };
        let span = ids.index_of(*span_id).ok_or(E::UnknownSpan { item })?;
        match edge {
            SpanEdge::Leading => {
                if !seen_leading_spans.insert(*span_id) {
                    return Err(E::InvalidSpanBoundary { item });
                }
                positions[span].0 = Some(item);
                if let Some(&outer) = open_spans.last()
                    && ast.decoration_spans[outer].byte_range
                        == ast.decoration_spans[span].byte_range
                    && outer > span
                {
                    return Err(E::InvalidSpanBoundary { item });
                }
                open_spans.push(span);
            }
            SpanEdge::Trailing => {
                if open_spans.last() != Some(&span) {
                    return Err(E::InvalidSpanBoundary { item });
                }
                open_spans.pop();
                positions[span].1 = Some(item);
            }
        }
    }
    positions
        .into_iter()
        .enumerate()
        .map(|(span, edges)| match edges {
            (Some(leading), Some(trailing)) => Ok(BoundaryItems { leading, trailing }),
            _ => Err(E::MissingSpanBoundary { span }),
        })
        .collect::<Result<_, _>>()
        .map(SpanBoundaries)
}
