//! Terminal-safe text handling. Transcript text is untrusted data: every
//! character that reaches the screen passes through `clean`, and every width
//! calculation uses display columns rather than bytes or chars.
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Replaces characters a terminal would interpret instead of display: C0/C1
/// controls (including ESC, so ANSI sequences render as inert text) and the
/// bidirectional overrides that can visually reorder a line.
pub(crate) fn clean(c: char) -> char {
    let bidi =
        matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
    if c.is_control() || bidi {
        ' '
    } else {
        c
    }
}

/// Sanitizes `value` and truncates it to at most `width` display columns.
pub fn safe(value: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in value.chars().map(clean) {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out
}

/// Sanitizes and word-wraps `text` to lines of at most `width` columns.
/// Newlines separate paragraphs; words wider than a line are split.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let paragraph: String = paragraph.chars().map(clean).collect();
        if paragraph.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if UnicodeWidthStr::width(word) > width {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                let mut used = 0;
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if used + cw > width && !line.is_empty() {
                        out.push(std::mem::take(&mut line));
                        used = 0;
                    }
                    line.push(c);
                    used += cw;
                }
                continue;
            }
            if !line.is_empty()
                && UnicodeWidthStr::width(line.as_str()) + 1 + UnicodeWidthStr::width(word) > width
            {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

pub(crate) fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Terms from an FTS5 query worth highlighting: words, with phrase quotes,
/// prefix stars, grouping, column filters and boolean operators removed.
pub(crate) fn query_terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for token in query.split_whitespace() {
        if matches!(token, "AND" | "OR" | "NOT" | "NEAR") {
            continue;
        }
        for word in token.split(|c: char| !c.is_alphanumeric()) {
            let word: String = word.chars().map(fold).collect();
            if !word.is_empty() && !terms.contains(&word) {
                terms.push(word);
            }
        }
    }
    terms
}

/// Byte ranges of `line` to highlight: each word that starts with a query
/// term, extended to the end of that word (FTS matches whole tokens and
/// prefixes, so the whole word is what matched).
pub(crate) fn matches(line: &str, terms: &[String]) -> Vec<(usize, usize)> {
    if terms.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let at_word_start =
            chars[i].1.is_alphanumeric() && (i == 0 || !chars[i - 1].1.is_alphanumeric());
        if at_word_start {
            let found = terms.iter().any(|term| {
                let mut j = i;
                for t in term.chars() {
                    if j >= chars.len() || fold(chars[j].1) != t {
                        return false;
                    }
                    j += 1;
                }
                true
            });
            if found {
                let mut end = i;
                while end < chars.len() && chars[end].1.is_alphanumeric() {
                    end += 1;
                }
                let end_byte = chars.get(end).map_or(line.len(), |(b, _)| *b);
                out.push((chars[i].0, end_byte));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_neutralizes_controls_and_respects_wide_glyphs() {
        assert_eq!(safe("a\n雪雪x", 4), "a 雪");
        assert_eq!(safe("\u{1b}[31mred", 20), " [31mred");
        assert_eq!(safe("a\u{202e}b", 5), "a b");
    }

    #[test]
    fn wrap_bounds_every_line_and_strips_controls() {
        let lines = wrap("abcdefghij 雪 \u{1b}[2J\u{7}bell e\u{301}\u{301}", 4);
        assert!(lines.iter().all(|s| width(s) <= 4), "{lines:?}");
        assert!(lines.iter().all(|s| !s.chars().any(char::is_control)));
        assert_eq!(wrap("", 10), vec![String::new()]);
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
    }

    #[test]
    fn query_terms_drop_fts_syntax() {
        assert_eq!(
            query_terms(r#""Portfolio visibility" OR portf* NOT (x:y)"#),
            vec!["portfolio", "visibility", "portf", "x", "y"]
        );
        assert!(query_terms("  ").is_empty());
    }

    #[test]
    fn matches_whole_words_by_prefix_case_insensitively() {
        let line = "Portfolio visibility; notportfolio portfolios";
        let terms = query_terms("portfolio");
        let spans: Vec<&str> = matches(line, &terms)
            .into_iter()
            .map(|(a, b)| &line[a..b])
            .collect();
        assert_eq!(spans, vec!["Portfolio", "portfolios"]);
        assert!(matches("雪 portfolio", &terms)
            .iter()
            .all(|&(a, b)| "雪 portfolio".is_char_boundary(a) && b <= "雪 portfolio".len()));
    }
}
