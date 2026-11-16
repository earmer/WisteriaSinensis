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
        if !bytes[i].eq_ignore_ascii_case(&b'w') && bytes[i] != b':' {
            i += 1;
            continue;
        }
        let at_word_start = i == 0
            || text[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        let start = if at_word_start
            && bytes[i..]
                .get(..4)
                .is_some_and(|w| w.eq_ignore_ascii_case(b"www."))
        {
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
            i += 1;
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
            match c {
                // A closing bracket stays when the URL opened it.
                ')' if paren >= 0 => break,
                ']' if square >= 0 => break,
                ')' => paren += 1,
                ']' => square += 1,
                '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => {}
                _ => break,
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
    let bytes = text.as_bytes();
    let s = &text[url.as_range()];
    let base = url.start();
    let scheme_end = s.find("://").map(|k| base + k + 3);
    let scan_start = scheme_end.unwrap_or(url.start());
    let lo = graphemes.partition_point(|&g| g < scan_start);
    let hi = graphemes.partition_point(|&g| g < url.end());
    let slice = &graphemes[lo..hi];

    let beside_bracket =
        |at: usize| matches!(bytes[at - 1], b'(' | b'[') || matches!(bytes[at], b'(' | b'[');
    let emit_long_piece = |out: &mut Vec<(usize, i32)>,
                           candidates: &[usize],
                           piece_start: usize,
                           piece_end: usize| {
        if piece_end.saturating_sub(piece_start) < LONG_PIECE {
            return;
        }
        for &g in candidates {
            if g > piece_start && !beside_bracket(g) {
                out.push((g, DiscretionaryNode::URL_PENALTY_MAX));
            }
        }
    };

    let mut out = Vec::new();
    let mut piece_start = scan_start;
    let mut piece_start_idx = 0usize;
    for (idx, &g) in slice.iter().enumerate() {
        if g <= url.start() || beside_bracket(g) {
            continue;
        }
        let is_mark = scheme_end == Some(g)
            || matches!(
                bytes[g],
                b'.' | b',' | b'-' | b'_' | b'?' | b'#' | b'%' | b'~' | b'=' | b'&'
            )
            || matches!(bytes[g - 1], b'/' | b'=' | b'&');
        if !is_mark {
            continue;
        }
        emit_long_piece(&mut out, &slice[piece_start_idx..idx], piece_start, g);
        out.push((g, DiscretionaryNode::URL_PENALTY_MIN));
        piece_start = g;
        piece_start_idx = idx + 1;
    }
    emit_long_piece(&mut out, &slice[piece_start_idx..], piece_start, url.end());
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
