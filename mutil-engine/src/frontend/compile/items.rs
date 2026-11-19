//! The item sequence (spec §3 Step 6, FE-19, FE-27, FE-31, WN-17, WN-21,
//! AST-8).

use super::spacing::Seam;
use super::units::{Unit, Units};
use super::{Compiler, Shaped};
use crate::frontend::{
    BoxDecorationBreak, BoxNode, DiscretionaryNode, GlueKind, GlueNode, InlineDecorationSpan,
    InlineObjectBox, MAX_DECORATION_NESTING_DEPTH, SpanEdge, TextBox, WordNode, bidi,
};
use crate::types::{
    BidiClass, BidiLevel, FontInstanceRef, FontRef, GlyphOrientation, GraphemeAtom, ScriptTag,
    TextRange,
};
use crate::{ShapedCluster, ShapingRun};
use smallvec::SmallVec;

/// The storage a text box is closed over.
struct TextBoxes<'a> {
    runs: &'a [ShapingRun],
    clusters: &'a [ShapedCluster],
    atoms: &'a [GraphemeAtom],
}

impl TextBoxes<'_> {
    /// Closes the box that opened at atom `open`, if any, before `end_atom`.
    fn close(&self, open: Option<usize>, end_atom: usize, items: &mut Vec<WordNode>) {
        let Some(first) = open else {
            return;
        };
        let (runs, clusters) = (self.runs, self.clusters);
        // Box edges are cluster edges once CLU-8 has been applied.
        let run_index = runs.partition_point(|r| r.atom_range.end <= first);
        let run = &runs[run_index];
        let c0 = run.cluster_range.start
            + clusters[run.cluster_range.clone()]
                .partition_point(|c| run.atom_range.start + c.atom_range.end <= first);
        let cluster = c0
            + clusters[c0..run.cluster_range.end]
                .partition_point(|c| run.atom_range.start + c.atom_range.start < end_atom);
        let natural_width = ShapedCluster::total_width(&clusters[c0..cluster]);
        items.push(WordNode::Box(BoxNode::Text(TextBox {
            run_index,
            byte_range: TextRange::new(
                self.atoms[first].byte_range.start(),
                self.atoms[end_atom - 1].byte_range.end(),
            ),
            atom_range: first..end_atom,
            cluster_range: c0..cluster,
            natural_width,
        })));
    }
}

