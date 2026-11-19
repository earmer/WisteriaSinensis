//! The HarfRust backend and its in-memory font registry (spec §4.7, SHP-21,
//! SHP-22, SHP-23, SHP-25).
//!
//! Fonts are parsed once, when they are registered; a variable instance is
//! built once per coordinate set and cached, with its metrics. Shaping uses
//! HarfRust, metrics come from `read-fonts`.

use super::clusters::{
    EdgeFonts, TaggedGlyph, atom_containing, finish_window, plan_window, push_clusters,
    push_empty_clusters,
};
use super::{FontFace, FontSource, ReshapeRequest, ShapeOutput, TextShaper};
use crate::types::{
    Direction, FontFamily, FontId, FontInstanceRef, FontMetrics, FontRef, FontStyle, FontVariation,
    GlyphOrientation, GraphemeAtom, LocaleTag, RawShapedGlyph, ScriptTag, ShapingRun, TextRange,
};
use core::fmt;
use harfrust::{
    Buffer, BufferFlags, Feature, GlyphInfo, GlyphPosition, Language, Script, ShapeOptions,
    ShaperFont, Tag,
};
use read_fonts::model::Font;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

mod face_choice;
mod metrics;
mod registry;
#[cfg(test)]
mod tests;

use metrics::{UnitMetrics, gdef_carets, unit_metrics};
pub use registry::{CjkRegion, FaceProfile, FontRegistry};

/// How many bytes of text on each side of a shaped segment reach HarfRust as
/// context (spec SHP-3). HarfBuzz reads at most five code points.
const CONTEXT_BYTES: usize = 32;

/// A variable instance, keyed by face and the bit patterns of its
/// coordinates (spec FNT-1 makes them sound keys).
type InstanceKey = (u32, SmallVec<[([u8; 4], u32); 2]>);

/// The HarfRust shaping backend over one shared [`FontRegistry`]
/// (spec SHP-22). It is also the [`FontSource`] of its own id space.
///
/// It implements the whole [`TextShaper`] contract for horizontal and
/// vertical-upright text, GDEF ligature carets included.
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

