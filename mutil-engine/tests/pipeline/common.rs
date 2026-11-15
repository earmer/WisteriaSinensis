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
