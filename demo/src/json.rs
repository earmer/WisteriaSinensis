//! The JSON the page reads. Every value is an engine number, an id or a short
//! string, so a few `Display` helpers stand in for a serializer.

use std::fmt::{self, Display, Write as _};

use crate::markup::Marked;
use mutil_engine::{
    CaretAffinity, CaretVisual, CursorMove, CursorPosition, DecorationKind, HitResult,
    ParagraphEdge, PositionedLine, Rect, SpanEdgeCut, ValidatedLayout,
};

/// A number, or `null` when it is not finite.
struct Num(f32);

impl Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // An infinity is legitimate: an unbounded slot's end is
        // `f32::INFINITY`, and the page reads its `null` as "no edge". NaN
        // never is, because the gates reject it (AX-3).
        debug_assert!(!self.0.is_nan(), "NaN in demo JSON");
        if self.0.is_finite() {
            write!(f, "{}", self.0)
        } else {
            f.write_str("null")
        }
    }
}

/// A quoted, escaped string.
struct Str<'a>(&'a str);

impl Display for Str<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('"')?;
        for c in self.0.chars() {
            match c {
                '"' | '\\' => write!(f, "\\{c}")?,
                c if c < ' ' => write!(f, "\\u{:04x}", u32::from(c))?,
                c => f.write_char(c)?,
            }
        }
        f.write_char('"')
    }
}

fn opt(value: Option<impl Display>) -> String {
    value.map_or_else(|| "null".to_owned(), |value| value.to_string())
}

fn list<T>(items: impl IntoIterator<Item = T>, item: impl FnMut(T) -> String) -> String {
    let items: Vec<String> = items.into_iter().map(item).collect();
    format!("[{}]", items.join(","))
}

fn interval(start: f32, end: f32) -> String {
    format!("[{},{}]", Num(start), Num(end))
}

fn rect(rect: Rect) -> String {
    let [x, y, w, h] = [rect.x, rect.y, rect.width, rect.height].map(Num);
    format!("{{\"x\":{x},\"y\":{y},\"w\":{w},\"h\":{h}}}")
}

pub fn error_json(stage: &str, message: impl Display) -> String {
    let message = message.to_string();
    let (stage, message) = (Str(stage), Str(&message));
    format!("{{\"ok\":false,\"stage\":{stage},\"error\":{message}}}")
}

pub fn font_json(registered: Option<(u32, u16)>) -> String {
    let face = registered.map(|(id, upem)| format!("{id},\"upem\":{upem}"));
    format!("{{\"id\":{}}}", opt(face))
}

/// The components the engine itself reports as installed (spec SCO-8); a
/// `null` version is one not implemented.
pub fn conformance_json() -> String {
    let report = mutil_engine::conformance_report();
    let unicode_version = mutil_engine::UNICODE_DATA_VERSION;
    let mut out = format!("{{\"unicode\":{}", Str(unicode_version));
    for (name, version) in [
        ("grapheme", report.grapheme),
        ("bidi", report.bidi),
        ("lineBreak", report.line_break),
        ("word", report.word),
        ("script", report.script),
        ("emoji", report.emoji),
    ] {
        let _ = write!(out, ",\"{name}\":{}", opt(version.map(Str)));
    }
    out + "}"
}

fn edge_cut_str(cut: SpanEdgeCut) -> &'static str {
    match cut {
        SpanEdgeCut::LogicalEnd => "logicalEnd",
        SpanEdgeCut::ClonedWrap => "clonedWrap",
        SpanEdgeCut::SlicedWrap => "slicedWrap",
        SpanEdgeCut::BidiSplit => "bidiSplit",
    }
}

/// The engine's own verdict on the paragraph: every line, its slots, carets,
/// span fragments, decorations, and every placed glyph origin with byte-range
/// paint attribution (spec HST-3/HST-4, FE-17).
pub fn layout_json(layout: &ValidatedLayout<'_>, id: usize, marked: &Marked) -> String {
    let paragraph = layout.paragraph();
    let runs: Vec<(usize, String)> = layout
        .glyph_runs()
        .map(|run| {
            let glyphs = list(run.glyphs(), |glyph| {
                let origin = glyph.origin;
                let (b_start, b_end) = match glyph.byte_range {
                    Some(r) => (r.start().to_string(), r.end().to_string()),
                    None => ("null".to_owned(), "null".to_owned()),
                };
                format!(
                    "[{},{},{},{b_start},{b_end},{}]",
                    glyph.glyph_id,
                    Num(origin.x),
                    Num(origin.y),
                    Num(glyph.advance),
                )
            });
            let size = Num(run.font_size() * run.font().scale);
            let x_scale = Num(run.x_scale());
            let font = run.font().font_id.0;
            let json = format!(
                "{{\"font\":{font},\"size\":{size},\"xScale\":{x_scale},\"glyphs\":{glyphs}}}"
            );
            (run.line(), json)
        })
        .collect();
    let lines = list(paragraph.lines.iter().enumerate(), |(index, line)| {
        let line_runs = runs.iter().filter(|(run_line, _)| *run_line == index);
        line_json(line, list(line_runs, |(_, json)| json.clone()))
    });
    let paint_spans = list(&marked.paint_spans, |span| {
        format!(
            "{{\"start\":{},\"end\":{},\"color\":{}}}",
            span.byte_range.start(),
            span.byte_range.end(),
            Str(&span.color),
        )
    });
    let span_styles = list(&marked.box_spans, |b| {
        format!(
            "{{\"spanId\":{},\"kind\":{}}}",
            b.span.span_id,
            Str(b.kind.as_str()),
        )
    });
    let direction = if paragraph.base_direction.is_rtl() {
        "rtl"
    } else {
        "ltr"
    };
    format!(
        "{{\"ok\":true,\"id\":{id},\"blockSize\":{},\"direction\":\"{direction}\",\
         \"paintSpans\":{paint_spans},\"spanStyles\":{span_styles},\"lines\":{lines}}}",
        Num(paragraph.total_block_size)
    )
}

