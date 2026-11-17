//! The harness state behind the C ABI: the font table, the options the page
//! set, and every paragraph laid out since the page last cleared them.

use mutil_engine::{
    BaselineAlignment, BoxHeightStyle, CursorPosition, FontStyle, FrontendOptions, LayoutOptions,
    LineHeight, LocalePolicy, LocaleTag, Motion, ParagraphInput, Point, PositionedParagraph,
    QuotePolicy, StyleSpan, TextDecoration, TextRange, TextStyle, ValidatedLayout,
    compile_paragraph, layout_paragraph,
};

use crate::fonts::{self, Fonts};
use crate::json::{
    caret_json, cursor_move_json, error_json, hit_json, layout_json, range_rects_json,
    selection_rects_json,
};
use crate::markup;
use crate::options::{ExtraOptions, Phase2};

/// A laid-out paragraph as the page drew it: its text without the locale
/// markup, the tags removed from its source, and the paragraph.
struct StoredParagraph {
    text: String,
    removed: Vec<core::ops::Range<usize>>,
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

impl LayoutRequest<'_> {
    fn layout_options(&self) -> LayoutOptions {
        LayoutOptions {
            line_height: if self.line_height > 0.0 {
                LineHeight::Fixed(self.line_height)
            } else {
                LineHeight::Normal
            },
            justify: self.phase2.justify,
            compress_last_lines: self.phase2.compress_last_lines,
            hang_mode: self.phase2.hang_mode,
            whitespace: self.phase2.whitespace,
            ..LayoutOptions::default()
        }
    }

    fn frontend_options(&self, extra: ExtraOptions, locale: Option<LocaleTag>) -> FrontendOptions {
        FrontendOptions {
            quote_policy: if self.cjk_quotes {
                QuotePolicy::CjkAware
            } else {
                QuotePolicy::Standard
            },
            default_locale: locale.unwrap_or_default(),
            locale_policy: if self.heuristics {
                LocalePolicy::ExperimentalHeuristics
            } else {
                LocalePolicy::ExplicitOnly
            },
            base_direction: self.phase2.base_direction,
            autospace_profile: self.phase2.autospace,
            line_break_strictness: extra.line_break,
            word_break_option: extra.word_break,
            ..FrontendOptions::default()
        }
    }
}

/// The registered faces, the page's options and the laid-out paragraphs.
#[derive(Default)]
pub struct Demo {
    fonts: Fonts,
    paragraphs: Vec<ValidatedParagraph>,
    pub extra: ExtraOptions,
}

