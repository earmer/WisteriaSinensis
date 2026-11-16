//! The shared test shaper and styles.

use mutil_engine::*;
use std::sync::{Arc, LazyLock};

/// The vendored subset of JetBrains Mono (1000 units per em, every spacing
/// glyph 600 units).
pub static FONT_BYTES: &[u8] = include_bytes!("../assets/JetBrainsMono-Subset.ttf");

pub const FAMILY: &str = "JetBrains Mono";

/// A shaper over a registry holding the test font as face 0, also the
/// generic monospace family.
pub static SHAPER: LazyLock<HarfRustShaper> = LazyLock::new(|| {
    let mut registry = FontRegistry::new();
    let id = registry
        .add_face(
            FAMILY.into(),
            FontStyle::default(),
            FontFace::new(FontBytes::from_static(FONT_BYTES), 0),
        )
        .expect("the test font parses");
    assert_eq!(id, FontId(0));
    registry.set_generic(GenericFamily::Monospace, FAMILY.into());
    HarfRustShaper::new(Arc::new(registry))
});

/// The test font at `font_size`.
pub fn style(font_size: f32) -> TextStyle {
    TextStyle {
        families: Arc::from([FontFamily::Named(FAMILY.into())]),
        font_size,
        style: FontStyle::default(),
        features: FontFeatures::default(),
        decoration: TextDecoration::default(),
        baseline_align: BaselineAlignment::Auto,
        letter_spacing: 0.0,
        word_spacing: 0.0,
        script: None,
        locale: None,
    }
}

pub fn instance(font_size: f32) -> FontInstanceRef<'static> {
    FontInstanceRef {
        font: FontRef::unscaled(FontId(0)),
        font_size,
        variations: &[],
    }
}

/// Asserts `a` and `b` agree to f32 rounding.
#[track_caller]
pub fn assert_close(a: f32, b: f32) {
    assert!((a - b).abs() <= 1e-4 * b.abs().max(1.0), "{a} != {b}");
}

/// A paragraph of `text` in `style` with nothing else.
pub fn input<'a>(text: &'a str, style: &'a TextStyle) -> ParagraphInput<'a> {
    ParagraphInput {
        text,
        default_style: style,
        style_spans: &[],
        inline_objects: &[],
        decoration_spans: &[],
        atomic_spans: &[],
        bidi_spans: &[],
        annotations: &[],
    }
}

/// Compiles `text` in the test font at 10 units per em.
pub fn compile(text: &str) -> ParagraphAst {
    let style = style(10.0);
    compile_paragraph(&input(text, &style), &FrontendOptions::default(), &*SHAPER)
        .expect("the frontend accepts the text")
}

/// A column `wrap_width` wide.
pub fn column(wrap_width: f32) -> LayoutConstraints {
    LayoutConstraints {
        wrap_width,
        ..Default::default()
    }
}

/// A compact rendering of the item sequence: `[ab]` a text box, `_` glue,
/// `|` a direct break, `T` a tab, `H` a hard break, `O` an object and `$`
/// the sentinel.
pub fn shape_of(ast: &ParagraphAst, text: &str) -> String {
    ast.items
        .iter()
        .map(|item| match item {
            WordNode::Box(BoxNode::Text { byte_range, .. }) => {
                format!("[{}]", &text[byte_range.as_range()])
            }
            WordNode::Box(BoxNode::InlineObject { .. }) => "O".into(),
            WordNode::Box(BoxNode::SpanBoundary { .. }) => "S".into(),
            WordNode::Glue(_) => "_".into(),
            WordNode::Tab { .. } => "T".into(),
            WordNode::Discretionary(d) if d.is_paragraph_end() => "$".into(),
            WordNode::Discretionary(d) if d.is_mandatory() => "H".into(),
            WordNode::Discretionary(_) => "|".into(),
        })
        .collect()
}

