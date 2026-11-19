//! Gate completeness (ADR-0004 rule 5): for every hand-built
//! `PositionedParagraph` the output gate G4 accepts, every §6 query is total
//! over arbitrary query arguments (ADR-0004 rule 4). It neither panics nor
//! answers a non-finite coordinate or a rectangle of negative size.
//!
//! The paragraphs are perturbations of a valid three-line fixture: random
//! caret-stop tables (empty ones included), offsets, `x` values (huge, NaN,
//! negative), baselines up to 1e9, atomic fragments and an atomic chip that
//! leaves lines stopless, and a clamp. Most perturbations are rejected; the
//! property only speaks about the accepted ones.

use crate::common::*;
use mutil_engine::*;
use proptest::prelude::*;
use std::panic::{AssertUnwindSafe, catch_unwind};

const TEXTS: [&str; 2] = ["abcdef", "ab\u{e9}ef"];

/// Floats that break naive arithmetic, and a few ordinary ones.
fn wild_f32() -> impl Strategy<Value = f32> {
    prop_oneof![
        3 => prop::sample::select(vec![
            0.0_f32, 1e-4, 25.0, 49.9999, 50.0, 50.0001, 99.9999, 100.0, -0.0, -1.0, -100.0,
            1e6, 1e9, -1e9, 1e38, f32::MAX, f32::MIN, f32::NAN, f32::INFINITY,
            f32::NEG_INFINITY, f32::MIN_POSITIVE,
        ]),
        1 => -200.0_f32..300.0,
        1 => any::<f32>(),
    ]
}

/// Positions in a line's bytes, past them and past the text.
fn offset() -> impl Strategy<Value = usize> {
    0usize..10
}

#[derive(Clone, Debug)]
struct StopSpec {
    x: f32,
    offset: usize,
    upstream: bool,
    rtl: bool,
}

fn stop_spec() -> impl Strategy<Value = StopSpec> {
    (
        wild_f32(),
        offset(),
        any::<bool>(),
        prop::bool::weighted(0.1),
    )
        .prop_map(|(x, offset, upstream, rtl)| StopSpec {
            x,
            offset,
            upstream,
            rtl,
        })
}

/// One change to the valid fixture.
#[derive(Clone, Debug)]
enum Perturbation {
    /// Replaces line `line`'s stop table (sorted by `x` when `sorted`).
    Stops {
        line: usize,
        stops: Vec<StopSpec>,
        sorted: bool,
    },
    /// Moves stop `index` of `line` to `x`.
    StopX { line: usize, index: usize, x: f32 },
    /// Changes the offset of stop `index` of `line`.
    StopOffset {
        line: usize,
        index: usize,
        offset: usize,
    },
    /// Moves every line down by `by` (and the block size with them).
    Shift { by: f32 },
    /// Sets one line's baseline.
    Baseline { line: usize, y: f32 },
    /// Moves glyph `glyph` of `line` to `x`.
    GlyphX { line: usize, glyph: usize, x: f32 },
    /// Adds an atomic fragment to `line`.
    Fragment {
        line: usize,
        full: (usize, usize),
        bytes: (usize, usize),
        x: f32,
        width: f32,
    },
    /// Clamps the paragraph to its first two lines.
    Clamp,
    /// Sets one line's ascent and descent.
    Metrics {
        line: usize,
        ascent: f32,
        descent: f32,
    },
}

fn perturbation() -> impl Strategy<Value = Perturbation> {
    let line = 0usize..3;
    prop_oneof![
        (
            line.clone(),
            prop::collection::vec(stop_spec(), 0..5),
            any::<bool>()
        )
            .prop_map(|(line, stops, sorted)| Perturbation::Stops {
                line,
                stops,
                sorted
            }),
        (line.clone(), 0usize..4, wild_f32()).prop_map(|(line, index, x)| Perturbation::StopX {
            line,
            index,
            x
        }),
        (line.clone(), 0usize..4, offset()).prop_map(|(line, index, offset)| {
            Perturbation::StopOffset {
                line,
                index,
                offset,
            }
        }),
        prop::sample::select(vec![1e3_f32, 1e5, 1e6, 1e7, 3e7, 1e8, 1e9, -10.0])
            .prop_map(|by| Perturbation::Shift { by }),
        (line.clone(), wild_f32()).prop_map(|(line, y)| Perturbation::Baseline { line, y }),
        (line.clone(), 0usize..2, wild_f32()).prop_map(|(line, glyph, x)| Perturbation::GlyphX {
            line,
            glyph,
            x
        }),
        (
            line.clone(),
            (offset(), offset()),
            (offset(), offset()),
            wild_f32(),
            wild_f32()
        )
            .prop_map(|(line, full, bytes, x, width)| Perturbation::Fragment {
                line,
                full,
                bytes,
                x,
                width
            }),
        Just(Perturbation::Clamp),
        (line, wild_f32(), wild_f32()).prop_map(|(line, ascent, descent)| {
            Perturbation::Metrics {
                line,
                ascent,
                descent,
            }
        }),
    ]
}

