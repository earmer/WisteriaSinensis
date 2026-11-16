//! The item sequence (spec §3 Step 6, FE-19, FE-27, WN-17, WN-21, AST-8).

use super::super::{
    BoxDecorationBreak, BoxNode, DiscretionaryNode, GlueKind, GlueNode, InlineDecorationSpan,
    MAX_DECORATION_NESTING_DEPTH, ParagraphAst, SpanEdge, WordNode,
};
use super::{Compiler, Unit};
use crate::types::{
    BidiClass, BidiLevel, FontInstanceRef, FontRef, GlyphOrientation, ScriptTag, TextRange,
};
use smallvec::SmallVec;

impl Compiler<'_> {
    /// Step 6: the item sequence (FE-19, FE-27, WN-17, WN-21).
    pub(super) fn assemble_items(
        &self,
        box_starts: &[bool],
        breaks_after: &[Option<i32>],
        ast: &mut ParagraphAst,
    ) {
        let objects = self.input.inline_objects;
        let runs = &ast.runs;
        let clusters = &ast.clusters;
        let atoms = &self.atoms;
        let mut space_cache: Vec<Option<f32>> = vec![None; self.styles.len()];
        let mut items: Vec<WordNode> = Vec::new();
        // The open text box: its first atom, plus monotonic run and cluster cursors.
        let mut open: Option<usize> = None;
        let mut run_cursor = 0usize;
        let mut cluster_cursor = 0usize;
        let mut close = |open: &mut Option<usize>, end_atom: usize, items: &mut Vec<WordNode>| {
            let Some(first) = open.take() else {
                return;
            };
            // Box edges are cluster edges once CLU-8 has been applied.
            while runs[run_cursor].atom_range.end <= first {
                run_cursor += 1;
                cluster_cursor = runs[run_cursor].cluster_range.start;
            }
            let run = &runs[run_cursor];
            while cluster_cursor < run.cluster_range.end
                && run.atom_range.start + clusters[cluster_cursor].atom_range.end <= first
            {
                cluster_cursor += 1;
            }
            let c0 = cluster_cursor;
            let mut natural_width = 0.0;
            while cluster_cursor < run.cluster_range.end
                && run.atom_range.start + clusters[cluster_cursor].atom_range.start < end_atom
            {
                natural_width += clusters[cluster_cursor].natural_width;
                cluster_cursor += 1;
            }
            let c1 = cluster_cursor;
            items.push(WordNode::Box(BoxNode::Text {
                run_index: run_cursor,
                byte_range: TextRange::new(
                    atoms[first].byte_range.start(),
                    atoms[end_atom - 1].byte_range.end(),
                ),
                atom_range: first..end_atom,
                cluster_range: c0..c1,
                natural_width,
            }));
        };
        let mut atom_end = 0;
        let mut boundaries =
            BoundarySweep::new(self.input.decoration_spans, self.span_edge_levels());
        boundaries.emit_leading(0, &mut items);
        for (i, unit) in self.units.iter().enumerate() {
            match *unit {
                Unit::Atom(a) => {
                    if box_starts[a] {
                        close(&mut open, a, &mut items);
                        open = Some(a);
                    }
                    atom_end = a + 1;
                }
                other => {
                    close(&mut open, atom_end, &mut items);
                    items.push(match other {
                        Unit::Glue(range, style) => {
                            let width =
                                *space_cache[style].get_or_insert_with(|| self.glue_width(style));
                            WordNode::Glue(GlueNode {
                                original_bidi_class: BidiClass::Ws,
                                bidi_level: self.levels.at(range.start()),
                                is_collapsible: true,
                                byte_range: range,
                                kind: GlueKind::WordSpace,
                                natural_width: width,
                                max_stretch: width / 2.0,
                                max_shrink: width / 3.0,
                            })
                        }
                        Unit::Tab(range) => WordNode::Tab { byte_range: range },
                        Unit::HardBreak(range) => {
                            let mut node = DiscretionaryNode::hard_break(range);
                            boundaries.apply_clones(&mut node);
                            WordNode::Discretionary(node)
                        }
                        Unit::Object(index) => {
                            let o = &objects[index];
                            WordNode::Box(BoxNode::InlineObject {
                                baseline_align: o.baseline_align,
                                byte_range: o.byte_range,
                                box_id: o.box_id,
                                width: o.width,
                                ascent: o.ascent,
                                descent: o.descent,
                                overflow_policy: o.overflow_policy,
                                bidi_level: self.levels.at(o.byte_range.start()),
                            })
                        }
                        Unit::Atom(_) => unreachable!("handled above"),
                    });
                }
            }
            let end = unit.range(atoms, objects).end();
            let has_trailing = boundaries.has_trailing_at(end);
            let has_leading = end < self.text.len() && boundaries.has_leading_at(end);
            if breaks_after[i].is_some() || has_trailing || has_leading {
                close(&mut open, atom_end, &mut items);
            }
            if has_trailing {
                boundaries.emit_trailing(end, &mut items);
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
        close(&mut open, atom_end, &mut items);
        items.push(WordNode::Discretionary(DiscretionaryNode::paragraph_end(
            self.text.len(),
        )));
        ast.items = items;
        ast.decoration_spans = self.input.decoration_spans.to_vec();
        ast.atomic_spans = self.input.atomic_spans.to_vec();
    }

    /// The levels of each decoration span's leading and trailing boundary:
    /// those of the first and last inline entity inside it, a text atom or an
    /// inline object, else the base level (WN-8).
    fn span_edge_levels(&self) -> Vec<(BidiLevel, BidiLevel)> {
        let atoms = &self.atoms;
        let objects = self.input.inline_objects;
        let base = self.levels.direction.base_level();
        self.input
            .decoration_spans
            .iter()
            .map(|span| {
                let r = span.byte_range;
                // Edges lie between units (AST-7), so an entity that starts
                // inside the span lies inside it.
                let first_atom = atoms
                    .get(atoms.partition_point(|a| a.byte_range.start() < r.start()))
                    .filter(|a| a.byte_range.start() < r.end())
                    .map(|a| (a.byte_range.start(), a.bidi_level));
                let first_object = objects
                    .get(objects.partition_point(|o| o.byte_range.start() < r.start()))
                    .filter(|o| o.byte_range.start() < r.end())
                    .map(|o| (o.byte_range.start(), self.levels.at(o.byte_range.start())));
                let last_atom = atoms[..atoms.partition_point(|a| a.byte_range.end() <= r.end())]
                    .last()
                    .filter(|a| a.byte_range.end() > r.start())
                    .map(|a| (a.byte_range.end(), a.bidi_level));
                let last_object = objects
                    [..objects.partition_point(|o| o.byte_range.end() <= r.end())]
                    .last()
                    .filter(|o| o.byte_range.end() > r.start())
                    .map(|o| (o.byte_range.end(), self.levels.at(o.byte_range.start())));
                let leading = first_atom
                    .into_iter()
                    .chain(first_object)
                    .min_by_key(|e| e.0);
                let trailing = last_atom.into_iter().chain(last_object).max_by_key(|e| e.0);
                (
                    leading.map_or(base, |e| e.1),
                    trailing.map_or(base, |e| e.1),
                )
            })
            .collect()
    }

    /// `max(0, space advance + word_spacing)` for the primary font of a style
    /// (FE-27, SHP-20), leaving non-finite shaper advances intact for the input
    /// gate.
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
        let raw = advance + s.word_spacing;
        if raw.is_finite() { raw.max(0.0) } else { raw }
    }
}

