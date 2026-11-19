//! Font-unit metrics, GDEF ligature carets, and BASE table offsets.

use crate::types::FontMetrics;
use harfrust::Tag;
use read_fonts::TableProvider;
use read_fonts::model::Font;
use read_fonts::model::metrics::{LineBox, Metrics};
use read_fonts::types::F48Dot16;
use read_fonts::types::GlyphId;
use smallvec::SmallVec;

/// [`FontMetrics`] in font units, before a size is applied.
#[derive(Clone, Copy, Debug)]
pub(super) struct UnitMetrics {
    pub(super) units_per_em: f32,
    metrics: FontMetrics,
}

impl UnitMetrics {
    /// The metrics of an instance `font_size · scale` layout units per em.
    pub(super) fn scaled(&self, size: f32) -> FontMetrics {
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

/// The font's [`FontMetrics`] in font units at its variation location, with
/// the documented fallbacks for whatever its tables leave out (spec SHP-17,
/// SHP-23). The line metrics come from read-fonts' `Metrics`, which applies
/// the `MVAR` deltas and reads an ascender, descender or thickness by its
/// size, the convention SHP-23 adopts for the font format; every other value
/// is passed on as read and judged by the shaper gate (SHP-18, AX-3). Every
/// offset is positive towards line-over: `post` and `OS/2` already store the
/// underline and strike-out positions that way.
pub(super) fn unit_metrics(font: &Font) -> UnitMetrics {
    let metrics = font.metrics();
    // The stated value, not `Metrics::units_per_em`, which replaces one
    // outside 16..=16384 with 1000: glyph advances are in the stated units.
    // `FontRegistry::add_face` refuses a face whose value is 0 (SHP-21).
    let upem = f32::from(metrics.raw_units_per_em);
    let (ascent, descent, line_gap) = horizontal_line(metrics, upem);
    let (x_height, cap_height) = ink_heights(font, metrics, upem);
    let style = font.style_metrics();
    let (underline_position, underline_thickness) = style
        .underline
        .map(|d| (f(d.position), f(d.thickness).abs()))
        .unwrap_or((-0.1 * upem, 0.05 * upem));
    let strikeout_position = style
        .strike_through
        .map(|d| f(d.position))
        .unwrap_or(x_height / 2.0);
    let (vertical_ascent, vertical_descent, vertical_line_gap) = nonzero(metrics.vhea_line)
        .map(|l| (f(l.ascender), -f(l.descender), f(l.line_gap)))
        .unwrap_or((0.5 * upem, 0.5 * upem, 0.0));
    let base = base_offsets(font);
    let ideographic = base.ideo.unwrap_or(-descent);
    let central = match (base.ideo, base.idtp) {
        (Some(bottom), Some(top)) => (bottom + top) / 2.0,
        _ => ideographic + 0.5 * upem,
    };
    UnitMetrics {
        units_per_em: upem,
        metrics: FontMetrics {
            vertical_ascent,
            vertical_descent,
            vertical_line_gap,
            ideographic_baseline_offset: ideographic,
            central_baseline_offset: central,
            hanging_baseline_offset: base.hang.unwrap_or(0.8 * ascent),
            mathematical_baseline_offset: base.math.unwrap_or(0.5 * x_height),
            ascent,
            descent,
            line_gap,
            underline_position,
            underline_thickness,
            strikeout_position,
            x_height,
            cap_height,
            use_typo_metrics: metrics.use_typo_metrics,
        },
    }
}

fn f(v: read_fonts::types::F48Dot16) -> f32 {
    v.to_f32()
}

/// The line box of a table, unless it is all zero; a zero table is skipped in
/// favour of the next (spec SHP-23). `Metrics` gives the ascender as a size
/// and the descender as a negative size, so a non-zero box spans a positive
/// height.
fn nonzero(line: Option<LineBox>) -> Option<LineBox> {
    line.filter(|l| l.ascender.to_f32() - l.descender.to_f32() > 0.0)
}

/// Ascent, descent and line gap from `OS/2` typo or `hhea` (by
/// `use_typo_metrics`), else `OS/2` win, else 0.8 / 0.2 em (spec SHP-17).
fn horizontal_line(metrics: &Metrics, upem: f32) -> (f32, f32, f32) {
    let typo = nonzero(metrics.typo_line);
    let hhea = nonzero(metrics.hhea_line);
    let chosen = if metrics.use_typo_metrics {
        typo.or(hhea)
    } else {
        hhea.or(typo)
    };
    match (chosen, metrics.win_line) {
        (Some(line), _) => (f(line.ascender), -f(line.descender), f(line.line_gap)),
        (None, Some(win)) if f(win.ascender) - f(win.descender) > 0.0 => {
            (f(win.ascender), -f(win.descender), 0.0)
        }
        _ => (0.8 * upem, 0.2 * upem, 0.0),
    }
}

/// The x-height and cap height: the table value when positive, else the
/// ink top of `x` and `H`, else 0.5 / 0.7 em (spec SHP-17).
fn ink_heights(font: &Font, metrics: &Metrics, upem: f32) -> (f32, f32) {
    let height = |table: Option<F48Dot16>, c: char, default: f32| {
        table
            .map(f)
            .filter(|v| *v > 0.0)
            .unwrap_or_else(|| metrics_value(font, c).unwrap_or(default))
    };
    (
        height(metrics.x_height, 'x', 0.5 * upem),
        height(metrics.cap_height, 'H', 0.7 * upem),
    )
}

/// The top of the ink of `c`'s glyph, in font units.
pub(super) fn metrics_value(font: &Font, c: char) -> Option<f32> {
    let glyph: GlyphId = font.charmap().map_unicode(c)?;
    let extents = font.glyph_metrics().extents(glyph)?;
    Some(extents.y_bearing).filter(|v| *v > 0.0)
}

/// The GDEF ligature caret coordinates of `glyph_id` in font units, at the
/// font's variation location (spec SHP-7). A caret given as a contour point
/// (format 2) needs the outline, which the backend does not load, so a glyph
/// with one answers none and takes the equal-slot fallback (INT-5); device
/// tables only adjust hinted sizes and are ignored.
pub(super) fn gdef_carets(font: &Font, glyph_id: u32) -> SmallVec<[f32; 4]> {
    use read_fonts::tables::gdef::CaretValue;
    use read_fonts::tables::layout::DeviceOrVariationIndex;
    use read_fonts::tables::variations::DeltaSetIndex;
    let read = || -> Option<SmallVec<[f32; 4]>> {
        let tables = font.tables();
        let gdef = tables.gdef().ok()?;
        let list = gdef.lig_caret_list()?.ok()?;
        let index = list.coverage().ok()?.get(GlyphId::new(glyph_id))?;
        let glyph = list.lig_glyphs().get(usize::try_from(index).ok()?).ok()?;
        let coords = font.normalized_coords();
        let store = || gdef.item_var_store()?.ok();
        glyph
            .caret_values()
            .iter()
            .map(|caret| match caret.ok()? {
                CaretValue::Format1(c) => Some(f32::from(c.coordinate())),
                CaretValue::Format2(_) => None,
                CaretValue::Format3(c) => {
                    let base = f32::from(c.coordinate());
                    let delta = match c.device() {
                        Ok(DeviceOrVariationIndex::VariationIndex(v))
                            if coords.iter().any(|c| c.to_bits() != 0) =>
                        {
                            let index = DeltaSetIndex {
                                outer: v.delta_set_outer_index(),
                                inner: v.delta_set_inner_index(),
                            };
                            store()?.compute_delta(index, coords)?.to_f32()
                        }
                        _ => 0.0,
                    };
                    Some(base + delta)
                }
            })
            .collect()
    };
    read().unwrap_or_default()
}

#[derive(Default)]
struct BaseOffsets {
    ideo: Option<f32>,
    idtp: Option<f32>,
    hang: Option<f32>,
    math: Option<f32>,
}

/// Baseline offsets read from the horizontal axis of `BASE`, relative to the
/// alphabetic (`romn`) baseline when the font gives one.
fn base_offsets(font: &Font) -> BaseOffsets {
    let read = || -> Option<BaseOffsets> {
        let tables = font.tables();
        let base = tables.base().ok()?;
        let axis = base.horiz_axis()?.ok()?;
        let tag_list = axis.base_tag_list()?.ok()?;
        let tags = tag_list.baseline_tags();
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
            let index = tags.iter().position(|t| t.get() == Tag::new(tag))?;
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
