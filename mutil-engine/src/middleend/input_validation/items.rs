//! Item-level ordering and node-by-node validation for input validation.

use super::spans::{DecorationIds, SpanIndex};
use super::storage::{TextBoxes, check_text_payload};
use super::{LayoutInputError, fits, in_byte_order, is_range_edge};
use crate::frontend::{
    BoxDecorationBreak, BoxNode, BreakAction, DiscretionaryNode, GlueKind, GlueNode,
    HyphenOpportunity, InlineDecorationSpan, InlineObjectBox, MAX_DECORATION_NESTING_DEPTH,
    ParagraphAst, SpanEdge, TextBox, WordNode, is_word_space,
};
use crate::middleend::tolerance::{approx_eq, approx_eq_f64, tolerance};
use crate::types::{BidiClass, BidiLevel, TextRange, is_extent, is_measurement};
use std::cell::OnceCell;

/// Every item fits inside `text`. Run before indexing spans against items, so
/// the index is built on ordered ranges and cannot scan or clone unboundedly
/// for a malformed AST; the item checks may assume it.
pub(super) fn check_items_fit(ast: &ParagraphAst, text: &str) -> Result<(), LayoutInputError> {
    match ast
        .items
        .iter()
        .position(|node| !fits(text, node.byte_span()))
    {
        Some(item) => Err(LayoutInputError::ItemOutsideText { item }),
        None => Ok(()),
    }
}

/// Each item on its own, in item order.
pub(super) fn check_each_item(
    ast: &ParagraphAst,
    text: &str,
    boxes: &TextBoxes<'_>,
    ids: &DecorationIds,
    spans: &SpanIndex,
) -> Result<(), LayoutInputError> {
    let mut checks = ItemChecks::new(ast, text, boxes, ids, spans);
    for (item, node) in ast.items.iter().enumerate() {
        checks.check(item, node)?;
    }
    Ok(())
}

/// `items` ends with the paragraph-end sentinel.
pub(super) fn check_paragraph_end(ast: &ParagraphAst) -> Result<(), LayoutInputError> {
    match ast.items.last() {
        Some(WordNode::Discretionary(d)) if d.is_paragraph_end() => Ok(()),
        _ => Err(LayoutInputError::MissingParagraphEnd),
    }
}

/// Monotone progression: each item starts at or after the end of the previous
/// one (zero-width nodes may share an offset). Among the zero-width nodes at one
/// offset a `CjkAutospace` glue precedes every break opportunity, as a
/// breakable space's glue precedes its discretionary (so the spacing stays on
/// the line that ends there): `[Glue(CjkAutospace), Discretionary]`.
pub(super) fn check_item_order(ast: &ParagraphAst) -> Result<ItemsInOrder, LayoutInputError> {
    let out_of_order = |w: &[WordNode]| w[0].byte_span().end() > w[1].byte_span().start();
    if let Some(i) = ast.items.windows(2).position(out_of_order) {
        return Err(LayoutInputError::ItemsOutOfOrder { item: i + 1 });
    }
    // Offset of the break opportunity seen in the current run of zero-width
    // nodes, if any.
    let mut break_opportunity_offset: Option<usize> = None;
    for (item, node) in ast.items.iter().enumerate() {
        let range = node.byte_span();
        if !range.is_empty() || break_opportunity_offset != Some(range.start()) {
            break_opportunity_offset = None;
        }
        if node.is_break_opportunity() && range.is_empty() {
            break_opportunity_offset = Some(range.start());
        } else if matches!(node, WordNode::Glue(glue) if glue.kind == GlueKind::CjkAutospace)
            && break_opportunity_offset == Some(range.start())
        {
            return Err(LayoutInputError::ItemsOutOfOrder { item });
        }
    }
    Ok(ItemsInOrder(()))
}

/// Proof from [`check_item_order`] that the items' byte spans ascend (the
/// atoms inside a box are proved in order later, by `check_each_item`).
pub(super) struct ItemsInOrder(());

