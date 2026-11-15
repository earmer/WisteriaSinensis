//! A browser harness for `mutil-engine`.
//!
//! Nothing here lays out text itself. The engine compiles, breaks and
//! validates the paragraph; this crate only
//!
//! * registers font faces and keeps their bytes for outline extraction,
//! * turns a validated layout into numbers a canvas can draw
//!   (`ValidatedLayout::glyph_runs`, spec HST-3/HST-4), and
//! * pulls glyph outlines out of the same bytes with `skrifa` for the
//!   renderer.
//!
//! The boundary is a small C ABI so the page needs no JS package manager:
//! strings and byte arrays are copied through exported `alloc`/`free`, and
//! every call returns a length-prefixed, packed pointer to UTF-8 JSON (or an
//! SVG path). The demo is compiled to `wasm32-unknown-unknown`; the crate is
//! also a plain `rlib` so `cargo test` can exercise it natively.

mod markup;

use std::sync::Arc;
use std::sync::Mutex;

use mutil_engine::{
    BaselineAlignment, FontBytes, FontFace, FontFamily, FontFeatures, FontRegistry, FontStyle,
    FontVariation, FrontendOptions, HarfRustShaper, LayoutConstraints, LayoutOptions, LineHeight,
    LocalePolicy, LocaleTag, ParagraphInput, QuotePolicy, StyleSpan, TextDecoration, TextStyle,
    ValidatedLayout, compile_paragraph, conformance_report, layout_paragraph,
};
use skrifa::outline::DrawSettings;
use skrifa::prelude::{LocationRef, Size};
use skrifa::{FontRef as SkrifaFont, GlyphId, MetadataProvider};

/// The registered families in the order every paragraph's stack lists them
/// after the chosen face.
const FIXED_STACK: [&str; 11] = [
    "Noto Sans",
    "Noto Serif",
    "JetBrains Mono",
    "Noto Sans SC",
    "Noto Sans TC",
    "Noto Sans JP",
    "Noto Sans Devanagari",
    "Noto Sans Thai",
    "Noto Sans Lao",
    "Noto Sans Khmer",
    "Noto Sans Myanmar",
];

/// The harness asks for Regular. Noto Sans JP's variable font defaults to
/// `wght=100`, so the setting is explicit and applied to shaping and drawing
/// alike; an outline drawn at a different instance than the layout would be a
/// renderer bug, not a design choice.
const WEIGHT: f32 = 400.0;

struct FaceEntry {
    family: Arc<str>,
    bytes: FontBytes,
}

/// The registered faces and the shaper built over them. The registry is
/// rebuilt only when a face is added, which the page does before laying out.
struct Demo {
    faces: Vec<FaceEntry>,
    shaper: Option<HarfRustShaper>,
}

impl Demo {
    fn new() -> Self {
        Self {
            faces: Vec::new(),
            shaper: None,
        }
    }

    fn add_font(&mut self, family: &str, bytes: Vec<u8>) -> Option<(u32, u16)> {
        if self.faces.iter().any(|face| *face.family == *family) {
            return None;
        }
        let font = SkrifaFont::new(bytes.as_slice()).ok()?;
        let upem = font
            .metrics(Size::unscaled(), LocationRef::default())
            .units_per_em;
        self.faces.push(FaceEntry {
            family: Arc::from(family),
            bytes: FontBytes::new(bytes),
        });
        self.shaper = None;
        let id = u32::try_from(self.faces.len() - 1).ok()?;
        Some((id, upem))
    }

    fn shaper(&mut self) -> &HarfRustShaper {
        if self.shaper.is_none() {
            let mut registry = FontRegistry::new();
            for face in &self.faces {
                registry.add_face(
                    Arc::clone(&face.family),
                    FontStyle::default(),
                    FontFace::new(face.bytes.clone(), 0),
                );
            }
            self.shaper = Some(HarfRustShaper::new(Arc::new(registry)));
        }
        self.shaper.as_ref().expect("just built")
    }

