use super::spans::{check_boundary_order, check_decoration_spans};
use super::*;
use crate::frontend::RepeatedOperator;
use crate::frontend::*;
use crate::middleend::{LayoutConstraints, LayoutOptions};
use crate::middleend::{LineHeight, TabStops};
use crate::types::*;
use crate::types::{BidiDirection, FontMetrics, TextRange};

fn constraints() -> LayoutConstraints {
    LayoutConstraints {
        wrap_width: 100.0,
        tab_stops: TabStops::DEFAULT,
        ..Default::default()
    }
}

fn ast() -> ParagraphAst {
    ParagraphAst {
        annotations: Vec::new(),
        bidi_spans: Vec::new(),
        writing_mode: crate::types::WritingMode::HorizontalTb,
        base_direction: BidiDirection::Ltr,
        strut_metrics: FontMetrics {
            vertical_ascent: 5.0,
            vertical_descent: 5.0,
            vertical_line_gap: 0.0,
            ascent: 8.0,
            descent: 2.0,
            line_gap: 1.0,
            ..Default::default()
        },
        strut_font_size: 10.0,
        runs: vec![],
        atoms: vec![],
        clusters: vec![],
        glyphs: vec![],
        items: vec![sentinel(0)],
        decoration_spans: vec![],
        atomic_spans: vec![],
    }
}

/// Line separator break (U+2028).
fn hard_break() -> WordNode {
    WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(0, 3)))
}

/// The paragraph-end sentinel for a text of `len` bytes.
fn sentinel(len: usize) -> WordNode {
    WordNode::Discretionary(DiscretionaryNode::paragraph_end(len))
}

#[test]
fn the_policy_resolves_the_strut_from_raw_metrics_and_options() {
    let resolve = |options: &LayoutOptions| {
        validate_layout_inputs(&ast(), "", &constraints(), options)
            .unwrap()
            .policy
    };
    let normal = resolve(&LayoutOptions::default());
    assert_eq!(normal.strut.line_height(), 11.0);
    assert_eq!(normal.wrap_width, constraints().wrap_width);
    let fixed = resolve(&LayoutOptions {
        line_height: LineHeight::Fixed(1.5),
        ..Default::default()
    });
    // 1.5 * 10 = 15, half-leading folded in, gap gone.
    assert_eq!(fixed.strut.line_height(), 15.0);
    assert_eq!(fixed.strut.line_gap, 0.0);
    // The AST itself is untouched by the policy.
    assert_eq!(ast().strut_metrics.line_gap, 1.0);
}

#[test]
fn bad_inputs_are_named() {
    let o = LayoutOptions::default();
    let bad_width = LayoutConstraints {
        wrap_width: f32::NAN,
        ..constraints()
    };
    assert_eq!(
        check_layout_inputs(&ast(), "", &bad_width, &o),
        Err(LayoutInputError::InvalidConstraints)
    );
    let mut outside = ast();
    outside.items = vec![sentinel(1)];
    assert_eq!(
        check_layout_inputs(&outside, "", &constraints(), &o),
        Err(LayoutInputError::ItemOutsideText { item: 0 })
    );
    let mut a = ast();
    a.strut_metrics.ascent = f32::INFINITY;
    assert_eq!(
        check_layout_inputs(&a, "", &constraints(), &o),
        Err(LayoutInputError::InvalidStrut)
    );
    // Splitting a UTF-8 sequence is outside the text.
    let mut a = ast();
    a.items = vec![hard_break()];
    assert_eq!(
        check_layout_inputs(&a, "é", &constraints(), &o),
        Err(LayoutInputError::ItemOutsideText { item: 0 })
    );
    assert!(!LayoutInputError::InvalidStrut.to_string().is_empty());
}

use crate::frontend::{AtomicEditSpan, BoxOverflowPolicy, GlueTier};
use crate::types::{AtomFlags, GraphemeAtom, LineBreakClass, ScriptTag, ShapedCluster};

fn atom(start: usize, end: usize) -> GraphemeAtom {
    GraphemeAtom {
        orientation: crate::types::GlyphOrientation::Horizontal,
        byte_range: TextRange::new(start, end),
        script: ScriptTag::LATIN,
        bidi_level: crate::types::BidiLevel::LTR,
        linebreak_class: LineBreakClass::Al,
        flags: AtomFlags::EMPTY,
    }
}

use std::cell::RefCell;

/// Marks the ranges of a staged box (see [`text_box`]): slot `k` is the
/// range `STAGED + k..STAGED + k + 1`.
const STAGED: usize = usize::MAX / 2;

thread_local! {
    /// Atoms and clusters of the boxes made by [`text_box`] that no AST
    /// stores yet; [`settle`] moves them into the AST's paragraph storage.
    static STAGE: RefCell<Vec<(Vec<GraphemeAtom>, Vec<ShapedCluster>)>> =
        const { RefCell::new(Vec::new()) };
}

/// A text box over `atoms`, one cluster per atom. The range is built
/// without the `start <= end` assertion so out-of-order atoms can be tested.
///
/// The box is *staged*: its atoms and clusters wait in a thread-local queue
/// until [`settle`] (called by every `with_*` builder and by [`tiled`])
/// lays all boxes out in the AST's storage with correct ranges.
fn text_box(atoms: Vec<GraphemeAtom>) -> WordNode {
    let clusters = atoms
        .iter()
        .enumerate()
        .map(|(i, a)| ShapedCluster {
            orientation: crate::types::GlyphOrientation::Horizontal,
            byte_range: a.byte_range,
            atom_range: i..i + 1,
            font: crate::types::FontRef::unscaled(crate::types::FontId(0)),
            bidi_level: crate::types::BidiLevel::LTR,
            flags: AtomFlags::EMPTY,
            glyph_range: 0..0,
            natural_width: 0.0,
            safe_to_break_before: true,
            safe_to_concat_before: true,
        })
        .collect();
    let byte_range = match (atoms.first(), atoms.last()) {
        (Some(f), Some(l)) => TextRange::new(f.byte_range.start(), l.byte_range.end()),
        _ => TextRange::EMPTY,
    };
    let slot = STAGE.with(|stage| {
        let mut stage = stage.borrow_mut();
        stage.push((atoms, clusters));
        STAGED + stage.len() - 1
    });
    WordNode::Box(BoxNode::Text(TextBox {
        run_index: 0,
        byte_range,
        atom_range: slot..slot + 1,
        cluster_range: slot..slot + 1,
        natural_width: 0.0,
    }))
}

/// Re-lays the paragraph storage of `a` in item order: every text box
/// (staged by [`text_box`] or already stored) gets its atoms and clusters
/// appended to its run's range, so ranges tile, boxes are consecutive per
/// run, and boxes dropped from `items` leave no storage behind. A box's
/// clusters keep their offsets relative to its first atom.
fn settle(a: &mut ParagraphAst) {
    let (old_atoms, old_clusters) = (
        std::mem::take(&mut a.atoms),
        std::mem::take(&mut a.clusters),
    );
    // Per run: its atoms and clusters, plus each box's local starts.
    let mut per_run = vec![(Vec::new(), Vec::new()); a.runs.len()];
    let mut starts = Vec::new();
    for node in &a.items {
        let WordNode::Box(BoxNode::Text(TextBox {
            run_index,
            atom_range,
            cluster_range,
            ..
        })) = node
        else {
            continue;
        };
        let (atoms, clusters, first) = if atom_range.start >= STAGED {
            let slot = atom_range.start - STAGED;
            let (atoms, clusters) = STAGE.with(|stage| stage.borrow()[slot].clone());
            (atoms, clusters, 0)
        } else {
            let clusters = old_clusters[cluster_range.clone()].to_vec();
            let first = clusters.first().map_or(0, |c| c.atom_range.start);
            (old_atoms[atom_range.clone()].to_vec(), clusters, first)
        };
        let (run_atoms, run_clusters) = &mut per_run[*run_index];
        starts.push((
            (run_atoms.len(), atoms.len()),
            (run_clusters.len(), clusters.len()),
        ));
        let shift = run_atoms.len();
        run_atoms.extend(atoms);
        run_clusters.extend(clusters.into_iter().map(|mut c| {
            c.atom_range = c.atom_range.start - first + shift..c.atom_range.end - first + shift;
            c
        }));
    }
    let (mut atom_at, mut cluster_at) = (0, 0);
    let mut run_starts = Vec::new();
    for (run, (atoms, clusters)) in a.runs.iter_mut().zip(per_run) {
        run.atom_range = atom_at..atom_at + atoms.len();
        run.cluster_range = cluster_at..cluster_at + clusters.len();
        run_starts.push((atom_at, cluster_at));
        (atom_at, cluster_at) = (run.atom_range.end, run.cluster_range.end);
        a.atoms.extend(atoms);
        a.clusters.extend(clusters);
    }
    let mut starts = starts.into_iter();
    for node in &mut a.items {
        if let WordNode::Box(BoxNode::Text(TextBox {
            run_index,
            atom_range,
            cluster_range,
            ..
        })) = node
        {
            let ((local_atom, n_atoms), (local_cluster, n_clusters)) = starts.next().unwrap();
            let (run_atom, run_cluster) = run_starts[*run_index];
            *atom_range = run_atom + local_atom..run_atom + local_atom + n_atoms;
            *cluster_range = run_cluster + local_cluster..run_cluster + local_cluster + n_clusters;
        }
    }
}

fn run() -> crate::frontend::ShapingRun {
    crate::frontend::ShapingRun {
        word_spacing: 0.0,
        baseline_align: crate::types::BaselineAlignment::Auto,
        letter_spacing: 0.0,
        orientation: crate::types::GlyphOrientation::Horizontal,
        features: Default::default(),

        byte_range: TextRange::new(0, 4),
        families: std::sync::Arc::from([]),
        font_size: 10.0,
        style: Default::default(),
        script: ScriptTag::LATIN,
        locale: Default::default(),
        bidi_level: BidiLevel::LTR,
        decoration: Default::default(),
        atom_range: 0..0,
        cluster_range: 0..0,
    }
}

