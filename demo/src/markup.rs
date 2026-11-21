//! Inline locale markup for the harness's text box.
//!
//! The engine takes plain UTF-8 paragraphs (spec AX-1) and never guesses
//! between Simplified and Traditional Han; a rich-text host states inline
//! language switches as `StyleSpan`s. This module is that host step for the
//! page: it strips tags such as `<zh-TW>…</zh-TW>`, `<ja>…</ja>` or
//! `<lang=zh-Hant>…</lang>` and returns the plain text with the byte ranges
//! each tag covered and its locale, read with `LocaleTag::try_from_str_lenient`.
//!
//! A tag is markup only when it parses as a locale and is closed by its own
//! closing tag (`</lang>` for the `lang=` form), which pairs with the nearest
//! open opener of its name; a `lang=` value is unquoted or inside one
//! matching pair of quotes. Anything else, an unclosed opener or a stray
//! closer included, stays in the text as typed. Nested tags
//! are flattened: each byte takes its innermost tag, so the spans the engine
//! receives are disjoint and ascending (spec FE-25).

use mutil_engine::{
    BoxDecorationBreak, BoxHeightStyle, InlineDecorationSpan, LocaleTag,
    MAX_DECORATION_NESTING_DEPTH, TextDecoration, TextRange,
};

/// Host-side foreground colour attribution over source bytes (never splits
/// shaping runs, spec FE-17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaintSpan {
    pub byte_range: TextRange,
    pub color: String,
}

/// Visual preset for an inline decoration box in the demo canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxStyleKind {
    Mark,
    Pill,
    Ribbon,
}

impl BoxStyleKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mark => "mark",
            Self::Pill => "pill",
            Self::Ribbon => "ribbon",
        }
    }

    fn to_span(self, span_id: u64, byte_range: TextRange) -> InlineDecorationSpan {
        match self {
            Self::Mark => InlineDecorationSpan {
                span_id,
                byte_range,
                start_padding: 3.0,
                end_padding: 3.0,
                break_mode: BoxDecorationBreak::Slice,
                height_style: BoxHeightStyle::Strut,
            },
            Self::Pill => InlineDecorationSpan {
                span_id,
                byte_range,
                start_padding: 6.0,
                end_padding: 6.0,
                break_mode: BoxDecorationBreak::Clone,
                height_style: BoxHeightStyle::Strut,
            },
            Self::Ribbon => InlineDecorationSpan {
                span_id,
                byte_range,
                start_padding: 4.0,
                end_padding: 4.0,
                break_mode: BoxDecorationBreak::Slice,
                height_style: BoxHeightStyle::Tight,
            },
        }
    }
}

/// An engine [`InlineDecorationSpan`] paired with its host-side canvas style.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxSpan {
    pub span: InlineDecorationSpan,
    pub kind: BoxStyleKind,
}

/// Plain text and the locale, decoration, paint, and box spans of each tagged
/// stretch of it.
#[derive(Debug, PartialEq)]
pub struct Marked {
    /// The text with the markup removed.
    pub text: String,
    /// Disjoint, ascending, non-empty ranges of `text` with their locale.
    pub spans: Vec<(TextRange, LocaleTag)>,
    /// Disjoint, ascending, non-empty ranges of `text` with underline /
    /// strikethrough decorations.
    pub decorations: Vec<(TextRange, TextDecoration)>,
    /// Host-side foreground colour spans (never split shaping runs, FE-17).
    pub paint_spans: Vec<PaintSpan>,
    /// Engine inline decoration spans, ordered for G1 (`AST-8`).
    pub box_spans: Vec<BoxSpan>,
    /// The removed tags, as ascending byte ranges of the source.
    pub removed: Vec<core::ops::Range<usize>>,
}

impl Marked {
    /// The offset in `text` of byte `source_offset` of the source.
    #[cfg(test)]
    fn text_offset(&self, source_offset: usize) -> usize {
        text_offset(&self.removed, source_offset)
    }
}

/// The offset in the marked text of byte `source_offset` of a source whose
/// `removed` tags are ascending: an offset inside a tag maps to where the
/// tag was.
#[must_use]
pub fn text_offset(removed: &[core::ops::Range<usize>], source_offset: usize) -> usize {
    let mut gone = 0;
    for tag in removed {
        if tag.end <= source_offset {
            gone += tag.len();
        } else {
            return source_offset.min(tag.start) - gone;
        }
    }
    source_offset - gone
}

enum TagKind {
    Locale(LocaleTag),
    Underline,
    Strikethrough,
    Color(String),
    Box(BoxStyleKind),
}