/// Byte bounds of the **hyphen chain** of every text box, indexed by item
/// (entries of other items are unspecified). A chain is a maximal sequence of
/// text boxes joined by hyphenation seams ([`ParagraphAst::hyphen_seam`]):
/// `TextShaper::reshape_window` takes one `ShapingRun`, and a box cannot
/// straddle a span boundary.
///
/// Built in two passes over the items, so the cost is linear however long a
/// chain is.
fn hyphen_chains(ast: &ParagraphAst, boxes: &TextBoxes<'_>) -> Vec<(usize, usize)> {
    let len = ast.items.len();
    let mut chains: Vec<(usize, usize)> = (0..len)
        .map(|item| {
            boxes.get(item).map_or((0, 0), |view| {
                (view.byte_range.start(), view.byte_range.end())
            })
        })
        .collect();
    for item in 2..len {
        if ast.hyphen_seam(item - 1).is_some() {
            chains[item].0 = chains[item - 2].0;
        }
    }
    for item in (0..len.saturating_sub(2)).rev() {
        if ast.hyphen_seam(item + 1).is_some() {
            chains[item].1 = chains[item + 2].1;
        }
    }
    chains
}

/// A discretionary's break widths net of its cloned spans' padding.
#[derive(Clone, Copy)]
struct NetWidths {
    /// Net of `pre_break_width`.
    pre: f64,
    /// Net of `post_break_width`.
    post: f64,
}

/// Everything the per-item checks share.
struct ItemChecks<'a> {
    ast: &'a ParagraphAst,
    text: &'a str,
    boxes: &'a TextBoxes<'a>,
    ids: &'a DecorationIds,
    spans: &'a SpanIndex,
    /// Hyphen-chain bounds per item ([`hyphen_chains`]), built lazily on the
    /// first hyphenation discretionary.
    chains: OnceCell<Vec<(usize, usize)>>,
    /// Per run, whether its atoms are in byte order ([`in_byte_order`]), built
    /// lazily on the first hyphenation discretionary.
    runs_in_byte_order: OnceCell<Vec<bool>>,
    /// Index of the last discretionary, where the sentinel belongs.
    last_break: Option<usize>,
    /// The font size of the run of the last text box checked (WN-13).
    last_text_font_size: Option<f32>,
}

impl<'a> ItemChecks<'a> {
    fn new(
        ast: &'a ParagraphAst,
        text: &'a str,
        boxes: &'a TextBoxes<'a>,
        ids: &'a DecorationIds,
        spans: &'a SpanIndex,
    ) -> Self {
        Self {
            ast,
            text,
            boxes,
            ids,
            spans,
            chains: OnceCell::new(),
            runs_in_byte_order: OnceCell::new(),
            last_break: ast.items.iter().rposition(WordNode::is_break_opportunity),
            last_text_font_size: None,
        }
    }

    /// The bidi level `node` carries, if it carries one.
    fn bidi_level(&self, node: &WordNode) -> Option<BidiLevel> {
        match node {
            WordNode::Box(BoxNode::Text(TextBox { run_index, .. })) => {
                Some(self.ast.runs[*run_index].bidi_level)
            }
            WordNode::Box(
                BoxNode::InlineObject(InlineObjectBox { bidi_level, .. })
                | BoxNode::SpanBoundary { bidi_level, .. },
            )
            | WordNode::Glue(GlueNode { bidi_level, .. }) => Some(*bidi_level),
            WordNode::Discretionary(_) | WordNode::Tab { .. } => None,
        }
    }

