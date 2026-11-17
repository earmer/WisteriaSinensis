//! Carets and movement on bidi lines (spec CRT-3, INT-5, INT-11, INT-13,
//! INT-20), and the gates' level checks on real compiled data (AST-13,
//! OUT-3).

use crate::acceptance_phase2::{glyph_at, glyphs};
use crate::common::*;
use mutil_engine::*;

fn pos(byte_offset: usize, affinity: CaretAffinity) -> CursorPosition {
    CursorPosition {
        byte_offset,
        affinity,
    }
}

fn down(byte_offset: usize) -> CursorPosition {
    pos(byte_offset, CaretAffinity::Downstream)
}

/// OUT-2's tolerance.
fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= f32::max(1e-3, 1e-5 * a.abs().max(b.abs()))
}

fn stops_at(line: &PositionedLine, offset: usize) -> Vec<CaretStop> {
    line.caret_stops
        .iter()
        .filter(|s| s.offset == offset)
        .copied()
        .collect()
}

#[test]
fn crt3_a_direction_boundary_has_two_stops_with_affinities_and_levels() {
    // LTR "abcאבג": offset 3 ends "c" (level 0) and starts א (level 1),
    // whose leading edge is the right end of the line.
    let text = "abcאבג";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let line = &p.lines[0];
    let c = glyph_at(line, 2);
    let alef = glyph_at(line, 3);
    let stops = stops_at(line, 3);
    assert_eq!(stops.len(), 2, "{stops:?}");
    let up = stops
        .iter()
        .find(|s| s.affinity == CaretAffinity::Upstream)
        .unwrap();
    let dn = stops
        .iter()
        .find(|s| s.affinity == CaretAffinity::Downstream)
        .unwrap();
    assert_close(up.x, c.x + c.advance);
    assert_eq!(up.level.get(), 0);
    assert_close(dn.x, alef.x + alef.advance);
    assert_eq!(dn.level.get(), 1);
    // RTL "אבגabc": offset 6 ends ג (level 1, trailing edge on its left) and
    // starts "a" (level 2, leading edge on its left, the line's left end).
    let text = "אבגabc";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let line = &p.lines[0];
    let stops = stops_at(line, 6);
    assert_eq!(stops.len(), 2, "{stops:?}");
    for s in &stops {
        let (x, level) = match s.affinity {
            CaretAffinity::Upstream => (glyph_at(line, 4).x, 1),
            CaretAffinity::Downstream => (glyph_at(line, 6).x, 2),
        };
        assert_close(s.x, x);
        assert_eq!(s.level.get(), level, "{s:?}");
    }
}

#[test]
fn int5_rtl_ligature_sub_slots_pair_gdef_carets_with_atoms_in_reverse() {
    // اللّٰه: Noto Naskh Arabic forms U+FDF2 (1190 units, GDEF carets at 453,
    // 692 and 960) over four atoms: ا (0..2), ل (2..4), لّٰ (4..10), ه
    // (10..12). From the cluster's left edge the carets are the boundaries
    // before ه, before لّٰ and before the second ل (SHP-7, INT-5).
    let text = "\u{627}\u{644}\u{644}\u{651}\u{670}\u{647}";
    let ast = compile_bidi(text, &[ARABIC]);
    assert_eq!(ast.atoms.len(), 4);
    let [cluster] = ast.clusters.as_slice() else {
        panic!("{:#?}", ast.clusters)
    };
    assert!(cluster.is_ligature());
    assert_eq!(cluster.atom_count(), 4);
    let ligature = glyph_for(RTL_FONTS[1].1, '\u{FDF2}').unwrap();
    assert_eq!(cluster_glyph_ids(&ast), [ligature]);
    assert_close(cluster.natural_width, 11.9);
    let p = lay_out_bidi_ast(&ast, text, &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    let line = &p.lines[0];
    let left = glyph_at(line, 0).x;
    for (offset, dx) in [(12, 0.0), (10, 4.53), (4, 6.92), (2, 9.60), (0, 11.9)] {
        let stops = stops_at(line, offset);
        assert_eq!(stops.len(), 1, "{offset}: {stops:?}");
        assert_close(stops[0].x, left + dx);
        assert_close(l.caret_geometry(down(offset)).primary_rect.x, left + dx);
    }
    // The stops are in visual order, so the offsets fall left to right.
    let offsets: Vec<usize> = line.caret_stops.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, [12, 10, 4, 2, 0]);
}

