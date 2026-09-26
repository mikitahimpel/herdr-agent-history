//! Translation of user search text into an FTS5 `MATCH` expression.
//!
//! Users type ordinary text, so every character is literal except the small
//! documented syntax: double-quoted phrases, a trailing `*` prefix marker, and
//! the uppercase binary operators `AND`, `OR` and `NOT`. Everything else,
//! including `-`, `:`, `+`, `^`, parentheses and `NEAR`, is wrapped in an FTS5
//! string so the tokenizer only ever sees it as text to split into words.

/// Converts user search text into an FTS5 expression that cannot fail to parse.
///
/// Tokens are separated by whitespace outside double quotes. A token that is
/// exactly one quoted span is kept as a phrase (an unclosed quote is closed at
/// the end of the input, and `""` inside a span stands for a literal quote). A
/// trailing `*` on a phrase or bare token makes it a prefix query. `AND`, `OR`
/// and `NOT` stay operators only between two terms; anywhere else they are
/// searched as words. Every other token becomes a quoted string, so the
/// tokenizer, not the query parser, decides which characters separate words:
/// `rate-limit` is the phrase `rate limit`, and `C++` matches the indexed text
/// `C++` exactly as it was tokenized.
pub fn fts_match_expression(query: &str) -> String {
    let tokens: Vec<Token> = split(query).into_iter().map(classify).collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut last_was_term = false;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Term(term) => {
                out.push(term.clone());
                last_was_term = true;
            }
            Token::Operator(op) => {
                let next = tokens.get(i + 1);
                if *op == "AND" && matches!(next, Some(Token::Operator("NOT"))) && last_was_term {
                    // `a AND NOT b` means `a NOT b`; FTS5 rejects the former.
                    continue;
                }
                if last_was_term && matches!(next, Some(Token::Term(_))) {
                    out.push((*op).to_string());
                    last_was_term = false;
                } else {
                    out.push(quote(op, false));
                    last_was_term = true;
                }
            }
        }
    }
    out.join(" ")
}

enum Token {
    Term(String),
    Operator(&'static str),
}

/// Splits on whitespace that is not inside a double-quoted span.
fn split(query: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut in_quote = false;
    for (i, c) in query.char_indices() {
        if c == '"' {
            in_quote = !in_quote;
        }
        if c.is_whitespace() && !in_quote {
            if let Some(s) = start.take() {
                tokens.push(&query[s..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        tokens.push(&query[s..]);
    }
    tokens
}

fn classify(raw: &str) -> Token {
    match raw {
        "AND" => return Token::Operator("AND"),
        "OR" => return Token::Operator("OR"),
        "NOT" => return Token::Operator("NOT"),
        _ => {}
    }
    if let Some((text, prefix)) = phrase(raw) {
        return Token::Term(quote(&text, prefix));
    }
    let body = raw.trim_end_matches('*');
    if body.is_empty() {
        Token::Term(quote(raw, false))
    } else {
        Token::Term(quote(body, body.len() < raw.len()))
    }
}

/// Parses a token that is exactly one quoted span, optionally followed by `*`.
fn phrase(raw: &str) -> Option<(String, bool)> {
    let rest = raw.strip_prefix('"')?;
    let mut text = String::new();
    let mut chars = rest.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '"' {
            text.push(c);
        } else if matches!(chars.peek(), Some((_, '"'))) {
            chars.next();
            text.push('"');
        } else {
            let tail = &rest[i + 1..];
            return if tail.is_empty() {
                Some((text, false))
            } else if tail.chars().all(|c| c == '*') {
                Some((text, true))
            } else {
                None
            };
        }
    }
    Some((text, false))
}

fn quote(text: &str, prefix: bool) -> String {
    let mut s = String::with_capacity(text.len() + 3);
    s.push('"');
    s.push_str(&text.replace('"', "\"\""));
    s.push('"');
    if prefix {
        s.push('*');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::fts_match_expression as fts;

    #[test]
    fn punctuation_is_quoted_as_literal_text() {
        assert_eq!(fts("cobalt-heron"), r#""cobalt-heron""#);
        assert_eq!(fts("rate-limit"), r#""rate-limit""#);
        assert_eq!(fts("foo:bar"), r#""foo:bar""#);
        assert_eq!(fts("what?"), r#""what?""#);
        assert_eq!(fts("C++"), r#""C++""#);
        assert_eq!(fts("a(b)"), r#""a(b)""#);
        assert_eq!(fts("dev@example.com"), r#""dev@example.com""#);
        assert_eq!(fts("/usr/local/bin"), r#""/usr/local/bin""#);
        assert_eq!(fts("-"), r#""-""#);
        assert_eq!(fts("-foo ^bar"), r#""-foo" "^bar""#);
        assert_eq!(fts("NEAR(a b)"), r#""NEAR(a" "b)""#);
        assert_eq!(fts("text:secret"), r#""text:secret""#);
    }

    #[test]
    fn quotes_are_balanced_and_escaped() {
        assert_eq!(fts(r#"""#), r#""""#);
        assert_eq!(fts(r#""portfolio visibility"#), r#""portfolio visibility""#);
        assert_eq!(fts(r#"say "hi"#), r#""say" "hi""#);
        assert_eq!(fts(r#"5" screen"#), r#""5"" screen""#);
        assert_eq!(fts(r#"it"s"#), r#""it""s""#);
        assert_eq!(fts(r#""a ""b"" c""#), r#""a ""b"" c""#);
        assert_eq!(fts(r#""foo"bar"#), r#""""foo""bar""#);
    }

    #[test]
    fn documented_syntax_is_preserved() {
        assert_eq!(
            fts(r#""portfolio visibility""#),
            r#""portfolio visibility""#
        );
        assert_eq!(fts("portfolio*"), r#""portfolio"*"#);
        assert_eq!(fts(r#""portfolio vis"*"#), r#""portfolio vis"*"#);
        assert_eq!(fts("rate-lim*"), r#""rate-lim"*"#);
        assert_eq!(fts("a AND b"), r#""a" AND "b""#);
        assert_eq!(fts("a OR b"), r#""a" OR "b""#);
        assert_eq!(fts("a NOT b"), r#""a" NOT "b""#);
        assert_eq!(fts("a AND NOT b"), r#""a" NOT "b""#);
        assert_eq!(
            fts(r#"alph* AND "beta gamma""#),
            r#""alph"* AND "beta gamma""#
        );
    }

    #[test]
    fn misplaced_operators_become_words() {
        assert_eq!(fts("AND"), r#""AND""#);
        assert_eq!(fts("NOT a"), r#""NOT" "a""#);
        assert_eq!(fts("a OR"), r#""a" "OR""#);
        assert_eq!(fts("a AND OR b"), r#""a" "AND" OR "b""#);
        assert_eq!(fts("a and b"), r#""a" "and" "b""#);
    }

    #[test]
    fn punctuation_only_and_whitespace_queries() {
        assert_eq!(fts("?!"), r#""?!""#);
        assert_eq!(fts("*"), r#""*""#);
        assert_eq!(fts("   "), "");
        assert_eq!(fts(" \t a \n b "), r#""a" "b""#);
    }

    #[test]
    fn non_ascii_text_is_kept_intact() {
        assert_eq!(fts("café-crème"), r#""café-crème""#);
        assert_eq!(fts("日本語 検索*"), r#""日本語" "検索"*"#);
        assert_eq!(fts("\u{201c}smart\u{201d}"), "\"\u{201c}smart\u{201d}\"");
    }
}
