//! The HarfRust backend and its in-memory font registry (spec §4.7, SHP-21,
//! SHP-22).
//!
//! Fonts are parsed once, when they are registered; a variable instance is
//! built once per coordinate set and cached, with its metrics. Shaping uses
//! HarfRust, metrics come from `read-fonts`.

use super::{FontFace, FontSource, LeadingEdit, TextShaper, TrailingEdit};
use crate::types::{
    Direction, FontFamily, FontId, FontInstanceRef, FontMetrics, FontRef, FontSlant, FontStyle,
    FontVariation, GenericFamily, GlyphOrientation, GraphemeAtom, LocaleTag, RawShapedGlyph,
    ScriptTag, ShapedCluster, ShapingRun, TextRange,
};
use core::fmt;
use harfrust::{Buffer, BufferFlags, Feature, Language, Script, ShapeOptions, ShaperFont, Tag};
use read_fonts::TableProvider;
use read_fonts::model::Font;
use read_fonts::types::GlyphId;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

/// How many bytes of text on each side of a shaped segment reach HarfRust as
/// context (spec SHP-3). HarfBuzz reads at most five code points.
const CONTEXT_BYTES: usize = 32;

/// One registered face: the family and style it answers to, its bytes, the
/// parsed font at its default location and that location's metrics, in font
/// units.
#[derive(Clone)]
struct RegisteredFace {
    family: Arc<str>,
    style: FontStyle,
    face: FontFace,
    font: Font,
    metrics: UnitMetrics,
}

/// [`FontMetrics`] in font units, before a size is applied.
#[derive(Clone, Copy, Debug)]
struct UnitMetrics {
    units_per_em: f32,
    metrics: FontMetrics,
}

impl UnitMetrics {
    /// The metrics of an instance `font_size · scale` layout units per em.
    fn scaled(&self, size: f32) -> FontMetrics {
        let k = size / self.units_per_em;
        let m = &self.metrics;
        FontMetrics {
            vertical_ascent: m.vertical_ascent * k,
            vertical_descent: m.vertical_descent * k,
            vertical_line_gap: m.vertical_line_gap * k,
            ideographic_baseline_offset: m.ideographic_baseline_offset * k,
            central_baseline_offset: m.central_baseline_offset * k,
            hanging_baseline_offset: m.hanging_baseline_offset * k,
            mathematical_baseline_offset: m.mathematical_baseline_offset * k,
            ascent: m.ascent * k,
            descent: m.descent * k,
            line_gap: m.line_gap * k,
            underline_position: m.underline_position * k,
            underline_thickness: m.underline_thickness * k,
            strikeout_position: m.strikeout_position * k,
            x_height: m.x_height * k,
            cap_height: m.cap_height * k,
            use_typo_metrics: m.use_typo_metrics,
        }
    }
}

impl fmt::Debug for RegisteredFace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredFace")
            .field("family", &self.family)
            .field("style", &self.style)
            .field("face", &self.face)
            .finish_non_exhaustive()
    }
}

/// An in-memory set of font faces: the id space of a [`HarfRustShaper`] and
/// the [`FontSource`] a renderer resolves its ids through (spec SHP-21,
/// HST-2).
///
/// Faces are added while the registry is being built; once it is shared it
/// no longer changes, so an id always names the same face. A `FontId` is the
/// index of its face in registration order and is never reused.
#[derive(Clone, Debug, Default)]
pub struct FontRegistry {
    faces: Vec<RegisteredFace>,
    generics: Vec<(GenericFamily, Arc<str>)>,
}