fn cluster_glyph_ids(ast: &ParagraphAst) -> Vec<u32> {
    ast.glyphs.iter().map(|g| g.glyph_id).collect()
}

#[test]
fn int11_the_primary_caret_follows_the_affinity_and_the_secondary_the_other() {
    let text = "abcאבג";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    let line = &p.lines[0];
    let x = |affinity| {
        line.caret_stops
            .iter()
            .find(|s| s.offset == 3 && s.affinity == affinity)
            .unwrap()
            .x
    };
    let (up, dn) = (x(CaretAffinity::Upstream), x(CaretAffinity::Downstream));
    for (affinity, primary, secondary) in [
        (CaretAffinity::Upstream, up, dn),
        (CaretAffinity::Downstream, dn, up),
    ] {
        let caret = l.caret_geometry(pos(3, affinity));
        assert_close(caret.primary_rect.x, primary);
        assert_eq!(caret.primary_rect.width, 0.0);
        let other = caret.secondary_rect.expect("a split caret");
        assert_close(other.x, secondary);
        assert_close(other.y, caret.primary_rect.y);
        assert_close(other.height, line.line_height);
    }
    // One position: no secondary caret.
    assert!(l.caret_geometry(down(1)).secondary_rect.is_none());
    assert!(l.caret_geometry(down(5)).secondary_rect.is_none());
}

/// INT-13's visual step over `line`'s stop table from the stop at `x`.
fn expected_step(line: &PositionedLine, x: f32, right: bool) -> Option<CaretStop> {
    let ahead = line
        .caret_stops
        .iter()
        .filter(|s| !approx(s.x, x) && (s.x > x) == right);
    let target = if right {
        ahead.map(|s| s.x).reduce(f32::min)?
    } else {
        ahead.map(|s| s.x).reduce(f32::max)?
    };
    let mut at_target = line.caret_stops.iter().filter(|s| approx(s.x, target));
    if right {
        at_target.next().copied()
    } else {
        at_target.next_back().copied()
    }
}

#[test]
fn int13_visual_arrows_skip_coincident_stops() {
    for text in ["abc אבג", "abcאבג", "אבג abc 123 דהו", "אבג abc \tdef"] {
        let p = lay_out_bidi(text, &[HEBREW], &column(300.0), &LayoutOptions::default());
        assert_eq!(p.lines.len(), 1, "{text}");
        let l = ValidatedLayout::new(&p, text).unwrap();
        let line = &p.lines[0];
        let rtl = p.base_direction == Direction::Rtl;
        for stop in &line.caret_stops {
            let twins = line
                .caret_stops
                .iter()
                .filter(|s| s.offset == stop.offset && s.affinity == stop.affinity)
                .count();
            if twins > 1 {
                continue;
            }
            let from = pos(stop.offset, stop.affinity);
            for (motion, right) in [(Motion::Right, true), (Motion::Left, false)] {
                let got = l.move_cursor(from, motion, None);
                let expected = match expected_step(line, stop.x, right) {
                    Some(s) => CursorMove::Moved(pos(s.offset, s.affinity)),
                    // Past the edge of the only line: towards the line start
                    // of the base direction exits at the start.
                    None => CursorMove::Exited {
                        edge: if right == rtl {
                            ParagraphEdge::Start
                        } else {
                            ParagraphEdge::End
                        },
                        inline_position: None,
                    },
                };
                assert_eq!(got, expected, "{text}: {motion:?} from {stop:?}");
            }
        }
    }
}

#[test]
fn int13_arrows_exit_by_the_visual_stops_not_by_offsets() {
    // LTR "abcאבג": the rightmost stop is offset 3, and Right from it leaves
    // the paragraph at its end although 3 is not the text's end.
    let text = "abcאבג";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    let exit = |edge| CursorMove::Exited {
        edge,
        inline_position: None,
    };
    assert_eq!(
        l.move_cursor(down(3), Motion::Right, None),
        exit(ParagraphEdge::End)
    );
    assert_eq!(
        l.move_cursor(down(0), Motion::Left, None),
        exit(ParagraphEdge::Start)
    );
    // RTL: right is the line start.
    let text = "אבג";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    assert_eq!(
        l.move_cursor(down(0), Motion::Right, None),
        exit(ParagraphEdge::Start)
    );
    assert_eq!(moved(l.move_cursor(down(0), Motion::Left, None)), 2);
    let leftmost = p.lines[0].extreme_caret_stop(HangSide::Left).unwrap();
    assert_eq!(leftmost.offset, 6);
    assert_eq!(
        l.move_cursor(pos(6, leftmost.affinity), Motion::Left, None),
        exit(ParagraphEdge::End)
    );
}