impl Compiler<'_> {
    /// Step 6: the item sequence (FE-19, FE-27, WN-17, WN-21).
    pub(super) fn assemble_items(
        &self,
        units: &Units,
        levels: &bidi::TextLevels,
        box_starts: &[bool],
        breaks_after: &[Option<i32>],
        seams: &[Seam],
        shaped: &Shaped,
    ) -> Vec<WordNode> {
        let mut space_cache: Vec<Option<f32>> = vec![None; self.styles.len()];
        let mut items: Vec<WordNode> = Vec::new();
        let boxes = TextBoxes {
            runs: &shaped.runs,
            clusters: &shaped.clusters,
            atoms: &units.atoms,
        };
        // The first atom of the open text box.
        let mut open: Option<usize> = None;
        let mut atom_end = 0;
        let mut seams = seams.iter().copied().peekable();
        let boundaries = SpanBoundaries::new(
            self.input.decoration_spans,
            self.span_edge_levels(units, levels),
        );
        boundaries.emit_leading(0, &mut items);
        for (i, &unit) in units.units.iter().enumerate() {
            let node = match unit {
                Unit::Atom(a) => {
                    if box_starts[a] {
                        boxes.close(open.take(), a, &mut items);
                        open = Some(a);
                    }
                    atom_end = a + 1;
                    None
                }
                Unit::Glue(range, style) => {
                    Some(self.glue_node(range, style, levels, &mut space_cache))
                }
                Unit::Tab(range) => Some(WordNode::Tab { byte_range: range }),
                Unit::HardBreak(range) => {
                    let mut node = DiscretionaryNode::hard_break(range);
                    boundaries.apply_clones(&mut node);
                    Some(WordNode::Discretionary(node))
                }
                Unit::Object(index) => Some(self.object_node(index, levels)),
            };
            if let Some(node) = node {
                boxes.close(open.take(), atom_end, &mut items);
                items.push(node);
            }
            let end = unit.range(&units.atoms, self.input.inline_objects).end();
            let has_trailing = boundaries.has_trailing_at(end);
            let has_leading = end < self.text.len() && boundaries.has_leading_at(end);
            let seam = seams.next_if(|seam| matches!(unit, Unit::Atom(a) if a == seam.after));
            if breaks_after[i].is_some() || has_trailing || has_leading || seam.is_some() {
                boxes.close(open.take(), atom_end, &mut items);
            }
            if has_trailing {
                boundaries.emit_trailing(end, &mut items);
            }
            // Synthetic glue follows the trailing boundaries and precedes the
            // discretionary and the leading ones (FE-31, AST-8).
            if let Some(seam) = seam {
                items.push(self.seam_node(seam, &units.atoms));
            }
            if let Some(penalty) = breaks_after[i] {
                let mut node = DiscretionaryNode::direct(end);
                node.penalty = penalty;
                boundaries.apply_clones(&mut node);
                items.push(WordNode::Discretionary(node));
            }
            if has_leading {
                boundaries.emit_leading(end, &mut items);
            }
        }
        boxes.close(open.take(), atom_end, &mut items);
        items.push(WordNode::Discretionary(DiscretionaryNode::paragraph_end(
            self.text.len(),
        )));
        items
    }

    /// A word space's glue: the style's space advance, stretching by half
    /// and shrinking by a third (FE-27).
    fn glue_node(
        &self,
        range: TextRange,
        style: usize,
        levels: &bidi::TextLevels,
        space_cache: &mut [Option<f32>],
    ) -> WordNode {
        let width = *space_cache[style].get_or_insert_with(|| self.glue_width(style));
        WordNode::Glue(GlueNode {
            original_bidi_class: BidiClass::Ws,
            bidi_level: levels.at(range.start()),
            is_collapsible: true,
            byte_range: range,
            kind: GlueKind::WordSpace,
            natural_width: width,
            max_stretch: width / 2.0,
            max_shrink: width / 3.0,
        })
    }

    /// An inline object's box, at the level of its bytes.
    fn object_node(&self, index: usize, levels: &bidi::TextLevels) -> WordNode {
        let o = &self.input.inline_objects[index];
        WordNode::Box(BoxNode::InlineObject(InlineObjectBox {
            baseline_align: o.baseline_align,
            byte_range: o.byte_range,
            box_id: o.box_id,
            width: o.width,
            ascent: o.ascent,
            descent: o.descent,
            overflow_policy: o.overflow_policy,
            bidi_level: levels.at(o.byte_range.start()),
        }))
    }

    /// The levels of each decoration span's leading and trailing boundary:
    /// those of the first and last inline entity inside it, a text atom or an
    /// inline object, else the base level (WN-8).
    fn span_edge_levels(
        &self,
        units: &Units,
        levels: &bidi::TextLevels,
    ) -> Vec<(BidiLevel, BidiLevel)> {
        let objects = self.input.inline_objects;
        let unit_range = |u: &Unit| u.range(&units.atoms, objects);
        let entity_level = |unit: &Unit| match *unit {
            Unit::Atom(a) => Some(units.atoms[a].bidi_level),
            Unit::Object(o) => Some(levels.at(objects[o].byte_range.start())),
            _ => None,
        };
        let base = levels.direction.base_level();
        self.input
            .decoration_spans
            .iter()
            .map(|span| {
                // Edges lie between units (AST-7).
                let r = span.byte_range;
                let first = units
                    .units
                    .partition_point(|u| unit_range(u).start() < r.start());
                let end = units
                    .units
                    .partition_point(|u| unit_range(u).start() < r.end());
                let inside = &units.units[first..end];
                (
                    inside.iter().find_map(entity_level).unwrap_or(base),
                    inside.iter().rev().find_map(entity_level).unwrap_or(base),
                )
            })
            .collect()
    }

    /// `max(0, space advance + word_spacing)` for the primary font of a style
    /// (FE-27, SHP-20). A space advance that is negative or not finite is the
    /// backend's fault (SHP-18, AX-3): it is passed on unchanged, for the input
    /// gate to reject as an invalid glue.
    fn glue_width(&self, style: usize) -> f32 {
        let s = &self.styles[style];
        let (font, _) = self.shaper.resolve_primary_font_metrics(
            &s.families,
            s.style,
            s.script.unwrap_or(ScriptTag::LATIN),
            &self.locale_of(style),
            s.font_size,
            s.features.variations(),
        );
        let advance = self.shaper.space_advance(
            FontInstanceRef {
                font: FontRef::unscaled(font),
                font_size: s.font_size,
                variations: s.features.variations(),
            },
            GlyphOrientation::Horizontal,
        );
        if !(advance.is_finite() && advance >= 0.0) {
            return advance;
        }
        let raw = advance + s.word_spacing;
        if raw.is_finite() { raw.max(0.0) } else { raw }
    }
}