    /// Checks `items[item]` on its own. Items are checked in order.
    fn check(&mut self, item: usize, node: &WordNode) -> Result<(), LayoutInputError> {
        // UAX #9 never resolves a level below the paragraph's (AST-13).
        if self
            .bidi_level(node)
            .is_some_and(|level| level < self.ast.base_direction.base_level())
        {
            return Err(LayoutInputError::InvalidBidiLevel { item });
        }
        match node {
            WordNode::Box(BoxNode::Text(text)) => {
                let run = &self.ast.runs[text.run_index];
                self.last_text_font_size = Some(run.font_size);
                // Resolved from the matched box rather than read back from
                // `boxes` by index, which would need an `Option` for a fact
                // the match already states (pipeline rules 5 and 6).
                let view = &self.ast.resolve(text);
                let base = view.atom_start - run.atom_range.start;
                check_text_payload(self.text, run, view, &self.ast.glyphs, base, item)
            }
            WordNode::Box(BoxNode::InlineObject(InlineObjectBox {
                baseline_align,
                byte_range,
                width,
                ascent,
                descent,
                ..
            })) => {
                // An object owns the bytes carets and selections address it by.
                if byte_range.is_empty()
                    || !baseline_align.is_valid()
                    || ![*width, *ascent, *descent].into_iter().all(is_extent)
                {
                    return Err(LayoutInputError::InvalidInlineBox { item });
                }
                Ok(())
            }
            WordNode::Box(BoxNode::SpanBoundary {
                byte_range,
                span_id,
                edge,
                width,
                bidi_level,
            }) => self.check_span_boundary(item, *byte_range, *span_id, *edge, *width, *bidi_level),
            WordNode::Glue(glue) => self.check_glue(item, glue),
            WordNode::Discretionary(d) => self.check_discretionary(item, d),
            WordNode::Tab { byte_range } => {
                // A tab is source text; its geometric width is resolved later.
                if byte_range.is_empty() {
                    return Err(LayoutInputError::InvalidTab { item });
                }
                Ok(())
            }
        }
    }

    /// A span boundary: an empty range at its span's start (`Leading`) or end
    /// (`Trailing`), as wide as that edge's padding, at the bidi level of the
    /// first or last atom inside the span.
    fn check_span_boundary(
        &self,
        item: usize,
        byte_range: TextRange,
        span_id: u64,
        edge: SpanEdge,
        width: f32,
        bidi_level: BidiLevel,
    ) -> Result<(), LayoutInputError> {
        use LayoutInputError as E;
        if !(byte_range.is_empty() && is_extent(width)) {
            return Err(E::InvalidSpanBoundary { item });
        }
        let Some(span_index) = self.ids.index_of(span_id) else {
            return Err(E::UnknownSpan { item });
        };
        let span = &self.ast.decoration_spans[span_index];
        let base = self.ast.base_direction.base_level();
        let (first_level, last_level) = self.spans.levels[span_index];
        let (anchor, padding, level) = match edge {
            SpanEdge::Leading => (span.byte_range.start(), span.start_padding, first_level),
            SpanEdge::Trailing => (span.byte_range.end(), span.end_padding, last_level),
        };
        if byte_range.start() != anchor
            || !approx_eq(width, padding)
            || level.unwrap_or(base) != bidi_level
        {
            return Err(E::InvalidSpanBoundary { item });
        }
        Ok(())
    }

    /// Glue: non-negative measurements (MSR-1); the compression pool is
    /// shrink only and shrinks at most half the font size of the run of the
    /// nearest text box before it (WN-13); every glue is collapsible; and the
    /// node matches its source and its original class (WN-14). A `WordSpace`
    /// owns a non-empty run of word-space characters (a no-break space is
    /// text, WN-12) and is class WS; the synthetic kinds own no bytes and
    /// are class BN (WN-10).
    fn check_glue(&self, item: usize, glue: &GlueNode) -> Result<(), LayoutInputError> {
        let sizes_are_valid = [glue.natural_width, glue.max_stretch, glue.max_shrink]
            .into_iter()
            .all(is_extent);
        let pool_is_valid = glue.kind != GlueKind::CjkPunctCompression
            || (glue.natural_width == 0.0
                && glue.max_stretch == 0.0
                && self.last_text_font_size.is_some_and(|size| {
                    let bound = 0.5 * size;
                    glue.max_shrink <= bound + tolerance(glue.max_shrink, bound)
                }));
        let matches_source = if glue.kind.is_synthetic() {
            glue.byte_range.is_empty() && glue.original_bidi_class == BidiClass::Bn
        } else {
            // Every item was checked to fit `text` before the item checks.
            let source = &self.text[glue.byte_range.as_range()];
            glue.original_bidi_class == BidiClass::Ws
                && !source.is_empty()
                && source.chars().all(is_word_space)
        };
        if sizes_are_valid && pool_is_valid && glue.is_collapsible && matches_source {
            Ok(())
        } else {
            Err(LayoutInputError::InvalidGlue { item })
        }
    }

