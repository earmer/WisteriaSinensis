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
    attach_synthesized, finite_nonnegative, finite_or_zero, plan_window, push_clusters,
    push_empty_clusters,
};
use super::{LeadingEdit, ShaperProfile, TextShaper, TrailingEdit};
use crate::types::{
    Direction, FontFamily, FontFeatures, FontId, FontInstanceRef, FontMetrics, FontRef, FontStyle,
    FontVariation, GlyphOrientation, GraphemeAtom, LocaleTag, RawShapedGlyph, ScriptTag,
    ShapedCluster, ShapingRun, TextRange,
};
use smallvec::SmallVec;

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
    /// The profile this adapter runs as.
    pub const PROFILE: ShaperProfile = ShaperProfile::LineOnly;

    /// The adapter over `backend`.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// The backend, whose font ids the clusters carry.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Shapes `string` and appends the clusters of `atoms[range]`, whose text
    /// is `string[lead..string.len() - trail]` starting at paragraph offset
    /// `start`; the glyphs of the `lead` and `trail` bytes are returned,
    /// marked synthesized, for the caller to attach.
    #[expect(
        clippy::too_many_arguments,
        reason = "the shaping context of one window"
    )]
    fn shape_atoms(
        &self,
        string: &str,
        (lead, trail): (usize, usize),
        start: usize,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        range: core::ops::Range<usize>,
        text: &str,
        output_clusters: &mut Vec<ShapedCluster>,
        output_glyphs: &mut Vec<RawShapedGlyph>,
    ) -> (Vec<RawShapedGlyph>, Vec<RawShapedGlyph>, FontId) {
        let glyphs = self.backend.shape_line(
            string,
            &run.families,
            run.style,
            run.font_size,
            &run.features,
        );
        let kept_end = string.len() - trail;
        let raw = |g: &LineGlyph, synthesized: bool| RawShapedGlyph {
            glyph_id: g.glyph_id,
            x_advance: finite_or_zero(g.x_advance),
            x_offset: finite_or_zero(g.x_offset),
            y_offset: finite_or_zero(g.y_offset),
            is_synthesized: synthesized,
        };
        let mut leading = Vec::new();
        let mut trailing = Vec::new();
        // (atom, font, glyph) for the kept text.
        let mut kept: Vec<(usize, FontId, RawShapedGlyph)> = Vec::new();
        for g in &glyphs {
            if g.byte_index < lead {
                leading.push(raw(g, true));
            } else if g.byte_index >= kept_end || range.is_empty() {
                trailing.push(raw(g, true));
            } else {
                let byte = start + (g.byte_index - lead);
                let atom = range.start
                    + atoms[range.clone()]
                        .partition_point(|a| a.byte_range.start() <= byte)
                        .saturating_sub(1);
                // A glyph of a gap between atoms (a word space, a tab, an
                // object's bytes) belongs to no cluster: the frontend sizes
                // the gap itself (SHP-5, WN-10).
                let bytes = atoms[atom].byte_range;
                if byte < bytes.start() || byte >= bytes.end() {
                    continue;
                }
                kept.push((atom, g.font, raw(g, false)));
            }
        }
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
        if range.is_empty() {
            return (leading, trailing, fallback);
        }
        if kept.is_empty() {
            push_empty_clusters(run, atoms, range, fallback, output_clusters, output_glyphs);
            return (leading, trailing, fallback);
        }
        // A seam is unambiguous at the start of the shaped text and after
        // white space; every other seam is reported unsafe (SHP-18).
        let safe_before = |atom: usize| {
            (atom == range.start && lead == 0)
                || text[..atoms[atom].byte_range.start()]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
        };
        // Split by face at atom boundaries: a face change inside an atom
        // keeps the atom's first face (the backend contract forbids one).
        // Entries store `(first atom, font, first glyph)`.
        let mut segments: Vec<(usize, FontId, usize)> = Vec::new();
        for (i, (atom, font, _)) in kept.iter().enumerate() {
            match segments.last() {
                Some((_, last_font, _)) if last_font == font => {}
                Some((first, _, _)) if *atom <= *first => {}
                _ => segments.push((*atom, *font, i)),
            }
        }
        segments[0].0 = range.start;
        for (k, &(first_atom, font, first_glyph)) in segments.iter().enumerate() {
            let (end_atom, end_glyph) = segments
                .get(k + 1)
                .map_or((range.end, kept.len()), |&(a, _, g)| (a, g));
            let tagged: Vec<(usize, RawShapedGlyph, bool, bool)> = kept[first_glyph..end_glyph]
                .iter()
                .map(|&(atom, _, glyph)| {
                    let unsafe_seam = !safe_before(atom.max(first_atom));
                    (atom.max(first_atom), glyph, unsafe_seam, unsafe_seam)
                })
                .collect();
            push_clusters(
                run,
                atoms,
                first_atom..end_atom,
                FontRef::unscaled(font),
                &tagged,
                output_clusters,
                output_glyphs,
            );
        }
        (leading, trailing, fallback)
    }
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
        output_clusters: &mut Vec<ShapedCluster>,
        output_glyphs: &mut Vec<RawShapedGlyph>,
    ) {
        if atoms.is_empty() {
            return;
        }
        let bytes = run.byte_range;
        self.shape_atoms(
            &text[bytes.as_range()],
            (0, 0),
            bytes.start(),
            run,
            atoms,
            0..atoms.len(),
            text,
            output_clusters,
            output_glyphs,
        );
    }

    fn reshape_window(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        window: TextRange,
        leading_edit: LeadingEdit,
        trailing_edit: TrailingEdit,
        output_clusters: &mut Vec<ShapedCluster>,
        output_glyphs: &mut Vec<RawShapedGlyph>,
    ) {
        let Some(plan) = plan_window(atoms, window, &leading_edit, &trailing_edit) else {
            return;
        };
        let kept_bytes = if plan.kept_start < plan.kept_end {
            atoms[plan.kept_start].byte_range.start()..atoms[plan.kept_end - 1].byte_range.end()
        } else {
            0..0
        };
        let trailing_str = plan.trailing_text.as_str();
        let shaped_input: std::borrow::Cow<'_, str> =
            if plan.leading_text.is_empty() && trailing_str.is_empty() {
                std::borrow::Cow::Borrowed(&text[kept_bytes.clone()])
            } else {
                let mut s = String::with_capacity(
                    plan.leading_text.len() + kept_bytes.len() + trailing_str.len(),
                );
                s.push_str(plan.leading_text);
                s.push_str(&text[kept_bytes.clone()]);
                s.push_str(trailing_str);
                std::borrow::Cow::Owned(s)
            };
        let first_cluster = output_clusters.len();
        // Emit any cut leading atoms directly with a placeholder face, then
        // patch them once `shape_atoms` reports the backend's chosen face.
        push_empty_clusters(
            run,
            atoms,
            plan.first..plan.kept_start,
            FontId::default(),
            output_clusters,
            output_glyphs,
        );
        let (leading, trailing, font) = self.shape_atoms(
            &shaped_input,
            (plan.leading_text.len(), trailing_str.len()),
            kept_bytes.start,
            run,
            atoms,
            plan.kept_start..plan.kept_end,
            text,
            output_clusters,
            output_glyphs,
        );
        if plan.first < plan.kept_start {
            let lead_count = plan.kept_start - plan.first;
            for cluster in &mut output_clusters[first_cluster..first_cluster + lead_count] {
                cluster.font = FontRef::unscaled(font);
            }
        }
        push_empty_clusters(
            run,
            atoms,
            plan.kept_end..plan.end,
            font,
            output_clusters,
            output_glyphs,
        );
        let appended = output_clusters.len();
        if appended == first_cluster {
            return;
        }
        output_clusters[first_cluster].safe_to_concat_before = false;
        output_clusters[first_cluster].safe_to_break_before =
            output_clusters[first_cluster].safe_to_break_before && plan.leading_text.is_empty();
        attach_synthesized(
            &mut output_clusters[first_cluster..],
            output_glyphs,
            leading,
            trailing,
        );
    }

    fn ligature_carets(
        &self,
        _instance: FontInstanceRef<'_>,
        _glyph_id: u32,
        _direction: Direction,
    ) -> SmallVec<[f32; 4]> {
        SmallVec::new()
    }

    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32 {
        finite_nonnegative(self.backend.space_advance(instance, orientation))
    }
}