/// Shared input/output target for shaping one run or window segment.
struct ShapingTarget<'a> {
    text: &'a str,
    /// The bytes of `text` readable as context; its edges are text edges.
    context: TextRange,
    run: &'a ShapingRun,
    atoms: &'a [GraphemeAtom],
    output: &'a mut ShapeOutput,
    /// One HarfRust buffer reused by every font run of the call.
    buffer: &'a mut Buffer,
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
        // Distinct axes commute, so the key lists them by tag: `[wght, wdth]`
        // and `[wdth, wght]` share one instance. The sort is stable, so a
        // repeated tag keeps its order and its meaning.
        let mut coordinates: SmallVec<[([u8; 4], u32); 2]> = variations
            .iter()
            .map(|v| (v.tag(), v.value().to_bits()))
            .collect();
        coordinates.sort_by_key(|&(tag, _)| tag);
        let key: InstanceKey = (id.0, coordinates);
        let mut cache = self
            .instances
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let entry = bounded_entry(&mut cache, key, MAX_CACHED_INSTANCES, || {
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

    /// Shapes the atoms `target.atoms[range]` of `target.run`, all contiguous
    /// in bytes, appending clusters and glyphs. `cut_before` marks a seam the
    /// caller made: the first cluster is then not safe to concatenate (CLU-8).
    fn shape_segment(
        &self,
        target: &mut ShapingTarget<'_>,
        range: core::ops::Range<usize>,
        candidates: &[FontId],
        cut_before: bool,
    ) {
        let mut start = range.start;
        let group = target.run.orientation == GlyphOrientation::HorizontalInVertical;
        let face_at = |t: &ShapingTarget<'_>, atom: usize| {
            self.face_for(t.text, t.run, &t.atoms[atom], candidates)
        };
        let mut pending = (start < range.end).then(|| {
            if group {
                self.face_for_group(
                    target.text,
                    target.run,
                    &target.atoms[range.clone()],
                    candidates,
                )
            } else {
                face_at(target, start)
            }
        });
        while let Some(font) = pending {
            let end = if group {
                range.end
            } else {
                (start + 1..range.end)
                    .find(|&i| face_at(target, i) != font)
                    .unwrap_or(range.end)
            };
            pending = (end < range.end).then(|| face_at(target, end));
            let first_cluster = target.output.clusters.len();
            self.shape_font_run(target, start..end, font);
            if cut_before
                && start == range.start
                && let Some(cluster) = target.output.clusters.get_mut(first_cluster)
            {
                cluster.safe_to_concat_before = false;
            }
            start = end;
        }
    }

    /// Shapes `target.atoms[range]` with one face. Without a face every atom
    /// gets an empty, zero-width cluster.
    fn shape_font_run(
        &self,
        target: &mut ShapingTarget<'_>,
        range: core::ops::Range<usize>,
        font: Option<FontId>,
    ) {
        let text = target.text;
        let run = target.run;
        let atoms = target.atoms;
        let bytes = TextRange::new(
            atoms[range.start].byte_range.start(),
            atoms[range.end - 1].byte_range.end(),
        );
        let Some(font_id) = font else {
            push_empty_clusters(run, atoms, range, FontId::default(), target.output);
            return;
        };
        let buffer = &mut *target.buffer;
        buffer.reset();
        for (offset, c) in text[bytes.as_range()].char_indices() {
            buffer.push(c as u32, u32::try_from(offset).unwrap_or(u32::MAX));
        }
        let context = &text[..target.context.end()];
        buffer.set_pre_context(context_before(
            context,
            target.context.start(),
            bytes.start(),
        ));
        buffer.set_post_context(context_after(context, bytes.end()));
        let mut flags = BufferFlags::PRODUCE_UNSAFE_TO_CONCAT;
        if bytes.start() == target.context.start() {
            flags |= BufferFlags::BEGINNING_OF_TEXT;
        }
        if bytes.end() == target.context.end() {
            flags |= BufferFlags::END_OF_TEXT;
        }
        buffer.set_flags(flags);
        let Some(k) = self.shape_run_buffer(run, font_id, buffer) else {
            push_empty_clusters(run, atoms, range, font_id, target.output);
            return;
        };
        let vertical = run.orientation == GlyphOrientation::VerticalUpright;
        let mut glyphs: Vec<TaggedGlyph> = buffer
            .glyph_infos()
            .iter()
            .zip(buffer.glyph_positions())
            .map(|(info, pos)| TaggedGlyph {
                atom: atom_containing(atoms, range.clone(), bytes.start() + info.cluster as usize),
                font: font_id,
                glyph: raw_glyph(info, pos, k, vertical, false),
                unsafe_break: info.unsafe_to_break(),
                unsafe_concat: info.unsafe_to_concat(),
            })
            .collect();
        if run.bidi_level.direction().is_rtl() && !vertical {
            // HarfRust returns visual order; the contract is logical (SHP-11).
            glyphs.reverse();
        }
        push_clusters(run, atoms, range, &glyphs, font_id, target.output);
    }

    /// Glyphs of `replacement` in `font`, marked synthesized, for a
    /// hyphenation edit (spec SHP-6, CLU-7).
    fn synthesized_glyphs(
        &self,
        run: &ShapingRun,
        font: FontId,
        replacement: &str,
    ) -> Vec<RawShapedGlyph> {
        let mut buffer = Buffer::new();
        buffer.push_str(replacement);
        // The replacement takes the kept text's language, features and size.
        let Some(k) = self.shape_run_buffer(run, font, &mut buffer) else {
            return Vec::new();
        };
        let vertical = run.orientation == GlyphOrientation::VerticalUpright;
        let mut glyphs: Vec<RawShapedGlyph> = buffer
            .glyph_infos()
            .iter()
            .zip(buffer.glyph_positions())
            .map(|(info, pos)| raw_glyph(info, pos, k, vertical, true))
            .collect();
        if run.bidi_level.direction().is_rtl() && !vertical {
            glyphs.reverse();
        }
        glyphs
    }

    /// Shapes the filled `buffer` with face `font` under `run`'s direction,
    /// script, language, features and size, and returns the scale from font
    /// units (`font_size / units_per_em`); `None` without the instance or
    /// when HarfRust reports an error. A buffer that was reset and refilled
    /// has no output and no cached plan, so none of `ShapeError`'s misuse
    /// cases can arise.
    fn shape_run_buffer(&self, run: &ShapingRun, font: FontId, buffer: &mut Buffer) -> Option<f32> {
        let (parsed, units) = self.instance(font, run.features.variations())?;
        let vertical = run.orientation == GlyphOrientation::VerticalUpright;
        let rtl = run.bidi_level.direction().is_rtl();
        shape_buffer(run, &parsed, buffer, vertical, rtl)
            .then_some(run.font_size / units.units_per_em)
    }
}