    /// A discretionary: its penalty and widths, the bytes it owns, the cloned
    /// spans it lists and the widths its action implies.
    fn check_discretionary(
        &self,
        item: usize,
        d: &DiscretionaryNode,
    ) -> Result<(), LayoutInputError> {
        let invalid = LayoutInputError::InvalidDiscretionary { item };
        if !self.discretionary_is_well_formed(item, d) {
            return Err(invalid);
        }
        let net = self.net_break_widths(item, d)?;
        self.check_break_action(item, d, net)
    }

    /// The discretionary's own fields: finite widths, a penalty in range, a
    /// mandatory break exactly when hard, the bytes it owns and unique
    /// cloned spans.
    fn discretionary_is_well_formed(&self, item: usize, d: &DiscretionaryNode) -> bool {
        let widths_are_finite = [d.pre_break_width, d.post_break_width, d.no_break_width]
            .into_iter()
            .all(is_measurement);
        let is_hard = matches!(d.action, BreakAction::HardBreak);
        // Ownership: a soft opportunity owns nothing; a hard break in the text
        // owns exactly its U+2028; the paragraph-end sentinel is the empty
        // range at the end of the text, on the last discretionary.
        let owns_its_bytes = if d.is_paragraph_end() {
            d.byte_range == TextRange::empty_at(self.text.len()) && Some(item) == self.last_break
        } else if is_hard {
            d.byte_range.slice(self.text) == Some("\u{2028}")
        } else {
            d.byte_range.is_empty()
        };
        let clones_are_unique = d.cloned_spans.len() <= MAX_DECORATION_NESTING_DEPTH
            && d.cloned_spans
                .iter()
                .enumerate()
                .all(|(i, id)| !d.cloned_spans[..i].contains(id));
        widths_are_finite
            && d.penalty >= DiscretionaryNode::MANDATORY_BREAK_PENALTY
            && d.penalty < DiscretionaryNode::FORBIDDEN_BREAK_PENALTY
            && is_hard == d.is_mandatory()
            && (!is_hard || d.no_break_width == 0.0)
            && owns_its_bytes
            && clones_are_unique
    }

    /// The break widths net of cloned padding. `cloned_spans` is exactly the
    /// set of `Clone` spans the break falls strictly inside: a chip split
    /// without its cloned padding is wrong, and so is cloning a span the
    /// break does not cut. Line `k` gains each listed span's `end_padding`,
    /// line `k + 1` its `start_padding`; what remains of the widths is the
    /// action's own delta.
    fn net_break_widths(
        &self,
        item: usize,
        d: &DiscretionaryNode,
    ) -> Result<NetWidths, LayoutInputError> {
        use LayoutInputError as E;
        let offset = d.byte_offset();
        let is_cut = |span: &InlineDecorationSpan| {
            span.break_mode == BoxDecorationBreak::Clone
                && span.byte_range.start() < offset
                && offset < span.byte_range.end()
        };
        let spans = &self.ast.decoration_spans;
        if self
            .spans
            .touching(item)
            .iter()
            .any(|&i| is_cut(&spans[i]) && !d.cloned_spans.contains(&spans[i].span_id))
        {
            return Err(E::InvalidDiscretionary { item });
        }
        let (mut clone_pre, mut clone_post) = (0.0_f64, 0.0_f64);
        for id in &d.cloned_spans {
            let Some(span) = self
                .ids
                .index_of(*id)
                .map(|i| &spans[i])
                .filter(|s| is_cut(s))
            else {
                return Err(E::UnknownSpan { item });
            };
            clone_pre += f64::from(span.end_padding);
            clone_post += f64::from(span.start_padding);
        }
        Ok(NetWidths {
            pre: f64::from(d.pre_break_width) - clone_pre,
            post: f64::from(d.post_break_width) - clone_post,
        })
    }

