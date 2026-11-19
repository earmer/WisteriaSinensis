//! The `LineOnly` profile (spec §4.6, SHP-18, SHP-24): a [`TextShaper`] over
//! a backend that only shapes whole strings into positioned glyphs, such as a
//! platform text system's line layout.
//!
//! The adapter rebuilds clusters from the glyphs' byte indices, merging them
//! to whole atoms as the full-contract backends do. Such a backend reports no
//! break-safety flags, so every seam is ambiguous except the start of the
//! shaped text and a seam after white space; there the clusters are reported
//! safe, and everywhere else `safe_to_break_before` and
//! `safe_to_concat_before` are `false`, so an emergency seam re-shapes both
//! sides. The backend sees no locale, so `locl` is not honoured, and the
//! adapter has no ligature carets: a ligature takes INT-5's equal slots.

use super::clusters::{
    EdgeFonts, TaggedGlyph, atom_containing, finish_window, plan_window, push_clusters,
    push_empty_clusters,
};
use super::{ReshapeRequest, ShapeOutput, TextShaper};
use crate::types::{
    FontFamily, FontFeatures, FontId, FontInstanceRef, FontMetrics, FontStyle, FontVariation,
    GlyphOrientation, GraphemeAtom, LocaleTag, RawShapedGlyph, ScriptTag, ShapingRun,
};

/// One glyph of a [`LineShaper`]'s output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineGlyph {
    /// The face the glyph comes from, in the backend's id space.
    pub font: FontId,
    /// The glyph in that face.
    pub glyph_id: u32,
    /// The byte offset, in the shaped string, of the first character of the
    /// source cluster the glyph belongs to.
    pub byte_index: usize,
    /// Inline advance in layout units.
    pub x_advance: f32,
    /// Inline offset in layout units, positive towards the inline end.
    pub x_offset: f32,
    /// Block offset in layout units, positive towards line-over.
    pub y_offset: f32,
}

/// A backend that shapes one horizontal, left-to-right string at a time.
///
/// It chooses faces itself, from the family stack and the style, and returns
/// the glyphs in left-to-right order; all glyphs of one character come from
/// one face. It sees no locale and no surrounding text. Its metrics answer as
/// [`TextShaper`]'s do.
pub trait LineShaper: Send + Sync {
    /// The glyphs of `text` at `font_size` with `features` (variations
    /// included).
    fn shape_line(
        &self,
        text: &str,
        families: &[FontFamily],
        style: FontStyle,
        font_size: f32,
        features: &FontFeatures,
    ) -> Vec<LineGlyph>;

    /// As [`TextShaper::font_metrics`].
    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics;

    /// As [`TextShaper::resolve_primary_font_metrics`].
    fn resolve_primary_font_metrics(
        &self,
        families: &[FontFamily],
        style: FontStyle,
        script: ScriptTag,
        locale: &LocaleTag,
        font_size: f32,
        variations: &[FontVariation],
    ) -> (FontId, FontMetrics);

    /// As [`TextShaper::space_advance`].
    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32;
}

/// The [`TextShaper`] of the `LineOnly` profile over a [`LineShaper`]
/// (spec SHP-24).
#[derive(Clone, Debug)]
pub struct LineOnlyShaper<B> {
    backend: B,
}

impl<B: LineShaper> LineOnlyShaper<B> {
    /// The adapter over `backend`.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// The backend, whose font ids the clusters carry.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Shapes `window.string` and appends the clusters of `window.range`;
    /// the glyphs of the `lead` and `trail` bytes are returned, marked
    /// synthesized, for the caller to attach, with the face the cut edges
    /// take.
    fn shape_atoms(
        &self,
        window: &LineWindow<'_>,
        output: &mut ShapeOutput,
    ) -> (EdgeFonts, Synthesized) {
        let run = window.run;
        let range = &window.range;
        let glyphs = self.backend.shape_line(
            window.string,
            &run.families,
            run.style,
            run.font_size,
            &run.features,
        );
        let (kept, synthesized) = window.split_glyphs(&glyphs);
        // With no glyph to name it, a cluster takes the run's primary face.
        let fallback = glyphs.first().map_or_else(
            || {
                self.backend
                    .resolve_primary_font_metrics(
                        &run.families,
                        run.style,
                        run.script,
                        &run.locale,
                        run.font_size,
                        run.features.variations(),
                    )
                    .0
            },
            |g| g.font,
        );
        let mut edges = EdgeFonts {
            first: fallback,
            last: fallback,
        };
        if range.is_empty() {
            return (edges, synthesized);
        }
        if kept.is_empty() {
            push_empty_clusters(run, window.atoms, range.clone(), fallback, output);
        } else {
            let kept_first = output.clusters.len();
            push_clusters(run, window.atoms, range.clone(), &kept, fallback, output);
            // The cut edges take the face of the kept cluster beside them
            // (SHP-6).
            if let (Some(first), Some(last)) = (
                output.clusters[kept_first..].first(),
                output.clusters.last(),
            ) {
                edges = EdgeFonts {
                    first: first.font.font_id,
                    last: last.font.font_id,
                };
            }
        }
        (edges, synthesized)
    }
}