impl Demo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_font(&mut self, family: &str, bytes: Vec<u8>) -> Option<(u32, u16)> {
        self.fonts.add(family, bytes)
    }

    pub fn clear_paragraphs(&mut self) {
        self.paragraphs.clear();
    }

    /// The outline of `glyph` in `font` as SVG path data, empty when there
    /// is none.
    pub fn outline(&self, font: u32, glyph: u32) -> String {
        self.fonts.outline(font, glyph).unwrap_or_default()
    }

    fn default_style(
        &self,
        font_size: f32,
        primary: Option<&str>,
        locale: Option<LocaleTag>,
    ) -> TextStyle {
        TextStyle {
            families: self.fonts.family_stack(primary),
            font_size,
            style: FontStyle::default(),
            features: fonts::regular_weight(),
            decoration: TextDecoration::default(),
            baseline_align: BaselineAlignment::Auto,
            letter_spacing: self.extra.letter_spacing,
            word_spacing: self.extra.word_spacing,
            script: None,
            locale,
        }
    }

    /// Compiles, lays out and validates one paragraph. The returned JSON is
    /// the engine's output, not a re-derived approximation.
    pub fn layout(&mut self, req: &LayoutRequest<'_>) -> String {
        let Ok(locale) = req.locale.map(LocaleTag::parse_lenient).transpose() else {
            return error_json("input", "not a BCP 47 language tag", None);
        };
        // The host step: inline locale tags become style spans over the
        // plain paragraph text (spec AX-1, FE-25).
        let marked = markup::parse(req.text);
        let style = self.default_style(req.font_size, req.primary, locale.clone());
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
        let frontend = req.frontend_options(self.extra, locale);
        let constraints = self.extra.constraints(req.width);
        let shaper = self.fonts.shaper();
        let ast = match compile_paragraph(&input, &frontend, shaper) {
            Ok(ast) => ast,
            Err(error) => return error_json("compile", &error.to_string(), None),
        };
        let paragraph = match layout_paragraph(
            &ast,
            &marked.text,
            shaper,
            &constraints,
            &req.layout_options(),
        ) {
            Ok(paragraph) => paragraph,
            Err(error) => return error_json("layout", &error.to_string(), None),
        };
        self.store(
            StoredParagraph {
                text: marked.text,
                removed: marked.removed,
                paragraph,
            },
            req,
        )
    }

    /// Validates a laid-out paragraph, keeps it for later queries and
    /// returns its JSON under its new id.
    fn store(&mut self, stored: StoredParagraph, req: &LayoutRequest<'_>) -> String {
        let validated = match ValidatedParagraph::try_new(stored, |s| {
            ValidatedLayout::new(&s.paragraph, &s.text)
        }) {
            Ok(validated) => validated,
            Err((line, error)) => return error_json("validate", &error.to_string(), Some(line)),
        };
        let id = self.paragraphs.len();
        let json = layout_json(validated.borrow_dependent(), req.font_size, req.width, id);
        self.paragraphs.push(validated);
        json
    }

    fn validated(&self, para_id: usize) -> Option<&ValidatedLayout<'_>> {
        Some(self.paragraphs.get(para_id)?.borrow_dependent())
    }

    /// The offset in paragraph `para_id`'s laid-out text of byte
    /// `source_offset` of its source, which may hold locale markup.
    pub fn text_offset(&self, para_id: usize, source_offset: usize) -> Option<usize> {
        let stored = self.paragraphs.get(para_id)?.borrow_owner();
        Some(markup::text_offset(&stored.removed, source_offset).min(stored.text.len()))
    }

    pub fn hit_test(&self, para_id: usize, x: f32, y: f32) -> Option<String> {
        let layout = self.validated(para_id)?;
        Some(hit_json(layout.hit_test(Point { x, y })))
    }

    /// Hit-tests line `line` (`u32::MAX` for the last line) at
    /// `inline_position`.
    pub fn hit_test_on_line(
        &self,
        para_id: usize,
        line: u32,
        inline_position: f32,
    ) -> Option<String> {
        let layout = self.validated(para_id)?;
        let line_idx = if line == u32::MAX {
            layout.paragraph().lines.len().saturating_sub(1)
        } else {
            line as usize
        };
        Some(hit_json(layout.hit_test_on_line(line_idx, inline_position)))
    }

    pub fn caret_geometry(&self, para_id: usize, pos: CursorPosition) -> Option<String> {
        let layout = self.validated(para_id)?;
        Some(caret_json(layout.caret_geometry(pos)))
    }

    pub fn move_cursor(
        &self,
        para_id: usize,
        pos: CursorPosition,
        motion: Motion,
        preferred_inline: Option<f32>,
    ) -> Option<String> {
        let layout = self.validated(para_id)?;
        Some(cursor_move_json(layout.move_cursor(
            pos,
            motion,
            preferred_inline,
        )))
    }

    pub fn selection_geometry(
        &mut self,
        para_id: usize,
        start: usize,
        end: usize,
        height: BoxHeightStyle,
    ) -> Option<String> {
        let shaper = self.fonts.shaper();
        let layout = self.paragraphs.get(para_id)?.borrow_dependent();
        let range = clamped_range(layout, start, end);
        Some(selection_rects_json(
            &layout.selection_geometry(shaper, range, height),
        ))
    }

    pub fn range_rects(&self, para_id: usize, start: usize, end: usize) -> Option<String> {
        let layout = self.validated(para_id)?;
        Some(range_rects_json(
            &layout.range_rects(clamped_range(layout, start, end)),
        ))
    }
}

/// The range between `start` and `end`, in either order, clamped to the
/// layout's text.
fn clamped_range(layout: &ValidatedLayout<'_>, start: usize, end: usize) -> TextRange {
    let len = layout.text().len();
    let (start, end) = (start.min(len), end.min(len));
    TextRange::new(start.min(end), start.max(end))
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