impl FontRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `face` under `family` and `style` and returns its id, or
    /// `None`, registering nothing, when its bytes hold no font at its face
    /// index. The bytes are shared, not copied (spec HST-1); they are parsed
    /// here, once.
    pub fn add_face(
        &mut self,
        family: Arc<str>,
        style: FontStyle,
        face: FontFace,
    ) -> Option<FontId> {
        let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(face.bytes().clone());
        let font = Font::new(bytes, face.face_index())?;
        let id = FontId(u32::try_from(self.faces.len()).ok()?);
        let metrics = unit_metrics(&font);
        self.faces.push(RegisteredFace {
            family,
            style,
            face,
            font,
            metrics,
        });
        Some(id)
    }

    /// Names the family a generic keyword resolves to; a later call for the
    /// same keyword replaces the earlier one.
    pub fn set_generic(&mut self, generic: GenericFamily, family: Arc<str>) {
        self.generics.retain(|(keyword, _)| *keyword != generic);
        self.generics.push((generic, family));
    }

    fn face(&self, id: FontId) -> Option<&RegisteredFace> {
        self.faces.get(usize::try_from(id.0).ok()?)
    }

    /// The faces to try for `families` and `style`, best first: the best
    /// style match of each family of the stack in order, then every other
    /// face in registration order (the deterministic last resort, SHP-12).
    fn candidates(&self, families: &[FontFamily], style: FontStyle) -> Vec<FontId> {
        let mut order: Vec<FontId> = Vec::new();
        for family in families {
            let name = match family {
                FontFamily::Named(name) => Some(name.clone()),
                FontFamily::Generic(generic) => self
                    .generics
                    .iter()
                    .find(|(keyword, _)| keyword == generic)
                    .map(|(_, name)| name.clone()),
            };
            let Some(name) = name else { continue };
            let mut members: Vec<(u32, FontId)> = self
                .faces
                .iter()
                .enumerate()
                .filter(|(_, face)| face.family.eq_ignore_ascii_case(&name))
                .map(|(index, face)| (style_distance(face.style, style), id_of(index)))
                .collect();
            members.sort_by_key(|&(distance, id)| (distance, id.0));
            for (_, id) in members {
                if !order.contains(&id) {
                    order.push(id);
                }
            }
        }
        for index in 0..self.faces.len() {
            let id = id_of(index);
            if !order.contains(&id) {
                order.push(id);
            }
        }
        order
    }
}

fn id_of(index: usize) -> FontId {
    // `add_face` refuses an index that does not fit.
    FontId(u32::try_from(index).unwrap_or(u32::MAX))
}

/// How far `face` is from the requested `style`: a slant mismatch outweighs
/// any weight difference.
fn style_distance(face: FontStyle, wanted: FontStyle) -> u32 {
    let slant = match (face.slant, wanted.slant) {
        (a, b) if a == b => 0,
        (FontSlant::Italic, FontSlant::Oblique) | (FontSlant::Oblique, FontSlant::Italic) => 1,
        _ => 2,
    };
    slant * 10_000 + u32::from(face.weight.get().abs_diff(wanted.weight.get()))
}

impl FontSource for FontRegistry {
    fn resolve_font(&self, font: FontId) -> Option<FontFace> {
        self.face(font).map(|registered| registered.face.clone())
    }
}

/// A variable instance, keyed by face and the bit patterns of its
/// coordinates (spec FNT-1 makes them sound keys).
type InstanceKey = (u32, SmallVec<[([u8; 4], u32); 2]>);

/// The HarfRust shaping backend over one shared [`FontRegistry`]
/// (spec SHP-22). It is also the [`FontSource`] of its own id space.
///
/// It implements the whole [`TextShaper`] contract for horizontal and
/// vertical-upright text except GDEF ligature carets, which it does not read
/// yet (`ligature_carets` answers none, the documented equal-slot fallback).
#[derive(Clone)]
pub struct HarfRustShaper {
    registry: Arc<FontRegistry>,
    instances: Arc<Mutex<HashMap<InstanceKey, (Font, UnitMetrics)>>>,
}

impl fmt::Debug for HarfRustShaper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HarfRustShaper")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

impl HarfRustShaper {
    /// A shaper over `registry`.
    #[must_use]
    pub fn new(registry: Arc<FontRegistry>) -> Self {
        Self {
            registry,
            instances: Arc::default(),
        }
    }

    /// The registry the shaper's ids belong to, to share with a renderer.
    #[must_use]
    pub fn registry(&self) -> &Arc<FontRegistry> {
        &self.registry
    }

    /// The parsed font of `id` at `variations`, and its metrics in font
    /// units. The default location is the registered font itself; any other
    /// is built once and cached.
    fn instance(&self, id: FontId, variations: &[FontVariation]) -> Option<(Font, UnitMetrics)> {
        let face = self.registry.face(id)?;
        if variations.is_empty() || face.font.axes().is_empty() {
            return Some((face.font.clone(), face.metrics));
        }
        let key: InstanceKey = (
            id.0,
            variations
                .iter()
                .map(|v| (v.tag(), v.value().to_bits()))
                .collect(),
        );
        let mut cache = self
            .instances
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let entry = cache.entry(key).or_insert_with(|| {
            let font = face
                .font
                .instance_builder()
                .variations(variations.iter().map(|v| (Tag::new(&v.tag()), v.value())))
                .build();
            let metrics = unit_metrics(&font);
            (font, metrics)
        });
        Some(entry.clone())
    }