enum Token {
    Text(core::ops::Range<usize>),
    Open {
        close: String,
        kind: TagKind,
        raw: core::ops::Range<usize>,
    },
    Close {
        key: String,
        raw: core::ops::Range<usize>,
    },
}

fn unquote(value: &str) -> &str {
    match value.as_bytes() {
        [q @ (b'"' | b'\''), .., last] if last == q && value.len() >= 2 => {
            &value[1..value.len() - 1]
        }
        _ => value,
    }
}

/// Reads `<…>` at `at`, if it is a tag of this markup.
fn tag_at(source: &str, at: usize) -> Option<(Token, usize)> {
    let rest = &source[at..];
    // A tag holds no `<`: the scan stops at the next one, so each byte is
    // read once per `<` before it and tokenizing stays linear.
    let end = 1 + rest[1..].find(['<', '>'])?;
    if rest.as_bytes()[end] != b'>' {
        return None;
    }
    let inner = &rest[1..end];
    let raw = at..at + end + 1;
    let name_ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    if let Some(key) = inner.strip_prefix('/') {
        return name_ok(key).then(|| {
            (
                Token::Close {
                    key: key.to_ascii_lowercase(),
                    raw: raw.clone(),
                },
                raw.end,
            )
        });
    }
    let (close, kind) = if let Some((attr, value)) = inner.split_once('=') {
        let value = unquote(value);
        match attr.to_ascii_lowercase().as_str() {
            "lang" => {
                if !name_ok(value) {
                    return None;
                }
                let locale = LocaleTag::try_from_str_lenient(value).ok()?;
                ("lang".to_owned(), TagKind::Locale(locale))
            }
            "color" => {
                let color_ok = !value.is_empty()
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'#' || b == b'-' || b == b'_');
                if !color_ok {
                    return None;
                }
                ("color".to_owned(), TagKind::Color(value.to_owned()))
            }
            _ => return None,
        }
    } else {
        let key = inner.to_ascii_lowercase();
        match key.as_str() {
            "u" => (key, TagKind::Underline),
            "s" => (key, TagKind::Strikethrough),
            "mark" => (key, TagKind::Box(BoxStyleKind::Mark)),
            "pill" => (key, TagKind::Box(BoxStyleKind::Pill)),
            "ribbon" => (key, TagKind::Box(BoxStyleKind::Ribbon)),
            _ => {
                if !name_ok(inner) {
                    return None;
                }
                let locale = LocaleTag::try_from_str_lenient(inner).ok()?;
                (key, TagKind::Locale(locale))
            }
        }
    };
    Some((
        Token::Open {
            close,
            kind,
            raw: raw.clone(),
        },
        raw.end,
    ))
}

fn tokens(source: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    while let Some(offset) = source[i..].find('<') {
        let at = i + offset;
        match tag_at(source, at) {
            Some((token, next)) => {
                if text_start < at {
                    out.push(Token::Text(text_start..at));
                }
                out.push(token);
                text_start = next;
                i = next;
            }
            None => i = at + 1,
        }
    }
    if text_start < source.len() {
        out.push(Token::Text(text_start..source.len()));
    }
    out
}

/// Pairs each closer with the nearest open opener of its key; openers left
/// above it on the stack stay unpaired. Each opener is pushed and removed at
/// most once, so the pass is linear in the number of tokens.
fn pair(tokens: &[Token]) -> Vec<bool> {
    use std::collections::HashMap;
    let mut paired = vec![false; tokens.len()];
    // Open openers in order, and for each key the indices into `stack` of
    // its openers, innermost last.
    let mut stack: Vec<(usize, &str)> = Vec::new();
    let mut by_key: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Open { close, .. } => {
                by_key.entry(close).or_default().push(stack.len());
                stack.push((index, close));
            }
            Token::Close { key, .. } => {
                let Some(&at) = by_key.get(key.as_str()).and_then(|v| v.last()) else {
                    continue;
                };
                // Openers above `at` were never closed: they stay literal.
                while stack.len() > at + 1 {
                    let (_, dropped) = stack.pop().expect("above at");
                    by_key.get_mut(dropped).expect("tracked").pop();
                }
                let (open, _) = stack.pop().expect("at is open");
                by_key.get_mut(key.as_str()).expect("tracked").pop();
                paired[open] = true;
                paired[index] = true;
            }
            Token::Text(_) => {}
        }
    }
    paired
}

