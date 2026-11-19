//! Cluster reconstruction and window-reshaping helpers shared by the shaping
//! backends (spec SHP-6, SHP-13).

use super::{LeadingEdit, ShapeOutput, TrailingEdit};
use crate::types::{
    AtomFlags, FontId, FontRef, GraphemeAtom, RawShapedGlyph, ShapedCluster, ShapingRun, TextRange,
};
use smallvec::SmallVec;

/// Appends one glyph-less cluster per atom in `atoms[range]`, or one merged
/// cluster for `HorizontalInVertical` (spec SHP-5).
pub(crate) fn push_empty_clusters(
    run: &ShapingRun,
    atoms: &[GraphemeAtom],
    range: core::ops::Range<usize>,
    font: FontId,
    output: &mut ShapeOutput,
) {
    if range.is_empty() {
        return;
    }
    let at = output.glyphs.len();
    if run.orientation == crate::types::GlyphOrientation::HorizontalInVertical {
        output.clusters.push(ShapedCluster {
            orientation: run.orientation,
            byte_range: TextRange::new(
                atoms[range.start].byte_range.start(),
                atoms[range.end - 1].byte_range.end(),
            ),
            atom_range: range.clone(),
            font: FontRef::unscaled(font),
            bidi_level: run.bidi_level,
            flags: AtomFlags::for_cluster(&atoms[range]),
            glyph_range: at..at,
            natural_width: 0.0,
            safe_to_break_before: true,
            safe_to_concat_before: true,
        });
        return;
    }
    for (offset, atom) in atoms[range.clone()].iter().enumerate() {
        let index = range.start + offset;
        output.clusters.push(ShapedCluster {
            orientation: run.orientation,
            byte_range: atom.byte_range,
            atom_range: index..index + 1,
            font: FontRef::unscaled(font),
            bidi_level: run.bidi_level,
            flags: atom.flags,
            glyph_range: at..at,
            natural_width: 0.0,
            safe_to_break_before: true,
            safe_to_concat_before: true,
        });
    }
}

/// One shaped glyph with what cluster reconstruction needs to place it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaggedGlyph {
    /// The atom holding the glyph's cluster value.
    pub(crate) atom: usize,
    /// The face the glyph's id belongs to.
    pub(crate) font: FontId,
    pub(crate) glyph: RawShapedGlyph,
    pub(crate) unsafe_break: bool,
    pub(crate) unsafe_concat: bool,
}

/// The atom of `atoms[range]` whose byte range starts last at or before
/// `byte`; the range's first atom when `byte` lies before all of them.
pub(crate) fn atom_containing(
    atoms: &[GraphemeAtom],
    range: core::ops::Range<usize>,
    byte: usize,
) -> usize {
    range.start
        + atoms[range]
            .partition_point(|a| a.byte_range.start() <= byte)
            .saturating_sub(1)
}

