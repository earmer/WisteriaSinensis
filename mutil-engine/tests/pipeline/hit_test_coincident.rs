//! Hit testing over coincident caret stops (spec INT-7, INT-8, INT-20,
//! CRT-1, CRT-3) and the equal-slot carets of a glyphless multi-atom cluster
//! (INT-5).
//!
//! On a line whose stops are in non-decreasing `x`, a point at or left of the
//! first stop's `x` (or NaN) takes `stops[0]`, a point at or right of the last
//! stop's `x` takes the last stop, and a point strictly between takes the
//! nearer of its two neighbouring stops, the right one at the midpoint; of
//! several stops at the right neighbour's `x` the first is taken, of several
//! at the left neighbour's `x` the last.

use crate::common::*;
use mutil_engine::*;
use smallvec::{SmallVec, smallvec};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Far enough from a stop to cross no neighbour on the lines below, whose
/// distinct stop positions are at least 2.7 apart.
const EPS: f32 = 0.05;

fn position_of(stop: &CaretStop) -> CursorPosition {
    CursorPosition {
        byte_offset: stop.offset,
        affinity: stop.affinity,
    }
}

/// The contract above, over `stops` alone: the stop a hit at `x` resolves to
/// and whether it is `inside`.
fn expected_hit(stops: &[CaretStop], x: f32) -> (CaretStop, bool) {
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    if x.is_nan() || x < first.x {
        return (first, false);
    }
    if x == first.x {
        return (first, true);
    }
    if x >= last.x {
        return (last, x == last.x);
    }
    // The first stop strictly right of x, and the last stop at or left of it.
    let r_index = stops.iter().position(|s| s.x > x).unwrap();
    let (l, r) = (stops[r_index - 1], stops[r_index]);
    let nearer = if r.x - x <= x - l.x { r } else { l };
    (nearer, true)
}

/// The runs of at least two stops sharing one `x`, as index ranges.
fn coincident_groups(stops: &[CaretStop]) -> Vec<std::ops::Range<usize>> {
    let mut groups = Vec::new();
    let mut start = 0;
    for end in 1..=stops.len() {
        if end == stops.len() || stops[end].x != stops[start].x {
            if end - start >= 2 {
                groups.push(start..end);
            }
            start = end;
        }
    }
    groups
}

struct Line {
    text: &'static str,
    paragraph: PositionedParagraph,
}

impl Line {
    fn lay_out(text: &'static str) -> Self {
        let paragraph = lay_out_bidi(text, &[HEBREW], &column(300.0), &LayoutOptions::default());
        assert_eq!(paragraph.lines.len(), 1, "{text:?}");
        Self { text, paragraph }
    }

    fn stops(&self) -> &[CaretStop] {
        &self.paragraph.lines[0].caret_stops
    }

    fn layout(&self) -> ValidatedLayout<'_> {
        ValidatedLayout::try_from_paragraph(&self.paragraph, self.text).unwrap()
    }

    /// Hits `x` on the line through both queries, which must agree, and
    /// returns the result.
    #[track_caller]
    fn hit(&self, x: f32) -> HitResult {
        let l = self.layout();
        let on_line = l.hit_test_on_line(0, x);
        let y = self.paragraph.lines[0].baseline_y;
        let by_point = l.hit_test(Point { x, y });
        assert_eq!(on_line, by_point, "{:?} at x = {x}", self.text);
        on_line
    }

    /// Asserts the hit at `x` lands on `stop` with `inside`.
    #[track_caller]
    fn assert_hit(&self, x: f32, stop: CaretStop, inside: bool) {
        let hit = self.hit(x);
        assert_eq!(
            (hit.position, hit.inside),
            (position_of(&stop), inside),
            "{:?} at x = {x}, expected {stop:?}; stops {:#?}",
            self.text,
            self.stops()
        );
    }

    /// The groups touching the line's first stop, its last stop, and neither.
    fn groups(&self) -> (Vec<std::ops::Range<usize>>, Vec<std::ops::Range<usize>>) {
        let n = self.stops().len();
        coincident_groups(self.stops())
            .into_iter()
            .partition(|g| g.start == 0 || g.end == n)
    }
}

