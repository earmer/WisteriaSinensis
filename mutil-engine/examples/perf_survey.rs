//! Comprehensive performance benchmark & allocation profiler for `mutil-engine`.
//!
//! Run in release mode to output structured JSON to stdout:
//!
//! ```bash
//! cd mutil-engine
//! cargo run --release --example perf_survey > target/perf_data.json
//! python3 ../scripts/plot_perf.py target/perf_data.json
//! ```

use mimalloc::MiMalloc;
use mutil_engine::*;
use stats_alloc::{Region, StatsAlloc};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

#[global_allocator]
static GLOBAL: StatsAlloc<MiMalloc> = StatsAlloc::new(MiMalloc);

struct FastLineShaper;

impl LineShaper for FastLineShaper {
    fn shape_line(
        &self,
        text: &str,
        _families: &[FontFamily],
        _style: FontStyle,
        font_size: f32,
        _features: &FontFeatures,
    ) -> Vec<LineGlyph> {
        let adv = font_size * 0.5;
        text.char_indices()
            .map(|(byte_index, _)| LineGlyph {
                font: FontId(0),
                glyph_id: 1,
                byte_index,
                x_advance: adv,
                x_offset: 0.0,
                y_offset: 0.0,
            })
            .collect()
    }

    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics {
        let s = instance.font_size;
        FontMetrics {
            ascent: s * 0.8,
            descent: s * 0.2,
            line_gap: s * 0.1,
            vertical_ascent: s * 0.5,
            vertical_descent: s * 0.5,
            ..Default::default()
        }
    }

    fn resolve_primary_font_metrics(
        &self,
        _families: &[FontFamily],
        _style: FontStyle,
        _script: ScriptTag,
        _locale: &LocaleTag,
        font_size: f32,
        _variations: &[FontVariation],
    ) -> (FontId, FontMetrics) {
        (
            FontId(0),
            self.font_metrics(FontInstanceRef {
                font: FontRef::unscaled(FontId(0)),
                font_size,
                variations: &[],
            }),
        )
    }

    fn space_advance(&self, instance: FontInstanceRef<'_>, _orientation: GlyphOrientation) -> f32 {
        instance.font_size * 0.25
    }
}

fn load_shaper() -> HarfRustShaper {
    let mut registry = FontRegistry::new();
    let fonts: &[(&str, &str)] = &[
        ("Noto Sans", "../demo/fonts/NotoSans[wdth,wght].ttf"),
        ("Noto Sans SC", "../demo/fonts/NotoSansSC[wght].ttf"),
        (
            "Noto Naskh Arabic",
            "../demo/fonts/NotoNaskhArabic[wght].ttf",
        ),
        (
            "Noto Sans Devanagari",
            "../demo/fonts/NotoSansDevanagari[wdth,wght].ttf",
        ),
    ];
    for (family, path) in fonts {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        registry
            .add_face(
                (*family).into(),
                FontStyle::default(),
                FontFace::new(FontBytes::new(bytes), 0),
            )
            .unwrap_or_else(|| panic!("parse {path}"));
    }
    registry.set_generic(GenericFamily::SansSerif, "Noto Sans".into());
    HarfRustShaper::new(Arc::new(registry))
}

fn default_style() -> TextStyle {
    TextStyle {
        families: Arc::from([
            FontFamily::Named("Noto Sans".into()),
            FontFamily::Named("Noto Sans SC".into()),
            FontFamily::Named("Noto Naskh Arabic".into()),
            FontFamily::Named("Noto Sans Devanagari".into()),
        ]),
        font_size: 16.0,
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

fn make_latin(target_chars: usize) -> String {
    let base = "Typography is the art and technique of arranging type to make written language legible, readable, and appealing when displayed. The arrangement of type involves selecting typefaces, point sizes, line lengths, line-spacing, and letter-spacing. ";
    let mut s = String::with_capacity(target_chars + base.len());
    while s.chars().count() < target_chars {
        s.push_str(base);
    }
    s.chars().take(target_chars).collect()
}

fn make_cjk_mixed(target_chars: usize) -> String {
    let base = "排版引擎需要处理中文与English混合文本的自动间距（CJK autospace），以及标点挤压、禁则处理和Unicode双向算法。Modern layout engines handle Rust、WebAssembly和OpenType字形塑形。";
    let mut s = String::with_capacity(target_chars * 3);
    while s.chars().count() < target_chars {
        s.push_str(base);
    }
    s.chars().take(target_chars).collect()
}

fn make_arabic_bidi(target_chars: usize) -> String {
    let base = "التخطيط النصي متعدد اللغات يدعم اللغة العربية مع English words و 12345 أرقام في نفس الفقرة بدقة عالية. ";
    let mut s = String::with_capacity(target_chars * 2);
    while s.chars().count() < target_chars {
        s.push_str(base);
    }
    s.chars().take(target_chars).collect()
}

fn make_devanagari(target_chars: usize) -> String {
    let base = "देवनागरी लिपि में संयुक्ताक्षर और मात्राओं का सही आकार निर्धारण अत्यंत महत्वपूर्ण है ताकि पाठ स्पष्ट रूप से पढ़ा जा सके। ";
    let mut s = String::with_capacity(target_chars * 3);
    while s.chars().count() < target_chars {
        s.push_str(base);
    }
    s.chars().take(target_chars).collect()
}

#[derive(Clone, Copy)]
struct Meas {
    median_us: f64,
    allocs: usize,
    bytes: usize,
}

fn measure<T>(iters: usize, mut f: impl FnMut() -> T) -> Meas {
    black_box(f());
    let reg = Region::new(&GLOBAL);
    black_box(f());
    let stats = reg.change();

    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        black_box(f());
        times.push(start.elapsed());
    }
    times.sort();
    let median = times[iters / 2];
    Meas {
        median_us: median.as_secs_f64() * 1e6,
        allocs: stats.allocations,
        bytes: stats.bytes_allocated,
    }
}

/// One benchmarked paragraph: its text and spans, the shaper, and how it is
/// laid out.
struct Workload<'a> {
    name: &'a str,
    text: &'a str,
    style: &'a TextStyle,
    style_spans: &'a [StyleSpan],
    dec_spans: &'a [InlineDecorationSpan],
    shaper: &'a dyn TextShaper,
    wrap_width: f32,
    justify: bool,
    iters: usize,
}

