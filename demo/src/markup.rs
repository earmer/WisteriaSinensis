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
//! closing tag (`</lang>` for the `lang=` form), which pairs with the nearest
//! open opener of its name; a `lang=` value is unquoted or inside one
//! matching pair of quotes. Anything else, an unclosed opener or a stray
//! closer included, stays in the text as typed. Nested tags
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
    let (close, value) = match inner.split_once('=') {
        Some((attr, value)) if attr.eq_ignore_ascii_case("lang") => {
            // Unquoted, or inside one matching pair of quotes.
            let value = match value.as_bytes() {
                [q @ (b'"' | b'\''), .., last] if last == q && value.len() >= 2 => {
                    &value[1..value.len() - 1]
                }
                _ => value,
            };
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

/// Strips the locale markup from `source` (see the module documentation).
#[must_use]
pub fn parse(source: &str) -> Marked {
    let tokens = tokens(source);
    let paired = pair(&tokens);
    let mut text = String::with_capacity(source.len());
    let mut spans: Vec<(TextRange, LocaleTag)> = Vec::new();
    // The open paired tags, innermost last; the stretch since the last tag
    // edge takes the innermost one's locale.
    let mut open: Vec<&LocaleTag> = Vec::new();
    let mut since = 0;
    let mut flush = |open: &[&LocaleTag], since: usize, end: usize| {
        let Some(&locale) = open.last() else { return };
        if since == end {
            return;
        }
        match spans.last_mut() {
            Some((range, last)) if range.end() == since && last == locale => {
                *range = TextRange::new(range.start(), end);
            }
            _ => spans.push((TextRange::new(since, end), locale.clone())),
        }
    };
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Text(range) => text.push_str(&source[range.clone()]),
            Token::Open { raw, .. } | Token::Close { raw, .. } if !paired[index] => {
                text.push_str(&source[raw.clone()]);
            }
            Token::Open { locale, .. } => {
                flush(&open, since, text.len());
                open.push(locale);
                since = text.len();
            }
            Token::Close { .. } => {
                flush(&open, since, text.len());
                open.pop();
                since = text.len();
            }
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
                spans: vec![]
            }
        );
    }
}