    /// The family stack handed to the engine: the chosen face, then the
    /// Latin faces, the CJK faces, the Indic and Southeast Asian faces and
    /// anything else registered. It is the same for every locale: the engine
    /// routes CJK runs to the regional face of their locale (spec SHP-12,
    /// SHP-25), so the page never reorders the stack.
    fn family_stack(&self, primary: Option<&str>) -> Arc<[FontFamily]> {
        let mut names: Vec<Arc<str>> = Vec::new();
        let mut push = |family: &str| {
            if let Some(face) = self.faces.iter().find(|face| *face.family == *family)
                && !names.iter().any(|name| **name == *face.family)
            {
                names.push(Arc::clone(&face.family));
            }
        };
        if let Some(primary) = primary {
            push(primary);
        }
        for family in FIXED_STACK {
            push(family);
        }
        for face in &self.faces {
            push(&face.family);
        }
        names
            .into_iter()
            .map(FontFamily::Named)
            .collect::<Vec<_>>()
            .into()
    }

    /// Compiles, lays out and validates one paragraph. The returned JSON is
    /// the engine's output, not a re-derived approximation.
    #[expect(
        clippy::too_many_arguments,
        reason = "one flat layout call crosses the wasm boundary"
    )]
    fn layout(
        &mut self,
        text: &str,
        width: f32,
        font_size: f32,
        line_height: f32,
        locale: Option<&str>,
        cjk_quotes: bool,
        primary: Option<&str>,
        heuristics: bool,
    ) -> String {
        let locale = match locale {
            Some(tag) => match LocaleTag::parse_lenient(tag) {
                Ok(tag) => Some(tag),
                Err(_) => return error_json("input", "not a BCP 47 language tag", None),
            },
            None => None,
        };
        // The host step: inline locale tags become style spans over the
        // plain paragraph text (spec AX-1, FE-25).
        let marked = markup::parse(text);
        let text = marked.text.as_str();
        let mut features = FontFeatures::new();
        if let Some(weight) = FontVariation::new(*b"wght", WEIGHT) {
            features.push_variation(weight);
        }
        let style = TextStyle {
            families: self.family_stack(primary),
            font_size,
            style: FontStyle::default(),
            features,
            decoration: TextDecoration::default(),
            baseline_align: BaselineAlignment::Auto,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            script: None,
            locale: locale.clone(),
        };
        let style_spans: Vec<StyleSpan> = marked
            .spans
            .iter()
            .map(|(range, locale)| StyleSpan {
                byte_range: *range,
                style: TextStyle {
                    locale: Some(locale.clone()),
                    ..style.clone()
                },
            })
            .collect();
        let input = ParagraphInput {
            text,
            default_style: &style,
            style_spans: &style_spans,
            inline_objects: &[],
            decoration_spans: &[],
            atomic_spans: &[],
            bidi_spans: &[],
            annotations: &[],
        };
        let constraints = LayoutConstraints {
            wrap_width: width,
            ..LayoutConstraints::default()
        };
        let options = LayoutOptions {
            line_height: if line_height > 0.0 {
                LineHeight::Fixed(line_height)
            } else {
                LineHeight::Normal
            },
            ..LayoutOptions::default()
        };
        let frontend = FrontendOptions {
            quote_policy: if cjk_quotes {
                QuotePolicy::CjkAware
            } else {
                QuotePolicy::Standard
            },
            default_locale: locale.unwrap_or_default(),
            locale_policy: if heuristics {
                LocalePolicy::ExperimentalHeuristics
            } else {
                LocalePolicy::ExplicitOnly
            },
            ..FrontendOptions::default()
        };
        let ast = match compile_paragraph(&input, &frontend, self.shaper()) {
            Ok(ast) => ast,
            Err(error) => return error_json("compile", &error.to_string(), None),
        };
        let paragraph = match layout_paragraph(
            &ast,
            text,
            self.shaper.as_ref().expect("built above"),
            &constraints,
            &options,
        ) {
            Ok(paragraph) => paragraph,
            Err(error) => return error_json("layout", &error.to_string(), None),
        };
        let layout = match ValidatedLayout::new(&paragraph, text) {
            Ok(layout) => layout,
            Err((line, error)) => {
                return error_json("validate", &error.to_string(), Some(line));
            }
        };
        out_json(&layout, font_size, width).finish()
    }

    fn outline(&self, font: u32, glyph: u32) -> String {
        let Ok(index) = usize::try_from(font) else {
            return String::new();
        };
        let Some(face) = self.faces.get(index) else {
            return String::new();
        };
        let Ok(font) = SkrifaFont::new(face.bytes.as_bytes()) else {
            return String::new();
        };
        let Some(outline) = font.outline_glyphs().get(GlyphId::new(glyph)) else {
            return String::new();
        };
        // Draw the same instance the layout shaped (see `WEIGHT`).
        let location = font.axes().location([("wght", WEIGHT)]);
        let mut pen = PathPen::default();
        // Font units, no hinting (spec HST-5/HST-6). The pen flips y into
        // canvas coordinates.
        if outline
            .draw(
                DrawSettings::unhinted(Size::unscaled(), &location),
                &mut pen,
            )
            .is_err()
        {
            return String::new();
        }
        pen.0
    }
}