/// `items` exactly as given (the caller supplies the sentinel).
fn with_raw_items(items: Vec<WordNode>) -> ParagraphAst {
    let mut a = ParagraphAst {
        runs: vec![run()],
        items,
        ..ast()
    };
    settle(&mut a);
    a
}

/// `items` followed by the sentinel of a `len`-byte text.
fn with_items_for(len: usize, mut items: Vec<WordNode>) -> ParagraphAst {
    items.push(sentinel(len));
    let mut a = with_raw_items(items);
    a.runs[0].byte_range = TextRange::new(0, len.max(4));
    a
}

/// `items` followed by the sentinel of the four-byte text most tests use.
fn with_items(items: Vec<WordNode>) -> ParagraphAst {
    with_items_for(4, items)
}

/// Complete partial fixtures with ordinary text, split at every supplied node.
fn tiled(mut a: ParagraphAst) -> ParagraphAst {
    let mut items = Vec::new();
    let mut covered = 0;
    for node in std::mem::take(&mut a.items) {
        let r = node.byte_span();
        if r.start() > covered {
            items.push(text_box(
                (covered..r.start()).map(|i| atom(i, i + 1)).collect(),
            ));
        }
        covered = covered.max(r.end());
        items.push(node);
    }
    a.items = items;
    settle(&mut a);
    a
}

/// Adds the mandatory `Leading` / `Trailing` boundary pair of every
/// decoration span where the enclosure rule wants it: a `Leading` before the
/// first item that starts at or after the span start (a break opportunity
/// at the start stays before it), a `Trailing` before the first item that
/// starts at or after the span end. Equal edges come out nested (`Leading`s
/// inserted innermost first, `Trailing`s outermost last).
fn closed(mut a: ParagraphAst) -> ParagraphAst {
    let spans = a.decoration_spans.clone();
    for sp in spans.iter().rev() {
        let at = a
            .items
            .iter()
            .position(|n| {
                let r = n.byte_span();
                let seam = n.is_break_opportunity()
                    || matches!(n, WordNode::Glue(g) if g.kind == GlueKind::CjkAutospace);
                r.start() >= sp.byte_range.start()
                    && !(seam && r.is_empty() && r.start() == sp.byte_range.start())
            })
            .unwrap_or(a.items.len() - 1);
        a.items.insert(
            at,
            bnd(sp.span_id, SpanEdge::Leading, sp.byte_range.start()),
        );
    }
    for sp in &spans {
        let at = a
            .items
            .iter()
            .position(|n| n.byte_span().start() >= sp.byte_range.end())
            .unwrap_or(a.items.len() - 1);
        a.items
            .insert(at, bnd(sp.span_id, SpanEdge::Trailing, sp.byte_range.end()));
    }
    a
}

#[test]
fn inline_objects_must_have_finite_non_negative_sizes() {
    let o = LayoutOptions::default();
    for (w, a, d) in [
        (f32::NAN, 1.0, 1.0),
        (1.0, -1.0, 1.0),
        (1.0, 1.0, f32::INFINITY),
    ] {
        let a = with_items(vec![WordNode::Box(BoxNode::InlineObject(
            InlineObjectBox {
                bidi_level: BidiLevel::LTR,
                baseline_align: crate::types::BaselineAlignment::Auto,
                byte_range: TextRange::new(0, 1),
                box_id: 1,
                width: w,
                ascent: a,
                descent: d,
                overflow_policy: BoxOverflowPolicy::AllowOverflow,
            },
        ))]);
        assert_eq!(
            check_layout_inputs(&a, "abcd", &constraints(), &o),
            Err(LayoutInputError::InvalidInlineBox { item: 0 })
        );
    }
}

fn glue(natural: f32, stretch: f32, shrink: f32) -> WordNode {
    WordNode::Glue(GlueNode {
        original_bidi_class: crate::types::BidiClass::Ws,
        bidi_level: BidiLevel::LTR,
        is_collapsible: true,

        byte_range: TextRange::new(1, 2),
        kind: GlueKind::WordSpace,
        natural_width: natural,
        max_stretch: stretch,
        max_shrink: shrink,
    })
}

#[test]
fn glue_sizes_must_be_finite_and_non_negative() {
    let o = LayoutOptions::default();
    let check = |mut g: WordNode| {
        if let WordNode::Glue(glue) = &mut g {
            glue.byte_range = TextRange::new(0, 4);
        }
        check_layout_inputs(&with_items(vec![g]), "    ", &constraints(), &o)
    };
    assert!(check(glue(3.0, 1.5, 1.0)).is_ok());
    let bad = Err(LayoutInputError::InvalidGlue { item: 0 });
    assert_eq!(check(glue(f32::NAN, 0.0, 0.0)), bad);
    assert_eq!(check(glue(3.0, -1.0, 0.0)), bad);
    assert_eq!(check(glue(3.0, 0.0, f32::INFINITY)), bad);
}

#[test]
fn discretionary_penalty_and_widths_are_checked() {
    let o = LayoutOptions::default();
    let check = |d: DiscretionaryNode| {
        check_layout_inputs(
            &with_items(vec![WordNode::Discretionary(d)]),
            "abcd",
            &constraints(),
            &o,
        )
    };
    let bad = Err(LayoutInputError::InvalidDiscretionary { item: 0 });
    assert!(
        check_layout_inputs(
            &tiled(with_items(vec![WordNode::Discretionary(
                DiscretionaryNode::direct(2)
            )])),
            "abcd",
            &constraints(),
            &o
        )
        .is_ok()
    );
    // A plain break owns no width of its own (outside a cloned span): any
    // non-zero pre/post width, and any non-finite one, is rejected.
    let mut d = DiscretionaryNode::direct(2);
    d.pre_break_width = -2.5;
    assert_eq!(check(d.clone()), bad);
    d.pre_break_width = 0.0;
    d.post_break_width = 1.0;
    assert_eq!(check(d.clone()), bad);
    d.post_break_width = f32::NAN;
    assert_eq!(check(d), bad);
    // A forbidden break is an omitted node, never a node.
    let mut d = DiscretionaryNode::direct(2);
    d.penalty = DiscretionaryNode::FORBIDDEN_BREAK_PENALTY;
    assert_eq!(check(d), bad);
    // HardBreak and a mandatory penalty go together.
    let mut d = DiscretionaryNode::paragraph_end(4);
    d.penalty = 0;
    assert_eq!(check(d), bad);
    let mut d = DiscretionaryNode::direct(2);
    d.penalty = DiscretionaryNode::MANDATORY_BREAK_PENALTY;
    assert_eq!(check(d), bad);
    // The offset must lie in the text, on a code point boundary.
    assert_eq!(
        check_layout_inputs(
            &with_items(vec![WordNode::Discretionary(DiscretionaryNode::direct(9))]),
            "abcd",
            &constraints(),
            &o
        ),
        Err(LayoutInputError::ItemOutsideText { item: 0 })
    );
}

/// Word "ab" + "é" (bytes 0..4) split at `at` into two boxes around a
/// hyphenation discretionary.
fn hyphenated(at: usize, opportunity: HyphenOpportunity) -> ParagraphAst {
    let atoms = vec![atom(0, 1), atom(1, 2), atom(2, 4)];
    let (head, tail): (Vec<_>, Vec<_>) = atoms.into_iter().partition(|a| a.byte_range.end() <= at);
    let d = DiscretionaryNode {
        byte_range: TextRange::empty_at(at),
        penalty: DiscretionaryNode::HYPHEN_PENALTY,
        flagged: true,
        pre_break_width: 5.0,
        post_break_width: 0.0,
        no_break_width: 0.0,
        action: BreakAction::Hyphenate(opportunity),
        cloned_spans: Default::default(),
    };
    with_items(vec![
        text_box(head),
        WordNode::Discretionary(d),
        text_box(tail),
    ])
}

fn opportunity(prefix_cut: u32, suffix_cut: u32) -> HyphenOpportunity {
    HyphenOpportunity {
        prefix_cut_bytes: prefix_cut,
        suffix_cut_bytes: suffix_cut,
        ..Default::default()
    }
}

#[test]
fn hyphen_opportunities_must_sit_on_atom_boundaries_between_two_boxes() {
    let o = LayoutOptions::default();
    let check = |at: usize, h: HyphenOpportunity| {
        check_layout_inputs(&hyphenated(at, h), "abé", &constraints(), &o)
    };
    assert!(check(1, opportunity(0, 0)).is_ok());
    // Split at 2: head "a" "b" (0..2), tail "é" (2..4). Cuts end at 1 and 4.
    assert!(check(2, opportunity(1, 2)).is_ok());
    let bad = Err(LayoutInputError::InvalidHyphenOpportunity { item: 1 });
    assert_eq!(check(2, opportunity(3, 0)), bad); // cut before the head
    assert_eq!(check(1, opportunity(0, 2)), bad); // ends mid-atom (at 3)
    assert_eq!(check(2, opportunity(0, 3)), bad); // cut past the tail
    // The break position is the discretionary's own. Moved to offset 2,
    // inside the tail box, it is out of item order.
    let mut a = hyphenated(1, opportunity(0, 0));
    if let WordNode::Discretionary(d) = &mut a.items[1] {
        d.byte_range = TextRange::empty_at(2);
    }
    assert_eq!(
        check_layout_inputs(&a, "abé", &constraints(), &o),
        Err(LayoutInputError::ItemsOutOfOrder { item: 2 })
    );
    // It needs a text box on each side.
    let mut a = hyphenated(1, opportunity(0, 0));
    a.items.remove(2);
    settle(&mut a);
    assert_eq!(check_layout_inputs(&a, "abé", &constraints(), &o), bad);
}