    /// The face that shapes `atom`: the first candidate whose character map
    /// covers all of it, else the first that covers its first code point,
    /// else the first candidate (spec SHP-12).
    fn face_for(&self, text: &str, atom: &GraphemeAtom, candidates: &[FontId]) -> Option<FontId> {
        let chars = || text[atom.byte_range.as_range()].chars();
        let covers = |id: FontId, c: char| {
            is_coverage_exempt(c)
                || self
                    .registry
                    .face(id)
                    .is_some_and(|face| face.font.charmap().map_unicode(c).is_some())
        };
        candidates
            .iter()
            .copied()
            .find(|&id| chars().all(|c| covers(id, c)))
            .or_else(|| {
                let first = chars().next()?;
                candidates.iter().copied().find(|&id| covers(id, first))
            })
            .or_else(|| candidates.first().copied())
    }

    /// Shapes the atoms `atoms[range]` of `run`, all contiguous in bytes,
    /// appending clusters and glyphs. `cut_before` marks a seam the caller
    /// made: the first cluster is then not safe to concatenate (CLU-8).
    #[expect(
        clippy::too_many_arguments,
        reason = "the shaping context of one segment"
    )]
    fn shape_segment(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        range: core::ops::Range<usize>,
        candidates: &[FontId],
        cut_before: bool,
        output_clusters: &mut Vec<ShapedCluster>,
        output_glyphs: &mut Vec<RawShapedGlyph>,
    ) {
        let mut start = range.start;
        while start < range.end {
            let font = self.face_for(text, &atoms[start], candidates);
            let mut end = start + 1;
            while end < range.end && self.face_for(text, &atoms[end], candidates) == font {
                end += 1;
            }
            let first_cluster = output_clusters.len();
            self.shape_font_run(
                text,
                run,
                atoms,
                start..end,
                font,
                output_clusters,
                output_glyphs,
            );
            if cut_before
                && start == range.start
                && let Some(cluster) = output_clusters.get_mut(first_cluster)
            {
                cluster.safe_to_concat_before = false;
            }
            start = end;
        }
    }

    /// Shapes `atoms[range]` with one face. Without a face every atom gets
    /// an empty, zero-width cluster.
    #[expect(
        clippy::too_many_arguments,
        reason = "the shaping context of one font run"
    )]
    fn shape_font_run(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        range: core::ops::Range<usize>,
        font: Option<FontId>,
        output_clusters: &mut Vec<ShapedCluster>,
        output_glyphs: &mut Vec<RawShapedGlyph>,
    ) {
        let bytes = TextRange::new(
            atoms[range.start].byte_range.start(),
            atoms[range.end - 1].byte_range.end(),
        );
        let instance =
            font.and_then(|id| Some((id, self.instance(id, run.features.variations())?)));
        let Some((font_id, (parsed, units))) = instance else {
            for (index, atom) in atoms.iter().enumerate().take(range.end).skip(range.start) {
                output_clusters.push(empty_cluster(run, atom, index, font.unwrap_or_default()));
                let at = output_glyphs.len();
                output_clusters.last_mut().expect("just pushed").glyph_range = at..at;
            }
            return;
        };
        let vertical = run.orientation == GlyphOrientation::VerticalUpright;
        let rtl = run.bidi_level.direction().is_rtl();
        let mut buffer = Buffer::new();
        for (offset, c) in text[bytes.as_range()].char_indices() {
            buffer.push(c as u32, u32::try_from(offset).unwrap_or(u32::MAX));
        }
        buffer.set_pre_context(context_before(text, bytes.start()));
        buffer.set_post_context(context_after(text, bytes.end()));
        buffer.set_direction(if vertical {
            harfrust::Direction::TopToBottom
        } else if rtl {
            harfrust::Direction::RightToLeft
        } else {
            harfrust::Direction::LeftToRight
        });
        buffer.set_script(Script::from_iso15924_tag(Tag::new(&run.script.as_bytes())));
        if run.locale.as_str() != "und" {
            buffer.set_language(Language::new(run.locale.as_str()));
        }
        let mut flags = BufferFlags::PRODUCE_UNSAFE_TO_CONCAT;
        if bytes.start() == 0 {
            flags |= BufferFlags::BEGINNING_OF_TEXT;
        }
        if bytes.end() == text.len() {
            flags |= BufferFlags::END_OF_TEXT;
        }
        buffer.set_flags(flags);
        let features: Vec<Feature> = run
            .features
            .as_slice()
            .iter()
            .map(|f| Feature::new(Tag::new(&f.tag), f.value, ..))
            .collect();
        let shaper_font = ShaperFont::new(&parsed);
        let options = ShapeOptions::new()
            .features(&features)
            .point_size(Some(run.font_size));
        if harfrust::shape(&shaper_font, &mut buffer, options).is_err() {
            for (index, atom) in atoms.iter().enumerate().take(range.end).skip(range.start) {
                output_clusters.push(empty_cluster(run, atom, index, font_id));
                let at = output_glyphs.len();
                output_clusters.last_mut().expect("just pushed").glyph_range = at..at;
            }
            return;
        }
        let k = run.font_size / units.units_per_em;
        let mut glyphs: Vec<(usize, RawShapedGlyph, bool, bool)> = buffer
            .glyph_infos()
            .iter()
            .zip(buffer.glyph_positions())
            .map(|(info, pos)| {
                let byte = bytes.start() + info.cluster as usize;
                // The atom holding the glyph's cluster value.
                let atom = range.start
                    + atoms[range.clone()]
                        .partition_point(|a| a.byte_range.start() <= byte)
                        .saturating_sub(1);
                let (advance, x_offset, y_offset) = if vertical {
                    (
                        -pos.y_advance as f32 * k,
                        -pos.y_offset as f32 * k,
                        pos.x_offset as f32 * k,
                    )
                } else {
                    (
                        pos.x_advance as f32 * k,
                        pos.x_offset as f32 * k,
                        pos.y_offset as f32 * k,
                    )
                };
                let glyph = RawShapedGlyph {
                    glyph_id: info.glyph_id,
                    x_advance: advance,
                    x_offset,
                    y_offset,
                    is_synthesized: false,
                };
                (atom, glyph, info.unsafe_to_break(), info.unsafe_to_concat())
            })
            .collect();
        if rtl && !vertical {
            // HarfRust returns visual order; the contract is logical (SHP-11).
            glyphs.reverse();
        }
        push_clusters(
            run,
            atoms,
            range,
            FontRef::unscaled(font_id),
            &glyphs,
            output_clusters,
            output_glyphs,
        );
    }

    /// Glyphs of `replacement` in `font`, marked synthesized, for a
    /// hyphenation edit (spec SHP-6, CLU-7).
    fn synthesized_glyphs(
        &self,
        run: &ShapingRun,
        font: FontId,
        replacement: &str,
    ) -> Vec<RawShapedGlyph> {
        let Some((parsed, units)) = self.instance(font, run.features.variations()) else {
            return Vec::new();
        };
        let mut buffer = Buffer::new();
        buffer.push_str(replacement);
        buffer.set_direction(if run.bidi_level.direction().is_rtl() {
            harfrust::Direction::RightToLeft
        } else {
            harfrust::Direction::LeftToRight
        });
        buffer.set_script(Script::from_iso15924_tag(Tag::new(&run.script.as_bytes())));
        let shaper_font = ShaperFont::new(&parsed);
        if harfrust::shape(&shaper_font, &mut buffer, ShapeOptions::new()).is_err() {
            return Vec::new();
        }
        let k = run.font_size / units.units_per_em;
        let mut glyphs: Vec<RawShapedGlyph> = buffer
            .glyph_infos()
            .iter()
            .zip(buffer.glyph_positions())
            .map(|(info, pos)| RawShapedGlyph {
                glyph_id: info.glyph_id,
                x_advance: pos.x_advance as f32 * k,
                x_offset: pos.x_offset as f32 * k,
                y_offset: pos.y_offset as f32 * k,
                is_synthesized: true,
            })
            .collect();
        if run.bidi_level.direction().is_rtl() {
            glyphs.reverse();
        }
        glyphs
    }
}

