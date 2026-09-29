//! Translation of user search text into an FTS5 `MATCH` expression.
//!
//! Users type ordinary text, so every character is literal except the small
//! documented syntax: double-quoted phrases, a trailing `*` prefix marker, and
//! the uppercase binary operators `AND`, `OR` and `NOT`. Everything else,
//! including `-`, `:`, `+`, `^`, parentheses and `NEAR`, is wrapped in an FTS5
//! string so the tokenizer only ever sees it as text to split into words.
//!
//! When that exact expression matches nothing, [`lenient_match_expression`]
//! builds a retry in which misspelled bare words also match indexed words a
//! small edit distance away. The retry is pure string matching over the words
//! already in the index.

use crate::Widening;

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
    match build(query, |_| Ok::<_, std::convert::Infallible>(None)) {
        Ok(expression) => expression,
        Err(never) => match never {},
    }
}

/// Words shorter than this are never widened: at one edit apart, almost every
/// short word is near some other indexed word.
pub const MIN_WIDEN_CHARS: usize = 4;
/// At most this many distinct words are widened in one query, so a long pasted
/// query cannot turn into an unbounded number of vocabulary scans.
pub const MAX_WIDENED_WORDS: usize = 8;
/// At most this many near words are searched for each widened word.
pub const MAX_NEAR_WORDS: usize = 5;

/// The expression for the lenient retry of a query whose exact expression
/// matched nothing, or `None` when no word in it can be widened.
///
/// Only a bare word can be widened: a token of letters and digits (at least one
/// letter, at least [`MIN_WIDEN_CHARS`] long) with no quotes and no `*`, that is
/// not the operand of `NOT`. Everything else — phrases, prefixes, operators,
/// negated words and punctuated tokens — is emitted exactly as
/// [`fts_match_expression`] emits it. For each bare word, `near` receives the
/// lowercase word and returns `None` to keep it exact (it is already indexed),
/// or the indexed words to search in its place. A widened word becomes
/// `("word"* OR "near1" OR ...)`, so the retry still requires every other term.
pub fn lenient_match_expression<E>(
    query: &str,
    mut near: impl FnMut(&str) -> Result<Option<Vec<String>>, E>,
) -> Result<Option<(String, Vec<Widening>)>, E> {
    let mut widened: Vec<Widening> = Vec::new();
    let mut kept: Vec<String> = Vec::new();
    let expression = build(query, |word| {
        if let Some(w) = widened.iter().find(|w| w.typed == word) {
            return Ok(Some(group(&w.typed, &w.near)));
        }
        if kept.iter().any(|k| k == word) || widened.len() >= MAX_WIDENED_WORDS {
            return Ok(None);
        }
        match near(word)? {
            Some(words) => {
                let expression = group(word, &words);
                widened.push(Widening {
                    typed: word.to_string(),
                    near: words,
                });
                Ok(Some(expression))
            }
            None => {
                kept.push(word.to_string());
                Ok(None)
            }
        }
    })?;
    Ok((!widened.is_empty()).then_some((expression, widened)))
}

