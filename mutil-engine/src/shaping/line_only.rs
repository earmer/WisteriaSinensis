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

use super::clusters::{empty_cluster, push_clusters};
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
            x_advance: finite(g.x_advance),
            x_offset: finite(g.x_offset),
            y_offset: finite(g.y_offset),
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
            for index in range {
                let mut cluster = empty_cluster(run, &atoms[index], index, fallback);
                cluster.glyph_range = output_glyphs.len()..output_glyphs.len();
                output_clusters.push(cluster);
            }
            return (leading, trailing, fallback);
        }
        // A seam is unambiguous at the start of the shaped text and after
        // white space; every other seam is reported unsafe (SHP-18).
        let safe_before = |atom: usize| {
            atom == range.start
                || text[..atoms[atom].byte_range.start()]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
        };
        // Split by face at atom boundaries: a face change inside an atom
        // keeps the atom's first face (the backend contract forbids one).
        let mut segments: Vec<(usize, FontId, usize)> = Vec::new(); // (first atom, font, first glyph)
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

fn finite(v: f32) -> f32 {
    if v.is_finite() { v } else { 0.0 }
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
        let first = atoms.partition_point(|a| a.byte_range.start() < window.start());
        let end = atoms.partition_point(|a| a.byte_range.end() <= window.end());
        if first >= end {
            return;
        }
        let (leading_cut, leading_text) = match &leading_edit {
            LeadingEdit::None => (0, String::new()),
            LeadingEdit::ReplaceSuffix {
                replacement,
                cut_bytes,
            } => (*cut_bytes as usize, replacement.to_string()),
        };
        let (trailing_cut, trailing_text) = match &trailing_edit {
            TrailingEdit::None => (0, String::new()),
            TrailingEdit::AppendHyphen { hyphen } => (0, hyphen.to_string()),
            TrailingEdit::ReplacePrefix {
                replacement,
                cut_bytes,
                hyphen,
            } => (*cut_bytes as usize, format!("{replacement}{hyphen}")),
        };
        let kept_start = atoms
            .partition_point(|a| a.byte_range.start() < window.start() + leading_cut)
            .clamp(first, end);
        let kept_end = atoms
            .partition_point(|a| a.byte_range.end() <= window.end().saturating_sub(trailing_cut))
            .clamp(kept_start, end);
        let kept_bytes = if kept_start < kept_end {
            atoms[kept_start].byte_range.start()..atoms[kept_end - 1].byte_range.end()
        } else {
            0..0
        };
        let string = format!("{leading_text}{}{trailing_text}", &text[kept_bytes.clone()]);
        let first_cluster = output_clusters.len();
        // Shape the kept atoms aside first: the cut atoms before them take
        // empty clusters in the face the backend chose (SHP-6 keeps the
        // window's atoms partitioned).
        let mut clusters = Vec::new();
        let mut glyphs = Vec::new();
        let (leading, trailing, font) = self.shape_atoms(
            &string,
            (leading_text.len(), trailing_text.len()),
            kept_bytes.start,
            run,
            atoms,
            kept_start..kept_end,
            text,
            &mut clusters,
            &mut glyphs,
        );
        let blank = |index: usize, out: &mut Vec<ShapedCluster>, at: usize| {
            let mut cluster = empty_cluster(run, &atoms[index], index, font);
            cluster.glyph_range = at..at;
            out.push(cluster);
        };
        for index in first..kept_start {
            blank(index, output_clusters, output_glyphs.len());
        }
        let offset = output_glyphs.len();
        output_glyphs.extend(glyphs);
        output_clusters.extend(clusters.into_iter().map(|mut c| {
            c.glyph_range = c.glyph_range.start + offset..c.glyph_range.end + offset;
            c
        }));
        for index in kept_end..end {
            blank(index, output_clusters, output_glyphs.len());
        }
        let appended = output_clusters.len();
        if appended == first_cluster {
            return;
        }
        output_clusters[first_cluster].safe_to_concat_before = false;
        output_clusters[first_cluster].safe_to_break_before =
            output_clusters[first_cluster].safe_to_break_before && leading_text.is_empty();
        attach(
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
        let advance = self.backend.space_advance(instance, orientation);
        if advance.is_finite() {
            advance.max(0.0)
        } else {
            0.0
        }
    }
}

/// Puts `leading` glyphs before the first of `clusters` and `trailing`
/// glyphs after the last one's (SHP-6), shifting the glyph ranges between.
fn attach(
    clusters: &mut [ShapedCluster],
    glyphs: &mut Vec<RawShapedGlyph>,
    leading: Vec<RawShapedGlyph>,
    trailing: Vec<RawShapedGlyph>,
) {
    let width = |g: &[RawShapedGlyph]| g.iter().map(|g| g.x_advance).sum::<f32>();
    if !trailing.is_empty() {
        let w = width(&trailing);
        glyphs.extend(trailing);
        let last = clusters.last_mut().expect("a cluster was appended");
        last.glyph_range.end = glyphs.len();
        last.natural_width = (last.natural_width + w).max(0.0);
    }
    if !leading.is_empty() {
        let w = width(&leading);
        let count = leading.len();
        let at = clusters[0].glyph_range.start;
        glyphs.splice(at..at, leading);
        for (k, cluster) in clusters.iter_mut().enumerate() {
            if k == 0 {
                cluster.glyph_range.end += count;
                cluster.natural_width = (cluster.natural_width + w).max(0.0);
            } else {
                cluster.glyph_range =
                    cluster.glyph_range.start + count..cluster.glyph_range.end + count;
            }
        }
    }
}