/// The coincident group holding the line's first stop; panics if none.
fn start_group(line: &Line) -> std::ops::Range<usize> {
    let (edges, _) = line.groups();
    let group = edges.into_iter().find(|g| g.start == 0);
    group.unwrap_or_else(|| panic!("{:?}: no start group in {:#?}", line.text, line.stops()))
}

/// The coincident group holding the line's last stop; panics if none.
fn end_group(line: &Line) -> std::ops::Range<usize> {
    let n = line.stops().len();
    let (edges, _) = line.groups();
    let group = edges.into_iter().find(|g| g.end == n);
    group.unwrap_or_else(|| panic!("{:?}: no end group in {:#?}", line.text, line.stops()))
}

#[test]
fn int20_a_coincident_group_at_the_line_start_resolves_to_its_first_stop_at_and_before_it() {
    // RTL "אבגabc ": the trailing space collapses onto the left edge, where
    // "a" starts and the text ends (INT-20, CRT-3).
    for text in ["אבגabc ", "אבג  "] {
        let line = Line::lay_out(text);
        let stops = line.stops();
        let group = start_group(&line);
        assert!(group.len() >= 2, "{group:?}");
        let gx = stops[0].x;
        let (first, last_of_group) = (stops[0], stops[group.end - 1]);
        // Before and at the group: its first stop; only exactly at it inside.
        line.assert_hit(gx - EPS, first, false);
        line.assert_hit(gx - 100.0, first, false);
        line.assert_hit(f32::NAN, first, false);
        line.assert_hit(gx, first, true);
        // Just right of it the group is the left neighbour: its last stop.
        line.assert_hit(gx + EPS, last_of_group, true);
        // Continuity at the edge, and agreement with INT-8's extreme stop.
        assert_eq!(line.hit(gx).position, line.hit(gx - EPS).position);
        let extreme = line.paragraph.lines[0]
            .extreme_caret_stop(HangSide::Left)
            .unwrap();
        assert_eq!(line.hit(gx).position, position_of(extreme), "{text:?}");
    }
}

#[test]
fn int20_a_coincident_group_at_the_line_end_resolves_to_its_last_stop_at_and_after_it() {
    // LTR "abcאבג ": the Hebrew's leading edge, the end of the Hebrew run and
    // the collapsed trailing space all lie on the right edge; LTR "abc  "
    // collapses two spaces onto "c"'s right edge.
    for text in ["abcאבג ", "abcאבג  ", "abc  "] {
        let line = Line::lay_out(text);
        let stops = line.stops();
        let group = end_group(&line);
        assert!(group.len() >= 2, "{group:?}");
        let last = stops[stops.len() - 1];
        let gx = last.x;
        let first_of_group = stops[group.start];
        // Approached from the left the group is the right neighbour: its
        // first stop.
        line.assert_hit(gx - EPS, first_of_group, true);
        // At and after it: the line's last stop; only exactly at it inside.
        line.assert_hit(gx, last, true);
        line.assert_hit(gx + EPS, last, false);
        line.assert_hit(gx + 100.0, last, false);
        assert_eq!(line.hit(gx).position, line.hit(gx + EPS).position);
        let extreme = line.paragraph.lines[0]
            .extreme_caret_stop(HangSide::Right)
            .unwrap();
        assert_eq!(line.hit(gx).position, position_of(extreme), "{text:?}");
    }
}

#[test]
fn int20_an_interior_coincident_group_takes_its_first_stop_from_the_left_and_its_last_at_and_after_it()
 {
    // Direction boundaries inside the line: "abcאבג" (3 upstream and 9 at
    // one x), "abc אבג def" (two groups) and RTL "אבג abc דהו" (two groups).
    let mut checked = 0;
    for text in ["abcאבג", "abc אבג def", "אבג abc דהו", "אבגabc"] {
        let line = Line::lay_out(text);
        let stops = line.stops();
        let (_, interior) = line.groups();
        for group in interior {
            assert!(group.len() >= 2, "{group:?}");
            let gx = stops[group.start].x;
            let (first_of_group, last_of_group) = (stops[group.start], stops[group.end - 1]);
            line.assert_hit(gx - EPS, first_of_group, true);
            line.assert_hit(gx, last_of_group, true);
            line.assert_hit(gx + EPS, last_of_group, true);
            checked += 1;
        }
    }
    assert!(checked >= 5, "only {checked} interior groups");
}