/// Code points shaped by whatever face shapes the rest of their atom:
/// joiners and variation selectors, which faces rarely map.
fn is_coverage_exempt(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}')
}

fn context_before(text: &str, at: usize) -> &str {
    let mut start = at.saturating_sub(CONTEXT_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..at]
}

fn context_after(text: &str, at: usize) -> &str {
    let mut end = (at + CONTEXT_BYTES).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[at..end]
}

/// A glyph-less cluster over one atom.
fn empty_cluster(
    run: &ShapingRun,
    atom: &GraphemeAtom,
    index: usize,
    font: FontId,
) -> ShapedCluster {
    ShapedCluster {
        orientation: run.orientation,
        byte_range: atom.byte_range,
        atom_range: index..index + 1,
        font: FontRef::unscaled(font),
        bidi_level: run.bidi_level,
        flags: atom.flags,
        glyph_range: 0..0,
        natural_width: 0.0,
        safe_to_break_before: true,
        safe_to_concat_before: true,
    }
}

/// Groups glyphs, each tagged with the atom holding its cluster value, into
/// clusters of whole atoms (spec SHP-13): a glyph that reaches back into an
/// earlier cluster merges the two, and an atom with no glyph of its own (a
/// ligature component) joins the cluster before it.
fn push_clusters(
    run: &ShapingRun,
    atoms: &[GraphemeAtom],
    range: core::ops::Range<usize>,
    font: FontRef,
    glyphs: &[(usize, RawShapedGlyph, bool, bool)],
    output_clusters: &mut Vec<ShapedCluster>,
    output_glyphs: &mut Vec<RawShapedGlyph>,
) {
    // Groups: (first atom, glyph indices).
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for (index, (atom, ..)) in glyphs.iter().enumerate() {
        let atom = *atom;
        match groups.last_mut() {
            Some((start, members)) if atom == *start => members.push(index),
            Some((start, _)) if atom > *start => groups.push((atom, vec![index])),
            Some(_) => {
                // Out of order: merge back until the group starts at or
                // before this atom.
                let mut merged = vec![index];
                while let Some((start, members)) = groups.pop() {
                    merged.extend(members);
                    if start <= atom {
                        merged.sort_unstable();
                        groups.push((start, merged));
                        break;
                    }
                    if groups.is_empty() {
                        merged.sort_unstable();
                        groups.push((atom.min(start), merged));
                        break;
                    }
                }
            }
            None => groups.push((atom, vec![index])),
        }
    }
    if groups.is_empty() {
        groups.push((range.start, Vec::new()));
    }
    groups[0].0 = range.start;
    for (k, (start, members)) in groups.iter().enumerate() {
        let end = groups.get(k + 1).map_or(range.end, |(next, _)| *next);
        let first = output_glyphs.len();
        output_glyphs.extend(members.iter().map(|&i| glyphs[i].1));
        let (unsafe_break, unsafe_concat) = members
            .first()
            .map_or((false, false), |&i| (glyphs[i].2, glyphs[i].3));
        let natural_width = members
            .iter()
            .map(|&i| glyphs[i].1.x_advance)
            .sum::<f32>()
            .max(0.0);
        output_clusters.push(ShapedCluster {
            orientation: run.orientation,
            byte_range: TextRange::new(
                atoms[*start].byte_range.start(),
                atoms[end - 1].byte_range.end(),
            ),
            atom_range: *start..end,
            font,
            bidi_level: run.bidi_level,
            flags: crate::types::AtomFlags::for_cluster(&atoms[*start..end]),
            glyph_range: first..output_glyphs.len(),
            natural_width,
            safe_to_break_before: !unsafe_break,
            safe_to_concat_before: !unsafe_break && !unsafe_concat,
        });
    }
}