/// Strips the locale and rich-text markup from `source` (see the module
/// documentation).
#[must_use]
pub fn parse(source: &str) -> Marked {
    let tokens = tokens(source);
    let paired = pair(&tokens);
    let mut text = String::with_capacity(source.len());
    let mut spans: Vec<(TextRange, LocaleTag)> = Vec::new();
    let mut decorations: Vec<(TextRange, TextDecoration)> = Vec::new();
    let mut paint_spans: Vec<PaintSpan> = Vec::new();
    let mut raw_boxes: Vec<(TextRange, BoxStyleKind)> = Vec::new();
    let mut removed = Vec::new();
    // Stacks of currently open paired tags.
    let mut open_kinds: Vec<&TagKind> = Vec::new();
    let mut locales: Vec<&LocaleTag> = Vec::new();
    let mut colors: Vec<&str> = Vec::new();
    let mut box_starts: Vec<(usize, BoxStyleKind)> = Vec::new();
    let mut underline_depth: usize = 0;
    let mut strike_depth: usize = 0;
    let mut since = 0;

    let mut flush = |locales: &[&LocaleTag],
                     colors: &[&str],
                     u_depth: usize,
                     s_depth: usize,
                     since: usize,
                     end: usize| {
        if since == end {
            return;
        }
        let range = TextRange::new(since, end);
        if let Some(&locale) = locales.last() {
            match spans.last_mut() {
                Some((prev, last)) if prev.end() == since && last == locale => {
                    *prev = TextRange::new(prev.start(), end);
                }
                _ => spans.push((range, locale.clone())),
            }
        }
        if u_depth > 0 || s_depth > 0 {
            let deco = TextDecoration {
                underline: u_depth > 0,
                strikethrough: s_depth > 0,
            };
            match decorations.last_mut() {
                Some((prev, last)) if prev.end() == since && *last == deco => {
                    *prev = TextRange::new(prev.start(), end);
                }
                _ => decorations.push((range, deco)),
            }
        }
        if let Some(&color) = colors.last() {
            match paint_spans.last_mut() {
                Some(prev) if prev.byte_range.end() == since && prev.color == color => {
                    prev.byte_range = TextRange::new(prev.byte_range.start(), end);
                }
                _ => paint_spans.push(PaintSpan {
                    byte_range: range,
                    color: color.to_owned(),
                }),
            }
        }
    };

    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Text(range) => text.push_str(&source[range.clone()]),
            Token::Open { raw, .. } | Token::Close { raw, .. } if !paired[index] => {
                text.push_str(&source[raw.clone()]);
            }
            Token::Open { kind, raw, .. } => {
                removed.push(raw.clone());
                flush(
                    &locales,
                    &colors,
                    underline_depth,
                    strike_depth,
                    since,
                    text.len(),
                );
                open_kinds.push(kind);
                match kind {
                    TagKind::Locale(locale) => locales.push(locale),
                    TagKind::Underline => underline_depth += 1,
                    TagKind::Strikethrough => strike_depth += 1,
                    TagKind::Color(color) => colors.push(color),
                    TagKind::Box(style) => box_starts.push((text.len(), *style)),
                }
                since = text.len();
            }
            Token::Close { raw, .. } => {
                removed.push(raw.clone());
                flush(
                    &locales,
                    &colors,
                    underline_depth,
                    strike_depth,
                    since,
                    text.len(),
                );
                if let Some(kind) = open_kinds.pop() {
                    match kind {
                        TagKind::Locale(_) => {
                            locales.pop();
                        }
                        TagKind::Underline => underline_depth -= 1,
                        TagKind::Strikethrough => strike_depth -= 1,
                        TagKind::Color(_) => {
                            colors.pop();
                        }
                        TagKind::Box(_) => {
                            if let Some((start, style)) = box_starts.pop()
                                && start < text.len()
                                && box_starts.len() < MAX_DECORATION_NESTING_DEPTH
                            {
                                raw_boxes.push((TextRange::new(start, text.len()), style));
                            }
                        }
                    }
                }
                since = text.len();
            }
        }
    }
    raw_boxes.sort_by_key(|(r, _)| (r.start(), core::cmp::Reverse(r.end())));
    let box_spans = raw_boxes
        .into_iter()
        .enumerate()
        .map(|(i, (range, kind))| BoxSpan {
            span: kind.to_span((i + 1) as u64, range),
            kind,
        })
        .collect();
    Marked {
        text,
        spans,
        decorations,
        paint_spans,
        box_spans,
        removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(source: &str) -> (String, Vec<(String, String)>) {
        let marked = parse(source);
        let spans = marked
            .spans
            .iter()
            .map(|(range, locale)| {
                (
                    marked.text[range.as_range()].to_owned(),
                    locale.as_str().to_owned(),
                )
            })
            .collect();
        (marked.text, spans)
    }

    #[test]
    fn tags_become_spans_over_plain_text() {
        let (text, spans) = spans(
            "<zh-CN>令你甘心。骨肉分离。</zh-CN> <zh-TW>令你甘心。骨肉分離。</zh-TW> <ja>令。骨肉分離。</ja>",
        );
        assert_eq!(
            text,
            "令你甘心。骨肉分离。 令你甘心。骨肉分離。 令。骨肉分離。"
        );
        assert_eq!(
            spans,
            [
                ("令你甘心。骨肉分离。".into(), "zh-CN".into()),
                ("令你甘心。骨肉分離。".into(), "zh-TW".into()),
                ("令。骨肉分離。".into(), "ja".into()),
            ]
        );
    }

    #[test]
    fn the_lang_form_and_shorthand_are_read_leniently() {
        let (text, spans) = spans("a<lang=zh-Hant>令</lang>b<jp>骨</jp><lang='tw'>x</lang>");
        assert_eq!(text, "a令b骨x");
        assert_eq!(
            spans,
            [
                ("令".into(), "zh-Hant".into()),
                ("骨".into(), "ja-JP".into()),
                ("x".into(), "zh-TW".into()),
            ]
        );
    }

    #[test]
    fn nested_tags_take_the_innermost_locale() {
        let (text, spans) = spans("<zh-TW>甲<ja>乙</ja>丙</zh-TW>");
        assert_eq!(text, "甲乙丙");
        assert_eq!(
            spans,
            [
                ("甲".into(), "zh-TW".into()),
                ("乙".into(), "ja".into()),
                ("丙".into(), "zh-TW".into()),
            ]
        );
    }

    #[test]
    fn a_closer_pairs_with_the_nearest_opener_of_its_key() {
        let (text, spans) = spans("<zh-TW>甲<ja>乙</zh-TW>丙");
        assert_eq!(text, "甲<ja>乙丙");
        assert_eq!(spans, [("甲<ja>乙".into(), "zh-TW".into())]);
    }

    #[test]
    fn quotes_must_match() {
        let (text, spans) = spans("<lang=\"zh-TW\">甲</lang>");
        assert_eq!((text.as_str(), spans.len()), ("甲", 1));
        for source in [
            "<lang=\"zh-TW'>甲</lang>",
            "<lang=\"zh-TW>甲</lang>",
            "<lang=\">甲</lang>",
        ] {
            let marked = parse(source);
            assert!(marked.spans.is_empty(), "{source}");
            assert!(marked.text.starts_with("<lang="), "{source}");
        }
    }

    #[test]
    fn many_tags_parse_in_linear_time() {
        let source =
            "<ja>甲</ja>".repeat(50_000) + &"<zh-TW>".repeat(50_000) + &"</ja>".repeat(50_000);
        let started = std::time::Instant::now();
        let marked = parse(&source);
        assert_eq!(marked.spans.len(), 1, "adjacent ja spans merge");
        assert!(started.elapsed().as_secs() < 5);
    }

    #[test]
    fn a_long_run_of_openers_tokenizes_in_linear_time() {
        for source in ["<".repeat(200_000), "<".repeat(200_000) + ">"] {
            let started = std::time::Instant::now();
            assert_eq!(parse(&source).text, source);
            assert!(started.elapsed().as_secs() < 5);
        }
    }

    #[test]
    fn unpaired_or_foreign_tags_stay_as_text() {
        for source in [
            "a < b",
            "<ja>open",
            "close</ja>",
            "<b>bold</b>",
            "<ja>x</zh-TW>",
            "<x=1>y</x>",
        ] {
            let marked = parse(source);
            assert_eq!(marked.text, source, "{source}");
            assert!(marked.spans.is_empty(), "{source}");
        }
        // An empty pair is markup with nothing to cover.
        assert_eq!(
            parse("a<ja></ja>b"),
            Marked {
                text: "ab".into(),
                spans: vec![],
                decorations: vec![],
                paint_spans: vec![],
                box_spans: vec![],
                removed: vec![1..5, 5..10],
            }
        );
    }

    #[test]
    fn source_offsets_map_past_removed_tags() {
        let marked = parse("ab<ja>cd</ja>ef");
        assert_eq!(marked.text, "abcdef");
        // Before, inside, between and after the tags.
        for (source, text) in [
            (0, 0),
            (2, 2),
            (4, 2),
            (6, 2),
            (7, 3),
            (8, 4),
            (12, 4),
            (13, 4),
            (15, 6),
        ] {
            assert_eq!(marked.text_offset(source), text, "source byte {source}");
        }
    }
}