#[test]
fn int20_the_line_edges_clamp_to_the_first_and_last_stop_without_a_group() {
    // "abcאבג": the left edge holds offset 0 alone and the right edge
    // offset 3 downstream alone.
    let line = Line::lay_out("abcאבג");
    let stops = line.stops();
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    assert_ne!(first.x, stops[1].x);
    assert_ne!(last.x, stops[stops.len() - 2].x);
    line.assert_hit(first.x - EPS, first, false);
    line.assert_hit(first.x, first, true);
    line.assert_hit(last.x, last, true);
    line.assert_hit(last.x + EPS, last, false);
    line.assert_hit(f32::NAN, first, false);
    line.assert_hit(f32::NEG_INFINITY, first, false);
    line.assert_hit(f32::INFINITY, last, false);
}

#[test]
fn int20_hits_follow_the_nearest_stop_rule_everywhere_on_bidi_lines() {
    for text in [
        "abcאבג",
        "abcאבג ",
        "abcאבג  ",
        "abc  ",
        "abc אבג def",
        "abc אבג 123 def",
        "abcאבגdef",
        "אבגabc",
        "אבגabc ",
        "אבג  ",
        "אבג abc דהו",
        "אבגabcדהו",
    ] {
        let line = Line::lay_out(text);
        let stops = line.stops();
        assert!(
            !coincident_groups(stops).is_empty(),
            "{text:?} has no coincident group: {stops:#?}"
        );
        let mut probes = vec![f32::NAN, stops[0].x - 50.0, stops[stops.len() - 1].x + 50.0];
        for s in stops {
            probes.extend([s.x - EPS, s.x, s.x + EPS]);
        }
        for pair in stops.windows(2).filter(|w| w[0].x < w[1].x) {
            let mid = (pair[0].x + pair[1].x) / 2.0;
            probes.extend([mid - EPS, mid, mid + EPS]);
        }
        for x in probes {
            let (stop, inside) = expected_hit(stops, x);
            line.assert_hit(x, stop, inside);
        }
    }
}

/// A shaper over a HarfRust shaper that merges the clusters of `merged`'s
/// bytes into one cluster, dropping their glyphs unless `keep_glyphs`, and
/// counts and answers `ligature_carets` calls with two valid but unequal
/// carets.
struct GlyphlessMerge {
    inner: &'static HarfRustShaper,
    merged: TextRange,
    keep_glyphs: bool,
    caret_calls: AtomicUsize,
}

impl GlyphlessMerge {
    fn new(inner: &'static HarfRustShaper, merged: TextRange) -> Self {
        Self {
            inner,
            merged,
            keep_glyphs: false,
            caret_calls: AtomicUsize::new(0),
        }
    }

    /// Appends `shaped` to `output`, merging the clusters inside `merged`.
    fn splice(&self, shaped: ShapeOutput, atoms: &[GraphemeAtom], output: &mut ShapeOutput) {
        let mut pending: Option<ShapedCluster> = None;
        for cluster in shaped.clusters {
            if self.merged.contains_range(cluster.byte_range) {
                let start = output.glyphs.len();
                if self.keep_glyphs {
                    output
                        .glyphs
                        .extend_from_slice(&shaped.glyphs[cluster.glyph_range.clone()]);
                }
                let end = output.glyphs.len();
                pending = Some(match pending.take() {
                    None => ShapedCluster {
                        glyph_range: start..end,
                        ..cluster
                    },
                    Some(p) => ShapedCluster {
                        byte_range: p.byte_range.cover(cluster.byte_range),
                        atom_range: p.atom_range.start..cluster.atom_range.end,
                        glyph_range: p.glyph_range.start..end,
                        natural_width: p.natural_width + cluster.natural_width,
                        ..p
                    },
                });
                continue;
            }
            Self::flush(&mut pending, atoms, output);
            let start = output.glyphs.len();
            output
                .glyphs
                .extend_from_slice(&shaped.glyphs[cluster.glyph_range.clone()]);
            output.clusters.push(ShapedCluster {
                glyph_range: start..output.glyphs.len(),
                ..cluster
            });
        }
        Self::flush(&mut pending, atoms, output);
    }