/// The maximal byte-contiguous stretches of `atoms[range]`.
fn contiguous_segments(
    atoms: &[GraphemeAtom],
    range: core::ops::Range<usize>,
) -> Vec<core::ops::Range<usize>> {
    let mut segments = Vec::new();
    let mut start = range.start;
    for index in range.clone() {
        if index > start && atoms[index - 1].byte_range.end() != atoms[index].byte_range.start() {
            segments.push(start..index);
            start = index;
        }
    }
    if start < range.end {
        segments.push(start..range.end);
    }
    segments
}

impl TextShaper for HarfRustShaper {
    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics {
        self.instance(instance.font.font_id, instance.variations)
            .map(|(_, units)| units.scaled(instance.font_size * instance.font.scale))
            .unwrap_or_default()
    }

    fn resolve_primary_font_metrics(
        &self,
        families: &[FontFamily],
        style: FontStyle,
        _script: ScriptTag,
        _locale: &LocaleTag,
        font_size: f32,
        variations: &[FontVariation],
    ) -> (FontId, FontMetrics) {
        let Some(&primary) = self.registry.candidates(families, style).first() else {
            return (FontId::default(), FontMetrics::default());
        };
        let metrics = self.font_metrics(FontInstanceRef {
            font: FontRef::unscaled(primary),
            font_size,
            variations,
        });
        (primary, metrics)
    }