/// Groups glyphs into clusters of whole atoms (spec SHP-13): a glyph that
/// reaches back into an earlier cluster merges the two, and an atom with no
/// glyph of its own (a ligature component) joins the cluster before it.
///
/// A cluster has one face, its first glyph's (`fallback_font` when there is
/// no glyph at all). A later glyph of another face has no meaning in that
/// face, so it becomes the face's `.notdef` (glyph 0) with its advance kept
/// (spec SHP-5, SHP-24).
pub(crate) fn push_clusters(
    run: &ShapingRun,
    atoms: &[GraphemeAtom],
    range: core::ops::Range<usize>,
    glyphs: &[TaggedGlyph],
    fallback_font: FontId,
    output: &mut ShapeOutput,
) {
    if range.is_empty() {
        return;
    }
    let ShapeOutput {
        clusters: output_clusters,
        glyphs: output_glyphs,
    } = output;
    struct Group {
        min_atom: usize,
        max_atom: usize,
        glyphs: core::ops::Range<usize>,
    }

    // Group glyphs so `atom_range` partitions stay strictly monotonic and
    // non-overlapping (`prev.max_atom < next.min_atom`) with contiguous glyph
    // slices (`prev.glyphs.end == next.glyphs.start`).
    let mut groups: SmallVec<[Group; 8]> = SmallVec::new();
    if run.orientation == crate::types::GlyphOrientation::HorizontalInVertical {
        groups.push(Group {
            min_atom: range.start,
            max_atom: range.end - 1,
            glyphs: 0..glyphs.len(),
        });
    } else {
        for (index, tagged) in glyphs.iter().enumerate() {
            let atom = tagged.atom;
            match groups.last_mut() {
                Some(last) if atom >= last.min_atom && atom <= last.max_atom => {
                    last.glyphs.end = index + 1;
                }
                Some(last) if atom < last.min_atom => {
                    let mut min_atom = atom;
                    let mut max_atom = atom;
                    let mut members_start = index;
                    while let Some(prev) = groups.last() {
                        if prev.max_atom < atom {
                            break;
                        }
                        min_atom = min_atom.min(prev.min_atom);
                        max_atom = max_atom.max(prev.max_atom);
                        members_start = prev.glyphs.start;
                        groups.pop();
                    }
                    groups.push(Group {
                        min_atom,
                        max_atom,
                        glyphs: members_start..index + 1,
                    });
                }
                _ => groups.push(Group {
                    min_atom: atom,
                    max_atom: atom,
                    glyphs: index..index + 1,
                }),
            }
        }
        if groups.is_empty() {
            groups.push(Group {
                min_atom: range.start,
                max_atom: range.start,
                glyphs: 0..0,
            });
        }
        groups[0].min_atom = range.start;
    }
    for (k, group) in groups.iter().enumerate() {
        let start = group.min_atom;
        let end = groups.get(k + 1).map_or(range.end, |next| next.min_atom);
        let first = output_glyphs.len();
        let slice = &glyphs[group.glyphs.clone()];
        let font = slice.first().map_or(fallback_font, |g| g.font);
        output_glyphs.extend(slice.iter().map(|g| RawShapedGlyph {
            glyph_id: if g.font == font { g.glyph.glyph_id } else { 0 },
            ..g.glyph
        }));
        let (unsafe_break, unsafe_concat) = slice
            .first()
            .map_or((false, false), |g| (g.unsafe_break, g.unsafe_concat));
        let natural_width = slice
            .iter()
            .map(|g| g.glyph.x_advance)
            .sum::<f32>()
            .max(0.0);
        output_clusters.push(ShapedCluster {
            orientation: run.orientation,
            byte_range: TextRange::new(
                atoms[start].byte_range.start(),
                atoms[end - 1].byte_range.end(),
            ),
            atom_range: start..end,
            font: FontRef::unscaled(font),
            bidi_level: run.bidi_level,
            flags: AtomFlags::for_cluster(&atoms[start..end]),
            glyph_range: first..output_glyphs.len(),
            natural_width,
            safe_to_break_before: !unsafe_break,
            safe_to_concat_before: !unsafe_break && !unsafe_concat,
        });
    }
}

/// Atom bounds and edge replacement text computed for a `reshape_window` call.
pub(crate) struct WindowPlan<'a> {
    pub(crate) first: usize,
    pub(crate) end: usize,
    pub(crate) kept_start: usize,
    pub(crate) kept_end: usize,
    pub(crate) leading_text: &'a str,
    pub(crate) trailing_text: String,
}