/// Picks the indexed words close enough to `typed` to search in its place,
/// closest first and, among equally close words, the most widespread first.
///
/// `vocabulary` yields indexed words with the number of chunks containing
/// each. Words that start with `typed` are left out because the widened query
/// already searches `typed*`. Up to seven characters allow one edit, longer
/// words two; an edit is an insertion, deletion, substitution, or a swap of
/// two adjacent characters.
pub fn near_words(typed: &str, vocabulary: impl IntoIterator<Item = (String, u64)>) -> Vec<String> {
    let typed_chars: Vec<char> = typed.chars().collect();
    let max = if typed_chars.len() >= 8 { 2 } else { 1 };
    let mut found: Vec<(usize, u64, String)> = vocabulary
        .into_iter()
        .filter(|(word, _)| !word.starts_with(typed))
        .filter_map(|(word, docs)| {
            let chars: Vec<char> = word.chars().collect();
            edit_distance(&typed_chars, &chars, max).map(|d| (d, docs, word))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    found.truncate(MAX_NEAR_WORDS);
    found.into_iter().map(|(_, _, word)| word).collect()
}

/// Optimal string alignment distance between `a` and `b`, or `None` when it
/// exceeds `max`.
fn edit_distance(a: &[char], b: &[char], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let width = b.len() + 1;
    let mut rows = vec![vec![0usize; width]; a.len() + 1];
    for (j, cell) in rows[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        rows[i][0] = i;
        let mut best = rows[i][0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (rows[i - 1][j] + 1)
                .min(rows[i][j - 1] + 1)
                .min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = d;
            best = best.min(d);
        }
        if best > max {
            return None;
        }
    }
    let d = rows[a.len()][b.len()];
    (d <= max).then_some(d)
}

fn group(word: &str, near: &[String]) -> String {
    let mut parts = vec![quote(word, true)];
    parts.extend(near.iter().map(|w| quote(w, false)));
    format!("({})", parts.join(" OR "))
}

/// Emits the expression, asking `widen` for a replacement for each bare word
/// that is not negated.
fn build<E>(
    query: &str,
    mut widen: impl FnMut(&str) -> Result<Option<String>, E>,
) -> Result<String, E> {
    let tokens: Vec<Token> = split(query).into_iter().map(classify).collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut last_was_term = false;
    let mut last_was_group = false;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::Term(term, word) => {
                let negated = out.last().is_some_and(|op| op == "NOT");
                let replacement = match word {
                    Some(word) if !negated => widen(word)?,
                    _ => None,
                };
                let group = replacement.is_some();
                if last_was_term && (group || last_was_group) {
                    // FTS5 accepts implicit AND between phrases only, not
                    // next to a parenthesized group.
                    out.push("AND".to_string());
                }
                out.push(replacement.unwrap_or_else(|| term.clone()));
                last_was_term = true;
                last_was_group = group;
            }
            Token::Operator(op) => {
                let next = tokens.get(i + 1);
                if *op == "AND" && matches!(next, Some(Token::Operator("NOT"))) && last_was_term {
                    // `a AND NOT b` means `a NOT b`; FTS5 rejects the former.
                    continue;
                }
                if last_was_term && matches!(next, Some(Token::Term(..))) {
                    out.push((*op).to_string());
                    last_was_term = false;
                } else {
                    if last_was_group {
                        out.push("AND".to_string());
                    }
                    out.push(quote(op, false));
                    last_was_term = true;
                }
                last_was_group = false;
            }
        }
    }
    Ok(out.join(" "))
}