fn harfrust_direction(vertical: bool, rtl: bool) -> harfrust::Direction {
    if vertical {
        harfrust::Direction::TopToBottom
    } else if rtl {
        harfrust::Direction::RightToLeft
    } else {
        harfrust::Direction::LeftToRight
    }
}

/// Sets the run's direction, script and language on `buffer` and shapes it
/// with `font` under the run's features and size; `false` when HarfRust
/// reports an error.
fn shape_buffer(
    run: &ShapingRun,
    font: &Font,
    buffer: &mut Buffer,
    vertical: bool,
    rtl: bool,
) -> bool {
    buffer.set_direction(harfrust_direction(vertical, rtl));
    buffer.set_script(Script::from_iso15924_tag(Tag::new(&run.script.as_bytes())));
    if run.locale.as_str() != "und" {
        buffer.set_language(Language::new(run.locale.as_str()));
    }
    let features = run_features(run);
    let options = ShapeOptions::new()
        .features(&features)
        .point_size(Some(run.font_size));
    harfrust::shape(&ShaperFont::new(font), buffer, options).is_ok()
}

/// One shaped glyph in flow space, scaled from font units by `k`.
fn raw_glyph(
    info: &GlyphInfo,
    pos: &GlyphPosition,
    k: f32,
    vertical: bool,
    is_synthesized: bool,
) -> RawShapedGlyph {
    let [x_advance, y_advance, x_offset, y_offset] =
        [pos.x_advance, pos.y_advance, pos.x_offset, pos.y_offset].map(|v| v as f32 * k);
    // Along the flow: a vertical advance runs down the page.
    let (x_advance, x_offset, y_offset) = if vertical {
        (-y_advance, -y_offset, x_offset)
    } else {
        (x_advance, x_offset, y_offset)
    };
    RawShapedGlyph {
        glyph_id: info.glyph_id,
        x_advance,
        x_offset,
        y_offset,
        is_synthesized,
    }
}

