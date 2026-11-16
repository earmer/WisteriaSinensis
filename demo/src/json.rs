//! Minimal JSON serialization for the browser harness boundary.

use std::fmt::Write as _;

use mutil_engine::{ConformanceReport, PositionedLine, ValidatedLayout};

/// Minimal JSON writer. Every value written here is an engine output number,
/// an id, or a string from an error, so only strings need escaping.
pub struct Out(String);

impl Out {
    pub fn new() -> Self {
        Self(String::with_capacity(4096))
    }

    pub fn raw(&mut self, text: &str) {
        self.0.push_str(text);
    }

    pub fn number(&mut self, value: f32) {
        if value.is_finite() {
            let _ = write!(self.0, "{value}");
        } else {
            self.0.push_str("null");
        }
    }

    pub fn uint(&mut self, value: usize) {
        let _ = write!(self.0, "{value}");
    }

    pub fn u32(&mut self, value: u32) {
        let _ = write!(self.0, "{value}");
    }

    pub fn string(&mut self, text: &str) {
        self.0.push('"');
        for c in text.chars() {
            match c {
                '"' => self.0.push_str("\\\""),
                '\\' => self.0.push_str("\\\\"),
                '\n' => self.0.push_str("\\n"),
                '\r' => self.0.push_str("\\r"),
                '\t' => self.0.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(self.0, "\\u{:04x}", c as u32);
                }
                c => self.0.push(c),
            }
        }
        self.0.push('"');
    }

    pub fn finish(self) -> String {
        self.0
    }
}

pub fn error_json(stage: &str, message: &str, line: Option<usize>) -> String {
    let mut out = Out::new();
    out.raw("{\"ok\":false,\"stage\":");
    out.string(stage);
    out.raw(",\"error\":");
    out.string(message);
    out.raw(",\"line\":");
    match line {
        Some(line) => out.uint(line),
        None => out.raw("null"),
    }
    out.raw("}");
    out.finish()
}

pub fn font_json(registered: Option<(u32, u16)>) -> String {
    match registered {
        Some((id, upem)) => {
            let mut out = Out::new();
            out.raw("{\"id\":");
            out.u32(id);
            out.raw(",\"upem\":");
            out.uint(upem as usize);
            out.raw("}");
            out.finish()
        }
        None => "{\"id\":null}".to_string(),
    }
}

pub fn conformance_json(unicode_version: &str, report: &ConformanceReport) -> String {
    let mut out = Out::new();
    out.raw("{\"unicode\":");
    out.string(unicode_version);
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

pub fn layout_json(layout: &ValidatedLayout<'_>, font_size: f32, width: f32) -> String {
    let paragraph = layout.paragraph();
    let runs: Vec<RunJson> = layout
        .glyph_runs()
        .map(|run| RunJson {
            line: run.line(),
            font: run.font().font_id.0,
            size: run.font_size() * run.font().scale,
            glyphs: run
                .glyphs()
                .map(|glyph| (glyph.glyph_id, glyph.origin.x, glyph.origin.y))
                .collect(),
        })
        .collect();

    let mut out = Out::new();
    out.raw("{\"ok\":true,\"blockSize\":");
    out.number(paragraph.total_block_size);
    out.raw(",\"fontSize\":");
    out.number(font_size);
    out.raw(",\"width\":");
    out.number(width);
    out.raw(",\"lineCount\":");
    out.uint(paragraph.lines.len());
    out.raw(",\"lineHeight\":");
    out.number(paragraph.lines.first().map_or(0.0, |line| line.line_height));
    out.raw(if paragraph.base_direction.is_rtl() {
        ",\"direction\":\"rtl\""
    } else {
        ",\"direction\":\"ltr\""
    });
    out.raw(",\"lines\":[");
    for (index, line) in paragraph.lines.iter().enumerate() {
        if index > 0 {
            out.raw(",");
        }
        write_line(&mut out, line, runs.iter().filter(|run| run.line == index));
    }
    out.raw("]}");
    out.finish()
}

fn write_line<'a>(out: &mut Out, line: &PositionedLine, runs: impl Iterator<Item = &'a RunJson>) {
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
    out.uint(line.visible_byte_end);
    out.raw(if line.is_justified {
        ",\"justified\":true"
    } else {
        ",\"justified\":false"
    });
    out.raw(",\"hang\":[");
    out.number(line.hanging_left());
    out.raw(",");
    out.number(line.hanging_right());
    out.raw("]");
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
    for (run_index, run) in runs.enumerate() {
        if run_index > 0 {
            out.raw(",");
        }
        write_run(out, run);
    }
    out.raw("]}");
}

fn write_run(out: &mut Out, run: &RunJson) {
    out.raw("{\"font\":");
    out.u32(run.font);
    out.raw(",\"size\":");
    out.number(run.size);
    out.raw(",\"glyphs\":[");
    for (glyph_index, (glyph, x, y)) in run.glyphs.iter().enumerate() {
        if glyph_index > 0 {
            out.raw(",");
        }
        out.raw("[");
        out.u32(*glyph);
        out.raw(",");
        out.number(*x);
        out.raw(",");
        out.number(*y);
        out.raw("]");
    }
    out.raw("]}");
}