fn error_json(stage: &str, message: &str, line: Option<usize>) -> String {
    let mut out = Out::new();
    out.raw("{\"ok\":false,\"stage\":");
    out.string(stage);
    out.raw(",\"error\":");
    out.string(message);
    out.raw(",\"line\":");
    match line {
        Some(line) => out.number(line as f32),
        None => out.raw("null"),
    }
    out.raw("}");
    out.finish()
}

/// The engine's own verdict on the paragraph: every line, its slots and
/// carets, and every placed glyph origin (spec HST-3/HST-4).
struct RunJson {
    line: usize,
    font: u32,
    size: f32,
    glyphs: Vec<(u32, f32, f32)>,
}

fn out_json(layout: &ValidatedLayout<'_>, font_size: f32, width: f32) -> Out {
    let paragraph = layout.paragraph();
    let mut out = Out::new();
    let mut runs: Vec<RunJson> = Vec::new();
    for run in layout.glyph_runs() {
        let size = run.font_size() * run.font().scale;
        let mut glyphs = Vec::new();
        for glyph in run.glyphs() {
            glyphs.push((glyph.glyph_id, glyph.origin.x, glyph.origin.y));
        }
        runs.push(RunJson {
            line: run.line(),
            font: run.font().font_id.0,
            size,
            glyphs,
        });
    }
    out.raw("{\"ok\":true,\"blockSize\":");
    out.number(paragraph.total_block_size);
    out.raw(",\"fontSize\":");
    out.number(font_size);
    out.raw(",\"width\":");
    out.number(width);
    out.raw(",\"lineCount\":");
    out.number(paragraph.lines.len() as f32);
    out.raw(",\"lineHeight\":");
    out.number(paragraph.lines.first().map_or(0.0, |line| line.line_height));
    out.raw(",\"lines\":[");
    for (index, line) in paragraph.lines.iter().enumerate() {
        if index > 0 {
            out.raw(",");
        }
        out.raw("{\"top\":");
        out.number(line.baseline_y - line.ascent);
        out.raw(",\"baseline\":");
        out.number(line.baseline_y);
        out.raw(",\"ascent\":");
        out.number(line.ascent);
        out.raw(",\"descent\":");
        out.number(line.descent);
        out.raw(",\"height\":");
        out.number(line.line_height);
        out.raw(",\"visibleEnd\":");
        out.number(line.visible_byte_end as f32);
        out.raw(",\"slots\":[");
        for (slot_index, slot) in line.slots.iter().enumerate() {
            if slot_index > 0 {
                out.raw(",");
            }
            out.raw("[");
            out.number(slot.content_start);
            out.raw(",");
            out.number(slot.content_end());
            out.raw("]");
        }
        out.raw("],\"carets\":[");
        for (caret_index, stop) in line.caret_stops.iter().enumerate() {
            if caret_index > 0 {
                out.raw(",");
            }
            out.number(stop.x);
        }
        out.raw("],\"runs\":[");
        let mut first = true;
        for run in runs.iter().filter(|run| run.line == index) {
            if !first {
                out.raw(",");
            }
            first = false;
            out.raw("{\"font\":");
            out.number(run.font as f32);
            out.raw(",\"size\":");
            out.number(run.size);
            out.raw(",\"glyphs\":[");
            for (glyph_index, (glyph, x, y)) in run.glyphs.iter().enumerate() {
                if glyph_index > 0 {
                    out.raw(",");
                }
                out.raw("[");
                out.number(*glyph as f32);
                out.raw(",");
                out.number(*x);
                out.raw(",");
                out.number(*y);
                out.raw("]");
            }
            out.raw("]}");
        }
        out.raw("]}");
    }
    out.raw("]}");
    out
}