#[test]
fn unordered_atoms_report_the_item_error_before_hyphen_or_span_errors() {
    let o = LayoutOptions::default();
    // The tail box's atoms are out of byte order. The hyphen at 1 must not
    // binary-search them; the box itself is reported.
    let mut a = with_items(vec![
        text_box(vec![atom(0, 1)]),
        hyphenated(1, opportunity(0, 1)).items[1].clone(),
        text_box(vec![atom(3, 4), atom(1, 2), atom(2, 3)]),
    ]);
    if let WordNode::Box(BoxNode::Text(TextBox { byte_range, .. })) = &mut a.items[2] {
        *byte_range = TextRange::new(1, 4);
    }
    assert_eq!(
        check_layout_inputs(&a, "abcd", &constraints(), &o),
        Err(LayoutInputError::ItemNotSelfContained { item: 2 })
    );
    // Items out of order: the atomic span is not checked against them.
    let mut a = with_items(vec![text_box(vec![atom(1, 2)]), text_box(vec![atom(0, 1)])]);
    a.atomic_spans = vec![AtomicEditSpan {
        span_id: 1,
        byte_range: TextRange::new(0, 1),
    }];
    assert_eq!(
        check_layout_inputs(&a, "abcd", &constraints(), &o),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );
}

/// "abcd" as `"a" | "b" | "cd"`: boxes at 0..1, 1..2 and 2..4 around two
/// hyphenation points. `first` is the item between the first two boxes,
/// `second_cut` the `(prefix, suffix)` cut of the point at 2.
fn three_part_word(first: WordNode, second_cut: (u32, u32)) -> ParagraphAst {
    let hyphen = |at: usize, prefix: u32, suffix: u32| {
        WordNode::Discretionary(DiscretionaryNode {
            byte_range: TextRange::empty_at(at),
            penalty: DiscretionaryNode::HYPHEN_PENALTY,
            flagged: true,
            pre_break_width: 5.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::Hyphenate(opportunity(prefix, suffix)),
            cloned_spans: Default::default(),
        })
    };
    with_items(vec![
        text_box(vec![atom(0, 1)]),
        first,
        text_box(vec![atom(1, 2)]),
        hyphen(2, second_cut.0, second_cut.1),
        text_box(vec![atom(2, 3), atom(3, 4)]),
    ])
}

#[test]
fn hyphen_cuts_stay_within_the_same_run_hyphen_chain() {
    let o = LayoutOptions::default();
    let hyphen_at_1 = || {
        WordNode::Discretionary(DiscretionaryNode {
            byte_range: TextRange::empty_at(1),
            penalty: DiscretionaryNode::HYPHEN_PENALTY,
            flagged: true,
            pre_break_width: 5.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::Hyphenate(opportunity(0, 0)),
            cloned_spans: Default::default(),
        })
    };
    let check = |a: &ParagraphAst| check_layout_inputs(a, "abcd", &constraints(), &o);
    let bad = Err(LayoutInputError::InvalidHyphenOpportunity { item: 3 });
    // A 2-byte spelling-change cut crosses the unchosen point one byte away.
    assert!(check(&three_part_word(hyphen_at_1(), (2, 1))).is_ok());
    // The suffix cut crosses a later point just the same.
    let mut first = hyphen_at_1();
    if let WordNode::Discretionary(d) = &mut first {
        d.action = BreakAction::Hyphenate(opportunity(0, 2));
    }
    assert!(check(&three_part_word(first, (0, 0))).is_ok());
    // ... but not the start of the word, nor a cut past its end.
    assert_eq!(check(&three_part_word(hyphen_at_1(), (3, 0))), bad);
    assert_eq!(check(&three_part_word(hyphen_at_1(), (0, 3))), bad);
    // Anything but a hyphenation point ends the chain.
    let space = WordNode::Glue(GlueNode {
        original_bidi_class: crate::types::BidiClass::Bn,
        bidi_level: BidiLevel::LTR,
        is_collapsible: true,

        byte_range: TextRange::empty_at(1),
        kind: GlueKind::CjkAutospace,
        natural_width: 1.0,
        max_stretch: 0.0,
        max_shrink: 0.0,
    });
    assert_eq!(check(&three_part_word(space, (2, 0))), bad);
    assert!(check(&three_part_word(hyphen_at_1(), (1, 0))).is_ok());
    let direct = WordNode::Discretionary(DiscretionaryNode::direct(1));
    assert_eq!(check(&three_part_word(direct, (2, 0))), bad);
    assert!(check(&three_part_word(hyphen_at_1(), (1, 0))).is_ok());
}

#[test]
fn a_hyphenation_needs_the_same_run_on_both_sides() {
    let o = LayoutOptions::default();
    // "ab" | "é" around a hyphenation point at 2, in two different runs: a
    // word is split within one ShapingRun, and the hyphen glyph and
    // reshape_window are scoped to one.
    let split = |tail_run: usize| {
        let mut a = hyphenated(2, opportunity(0, 0));
        // The body runs are disjoint and in text order: `ab` | `é` when the
        // tail has its own run, otherwise the second run is empty.
        let mut tail = run();
        if tail_run == 1 {
            a.runs[0].byte_range = TextRange::new(0, 2);
            tail.byte_range = TextRange::new(2, 4);
        } else {
            tail.byte_range = TextRange::empty_at(4);
        }
        a.runs.push(tail);
        if let WordNode::Box(BoxNode::Text(TextBox { run_index, .. })) = &mut a.items[2] {
            *run_index = tail_run;
        }
        settle(&mut a);
        check_layout_inputs(&a, "abé", &constraints(), &o)
    };
    assert!(split(0).is_ok());
    assert_eq!(
        split(1),
        Err(LayoutInputError::InvalidHyphenOpportunity { item: 1 })
    );
    // The same holds inside a longer chain: the first point already splits runs.
    let mut a = three_part_word(
        WordNode::Discretionary(DiscretionaryNode {
            byte_range: TextRange::empty_at(1),
            penalty: DiscretionaryNode::HYPHEN_PENALTY,
            flagged: true,
            pre_break_width: 5.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::Hyphenate(opportunity(0, 0)),
            cloned_spans: Default::default(),
        }),
        (0, 0),
    );
    let mut tail = run();
    a.runs[0].byte_range = TextRange::new(0, 1);
    tail.byte_range = TextRange::new(1, 4);
    a.runs.push(tail);
    for item in [2, 4] {
        if let WordNode::Box(BoxNode::Text(TextBox { run_index, .. })) = &mut a.items[item] {
            *run_index = 1;
        }
    }
    settle(&mut a);
    assert_eq!(
        check_layout_inputs(&a, "abcd", &constraints(), &o),
        Err(LayoutInputError::InvalidHyphenOpportunity { item: 1 })
    );
}

#[test]
fn break_widths_carry_the_padding_of_the_cloned_spans() {
    use crate::frontend::BoxDecorationBreak::Clone;
    let o = LayoutOptions::default();
    // chip(5) spans 1..3 with 2.0 padding on each side; a break at 2 inside it.
    let build = |action: BreakAction, pre: f32, post: f32, cloned: bool| {
        let mut d = DiscretionaryNode::direct(2);
        d.action = action;
        d.pre_break_width = pre;
        d.post_break_width = post;
        if cloned {
            d.cloned_spans.push(5);
        }
        let mut a = with_items(vec![
            with_width(text_box(vec![atom(0, 1)])),
            with_width(text_box(vec![atom(1, 2)])),
            WordNode::Discretionary(d),
            with_width(text_box(vec![atom(2, 3)])),
            with_width(text_box(vec![atom(3, 4)])),
        ]);
        a.decoration_spans = vec![chip(5, Clone)];
        closed(a)
    };
    let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
    let math = |advance: Option<f32>| BreakAction::MathOperator {
        repeat: advance.map(|advance| RepeatedOperator {
            glyph_id: 7,
            advance,
        }),
    };
    // The sums must match exactly for the plain actions.
    for action in [BreakAction::Direct, math(None)] {
        assert!(check(build(action.clone(), 2.0, 2.0, true)).is_ok());
        assert_eq!(
            check(build(action.clone(), 0.0, 2.0, true)),
            Err(LayoutInputError::InvalidDiscretionary { item: 3 })
        );
        assert_eq!(
            check(build(action.clone(), 2.0, 0.0, true)),
            Err(LayoutInputError::InvalidDiscretionary { item: 3 })
        );
        assert_eq!(
            check(build(action, 3.0, 2.0, true)),
            Err(LayoutInputError::InvalidDiscretionary { item: 3 })
        );
    }
    // A repeated operator opens the next line: exactly its advance on
    // top of the cloned start padding, and nothing at the line end.
    assert!(check(build(math(Some(7.0)), 2.0, 9.0, true)).is_ok());
    for (pre, post) in [(9.0, 2.0), (2.0, 2.0), (2.0, 8.0), (3.0, 9.0)] {
        assert_eq!(
            check(build(math(Some(7.0)), pre, post, true)),
            Err(LayoutInputError::InvalidDiscretionary { item: 3 }),
            "{pre} {post}"
        );
    }
    for advance in [-1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(
            check(build(math(Some(advance)), 2.0, 2.0, true)),
            Err(LayoutInputError::InvalidDiscretionary { item: 3 })
        );
    }
    // Without a listed span the widths are zero, and a Clone span the break
    // is inside must be listed.
    assert_eq!(
        check(build(BreakAction::Direct, 2.0, 2.0, false)),
        Err(LayoutInputError::InvalidDiscretionary { item: 3 })
    );
    // Punctuation adjustment: the clone padding is subtracted first, and
    // what remains must be a real, affordable delta (edge clusters are 4 wide).
    let adjust = |pre, post| check(build(BreakAction::CjkPunctAdjust, pre, post, true));
    assert!(adjust(1.0, 2.0).is_ok()); // net_pre = -1
    assert!(adjust(2.0, 5.0).is_ok()); // net_post = +3
    assert!(adjust(-2.0, 2.0).is_ok()); // net_pre = -4: exactly zero left
    assert_eq!(
        adjust(2.0, 2.0),
        Err(LayoutInputError::InvalidDiscretionary { item: 3 })
    ); // padding alone adjusts nothing
    assert_eq!(
        adjust(-2.5, 2.0),
        Err(LayoutInputError::InvalidDiscretionary { item: 3 })
    ); // net_pre = -4.5
    assert_eq!(
        adjust(2.0, -2.5),
        Err(LayoutInputError::InvalidDiscretionary { item: 3 })
    ); // net_post = -4.5
}