/// The `SpanBoundary` items and `Clone` discretionary padding of the
/// decoration spans in text order (WN-8, WN-21, AST-8), from one sweep: asked
/// at ascending offsets, among them every span edge (AST-7, checked by
/// `check_span_edges`), it emits leading boundaries outer first, trailing ones
/// inner first, and clones open `Clone` spans onto enclosed discretionaries.
struct BoundarySweep<'s> {
    spans: &'s [InlineDecorationSpan],
    /// The levels of each span's leading and trailing boundary (WN-8).
    edge_levels: Vec<(BidiLevel, BidiLevel)>,
    next: usize,
    /// Indices of the open spans, outermost first.
    open: SmallVec<[usize; MAX_DECORATION_NESTING_DEPTH]>,
}

impl<'s> BoundarySweep<'s> {
    fn new(spans: &'s [InlineDecorationSpan], edge_levels: Vec<(BidiLevel, BidiLevel)>) -> Self {
        Self {
            spans,
            edge_levels,
            next: 0,
            open: SmallVec::new(),
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

    fn has_leading_at(&self, offset: usize) -> bool {
        self.spans
            .get(self.next)
            .is_some_and(|span| span.byte_range.start() == offset)
    }

    fn has_trailing_at(&self, offset: usize) -> bool {
        self.open
            .last()
            .is_some_and(|&span| self.spans[span].byte_range.end() == offset)
    }

    /// Emits the spans starting at `offset`, outer first.
    fn emit_leading(&mut self, offset: usize, items: &mut Vec<WordNode>) {
        while self.has_leading_at(offset) {
            items.push(self.node(self.next, SpanEdge::Leading));
            self.open.push(self.next);
            self.next += 1;
        }
    }

    /// Emits the spans ending at `offset`, inner first.
    fn emit_trailing(&mut self, offset: usize, items: &mut Vec<WordNode>) {
        while self.has_trailing_at(offset) {
            let span = self
                .open
                .pop()
                .expect("a trailing edge closes an open span");
            items.push(self.node(span, SpanEdge::Trailing));
        }
    }

    /// Applies open `Clone` spans strictly containing `d`'s break offset.
    fn apply_clones(&self, d: &mut DiscretionaryNode) {
        let offset = d.byte_offset();
        for span in self.open.iter().map(|&s| &self.spans[s]) {
            if span.break_mode == BoxDecorationBreak::Clone
                && span.byte_range.start() < offset
                && offset < span.byte_range.end()
            {
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