/// One string handed to the backend, and the atoms whose clusters it
/// rebuilds.
struct LineWindow<'a> {
    /// The text shaped: the leading edit text, the kept text, the trailing
    /// edit text.
    string: &'a str,
    /// Byte lengths of the leading and trailing edit text in `string`.
    lead: usize,
    trail: usize,
    /// The paragraph offset of the kept text.
    start: usize,
    run: &'a ShapingRun,
    atoms: &'a [GraphemeAtom],
    /// The atoms of the kept text.
    range: core::ops::Range<usize>,
    /// The paragraph text.
    text: &'a str,
}

impl LineWindow<'_> {
    /// Sorts the backend's glyphs into the kept text's, tagged with their
    /// atoms, and the edit text's at the two edges, marked synthesized.
    fn split_glyphs(&self, glyphs: &[LineGlyph]) -> (Vec<TaggedGlyph>, Synthesized) {
        let Self {
            string,
            lead,
            trail,
            start,
            atoms,
            ref range,
            text,
            ..
        } = *self;
        let kept_end = string.len() - trail;
        let raw = |g: &LineGlyph, synthesized: bool| RawShapedGlyph {
            glyph_id: g.glyph_id,
            x_advance: g.x_advance,
            x_offset: g.x_offset,
            y_offset: g.y_offset,
            is_synthesized: synthesized,
        };
        // A seam is unambiguous at the start of the shaped text and after
        // white space; every other seam is reported unsafe (SHP-18).
        let safe_before = |atom: usize| {
            (atom == range.start && lead == 0)
                || text[..atoms[atom].byte_range.start()]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
        };
        let mut synthesized = Synthesized::default();
        let mut kept: Vec<TaggedGlyph> = Vec::new();
        for g in glyphs {
            if g.byte_index < lead {
                synthesized.leading.push(raw(g, true));
            } else if g.byte_index >= kept_end || range.is_empty() {
                synthesized.trailing.push(raw(g, true));
            } else {
                let byte = start + (g.byte_index - lead);
                let atom = atom_containing(atoms, range.clone(), byte);
                // A glyph of a gap between atoms (a word space, a tab, an
                // object's bytes) belongs to no cluster: the frontend sizes
                // the gap itself (SHP-5, WN-10).
                if !atoms[atom].byte_range.contains(byte) {
                    continue;
                }
                let unsafe_seam = !safe_before(atom);
                kept.push(TaggedGlyph {
                    atom,
                    font: g.font,
                    glyph: raw(g, false),
                    unsafe_break: unsafe_seam,
                    unsafe_concat: unsafe_seam,
                });
            }
        }
        (kept, synthesized)
    }
}

/// The glyphs the backend returned for the edit text at the two edges.
#[derive(Default)]
struct Synthesized {
    leading: Vec<RawShapedGlyph>,
    trailing: Vec<RawShapedGlyph>,
}

impl<B: LineShaper> TextShaper for LineOnlyShaper<B> {
    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics {
        self.backend.font_metrics(instance)
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
        self.backend
            .resolve_primary_font_metrics(families, style, script, locale, font_size, variations)
    }

    fn shape_run(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        output: &mut ShapeOutput,
    ) {
        if atoms.is_empty() {
            return;
        }
        let bytes = run.byte_range;
        let window = LineWindow {
            string: &text[bytes.as_range()],
            lead: 0,
            trail: 0,
            start: bytes.start(),
            run,
            atoms,
            range: 0..atoms.len(),
            text,
        };
        self.shape_atoms(&window, output);
    }

    fn reshape_window(&self, request: &ReshapeRequest<'_>, output: &mut ShapeOutput) {
        let ReshapeRequest {
            text,
            run,
            atoms,
            window,
            leading_edit,
            trailing_edit,
            // The backend shapes each string without context (SHP-24), so
            // every window edge already reads as a text edge.
            context: _,
        } = *request;
        let Some(plan) = plan_window(atoms, window, leading_edit, trailing_edit) else {
            return;
        };
        let kept_bytes = if plan.kept_start < plan.kept_end {
            atoms[plan.kept_start].byte_range.start()..atoms[plan.kept_end - 1].byte_range.end()
        } else {
            0..0
        };
        let trailing_str = &plan.trailing_text;
        let kept_text = &text[kept_bytes.clone()];
        let shaped_input: std::borrow::Cow<'_, str> =
            if plan.leading_text.is_empty() && trailing_str.is_empty() {
                kept_text.into()
            } else {
                [plan.leading_text, kept_text, trailing_str].concat().into()
            };
        finish_window(
            run,
            atoms,
            &plan,
            output,
            |output| {
                let window = LineWindow {
                    string: &shaped_input,
                    lead: plan.leading_text.len(),
                    trail: trailing_str.len(),
                    start: kept_bytes.start,
                    run,
                    atoms,
                    range: plan.kept_start..plan.kept_end,
                    text,
                };
                self.shape_atoms(&window, output)
            },
            |synthesized, _| (synthesized.leading, synthesized.trailing),
        );
    }

    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32 {
        self.backend.space_advance(instance, orientation)
    }
}
