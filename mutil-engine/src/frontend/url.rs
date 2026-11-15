//! URL tailoring (spec FE-22) and the no-line-end words (FE-23).
//!
//! A URL is a scheme (a letter, then letters, digits, `+`, `-` or `.`)
//! followed by `://`, or a word starting `www.`, up to the next white space,
//! without trailing sentence punctuation. Inside it the hook adds break
//! opportunities after `://`, before `.`, `,`, `-`, `_`, `?`, `#`, `%` and
//! `~`, after `/`, and on both sides of `=` and `&`, never beside `(` or `[`;
//! a piece of 16 bytes or more between those breaks may break at any
//! grapheme boundary. Every opportunity strictly inside a URL, the
//! segmenter's included, takes a penalty in the URL band (WN-19): the
//! band's minimum at the punctuation, its maximum inside a long piece.

use super::DiscretionaryNode;
use crate::types::TextRange;

/// The length from which a piece between URL breaks may break anywhere.
const LONG_PIECE: usize = 16;

/// The URLs of `text`, ascending and disjoint.
pub(crate) fn url_ranges(text: &str) -> Vec<TextRange> {
    let bytes = text.as_bytes();
    let mut out: Vec<TextRange> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let at_word_start = i == 0
            || text[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        let start = if at_word_start && text[i..].starts_with("www.") {
            Some(i)
        } else if text[i..].starts_with("://") {
            // Walk back over the scheme.
            let mut s = i;
            while s > 0
                && matches!(bytes[s - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'+' | b'-' | b'.')
            {
                s -= 1;
            }
            while s < i && !bytes[s].is_ascii_alphabetic() {
                s += 1;
            }
            (s < i && out.last().is_none_or(|r| r.end() <= s)).then_some(s)
        } else {
            None
        };
        let Some(start) = start else {
            i += text[i..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let mut end = text[start..]
            .char_indices()
            .find(|&(_, c)| c.is_whitespace())
            .map_or(text.len(), |(k, _)| start + k);
        // Bracket balances over `start..end`, counted once and kept as the
        // tail is trimmed, so trimming is linear in the URL's length.
        let (mut paren, mut square) = (0_isize, 0_isize);
        for b in &bytes[start..end] {
            match b {
                b'(' => paren += 1,
                b')' => paren -= 1,
                b'[' => square += 1,
                b']' => square -= 1,
                _ => {}
            }
        }
        while let Some(c) = text[start..end].chars().next_back() {
            if !matches!(
                c,
                '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | ')' | ']'
            ) {
                break;
            }
            // A closing bracket stays when the URL opened it.
            let opened = match c {
                ')' => paren >= 0,
                ']' => square >= 0,
                _ => false,
            };
            if opened {
                break;
            }
            match c {
                ')' => paren += 1,
                ']' => square += 1,
                _ => {}
            }
            end -= c.len_utf8();
        }
        if end > start + 3 {
            out.push(TextRange::new(start, end));
        }
        i = end.max(i + 1);
        while !text.is_char_boundary(i) {
            i += 1;
        }
    }
    out
}

/// The opportunities the hook adds inside `url` with their penalties, at
/// offsets strictly inside it that are grapheme boundaries.
pub(crate) fn url_opportunities(
    text: &str,
    url: TextRange,
    graphemes: &[usize],
) -> Vec<(usize, i32)> {
    let s = &text[url.as_range()];
    let base = url.start();
    let scheme_end = s.find("://").map(|k| base + k + 3);
    let mut marks: Vec<usize> = Vec::new();
    for (k, c) in s.char_indices() {
        let at = base + k;
        match c {
            '.' | ',' | '-' | '_' | '?' | '#' | '%' | '~' => marks.push(at),
            '/' => marks.push(at + 1),
            '=' | '&' => {
                marks.push(at);
                marks.push(at + 1);
            }
            _ => {}
        }
    }
    if let Some(e) = scheme_end {
        marks.retain(|&m| m >= e);
        marks.push(e);
    }
    let beside_bracket =
        |at: usize| text[..at].ends_with(['(', '[']) || text[at..].starts_with(['(', '[']);
    let inside = |at: usize| at > url.start() && at < url.end();
    marks.retain(|&m| inside(m) && !beside_bracket(m) && graphemes.binary_search(&m).is_ok());
    marks.sort_unstable();
    marks.dedup();
    let mut out: Vec<(usize, i32)> = marks
        .iter()
        .map(|&m| (m, DiscretionaryNode::URL_PENALTY_MIN))
        .collect();
    // Long pieces between the marks may break anywhere.
    let mut edges = vec![scheme_end.unwrap_or(url.start())];
    edges.extend(marks.iter().copied());
    edges.push(url.end());
    for pair in edges.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if b.saturating_sub(a) < LONG_PIECE {
            continue;
        }
        let lo = graphemes.partition_point(|&g| g <= a);
        let hi = graphemes.partition_point(|&g| g < b);
        for &g in &graphemes[lo..hi] {
            if inside(g) && !beside_bracket(g) {
                out.push((g, DiscretionaryNode::URL_PENALTY_MAX));
            }
        }
    }
    out.sort_unstable();
    out.dedup_by_key(|(at, _)| *at);
    out
}

/// The one-letter words that bind to the next word in `language` (FE-23).
pub(crate) fn no_line_end_words(language: &str) -> &'static str {
    match language {
        "cs" => "AIiVvOoUuSsZzKk",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graphemes(text: &str) -> Vec<usize> {
        text.char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect()
    }

    #[test]
    fn urls_are_found_without_trailing_punctuation() {
        let text = "see https://example.com/a(b) and www.rust-lang.org.";
        let found: Vec<&str> = url_ranges(text)
            .iter()
            .map(|r| &text[r.as_range()])
            .collect();
        assert_eq!(found, ["https://example.com/a(b)", "www.rust-lang.org"]);
        assert!(url_ranges("no url: here").is_empty());
        // Unopened closers go, opened ones stay, in one linear pass.
        let text = "(see https://x.org/a(b)c))) now";
        let found: Vec<&str> = url_ranges(text)
            .iter()
            .map(|r| &text[r.as_range()])
            .collect();
        assert_eq!(found, ["https://x.org/a(b)c"]);
        let long = format!("https://x.org/{}", ")".repeat(100_000));
        assert_eq!(url_ranges(&long)[0].end(), "https://x.org/".len());
    }

    #[test]
    fn url_breaks_fall_after_the_scheme_and_at_punctuation() {
        let text = "https://example.com/path?q=1";
        let url = url_ranges(text)[0];
        let at: Vec<&str> = url_opportunities(text, url, &graphemes(text))
            .iter()
            .map(|&(k, _)| &text[k..])
            .collect();
        assert_eq!(
            at,
            [
                "example.com/path?q=1",
                ".com/path?q=1",
                "path?q=1",
                "?q=1",
                "=1",
                "1"
            ]
        );
    }

    #[test]
    fn long_pieces_break_anywhere_but_beside_brackets() {
        let text = "https://x.org/abcdefghijklmnopqrst(u)";
        let url = url_ranges(text)[0];
        let ops = url_opportunities(text, url, &graphemes(text));
        assert!(
            ops.iter()
                .any(|&(k, p)| &text[k..k + 1] == "f" && p == DiscretionaryNode::URL_PENALTY_MAX)
        );
        assert!(
            !ops.iter()
                .any(|&(k, _)| text[..k].ends_with('(') || text[k..].starts_with('('))
        );
    }
}