#[test]
fn a_non_finite_cluster_total_cannot_match_a_finite_cached_width() {
    let o = LayoutOptions::default();
    let mut a = with_items(vec![text_box(vec![atom(0, 1)])]);
    a.clusters[0].natural_width = f32::INFINITY;
    assert_eq!(
        check_layout_inputs(&a, "abcd", &constraints(), &o),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
}

#[test]
fn punctuation_deltas_are_judged_net_of_large_clone_padding() {
    use crate::frontend::BoxDecorationBreak::Clone;
    let o = LayoutOptions::default();
    // A chip with 1e6 of padding on each side: the padded widths are huge,
    // the net deltas small, and only the net deltas decide.
    let big = |pre: f32, post: f32| {
        let mut d = DiscretionaryNode::direct(2);
        d.action = BreakAction::CjkPunctAdjust;
        d.pre_break_width = pre;
        d.post_break_width = post;
        d.cloned_spans.push(5);
        let mut a = with_items(vec![
            with_width(text_box(vec![atom(0, 1)])),
            with_width(text_box(vec![atom(1, 2)])),
            WordNode::Discretionary(d),
            with_width(text_box(vec![atom(2, 3)])),
            with_width(text_box(vec![atom(3, 4)])),
        ]);
        let mut sp = chip(5, Clone);
        sp.start_padding = 1.0e6;
        sp.end_padding = 1.0e6;
        a.decoration_spans = vec![sp];
        // Boundary widths follow the padding.
        let mut a = closed(a);
        for n in &mut a.items {
            if let WordNode::Box(BoxNode::SpanBoundary { width, .. }) = n {
                *width = 1.0e6;
            }
        }
        check_layout_inputs(&a, "abcd", &constraints(), &o)
    };
    assert!(big(1.0e6 - 1.0, 1.0e6).is_ok()); // net_pre = -1 of 4
    assert!(big(1.0e6, 1.0e6 + 0.5).is_ok()); // a small restore is real
    assert_eq!(
        big(1.0e6 - 5.0, 1.0e6),
        Err(LayoutInputError::InvalidDiscretionary { item: 3 })
    ); // net_pre = -5 empties the cluster
    assert_eq!(
        big(1.0e6, 1.0e6),
        Err(LayoutInputError::InvalidDiscretionary { item: 3 })
    ); // nothing adjusted
}

#[test]
fn every_atom_must_fit_and_be_in_order() {
    let o = LayoutOptions::default();
    // Overall span fits "abcd" but the middle atom does not end inside it.
    let a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 9), atom(2, 4)])]);
    assert_eq!(
        check_layout_inputs(&a, "abcd", &constraints(), &o),
        Err(LayoutInputError::ItemOutsideText { item: 0 })
    );
    // A byte range that disagrees with its atoms is not self-contained.
    let mut a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 2)])]);
    if let WordNode::Box(BoxNode::Text(TextBox { byte_range, .. })) = &mut a.items[0] {
        *byte_range = TextRange::new(0, 3);
    }
    assert_eq!(
        check_layout_inputs(&a, "abcd", &constraints(), &o),
        Err(LayoutInputError::ItemNotSelfContained { item: 0 })
    );
    // A text box naming a run that does not exist.
    let mut a = with_items(vec![text_box(vec![atom(0, 1)])]);
    a.runs.clear();
    a.atoms.clear();
    a.clusters.clear();
    assert_eq!(
        check_layout_inputs(&a, "abcd", &constraints(), &o),
        Err(LayoutInputError::RunIndexOutOfRange { item: 0 })
    );
}

fn chip(span_id: u64, mode: crate::frontend::BoxDecorationBreak) -> InlineDecorationSpan {
    InlineDecorationSpan {
        span_id,
        byte_range: TextRange::new(1, 3),
        start_padding: 2.0,
        end_padding: 2.0,
        break_mode: mode,
        height_style: crate::types::BoxHeightStyle::Strut,
    }
}

#[test]
fn span_boundaries_and_clone_padding_must_name_a_known_span() {
    use crate::frontend::BoxDecorationBreak::{Clone, Slice};
    let o = LayoutOptions::default();
    let boundary = |width: f32, range: TextRange| {
        WordNode::Box(BoxNode::SpanBoundary {
            byte_range: range,
            span_id: 5,
            edge: SpanEdge::Leading,
            width,
            bidi_level: BidiLevel::LTR,
        })
    };
    let with_span = |items, mode| {
        let mut a = with_items(items);
        a.decoration_spans = vec![chip(5, mode)];
        a
    };
    let ok = tiled(with_span(
        vec![
            boundary(2.0, TextRange::empty_at(1)),
            bnd(5, SpanEdge::Trailing, 3),
        ],
        Slice,
    ));
    assert!(check_layout_inputs(&ok, "abcd", &constraints(), &o).is_ok());
    let unknown = with_items(vec![boundary(2.0, TextRange::empty_at(1))]);
    assert_eq!(
        check_layout_inputs(&unknown, "abcd", &constraints(), &o),
        Err(LayoutInputError::UnknownSpan { item: 0 })
    );
    for bad in [
        boundary(2.0, TextRange::new(1, 2)), // not empty
        boundary(-1.0, TextRange::empty_at(1)),
        boundary(f32::NAN, TextRange::empty_at(1)),
    ] {
        let a = with_span(vec![bad], Slice);
        assert_eq!(
            check_layout_inputs(&a, "abcd", &constraints(), &o),
            Err(LayoutInputError::InvalidSpanBoundary { item: 0 })
        );
    }
    let clone_pad = |mode| {
        let mut d = DiscretionaryNode::direct(2);
        d.penalty = DiscretionaryNode::CLONED_SPAN_PENALTY;
        d.cloned_spans.push(5);
        // The cloned chip's padding rides in the break widths: line k gains
        // its end padding, line k + 1 its start padding (2.0 each).
        d.pre_break_width = 2.0;
        d.post_break_width = 2.0;
        closed(with_span(vec![WordNode::Discretionary(d)], mode))
    };
    assert!(check_layout_inputs(&tiled(clone_pad(Clone)), "abcd", &constraints(), &o).is_ok());
    // Padding is cloned only by a span that was declared to clone it.
    assert_eq!(
        check_layout_inputs(&clone_pad(Slice), "abcd", &constraints(), &o),
        Err(LayoutInputError::UnknownSpan { item: 1 }) // item 0 is the Leading edge
    );
}

#[test]
fn cloned_spans_are_orthogonal_to_the_action_and_must_enclose_the_break() {
    use crate::frontend::BoxDecorationBreak::Clone;
    let o = LayoutOptions::default();
    // A hyphenation break at 2 inside a cloning chip over 1..3: both apply.
    let mut a = hyphenated(2, opportunity(0, 0));
    // Boxes are split at the chip's start: "a" | "b" - "é".
    a.items[0] = text_box(vec![atom(0, 1)]);
    a.items.insert(1, text_box(vec![atom(1, 2)]));
    settle(&mut a);
    let mut over = chip(5, Clone);
    over.byte_range = TextRange::new(1, 4); // "b" and "é"
    a.decoration_spans = vec![over];
    if let WordNode::Discretionary(d) = &mut a.items[2] {
        d.cloned_spans.push(5);
        d.penalty += DiscretionaryNode::CLONED_SPAN_PENALTY;
    }
    let a = closed(a);
    assert!(check_layout_inputs(&a, "abé", &constraints(), &o).is_ok());
    // A break on the span's edge is not inside it; a repeated id is wrong.
    for (cloned, at, expected) in [
        (vec![5], 1, LayoutInputError::UnknownSpan { item: 0 }),
        (vec![5], 3, LayoutInputError::UnknownSpan { item: 2 }),
        (
            vec![5, 5],
            2,
            LayoutInputError::InvalidDiscretionary { item: 1 },
        ),
    ] {
        let mut d = DiscretionaryNode::direct(at);
        d.cloned_spans.extend(cloned);
        let mut b = with_items(vec![WordNode::Discretionary(d)]);
        b.decoration_spans = vec![chip(5, Clone)];
        let b = closed(b);
        assert_eq!(
            check_layout_inputs(&b, "abcd", &constraints(), &o),
            Err(expected),
            "break at {at}"
        );
    }
}

#[test]
fn soft_breaks_own_nothing_and_hard_breaks_own_their_terminator() {
    let o = LayoutOptions::default();
    let check = |text: &str, d: DiscretionaryNode| {
        let mut a = with_items_for(text.len(), vec![WordNode::Discretionary(d)]);
        a.runs[0].byte_range = TextRange::new(0, text.len());
        check_layout_inputs(&a, text, &constraints(), &o)
    };
    let bad = Err(LayoutInputError::InvalidDiscretionary { item: 0 });
    let text = "a\u{2028}b";
    let separator_len = 3;
    let newline = TextRange::new(1, 1 + separator_len);
    let mut a = tiled(with_items_for(
        text.len(),
        vec![WordNode::Discretionary(DiscretionaryNode::hard_break(
            newline,
        ))],
    ));
    a.runs[0].byte_range = TextRange::new(0, text.len());
    let result = check_layout_inputs(&a, text, &constraints(), &o);
    assert!(result.is_ok(), "{text:?}: {result:?}");
    // The terminator must be the whole range: a longer one swallows text.
    assert_eq!(
        check(
            text,
            DiscretionaryNode::hard_break(TextRange::new(1, 2 + separator_len))
        ),
        bad
    );
    // Not a terminator.
    assert_eq!(
        check("abcd", DiscretionaryNode::hard_break(TextRange::new(1, 2))),
        bad
    );
    // A mid-paragraph hard break must own its newline; only the end may not.
    assert_eq!(
        check(
            "abcd",
            DiscretionaryNode::hard_break(TextRange::empty_at(1))
        ),
        bad
    );
    // A soft opportunity owns nothing.
    let mut soft = DiscretionaryNode::direct(1);
    soft.byte_range = TextRange::new(1, 2);
    assert_eq!(check("abcd", soft), bad);
    assert_eq!(DiscretionaryNode::direct(3).byte_offset(), 3);
}

