//! The HarfRust backend and its in-memory font registry (spec §4.7, SHP-21,
//! SHP-22, SHP-23, SHP-25).
//!
//! Fonts are parsed once, when they are registered; a variable instance is
//! built once per coordinate set and cached, with its metrics. Shaping uses
//! HarfRust, metrics come from `read-fonts`.

use super::clusters::{
    EdgeFonts, TaggedGlyph, atom_containing, finish_window, plan_window,
    push_clusters_with_scratch, push_empty_clusters,
};
use super::{FontFace, FontSource, ReshapeRequest, ShapeOutput, TextShaper};
use crate::types::{
    Direction, FontFamily, FontId, FontInstanceRef, FontMetrics, FontRef, FontStyle, FontVariation,
    GlyphOrientation, GraphemeAtom, LocaleTag, RawShapedGlyph, ScriptTag, ShapingRun, TextRange,
};
use core::fmt;
use harfrust::{
    Buffer, BufferFlags, GlyphInfo, GlyphPosition, ShapeOptions, ShapePlan, ShaperFont, Tag,
};
use read_fonts::model::Font;
use smallvec::SmallVec;
use std::sync::{Arc, Mutex, PoisonError};

mod cache;
mod face_choice;
mod metrics;
mod registry;
#[cfg(test)]
mod tests;

use cache::{
    InstanceKey, MAX_CACHED_INSTANCES, MAX_CACHED_PLANS, PlanKey, PreparedFont, RunShapeSetup,
    ShaperScratch, TwoGenCache, with_scratch,
};
use face_choice::RunFaceMatcher;
use metrics::{UnitMetrics, gdef_carets, unit_metrics};
use registry::RegisteredFace;
pub use registry::{CjkRegion, FaceProfile, FontRegistry};

/// How many bytes of text on each side of a shaped segment reach HarfRust as
/// context (spec SHP-3). HarfBuzz reads at most five code points.
const CONTEXT_BYTES: usize = 32;

/// The HarfRust shaping backend over one shared [`FontRegistry`]
/// (spec SHP-22). It is also the [`FontSource`] of its own id space.
///
/// It implements the whole [`TextShaper`] contract for horizontal and
/// vertical-upright text, GDEF ligature carets included.
#[derive(Clone)]
pub struct HarfRustShaper {
    registry: Arc<FontRegistry>,
    instances: Arc<Mutex<TwoGenCache<InstanceKey, (Font, UnitMetrics)>>>,
    plans: Arc<Mutex<TwoGenCache<PlanKey, Arc<ShapePlan>>>>,
}

impl fmt::Debug for HarfRustShaper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HarfRustShaper")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

/// Shared input/output target for shaping one run or window segment.
struct ShapingTarget<'a, 'out, 'm, 'reg, 's> {
    text: &'a str,
    /// The bytes of `text` readable as context; its edges are text edges.
    context: TextRange,
    run: &'a ShapingRun,
    atoms: &'a [GraphemeAtom],
    output: &'out mut ShapeOutput,
    matcher: &'m RunFaceMatcher<'reg>,
    setup: &'m RunShapeSetup,
    scratch: &'s mut ShaperScratch,
    last_font: Option<PreparedFont>,
}

/// One resolved font instance, its cache key, and its unit metrics.
struct ResolvedInstance {
    key: InstanceKey,
    font: Font,
    units: UnitMetrics,
}

/// End of a same-face slice inside a segment and the face starting after it.
struct FontRunNext {
    end: usize,
    pending: Option<Option<FontId>>,
}