#[test]
fn int13_visual_arrows_across_soft_wrap_with_bidi_boundary_advance() {
    // Wrap right before an RTL run ("אבג") or an LTR run ("abc") so the
    // second line begins at a bidi direction boundary. Stepping Right from
    // start to end and Left from end to start must never stall on coincident
    // stops and must cross the wrap cleanly.
    for (text, wrap_width, expected_lines) in [
        ("abc אבג", 25.0, ["abc ", "אבג"].as_slice()),
        ("abc אבג def", 25.0, ["abc ", "אבג ", "def"].as_slice()),
        ("אבג abc", 25.0, ["אבג ", "abc"].as_slice()),
        ("אבג abc דהו", 25.0, ["אבג ", "abc ", "דהו"].as_slice()),
    ] {
        let p = lay_out_bidi(
            text,
            &[HEBREW],
            &column(wrap_width),
            &LayoutOptions::default(),
        );
        assert_eq!(line_texts(&p, text), expected_lines, "{text:?}");
        let l = ValidatedLayout::new(&p, text).unwrap();

        let total_stops: usize = p.lines.iter().map(|line| line.caret_stops.len()).sum();
        let line0_start = p.lines[0].caret_stops.first().copied().unwrap();
        let mut cur = pos(line0_start.offset, line0_start.affinity);
        let mut seen = vec![cur];
        let mut exited_right = false;
        for _ in 0..=total_stops {
            match l.move_cursor(cur, Motion::Right, None) {
                CursorMove::Moved(next) => {
                    assert!(
                        !seen.contains(&next),
                        "{text:?}: Right stalled or looped at {next:?} (history: {seen:?})"
                    );
                    seen.push(next);
                    cur = next;
                }
                CursorMove::Exited { .. } => {
                    exited_right = true;
                    break;
                }
            }
        }
        assert!(exited_right, "{text:?}: Right did not exit: {seen:?}");
        assert!(seen.len() >= 2, "{text:?}: too few stops visited");

        let last_line = p.lines.last().unwrap();
        let last_stop = last_line.caret_stops.last().copied().unwrap();
        let mut cur = pos(last_stop.offset, last_stop.affinity);
        let mut seen = vec![cur];
        let mut exited_left = false;
        for _ in 0..=total_stops {
            match l.move_cursor(cur, Motion::Left, None) {
                CursorMove::Moved(next) => {
                    assert!(
                        !seen.contains(&next),
                        "{text:?}: Left stalled or looped at {next:?} (history: {seen:?})"
                    );
                    seen.push(next);
                    cur = next;
                }
                CursorMove::Exited { .. } => {
                    exited_left = true;
                    break;
                }
            }
        }
        assert!(exited_left, "{text:?}: Left did not exit: {seen:?}");
        assert!(seen.len() >= 2, "{text:?}: too few stops visited");
    }
}

#[test]
fn int16_selection_geometry_across_bidi_boundary_uses_non_zero_width_slot_level() {
    // LTR base (level 0) + RTL Hebrew (level 1): boundary at byte 3.
    let text = "abcאבג";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();

    // Selecting only the RTL side starting at the boundary (3..9) must carry level 1,
    // not a zero-width boundary slot's level 0.
    let rtl_rects =
        l.selection_geometry(&*BIDI_SHAPER, TextRange::new(3, 9), BoxHeightStyle::Tight);
    assert_eq!(rtl_rects.len(), 1, "{rtl_rects:?}");
    assert_eq!(rtl_rects[0].level.get(), 1);
    assert!(rtl_rects[0].rect.width > 0.0);

    // Selecting only the LTR side ending at the boundary (0..3) must carry level 0.
    let ltr_rects =
        l.selection_geometry(&*BIDI_SHAPER, TextRange::new(0, 3), BoxHeightStyle::Tight);
    assert_eq!(ltr_rects.len(), 1, "{ltr_rects:?}");
    assert_eq!(ltr_rects[0].level.get(), 0);
    assert!(ltr_rects[0].rect.width > 0.0);

    // Selecting across the boundary (2..5 = "cא"): visually separated into two
    // non-zero-width pieces ("c" at level 0 and "א" at level 1).
    let cross_rects =
        l.selection_geometry(&*BIDI_SHAPER, TextRange::new(2, 5), BoxHeightStyle::Tight);
    assert_eq!(cross_rects.len(), 2, "{cross_rects:?}");
    let levels: Vec<u8> = cross_rects.iter().map(|r| r.level.get()).collect();
    assert_eq!(levels, [0, 1]);

    // RTL base (level 1) + embedded LTR (level 2): boundary at byte 6.
    let rtl_text = "אבגabc";
    let p2 = lay_out_bidi(
        rtl_text,
        &[HEBREW],
        &column(100.0),
        &LayoutOptions::default(),
    );
    let l2 = ValidatedLayout::new(&p2, rtl_text).unwrap();
    let emb_ltr = l2.selection_geometry(&*BIDI_SHAPER, TextRange::new(6, 9), BoxHeightStyle::Tight);
    assert_eq!(emb_ltr.len(), 1, "{emb_ltr:?}");
    assert_eq!(emb_ltr[0].level.get(), 2);
}