    fn flush(
        pending: &mut Option<ShapedCluster>,
        atoms: &[GraphemeAtom],
        output: &mut ShapeOutput,
    ) {
        if let Some(cluster) = pending.take() {
            let flags = AtomFlags::for_cluster(&atoms[cluster.atom_range.clone()]);
            output.clusters.push(ShapedCluster { flags, ..cluster });
        }
    }
}

impl TextShaper for GlyphlessMerge {
    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics {
        self.inner.font_metrics(instance)
    }

    fn resolve_primary_font_metrics(
        &self,
        families: &[FontFamily],
        style: FontStyle,
        script: ScriptTag,
        locale: &LocaleTag,
        font_size: f32,
        variations: &[FontVariation],
    ) -> (FontId, FontMetrics) {
        self.inner
            .resolve_primary_font_metrics(families, style, script, locale, font_size, variations)
    }

    fn shape_run(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        output: &mut ShapeOutput,
    ) {
        let mut shaped = ShapeOutput::default();
        self.inner.shape_run(text, run, atoms, &mut shaped);
        self.splice(shaped, atoms, output);
    }

    fn reshape_window(&self, request: &ReshapeRequest<'_>, output: &mut ShapeOutput) {
        let mut shaped = ShapeOutput::default();
        self.inner.reshape_window(request, &mut shaped);
        self.splice(shaped, request.atoms, output);
    }

    fn ligature_carets(
        &self,
        _instance: FontInstanceRef<'_>,
        _glyph_id: u32,
        _direction: Direction,
    ) -> SmallVec<[f32; 4]> {
        self.caret_calls.fetch_add(1, Ordering::Relaxed);
        smallvec![1.0, 2.0]
    }

    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32 {
        self.inner.space_advance(instance, orientation)
    }
}

/// Lays out `text` with `shaper` and returns its single line, after checking
/// that the merged bytes form one cluster of three atoms and no glyph, which
/// the frontend sizes from its (absent) glyphs: zero wide.
fn lay_out_merged(text: &'static str, style: &TextStyle, shaper: &GlyphlessMerge) -> Line {
    let ast = compile_paragraph(&input(text, style), &FrontendOptions::default(), shaper)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    let merged: Vec<&ShapedCluster> = ast
        .clusters
        .iter()
        .filter(|c| c.byte_range == shaper.merged)
        .collect();
    let [cluster] = merged.as_slice() else {
        panic!("{text:?}: {:#?}", ast.clusters)
    };
    assert_eq!(cluster.atom_count(), 3);
    assert!(cluster.glyph_range.is_empty());
    assert_eq!(cluster.natural_width, 0.0);
    let paragraph = lay_out_with(&ast, text, shaper);
    assert_eq!(paragraph.lines.len(), 1);
    Line { text, paragraph }
}

/// The offsets of `line`'s stops in visual order.
fn stop_offsets(line: &Line) -> Vec<usize> {
    line.stops().iter().map(|s| s.offset).collect()
}

