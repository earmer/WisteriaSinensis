//! Paragraph-wide atom and cluster storage: runs tile it, text boxes slice it.

use crate::common::*;
use mutil_engine::*;

const TEXT: &str = "abcd";

/// "ab" in run 0 and "cd" in run 1, two boxes per run's slice of the storage:
/// items `[ "a", "b", "c", "d", end ]` over runs `[0, 0, 1, 1]`.
fn two_runs() -> ParagraphAst {
    let mut a = ast(TEXT);
    let second = ShapingRun {
        byte_range: TextRange::new(2, 4),
        ..a.runs[0].clone()
    };
    a.runs[0].byte_range = TextRange::new(0, 2);
    a.runs.push(second);
    rebuild_items(&mut a, |a| {
        let mut word =
            |run: usize, at: usize| push_text(a, run, vec![atom(at)], vec![cluster(TEXT, at, 0)]);
        vec![word(0, 0), word(0, 1), word(1, 2), word(1, 3), end(4)]
    });
    a
}

#[test]
fn well_formed_storage_is_accepted_and_resolves_per_run() {
    let a = two_runs();
    assert!(check(&a, TEXT).is_ok());
    assert_eq!((a.atoms.len(), a.clusters.len()), (4, 4));
    assert_eq!(a.runs[0].atom_range, 0..2);
    assert_eq!(a.runs[1].atom_range, 2..4);
    assert_eq!(a.runs[1].cluster_range, 2..4);
    // Run slices and box views come straight from the paragraph arrays.
    assert_eq!(a.run_atoms(1), &a.atoms[2..4]);
    assert_eq!(a.run_clusters(1), &a.clusters[2..4]);
    let view = a.text_box(3).unwrap();
    assert_eq!(view.run_index, 1);
    assert_eq!(view.byte_range, TextRange::new(3, 4));
    assert_eq!(view.atoms, &a.atoms[3..4]);
    // Cluster ranges are run-local, not paragraph-level: the last box of run 1
    // is atom 1 of the run's slice and atom 3 of the paragraph's.
    assert_eq!(view.clusters[0].atom_range, 1..2);
    let run = &a.runs[1];
    assert_eq!(
        a.atoms[run.atom_range.start + view.clusters[0].atom_range.start],
        view.atoms[0]
    );
    assert!((0..a.items.len()).all(|item| a.is_self_contained(item)));
}

#[test]
fn accessors_never_panic_on_bad_indices() {
    let mut a = two_runs();
    assert!(a.run_atoms(2).is_empty());
    assert!(a.run_clusters(2).is_empty());
    assert!(a.text_box(4).is_none(), "the sentinel is not a text box");
    assert!(a.text_box(99).is_none());
    assert!(
        a.is_self_contained(4),
        "other nodes are trivially self-contained"
    );
    assert!(!a.is_self_contained(99));
    // Ranges past the storage resolve to nothing.
    a.runs[1].atom_range = 2..99;
    a.runs[1].cluster_range = 2..99;
    assert!(a.run_atoms(1).is_empty());
    assert!(a.run_clusters(1).is_empty());
    if let WordNode::Box(BoxNode::Text { atom_range, .. }) = &mut a.items[0] {
        *atom_range = 0..99;
    }
    assert!(a.text_box(0).is_none());
    assert!(!a.is_self_contained(0));
}

#[test]
fn a_cluster_range_that_is_not_run_local_is_not_self_contained() {
    let mut a = two_runs();
    // Item 3 is the second box of run 1; paragraph-level indices 3..4 are wrong
    // for its cluster, which must say 1..2.
    a.clusters[3].atom_range = 3..4;
    assert!(!a.is_self_contained(3));
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 3 })
    );
}

#[test]
fn runs_must_tile_the_atom_storage() {
    // A gap between runs.
    let mut a = two_runs();
    a.runs[1].atom_range = 3..4;
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
    // An overlap.
    let mut a = two_runs();
    a.runs[1].atom_range = 1..4;
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
    // A range past the storage.
    let mut a = two_runs();
    a.runs[1].atom_range = 2..5;
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
    // Atoms no run owns.
    let mut a = two_runs();
    a.atoms.push(atom(3));
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
}

#[test]
fn runs_must_tile_the_cluster_storage() {
    let mut a = two_runs();
    a.runs[0].cluster_range = 0..1;
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
    let mut a = two_runs();
    a.runs[1].cluster_range = 1..4;
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
    let mut a = two_runs();
    a.clusters.pop();
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
}

#[test]
fn a_box_must_lie_inside_its_own_run() {
    // Box 2 holds run 1's atoms but names run 0.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { run_index, .. }) = &mut a.items[2] {
        *run_index = 0;
    }
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 2 })
    );
    // A box stretched past the end of its run.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { atom_range, .. }) = &mut a.items[1] {
        *atom_range = 1..3;
    }
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 1 })
    );
    // Clusters likewise.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { cluster_range, .. }) = &mut a.items[1] {
        *cluster_range = 1..3;
    }
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 1 })
    );
}

#[test]
fn the_boxes_of_a_run_take_consecutive_ranges_in_item_order() {
    let mut a = two_runs();
    // Exchange the two boxes of run 0: each still lies inside the run, but
    // the first in item order no longer starts where the run does.
    a.items.swap(0, 1);
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 0 })
    );
    // A gap inside the run: the second box skips an atom.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { atom_range, .. }) = &mut a.items[1] {
        *atom_range = 2..2;
    }
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 1 })
    );
}