/// A staged text box (see [`text_box`]) with every cluster 4 wide.
fn with_width(mut n: WordNode) -> WordNode {
    if let WordNode::Box(BoxNode::Text(TextBox {
        atom_range,
        natural_width,
        ..
    })) = &mut n
    {
        let slot = atom_range.start - STAGED;
        STAGE.with(|stage| {
            let clusters = &mut stage.borrow_mut()[slot].1;
            clusters.iter_mut().for_each(|c| c.natural_width = 4.0);
            *natural_width = clusters.len() as f32 * 4.0;
        });
    }
    n
}

#[test]
fn cjk_punct_adjust_deltas_need_a_neighbour_and_stay_non_negative() {
    let o = LayoutOptions::default();
    // Two boxes whose edge clusters are 4 wide, around the break.
    let adjust = |pre: f32, post: f32| {
        let mut d = DiscretionaryNode::direct(2);
        d.action = BreakAction::CjkPunctAdjust;
        d.pre_break_width = pre;
        d.post_break_width = post;
        with_items(vec![
            with_width(text_box(vec![atom(0, 2)])),
            WordNode::Discretionary(d),
            with_width(text_box(vec![atom(2, 4)])),
        ])
    };
    let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
    // A trim to exactly zero is allowed; a restore is positive.
    assert!(check(adjust(-4.0, -4.0)).is_ok());
    assert!(check(adjust(2.0, 0.0)).is_ok());
    let bad = Err(LayoutInputError::InvalidDiscretionary { item: 1 });
    // An adjustment that adjusts nothing is no adjustment.
    assert_eq!(check(adjust(0.0, 0.0)), bad);
    assert_eq!(check(adjust(-4.5, 0.0)), bad); // last cluster would go negative
    assert_eq!(check(adjust(0.0, -4.5)), bad); // first cluster would go negative
    // A non-zero delta needs the text box it applies to.
    let mut a = adjust(-1.0, 0.0);
    a.items.remove(0);
    settle(&mut a);
    assert_eq!(
        check(a),
        Err(LayoutInputError::InvalidDiscretionary { item: 0 })
    );
    let mut a = adjust(0.0, -1.0);
    a.items.remove(2);
    settle(&mut a);
    assert_eq!(check(a), bad);
    // A delta on one side needs no neighbour on the other.
    let mut a = adjust(-1.0, 0.0);
    a.items.remove(2);
    assert!(check(tiled(a)).is_ok());
}

fn bnd(id: u64, edge: SpanEdge, at: usize) -> WordNode {
    WordNode::Box(BoxNode::SpanBoundary {
        byte_range: TextRange::empty_at(at),
        span_id: id,
        edge,
        width: 2.0,
        bidi_level: BidiLevel::LTR,
    })
}

#[test]
fn span_boundaries_nest_in_lifo_order_and_equal_ranges_follow_the_slice_order() {
    use crate::frontend::BoxDecorationBreak::Slice;
    use SpanEdge::{Leading as L, Trailing as T};
    let o = LayoutOptions::default();
    let ast = |boundaries: [(u64, SpanEdge, usize); 4]| {
        let [b0, b1, b2, b3] = boundaries.map(|(id, e, at)| bnd(id, e, at));
        // "a" [ b0 b1 "bc" b2 b3 ] "d"; spans 1 (outer) and 2 (inner) share 1..3.
        let mut a = with_items(vec![
            text_box(vec![atom(0, 1)]),
            b0,
            b1,
            text_box(vec![atom(1, 2), atom(2, 3)]),
            b2,
            b3,
            text_box(vec![atom(3, 4)]),
        ]);
        a.decoration_spans = vec![chip(1, Slice), chip(2, Slice)];
        a
    };
    let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
    // The lower index is the outer span: L1 L2 .. T2 T1.
    assert!(check(ast([(1, L, 1), (2, L, 1), (2, T, 3), (1, T, 3)])).is_ok());
    // Consistent LIFO but inverted (span 2 outside span 1): rejected.
    assert_eq!(
        check(ast([(2, L, 1), (1, L, 1), (1, T, 3), (2, T, 3)])),
        Err(LayoutInputError::InvalidSpanBoundary { item: 2 })
    );
    // Crossing (L1 L2 T1 T2) breaks LIFO.
    assert_eq!(
        check(ast([(1, L, 1), (2, L, 1), (1, T, 3), (2, T, 3)])),
        Err(LayoutInputError::InvalidSpanBoundary { item: 4 })
    );
    // A span opened twice.
    assert_eq!(
        check(ast([(1, L, 1), (1, L, 1), (1, T, 3), (1, T, 3)])),
        Err(LayoutInputError::InvalidSpanBoundary { item: 2 })
    );
    // Nested spans with different ranges obey the same stack.
    let mut nested = ast([(1, L, 1), (2, L, 1), (2, T, 3), (1, T, 3)]);
    nested.decoration_spans[1].byte_range = TextRange::new(1, 3);
    nested.decoration_spans[0].byte_range = TextRange::new(1, 4);
    let ids = check_decoration_spans(&nested, "abcd").unwrap();
    assert!(
        check_boundary_order(&nested, &ids).is_ok(),
        "inner span closes first"
    );
}

#[test]
fn the_paragraph_end_sentinel_is_empty_last_and_unique() {
    let o = LayoutOptions::default();
    let check = |text: &str, items: Vec<WordNode>| {
        let mut a = with_raw_items(items);
        a.runs[0].byte_range = TextRange::new(0, text.len());
        check_layout_inputs(&a, text, &constraints(), &o)
    };
    let newline = |a: usize, b: usize| {
        WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(a, b)))
    };
    assert_eq!(
        check("abcd", vec![sentinel(4)]),
        Err(LayoutInputError::UncoveredSource { byte: 0 })
    );
    // An empty paragraph is just the sentinel.
    assert!(check("", vec![sentinel(0)]).is_ok());
    // Text ending in a newline: the real break, then the sentinel.
    assert!(
        check(
            "ab\u{2028}",
            vec![
                text_box(vec![atom(0, 1), atom(1, 2)]),
                newline(2, 5),
                sentinel(5)
            ]
        )
        .is_ok()
    );
    let bad = |item| Err(LayoutInputError::InvalidDiscretionary { item });
    // The sentinel sits at the end of the text, not before it.
    assert_eq!(check("abcd", vec![sentinel(3)]), bad(0));
    // It must be the final discretionary.
    let soft = WordNode::Discretionary(DiscretionaryNode::direct(4));
    assert_eq!(check("abcd", vec![sentinel(4), soft]), bad(0));
    // At most once.
    assert_eq!(check("abcd", vec![sentinel(4), sentinel(4)]), bad(0));
    // An empty hard break is the sentinel by definition, so one before
    // the end is a sentinel out of place.
    assert_eq!(
        check(
            "ab\u{2028}",
            vec![newline(2, 2), newline(2, 5), sentinel(5)]
        ),
        bad(0)
    );
}

#[test]
fn the_items_must_end_with_the_sentinel() {
    let o = LayoutOptions::default();
    let check = |text: &str, items: Vec<WordNode>| {
        let mut a = with_raw_items(items);
        a.runs[0].byte_range = TextRange::new(0, text.len());
        check_layout_inputs(&a, text, &constraints(), &o)
    };
    let missing = Err(LayoutInputError::MissingParagraphEnd);
    // No items at all, even for an empty paragraph.
    assert_eq!(check("", vec![]), missing);
    // A real hard break is not the sentinel: "ab\u{2028}" needs both.
    let newline = WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(2, 5)));
    assert_eq!(check("ab\u{2028}", vec![newline]), missing);
    // A soft opportunity or a box last.
    let soft = WordNode::Discretionary(DiscretionaryNode::direct(2));
    assert_eq!(check("abcd", vec![soft]), missing);
    let text = text_box(vec![atom(0, 1)]);
    // Text after the sentinel is out of order before the sentinel check.
    assert_eq!(
        check("abcd", vec![sentinel(4), text]),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );
    // Four traces from the line-emission rule: N newlines, N + 1 mandatory
    // breaks, so N + 1 lines.
    let mandatory = |items: &[WordNode]| {
        items
            .iter()
            .filter(|n| matches!(n, WordNode::Discretionary(d) if d.is_mandatory()))
            .count()
    };
    let nl = |a: usize, b: usize| {
        WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(a, b)))
    };
    let cases: [(&str, Vec<WordNode>); 4] = [
        ("", vec![sentinel(0)]),
        (
            "abc",
            vec![
                text_box(vec![atom(0, 1), atom(1, 2), atom(2, 3)]),
                sentinel(3),
            ],
        ),
        (
            "abc\u{2028}",
            vec![
                text_box(vec![atom(0, 1), atom(1, 2), atom(2, 3)]),
                nl(3, 6),
                sentinel(6),
            ],
        ),
        (
            "a\u{2028}\u{2028}b",
            vec![
                text_box(vec![atom(0, 1)]),
                nl(1, 4),
                nl(4, 7),
                text_box(vec![atom(7, 8)]),
                sentinel(8),
            ],
        ),
    ];
    for (text, items) in cases {
        let newlines = text.matches('\u{2028}').count();
        assert_eq!(mandatory(&items), newlines + 1, "{text:?}");
        assert!(check(text, items).is_ok(), "{text:?}");
    }
}