fn moved(m: CursorMove) -> usize {
    match m {
        CursorMove::Moved(p) => p.byte_offset,
        CursorMove::Exited { .. } => panic!("unexpected exit: {m:?}"),
    }
}

#[test]
fn logical_and_word_motions_stay_logical_in_bidi_text() {
    let text = "אבג דהו";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    assert_eq!(moved(l.move_cursor(down(0), Motion::Next, None)), 2);
    assert_eq!(moved(l.move_cursor(down(4), Motion::Previous, None)), 2);
    assert!(matches!(
        l.move_cursor(down(0), Motion::Previous, None),
        CursorMove::Exited {
            edge: ParagraphEdge::Start,
            ..
        }
    ));
    assert!(matches!(
        l.move_cursor(down(13), Motion::Next, None),
        CursorMove::Exited {
            edge: ParagraphEdge::End,
            ..
        }
    ));
    assert_eq!(moved(l.move_cursor(down(0), Motion::WordNext, None)), 6);
    assert_eq!(
        moved(l.move_cursor(down(13), Motion::WordPrevious, None)),
        7
    );
    // WordLeft and WordRight resolve through the base direction: in RTL,
    // left is logically forward.
    assert_eq!(moved(l.move_cursor(down(0), Motion::WordLeft, None)), 6);
    assert_eq!(moved(l.move_cursor(down(13), Motion::WordRight, None)), 7);
    // LineStart and LineEnd never exit.
    assert!(matches!(
        l.move_cursor(down(0), Motion::LineStart, None),
        CursorMove::Moved(_)
    ));
    assert_eq!(moved(l.move_cursor(down(0), Motion::LineEnd, None)), 13);
    // On a mixed line, Next crosses the direction boundary logically.
    let text = "abcאבג";
    let p = lay_out_bidi(text, &[HEBREW], &column(100.0), &LayoutOptions::default());
    let l = ValidatedLayout::new(&p, text).unwrap();
    assert_eq!(moved(l.move_cursor(down(2), Motion::Next, None)), 3);
    assert_eq!(moved(l.move_cursor(down(3), Motion::Next, None)), 5);
}

#[test]
fn int20_a_soft_wrapped_rtl_line_ends_upstream_at_its_left_edge() {
    let text = "אבג דהו";
    let p = lay_out_bidi(text, &[HEBREW], &column(20.0), &LayoutOptions::default());
    assert_eq!(line_texts(&p, text), ["אבג ", "דהו"]);
    let line = &p.lines[0];
    assert_eq!(line.visible_byte_end, 6);
    let last = stops_at(line, 6);
    assert_eq!(last.len(), 1, "{last:?}");
    assert_eq!(last[0].affinity, CaretAffinity::Upstream);
    let left = glyphs(line).map(|g| g.x).fold(f32::INFINITY, f32::min);
    assert_close(last[0].x, left);
    // Under `Collapsible` the wrapped line has no stop past its visible end.
    assert!(line.caret_stops.iter().all(|s| s.offset <= 6));
}