/// Minimal JSON writer. Every value written here is an engine output number,
/// an id, or a string from an error, so only strings need escaping.
struct Out(String);

impl Out {
    fn new() -> Self {
        Self(String::with_capacity(4096))
    }
    fn raw(&mut self, text: &str) {
        self.0.push_str(text);
    }
    fn number(&mut self, value: f32) {
        if value.is_finite() {
            self.0.push_str(&value.to_string());
        } else {
            self.0.push_str("null");
        }
    }
    fn string(&mut self, text: &str) {
        self.0.push('"');
        for c in text.chars() {
            match c {
                '"' => self.0.push_str("\\\""),
                '\\' => self.0.push_str("\\\\"),
                '\n' => self.0.push_str("\\n"),
                '\r' => self.0.push_str("\\r"),
                '\t' => self.0.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    self.0.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => self.0.push(c),
            }
        }
        self.0.push('"');
    }
    fn finish(self) -> String {
        self.0
    }
}

/// Collects an outline as SVG path data, in font units with y pointing down
/// (the canvas convention), so the page can scale it by `size / upem`.
#[derive(Default)]
struct PathPen(String);

impl skrifa::outline::OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push('M');
        self.point(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push('L');
        self.point(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.push('Q');
        self.point(cx0, cy0);
        self.0.push(' ');
        self.point(x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.push('C');
        self.point(cx0, cy0);
        self.0.push(' ');
        self.point(cx1, cy1);
        self.0.push(' ');
        self.point(x, y);
    }
    fn close(&mut self) {
        self.0.push('Z');
    }
}

impl PathPen {
    fn point(&mut self, x: f32, y: f32) {
        // Two decimals of a font unit: far below a device pixel at any
        // plausible size, and short enough to keep the payload small.
        self.0.push_str(&format!("{:.2} {:.2}", x, -y));
    }
}

/// A global engine. The page registers faces once and lays out repeatedly.
static DEMO: Mutex<Option<Demo>> = Mutex::new(None);

fn with_demo<T>(f: impl FnOnce(&mut Demo) -> T) -> Option<T> {
    let mut guard = DEMO.lock().ok()?;
    Some(f(guard.get_or_insert_with(Demo::new)))
}

/// Packs a pointer and a length into one word: high 32 bits are the pointer,
/// low 32 the length (both fit in wasm32).
fn packed(ptr: *mut u8, len: usize) -> u64 {
    ((ptr as u64) << 32) | (len as u64 & 0xffff_ffff)
}

fn leak(bytes: Vec<u8>) -> u64 {
    let boxed = bytes.into_boxed_slice();
    let len = boxed.len();
    let ptr = Box::into_raw(boxed).cast::<u8>();
    packed(ptr, len)
}

/// Allocates `len` bytes for the page to write into; free with [`demo_free`].
#[unsafe(no_mangle)]
pub extern "C" fn demo_alloc(len: u32) -> *mut u8 {
    let len = len as usize;
    if len == 0 {
        return std::ptr::NonNull::<u8>::dangling().as_ptr();
    }
    let layout = std::alloc::Layout::array::<u8>(len).expect("byte layout");
    // SAFETY: the layout is non-zero-sized and valid.
    unsafe { std::alloc::alloc(layout) }
}

/// Releases a buffer from [`demo_alloc`] or a result from any call below.
///
/// # Safety
/// `ptr` must come from this module and `len` must be its exact length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_free(ptr: *mut u8, len: u32) {
    let len = len as usize;
    if len == 0 {
        return;
    }
    // SAFETY: the caller contract mirrors the allocator that produced `ptr`.
    unsafe {
        if let Ok(layout) = std::alloc::Layout::from_size_align(len, 1) {
            std::alloc::dealloc(ptr, layout);
        }
    }
}

