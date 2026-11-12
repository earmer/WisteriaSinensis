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
        let mut word = |run: usize, at: usize| push_text(a, run, vec![atom(at)], vec![cluster(0)]);
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
    if let WordNode::Box(BoxNode::Text { cluster_range, .. }) = &mut a.items[0] {
        *cluster_range = 0..99;
    }
    assert!(a.text_box(0).is_none());
    assert!(!a.is_self_contained(0));
}

#[test]
fn a_cluster_range_that_is_not_run_local_is_not_self_contained() {
    let mut a = two_runs();
    // Item 3 is the second box of run 1; paragraph-level indices 3..4 are wrong
    // for its cluster, which must say 1..2: the box does not resolve, and the
    // run's clusters no longer partition its atoms.
    a.clusters[3].atom_range = 3..4;
    assert!(!a.is_self_contained(3));
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
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
    // Run 0 keeps one cluster for its two atoms: found before the gap it
    // leaves for run 1.
    let mut a = two_runs();
    a.runs[0].cluster_range = 0..1;
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 0 })
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
    // A box with no cluster owns nothing.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { cluster_range, .. }) = &mut a.items[1] {
        *cluster_range = 2..2;
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
    // A box that gives up its cluster is empty, which no text box may be.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { cluster_range, .. }) = &mut a.items[3] {
        *cluster_range = 3..3;
    }
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 3 })
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
    let mut out = vec![cluster(0)];
    let mut out_glyphs = Vec::new();
    NoopShaper.reshape_window(
        TEXT,
        run,
        atoms,
        TextRange::new(2, 4),
        LeadingEdit::None,
        TrailingEdit::None,
        &mut out,
        &mut out_glyphs,
    );
    // The no-op shaper appends nothing and never clears the buffer.
    assert_eq!(out, vec![cluster(0)]);
    assert!(out_glyphs.is_empty());
}