#[test]
fn the_input_gate_rejects_levels_below_the_rtl_base_level() {
    // AST-13 on a compiled RTL paragraph: a glue, a text box's run (with its
    // atoms and clusters, so they still agree) or an inline object below
    // level 1 is `InvalidBidiLevel` at that item.
    let text = "אב cd \u{FFFC}";
    let style = script_style(&[HEBREW], 10.0);
    let objects = [InlineObjectInput {
        byte_range: TextRange::new(8, 11),
        box_id: 1,
        width: 5.0,
        ascent: 5.0,
        descent: 1.0,
        baseline_align: BaselineAlignment::Auto,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    let paragraph = ParagraphInput {
        inline_objects: &objects,
        ..input(text, &style)
    };
    let ast = compile_bidi_input(&paragraph, &FrontendOptions::default());
    assert_eq!(ast.base_direction, Direction::Rtl);
    let (constraints, options) = (column(200.0), LayoutOptions::default());
    let check = |a: &ParagraphAst| check_layout_inputs(a, text, &constraints, &options);
    assert_eq!(check(&ast), Ok(()));
    let find = |pred: &dyn Fn(&WordNode) -> bool| ast.items.iter().position(pred).unwrap();
    let glue = find(&|n| matches!(n, WordNode::Glue(_)));
    let mut a = ast.clone();
    let WordNode::Glue(g) = &mut a.items[glue] else {
        unreachable!()
    };
    g.bidi_level = BidiLevel::LTR;
    assert_eq!(
        check(&a),
        Err(LayoutInputError::InvalidBidiLevel { item: glue })
    );
    let object = find(&|n| matches!(n, WordNode::Box(BoxNode::InlineObject { .. })));
    let mut a = ast.clone();
    let WordNode::Box(BoxNode::InlineObject { bidi_level, .. }) = &mut a.items[object] else {
        unreachable!()
    };
    *bidi_level = BidiLevel::LTR;
    assert_eq!(
        check(&a),
        Err(LayoutInputError::InvalidBidiLevel { item: object })
    );
    // The Hebrew text box's run, atoms and clusters, all lowered to 0.
    let hebrew = find(&|n| matches!(n, WordNode::Box(BoxNode::Text { .. })));
    let mut a = ast.clone();
    let WordNode::Box(BoxNode::Text { run_index, .. }) = a.items[hebrew].clone() else {
        unreachable!()
    };
    let run = &mut a.runs[run_index];
    run.bidi_level = BidiLevel::LTR;
    let (atoms, clusters) = (run.atom_range.clone(), run.cluster_range.clone());
    for atom in &mut a.atoms[atoms] {
        atom.bidi_level = BidiLevel::LTR;
    }
    for cluster in &mut a.clusters[clusters] {
        cluster.bidi_level = BidiLevel::LTR;
    }
    assert_eq!(
        check(&a),
        Err(LayoutInputError::InvalidBidiLevel { item: hebrew })
    );
}

#[test]
fn the_input_gate_rejects_a_span_boundary_below_the_base_level() {
    // A boundary below the base level breaks both WN-8 (its level is not its
    // span's first entity's) and AST-13; either error names the boundary.
    let text = "אבג דהו";
    let style = script_style(&[HEBREW], 10.0);
    let spans = [InlineDecorationSpan {
        span_id: 1,
        byte_range: TextRange::new(7, 13),
        start_padding: 1.0,
        end_padding: 1.0,
        break_mode: BoxDecorationBreak::Slice,
    }];
    let paragraph = ParagraphInput {
        decoration_spans: &spans,
        ..input(text, &style)
    };
    let mut ast = compile_bidi_input(&paragraph, &FrontendOptions::default());
    let (constraints, options) = (column(200.0), LayoutOptions::default());
    assert_eq!(
        check_layout_inputs(&ast, text, &constraints, &options),
        Ok(())
    );
    let item = ast
        .items
        .iter()
        .position(|n| matches!(n, WordNode::Box(BoxNode::SpanBoundary { .. })))
        .unwrap();
    let WordNode::Box(BoxNode::SpanBoundary { bidi_level, .. }) = &mut ast.items[item] else {
        unreachable!()
    };
    *bidi_level = BidiLevel::LTR;
    let error = check_layout_inputs(&ast, text, &constraints, &options).unwrap_err();
    assert!(
        error == LayoutInputError::InvalidBidiLevel { item }
            || error == LayoutInputError::InvalidSpanBoundary { item },
        "{error:?}"
    );
}

#[test]
fn the_output_gate_rejects_visual_runs_out_of_screen_order() {
    // OUT-3, LV-2: a mixed line has several visual runs; swapping two of them
    // keeps each run's own order but breaks the order across runs.
    let text = "abc אבג def";
    let mut p = lay_out_bidi(text, &[HEBREW], &column(200.0), &LayoutOptions::default());
    let runs = &mut p.lines[0].visual_runs;
    assert!(runs.len() >= 2, "{runs:#?}");
    runs.swap(0, 1);
    match ValidatedLayout::new(&p, text) {
        Err((0, LineInvariantError::VisualRunsOutOfOrder { run })) => assert_eq!(run, 1),
        other => panic!("{:?}", other.err()),
    }
}