/// Compiles and lays out `text`, and passes the result through the output
/// gate.
#[track_caller]
pub fn lay_out(
    text: &str,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> PositionedParagraph {
    let ast = compile(text);
    let paragraph = layout_paragraph(&ast, text, &*SHAPER, constraints, options)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    if let Err((line, error)) = ValidatedLayout::new(&paragraph, text) {
        panic!("{text:?} in {constraints:?}, {options:?}: line {line}: {error}\n{paragraph:#?}");
    }
    paragraph
}

/// The text of each line, trailing whitespace included.
pub fn line_texts<'t>(paragraph: &PositionedParagraph, text: &'t str) -> Vec<&'t str> {
    paragraph
        .lines
        .iter()
        .map(|line| &text[line.byte_range.as_range()])
        .collect()
}

/// The vendored script fonts (`tests/assets/README.md`), in the order
/// [`SCRIPT_SHAPER`] registers them after the monospace face: face `i + 1`
/// is `SCRIPT_FONTS[i]`.
pub static SCRIPT_FONTS: &[(&str, &[u8])] = &[
    (
        "Noto Serif",
        include_bytes!("../assets/NotoSerif-Subset.ttf"),
    ),
    (
        "Noto Sans Devanagari",
        include_bytes!("../assets/NotoSansDevanagari-Subset.ttf"),
    ),
    (
        "Noto Sans JP",
        include_bytes!("../assets/NotoSansJP-Subset.ttf"),
    ),
    (
        "Noto Sans Thai",
        include_bytes!("../assets/NotoSansThai-Subset.ttf"),
    ),
    (
        "Noto Sans Lao",
        include_bytes!("../assets/NotoSansLao-Subset.ttf"),
    ),
    (
        "Noto Sans Khmer",
        include_bytes!("../assets/NotoSansKhmer-Subset.ttf"),
    ),
    (
        "Noto Sans Myanmar",
        include_bytes!("../assets/NotoSansMyanmar-Subset.ttf"),
    ),
];

/// A shaper over every vendored face: the monospace face as `FontId(0)`,
/// then [`SCRIPT_FONTS`]; Noto Serif is the generic serif family.
pub static SCRIPT_SHAPER: LazyLock<HarfRustShaper> = LazyLock::new(|| {
    let mut registry = FontRegistry::new();
    let faces = std::iter::once((FAMILY, FONT_BYTES)).chain(SCRIPT_FONTS.iter().copied());
    for (i, (family, bytes)) in faces.enumerate() {
        let id = registry
            .add_face(
                family.into(),
                FontStyle::default(),
                FontFace::new(FontBytes::from_static(bytes), 0),
            )
            .unwrap_or_else(|| panic!("{family} parses"));
        assert_eq!(id, FontId(i as u32));
    }
    registry.set_generic(GenericFamily::Serif, "Noto Serif".into());
    HarfRustShaper::new(Arc::new(registry))
});

/// `font_size` in the given family stack, shaped by [`SCRIPT_SHAPER`].
pub fn script_style(families: &[&str], font_size: f32) -> TextStyle {
    TextStyle {
        families: families
            .iter()
            .map(|f| FontFamily::Named((*f).into()))
            .collect(),
        ..style(font_size)
    }
}

/// Compiles `text` in `families` at 10 units per em with [`SCRIPT_SHAPER`].
pub fn compile_in(text: &str, families: &[&str]) -> ParagraphAst {
    let style = script_style(families, 10.0);
    compile_paragraph(
        &input(text, &style),
        &FrontendOptions::default(),
        &*SCRIPT_SHAPER,
    )
    .unwrap_or_else(|e| panic!("{text:?}: {e}"))
}

/// The glyph id that `font`'s cmap gives `c`, if any.
pub fn glyph_for(font: &[u8], c: char) -> Option<u32> {
    use read_fonts::TableProvider;
    let font = read_fonts::FontRef::new(font).expect("the test font parses");
    font.cmap().ok()?.map_codepoint(c).map(|g| g.to_u32())
}