impl<'a> Workload<'a> {
    /// A plain paragraph of `text`: no spans, not justified.
    fn plain(
        name: &'a str,
        text: &'a str,
        style: &'a TextStyle,
        shaper: &'a dyn TextShaper,
        wrap_width: f32,
        iters: usize,
    ) -> Self {
        Self {
            name,
            text,
            style,
            style_spans: &[],
            dec_spans: &[],
            shaper,
            wrap_width,
            justify: false,
            iters,
        }
    }
}

fn bench_workload(workload: &Workload<'_>) -> String {
    let &Workload {
        name,
        text,
        style,
        style_spans,
        dec_spans,
        shaper,
        wrap_width,
        justify,
        iters,
    } = workload;
    let input = ParagraphInput {
        text,
        default_style: style,
        style_spans,
        inline_objects: &[],
        decoration_spans: dec_spans,
        atomic_spans: &[],
        bidi_spans: &[],
        annotations: &[],
    };
    let fe_opts = FrontendOptions::default();
    let constraints = LayoutConstraints {
        wrap_width,
        ..Default::default()
    };
    let layout_opts = LayoutOptions {
        justify,
        ..Default::default()
    };

    let m_compile = measure(iters, || {
        compile_paragraph(black_box(&input), black_box(&fe_opts), shaper).unwrap()
    });
    let ast = compile_paragraph(&input, &fe_opts, shaper).unwrap();

    let m_val_in = measure(iters, || {
        ValidatedInputs::try_from_ast(
            black_box(&ast),
            black_box(text),
            black_box(&constraints),
            black_box(&layout_opts),
        )
        .unwrap_or_else(|e| panic!("val_in failed on {name}: {e:?}"));
    });

    let m_layout_total = measure(iters, || {
        layout_paragraph(
            black_box(&ast),
            black_box(text),
            shaper,
            black_box(&constraints),
            black_box(&layout_opts),
        )
        .unwrap()
    });
    let para = layout_paragraph(&ast, text, shaper, &constraints, &layout_opts).unwrap();

    let m_val_out = measure(iters, || {
        ValidatedLayout::try_from_paragraph(black_box(&para), black_box(text)).unwrap();
    });
    let validated = ValidatedLayout::try_from_paragraph(&para, text).unwrap();

    let mid_byte = text
        .char_indices()
        .nth(text.chars().count() / 2)
        .map_or(0, |(i, _)| i);
    let mid_pos = CursorPosition {
        byte_offset: mid_byte,
        affinity: CaretAffinity::Downstream,
    };
    let sel_range = TextRange::new(0, text.len());
    let hit_pt = Point {
        x: wrap_width * 0.5,
        y: para.total_block_size * 0.5,
    };

    let m_hit = measure(iters, || {
        for _ in 0..100 {
            black_box(validated.hit_test(black_box(hit_pt)));
        }
    });
    let m_caret = measure(iters, || {
        for _ in 0..100 {
            black_box(validated.caret_geometry(black_box(mid_pos)));
        }
    });
    let m_sel = measure(iters, || {
        black_box(validated.selection_geometry(shaper, black_box(sel_range), BoxHeightStyle::Tight))
    });

    format!(
        r#"  {{"name": "{name}", "chars": {}, "bytes": {}, "lines": {}, "runs": {}, "clusters": {}, "glyphs": {}, "compile_us": {:.2}, "compile_allocs": {}, "compile_bytes": {}, "val_in_us": {:.2}, "val_in_allocs": {}, "layout_total_us": {:.2}, "layout_allocs": {}, "layout_bytes": {}, "val_out_us": {:.2}, "val_out_allocs": {}, "hit_test_ns": {:.1}, "caret_geom_ns": {:.1}, "sel_geom_us": {:.2}, "sel_allocs": {}}}"#,
        text.chars().count(),
        text.len(),
        para.lines.len(),
        ast.runs.len(),
        ast.clusters.len(),
        ast.glyphs.len(),
        m_compile.median_us,
        m_compile.allocs,
        m_compile.bytes,
        m_val_in.median_us,
        m_val_in.allocs,
        m_layout_total.median_us,
        m_layout_total.allocs,
        m_layout_total.bytes,
        m_val_out.median_us,
        m_val_out.allocs,
        m_hit.median_us * 10.0,
        m_caret.median_us * 10.0,
        m_sel.median_us,
        m_sel.allocs,
    )
}