#[test]
fn the_tier_follows_the_kind_and_the_compression_pool_is_shrink_only() {
    assert_eq!(GlueKind::WordSpace.tier(), GlueTier::Tier0);
    assert_eq!(GlueKind::CjkPunctCompression.tier(), GlueTier::Tier0);
    // Tier 2 is derived from cluster flags: no kind of glue has it.
    assert_eq!(GlueKind::CjkAutospace.tier(), GlueTier::Tier1);

    let o = LayoutOptions::default();
    let pool = |natural, stretch, shrink| {
        WordNode::Glue(GlueNode {
            original_bidi_class: crate::types::BidiClass::Bn,
            bidi_level: BidiLevel::LTR,
            is_collapsible: true,
            byte_range: TextRange::empty_at(1),
            kind: GlueKind::CjkPunctCompression,
            natural_width: natural,
            max_stretch: stretch,
            max_shrink: shrink,
        })
    };
    let check = |g| check_layout_inputs(&with_items(vec![g]), "abcd", &constraints(), &o);
    assert!(
        check_layout_inputs(
            &tiled(with_items(vec![pool(0.0, 0.0, 5.0)])),
            "abcd",
            &constraints(),
            &o
        )
        .is_ok()
    );
    let bad = Err(LayoutInputError::InvalidGlue { item: 0 });
    assert_eq!(check(pool(1.0, 0.0, 5.0)), bad);
    assert_eq!(check(pool(0.0, 1.0, 5.0)), bad);
}

#[test]
fn a_word_space_owns_word_space_characters() {
    let o = LayoutOptions::default();
    let space = |range: (usize, usize), is_collapsible| {
        WordNode::Glue(GlueNode {
            original_bidi_class: crate::types::BidiClass::Ws,
            bidi_level: BidiLevel::LTR,
            is_collapsible,
            byte_range: TextRange::new(range.0, range.1),
            kind: GlueKind::WordSpace,
            natural_width: 3.0,
            max_stretch: 1.0,
            max_shrink: 1.0,
        })
    };
    let check = |text: &str, glue: WordNode| {
        let mut a = with_items_for(text.len(), vec![text_box(vec![atom(0, 1)]), glue]);
        a.runs[0].byte_range = TextRange::new(0, text.len());
        check_layout_inputs(&a, text, &constraints(), &o)
    };
    let bad = Err(LayoutInputError::InvalidGlue { item: 1 });
    // One space, a run of spaces, and the other space separators.
    assert_eq!(check("a ", space((1, 2), true)), Ok(()));
    assert_eq!(check("a  ", space((1, 3), true)), Ok(()));
    assert_eq!(check("a\u{2003}", space((1, 4), true)), Ok(()));
    // The ideographic space is text, not glue.
    assert_eq!(check("a\u{3000}", space((1, 4), true)), bad);
    assert_eq!(check("a\u{3000}", space((1, 4), false)), bad);
    // Letters are not a space, wholly or in part.
    assert_eq!(check("ab", space((1, 2), true)), bad);
    assert_eq!(check("a b", space((1, 3), true)), bad);
    // A word space owns bytes.
    assert_eq!(check("a", space((1, 1), true)), bad);
    // A no-break space is text, not glue, collapsible or not.
    for no_break in ["a\u{a0}", "a\u{2007}", "a\u{202f}"] {
        assert_eq!(check(no_break, space((1, no_break.len()), true)), bad);
        assert_eq!(check(no_break, space((1, no_break.len()), false)), bad);
    }
    // A word space is always collapsible.
    assert_eq!(check("a ", space((1, 2), false)), bad);
}

#[test]
fn span_boundary_bidi_level_follows_the_atom_inside_the_span() {
    use crate::frontend::BoxDecorationBreak::Slice;
    let o = LayoutOptions::default();
    let mut inner = atom(1, 2);
    inner.bidi_level = crate::types::BidiLevel::RTL;
    // Edge `edge` carries `level`; the other edge carries its correct level.
    let build = |edge: SpanEdge, level: BidiLevel| {
        let boundary = |e: SpanEdge, correct: BidiLevel, at: usize| {
            WordNode::Box(BoxNode::SpanBoundary {
                byte_range: TextRange::empty_at(at),
                span_id: 5,
                edge: e,
                width: 2.0,
                bidi_level: if e == edge { level } else { correct },
            })
        };
        let mut a = with_items(vec![
            text_box(vec![atom(0, 1)]),
            boundary(SpanEdge::Leading, BidiLevel::RTL, 1),
            text_box(vec![inner.clone()]),
            text_box(vec![atom(2, 3)]),
            boundary(SpanEdge::Trailing, BidiLevel::LTR, 3),
            text_box(vec![atom(3, 4)]),
        ]);
        // Three disjoint runs in text order: `a` | the level-1 atom | `cd`.
        let mut rtl_run = run();
        rtl_run.bidi_level = BidiLevel::RTL;
        rtl_run.byte_range = TextRange::new(1, 2);
        let mut tail = run();
        tail.byte_range = TextRange::new(2, 4);
        a.runs[0].byte_range = TextRange::new(0, 1);
        a.runs.extend([rtl_run, tail]);
        for (item, run) in [(2, 1), (3, 2), (5, 2)] {
            if let WordNode::Box(BoxNode::Text(TextBox { run_index, .. })) = &mut a.items[item] {
                *run_index = run;
            }
        }
        a.clusters[1].bidi_level = crate::types::BidiLevel::RTL;
        settle(&mut a);
        a.decoration_spans = vec![chip(5, Slice)];
        a
    };
    let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
    // First atom inside the span is level 1, the last is level 0.
    assert!(check(build(SpanEdge::Leading, BidiLevel::RTL)).is_ok());
    assert!(check(build(SpanEdge::Trailing, BidiLevel::LTR)).is_ok());
    assert_eq!(
        check(build(SpanEdge::Leading, BidiLevel::LTR)),
        Err(LayoutInputError::InvalidSpanBoundary { item: 1 })
    );
    assert_eq!(
        check(build(SpanEdge::Trailing, BidiLevel::RTL)),
        Err(LayoutInputError::InvalidSpanBoundary { item: 4 })
    );
}