enum Token {
    /// The quoted expression, and the lowercase word when the token is a bare
    /// word that a lenient retry may widen.
    Term(String, Option<String>),
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
        return Token::Term(quote(&text, prefix), None);
    }
    let body = raw.trim_end_matches('*');
    if body.is_empty() {
        return Token::Term(quote(raw, false), None);
    }
    let prefix = body.len() < raw.len();
    let widenable = !prefix
        && body.chars().count() >= MIN_WIDEN_CHARS
        && body.chars().all(char::is_alphanumeric)
        && body.chars().any(char::is_alphabetic);
    Token::Term(quote(body, prefix), widenable.then(|| body.to_lowercase()))
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
    use super::{lenient_match_expression, near_words, MAX_WIDENED_WORDS};
    use crate::Widening;

    /// The lenient expression when every bare word is unindexed and widened to
    /// its uppercase form, recording which words were asked about.
    fn lenient(query: &str) -> (Option<String>, Vec<String>) {
        let mut asked = Vec::new();
        let out = lenient_match_expression(query, |w| {
            asked.push(w.to_string());
            Ok::<_, ()>(Some(vec![w.to_uppercase()]))
        })
        .unwrap();
        (out.map(|(e, _)| e), asked)
    }

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

    #[test]
    fn lenient_widens_only_bare_unnegated_words() {
        assert_eq!(
            lenient("databse").0.unwrap(),
            r#"("databse"* OR "DATABSE")"#
        );
        let (expression, asked) =
            lenient(r#""exact phrase" AND databse OR portfol* NOT migration"#);
        assert_eq!(
            expression.unwrap(),
            r#""exact phrase" AND ("databse"* OR "DATABSE") OR "portfol"* NOT "migration""#
        );
        assert_eq!(asked, vec!["databse"]);
        // FTS5 needs an explicit AND next to a group; exact output never has one.
        assert_eq!(
            lenient("NOT databse OR").0.unwrap(),
            r#""NOT" AND ("databse"* OR "DATABSE") AND "OR""#
        );
        assert_eq!(fts("NOT databse OR"), r#""NOT" "databse" "OR""#);
        let (expression, asked) = lenient("Databse AND NOT migration");
        assert_eq!(
            expression.unwrap(),
            r#"("databse"* OR "DATABSE") NOT "migration""#
        );
        assert_eq!(asked, vec!["databse"]);
    }

    #[test]
    fn lenient_leaves_syntax_short_words_and_punctuation_alone() {
        for query in [
            r#""databse migration""#,
            "databse*",
            "teh",
            "abc",
            "rate-limt",
            "foo:barr",
            "1234567",
            "?!",
            "---",
            "",
            "AND",
            "a NOT databse",
        ] {
            assert_eq!(lenient(query), (None, Vec::new()), "{query:?}");
        }
    }

    #[test]
    fn lenient_keeps_indexed_words_exact_and_asks_once_per_word() {
        let mut asked = Vec::new();
        let (expression, widened) = lenient_match_expression("portfolio databse databse", |w| {
            asked.push(w.to_string());
            Ok::<_, ()>((w != "portfolio").then(|| vec!["database".to_string()]))
        })
        .unwrap()
        .unwrap();
        assert_eq!(
            expression,
            r#""portfolio" AND ("databse"* OR "database") AND ("databse"* OR "database")"#
        );
        assert_eq!(asked, vec!["portfolio", "databse"]);
        assert_eq!(
            widened,
            vec![Widening {
                typed: "databse".into(),
                near: vec!["database".into()],
            }]
        );
        let none = lenient_match_expression("portfolio", |_| Ok::<_, ()>(None)).unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn lenient_widens_a_bounded_number_of_words() {
        let query: Vec<String> = (0..MAX_WIDENED_WORDS + 3)
            .map(|i| format!("word{i}"))
            .collect();
        let (expression, asked) = lenient(&query.join(" "));
        assert_eq!(asked.len(), MAX_WIDENED_WORDS);
        assert!(expression
            .unwrap()
            .ends_with(r#") AND "word8" "word9" "word10""#));
    }

    #[test]
    fn lenient_propagates_lookup_errors() {
        let err = lenient_match_expression("databse", |_| Err::<Option<Vec<String>>, _>("boom"));
        assert_eq!(err, Err("boom"));
    }

    fn vocab(words: &[(&str, u64)]) -> Vec<(String, u64)> {
        words.iter().map(|(w, n)| (w.to_string(), *n)).collect()
    }

    #[test]
    fn near_words_allow_one_edit_up_to_seven_chars_and_two_beyond() {
        let words = vocab(&[
            ("database", 3),
            ("databases", 9),
            ("dataset", 4),
            ("datum", 1),
            ("worktree", 2),
        ]);
        // Insertion, deletion, substitution, and an adjacent swap.
        assert_eq!(near_words("databse", words.clone()), vec!["database"]);
        assert_eq!(
            near_words("databasse", words.clone()),
            vec!["databases", "database"]
        );
        assert_eq!(
            near_words("datobase", words.clone()),
            vec!["database", "databases"]
        );
        assert_eq!(
            near_words("databsae", words.clone()),
            vec!["database", "databases"]
        );
        // Two edits only for words of eight or more characters.
        assert_eq!(near_words("dtabse", words.clone()), Vec::<String>::new());
        assert_eq!(near_words("dtaabse", words.clone()), Vec::<String>::new());
        assert_eq!(near_words("dtaabsae", words.clone()), vec!["database"]);
        // Words the prefix already covers are not repeated.
        assert_eq!(near_words("worktre", words.clone()), Vec::<String>::new());
        assert_eq!(near_words("zzzzqqqq", words), Vec::<String>::new());
    }

    #[test]
    fn near_words_rank_closest_then_most_common_and_are_capped() {
        let words = vocab(&[
            ("chunkers", 90),
            ("chunking", 1),
            ("chunkier", 2),
            ("chunkily", 2),
            ("chunkist", 3),
            ("chunkism", 4),
            ("chunkiest", 5),
        ]);
        // One edit beats two however common, ties go to the more common word,
        // and `chunkers` (three edits) is too far.
        let found = near_words("chunkinz", words);
        assert_eq!(
            found,
            vec!["chunking", "chunkism", "chunkist", "chunkier", "chunkily"]
        );
        assert_eq!(found.len(), super::MAX_NEAR_WORDS);
    }
}