/// Resolves the window atom bounds, kept range, and edge replacement text for
/// `reshape_window`; returns `None` when `window` covers no atoms.
pub(crate) fn plan_window<'a>(
    atoms: &[GraphemeAtom],
    window: TextRange,
    leading_edit: &'a LeadingEdit,
    trailing_edit: &'a TrailingEdit,
) -> Option<WindowPlan<'a>> {
    let first = atoms.partition_point(|a| a.byte_range.start() < window.start());
    let end = atoms.partition_point(|a| a.byte_range.end() <= window.end());
    if first >= end {
        return None;
    }
    let (leading_cut, leading_text) = match leading_edit {
        LeadingEdit::None => (0, ""),
        LeadingEdit::ReplaceSuffix {
            replacement,
            cut_bytes,
        } => (*cut_bytes as usize, &**replacement),
    };
    let (trailing_cut, trailing_text) = match trailing_edit {
        TrailingEdit::None => (0, String::new()),
        TrailingEdit::AppendHyphen { hyphen } => (0, hyphen.to_string()),
        TrailingEdit::ReplacePrefix {
            replacement,
            cut_bytes,
            hyphen,
        } => {
            let text = if replacement.is_empty() {
                hyphen.to_string()
            } else {
                let mut s = String::with_capacity(replacement.len() + hyphen.len_utf8());
                s.push_str(replacement);
                s.push(*hyphen);
                s
            };
            (*cut_bytes as usize, text)
        }
    };
    let kept_start = atoms
        .partition_point(|a| a.byte_range.start() < window.start().saturating_add(leading_cut))
        .clamp(first, end);
    let kept_end = atoms
        .partition_point(|a| a.byte_range.end() <= window.end().saturating_sub(trailing_cut))
        .clamp(kept_start, end);
    Some(WindowPlan {
        first,
        end,
        kept_start,
        kept_end,
        leading_text,
        trailing_text,
    })
}

/// Attaches `leading` glyphs to the start of `clusters[0]` and `trailing`
/// glyphs to the end of `clusters.last()` (spec SHP-6), shifting interior
/// glyph ranges.
pub(crate) fn attach_synthesized(
    clusters: &mut [ShapedCluster],
    glyphs: &mut Vec<RawShapedGlyph>,
    leading: Vec<RawShapedGlyph>,
    trailing: Vec<RawShapedGlyph>,
) {
    let width = |gs: &[RawShapedGlyph]| gs.iter().map(|g| g.x_advance).sum::<f32>();
    if !trailing.is_empty() {
        let w = width(&trailing);
        glyphs.extend(trailing);
        if let Some(last) = clusters.last_mut() {
            last.glyph_range.end = glyphs.len();
            last.natural_width = (last.natural_width + w).max(0.0);
        }
    }
    if !leading.is_empty() && !clusters.is_empty() {
        let w = width(&leading);
        let count = leading.len();
        let at = clusters[0].glyph_range.start;
        glyphs.splice(at..at, leading);
        clusters[0].glyph_range.end += count;
        clusters[0].natural_width = (clusters[0].natural_width + w).max(0.0);
        for cluster in &mut clusters[1..] {
            cluster.glyph_range =
                cluster.glyph_range.start + count..cluster.glyph_range.end + count;
        }
    }
}

/// The faces the empty clusters at the two cut edges of a window take: the
/// face of the shaped text beside each.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EdgeFonts {
    pub(crate) first: FontId,
    pub(crate) last: FontId,
}

