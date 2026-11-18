//! A browser harness for `mutil-engine`. The engine compiles, breaks and
//! validates each paragraph; this crate only registers font faces, turns a
//! validated layout into numbers a canvas can draw (spec HST-3/HST-4) and
//! draws glyph outlines from the same bytes with `skrifa`.
//!
//! The boundary is a small C ABI, so the page needs no JS package manager:
//! strings and bytes are copied through `demo_alloc`/`demo_free`, enums travel
//! as integer codes, and results come back as packed pointers to UTF-8 JSON
//! (or SVG path data). The crate is also an `rlib` so `cargo test` runs natively.

mod buffers;
mod fonts;
mod json;
mod markup;

use std::sync::Mutex;

use mutil_engine::{
    AutospaceProfile, BaselineAlignment, BidiDirection, BoxHeightStyle, CaretAffinity,
    CursorPosition, FontStyle, FrontendOptions, HangEndMode, HangMode, HangStartMode,
    HangingWhitespaceMode, LayoutConstraints, LayoutOptions, LineBreakStrictness, LineHeight,
    LocalePolicy, LocaleTag, Motion, ParagraphInput, Point, PositionedParagraph, QuotePolicy,
    StyleSpan, TextDecoration, TextRange, TextStyle, ValidatedLayout, WordBreakOption,
    compile_paragraph, layout_paragraph,
};

use fonts::Fonts;
use json::{caret_json, cursor_move_json, error_json, hit_json, layout_json, rects_json};

/// A laid-out paragraph and its source with the locale markup stripped.
struct StoredParagraph {
    marked: markup::Marked,
    paragraph: PositionedParagraph,
}

self_cell::self_cell!(
    /// A stored paragraph with the view that passed the output gate, kept so
    /// queries never re-validate it.
    struct ValidatedParagraph {
        owner: StoredParagraph,
        #[covariant]
        dependent: ValidatedLayout,
    }
);

/// One `demo_layout` call.
pub struct LayoutRequest<'a> {
    pub text: &'a str,
    pub width: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub locale: Option<&'a str>,
    pub cjk_quotes: bool,
    pub primary: Option<&'a str>,
    pub heuristics: bool,
    pub phase2: Phase2,
}

/// The Phase 2 options packed into one `u32` across the C ABI: bits 0-1 the
/// base direction (0 from the text, 1 LTR, 2 RTL), bit 2 `justify`, bits 3-4
/// the start hang (0 none, 1 first line, 2 all lines), bits 5-6 the end hang
/// (0 none, 1 allow, 2 force), bit 7 the `CssCompat` autospace profile, bit 8
/// preserved trailing whitespace and bit 9 `compress_last_lines`.
#[derive(Clone, Copy, Default)]
pub struct Phase2(pub u32);

impl Phase2 {
    /// Decodes the two-bit field at `shift` through `table` (see [`pick`]).
    fn field<T: Copy>(self, shift: u32, table: &[T]) -> T {
        pick((self.0 >> shift) & 0b11, table)
    }

    fn flag(self, bit: u32) -> bool {
        self.0 & (1 << bit) != 0
    }
}

/// Decodes an ABI enum code: `table[code]`, or `table[0]` for an unknown one.
fn pick<T: Copy>(code: impl Into<u32>, table: &[T]) -> T {
    let code: u32 = code.into();
    table.get(code as usize).copied().unwrap_or(table[0])
}

/// The options set once by `demo_set_options`, which every later layout
/// completes with its own.
#[derive(Default)]
struct ExtraOptions {
    frontend: FrontendOptions,
    constraints: LayoutConstraints,
    letter_spacing: f32,
    word_spacing: f32,
}

/// The registered faces, the page's options and the laid-out paragraphs.
#[derive(Default)]
pub struct Demo {
    fonts: Fonts,
    paragraphs: Vec<ValidatedParagraph>,
    extra: ExtraOptions,
}