fn read_str<'a>(ptr: *const u8, len: u32) -> Option<&'a str> {
    if len == 0 {
        return Some("");
    }
    // SAFETY: the page wrote `len` bytes at `ptr` before the call.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    std::str::from_utf8(bytes).ok()
}

/// Copies `bytes` into the engine's face table and registers it under
/// `family`. Returns `{"id":n,"upem":n}` or `{"id":null}` when the bytes hold
/// no font.
///
/// # Safety
/// `family_ptr` and `data_ptr` must each point to the stated number of bytes
/// that stay readable for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_add_font(
    family_ptr: *const u8,
    family_len: u32,
    data_ptr: *const u8,
    data_len: u32,
) -> u64 {
    let Some(family) = read_str(family_ptr, family_len) else {
        return leak(b"{\"id\":null}".to_vec());
    };
    if data_len == 0 {
        return leak(b"{\"id\":null}".to_vec());
    }
    // SAFETY: the page wrote `data_len` bytes at `data_ptr`.
    let bytes = unsafe { std::slice::from_raw_parts(data_ptr, data_len as usize) }.to_vec();
    let result = with_demo(|demo| demo.add_font(family, bytes));
    let json = match result {
        Some(Some((id, upem))) => {
            let mut out = Out::new();
            out.raw("{\"id\":");
            out.number(id as f32);
            out.raw(",\"upem\":");
            out.number(upem as f32);
            out.raw("}");
            out.finish()
        }
        _ => "{\"id\":null}".to_string(),
    };
    leak(json.into_bytes())
}

/// Lays out one paragraph. `line_height` 0 means normal.
///
/// # Safety
/// `text_ptr` must point to `text_len` bytes that stay readable for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_layout(
    text_ptr: *const u8,
    text_len: u32,
    width: f32,
    font_size: f32,
    line_height: f32,
    locale_ptr: *const u8,
    locale_len: u32,
    cjk_quotes: u32,
    primary_ptr: *const u8,
    primary_len: u32,
    heuristics: u32,
) -> u64 {
    let Some(text) = read_str(text_ptr, text_len) else {
        return leak(error_json("input", "text is not UTF-8", None).into_bytes());
    };
    let Some(locale) = read_str(locale_ptr, locale_len) else {
        return leak(error_json("input", "locale is not UTF-8", None).into_bytes());
    };
    let Some(primary) = read_str(primary_ptr, primary_len) else {
        return leak(error_json("input", "face is not UTF-8", None).into_bytes());
    };
    let locale = (!locale.is_empty()).then_some(locale);
    let primary = (!primary.is_empty()).then_some(primary);
    let result = with_demo(|demo| {
        demo.layout(
            text,
            width,
            font_size,
            line_height,
            locale,
            cjk_quotes != 0,
            primary,
            heuristics != 0,
        )
    });
    let json =
        result.unwrap_or_else(|| error_json("engine", "the engine is not initialised", None));
    leak(json.into_bytes())
}

/// The outline of `glyph` in `font`, in font units, y down, as SVG path data.
#[unsafe(no_mangle)]
pub extern "C" fn demo_outline(font_id: u32, glyph_id: u32) -> u64 {
    let path = with_demo(|demo| demo.outline(font_id, glyph_id)).unwrap_or_default();
    leak(path.into_bytes())
}