/// The reshape-window skeleton both backends share (spec SHP-6): the cut
/// atoms keep empty clusters, so the window's atoms stay partitioned; the
/// kept atoms are shaped by `shape_kept`, which appends their clusters and
/// reports the edge faces and whatever `synthesize` needs; the edge clusters
/// then take those faces BEFORE `synthesize` shapes the edit text in them,
/// and the head is marked unsafe to concatenate (CLU-8) and, with a leading
/// edit, unsafe to break before.
pub(crate) fn finish_window<S>(
    run: &ShapingRun,
    atoms: &[GraphemeAtom],
    plan: &WindowPlan<'_>,
    output: &mut ShapeOutput,
    shape_kept: impl FnOnce(&mut ShapeOutput) -> (EdgeFonts, S),
    synthesize: impl FnOnce(S, EdgeFonts) -> (Vec<RawShapedGlyph>, Vec<RawShapedGlyph>),
) {
    let first_cluster = output.clusters.len();
    push_empty_clusters(
        run,
        atoms,
        plan.first..plan.kept_start,
        FontId::default(),
        output,
    );
    let kept_first = output.clusters.len();
    let (edges, shaped) = shape_kept(output);
    for cluster in &mut output.clusters[first_cluster..kept_first] {
        cluster.font = FontRef::unscaled(edges.first);
    }
    push_empty_clusters(run, atoms, plan.kept_end..plan.end, edges.last, output);
    let Some(head) = output.clusters.get_mut(first_cluster) else {
        return;
    };
    head.safe_to_concat_before = false;
    head.safe_to_break_before &= plan.leading_text.is_empty();
    let (leading, trailing) = synthesize(shaped, edges);
    attach_synthesized(
        &mut output.clusters[first_cluster..],
        &mut output.glyphs,
        leading,
        trailing,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        BaselineAlignment, BidiLevel, FontFeatures, FontStyle, GlyphOrientation, LineBreakClass,
        LocaleTag, ScriptTag, TextDecoration,
    };
    use std::sync::Arc;

    fn sample_run() -> ShapingRun {
        ShapingRun {
            baseline_align: BaselineAlignment::Auto,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            orientation: GlyphOrientation::Horizontal,
            features: FontFeatures::default(),
            byte_range: TextRange::new(0, 4),
            families: Arc::from([]),
            font_size: 16.0,
            style: FontStyle::default(),
            script: ScriptTag::LATIN,
            locale: LocaleTag::und(),
            bidi_level: BidiLevel::LTR,
            decoration: TextDecoration::default(),
            atom_range: 0..4,
            cluster_range: 0..0,
        }
    }

    fn sample_atoms(count: usize) -> Vec<GraphemeAtom> {
        (0..count)
            .map(|i| GraphemeAtom {
                orientation: GlyphOrientation::Horizontal,
                byte_range: TextRange::new(i, i + 1),
                script: ScriptTag::LATIN,
                bidi_level: BidiLevel::LTR,
                linebreak_class: LineBreakClass::Al,
                flags: AtomFlags::EMPTY,
            })
            .collect()
    }

    fn glyph(id: u32, advance: f32) -> RawShapedGlyph {
        RawShapedGlyph {
            glyph_id: id,
            x_advance: advance,
            x_offset: 0.0,
            y_offset: 0.0,
            is_synthesized: false,
        }
    }

    fn tag(
        atom: usize,
        glyph: RawShapedGlyph,
        unsafe_break: bool,
        unsafe_concat: bool,
    ) -> TaggedGlyph {
        TaggedGlyph {
            atom,
            font: FontId(1),
            glyph,
            unsafe_break,
            unsafe_concat,
        }
    }

    #[test]
    fn push_clusters_merges_out_of_order_and_empty_atom_ranges() {
        let run = sample_run();
        let atoms = sample_atoms(4);
        let tagged = [
            tag(0, glyph(10, 4.0), false, false),
            tag(2, glyph(20, 5.0), true, false),
            tag(3, glyph(30, 6.0), false, true),
            tag(1, glyph(40, 2.0), false, false),
        ];
        let mut output = ShapeOutput::default();
        push_clusters(&run, &atoms, 0..4, &tagged, FontId(1), &mut output);
        let ShapeOutput { clusters, glyphs } = output;
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].atom_range, 0..1);
        assert_eq!(clusters[0].glyph_range, 0..1);
        assert_eq!(clusters[1].atom_range, 1..4);
        assert_eq!(clusters[1].glyph_range, 1..4);
        assert_eq!(
            glyphs.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
            [10, 20, 30, 40]
        );
        assert!(!clusters[1].safe_to_break_before);
        assert!(!clusters[1].safe_to_concat_before);
    }

    #[test]
    fn push_clusters_merges_overlapping_out_of_order_spans() {
        let run = sample_run();
        let atoms = sample_atoms(4);

        // `[2, 0, 3, 1]`: `2` and `0` merge into `0..=2`, `3` starts `3..=3`,
        // then `1` falls inside `0..=2` and merges all four into `0..4`.
        let tagged = [
            tag(2, glyph(10, 1.0), false, false),
            tag(0, glyph(20, 2.0), false, false),
            tag(3, glyph(30, 3.0), false, false),
            tag(1, glyph(40, 4.0), false, false),
        ];
        let mut output = ShapeOutput::default();
        push_clusters(&run, &atoms, 0..4, &tagged, FontId(1), &mut output);
        assert_eq!(output.clusters.len(), 1);
        assert_eq!(output.clusters[0].atom_range, 0..4);
        assert_eq!(output.clusters[0].glyph_range, 0..4);

        // `[0, 3, 1, 2]`: `1` merges with `3` (`1..=3`), then `2` stays in
        // `1..=3` without splitting `0..1` or creating an overlapping range.
        let tagged = [
            tag(0, glyph(10, 1.0), false, false),
            tag(3, glyph(20, 2.0), false, false),
            tag(1, glyph(30, 3.0), false, false),
            tag(2, glyph(40, 4.0), false, false),
        ];
        output = ShapeOutput::default();
        push_clusters(&run, &atoms, 0..4, &tagged, FontId(1), &mut output);
        let clusters = output.clusters;
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].atom_range, 0..1);
        assert_eq!(clusters[0].glyph_range, 0..1);
        assert_eq!(clusters[1].atom_range, 1..4);
        assert_eq!(clusters[1].glyph_range, 1..4);
    }

    #[test]
    fn plan_window_clamps_overlapping_cuts_to_kept_start() {
        let atoms = sample_atoms(4);
        let leading = LeadingEdit::ReplaceSuffix {
            replacement: "x".into(),
            cut_bytes: 3,
        };
        let trailing = TrailingEdit::ReplacePrefix {
            replacement: "y".into(),
            cut_bytes: 3,
            hyphen: '-',
        };
        let plan = plan_window(&atoms, TextRange::new(0, 4), &leading, &trailing)
            .expect("window covers atoms");
        assert_eq!(plan.first, 0);
        assert_eq!(plan.end, 4);
        assert_eq!(plan.kept_start, 3);
        assert_eq!(plan.kept_end, 3);
        assert_eq!(plan.leading_text, "x");
        assert_eq!(plan.trailing_text, "y-");
    }

    #[test]
    fn attach_synthesized_updates_edges_and_interior_ranges() {
        let run = sample_run();
        let atoms = sample_atoms(3);
        let mut output = ShapeOutput::default();
        push_empty_clusters(&run, &atoms, 0..1, FontId(1), &mut output);
        push_clusters(
            &run,
            &atoms,
            1..2,
            &[tag(1, glyph(5, 3.0), false, false)],
            FontId(1),
            &mut output,
        );
        push_empty_clusters(&run, &atoms, 2..3, FontId(1), &mut output);
        let ShapeOutput {
            mut clusters,
            mut glyphs,
        } = output;

        let mut lead = glyph(100, 2.0);
        lead.is_synthesized = true;
        let mut trail = glyph(200, 1.5);
        trail.is_synthesized = true;
        attach_synthesized(&mut clusters, &mut glyphs, vec![lead], vec![trail]);

        assert_eq!(clusters[0].glyph_range, 0..1);
        assert_eq!(clusters[0].natural_width, 2.0);
        assert_eq!(clusters[1].glyph_range, 1..2);
        assert_eq!(clusters[2].glyph_range, 2..3);
        assert_eq!(clusters[2].natural_width, 1.5);
        assert_eq!(
            glyphs.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
            [100, 5, 200]
        );
    }
}