fn main() {
    let shaper = load_shaper();
    let stub = LineOnlyShaper::new(FastLineShaper);
    let style = default_style();
    let mut workloads_json = Vec::new();

    let latin = make_latin(2000);
    workloads_json.push(bench_workload(&Workload::plain(
        "latin_plain_harfrust",
        &latin,
        &style,
        &shaper,
        480.0,
        15,
    )));
    workloads_json.push(bench_workload(&Workload::plain(
        "latin_plain_stub",
        &latin,
        &style,
        &stub,
        480.0,
        15,
    )));
    workloads_json.push(bench_workload(&Workload {
        justify: true,
        ..Workload::plain("latin_justified", &latin, &style, &shaper, 480.0, 15)
    }));

    let cjk = make_cjk_mixed(2000);
    workloads_json.push(bench_workload(&Workload::plain(
        "cjk_mixed_harfrust",
        &cjk,
        &style,
        &shaper,
        480.0,
        15,
    )));
    workloads_json.push(bench_workload(&Workload::plain(
        "cjk_mixed_stub",
        &cjk,
        &style,
        &stub,
        480.0,
        15,
    )));

    let arabic = make_arabic_bidi(2000);
    workloads_json.push(bench_workload(&Workload::plain(
        "arabic_bidi_harfrust",
        &arabic,
        &style,
        &shaper,
        480.0,
        15,
    )));

    let deva = make_devanagari(2000);
    workloads_json.push(bench_workload(&Workload::plain(
        "devanagari_harfrust",
        &deva,
        &style,
        &shaper,
        480.0,
        15,
    )));

    let mut style_spans = Vec::new();
    let mut dec_spans = Vec::new();
    let mut alt_style = style.clone();
    alt_style.font_size = 18.0;
    let words: Vec<(usize, usize)> = {
        let mut out = Vec::new();
        let mut start = None;
        for (i, b) in latin.bytes().enumerate() {
            if b == b' ' {
                if let Some(s) = start.take() {
                    out.push((s, i));
                }
            } else if start.is_none() {
                start = Some(i);
            }
        }
        out
    };
    for (i, &(s, e)) in words.iter().step_by(4).take(50).enumerate() {
        style_spans.push(StyleSpan {
            byte_range: TextRange::new(s, e),
            style: alt_style.clone(),
        });
        dec_spans.push(InlineDecorationSpan {
            span_id: (i + 1) as u64,
            byte_range: TextRange::new(s, e),
            start_padding: 2.0,
            end_padding: 2.0,
            break_mode: BoxDecorationBreak::Slice,
            height_style: mutil_engine::BoxHeightStyle::Strut,
        });
    }
    workloads_json.push(bench_workload(&Workload {
        style_spans: &style_spans,
        dec_spans: &dec_spans,
        ..Workload::plain("rich_spans_harfrust", &latin, &style, &shaper, 480.0, 15)
    }));
    workloads_json.push(bench_workload(&Workload::plain(
        "narrow_emergency_harfrust",
        &latin,
        &style,
        &shaper,
        48.0,
        15,
    )));

    let mut scaling_json = Vec::new();
    for &n in &[100usize, 300, 1000, 3000, 10000] {
        let text = make_latin(n);
        let iters = if n >= 10000 { 5 } else { 11 };
        scaling_json.push(bench_workload(&Workload::plain(
            &format!("scale_latin_{n}"),
            &text,
            &style,
            &shaper,
            480.0,
            iters,
        )));
        let cjk_text = make_cjk_mixed(n);
        scaling_json.push(bench_workload(&Workload::plain(
            &format!("scale_cjk_{n}"),
            &cjk_text,
            &style,
            &shaper,
            480.0,
            iters,
        )));
    }

    let mut wide_json = Vec::new();
    for &n in &[100usize, 300, 1000, 3000] {
        let text = make_latin(n);
        wide_json.push(bench_workload(&Workload::plain(
            &format!("wide_single_line_{n}"),
            &text,
            &style,
            &stub,
            1e9,
            9,
        )));
    }

    println!("{{");
    println!("  \"workloads\": [");
    println!("{}", workloads_json.join(",\n"));
    println!("  ],");
    println!("  \"scaling\": [");
    println!("{}", scaling_json.join(",\n"));
    println!("  ],");
    println!("  \"wide_line\": [");
    println!("{}", wide_json.join(",\n"));
    println!("  ]");
    println!("}}");
}