#[test]
fn items_progress_monotonically_and_spans_enclose_their_items() {
    use crate::frontend::BoxDecorationBreak::Slice;
    use SpanEdge::{Leading as L, Trailing as T};
    let o = LayoutOptions::default();
    let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
    let a_ = || text_box(vec![atom(0, 1)]);
    let b_ = || text_box(vec![atom(1, 2)]);
    let c_ = || text_box(vec![atom(2, 3)]);
    let d_ = || text_box(vec![atom(3, 4)]);
    let brk = |at: usize| WordNode::Discretionary(DiscretionaryNode::direct(at));
    let span = |a: ParagraphAst| {
        let mut a = a;
        a.decoration_spans = vec![chip(5, Slice)]; // bytes 1..3
        a
    };
    // Monotone progression: a later item may not start before the previous
    // one ends (zero-width nodes may share an offset).
    assert_eq!(
        check(with_items(vec![b_(), a_()])),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );
    assert!(check(with_items(vec![a_(), brk(1), b_(), brk(2), c_(), d_()])).is_ok());
    assert_eq!(
        check(with_items(vec![a_(), b_(), brk(1)])),
        Err(LayoutInputError::ItemsOutOfOrder { item: 2 })
    );
    // Reopening a closed span is caught by the order check.
    assert_eq!(
        check(span(with_items(vec![
            a_(),
            bnd(5, L, 1),
            b_(),
            c_(),
            bnd(5, T, 3),
            bnd(5, L, 1),
            bnd(5, T, 3),
            d_(),
        ]))),
        Err(LayoutInputError::ItemsOutOfOrder { item: 5 })
    );
    // The well-formed chip: break before it, content between, break after.
    let good = || {
        vec![
            a_(),
            brk(1),
            bnd(5, L, 1),
            b_(),
            brk(2),
            c_(),
            bnd(5, T, 3),
            brk(3),
            d_(),
        ]
    };
    assert!(check(span(with_items(good()))).is_ok());
    // A break at the start after Leading would strand the boundary on the
    // previous line; one at the end before Trailing likewise.
    let mut late = good();
    late.swap(1, 2);
    assert_eq!(
        check(span(with_items(late))),
        Err(LayoutInputError::SpanEnclosure { span: 0, item: 2 })
    );
    let mut early = good();
    early.swap(6, 7);
    assert_eq!(
        check(span(with_items(early))),
        Err(LayoutInputError::SpanEnclosure { span: 0, item: 6 })
    );
    // Content outside the chip may not sit between its boundaries, and
    // content inside may not sit outside them.
    let outside = span(with_items(vec![
        a_(),
        bnd(5, L, 1),
        b_(),
        c_(),
        d_(),
        bnd(5, T, 3),
    ]));
    assert_eq!(
        check(outside),
        Err(LayoutInputError::ItemsOutOfOrder { item: 5 })
    );
    let inside = span(with_items(vec![
        a_(),
        b_(),
        bnd(5, L, 1),
        c_(),
        bnd(5, T, 3),
        d_(),
    ]));
    assert_eq!(
        check(inside),
        Err(LayoutInputError::ItemsOutOfOrder { item: 2 })
    );
    // A box that straddles a span edge must have been split by the frontend.
    let straddle = closed(span(with_items(vec![text_box(vec![
        atom(0, 1),
        atom(1, 2),
    ])])));
    // (No boundary can sit inside a box's range, so monotone order catches it.)
    assert_eq!(
        check(straddle),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );
    // Nested spans: the inner chip's nodes are inside the outer one.
    let mut nested = with_items(vec![
        bnd(1, L, 0),
        a_(),
        bnd(2, L, 1),
        b_(),
        bnd(2, T, 2),
        c_(),
        d_(),
        bnd(1, T, 4),
    ]);
    let mut inner = chip(2, Slice);
    inner.byte_range = TextRange::new(1, 2);
    let mut outer = chip(1, Slice);
    outer.byte_range = TextRange::new(0, 4);
    nested.decoration_spans = vec![outer, inner];
    // The sentinel at 4 follows Trailing(1) at 4: no stranded boundary.
    let r = check(nested);
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn cjk_autospace_at_a_span_edge_sits_outside_its_boundaries() {
    use crate::frontend::BoxDecorationBreak::Slice;
    use SpanEdge::{Leading as L, Trailing as T};
    let o = LayoutOptions::default();
    let autospace = |at: usize| {
        WordNode::Glue(GlueNode {
            original_bidi_class: crate::types::BidiClass::Bn,
            bidi_level: BidiLevel::LTR,
            is_collapsible: true,

            byte_range: TextRange::empty_at(at),
            kind: GlueKind::CjkAutospace,
            natural_width: 2.0,
            max_stretch: 1.0,
            max_shrink: 1.0,
        })
    };
    let brk = |at: usize| WordNode::Discretionary(DiscretionaryNode::direct(at));
    let a_ = || text_box(vec![atom(0, 1)]);
    let b_ = || text_box(vec![atom(1, 2)]);
    let c_ = || text_box(vec![atom(2, 3)]);
    let d_ = || text_box(vec![atom(3, 4)]);
    let span = |items| {
        let mut a = with_items(items);
        a.decoration_spans = vec![chip(5, Slice)]; // bytes 1..3
        a
    };
    let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
    // Seam spacing, then the break, then the chip; the chip's end mirrors it.
    let good = vec![
        a_(),
        autospace(1),
        brk(1),
        bnd(5, L, 1),
        b_(),
        c_(),
        bnd(5, T, 3),
        autospace(3),
        brk(3),
        d_(),
    ];
    assert!(check(span(good.clone())).is_ok());
    // The start seam after Leading would strand the spacing inside the chip.
    let mut late = good.clone();
    late.swap(1, 3);
    late.swap(2, 3); // a_, bnd L, autospace, brk ... (autospace now after Leading)
    assert_eq!(
        check(span(late)),
        Err(LayoutInputError::SpanEnclosure { span: 0, item: 2 })
    );
    // The end seam before Trailing likewise.
    let mut early = good.clone();
    early.swap(6, 7);
    assert_eq!(
        check(span(early)),
        Err(LayoutInputError::SpanEnclosure { span: 0, item: 6 })
    );
    // Two adjacent chips share the seam at 2: canonical order
    // [A Trailing, Glue(CjkAutospace), Discretionary, B Leading].
    let mut other = chip(6, Slice);
    other.byte_range = TextRange::new(2, 4);
    let mut a = with_items(vec![
        a_(),
        bnd(5, L, 1),
        b_(),
        c_(),
        bnd(5, T, 3),
        autospace(3),
        brk(3),
        bnd(6, L, 3),
        d_(),
        bnd(6, T, 4),
    ]);
    a.decoration_spans = vec![chip(5, Slice), other];
    // Chip 5 spans 1..3 and chip 6 starts at 2: fix the second to 3..4.
    a.decoration_spans[1].byte_range = TextRange::new(3, 4);
    let r = check(a.clone());
    assert!(r.is_ok(), "{r:?}");
    // The break before the seam glue is the wrong way round.
    let mut swapped = a.clone();
    swapped.items.swap(5, 6);
    assert_eq!(
        check(swapped),
        Err(LayoutInputError::ItemsOutOfOrder { item: 6 })
    );
    // B's Leading before the seam glue or break strands them inside B.
    a.items.swap(6, 7);
    assert_eq!(
        check(a),
        Err(LayoutInputError::SpanEnclosure { span: 1, item: 7 })
    );
}

#[test]
fn flagged_hyphens_cloned_spans_synthetic_glue_and_terminators_are_enforced() {
    use crate::frontend::BoxDecorationBreak::{Clone, Slice};
    let o = LayoutOptions::default();
    let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);

    // Hyphenation breaks are flagged.
    let mut a = hyphenated(1, opportunity(0, 0));
    if let WordNode::Discretionary(d) = &mut a.items[1] {
        d.flagged = false;
    }
    assert_eq!(
        check_layout_inputs(&a, "abé", &constraints(), &o),
        Err(LayoutInputError::InvalidHyphenOpportunity { item: 1 })
    );

    // A Clone span the break is strictly inside must be listed.
    let mut d = DiscretionaryNode::direct(2);
    d.penalty = DiscretionaryNode::CLONED_SPAN_PENALTY;
    let mut a = with_items(vec![WordNode::Discretionary(d)]);
    a.decoration_spans = vec![chip(5, Clone)]; // 1..3
    assert_eq!(
        check(closed(a)),
        Err(LayoutInputError::InvalidDiscretionary { item: 1 })
    );
    // A Slice span needs no listing.
    let mut a = with_items(vec![WordNode::Discretionary(DiscretionaryNode::direct(2))]);
    a.decoration_spans = vec![chip(5, Slice)];
    assert!(check(tiled(closed(a))).is_ok());

    // Synthetic glue owns no bytes.
    for kind in [GlueKind::CjkAutospace, GlueKind::CjkPunctCompression] {
        let a = with_items(vec![WordNode::Glue(GlueNode {
            original_bidi_class: crate::types::BidiClass::Ws,
            bidi_level: BidiLevel::LTR,
            is_collapsible: true,

            byte_range: TextRange::new(1, 2),
            kind,
            natural_width: 0.0,
            max_stretch: 0.0,
            max_shrink: 1.0,
        })]);
        assert_eq!(check(a), Err(LayoutInputError::InvalidGlue { item: 0 }));
    }

    // A boundary's width is its edge's padding; its level is in range even
    // when the span holds no atom to compare with.
    let mut inner = chip(5, Slice);
    inner.start_padding = 2.0;
    inner.end_padding = 6.0;
    let build = |lead_w: f32, trail_w: f32, level: BidiLevel| {
        let node = |edge, width, bidi_level| {
            WordNode::Box(BoxNode::SpanBoundary {
                byte_range: TextRange::empty_at(if edge == SpanEdge::Leading { 1 } else { 3 }),
                span_id: 5,
                edge,
                width,
                bidi_level,
            })
        };
        let mut a = with_items(vec![
            node(SpanEdge::Leading, lead_w, level),
            node(SpanEdge::Trailing, trail_w, level),
        ]);
        a.decoration_spans = vec![inner];
        a
    };
    assert!(check(tiled(build(2.0, 6.0, BidiLevel::LTR))).is_ok());
    let bad = |item| Err(LayoutInputError::InvalidSpanBoundary { item });
    assert_eq!(check(build(6.0, 6.0, BidiLevel::LTR)), bad(0));
    assert_eq!(check(build(2.0, 2.0, BidiLevel::LTR)), bad(1));
    // A level 127 is not representable at all.
    assert_eq!(BidiLevel::try_new(127), None);

    // Half of a CRLF pair is not a terminator.
    let run_over = |text: &str, items: Vec<WordNode>| {
        let mut a = with_items_for(text.len(), items);
        a.runs[0].byte_range = TextRange::new(0, text.len());
        check_layout_inputs(&a, text, &constraints(), &o)
    };
    let nl = |a: usize, b: usize| {
        WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(a, b)))
    };
    assert_eq!(
        run_over("a\r\nb", vec![nl(1, 3)]),
        Err(LayoutInputError::ParagraphSeparatorInText)
    );
    assert_eq!(
        run_over("a\r\nb", vec![nl(1, 2)]),
        Err(LayoutInputError::ParagraphSeparatorInText)
    ); // lone CR of a CRLF
    assert_eq!(
        run_over("a\r\nb", vec![nl(2, 3)]),
        Err(LayoutInputError::ParagraphSeparatorInText)
    ); // lone LF of a CRLF
    assert_eq!(
        run_over("a\rb", vec![nl(1, 2)]),
        Err(LayoutInputError::ParagraphSeparatorInText)
    );

    // A text box lies in its run and caches the total of its clusters.
    let mut a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 2)])]);
    a.runs[0].byte_range = TextRange::new(0, 1);
    assert_eq!(check(a), Err(LayoutInputError::ItemOutsideRun { item: 0 }));
    let mut a = with_items(vec![text_box(vec![atom(0, 1)])]);
    if let WordNode::Box(BoxNode::Text(TextBox { natural_width, .. })) = &mut a.items[0] {
        *natural_width = 9.0; // the cluster is 0 wide
    }
    assert_eq!(
        check(a),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
    let mut a = with_items(vec![text_box(vec![atom(0, 1)])]);
    if let WordNode::Box(BoxNode::Text(TextBox { natural_width, .. })) = &mut a.items[0] {
        *natural_width = f32::NAN;
    }
    assert_eq!(
        check(a),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
}

#[test]
fn every_decoration_span_needs_both_boundaries_even_without_padding() {
    use crate::frontend::BoxDecorationBreak::Slice;
    use SpanEdge::{Leading as L, Trailing as T};
    let o = LayoutOptions::default();
    // Zero padding: the boundary nodes are zero wide.
    let zb = |id: u64, e: SpanEdge, at: usize| match bnd(id, e, at) {
        WordNode::Box(BoxNode::SpanBoundary {
            byte_range,
            span_id,
            edge,
            bidi_level,
            ..
        }) => WordNode::Box(BoxNode::SpanBoundary {
            byte_range,
            span_id,
            edge,
            width: 0.0,
            bidi_level,
        }),
        other => other,
    };
    let zero = |id: u64| {
        let mut s = chip(id, Slice);
        s.start_padding = 0.0;
        s.end_padding = 0.0;
        s
    };
    let build = |boundaries: Vec<WordNode>| {
        // "a", the boundaries (an empty chip), "d": boxes split at the edges.
        let mut items = vec![text_box(vec![atom(0, 1)])];
        items.extend(boundaries);
        items.push(text_box(vec![atom(3, 4)]));
        let mut a = with_items(items);
        a.decoration_spans = vec![zero(5)]; // bytes 1..3
        a
    };
    let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
    assert!(check(tiled(build(vec![zb(5, L, 1), zb(5, T, 3)]))).is_ok());
    // Zero padding does not excuse a missing node.
    let missing = Err(LayoutInputError::MissingSpanBoundary { span: 0 });
    assert_eq!(check(build(vec![])), missing);
    assert_eq!(check(build(vec![zb(5, L, 1)])), missing);
    // A Trailing with no open Leading is a misplaced boundary.
    assert_eq!(
        check(build(vec![zb(5, T, 3)])),
        Err(LayoutInputError::InvalidSpanBoundary { item: 1 })
    );
    // A boundary before the preceding box starts is out of order.
    assert_eq!(
        check(build(vec![zb(5, L, 0), zb(5, T, 3)])),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );
    // Exactly one of each, at the span's own edges.
    for (wrong, item) in [
        (vec![zb(5, L, 1), zb(5, T, 2)], 2),
        (vec![zb(5, L, 1), zb(5, L, 1), zb(5, T, 3)], 2),
        (vec![zb(5, L, 1), zb(5, T, 3), zb(5, T, 3)], 3),
    ] {
        assert_eq!(
            check(build(wrong)),
            Err(LayoutInputError::InvalidSpanBoundary { item })
        );
    }
}

#[test]
fn span_streams_are_validated() {
    use crate::frontend::BoxDecorationBreak::Slice;
    let o = LayoutOptions::default();
    let check = |deco: Vec<InlineDecorationSpan>, atomic: Vec<AtomicEditSpan>| {
        let mut a = with_items(
            (0..4)
                .map(|i| text_box(vec![atom(i, i + 1)]))
                .collect::<Vec<_>>(),
        );
        a.decoration_spans = deco;
        a.atomic_spans = atomic;
        check_layout_inputs(&closed(a), "abcd", &constraints(), &o)
    };
    let atomic = |id, r: TextRange| AtomicEditSpan {
        span_id: id,
        byte_range: r,
    };
    assert!(check(vec![chip(1, Slice)], vec![atomic(1, TextRange::new(1, 3))]).is_ok());
    // Decoration: duplicate id, bad padding, outside the text.
    let mut padded = chip(2, Slice);
    padded.start_padding = f32::NAN;
    let mut outside = chip(3, Slice);
    outside.byte_range = TextRange::new(1, 9);
    for (list, index) in [
        (vec![chip(1, Slice), chip(1, Slice)], 1),
        (vec![padded], 0),
        (vec![chip(1, Slice), outside], 1),
    ] {
        assert_eq!(
            check(list, vec![]),
            Err(LayoutInputError::InvalidDecorationSpan { span: index })
        );
    }
    // Nesting: containment is fine (also at equal edges), a crossing is not,
    // nor is going deeper than the limit.
    let span = |id, a, b| {
        let mut s = chip(id, Slice);
        s.byte_range = TextRange::new(a, b);
        s
    };
    assert!(check(vec![span(1, 0, 4), span(2, 1, 3), span(3, 1, 2)], vec![]).is_ok());
    assert!(check(vec![span(1, 0, 2), span(2, 2, 4)], vec![]).is_ok());
    assert_eq!(
        check(vec![span(1, 0, 3), span(2, 1, 4)], vec![]),
        Err(LayoutInputError::InvalidDecorationSpan { span: 1 })
    );
    let too_deep: Vec<_> = (0..=crate::frontend::MAX_DECORATION_NESTING_DEPTH)
        .map(|i| span(i as u64, 0, 4))
        .collect();
    assert_eq!(
        check(too_deep, vec![]),
        Err(LayoutInputError::InvalidDecorationSpan {
            span: crate::frontend::MAX_DECORATION_NESTING_DEPTH
        })
    );
    // Atomic spans are strictly disjoint: touching is fine, nesting is not.
    assert!(
        check(
            vec![],
            vec![
                atomic(1, TextRange::new(0, 2)),
                atomic(2, TextRange::new(2, 4))
            ]
        )
        .is_ok()
    );
    for (a, b) in [((0, 3), (2, 4)), ((0, 4), (1, 2))] {
        assert_eq!(
            check(
                vec![],
                vec![
                    atomic(1, TextRange::new(a.0, a.1)),
                    atomic(2, TextRange::new(b.0, b.1))
                ]
            ),
            Err(LayoutInputError::InvalidAtomicSpan { span: 1 })
        );
    }
    // Atomic: empty, outside the text, duplicate id.
    for (list, index) in [
        (vec![atomic(1, TextRange::empty_at(2))], 0),
        (vec![atomic(1, TextRange::new(1, 9))], 0),
        (
            vec![
                atomic(1, TextRange::new(0, 1)),
                atomic(1, TextRange::new(2, 3)),
            ],
            1,
        ),
    ] {
        assert_eq!(
            check(vec![], list),
            Err(LayoutInputError::InvalidAtomicSpan { span: index })
        );
    }
}

#[test]
fn atomic_span_edges_must_be_atom_boundaries() {
    let o = LayoutOptions::default();
    let build = |start, end| {
        let mut a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 4)])]);
        a.atomic_spans = vec![AtomicEditSpan {
            span_id: 1,
            byte_range: TextRange::new(start, end),
        }];
        a
    };
    let check = |a: ParagraphAst| check_layout_inputs(&a, "aéb", &constraints(), &o);
    assert!(check(build(1, 4)).is_ok());
    // Offset 2 is inside the atom 1..4 (a char boundary of "é" is 1 and 3,
    // so use 3: still strictly inside the atom).
    assert_eq!(
        check(build(1, 3)),
        Err(LayoutInputError::InvalidAtomicSpan { span: 0 })
    );
    assert_eq!(
        check(build(3, 4)),
        Err(LayoutInputError::InvalidAtomicSpan { span: 0 })
    );
}