fn context_before(text: &str, from: usize, at: usize) -> &str {
    let mut start = at.saturating_sub(CONTEXT_BYTES).max(from);
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

/// The most variation instances the shaper keeps parsed at once. A caller
/// animating an axis asks for a new instance each frame: past this bound
/// the cache is cleared, so memory stays bounded and the working set is
/// rebuilt on demand.
const MAX_CACHED_INSTANCES: usize = 128;

/// The entry for `key` in `cache`, built by `build` when absent. A cache
/// holding `cap` entries is cleared before a new one goes in.
fn bounded_entry<K: Eq + std::hash::Hash, V>(
    cache: &mut HashMap<K, V>,
    key: K,
    cap: usize,
    build: impl FnOnce() -> V,
) -> &mut V {
    if cache.len() >= cap && !cache.contains_key(&key) {
        cache.clear();
    }
    cache.entry(key).or_insert_with(build)
}

/// The OpenType features of `run`, for HarfRust.
fn run_features(run: &ShapingRun) -> SmallVec<[Feature; 8]> {
    run.features
        .as_slice()
        .iter()
        .map(|f| Feature::new(Tag::new(&f.tag), f.value, ..))
        .collect()
}

/// The maximal byte-contiguous stretches of `atoms[range]`.
fn contiguous_segments(
    atoms: &[GraphemeAtom],
    range: core::ops::Range<usize>,
) -> SmallVec<[core::ops::Range<usize>; 4]> {
    let mut segments = SmallVec::new();
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
        output: &mut ShapeOutput,
    ) {
        let candidates = self.registry.candidates(&run.families, run.style);
        let mut buffer = Buffer::new();
        let mut target = ShapingTarget {
            text,
            context: TextRange::new(0, text.len()),
            run,
            atoms,
            output,
            buffer: &mut buffer,
        };
        for segment in contiguous_segments(atoms, 0..atoms.len()) {
            self.shape_segment(&mut target, segment, &candidates, false);
        }
    }

    fn reshape_window(&self, request: &ReshapeRequest<'_>, output: &mut ShapeOutput) {
        let ReshapeRequest {
            text,
            run,
            atoms,
            window,
            leading_edit,
            trailing_edit,
            context,
        } = *request;
        let Some(plan) = plan_window(atoms, window, leading_edit, trailing_edit) else {
            return;
        };
        let candidates = self.registry.candidates(&run.families, run.style);
        let fallback = candidates.first().copied().unwrap_or_default();
        finish_window(
            run,
            atoms,
            &plan,
            output,
            |output| {
                // A cut atom's empty cluster takes the face of the kept text
                // beside it, so a replacement attached to it is shaped in
                // that face rather than in the stack's first one.
                let kept_first = output.clusters.len();
                if plan.kept_start < plan.kept_end {
                    let mut buffer = Buffer::new();
                    let mut target = ShapingTarget {
                        text,
                        context,
                        run,
                        atoms,
                        output: &mut *output,
                        buffer: &mut buffer,
                    };
                    for (k, segment) in contiguous_segments(atoms, plan.kept_start..plan.kept_end)
                        .into_iter()
                        .enumerate()
                    {
                        self.shape_segment(&mut target, segment, &candidates, k == 0);
                    }
                }
                let kept = &output.clusters[kept_first..];
                let edges = match (kept.first(), kept.last()) {
                    (Some(first), Some(last)) => EdgeFonts {
                        first: first.font.font_id,
                        last: last.font.font_id,
                    },
                    _ => EdgeFonts {
                        first: fallback,
                        last: fallback,
                    },
                };
                (edges, ())
            },
            |(), edges| {
                let synthesize = |text: &str, font| {
                    if text.is_empty() {
                        Vec::new()
                    } else {
                        self.synthesized_glyphs(run, font, text)
                    }
                };
                (
                    synthesize(plan.leading_text, edges.first),
                    synthesize(&plan.trailing_text, edges.last),
                )
            },
        );
    }

    fn ligature_carets(
        &self,
        instance: FontInstanceRef<'_>,
        glyph_id: u32,
        _direction: Direction,
    ) -> SmallVec<[f32; 4]> {
        // GDEF caret coordinates increase in physical x from the glyph's
        // left edge in either direction (SHP-7); INT-5 pairs them for RTL.
        let Some((font, units)) = self.instance(instance.font.font_id, instance.variations) else {
            return SmallVec::new();
        };
        let scale = instance.font_size * instance.font.scale / units.units_per_em;
        let mut carets = gdef_carets(&font, glyph_id);
        for caret in &mut carets {
            *caret *= scale;
        }
        carets
    }

    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32 {
        let size = instance.font_size * instance.font.scale;
        // A face without a space falls back to a quarter em.
        self.instance(instance.font.font_id, instance.variations)
            .and_then(|(font, units)| {
                let glyph = font.charmap().map_unicode(' ')?;
                let metrics = font.glyph_metrics();
                let advance = if orientation == GlyphOrientation::VerticalUpright {
                    metrics.v_advance(glyph)
                } else {
                    metrics.h_advance(glyph)
                };
                Some(advance * size / units.units_per_em)
            })
            .unwrap_or(0.25 * size)
    }
}

impl FontSource for HarfRustShaper {
    fn resolve_font(&self, font: FontId) -> Option<FontFace> {
        self.registry.resolve_font(font)
    }
}