impl Demo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_font(&mut self, family: &str, bytes: Vec<u8>) -> Option<(u32, u16)> {
        self.fonts.add(family, bytes)
    }

    /// Drops every laid-out paragraph, keeping the registered faces.
    pub fn clear_paragraphs(&mut self) {
        self.paragraphs.clear();
    }

    /// The outline of `glyph` in `font` as SVG path data, empty when there
    /// is none.
    pub fn outline(&self, font: u32, glyph: u32) -> String {
        self.fonts.outline(font, glyph).unwrap_or_default()
    }

    /// Compiles, lays out and validates one paragraph, keeps it for later
    /// queries and returns the engine's output as JSON under its new id.
    pub fn layout(&mut self, req: &LayoutRequest<'_>) -> String {
        self.try_layout(req).unwrap_or_else(|error| error)
    }

    /// [`Demo::layout`], with the error JSON of the stage that failed.
    fn try_layout(&mut self, req: &LayoutRequest<'_>) -> Result<String, String> {
        let locale = req.locale.map(LocaleTag::parse_lenient).transpose();
        let locale = locale.map_err(|_| error_json("input", "not a BCP 47 language tag"))?;
        // The host step: inline locale tags become style spans over the
        // plain paragraph text (spec AX-1, FE-25).
        let marked = markup::parse(req.text);
        let style = TextStyle {
            families: self.fonts.family_stack(req.primary),
            font_size: req.font_size,
            style: FontStyle::default(),
            features: fonts::regular_weight(),
            decoration: TextDecoration::default(),
            baseline_align: BaselineAlignment::Auto,
            letter_spacing: self.extra.letter_spacing,
            word_spacing: self.extra.word_spacing,
            script: None,
            locale: locale.clone(),
        };
        let style_spans: Vec<StyleSpan> = marked
            .spans
            .iter()
            .map(|(range, span_locale)| StyleSpan {
                byte_range: *range,
                style: TextStyle {
                    locale: Some(span_locale.clone()),
                    ..style.clone()
                },
            })
            .collect();
        let input = ParagraphInput {
            text: &marked.text,
            default_style: &style,
            style_spans: &style_spans,
            inline_objects: &[],
            decoration_spans: &[],
            atomic_spans: &[],
            bidi_spans: &[],
            annotations: &[],
        };
        let constraints = LayoutConstraints {
            wrap_width: req.width,
            ..self.extra.constraints.clone()
        };
        let frontend = req.frontend(&self.extra.frontend, locale);
        let shaper = self.fonts.shaper();
        let ast =
            compile_paragraph(&input, &frontend, shaper).map_err(|e| error_json("compile", e))?;
        let options = req.layout_options();
        let paragraph = layout_paragraph(&ast, &marked.text, shaper, &constraints, &options)
            .map_err(|e| error_json("layout", e))?;
        let stored = StoredParagraph { marked, paragraph };
        let validated = ValidatedParagraph::try_new(stored, |s| {
            ValidatedLayout::new(&s.paragraph, &s.marked.text)
        })
        .map_err(|(_, e)| error_json("validate", e))?;
        let json = layout_json(validated.borrow_dependent(), self.paragraphs.len());
        self.paragraphs.push(validated);
        Ok(json)
    }
}

impl LayoutRequest<'_> {
    fn frontend(&self, extra: &FrontendOptions, locale: Option<LocaleTag>) -> FrontendOptions {
        use {AutospaceProfile::*, BidiDirection as Dir, LocalePolicy::*, QuotePolicy::*};
        let bits = self.phase2;
        FrontendOptions {
            quote_policy: pick(self.cjk_quotes, &[Standard, CjkAware]),
            default_locale: locale.unwrap_or_default(),
            locale_policy: pick(self.heuristics, &[ExplicitOnly, ExperimentalHeuristics]),
            base_direction: bits.field(0, &[None, Some(Dir::Ltr), Some(Dir::Rtl)]),
            autospace_profile: pick(bits.flag(7), &[Jlreq, CssCompat]),
            ..extra.clone()
        }
    }

    fn layout_options(&self) -> LayoutOptions {
        use {HangEndMode as End, HangStartMode as Start, HangingWhitespaceMode::*};
        let bits = self.phase2;
        LayoutOptions {
            line_height: if self.line_height > 0.0 {
                LineHeight::Fixed(self.line_height)
            } else {
                LineHeight::Normal
            },
            justify: bits.flag(2),
            compress_last_lines: bits.flag(9),
            hang_mode: HangMode {
                start: bits.field(3, &[Start::None, Start::FirstLine, Start::AllLines]),
                end: bits.field(5, &[End::None, End::Allow, End::Force]),
            },
            whitespace: pick(bits.flag(8), &[Collapsible, Preserved]),
            ..LayoutOptions::default()
        }
    }
}