    /// The widths the break's action implies.
    fn check_break_action(
        &self,
        item: usize,
        d: &DiscretionaryNode,
        net: NetWidths,
    ) -> Result<(), LayoutInputError> {
        use LayoutInputError as E;
        let invalid = Err(E::InvalidDiscretionary { item });
        let is_zero = |net: f64| approx_eq_f64(net, 0.0);
        match &d.action {
            // A hyphenation's net deltas are its reshaping deltas.
            BreakAction::Hyphenate(opportunity) => {
                if !d.flagged || !self.hyphen_is_valid(item, opportunity) {
                    return Err(E::InvalidHyphenOpportunity { item });
                }
            }
            // At least one net delta is real; each needs its neighbouring
            // text box and may not drive that edge cluster's width below zero.
            BreakAction::CjkPunctAdjust => {
                let fits_cluster = |net: f64, width: Option<f32>| {
                    is_zero(net)
                        || width
                            .is_some_and(|w| f64::from(w) + net >= -f64::from(tolerance(w, 0.0)))
                };
                let trailing = item
                    .checked_sub(1)
                    .and_then(|i| self.boxes.get(i)?.clusters.last().map(|c| c.natural_width));
                let leading = self
                    .boxes
                    .get(item + 1)
                    .and_then(|v| v.clusters.first().map(|c| c.natural_width));
                if (is_zero(net.pre) && is_zero(net.post))
                    || !fits_cluster(net.pre, trailing)
                    || !fits_cluster(net.post, leading)
                {
                    return invalid;
                }
            }
            BreakAction::Direct | BreakAction::HardBreak => {
                if !(is_zero(net.pre) && is_zero(net.post)) {
                    return invalid;
                }
            }
            // The repeated operator opens the next line.
            BreakAction::MathOperator { repeat } => {
                let advance = repeat.map_or(0.0, |operator| operator.advance);
                if !(is_extent(advance)
                    && is_zero(net.pre)
                    && approx_eq_f64(net.post, f64::from(advance)))
                {
                    return invalid;
                }
            }
        }
        Ok(())
    }

    /// Whether the hyphenation discretionary at `items[item]` is well formed.
    fn hyphen_is_valid(&self, item: usize, opportunity: &HyphenOpportunity) -> bool {
        let has_line_break = |s: &str| {
            s.chars()
                .any(crate::frontend::is_line_or_paragraph_separator)
        };
        if opportunity
            .prefix_replacement
            .as_deref()
            .is_some_and(has_line_break)
            || opportunity
                .suffix_replacement
                .as_deref()
                .is_some_and(has_line_break)
        {
            return false;
        }
        let Some(before) = self.ast.hyphen_seam(item).map(|seam| seam.before) else {
            return false;
        };
        let offset = before.byte_range.end();
        let (Some(prefix_start), Some(suffix_end)) = (
            offset.checked_sub(opportunity.prefix_cut_bytes as usize),
            offset.checked_add(opportunity.suffix_cut_bytes as usize),
        ) else {
            return false;
        };
        let chains = self
            .chains
            .get_or_init(|| hyphen_chains(self.ast, self.boxes));
        let runs_in_byte_order = self.runs_in_byte_order.get_or_init(|| {
            (0..self.ast.runs.len())
                .map(|run| in_byte_order(self.ast.run_atoms(run).iter().map(|a| a.byte_range)))
                .collect()
        });
        let atoms = self.ast.run_atoms(before.run_index);
        prefix_start >= chains[item - 1].0
            && suffix_end <= chains[item + 1].1
            // See `in_byte_order`: unordered atoms are reported by the box.
            && (!runs_in_byte_order[before.run_index]
                || (is_range_edge(atoms, |a| a.byte_range, prefix_start)
                    && is_range_edge(atoms, |a| a.byte_range, suffix_end)))
    }
}