/// The `SpanBoundary` items and `Clone` discretionary padding of the
/// decoration spans (WN-8, WN-21, AST-8). Every query is by offset: the
/// edges are unit edges (AST-7, checked by `check_span_edges`), the spans
/// nest, and they are ordered by start.
struct SpanBoundaries<'s> {
    spans: &'s [InlineDecorationSpan],
    /// The levels of each span's leading and trailing boundary (WN-8).
    edge_levels: Vec<(BidiLevel, BidiLevel)>,
    /// Span indices ordered by end offset, the inner span first among spans
    /// that end together.
    by_end: Vec<usize>,
    /// The span that directly encloses each span, if any.
    parent: Vec<Option<usize>>,
}

impl<'s> SpanBoundaries<'s> {
    fn new(spans: &'s [InlineDecorationSpan], edge_levels: Vec<(BidiLevel, BidiLevel)>) -> Self {
        let mut by_end: Vec<usize> = (0..spans.len()).collect();
        by_end.sort_by_key(|&i| (spans[i].byte_range.end(), core::cmp::Reverse(i)));
        let mut parent = Vec::with_capacity(spans.len());
        let mut open: SmallVec<[usize; MAX_DECORATION_NESTING_DEPTH]> = SmallVec::new();
        for (i, span) in spans.iter().enumerate() {
            while open
                .last()
                .is_some_and(|&top| spans[top].byte_range.end() <= span.byte_range.start())
            {
                open.pop();
            }
            parent.push(open.last().copied());
            open.push(i);
        }
        Self {
            spans,
            edge_levels,
            by_end,
            parent,
        }
    }

    fn node(&self, index: usize, edge: SpanEdge) -> WordNode {
        let span = &self.spans[index];
        let (leading, trailing) = self.edge_levels[index];
        let offset = match edge {
            SpanEdge::Leading => span.byte_range.start(),
            SpanEdge::Trailing => span.byte_range.end(),
        };
        WordNode::Box(BoxNode::SpanBoundary {
            byte_range: TextRange::empty_at(offset),
            span_id: span.span_id,
            edge,
            width: match edge {
                SpanEdge::Leading => span.start_padding,
                SpanEdge::Trailing => span.end_padding,
            },
            bidi_level: match edge {
                SpanEdge::Leading => leading,
                SpanEdge::Trailing => trailing,
            },
        })
    }

    /// The spans starting at `offset`, outer first.
    fn leading_at(&self, offset: usize) -> core::ops::Range<usize> {
        let start_of = |span: &InlineDecorationSpan| span.byte_range.start();
        self.spans.partition_point(|s| start_of(s) < offset)
            ..self.spans.partition_point(|s| start_of(s) <= offset)
    }

    /// The spans ending at `offset`, inner first.
    fn trailing_at(&self, offset: usize) -> &[usize] {
        let end_of = |&i: &usize| self.spans[i].byte_range.end();
        let first = self.by_end.partition_point(|i| end_of(i) < offset);
        let end = self.by_end.partition_point(|i| end_of(i) <= offset);
        &self.by_end[first..end]
    }

    fn has_leading_at(&self, offset: usize) -> bool {
        !self.leading_at(offset).is_empty()
    }

    fn has_trailing_at(&self, offset: usize) -> bool {
        !self.trailing_at(offset).is_empty()
    }

    /// Emits the spans starting at `offset`, outer first.
    fn emit_leading(&self, offset: usize, items: &mut Vec<WordNode>) {
        items.extend(
            self.leading_at(offset)
                .map(|span| self.node(span, SpanEdge::Leading)),
        );
    }

    /// Emits the spans ending at `offset`, inner first.
    fn emit_trailing(&self, offset: usize, items: &mut Vec<WordNode>) {
        items.extend(
            self.trailing_at(offset)
                .iter()
                .map(|&span| self.node(span, SpanEdge::Trailing)),
        );
    }

    /// Applies the `Clone` spans strictly containing `d`'s break offset, outer
    /// first.
    fn apply_clones(&self, d: &mut DiscretionaryNode) {
        let offset = d.byte_offset();
        // The last span that starts before the offset, then its ancestors:
        // every span that contains the offset is among them.
        let mut enclosing: SmallVec<[usize; MAX_DECORATION_NESTING_DEPTH]> = SmallVec::new();
        let mut next = self
            .spans
            .partition_point(|s| s.byte_range.start() < offset)
            .checked_sub(1);
        while let Some(span) = next {
            if offset < self.spans[span].byte_range.end() {
                enclosing.push(span);
            }
            next = self.parent[span];
        }
        for span in enclosing.iter().rev().map(|&s| &self.spans[s]) {
            if span.break_mode == BoxDecorationBreak::Clone {
                d.cloned_spans.push(span.span_id);
                d.pre_break_width += span.end_padding;
                d.post_break_width += span.start_padding;
                if !d.is_mandatory() {
                    d.penalty += DiscretionaryNode::CLONED_SPAN_PENALTY;
                }
            }
        }
    }
}