impl HarfRustShaper {
    /// A shaper over `registry`.
    #[must_use]
    pub fn new(registry: Arc<FontRegistry>) -> Self {
        Self {
            registry,
            instances: Arc::default(),
            plans: Arc::default(),
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
        let resolved = self.instance_with_key(id, face, variations);
        Some((resolved.font, resolved.units))
    }

    fn instance_with_key(
        &self,
        id: FontId,
        face: &RegisteredFace,
        variations: &[FontVariation],
    ) -> ResolvedInstance {
        if variations.is_empty() || face.font.axes().is_empty() {
            return ResolvedInstance {
                key: (id, SmallVec::new()),
                font: face.font.clone(),
                units: face.metrics,
            };
        }
        // Distinct axes commute, so the key lists them by tag: `[wght, wdth]`
        // and `[wdth, wght]` share one instance. The sort is stable, so a
        // repeated tag keeps its order and its meaning.
        let mut coordinates: SmallVec<[FontVariation; 2]> = SmallVec::from_slice(variations);
        coordinates.sort_by_key(|v| v.tag());
        let key: InstanceKey = (id, coordinates);
        let mut cache = self
            .instances
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (font, units) = cache
            .get_or_insert_with(key.clone(), MAX_CACHED_INSTANCES, || {
                let font = face
                    .font
                    .instance_builder()
                    .variations(variations.iter().map(|v| (Tag::new(&v.tag()), v.value())))
                    .build();
                let metrics = unit_metrics(&font);
                (font, metrics)
            })
            .clone();
        ResolvedInstance { key, font, units }
    }

    fn prepare_font(
        &self,
        run: &ShapingRun,
        setup: &RunShapeSetup,
        id: FontId,
    ) -> Option<PreparedFont> {
        let face = self.registry.face(id)?;
        let resolved = self.instance_with_key(id, face, run.features.variations());
        let plan_key = PlanKey {
            instance: resolved.key,
            direction: setup.direction,
            script: setup.script,
            language: setup.language.clone(),
            features: SmallVec::from_slice(run.features.as_slice()),
        };
        let plan = {
            let mut cache = self.plans.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(cache.get_or_insert_with(plan_key, MAX_CACHED_PLANS, || {
                Arc::new(ShapePlan::new(
                    &resolved.font,
                    setup.direction.to_harfrust(),
                    setup.script,
                    setup.language.as_ref(),
                    &setup.features,
                ))
            }))
        };
        Some(PreparedFont {
            id,
            font: resolved.font,
            scale: run.font_size / resolved.units.units_per_em,
            plan,
        })
    }

    /// Shapes the atoms `target.atoms[range]` of `target.run`, all contiguous
    /// in bytes, appending clusters and glyphs. `cut_before` marks a seam the
    /// caller made: the first cluster is then not safe to concatenate (CLU-8).
    fn shape_segment(
        &self,
        target: &mut ShapingTarget<'_, '_, '_, '_, '_>,
        range: core::ops::Range<usize>,
        cut_before: bool,
    ) {
        let mut start = range.start;
        let group = target.run.orientation == GlyphOrientation::HorizontalInVertical;
        let mut pending = (start < range.end).then(|| {
            if group {
                target
                    .matcher
                    .face_for_group(target.text, &target.atoms[range.clone()])
            } else {
                target.matcher.face_for(target.text, &target.atoms[start])
            }
        });
        while let Some(font) = pending {
            let next = if group {
                FontRunNext {
                    end: range.end,
                    pending: None,
                }
            } else {
                (start + 1..range.end)
                    .find_map(|end| {
                        let next_font = target.matcher.face_for(target.text, &target.atoms[end]);
                        (next_font != font).then_some(FontRunNext {
                            end,
                            pending: Some(next_font),
                        })
                    })
                    .unwrap_or(FontRunNext {
                        end: range.end,
                        pending: None,
                    })
            };
            pending = next.pending;
            let first_cluster = target.output.clusters.len();
            self.shape_font_run(target, start..next.end, font);
            if cut_before
                && start == range.start
                && let Some(cluster) = target.output.clusters.get_mut(first_cluster)
            {
                cluster.safe_to_concat_before = false;
            }
            start = next.end;
        }
    }

    /// Shapes `target.atoms[range]` with one face. Without a face every atom
    /// gets an empty, zero-width cluster.
    fn shape_font_run(
        &self,
        target: &mut ShapingTarget<'_, '_, '_, '_, '_>,
        range: core::ops::Range<usize>,
        font: Option<FontId>,
    ) {
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
        if target.last_font.as_ref().is_none_or(|p| p.id != font_id) {
            target.last_font = self.prepare_font(run, target.setup, font_id);
        }
        let Some(prepared) = target.last_font.as_ref() else {
            push_empty_clusters(run, atoms, range, font_id, target.output);
            return;
        };
        target.scratch.reset_buffer();
        let buffer = &mut target.scratch.buffer;
        fill_segment_buffer(buffer, target.text, target.context, bytes);
        if !shape_buffer(run.font_size, target.setup, prepared, buffer) {
            push_empty_clusters(run, atoms, range, font_id, target.output);
            return;
        }
        let scale = prepared.scale;
        target.collect_tagged_glyphs(&range, bytes.start(), font_id, scale);
        push_clusters_with_scratch(
            run,
            atoms,
            range,
            &target.scratch.tagged,
            font_id,
            target.output,
            &mut target.scratch.groups,
        );
    }

    /// Glyphs of `replacement` in `font`, marked synthesized, for a
    /// hyphenation edit (spec SHP-6, CLU-7).
    fn synthesized_glyphs(
        &self,
        run: &ShapingRun,
        setup: &RunShapeSetup,
        font: FontId,
        replacement: &str,
    ) -> SmallVec<[RawShapedGlyph; 4]> {
        let Some(prepared) = self.prepare_font(run, setup, font) else {
            return SmallVec::new();
        };
        with_scratch(|scratch| {
            scratch.reset_buffer();
            let buffer = &mut scratch.buffer;
            buffer.push_str(replacement);
            // The replacement takes the kept text's language, features and size.
            if !shape_buffer(run.font_size, setup, &prepared, buffer) {
                return SmallVec::new();
            }
            let vertical = setup.direction.is_vertical();
            let mut glyphs: SmallVec<[RawShapedGlyph; 4]> = buffer
                .glyph_infos()
                .iter()
                .zip(buffer.glyph_positions())
                .map(|(info, pos)| raw_glyph(info, pos, prepared.scale, vertical, true))
                .collect();
            if setup.direction.is_horizontal_rtl() {
                glyphs.reverse();
            }
            glyphs
        })
    }
}

/// Loads `text[bytes]` with its surrounding context slice and edge flags into
/// an already-reset `buffer`.
fn fill_segment_buffer(buffer: &mut Buffer, text: &str, context: TextRange, bytes: TextRange) {
    for (offset, c) in text[bytes.as_range()].char_indices() {
        buffer.push(c as u32, u32::try_from(offset).unwrap_or(u32::MAX));
    }
    let context_slice = &text[..context.end()];
    buffer.set_pre_context(context_before(
        context_slice,
        context.start(),
        bytes.start(),
    ));
    buffer.set_post_context(context_after(context_slice, bytes.end()));
    let mut flags = BufferFlags::PRODUCE_UNSAFE_TO_CONCAT;
    if bytes.start() == context.start() {
        flags |= BufferFlags::BEGINNING_OF_TEXT;
    }
    if bytes.end() == context.end() {
        flags |= BufferFlags::END_OF_TEXT;
    }
    buffer.set_flags(flags);
}

impl ShapingTarget<'_, '_, '_, '_, '_> {
    /// Converts shaped buffer glyphs into logical-order [`TaggedGlyph`]s inside
    /// `self.scratch.tagged` (reusing its allocation).
    fn collect_tagged_glyphs(
        &mut self,
        range: &core::ops::Range<usize>,
        byte_start: usize,
        font_id: FontId,
        scale: f32,
    ) {
        let buffer = &self.scratch.buffer;
        let tagged = &mut self.scratch.tagged;
        let atoms = self.atoms;
        let setup = self.setup;
        let vertical = setup.direction.is_vertical();
        tagged.clear();
        let mut atom_hint = range.start;
        for (info, pos) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
            let byte = byte_start + info.cluster as usize;
            let atom = if atoms[atom_hint].byte_range.contains(byte) {
                atom_hint
            } else if atom_hint + 1 < range.end && atoms[atom_hint + 1].byte_range.contains(byte) {
                atom_hint + 1
            } else if atom_hint > range.start && atoms[atom_hint - 1].byte_range.contains(byte) {
                atom_hint - 1
            } else {
                atom_containing(atoms, range.clone(), byte)
            };
            atom_hint = atom;
            tagged.push(TaggedGlyph {
                atom,
                font: font_id,
                glyph: raw_glyph(info, pos, scale, vertical, false),
                unsafe_break: info.unsafe_to_break(),
                unsafe_concat: info.unsafe_to_concat(),
            });
        }
        if setup.direction.is_horizontal_rtl() {
            // HarfRust returns visual order; the contract is logical (SHP-11).
            tagged.reverse();
        }
    }
}

