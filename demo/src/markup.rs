//! Inline locale markup for the harness's text box.
//!
//! The engine takes plain UTF-8 paragraphs (spec AX-1) and never guesses
//! between Simplified and Traditional Han; a rich-text host states inline
//! language switches as `StyleSpan`s. This module is that host step for the
//! page: it strips tags such as `<zh-TW>…</zh-TW>`, `<ja>…</ja>` or
//! `<lang=zh-Hant>…</lang>` and returns the plain text with the byte ranges
//! each tag covered and its locale, read with `LocaleTag::parse_lenient`.
//!
//! A tag is markup only when it parses as a locale and is closed by its own
//! closing tag (`</lang>` for the `lang=` form); anything else, an unclosed
//! opener or a stray closer included, stays in the text as typed. Nested tags
//! are flattened: each byte takes its innermost tag, so the spans the engine
//! receives are disjoint and ascending (spec FE-25).

use mutil_engine::{LocaleTag, TextRange};

/// Plain text and the locale of each tagged stretch of it.
#[derive(Debug, PartialEq, Eq)]
pub struct Marked {
    /// The text with the markup removed.
    pub text: String,
    /// Disjoint, ascending, non-empty ranges of `text` with their locale.
    pub spans: Vec<(TextRange, LocaleTag)>,
}

enum Token {
    Text(core::ops::Range<usize>),
    Open {
        close: String,
        locale: LocaleTag,
        raw: core::ops::Range<usize>,
    },
    Close {
        key: String,
        raw: core::ops::Range<usize>,
    },
}

/// Reads `<…>` at `at`, if it is a tag of this markup.
fn tag_at(source: &str, at: usize) -> Option<(Token, usize)> {
    let rest = &source[at..];
    let end = rest.find('>')?;
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
    let (close, value) = match inner.split_once('=') {
        Some((attr, value)) if attr.eq_ignore_ascii_case("lang") => {
            let value = value.trim_matches(|c| c == '"' || c == '\'');
            ("lang".to_owned(), value)
        }
        Some(_) => return None,
        None => (inner.to_ascii_lowercase(), inner),
    };
    if !name_ok(value) {
        return None;
    }
    let locale = LocaleTag::parse_lenient(value).ok()?;
    Some((
        Token::Open {
            close,
            locale,
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

/// Strips the locale markup from `source` (see the module documentation).
#[must_use]
pub fn parse(source: &str) -> Marked {
    let tokens = tokens(source);
    // Pair openers with closers; an unpaired tag is literal text.
    let mut paired = vec![false; tokens.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Open { .. } => stack.push(index),
            Token::Close { key, .. } => {
                let top = stack.last().copied();
                if let Some(open) = top
                    && let Token::Open { close, .. } = &tokens[open]
                    && close == key
                {
                    stack.pop();
                    paired[open] = true;
                    paired[index] = true;
                }
            }
            Token::Text(_) => {}
        }
    }
    let mut text = String::with_capacity(source.len());
    // (start, end, depth, locale) of each paired tag, in output bytes.
    let mut tagged: Vec<(usize, usize, usize, LocaleTag)> = Vec::new();
    let mut open: Vec<(usize, LocaleTag)> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Text(range) => text.push_str(&source[range.clone()]),
            Token::Open { raw, .. } | Token::Close { raw, .. } if !paired[index] => {
                text.push_str(&source[raw.clone()]);
            }
            Token::Open { locale, .. } => open.push((text.len(), locale.clone())),
            Token::Close { .. } => {
                let depth = open.len();
                let (start, locale) = open.pop().expect("paired");
                if start < text.len() {
                    tagged.push((start, text.len(), depth, locale));
                }
            }
        }
    }
    // Flatten: between consecutive edges, the deepest tag covering the
    // stretch gives its locale.
    let mut edges: Vec<usize> = tagged.iter().flat_map(|&(s, e, ..)| [s, e]).collect();
    edges.sort_unstable();
    edges.dedup();
    let mut spans: Vec<(TextRange, LocaleTag)> = Vec::new();
    for pair in edges.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let Some((.., locale)) = tagged
            .iter()
            .filter(|&&(s, e, ..)| s <= start && end <= e)
            .max_by_key(|&&(_, _, depth, _)| depth)
        else {
            continue;
        };
        match spans.last_mut() {
            Some((range, last)) if range.end() == start && last == locale => {
                *range = TextRange::new(range.start(), end);
            }
            _ => spans.push((TextRange::new(start, end), locale.clone())),
        }
    }
    Marked { text, spans }
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
                spans: vec![]
            }
        );
    }
}