/// The inline position of byte boundary `offset` on a line starting at
/// `start`: 50 units a byte.
fn x_of(start: usize, offset: usize) -> f32 {
    50.0 * (offset - start) as f32
}

/// Line `i` of `text` (bytes `2i..2i + 2`): one glyph per character, a stop
/// per character boundary not strictly inside `chip`, and the chip's
/// fragment where it crosses the line.
fn fixture_line(text: &str, i: usize, chip: Option<(usize, usize)>) -> PositionedLine {
    let (start, end) = (2 * i, 2 * i + 2);
    let mut l = line();
    l.byte_range = TextRange::new(start, end);
    l.visible_byte_end = end;
    l.baseline_y = 8.0 + 10.0 * i as f32;
    let bounds: Vec<usize> = (start..=end)
        .filter(|&o| text.is_char_boundary(o))
        .collect();
    let glyphs: Vec<PositionedGlyph> = bounds
        .windows(2)
        .map(|w| PositionedGlyph {
            byte_range: Some(TextRange::new(w[0], w[1])),
            ..glyph(x_of(start, w[0]), x_of(start, w[1]) - x_of(start, w[0]))
        })
        .collect();
    let mut r = run(glyphs[0]);
    r.byte_range = l.byte_range;
    r.glyphs = glyphs;
    l.visual_runs = vec![r];
    let interior = |o: usize| chip.is_some_and(|(s, e)| s < o && o < e);
    l.caret_stops = bounds
        .iter()
        .filter(|&&o| !interior(o))
        .map(|&o| CaretStop {
            x: x_of(start, o),
            offset: o,
            affinity: if o == end {
                CaretAffinity::Upstream
            } else {
                CaretAffinity::Downstream
            },
            level: BidiLevel::LTR,
        })
        .collect();
    if let Some((s, e)) = chip {
        let (a, b) = (s.max(start), e.min(end));
        if a < b {
            l.atomic_fragments = vec![AtomicFragment {
                atomic_id: 7,
                full_byte_range: TextRange::new(s, e),
                fragment_byte_range: TextRange::new(a, b),
                x: x_of(start, a),
                width: x_of(start, b) - x_of(start, a),
                bidi_level: BidiLevel::LTR,
            }];
        }
    }
    l
}

fn fixture(text: &str, chip: Option<(usize, usize)>) -> PositionedParagraph {
    let lines = (0..3).map(|i| fixture_line(text, i, chip)).collect();
    let mut p = paragraph(&ctx(), lines);
    p.total_block_size = 30.0;
    p
}

fn stop_of(spec: &StopSpec) -> CaretStop {
    CaretStop {
        x: spec.x,
        offset: spec.offset,
        affinity: if spec.upstream {
            CaretAffinity::Upstream
        } else {
            CaretAffinity::Downstream
        },
        level: if spec.rtl {
            BidiLevel::RTL
        } else {
            BidiLevel::LTR
        },
    }
}

fn range(a: usize, b: usize) -> TextRange {
    TextRange::new(a.min(b), a.max(b))
}

