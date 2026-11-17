//! Self-containment of text payloads: the [`ShapedCluster`] invariants of a
//! text box or an annotation's text (spec §2.3, §2.4).

use super::{BoxNode, ParagraphAst, WordNode};
use crate::types::{AtomFlags, GraphemeAtom, ShapedCluster, TextRange};
use core::ops::Range;

/// Checks the invariants documented on [`ShapedCluster`] for one text payload
/// (a text box, or an annotation's whole text). `atoms` and `clusters` are the
/// payload's resolved slices; `base` is the run-local index of its first atom
/// (`box.atom_range.start - run.atom_range.start`), which converts each
/// cluster's run-local `atom_range` into an index of `atoms`. A payload without
/// atoms is not self-contained: every text box owns source bytes.
pub(crate) fn text_is_self_contained(
    byte_range: TextRange,
    atoms: &[GraphemeAtom],
    clusters: &[ShapedCluster],
    base: usize,
) -> bool {
    spans_atoms(byte_range, atoms)
        && atoms
            .windows(2)
            .all(|w| w[0].byte_range.end() == w[1].byte_range.start())
        && atoms
            .iter()
            .all(|a| a.byte_range.start() < a.byte_range.end())
        && clusters_partition_atoms(atoms, clusters, base)
}

/// `true` when `atoms` is non-empty and `byte_range` runs from the first
/// atom's start to the last atom's end.
fn spans_atoms(byte_range: TextRange, atoms: &[GraphemeAtom]) -> bool {
    let (Some(first), Some(last)) = (atoms.first(), atoms.last()) else {
        return false;
    };
    byte_range.start == first.byte_range.start && byte_range.end == last.byte_range.end
}

/// The clusters' run-local `atom_range`s tile `base..base + atoms.len()` in
/// order, each cluster spans the bytes of its atoms, and (invariant 3) its
/// flags are the per-flag aggregate of its atoms'.
fn clusters_partition_atoms(
    atoms: &[GraphemeAtom],
    clusters: &[ShapedCluster],
    base: usize,
) -> bool {
    let end = base + atoms.len();
    let mut next = base;
    for cluster in clusters {
        let range = &cluster.atom_range;
        if range.start != next || range.end <= range.start || range.end > end {
            return false;
        }
        let cluster_atoms = &atoms[range.start - base..range.end - base];
        if !spans_atoms(cluster.byte_range, cluster_atoms)
            || cluster.flags != AtomFlags::for_cluster(cluster_atoms)
        {
            return false;
        }
        next = range.end;
    }
    next == end
}

/// `true` when `inner` lies within `outer`.
fn range_within(inner: &Range<usize>, outer: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

impl ParagraphAst {
    /// Checks the invariants documented on [`ShapedCluster`] for the text box at
    /// `items[item]`: it has at least one atom, its atoms are contiguous and
    /// tile its `byte_range`, its clusters partition its atoms in order with
    /// matching byte ranges, and each cluster's flags aggregate its atoms'.
    /// Every other node is trivially self-contained; an `item` past the end,
    /// or a text box whose ranges do not resolve or leave its run's ranges,
    /// is not.
    #[must_use]
    pub fn is_self_contained(&self, item: usize) -> bool {
        let Some(node) = self.items.get(item) else {
            return false;
        };
        let WordNode::Box(BoxNode::Text {
            run_index,
            byte_range,
            atom_range,
            cluster_range,
            ..
        }) = node
        else {
            return true;
        };
        let (Some(run), Some(atoms), Some(clusters)) = (
            self.runs.get(*run_index),
            self.atoms.get(atom_range.clone()),
            self.clusters.get(cluster_range.clone()),
        ) else {
            return false;
        };
        range_within(atom_range, &run.atom_range)
            && range_within(cluster_range, &run.cluster_range)
            && text_is_self_contained(
                *byte_range,
                atoms,
                clusters,
                atom_range.start - run.atom_range.start,
            )
    }
}