/// Sets the run's direction, script and language on `buffer` and shapes it
/// with `prepared`'s font and cached plan; `false` when HarfRust reports an
/// error. A buffer that was reset and refilled has no output, so none of
/// `ShapeError`'s misuse cases can arise.
fn shape_buffer(
    font_size: f32,
    setup: &RunShapeSetup,
    prepared: &PreparedFont,
    buffer: &mut Buffer,
) -> bool {
    buffer.set_direction(setup.direction.to_harfrust());
    buffer.set_script(setup.script);
    if let Some(ref language) = setup.language {
        buffer.set_language(Some(language.clone()));
    }
    let options = ShapeOptions::new()
        .plan(Some(&prepared.plan))
        .features(&setup.features)
        .point_size(Some(font_size));
    harfrust::shape(&ShaperFont::new(&prepared.font), buffer, options).is_ok()
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

/// Iterates the maximal byte-contiguous stretches of `atoms[range]`.
struct ContiguousSegments<'a> {
    atoms: &'a [GraphemeAtom],
    cursor: usize,
    end: usize,
}

impl<'a> ContiguousSegments<'a> {
    fn new(atoms: &'a [GraphemeAtom], range: core::ops::Range<usize>) -> Self {
        Self {
            atoms,
            cursor: range.start,
            end: range.end,
        }
    }
}