#[test]
fn the_boxes_of_a_run_must_cover_all_its_atoms_and_clusters() {
    // Dropping the last box of run 1 leaves its atom and cluster unowned.
    let mut a = two_runs();
    a.items.remove(3);
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
    // Same when only the clusters are left over.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { cluster_range, .. }) = &mut a.items[3] {
        *cluster_range = 3..3;
    }
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
}

#[test]
fn an_annotation_run_indexes_its_own_storage_from_zero() {
    let mut a = ast("ab");
    a.annotations = vec![annotation(TextRange::new(0, 2), AnnotationSide::Over)];
    assert_eq!(a.annotations[0].run.atom_range, 0..1);
    assert_eq!(a.annotations[0].run.cluster_range, 0..1);
    assert!(check(&a, "ab").is_ok());
    // A range that is not local to the annotation's atoms and clusters.
    let mut moved = a.clone();
    moved.annotations[0].run.atom_range = 1..2;
    assert_eq!(
        check(&moved, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
    let mut moved = a.clone();
    moved.annotations[0].run.cluster_range = 1..2;
    assert_eq!(
        check(&moved, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
    let mut short = a;
    short.annotations[0].run.atom_range = 0..0;
    assert_eq!(
        check(&short, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
}

#[test]
fn reshape_window_takes_the_run_slice_of_the_paragraph_storage_directly() {
    let a = two_runs();
    let run = &a.runs[1];
    // O(1), no copy: the slice the shaper gets is the run's part of the arena.
    let atoms = &a.atoms[run.atom_range.clone()];
    assert_eq!(atoms.len(), 2);
    assert!(std::ptr::eq(atoms.as_ptr(), a.run_atoms(1).as_ptr()));
    // Output cluster ranges index this slice (run-local), as the stored ones do.
    assert!(
        a.run_clusters(1)
            .iter()
            .all(|c| c.atom_range.end <= atoms.len())
    );
    let mut out = vec![cluster(TEXT, 2, 0)];
    NoopShaper.reshape_window(
        TEXT,
        run,
        atoms,
        TextRange::new(2, 4),
        LeadingEdit::None,
        TrailingEdit::None,
        &mut out,
    );
    // The no-op shaper appends nothing and never clears the buffer.
    assert_eq!(out, vec![cluster(TEXT, 2, 0)]);
}

#[test]
fn a_box_reaching_into_the_next_run_is_not_self_contained() {
    // Box 1 (run 0) also claims run 1's first atom and cluster, with byte and
    // atom ranges that would be consistent if the run did not end at 2.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text {
        byte_range,
        atom_range,
        cluster_range,
        ..
    }) = &mut a.items[1]
    {
        (*byte_range, *atom_range, *cluster_range) = (TextRange::new(1, 3), 1..3, 1..3);
    }
    a.clusters[2].atom_range = 2..3;
    assert!(!a.is_self_contained(1));
}

#[test]
fn body_runs_progress_through_the_text_without_overlap() {
    // Touching runs ("ab" | "cd") are the baseline.
    assert!(check(&two_runs(), TEXT).is_ok());
    // The second run starts before the first one ends. Every box still lies
    // inside its own run, so only the order of the runs is wrong.
    let mut a = two_runs();
    a.runs[0].byte_range = TextRange::new(0, 3);
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidShapingRun { run: 1 })
    );
    // Runs out of text order.
    let mut a = two_runs();
    a.runs[0].byte_range = TextRange::new(2, 4);
    a.runs[1].byte_range = TextRange::new(0, 2);
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidShapingRun { run: 1 })
    );
    // An inverted range is not a range of the text at all.
    let mut a = two_runs();
    a.runs[1].byte_range = TextRange { start: 4, end: 2 };
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::RunOutsideText { run: 1 })
    );
    // An empty run is allowed anywhere between its neighbours.
    let mut a = two_runs();
    let empty = ShapingRun {
        byte_range: TextRange::empty_at(2),
        atom_range: 2..2,
        cluster_range: 2..2,
        ..a.runs[0].clone()
    };
    a.runs.insert(1, empty);
    for item in &mut a.items {
        if let WordNode::Box(BoxNode::Text { run_index, .. }) = item {
            *run_index += usize::from(*run_index >= 1);
        }
    }
    assert!(check(&a, TEXT).is_ok());
}

#[test]
fn inverted_index_ranges_are_rejected_rather_than_read_as_empty() {
    let inverted = |from: usize, to: usize| core::ops::Range {
        start: from,
        end: to,
    };
    // A run's own ranges.
    let mut a = two_runs();
    a.runs[1].atom_range = inverted(4, 2);
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
    let mut a = two_runs();
    a.runs[1].cluster_range = inverted(4, 2);
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
    );
    // A box's ranges.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { atom_range, .. }) = &mut a.items[3] {
        *atom_range = inverted(4, 3);
    }
    assert!(a.text_box(3).is_none());
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 3 })
    );
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { cluster_range, .. }) = &mut a.items[3] {
        *cluster_range = inverted(4, 3);
    }
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 3 })
    );
    // A cluster's run-local atoms.
    let mut a = two_runs();
    a.clusters[3].atom_range = inverted(2, 1);
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 3 })
    );
    // An annotation's own storage.
    let mut a = ast("ab");
    a.annotations = vec![annotation(TextRange::new(0, 2), AnnotationSide::Over)];
    a.annotations[0].run.atom_range = inverted(1, 0);
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
    let mut a = ast("ab");
    a.annotations = vec![annotation(TextRange::new(0, 2), AnnotationSide::Over)];
    a.annotations[0].clusters[0].atom_range = inverted(1, 0);
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidAnnotationSpan { annotation: 0 })
    );
}