/// The components the engine itself reports as installed (spec SCO-8). A
/// `null` version means the component is not implemented.
#[unsafe(no_mangle)]
pub extern "C" fn demo_conformance() -> u64 {
    let report = conformance_report();
    let mut out = Out::new();
    out.raw("{\"unicode\":");
    out.string(mutil_engine::UNICODE_DATA_VERSION);
    for (name, value) in [
        ("grapheme", report.grapheme),
        ("bidi", report.bidi),
        ("lineBreak", report.line_break),
        ("word", report.word),
        ("script", report.script),
        ("emoji", report.emoji),
    ] {
        out.raw(",\"");
        out.raw(name);
        out.raw("\":");
        match value {
            Some(version) => out.string(version),
            None => out.raw("null"),
        }
    }
    out.raw("}");
    leak(out.finish().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Demo {
        let mut demo = Demo::new();
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../mutil-engine/tests/assets/JetBrainsMono-Subset.ttf"),
        )
        .expect("the test face");
        assert!(demo.add_font("JetBrains Mono", bytes).is_some());
        demo
    }

    #[test]
    fn a_paragraph_lays_out_and_validates() {
        let mut demo = engine();
        let json = demo.layout(
            "Hello world, hello world.",
            120.0,
            16.0,
            0.0,
            None,
            false,
            None,
            false,
        );
        assert!(json.starts_with("{\"ok\":true"), "{json}");
        assert!(json.contains("\"lines\":[{"), "{json}");
    }

    fn asset(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../mutil-engine/tests/assets")
                .join(name),
        )
        .expect("a vendored test face")
    }

    /// The face ids of the runs of the first line, in order.
    fn run_fonts(json: &str) -> Vec<u32> {
        json.match_indices("{\"font\":")
            .map(|(at, key)| {
                let digits: String = json[at + key.len()..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                digits.parse().expect("a font id")
            })
            .collect()
    }

    /// The sample line of the page: one fixed stack, three regional faces,
    /// chosen by the engine from the inline tags.
    #[test]
    fn inline_locale_tags_route_han_to_regional_faces() {
        let mut demo = Demo::new();
        let mut id =
            |family: &str, file: &str| demo.add_font(family, asset(file)).expect("registers").0;
        id("Noto Sans", "NotoSans-Subset.ttf");
        let sc = id("Noto Sans SC", "NotoSansSC-Subset.ttf");
        let tc = id("Noto Sans TC", "NotoSansTC-Subset.ttf");
        let jp = id("Noto Sans JP", "NotoSansJP-Subset.ttf");
        let json = demo.layout(
            "<zh-CN>令你甘心。骨肉分离。</zh-CN> <zh-TW>令你甘心。骨肉分離。</zh-TW> <ja>令。骨肉分離。</ja>",
            2000.0,
            18.0,
            0.0,
            None,
            true,
            Some("Noto Sans"),
            false,
        );
        assert!(json.starts_with("{\"ok\":true"), "{json}");
        let mut fonts = run_fonts(&json);
        fonts.dedup();
        // The word spaces are glue, drawn by no run.
        assert_eq!(fonts, [sc, tc, jp], "{json}");
    }

    #[test]
    fn a_shorthand_locale_is_accepted_and_a_malformed_one_is_not() {
        let mut demo = engine();
        let ok = demo.layout("abc", 200.0, 16.0, 0.0, Some("jp"), false, None, true);
        assert!(ok.starts_with("{\"ok\":true"), "{ok}");
        let bad = demo.layout("abc", 200.0, 16.0, 0.0, Some("ja JP"), false, None, false);
        assert!(bad.starts_with("{\"ok\":false"), "{bad}");
    }

    #[test]
    fn glyph_outlines_come_from_the_same_bytes() {
        let demo = engine();
        assert!(demo.outline(0, 43).starts_with('M'));
    }

    #[test]
    fn an_unregistered_font_has_no_outline() {
        let demo = engine();
        assert!(demo.outline(7, 0).is_empty());
    }
}