impl Iterator for ContiguousSegments<'_> {
    type Item = core::ops::Range<usize>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.end {
            return None;
        }
        let start = self.cursor;
        let mut index = start + 1;
        while index < self.end
            && self.atoms[index - 1].byte_range.end() == self.atoms[index].byte_range.start()
        {
            index += 1;
        }
        self.cursor = index;
        Some(start..index)
    }
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
        let Some(primary) = self.registry.primary_candidate(families, style) else {
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
        if atoms.is_empty() {
            return;
        }
        let matcher = RunFaceMatcher::new(&self.registry, run);
        let setup = RunShapeSetup::new(run);
        with_scratch(|scratch| {
            let mut target = ShapingTarget {
                text,
                context: TextRange::new(0, text.len()),
                run,
                atoms,
                output,
                matcher: &matcher,
                setup: &setup,
                scratch,
                last_font: None,
            };
            for segment in ContiguousSegments::new(atoms, 0..atoms.len()) {
                self.shape_segment(&mut target, segment, false);
            }
        });
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
        let matcher = RunFaceMatcher::new(&self.registry, run);
        let fallback = matcher.first_id();
        let setup = RunShapeSetup::new(run);
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
                    with_scratch(|scratch| {
                        let mut target = ShapingTarget {
                            text,
                            context,
                            run,
                            atoms,
                            output: &mut *output,
                            matcher: &matcher,
                            setup: &setup,
                            scratch,
                            last_font: None,
                        };
                        for (k, segment) in
                            ContiguousSegments::new(atoms, plan.kept_start..plan.kept_end)
                                .enumerate()
                        {
                            self.shape_segment(&mut target, segment, k == 0);
                        }
                    });
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
                        SmallVec::new()
                    } else {
                        self.synthesized_glyphs(run, &setup, font, text)
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

    fn variation_axis_range(&self, font: FontId, tag: [u8; 4]) -> Option<(f32, f32, f32)> {
        let face = self.registry.face(font)?;
        let target = Tag::new(&tag);
        face.font
            .axes()
            .iter()
            .find(|a| a.tag() == target)
            .map(|a| (a.min_value(), a.default_value(), a.max_value()))
    }
}

impl FontSource for HarfRustShaper {
    fn resolve_font(&self, font: FontId) -> Option<FontFace> {
        self.registry.resolve_font(font)
    }
}