    fn shape_run(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        output_clusters: &mut Vec<ShapedCluster>,
        output_glyphs: &mut Vec<RawShapedGlyph>,
    ) {
        let candidates = self.registry.candidates(&run.families, run.style);
        for segment in contiguous_segments(atoms, 0..atoms.len()) {
            self.shape_segment(
                text,
                run,
                atoms,
                segment,
                &candidates,
                false,
                output_clusters,
                output_glyphs,
            );
        }
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
        let candidates = self.registry.candidates(&run.families, run.style);
        let first_cluster = output_clusters.len();
        // A cut edit removes atoms: they keep empty clusters, so the window's
        // atoms stay partitioned (SHP-6), and the rest is shaped as is.
        let (leading_cut, leading_text) = match &leading_edit {
            LeadingEdit::None => (0, None),
            LeadingEdit::ReplaceSuffix {
                replacement,
                cut_bytes,
            } => (*cut_bytes as usize, Some(replacement.to_string())),
        };
        let (trailing_cut, trailing_text) = match &trailing_edit {
            TrailingEdit::None => (0, None),
            TrailingEdit::AppendHyphen { hyphen } => (0, Some(hyphen.to_string())),
            TrailingEdit::ReplacePrefix {
                replacement,
                cut_bytes,
                hyphen,
            } => (*cut_bytes as usize, Some(format!("{replacement}{hyphen}"))),
        };
        let kept_start =
            atoms.partition_point(|a| a.byte_range.start() < window.start() + leading_cut);
        let kept_end = atoms
            .partition_point(|a| a.byte_range.end() <= window.end().saturating_sub(trailing_cut));
        let (kept_start, kept_end) = (kept_start.clamp(first, end), kept_end.clamp(first, end));
        let fallback = candidates.first().copied().unwrap_or_default();
        let blank = |index: usize, out: &mut Vec<ShapedCluster>, glyphs: &Vec<RawShapedGlyph>| {
            let mut cluster = empty_cluster(run, &atoms[index], index, fallback);
            cluster.glyph_range = glyphs.len()..glyphs.len();
            out.push(cluster);
        };
        for index in first..kept_start.min(kept_end) {
            blank(index, output_clusters, output_glyphs);
        }
        if kept_start < kept_end {
            for (k, segment) in contiguous_segments(atoms, kept_start..kept_end)
                .into_iter()
                .enumerate()
            {
                self.shape_segment(
                    text,
                    run,
                    atoms,
                    segment,
                    &candidates,
                    k == 0,
                    output_clusters,
                    output_glyphs,
                );
            }
        }
        for index in kept_end.max(kept_start)..end {
            blank(index, output_clusters, output_glyphs);
        }
        let appended = output_clusters.len();
        if appended == first_cluster {
            return;
        }
        output_clusters[first_cluster].safe_to_concat_before = false;
        // Replacement glyphs go on the first (leading) or last (trailing)
        // cluster; glyph ranges after an insertion shift.
        if let Some(replacement) = trailing_text {
            let last = appended - 1;
            let font = output_clusters[last].font.font_id;
            let glyphs = self.synthesized_glyphs(run, font, &replacement);
            let width: f32 = glyphs.iter().map(|g| g.x_advance).sum();
            output_glyphs.extend(glyphs.iter().copied());
            let cluster = &mut output_clusters[last];
            cluster.glyph_range.end = output_glyphs.len();
            cluster.natural_width = (cluster.natural_width + width).max(0.0);
        }
        if let Some(replacement) = leading_text.filter(|r| !r.is_empty()) {
            let font = output_clusters[first_cluster].font.font_id;
            let glyphs = self.synthesized_glyphs(run, font, &replacement);
            let width: f32 = glyphs.iter().map(|g| g.x_advance).sum();
            let at = output_clusters[first_cluster].glyph_range.start;
            let count = glyphs.len();
            output_glyphs.splice(at..at, glyphs);
            for (k, cluster) in output_clusters[first_cluster..].iter_mut().enumerate() {
                if k == 0 {
                    cluster.glyph_range.end += count;
                    cluster.natural_width = (cluster.natural_width + width).max(0.0);
                } else {
                    cluster.glyph_range =
                        cluster.glyph_range.start + count..cluster.glyph_range.end + count;
                }
            }
        }
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
        let size = instance.font_size * instance.font.scale;
        let fallback = 0.25 * size;
        let Some((font, units)) = self.instance(instance.font.font_id, instance.variations) else {
            return fallback.max(0.0);
        };
        let Some(glyph) = font.charmap().map_unicode(' ') else {
            return fallback.max(0.0);
        };
        let metrics = font.glyph_metrics();
        let advance = if orientation == GlyphOrientation::VerticalUpright {
            metrics.v_advance(glyph)
        } else {
            metrics.h_advance(glyph)
        };
        let value = advance * size / units.units_per_em;
        if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        }
    }
}

