//! Cluster reconstruction shared by the shaping backends (spec SHP-13):
//! glyphs tagged with the atom holding their source position become clusters
//! of whole atoms.

use crate::types::{
    FontId, FontRef, GraphemeAtom, RawShapedGlyph, ShapedCluster, ShapingRun, TextRange,
};

/// A glyph-less cluster over one atom.
pub(crate) fn empty_cluster(
    run: &ShapingRun,
    atom: &GraphemeAtom,
    index: usize,
    font: FontId,
) -> ShapedCluster {
    ShapedCluster {
        orientation: run.orientation,
        byte_range: atom.byte_range,
        atom_range: index..index + 1,
        font: FontRef::unscaled(font),
        bidi_level: run.bidi_level,
        flags: atom.flags,
        glyph_range: 0..0,
        natural_width: 0.0,
        safe_to_break_before: true,
        safe_to_concat_before: true,
    }
}

/// Groups glyphs, each tagged with the atom holding its cluster value, into
/// clusters of whole atoms (spec SHP-13): a glyph that reaches back into an
/// earlier cluster merges the two, and an atom with no glyph of its own (a
/// ligature component) joins the cluster before it.
pub(crate) fn push_clusters(
    run: &ShapingRun,
    atoms: &[GraphemeAtom],
    range: core::ops::Range<usize>,
    font: FontRef,
    glyphs: &[(usize, RawShapedGlyph, bool, bool)],
    output_clusters: &mut Vec<ShapedCluster>,
    output_glyphs: &mut Vec<RawShapedGlyph>,
) {
    // Groups: (first atom, glyph indices).
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for (index, (atom, ..)) in glyphs.iter().enumerate() {
        let atom = *atom;
        match groups.last_mut() {
            Some((start, members)) if atom == *start => members.push(index),
            Some((start, _)) if atom > *start => groups.push((atom, vec![index])),
            Some(_) => {
                // Out of order: merge back until the group starts at or
                // before this atom.
                let mut merged = vec![index];
                while let Some((start, members)) = groups.pop() {
                    merged.extend(members);
                    if start <= atom {
                        merged.sort_unstable();
                        groups.push((start, merged));
                        break;
                    }
                    if groups.is_empty() {
                        merged.sort_unstable();
                        groups.push((atom.min(start), merged));
                        break;
                    }
                }
            }
            None => groups.push((atom, vec![index])),
        }
    }
    if groups.is_empty() {
        groups.push((range.start, Vec::new()));
    }
    groups[0].0 = range.start;
    for (k, (start, members)) in groups.iter().enumerate() {
        let end = groups.get(k + 1).map_or(range.end, |(next, _)| *next);
        let first = output_glyphs.len();
        output_glyphs.extend(members.iter().map(|&i| glyphs[i].1));
        let (unsafe_break, unsafe_concat) = members
            .first()
            .map_or((false, false), |&i| (glyphs[i].2, glyphs[i].3));
        let natural_width = members
            .iter()
            .map(|&i| glyphs[i].1.x_advance)
            .sum::<f32>()
            .max(0.0);
        output_clusters.push(ShapedCluster {
            orientation: run.orientation,
            byte_range: TextRange::new(
                atoms[*start].byte_range.start(),
                atoms[end - 1].byte_range.end(),
            ),
            atom_range: *start..end,
            font,
            bidi_level: run.bidi_level,
            flags: crate::types::AtomFlags::for_cluster(&atoms[*start..end]),
            glyph_range: first..output_glyphs.len(),
            natural_width,
            safe_to_break_before: !unsafe_break,
            safe_to_concat_before: !unsafe_break && !unsafe_concat,
        });
    }
}