#[test]
fn atomic_spans_wrap_content_not_whitespace() {
    let o = LayoutOptions::default();
    let ws = |kind: &str, range: (usize, usize)| match kind {
        "glue" => WordNode::Glue(GlueNode {
            original_bidi_class: crate::types::BidiClass::Ws,
            bidi_level: BidiLevel::LTR,
            is_collapsible: true,

            byte_range: TextRange::new(range.0, range.1),
            kind: GlueKind::WordSpace,
            natural_width: 3.0,
            max_stretch: 0.0,
            max_shrink: 0.0,
        }),
        _ => WordNode::Tab {
            byte_range: TextRange::new(range.0, range.1),
        },
    };
    // "ab", two spaces as one glue (2..4), "cd".
    let build = |kind: &str, ws_range: (usize, usize), span: (usize, usize)| {
        let mut a = with_items_for(
            6,
            vec![
                text_box(vec![atom(0, 1), atom(1, 2)]),
                ws(kind, ws_range),
                text_box(vec![atom(4, 5), atom(5, 6)]),
            ],
        );
        a.atomic_spans = vec![AtomicEditSpan {
            span_id: 1,
            byte_range: TextRange::new(span.0, span.1),
        }];
        a
    };
    let check = |a: ParagraphAst| check_layout_inputs(&a, "ab  cd", &constraints(), &o);
    let bad = Err(LayoutInputError::InvalidAtomicSpan { span: 0 });
    // Both edges at atom edges, with or without glue between them.
    assert!(check(build("glue", (2, 4), (0, 2))).is_ok());
    assert!(check(build("glue", (2, 4), (1, 5))).is_ok());
    assert!(check(build("glue", (2, 4), (0, 6))).is_ok());
    // An edge strictly inside the glue run, or the tab, is invalid ...
    assert_eq!(check(build("glue", (2, 4), (0, 3))), bad);
    assert_eq!(check(build("glue", (2, 4), (3, 6))), bad);
    assert_eq!(check(build("tab", (2, 4), (3, 5))), bad);
    // ... and so is a span made of whitespace alone (its inner edge is not
    // the edge of any atom or object), or one ending on an unowned offset.
    assert_eq!(check(build("glue", (2, 4), (2, 4))), bad);
    let mut a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 2)])]);
    a.atomic_spans = vec![AtomicEditSpan {
        span_id: 1,
        byte_range: TextRange::new(1, 3),
    }];
    assert_eq!(check_layout_inputs(&a, "abcd", &constraints(), &o), bad);
    // An inline object's edges count.
    let mut a = with_items(vec![
        text_box(vec![atom(0, 1)]),
        WordNode::Box(BoxNode::InlineObject(InlineObjectBox {
            bidi_level: BidiLevel::LTR,
            baseline_align: crate::types::BaselineAlignment::Auto,
            byte_range: TextRange::new(1, 3),
            box_id: 1,
            width: 1.0,
            ascent: 1.0,
            descent: 0.0,
            overflow_policy: BoxOverflowPolicy::Clip,
        })),
    ]);
    a.atomic_spans = vec![AtomicEditSpan {
        span_id: 1,
        byte_range: TextRange::new(1, 3),
    }];
    assert!(check_layout_inputs(&tiled(a), "abcd", &constraints(), &o).is_ok());
}

#[test]
fn strut_inputs_are_validated_after_resolution() {
    let negative = LayoutOptions {
        line_height: crate::middleend::LineHeight::Fixed(-1.0),
        ..Default::default()
    };
    assert_eq!(
        check_layout_inputs(&ast(), "", &constraints(), &negative),
        Err(LayoutInputError::InvalidStrut)
    );
    // Finite inputs whose resolved height overflows.
    let huge = LayoutOptions {
        line_height: crate::middleend::LineHeight::Fixed(f32::MAX),
        ..Default::default()
    };
    let mut a = ast();
    a.strut_font_size = 10.0;
    assert_eq!(
        check_layout_inputs(&a, "", &constraints(), &huge),
        Err(LayoutInputError::InvalidStrut)
    );
}