impl FontSource for HarfRustShaper {
    fn resolve_font(&self, font: FontId) -> Option<FontFace> {
        self.registry.resolve_font(font)
    }
}

/// The font's [`FontMetrics`] in font units, with the documented fallbacks
/// for whatever its tables leave out (spec FNT-4, SHP-17). Every offset is
/// positive towards line-over: `post` and `OS/2` already store the
/// underline and strike-out positions that way.
fn unit_metrics(font: &Font) -> UnitMetrics {
    let metrics = font.metrics();
    let upem = f32::from(metrics.units_per_em.max(1));
    let f = |v: read_fonts::types::F48Dot16| v.to_f32();
    let nonzero = |line: &Option<read_fonts::model::metrics::LineBox>| {
        line.filter(|l| l.ascender.to_f32() - l.descender.to_f32() > 0.0)
    };
    let typo = nonzero(&metrics.typo_line);
    let hhea = nonzero(&metrics.hhea_line);
    let chosen = if metrics.use_typo_metrics {
        typo.or(hhea)
    } else {
        hhea.or(typo)
    };
    let (ascent, descent, line_gap) = match (chosen, metrics.win_line) {
        (Some(line), _) => (
            f(line.ascender),
            -f(line.descender),
            f(line.line_gap).max(0.0),
        ),
        (None, Some(win)) if f(win.ascender) - f(win.descender) > 0.0 => {
            (f(win.ascender), -f(win.descender), 0.0)
        }
        _ => (0.8 * upem, 0.2 * upem, 0.0),
    };
    let (ascent, descent) = (ascent.max(0.0), descent.max(0.0));
    let ink_top = |c: char, default: f32| metrics_value(font, c).unwrap_or(default);
    let x_height = metrics
        .x_height
        .map(f)
        .filter(|v| *v > 0.0)
        .unwrap_or_else(|| ink_top('x', 0.5 * upem));
    let cap_height = metrics
        .cap_height
        .map(f)
        .filter(|v| *v > 0.0)
        .unwrap_or_else(|| ink_top('H', 0.7 * upem));
    let style = font.style_metrics();
    let (underline_position, underline_thickness) = style
        .underline
        .map(|d| (f(d.position), f(d.thickness).abs()))
        .unwrap_or((-0.1 * upem, 0.05 * upem));
    let strikeout_position = style
        .strike_through
        .map(|d| f(d.position))
        .unwrap_or(x_height / 2.0);
    let (vertical_ascent, vertical_descent, vertical_line_gap) = nonzero(&metrics.vhea_line)
        .map(|l| {
            (
                f(l.ascender).max(0.0),
                (-f(l.descender)).max(0.0),
                f(l.line_gap).max(0.0),
            )
        })
        .unwrap_or((0.5 * upem, 0.5 * upem, 0.0));
    let base = base_offsets(font);
    let ideographic = base.ideo.unwrap_or(-descent);
    let central = match (base.ideo, base.idtp) {
        (Some(bottom), Some(top)) => (bottom + top) / 2.0,
        _ => ideographic + 0.5 * upem,
    };
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    UnitMetrics {
        units_per_em: upem,
        metrics: FontMetrics {
            vertical_ascent: finite(vertical_ascent),
            vertical_descent: finite(vertical_descent),
            vertical_line_gap: finite(vertical_line_gap),
            ideographic_baseline_offset: finite(ideographic),
            central_baseline_offset: finite(central),
            hanging_baseline_offset: finite(base.hang.unwrap_or(0.8 * ascent)),
            mathematical_baseline_offset: finite(base.math.unwrap_or(0.5 * x_height)),
            ascent: finite(ascent),
            descent: finite(descent),
            line_gap: finite(line_gap),
            underline_position: finite(underline_position),
            underline_thickness: finite(underline_thickness),
            strikeout_position: finite(strikeout_position),
            x_height: finite(x_height.max(0.0)),
            cap_height: finite(cap_height.max(0.0)),
            use_typo_metrics: metrics.use_typo_metrics,
        },
    }
}