#[test]
fn int5_a_glyphless_cluster_of_three_atoms_has_no_sub_slots_and_asks_for_no_carets() {
    // "xabcy" with "abc" one cluster of three atoms and no glyph: zero wide,
    // so it has no sub-slots (INT-5), and with no glyph there is nothing to
    // ask GDEF carets of. Its two edges coincide on "x"'s right edge.
    let shaper = GlyphlessMerge::new(&SHAPER, TextRange::new(1, 4));
    let line = lay_out_merged("xabcy", &style(10.0), &shaper);
    assert_eq!(stop_offsets(&line), [0, 1, 4, 5]);
    let stops = line.stops();
    assert_close(stops[1].x, 6.0);
    assert_eq!(stops[1].x, stops[2].x);
    assert_eq!(shaper.caret_calls.load(Ordering::Relaxed), 0);
    // The coincident edges behave as any interior group: its first stop
    // from the left, its last at and after its x.
    line.assert_hit(6.0 - EPS, stops[1], true);
    line.assert_hit(6.0, stops[2], true);
    line.assert_hit(6.0 + EPS, stops[2], true);
}

#[test]
fn int5_a_glyphless_rtl_cluster_has_no_sub_slots_and_asks_for_no_carets() {
    // "אבגדה" with "בגד" (bytes 2..8) one glyphless cluster: its edges, the
    // logical boundaries 2 and 8, coincide; 4 and 6 have no stop.
    let shaper = GlyphlessMerge::new(&BIDI_SHAPER, TextRange::new(2, 8));
    let line = lay_out_merged("אבגדה", &script_style(&[HEBREW], 10.0), &shaper);
    assert_eq!(stop_offsets(&line), [10, 8, 2, 0]);
    let stops = line.stops();
    assert_eq!(stops[1].x, stops[2].x);
    assert_eq!(shaper.caret_calls.load(Ordering::Relaxed), 0);
    let gx = stops[1].x;
    line.assert_hit(gx - EPS, stops[1], true);
    line.assert_hit(gx, stops[2], true);
    line.assert_hit(gx + EPS, stops[2], true);
}

#[test]
fn int5_the_same_cluster_with_its_glyphs_takes_the_shapers_carets() {
    // The control for the two tests above: "abc" merged with its three
    // glyphs kept is an 18-wide ligature, whose carets (1 and 2, valid for
    // three atoms) the shaper is asked for and the stops follow.
    let shaper = GlyphlessMerge {
        keep_glyphs: true,
        ..GlyphlessMerge::new(&SHAPER, TextRange::new(1, 4))
    };
    let text = "xabcy";
    let ast = compile_paragraph(
        &input(text, &style(10.0)),
        &FrontendOptions::default(),
        &shaper,
    )
    .unwrap();
    let cluster = ast
        .clusters
        .iter()
        .find(|c| c.byte_range == shaper.merged)
        .unwrap();
    assert_eq!(cluster.atom_count(), 3);
    assert_close(cluster.natural_width, 18.0);
    let p = lay_out_with(&ast, text, &shaper);
    let line = &p.lines[0];
    let offsets: Vec<usize> = line.caret_stops.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, [0, 1, 2, 3, 4, 5]);
    for (stop, x) in line
        .caret_stops
        .iter()
        .zip([0.0, 6.0, 7.0, 8.0, 24.0, 30.0])
    {
        assert_close(stop.x, x);
    }
    assert!(shaper.caret_calls.load(Ordering::Relaxed) > 0);
}

fn lay_out_with(ast: &ParagraphAst, text: &str, shaper: &GlyphlessMerge) -> PositionedParagraph {
    let paragraph = layout_paragraph(ast, text, shaper, &column(300.0), &LayoutOptions::default())
        .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&paragraph, text) {
        panic!("{text:?}: line {line}: {error}\n{paragraph:#?}");
    }
    paragraph
}

#[test]
fn int20_no_offset_has_two_stops_with_one_affinity_at_one_position() {
    // "One stop per distinct offset and position" (INT-20): a coincident
    // group holds distinct offsets or affinities, never a repeated stop.
    for text in [
        "אבג ",
        "אבג  ",
        "אבגabc ",
        "abcאבג ",
        "abc  ",
        "abc אבג def",
    ] {
        let line = Line::lay_out(text);
        let stops = line.stops();
        for (i, a) in stops.iter().enumerate() {
            for b in &stops[i + 1..] {
                assert!(
                    !(a.offset == b.offset && a.affinity == b.affinity && a.x == b.x),
                    "{text:?}: {a:?} repeated in {stops:#?}"
                );
            }
        }
    }
}