/// A global engine. The page registers faces once and lays out repeatedly.
static DEMO: Mutex<Option<Demo>> = Mutex::new(None);

fn with_demo<T>(f: impl FnOnce(&mut Demo) -> T) -> Option<T> {
    let mut guard = DEMO.lock().ok()?;
    Some(f(guard.get_or_insert_with(Demo::new)))
}

/// Answers a query on paragraph `para_id`, or returns `fallback` when there
/// is no such paragraph.
fn query(
    fallback: &str,
    para_id: u32,
    answer: impl FnOnce(&ValidatedLayout<'_>, &mut Fonts) -> String,
) -> u64 {
    let json = with_demo(|demo| {
        let layout = demo.paragraphs.get(para_id as usize)?.borrow_dependent();
        Some(answer(layout, &mut demo.fonts))
    });
    leak(json.flatten().unwrap_or_else(|| fallback.to_owned()))
}

/// Hands `text` to the page as one word it releases with [`demo_free`]: the
/// buffer's address in the high 32 bits, its length in the low 32.
fn leak(text: String) -> u64 {
    let len = text.len() as u64;
    ((buffers::register(text.into_bytes().into_boxed_slice()) as u64) << 32) | len
}

fn read_str(ptr: u32, len: u32) -> Option<String> {
    String::from_utf8(buffers::read(ptr as usize, len as usize)?).ok()
}

/// A caret at `byte_offset`; a non-zero `affinity` is upstream.
fn cursor(byte_offset: u32, affinity: u32) -> CursorPosition {
    use CaretAffinity::{Downstream, Upstream};
    CursorPosition {
        byte_offset: byte_offset as usize,
        affinity: pick(affinity != 0, &[Downstream, Upstream]),
    }
}

/// The range between `start` and `end`, in either order, clamped to the
/// layout's text.
fn clamped_range(layout: &ValidatedLayout<'_>, start: u32, end: u32) -> TextRange {
    let len = layout.text().len();
    let (start, end) = ((start as usize).min(len), (end as usize).min(len));
    TextRange::new(start.min(end), start.max(end))
}

/// Allocates `len` zeroed bytes for the page to write into and returns their
/// address, or 0 when memory runs out. Release it with [`demo_free`].
#[unsafe(no_mangle)]
pub extern "C" fn demo_alloc(len: u32) -> u32 {
    buffers::zeroed(len as usize).map_or(0, |bytes| buffers::register(bytes) as u32)
}

/// Releases the buffer at `ptr`, from [`demo_alloc`] or a result word. An
/// address that is not a live buffer is ignored.
#[unsafe(no_mangle)]
pub extern "C" fn demo_free(ptr: u32) {
    buffers::release(ptr as usize);
}

/// The number of buffers not yet released; the page expects 0 between calls.
#[unsafe(no_mangle)]
pub extern "C" fn demo_live_buffers() -> u32 {
    buffers::count() as u32
}

/// Copies `bytes` into the engine's face table and registers it under
/// `family`. Returns `{"id":n,"upem":n}` or `{"id":null}` when the bytes hold
/// no font. Both buffers come from [`demo_alloc`] and stay the page's to free.
#[unsafe(no_mangle)]
pub extern "C" fn demo_add_font(
    family_ptr: u32,
    family_len: u32,
    data_ptr: u32,
    data_len: u32,
) -> u64 {
    let registered = read_str(family_ptr, family_len)
        .filter(|_| data_len != 0)
        .zip(buffers::read(data_ptr as usize, data_len as usize))
        .and_then(|(family, bytes)| with_demo(|demo| demo.add_font(&family, bytes)).flatten());
    leak(json::font_json(registered))
}

/// Lays out one paragraph. `line_height` 0 means normal; `phase2` packs the
/// bidi, justification, hanging and autospace options ([`Phase2`]).
/// The three strings come from [`demo_alloc`] and stay the page's to free.
#[unsafe(no_mangle)]
pub extern "C" fn demo_layout(
    text_ptr: u32,
    text_len: u32,
    width: f32,
    font_size: f32,
    line_height: f32,
    locale_ptr: u32,
    locale_len: u32,
    cjk_quotes: u32,
    primary_ptr: u32,
    primary_len: u32,
    heuristics: u32,
    phase2: u32,
) -> u64 {
    let strings = [
        (text_ptr, text_len, "text"),
        (locale_ptr, locale_len, "locale"),
        (primary_ptr, primary_len, "face"),
    ]
    .map(|(ptr, len, name)| read_str(ptr, len).ok_or(name));
    let [text, locale, primary] = match strings {
        [Ok(text), Ok(locale), Ok(primary)] => [text, locale, primary],
        [Err(name), ..] | [_, Err(name), _] | [.., Err(name)] => {
            return leak(error_json("input", format_args!("{name} is not UTF-8")));
        }
    };
    let req = LayoutRequest {
        text: &text,
        width,
        font_size,
        line_height,
        locale: (!locale.is_empty()).then_some(&*locale),
        cjk_quotes: cjk_quotes != 0,
        primary: (!primary.is_empty()).then_some(&*primary),
        heuristics: heuristics != 0,
        phase2: Phase2(phase2),
    };
    leak(
        with_demo(|demo| demo.layout(&req))
            .unwrap_or_else(|| error_json("engine", "the engine is not initialised")),
    )
}

/// The outline of `glyph` in `font`, in font units, y down, as SVG path data.
#[unsafe(no_mangle)]
pub extern "C" fn demo_outline(font_id: u32, glyph_id: u32) -> u64 {
    leak(with_demo(|demo| demo.outline(font_id, glyph_id)).unwrap_or_default())
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_conformance() -> u64 {
    leak(json::conformance_json())
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_clear_paragraphs() {
    with_demo(Demo::clear_paragraphs);
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_set_options(
    line_break: u32,
    word_break: u32,
    letter_spacing: f32,
    word_spacing: f32,
    first_line_indent: f32,
    hanging_indent: f32,
) {
    use {LineBreakStrictness::*, WordBreakOption as Wb};
    let frontend = FrontendOptions {
        line_break_strictness: pick(line_break, &[Normal, Loose, Strict, Anywhere]),
        word_break_option: pick(word_break, &[Wb::Normal, Wb::BreakAll, Wb::KeepAll]),
        ..FrontendOptions::default()
    };
    let constraints = LayoutConstraints {
        first_line_indent,
        hanging_indent,
        ..LayoutConstraints::default()
    };
    with_demo(|demo| {
        demo.extra = ExtraOptions {
            frontend,
            constraints,
            letter_spacing,
            word_spacing,
        };
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_hit_test(para_id: u32, x: f32, y: f32) -> u64 {
    query("null", para_id, |layout, _| {
        hit_json(layout.hit_test(Point { x, y }))
    })
}

/// Hit-tests line `line` (`u32::MAX` for the last line) at `inline_position`.
#[unsafe(no_mangle)]
pub extern "C" fn demo_hit_test_on_line(para_id: u32, line: u32, inline_position: f32) -> u64 {
    query("null", para_id, |layout, _| {
        let line = match line {
            u32::MAX => layout.paragraph().lines.len().saturating_sub(1),
            line => line as usize,
        };
        hit_json(layout.hit_test_on_line(line, inline_position))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_caret_geometry(para_id: u32, byte_offset: u32, affinity: u32) -> u64 {
    let pos = cursor(byte_offset, affinity);
    query("null", para_id, |layout, _| {
        caret_json(layout.caret_geometry(pos))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_move_cursor(
    para_id: u32,
    byte_offset: u32,
    affinity: u32,
    motion: u32,
    has_preferred: u32,
    preferred_inline: f32,
) -> u64 {
    use Motion::*;
    let pos = cursor(byte_offset, affinity);
    let motions = [
        Left, Right, Up, Down, LineStart, LineEnd, WordLeft, WordRight, Previous, Next,
    ];
    let preferred = (has_preferred != 0).then_some(preferred_inline);
    query("null", para_id, |layout, _| {
        cursor_move_json(layout.move_cursor(pos, pick(motion, &motions), preferred))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_selection_geometry(para_id: u32, start: u32, end: u32, height: u32) -> u64 {
    use BoxHeightStyle::{Max, Strut, Tight};
    let height = pick(height, &[Tight, Max, Strut]);
    query("[]", para_id, |layout, fonts| {
        let range = clamped_range(layout, start, end);
        let rects = layout.selection_geometry(fonts.shaper(), range, height);
        rects_json(rects.iter().map(|selected| selected.rect))
    })
}

/// The offset in paragraph `para_id`'s laid-out text of byte `source_offset`
/// of its source, past any locale tags; `u32::MAX` for no such paragraph.
#[unsafe(no_mangle)]
pub extern "C" fn demo_text_offset(para_id: u32, source_offset: u32) -> u32 {
    with_demo(|demo| {
        let stored = demo.paragraphs.get(para_id as usize)?.borrow_owner();
        let offset = markup::text_offset(&stored.marked.removed, source_offset as usize);
        Some(offset.min(stored.marked.text.len()))
    })
    .flatten()
    .map_or(u32::MAX, |offset| offset as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn demo_range_rects(para_id: u32, start: u32, end: u32) -> u64 {
    query("[]", para_id, |layout, _| {
        rects_json(layout.range_rects(clamped_range(layout, start, end)))
    })
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

    fn req<'a>(text: &'a str, width: f32) -> LayoutRequest<'a> {
        LayoutRequest {
            text,
            width,
            font_size: 16.0,
            line_height: 0.0,
            locale: None,
            cjk_quotes: false,
            primary: None,
            heuristics: false,
            phase2: Phase2::default(),
        }
    }

    #[test]
    fn a_paragraph_lays_out_and_validates() {
        let mut demo = engine();
        let json = demo.layout(&req("Hello world, hello world.", 120.0));
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
        let json = demo.layout(&LayoutRequest {
            font_size: 18.0,
            cjk_quotes: true,
            primary: Some("Noto Sans"),
            ..req(
                "<zh-CN>令你甘心。骨肉分离。</zh-CN> <zh-TW>令你甘心。骨肉分離。</zh-TW> <ja>令。骨肉分離。</ja>",
                2000.0,
            )
        });
        assert!(json.starts_with("{\"ok\":true"), "{json}");
        let mut fonts = run_fonts(&json);
        fonts.dedup();
        // The word spaces are glue, drawn by no run.
        assert_eq!(fonts, [sc, tc, jp], "{json}");
    }

    #[test]
    fn a_shorthand_locale_is_accepted_and_a_malformed_one_is_not() {
        let mut demo = engine();
        let ok = demo.layout(&LayoutRequest {
            locale: Some("jp"),
            heuristics: true,
            phase2: Phase2::default(),
            ..req("abc", 200.0)
        });
        assert!(ok.starts_with("{\"ok\":true"), "{ok}");
        let bad = demo.layout(&LayoutRequest {
            locale: Some("ja JP"),
            ..req("abc", 200.0)
        });
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