/// The top of the ink of `c`'s glyph, in font units.
fn metrics_value(font: &Font, c: char) -> Option<f32> {
    let glyph: GlyphId = font.charmap().map_unicode(c)?;
    let extents = font.glyph_metrics().extents(glyph)?;
    Some(extents.y_bearing).filter(|v| *v > 0.0)
}

/// Baseline offsets read from the horizontal axis of `BASE`, relative to the
/// alphabetic (`romn`) baseline when the font gives one.
#[derive(Default)]
struct BaseOffsets {
    ideo: Option<f32>,
    idtp: Option<f32>,
    hang: Option<f32>,
    math: Option<f32>,
}

fn base_offsets(font: &Font) -> BaseOffsets {
    let read = || -> Option<BaseOffsets> {
        let tables = font.tables();
        let base = tables.base().ok()?;
        let axis = base.horiz_axis()?.ok()?;
        let tags: Vec<Tag> = axis
            .base_tag_list()?
            .ok()?
            .baseline_tags()
            .iter()
            .map(|t| t.get())
            .collect();
        let list = axis.base_script_list().ok()?;
        let records = list.base_script_records();
        let record = records
            .iter()
            .find(|r| r.base_script_tag() == Tag::new(b"DFLT"))
            .or_else(|| {
                records
                    .iter()
                    .find(|r| r.base_script_tag() == Tag::new(b"latn"))
            })
            .or_else(|| records.first())?;
        let values = record
            .base_script(list.offset_data())
            .ok()?
            .base_values()?
            .ok()?;
        let coords = values.base_coords();
        let coordinate = |tag: &[u8; 4]| -> Option<f32> {
            let index = tags.iter().position(|t| *t == Tag::new(tag))?;
            let coord = coords.get(index).ok()?;
            Some(f32::from(coord.coordinate()))
        };
        let romn = coordinate(b"romn").unwrap_or(0.0);
        let relative = |tag: &[u8; 4]| coordinate(tag).map(|v| v - romn);
        Some(BaseOffsets {
            ideo: relative(b"ideo"),
            idtp: relative(b"idtp"),
            hang: relative(b"hang"),
            math: relative(b"math"),
        })
    };
    read().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaping::FontBytes;

    #[test]
    fn bytes_that_hold_no_font_are_not_registered() {
        let mut registry = FontRegistry::new();
        let face = FontFace::new(FontBytes::from_static(b"not a font"), 0);
        assert_eq!(
            registry.add_face("Test".into(), FontStyle::default(), face),
            None
        );
        assert!(registry.resolve_font(FontId(0)).is_none());
        let shaper = HarfRustShaper::new(Arc::new(registry));
        assert!(shaper.resolve_font(FontId(0)).is_none());
        assert!(shaper.registry().resolve_font(FontId(0)).is_none());
    }

    #[test]
    fn a_generic_keyword_keeps_its_last_family() {
        let mut registry = FontRegistry::new();
        registry.set_generic(GenericFamily::Serif, "A".into());
        registry.set_generic(GenericFamily::Serif, "B".into());
        assert_eq!(registry.generics, [(GenericFamily::Serif, Arc::from("B"))]);
    }

    /// The backend shares parsed fonts between layout threads (spec SHP-1).
    #[test]
    fn parsed_fonts_and_the_backend_are_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Font>();
        check::<FontRegistry>();
        check::<HarfRustShaper>();
    }

    #[test]
    fn an_empty_registry_answers_invalid_metrics_and_the_fallback_space() {
        let shaper = HarfRustShaper::new(Arc::new(FontRegistry::new()));
        let instance = FontInstanceRef {
            font: FontRef::unscaled(FontId(0)),
            font_size: 16.0,
            variations: &[],
        };
        assert!(!shaper.font_metrics(instance).is_valid());
        assert_eq!(
            shaper.space_advance(instance, GlyphOrientation::Horizontal),
            4.0
        );
    }

    #[test]
    fn a_slant_mismatch_outweighs_any_weight() {
        let regular = FontStyle::default();
        let italic = FontStyle {
            slant: FontSlant::Italic,
            ..regular
        };
        let black = FontStyle {
            weight: crate::types::FontWeight::new(900).unwrap(),
            ..regular
        };
        assert!(style_distance(black, regular) < style_distance(italic, regular));
        assert_eq!(style_distance(regular, regular), 0);
    }
}