fn apply(p: &mut PositionedParagraph, change: &Perturbation) {
    let n = p.lines.len();
    match change {
        Perturbation::Stops {
            line,
            stops,
            sorted,
        } => {
            let mut stops: Vec<CaretStop> = stops.iter().map(stop_of).collect();
            if *sorted {
                stops.sort_by(|a, b| a.x.total_cmp(&b.x));
            }
            p.lines[line % n].caret_stops = stops;
        }
        Perturbation::StopX { line, index, x } => {
            if let Some(s) = p.lines[line % n].caret_stops.get_mut(*index) {
                s.x = *x;
            }
        }
        Perturbation::StopOffset {
            line,
            index,
            offset,
        } => {
            if let Some(s) = p.lines[line % n].caret_stops.get_mut(*index) {
                s.offset = *offset;
            }
        }
        Perturbation::Shift { by } => {
            for l in &mut p.lines {
                l.baseline_y += by;
            }
            p.total_block_size += by;
        }
        Perturbation::Baseline { line, y } => p.lines[line % n].baseline_y = *y,
        Perturbation::GlyphX { line, glyph, x } => {
            let glyphs = &mut p.lines[line % n].visual_runs[0].glyphs;
            if let Some(g) = glyphs.get_mut(*glyph) {
                g.x = *x;
            }
        }
        Perturbation::Fragment {
            line,
            full,
            bytes,
            x,
            width,
        } => p.lines[line % n].atomic_fragments.push(AtomicFragment {
            atomic_id: 9,
            full_byte_range: range(full.0, full.1),
            fragment_byte_range: range(bytes.0, bytes.1),
            x: *x,
            width: *width,
            bidi_level: BidiLevel::LTR,
        }),
        Perturbation::Clamp => {
            if n == 3 {
                p.lines.pop();
                p.total_block_size -= 10.0;
                p.truncated_range = Some(TextRange::new(4, 6));
                p.policy.clamp = Some(LineClampPolicy::max_lines(2));
                p.lines[1].is_ellipsized = true;
            }
        }
        Perturbation::Metrics {
            line,
            ascent,
            descent,
        } => {
            let l = &mut p.lines[line % n];
            l.ascent = *ascent;
            l.descent = *descent;
        }
    }
}

/// A chip on character boundaries of `text`, or none.
fn chip(text: &'static str) -> impl Strategy<Value = Option<(usize, usize)>> {
    let bounds: Vec<usize> = (0..=text.len())
        .filter(|&o| text.is_char_boundary(o))
        .collect();
    prop::option::weighted(
        0.4,
        (
            prop::sample::select(bounds.clone()),
            prop::sample::select(bounds),
        )
            .prop_filter_map("a non-empty chip", |(a, b)| (a < b).then_some((a, b))),
    )
}

#[derive(Debug)]
struct Case {
    text: &'static str,
    chip: Option<(usize, usize)>,
    changes: Vec<Perturbation>,
}

fn case() -> impl Strategy<Value = Case> {
    prop::sample::select(TEXTS.to_vec()).prop_flat_map(|text| {
        (chip(text), prop::collection::vec(perturbation(), 0..4)).prop_map(
            move |(chip, changes)| Case {
                text,
                chip,
                changes,
            },
        )
    })
}

#[derive(Debug)]
struct Queries {
    points: Vec<(f32, f32)>,
    inline: Vec<(usize, f32)>,
    offsets: Vec<usize>,
    preferred: Option<f32>,
    ranges: Vec<(usize, usize)>,
}

fn queries() -> impl Strategy<Value = Queries> {
    (
        prop::collection::vec((wild_f32(), wild_f32()), 1..6),
        prop::collection::vec((0usize..4, wild_f32()), 1..6),
        prop::collection::vec(offset(), 1..6),
        prop::option::of(wild_f32()),
        prop::collection::vec((offset(), offset()), 1..4),
    )
        .prop_map(|(points, inline, offsets, preferred, ranges)| Queries {
            points,
            inline,
            offsets,
            preferred,
            ranges,
        })
}

/// Runs `query`, turning a panic into a failure that names the query.
#[track_caller]
fn total<T>(what: &str, query: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(query)) {
        Ok(answer) => answer,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default();
            panic!("{what} panicked on a paragraph G4 accepted: {message}")
        }
    }
}

#[track_caller]
fn assert_rect(rect: Rect, what: &str) {
    assert!(
        [rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|v| v.is_finite()),
        "{what}: non-finite {rect:?}"
    );
    assert!(
        rect.width >= 0.0 && rect.height >= 0.0,
        "{what}: negative {rect:?}"
    );
}