/// Compiles and lays out `text` in `families` with [`SCRIPT_SHAPER`], and
/// passes the result through the output gate.
#[track_caller]
pub fn lay_out_in(
    text: &str,
    families: &[&str],
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> PositionedParagraph {
    let ast = compile_in(text, families);
    let paragraph = layout_paragraph(&ast, text, &*SCRIPT_SHAPER, constraints, options)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    if let Err((line, error)) = ValidatedLayout::new(&paragraph, text) {
        panic!("{text:?} in {constraints:?}: line {line}: {error}\n{paragraph:#?}");
    }
    paragraph
}

/// The vendored right-to-left faces (`tests/assets/README.md`), registered
/// by [`BIDI_SHAPER`] after the faces of [`SCRIPT_SHAPER`]: face `8 + i` is
/// `RTL_FONTS[i]`.
pub static RTL_FONTS: &[(&str, &[u8])] = &[
    (
        "Noto Sans Hebrew",
        include_bytes!("../assets/NotoSansHebrew-Subset.ttf"),
    ),
    (
        "Noto Naskh Arabic",
        include_bytes!("../assets/NotoNaskhArabic-Subset.ttf"),
    ),
];

pub const HEBREW: &str = "Noto Sans Hebrew";
pub const ARABIC: &str = "Noto Naskh Arabic";
pub const HEBREW_ID: FontId = FontId(8);
pub const ARABIC_ID: FontId = FontId(9);

/// A shaper over the faces of [`SCRIPT_SHAPER`] (ids 0 to 7) followed by
/// [`RTL_FONTS`] (ids 8 and 9). [`SCRIPT_SHAPER`] itself is unchanged, so
/// the last-resort fallback order of the Phase 1 tests stays as it was.
pub static BIDI_SHAPER: LazyLock<HarfRustShaper> = LazyLock::new(|| {
    let mut registry = FontRegistry::new();
    let faces = std::iter::once((FAMILY, FONT_BYTES))
        .chain(SCRIPT_FONTS.iter().copied())
        .chain(RTL_FONTS.iter().copied());
    for (i, (family, bytes)) in faces.enumerate() {
        let id = registry
            .add_face(
                family.into(),
                FontStyle::default(),
                FontFace::new(FontBytes::from_static(bytes), 0),
            )
            .unwrap_or_else(|| panic!("{family} parses"));
        assert_eq!(id, FontId(i as u32));
    }
    registry.set_generic(GenericFamily::Serif, "Noto Serif".into());
    HarfRustShaper::new(Arc::new(registry))
});

/// Compiles `input` with [`BIDI_SHAPER`] under `options`.
#[track_caller]
pub fn compile_bidi_input(input: &ParagraphInput<'_>, options: &FrontendOptions) -> ParagraphAst {
    compile_paragraph(input, options, &*BIDI_SHAPER)
        .unwrap_or_else(|e| panic!("{:?}: {e}", input.text))
}

/// Compiles `text` in `families` at 10 units per em with [`BIDI_SHAPER`],
/// the base direction detected (FE-8).
#[track_caller]
pub fn compile_bidi(text: &str, families: &[&str]) -> ParagraphAst {
    let style = script_style(families, 10.0);
    compile_bidi_input(&input(text, &style), &FrontendOptions::default())
}

/// Lays out `ast` with [`BIDI_SHAPER`] and passes the result through the
/// output gate.
#[track_caller]
pub fn lay_out_bidi_ast(
    ast: &ParagraphAst,
    text: &str,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> PositionedParagraph {
    let paragraph = layout_paragraph(ast, text, &*BIDI_SHAPER, constraints, options)
        .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    if let Err((line, error)) = ValidatedLayout::new(&paragraph, text) {
        panic!("{text:?} in {constraints:?}, {options:?}: line {line}: {error}\n{paragraph:#?}");
    }
    paragraph
}

/// Compiles and lays out `text` in `families` with [`BIDI_SHAPER`], and
/// passes the result through the output gate.
#[track_caller]
pub fn lay_out_bidi(
    text: &str,
    families: &[&str],
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> PositionedParagraph {
    lay_out_bidi_ast(&compile_bidi(text, families), text, constraints, options)
}