/// One line: its box, the content and available interval of each slot (a
/// hanging mark protrudes from the latter, spec HNG-1), the pen interval of
/// each hanging glyph, its caret stops, span fragments, decorations and `runs`.
fn line_json(line: &PositionedLine, runs: String) -> String {
    let slots = list(&line.slots, |slot| {
        interval(slot.content_start, slot.content_end())
    });
    let columns = list(&line.slots, |slot| {
        interval(slot.constraint.start(), slot.constraint.end())
    });
    let hangs = list(line.hanging_glyphs(), |glyph| {
        interval(glyph.x, glyph.x + glyph.advance)
    });
    let carets = list(&line.caret_stops, |stop| Num(stop.x).to_string());
    let span_fragments = list(&line.span_fragments, |f| {
        format!(
            "{{\"spanId\":{},\"x\":{},\"width\":{},\"ascent\":{},\"descent\":{},\
             \"rtl\":{},\"edges\":[{},{}],\"ribbonOffset\":{},\"totalSpanAdvance\":{},\
             \"byteRange\":[{},{}]}}",
            f.span_id,
            Num(f.x),
            Num(f.width),
            Num(f.ascent),
            Num(f.descent),
            f.bidi_level.direction().is_rtl(),
            Str(edge_cut_str(f.edges.left)),
            Str(edge_cut_str(f.edges.right)),
            Num(f.ribbon_offset),
            Num(f.total_span_advance),
            f.byte_range.start(),
            f.byte_range.end(),
        )
    });
    let decorations = list(&line.decorations, |d| {
        let kind = match d.kind {
            DecorationKind::Underline => "underline",
            DecorationKind::Strikethrough => "strikethrough",
        };
        format!(
            "{{\"kind\":\"{kind}\",\"x\":{},\"width\":{},\"yOffset\":{},\"thickness\":{},\
             \"byteRange\":[{},{}]}}",
            Num(d.x),
            Num(d.width),
            Num(d.y_offset),
            Num(d.thickness),
            d.byte_range.start(),
            d.byte_range.end(),
        )
    });
    format!(
        "{{\"top\":{},\"baseline\":{},\"height\":{},\"justified\":{},\"hang\":{},\
         \"slots\":{slots},\"columns\":{columns},\"hangs\":{hangs},\"carets\":{carets},\
         \"spanFragments\":{span_fragments},\"decorations\":{decorations},\"runs\":{runs}}}",
        Num(line.baseline_y - line.ascent),
        Num(line.baseline_y),
        Num(line.line_height),
        line.is_justified,
        interval(line.hanging_left(), line.hanging_right()),
    )
}

fn position_json(prefix: &str, pos: CursorPosition) -> String {
    let affinity = match pos.affinity {
        CaretAffinity::Downstream => "downstream",
        CaretAffinity::Upstream => "upstream",
    };
    format!(
        "{{{prefix}\"byteOffset\":{},\"affinity\":\"{affinity}\"}}",
        pos.byte_offset
    )
}

pub fn hit_json(hit: HitResult) -> String {
    position_json("", hit.position)
}

pub fn caret_json(caret: CaretVisual) -> String {
    format!(
        "{{\"primary\":{},\"secondary\":{}}}",
        rect(caret.primary_rect),
        opt(caret.secondary_rect.map(rect))
    )
}

pub fn cursor_move_json(moved: CursorMove) -> String {
    match moved {
        CursorMove::Moved(pos) => position_json("\"kind\":\"moved\",", pos),
        CursorMove::Exited {
            edge,
            inline_position,
        } => {
            let edge = match edge {
                ParagraphEdge::Start => "start",
                ParagraphEdge::End => "end",
            };
            let inline_position = opt(inline_position.map(Num));
            format!(
                "{{\"kind\":\"exited\",\"edge\":\"{edge}\",\"inlinePosition\":{inline_position}}}"
            )
        }
    }
}

pub fn rects_json(rects: impl IntoIterator<Item = Rect>) -> String {
    list(rects, rect)
}