#[test]
fn a_box_reaching_into_the_next_run_is_not_self_contained() {
    // Box 1 (run 0) also claims run 1's first atom and cluster, with byte and
    // atom ranges that would be consistent if the run did not end at 2.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { cluster_range, .. }) = &mut a.items[1] {
        *cluster_range = 1..3;
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
    // An inverted range cannot be built at all.
    assert_eq!(TextRange::try_new(4, 2), None);
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
    // A box's range.
    let mut a = two_runs();
    if let WordNode::Box(BoxNode::Text { cluster_range, .. }) = &mut a.items[3] {
        *cluster_range = inverted(4, 3);
    }
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::ItemNotSelfContained { item: 3 })
    );
    // A cluster's run-local atoms: the run's clusters no longer partition them.
    let mut a = two_runs();
    a.clusters[3].atom_range = inverted(2, 1);
    assert_eq!(
        check(&a, TEXT),
        Err(LayoutInputError::InvalidRunStorage { run: 1 })
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

#[test]
fn cluster_glyph_ranges_tile_the_paragraph_glyphs() {
    let a = ast("abc");
    assert_eq!(a.glyphs.len(), 3);
    for (i, c) in a.clusters.iter().enumerate() {
        assert_eq!(c.glyph_range, i..i + 1);
        assert_eq!(a.cluster_glyphs(c), &a.glyphs[i..i + 1]);
    }
    assert_eq!(check(&a, "abc"), Ok(()));
    let rejected = |edit: fn(&mut ParagraphAst)| {
        let mut a = ast("abc");
        edit(&mut a);
        check(&a, "abc")
    };
    let invalid = |cluster| Err(LayoutInputError::InvalidGlyphStorage { cluster });
    // A gap, an overlap and a range past the end.
    assert_eq!(rejected(|a| a.clusters[1].glyph_range = 2..2), invalid(1));
    assert_eq!(rejected(|a| a.clusters[1].glyph_range = 0..2), invalid(1));
    assert_eq!(rejected(|a| a.clusters[2].glyph_range = 2..4), invalid(2));
    // A cluster that drops its glyph leaves it to no one.
    assert_eq!(rejected(|a| a.clusters[0].glyph_range = 0..0), invalid(1));
    // A glyph no cluster owns.
    assert_eq!(
        rejected(|a| a.glyphs.push(RawShapedGlyph::default())),
        invalid(2)
    );
    // A cluster may render nothing.
    assert_eq!(
        rejected(|a| {
            a.clusters[1].glyph_range = 1..1;
            a.clusters[2].glyph_range = 1..2;
            a.glyphs.pop();
        }),
        Ok(())
    );
}

/// The paragraph-level index of the first cluster of the text box at `item`.
fn first_cluster(a: &ParagraphAst, item: usize) -> usize {
    let WordNode::Box(BoxNode::Text { cluster_range, .. }) = &a.items[item] else {
        panic!("items[{item}] is not a text box");
    };
    cluster_range.start
}

fn window(atoms: core::ops::Range<usize>, bytes: (usize, usize)) -> ReshapeWindow {
    ReshapeWindow {
        run_index: 0,
        atom_range: atoms,
        byte_range: TextRange::new(bytes.0, bytes.1),
    }
}

#[test]
fn a_taken_break_reshapes_the_chain_cells_its_cuts_touch() {
    // "xaaaa": the boxes "a" are items 2, 4, 6, 8 (bytes 1..5), joined by the
    // seams 3, 5, 7 at offsets 2, 3, 4.
    let chain = |prefix: u32, suffix: u32| {
        hyphen_chain_ast(4, move |offset, _| {
            if offset == 3 {
                (prefix, suffix)
            } else {
                (0, 0)
            }
        })
        .1
    };
    // Without a cut: the box on each side of the break.
    assert_eq!(chain(0, 0).break_window(5), Some(window(2..4, (2, 4))));
    // A cut inside the neighbouring box changes nothing.
    assert_eq!(chain(1, 1).break_window(5), Some(window(2..4, (2, 4))));
    // A cut that crosses an unchosen seam pulls the next cell in.
    assert_eq!(chain(2, 0).break_window(5), Some(window(1..4, (1, 4))));
    assert_eq!(chain(0, 2).break_window(5), Some(window(2..5, (2, 5))));
    assert_eq!(chain(2, 2).break_window(5), Some(window(1..5, (1, 5))));
    // A cut that leaves the chain has no window (and fails validation).
    let (text, escaping) =
        hyphen_chain_ast(4, |offset, _| if offset == 3 { (3, 0) } else { (0, 0) });
    assert_eq!(escaping.break_window(5), None);
    assert_eq!(
        check(&escaping, &text),
        Err(LayoutInputError::InvalidHyphenOpportunity { item: 5 })
    );
    // Only hyphenation seams have one.
    let a = chain(0, 0);
    for item in [0, 1, 2, 4, a.items.len() - 1, a.items.len()] {
        assert_eq!(a.break_window(item), None, "item {item}");
    }
}

#[test]
fn untaken_unsafe_seams_are_rejoined_in_merged_windows() {
    let (_, mut a) = hyphen_chain_ast(4, |_, _| (0, 0));
    assert!(a.rejoin_windows([3, 5, 7]).is_empty());
    // The seams before items 4 and 6 split a cluster.
    for item in [4, 6] {
        let cluster = first_cluster(&a, item);
        a.clusters[cluster].safe_to_concat_before = false;
    }
    // Each unsafe seam alone: the two boxes it joins.
    assert_eq!(a.rejoin_windows([3])[..], [window(1..3, (1, 3))]);
    assert_eq!(a.rejoin_windows([5])[..], [window(2..4, (2, 4))]);
    // Together they share the box between them: one window, in any order,
    // and the safe seam 7 adds nothing.
    for untaken in [[3, 5, 7], [7, 5, 3]] {
        assert_eq!(a.rejoin_windows(untaken)[..], [window(1..4, (1, 4))]);
    }
    // Windows that only touch stay apart: the seam between them is not
    // rejoined.
    let cluster = first_cluster(&a, 6);
    a.clusters[cluster].safe_to_concat_before = true;
    let cluster = first_cluster(&a, 8);
    a.clusters[cluster].safe_to_concat_before = false;
    assert_eq!(
        a.rejoin_windows([7, 3])[..],
        [window(1..3, (1, 3)), window(3..5, (3, 5))]
    );
}