#[track_caller]
fn assert_position(text: &str, p: CursorPosition, what: &str) {
    assert!(
        p.byte_offset <= text.len() && text.is_char_boundary(p.byte_offset),
        "{what}: {p:?} is not a caret position of {text:?}"
    );
}

const MOTIONS: [Motion; 12] = [
    Motion::Left,
    Motion::Right,
    Motion::Previous,
    Motion::Next,
    Motion::WordLeft,
    Motion::WordRight,
    Motion::WordPrevious,
    Motion::WordNext,
    Motion::LineStart,
    Motion::LineEnd,
    Motion::Up,
    Motion::Down,
];

fn check_layout_queries(l: &ValidatedLayout<'_>, q: &Queries) {
    let text = l.text();
    // A block arrow exits with the caller's own preferred position (INT-15):
    // a non-finite one handed in comes back unchanged, which is not geometry
    // the engine computed.
    let echoes = |exit: Option<f32>| matches!((exit, q.preferred), (Some(a), Some(b)) if a.to_bits() == b.to_bits());
    for &(x, y) in &q.points {
        let what = format!("hit_test({x}, {y})");
        let hit = total(&what, || l.hit_test(Point { x, y }));
        assert_position(text, hit.position, &what);
        if x.is_nan() {
            assert!(!hit.inside, "{what}: {hit:?}");
        }
    }
    for &(line, x) in &q.inline {
        let what = format!("hit_test_on_line({line}, {x})");
        let hit = total(&what, || l.hit_test_on_line(line, x));
        assert_position(text, hit.position, &what);
        if x.is_nan() {
            assert!(!hit.inside, "{what}: {hit:?}");
        }
    }
    for &offset in &q.offsets {
        let what = format!("rect_for_offset({offset})");
        assert_rect(total(&what, || l.rect_for_offset(offset)), &what);
        for affinity in [CaretAffinity::Downstream, CaretAffinity::Upstream] {
            let at = CursorPosition {
                byte_offset: offset,
                affinity,
            };
            let what = format!("caret_geometry({at:?})");
            let caret = total(&what, || l.caret_geometry(at));
            assert_rect(caret.primary_rect, &what);
            if let Some(secondary) = caret.secondary_rect {
                assert_rect(secondary, &what);
            }
            for motion in MOTIONS {
                let what = format!("move_cursor({at:?}, {motion:?}, {:?})", q.preferred);
                match total(&what, || l.move_cursor(at, motion, q.preferred)) {
                    // A move with nowhere to go MAY leave the caller's
                    // position as it was, whatever it was.
                    CursorMove::Moved(to) if to == at => {}
                    CursorMove::Moved(to) => assert_position(text, to, &what),
                    CursorMove::Exited {
                        inline_position, ..
                    } => assert!(
                        inline_position.is_none_or(f32::is_finite) || echoes(inline_position),
                        "{what}: {inline_position:?}"
                    ),
                }
            }
        }
    }
    for &(a, b) in &q.ranges {
        let r = range(a, b);
        for height in [
            BoxHeightStyle::Tight,
            BoxHeightStyle::Max,
            BoxHeightStyle::Strut,
        ] {
            let what = format!("selection_geometry({r:?}, {height:?})");
            for piece in total(&what, || l.selection_geometry(&TEST_SHAPER, r, height)) {
                assert_rect(piece.rect, &what);
            }
        }
        let what = format!("range_rects({r:?})");
        for rect in total(&what, || l.range_rects(r)) {
            assert_rect(rect, &what);
        }
    }
    let placed = total("glyph_runs", || {
        l.glyph_runs()
            .map(|run| (run.font_size(), run.glyphs().collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    });
    for (font_size, glyphs) in placed {
        assert!(font_size.is_finite(), "glyph_runs: font size {font_size}");
        for g in glyphs {
            assert!(
                g.origin.x.is_finite() && g.origin.y.is_finite(),
                "glyph_runs: {g:?}"
            );
        }
    }
}

/// An AST of `text`: one atom and one cluster per character, in one box,
/// with `atomic_spans`.
fn editing_ast(text: &str, atomic_spans: Vec<AtomicEditSpan>) -> ParagraphAst {
    let mut a = ast(text);
    rebuild_items(&mut a, |a| {
        let atoms: Vec<GraphemeAtom> = text
            .char_indices()
            .map(|(i, c)| GraphemeAtom {
                byte_range: TextRange::new(i, i + c.len_utf8()),
                ..atom(i)
            })
            .collect();
        let clusters = atoms
            .iter()
            .enumerate()
            .map(|(local, atom)| ShapedCluster {
                byte_range: atom.byte_range,
                ..cluster(0, local)
            })
            .collect();
        vec![push_text(a, 0, atoms, clusters), end(text.len())]
    });
    a.atomic_spans = atomic_spans;
    a
}

/// The editing queries of the input gate over `text`, with `chip` as an
/// atomic span: total over every offset and range.
fn check_editing_queries(text: &str, chip: Option<(usize, usize)>, q: &Queries) {
    let spans: Vec<AtomicEditSpan> = chip
        .map(|(s, e)| AtomicEditSpan {
            span_id: 1,
            byte_range: TextRange::new(s, e),
        })
        .into_iter()
        .collect();
    let ast = editing_ast(text, spans);
    let (constraints, options) = (constraints(), LayoutOptions::default());
    let inputs = ValidatedInputs::try_from_ast(&ast, text, &constraints, &options)
        .unwrap_or_else(|e| panic!("the editing fixture of {text:?}, chip {chip:?}: {e}"));
    let on_boundaries = |r: TextRange| {
        r.end() <= text.len() && text.is_char_boundary(r.start()) && text.is_char_boundary(r.end())
    };
    for &offset in &q.offsets {
        let what = format!("delete_backward_range({offset})");
        let r = total(&what, || inputs.delete_backward_range(offset));
        assert!(on_boundaries(r), "{what}: {r:?}");
        let what = format!("delete_forward_range({offset})");
        let r = total(&what, || inputs.delete_forward_range(offset));
        assert!(on_boundaries(r), "{what}: {r:?}");
    }
    for &(a, b) in &q.ranges {
        let r = range(a, b);
        let what = format!("copied_text({r:?})");
        let copied = total(&what, || inputs.copied_text(r));
        assert!(text.contains(copied.as_str()), "{what}: {copied:?}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn adr4_queries_are_total_on_every_paragraph_g4_accepts(c in case(), q in queries()) {
        let mut p = fixture(c.text, c.chip);
        for change in &c.changes {
            apply(&mut p, change);
        }
        if let Ok(l) = ValidatedLayout::try_from_paragraph(&p, c.text) {
            check_layout_queries(&l, &q);
        }
        check_editing_queries(c.text, c.chip, &q);
    }
}

#[test]
fn the_unperturbed_fixtures_pass_the_output_gate() {
    // The property is only as strong as what G4 accepts: the fixture and
    // its chip variants, stopless lines included, must be accepted.
    for text in TEXTS {
        for chip in [None, Some((0, 6)), Some((1, 5)), Some((2, 4)), Some((0, 2))] {
            let p = fixture(text, chip);
            if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&p, text) {
                panic!("{text:?} chip {chip:?}: line {line}: {error}\n{p:#?}");
            }
        }
    }
    // A clamped fixture too.
    let mut p = fixture(TEXTS[0], None);
    apply(&mut p, &Perturbation::Clamp);
    assert!(ValidatedLayout::try_from_paragraph(&p, TEXTS[0]).is_ok());
}

/// `fixture(text)` with one atomic span over `full` whose fragments are
/// `(line, start, end, x, width)`, and the stops strictly inside it removed.
fn with_atomic_span(
    text: &str,
    full: (usize, usize),
    fragments: &[(usize, usize, usize, f32, f32)],
) -> PositionedParagraph {
    let mut p = fixture(text, None);
    for l in &mut p.lines {
        l.caret_stops
            .retain(|s| !(full.0 < s.offset && s.offset < full.1));
    }
    for &(line, start, end, x, width) in fragments {
        p.lines[line].atomic_fragments.push(AtomicFragment {
            atomic_id: 9,
            full_byte_range: TextRange::new(full.0, full.1),
            fragment_byte_range: TextRange::new(start, end),
            x,
            width,
            bidi_level: BidiLevel::LTR,
        });
    }
    p
}

/// Inline positions across the fixture's lines (0..100), past both edges,
/// and the non-numbers.
fn x_sweep() -> impl Iterator<Item = f32> {
    (-40..=56).map(|i| 2.5 * i as f32).chain([
        -1e9,
        1e9,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ])
}

/// G4 accepts `p`, and every hit on it, per line and by point, lands on a
/// caret position of `text`; a hit strictly inside the span's fragment on
/// line `line` snaps to one of the span's ends `full` (INT-10).
#[track_caller]
fn assert_hits_snap_to_the_span(
    p: &PositionedParagraph,
    text: &str,
    full: (usize, usize),
    fragments: &[(usize, f32, f32)],
) {
    let l = ValidatedLayout::try_from_paragraph(p, text)
        .unwrap_or_else(|(line, e)| panic!("G4 rejects the valid span {full:?}: line {line}: {e}"));
    let mut checked = 0;
    for line in 0..p.lines.len() {
        for x in x_sweep() {
            let what = format!("hit_test_on_line({line}, {x})");
            let hit = l.hit_test_on_line(line, x);
            assert_position(text, hit.position, &what);
            if let Some(&(_, fx, fw)) = fragments
                .iter()
                .find(|&&(fl, fx, fw)| fl == line && fx < x && x < fx + fw)
            {
                assert!(
                    [full.0, full.1].contains(&hit.position.byte_offset),
                    "{what}: {hit:?} inside the fragment at {fx}+{fw} does not snap to {full:?}"
                );
            }
            checked += 1;
        }
    }
    for y in [
        -1e9,
        -10.0,
        0.0,
        3.0,
        8.0,
        13.0,
        18.0,
        23.0,
        28.0,
        40.0,
        1e9,
        f32::NAN,
    ] {
        for x in x_sweep() {
            let what = format!("hit_test({x}, {y})");
            assert_position(text, l.hit_test(Point { x, y }).position, &what);
            checked += 1;
        }
    }
    assert!(checked > 0);
}

#[test]
fn frg3_int10_an_atomic_span_reaching_past_the_text_is_rejected_and_one_ending_at_it_snaps_inside_it()
 {
    // A span recorded as 4..8 over the six bytes of "abcdef": its rendered
    // part, 4..6, is covered by the fragment on the last line, but the span
    // lies outside the text.
    let past = with_atomic_span(TEXTS[0], (4, 8), &[(2, 4, 6, 0.0, 100.0)]);
    let verdict = ValidatedLayout::try_from_paragraph(&past, TEXTS[0]).map(|_| ());
    assert!(
        matches!(
            verdict,
            Err((2, LineInvariantError::InvalidAtomicFragment { index: 0 }))
        ),
        "{verdict:?}"
    );
    // The boundary case G4 accepts: the span ends exactly at the text end.
    let at_end = with_atomic_span(TEXTS[0], (4, 6), &[(2, 4, 6, 0.0, 100.0)]);
    assert_hits_snap_to_the_span(&at_end, TEXTS[0], (4, 6), &[(2, 0.0, 100.0)]);
}

#[test]
fn frg3_int10_an_atomic_span_ending_inside_a_character_is_rejected_and_one_ending_on_its_boundary_snaps()
 {
    // "é" is bytes 2..4 of "abéef": a span 0..3 ends inside it.
    let inside = with_atomic_span(
        TEXTS[1],
        (0, 3),
        &[(0, 0, 2, 0.0, 100.0), (1, 2, 3, 0.0, 50.0)],
    );
    let verdict = ValidatedLayout::try_from_paragraph(&inside, TEXTS[1]).map(|_| ());
    assert!(
        matches!(
            verdict,
            Err((_, LineInvariantError::InvalidAtomicFragment { index: 0 }))
        ),
        "{verdict:?}"
    );
    // The boundary case G4 accepts: the span ends exactly after "é".
    let on_boundary = with_atomic_span(
        TEXTS[1],
        (0, 4),
        &[(0, 0, 2, 0.0, 100.0), (1, 2, 4, 0.0, 100.0)],
    );
    assert_hits_snap_to_the_span(
        &on_boundary,
        TEXTS[1],
        (0, 4),
        &[(0, 0.0, 100.0), (1, 0.0, 100.0)],
    );
}
