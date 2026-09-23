//! Markdown for display only. Transcript text is sanitized before it is
//! parsed, so markup can change how characters are styled and laid out but
//! never which characters reach the terminal. Links are not followed, raw
//! HTML is shown as literal text, and nothing is executed.
use crate::{text::clean, theme::Palette};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

/// One display character and its style.
pub(crate) type Cell = (char, Style);

/// Deepest quote/list nesting drawn; deeper levels share the last indent.
const MAX_DEPTH: usize = 4;

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// Neutralizes terminal controls while keeping the line structure that
/// markdown needs: newlines stay, tabs become spaces, carriage returns go.
pub(crate) fn sanitize_source(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push('\n'),
            '\r' => {}
            '\t' => out.push_str("    "),
            c => out.push(clean(c)),
        }
    }
    out
}

/// Markdown reduced to its words, for one-line snippets: no markers,
/// backticks, emphasis or heading hashes.
pub(crate) fn plain(text: &str) -> String {
    let source = sanitize_source(text);
    let mut out = String::with_capacity(source.len());
    for event in Parser::new_ext(&source, options()) {
        match event {
            Event::Text(t)
            | Event::Code(t)
            | Event::Html(t)
            | Event::InlineHtml(t)
            | Event::InlineMath(t)
            | Event::DisplayMath(t)
            | Event::FootnoteReference(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak | Event::Rule => out.push(' '),
            Event::End(_) => out.push(' '),
            Event::TaskListMarker(done) => out.push_str(if done { "[x] " } else { "[ ] " }),
            Event::Start(_) => {}
        }
    }
    out.split_whitespace()
        .map(|w| w.chars().map(clean).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Renders `text` as styled lines no wider than `width` columns.
pub(crate) fn render(text: &str, width: usize, base: Style, p: &Palette) -> Vec<Vec<Cell>> {
    let source = sanitize_source(text);
    let mut r = Renderer {
        p,
        width: width.max(1),
        base,
        out: Vec::new(),
        inline: Vec::new(),
        styles: Vec::new(),
        lists: Vec::new(),
        marker: None,
        marker_width: 0,
        quote: 0,
        code: None,
        gap: false,
        row_cells: 0,
    };
    for event in Parser::new_ext(&source, options()) {
        r.event(event);
    }
    r.flush();
    r.out
}

struct Renderer<'p> {
    p: &'p Palette,
    width: usize,
    base: Style,
    out: Vec<Vec<Cell>>,
    /// Inline text of the current block; '\n' marks a hard break.
    inline: Vec<Cell>,
    styles: Vec<Style>,
    /// Open lists: the next number of an ordered list, `None` for bullets.
    lists: Vec<Option<u64>>,
    /// List marker still to be drawn on the next flushed line.
    marker: Option<Vec<Cell>>,
    marker_width: usize,
    quote: usize,
    code: Option<String>,
    /// A blank line is owed before the next block.
    gap: bool,
    row_cells: usize,
}

impl Renderer<'_> {
    fn style(&self) -> Style {
        self.styles.iter().fold(self.base, |acc, s| acc.patch(*s))
    }

    fn push_text(&mut self, text: &str, style: Style) {
        self.inline.extend(text.chars().map(|c| (clean(c), style)));
    }

    fn event(&mut self, event: Event) {
        let p = self.p;
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => match &mut self.code {
                Some(code) => code.push_str(&t),
                None => self.push_text(&t, self.style()),
            },
            Event::Code(t) => {
                let style = self.style().fg(p.peach).bg(p.surface0);
                self.push_text(&t, style);
            }
            Event::Html(t) | Event::InlineHtml(t) => {
                let style = self.style().fg(p.overlay0);
                self.push_text(&t, style);
            }
            Event::InlineMath(t) | Event::DisplayMath(t) | Event::FootnoteReference(t) => {
                self.push_text(&t, self.style());
            }
            Event::SoftBreak => self.push_text(" ", self.style()),
            Event::HardBreak => self.inline.push(('\n', self.base)),
            Event::Rule => {
                self.flush();
                self.block_gap();
                let rule = vec![('─', Style::new().fg(p.surface1)); self.content_width()];
                self.emit(rule, true);
                self.gap = true;
            }
            Event::TaskListMarker(done) => {
                let (mark, color) = if done {
                    ("☑ ", p.green)
                } else {
                    ("☐ ", p.overlay0)
                };
                self.push_text(mark, Style::new().fg(color));
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        let p = self.p;
        match tag {
            Tag::Paragraph | Tag::HtmlBlock => {
                self.flush();
                self.block_gap();
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.block_gap();
                let style = match level {
                    HeadingLevel::H1 => Style::new()
                        .fg(p.accent)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                    HeadingLevel::H2 => Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
                    _ => Style::new().fg(p.mauve).add_modifier(Modifier::BOLD),
                };
                self.styles.push(style);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.block_gap();
                self.quote += 1;
                self.styles
                    .push(Style::new().fg(p.subtext0).add_modifier(Modifier::ITALIC));
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.block_gap();
                if let CodeBlockKind::Fenced(lang) = kind {
                    let lang = lang.split_whitespace().next().unwrap_or("").to_string();
                    if !lang.is_empty() {
                        let label = format!(" {lang}");
                        let cells = label
                            .chars()
                            .map(|c| (clean(c), Style::new().fg(p.overlay0).bg(p.surface0)))
                            .collect();
                        self.emit_code_line(cells);
                    }
                }
                self.code = Some(String::new());
            }
            Tag::List(start) => {
                self.flush();
                if self.lists.is_empty() {
                    self.block_gap();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                self.marker_width = marker.chars().count();
                self.marker = Some(
                    marker
                        .chars()
                        .map(|c| (c, Style::new().fg(p.accent).add_modifier(Modifier::BOLD)))
                        .collect(),
                );
            }
            Tag::Emphasis => self
                .styles
                .push(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.styles.push(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .styles
                .push(Style::new().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { .. } => self
                .styles
                .push(Style::new().fg(p.blue).add_modifier(Modifier::UNDERLINED)),
            Tag::Image { .. } => {
                self.push_text("[image: ", Style::new().fg(p.overlay0));
                self.styles.push(Style::new().fg(p.overlay0));
            }
            Tag::Table(_) => {
                self.flush();
                self.block_gap();
            }
            Tag::TableHead => {
                self.row_cells = 0;
                self.styles.push(Style::new().add_modifier(Modifier::BOLD));
            }
            Tag::TableRow => self.row_cells = 0,
            Tag::TableCell => {
                if self.row_cells > 0 {
                    self.push_text(" │ ", Style::new().fg(p.surface1));
                }
                self.row_cells += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        let p = self.p;
        match tag {
            TagEnd::Paragraph | TagEnd::HtmlBlock => {
                self.flush();
                self.gap = true;
            }
            TagEnd::Heading(_) => {
                self.flush();
                self.styles.pop();
                self.gap = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.styles.pop();
                self.quote = self.quote.saturating_sub(1);
                self.gap = true;
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                let code = code.strip_suffix('\n').unwrap_or(&code);
                let style = Style::new().fg(p.text).bg(p.surface0);
                for line in code.split('\n') {
                    let cells = std::iter::once(' ')
                        .chain(line.chars())
                        .map(|c| (clean(c), style))
                        .collect();
                    self.emit_code_line(cells);
                }
                self.gap = true;
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.gap = true;
                }
            }
            TagEnd::Item => {
                self.flush();
                self.marker = None;
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.styles.pop();
            }
            TagEnd::TableHead => {
                self.flush();
                self.styles.pop();
                let rule = vec![('─', Style::new().fg(p.surface1)); self.content_width()];
                self.emit(rule, false);
            }
            TagEnd::Image => {
                self.styles.pop();
                self.push_text("]", Style::new().fg(p.overlay0));
            }
            TagEnd::TableRow => self.flush(),
            TagEnd::Table => {
                self.flush();
                self.gap = true;
            }
            _ => {}
        }
    }

    fn block_gap(&mut self) {
        if self.gap && !self.out.is_empty() {
            self.out.push(Vec::new());
        }
        self.gap = false;
    }

    /// Quote bars and list indentation for the next line.
    fn prefix(&mut self, first: bool) -> Vec<Cell> {
        let p = self.p;
        let mut cells = Vec::new();
        for _ in 0..self.quote.min(MAX_DEPTH) {
            cells.push(('▎', Style::new().fg(p.overlay0)));
            cells.push((' ', self.base));
        }
        if !self.lists.is_empty() {
            let indent = 2 * (self.lists.len() - 1).min(MAX_DEPTH);
            cells.extend(std::iter::repeat_n((' ', self.base), indent));
            match (first, self.marker.take()) {
                (true, Some(marker)) => cells.extend(marker),
                (_, marker) => {
                    self.marker = marker;
                    cells.extend(std::iter::repeat_n((' ', self.base), self.marker_width));
                }
            }
        }
        // Never let indentation consume the whole line.
        cells.truncate(self.width / 2);
        cells
    }

    fn content_width(&self) -> usize {
        let quote = 2 * self.quote.min(MAX_DEPTH);
        let list = if self.lists.is_empty() {
            0
        } else {
            2 * (self.lists.len() - 1).min(MAX_DEPTH) + self.marker_width
        };
        self.width
            .saturating_sub((quote + list).min(self.width / 2))
            .max(1)
    }

    fn emit(&mut self, cells: Vec<Cell>, first: bool) {
        let mut line = self.prefix(first);
        line.extend(cells);
        self.out.push(line);
    }

    /// A code line, hard-wrapped and padded so its background is a block.
    fn emit_code_line(&mut self, cells: Vec<Cell>) {
        let width = self.content_width();
        let style = cells.first().map_or(self.base, |c| c.1);
        for mut chunk in hard_wrap(&cells, width) {
            let used: usize = chunk.iter().map(|c| cell_width(c.0)).sum();
            chunk.extend(std::iter::repeat_n(
                (' ', style),
                width.saturating_sub(used),
            ));
            self.emit(chunk, false);
        }
    }

    fn flush(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        let inline = std::mem::take(&mut self.inline);
        let width = self.content_width();
        for (i, line) in wrap(&inline, width).into_iter().enumerate() {
            self.emit(line, i == 0);
        }
    }
}

fn cell_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Splits cells into chunks of at most `width` columns, ignoring words.
fn hard_wrap(cells: &[Cell], width: usize) -> Vec<Vec<Cell>> {
    let mut out = vec![Vec::new()];
    let mut used = 0;
    for &cell in cells {
        let w = cell_width(cell.0);
        if used + w > width && used > 0 {
            out.push(Vec::new());
            used = 0;
        }
        out.last_mut().expect("never empty").push(cell);
        used += w;
    }
    out
}

/// Word-wraps styled cells to `width` columns; '\n' forces a break and
/// words wider than a line are split.
pub(crate) fn wrap(cells: &[Cell], width: usize) -> Vec<Vec<Cell>> {
    let width = width.max(1);
    let mut out = Vec::new();
    for paragraph in cells.split(|c| c.0 == '\n') {
        let mut line: Vec<Cell> = Vec::new();
        let mut used = 0;
        for word in paragraph.split(|c| c.0 == ' ').filter(|w| !w.is_empty()) {
            let w: usize = word.iter().map(|c| cell_width(c.0)).sum();
            if used > 0 && used + 1 + w > width {
                out.push(std::mem::take(&mut line));
                used = 0;
            }
            if w > width {
                for chunk in hard_wrap(word, width) {
                    if used > 0 {
                        out.push(std::mem::take(&mut line));
                    }
                    used = chunk.iter().map(|c| cell_width(c.0)).sum();
                    line = chunk;
                }
                continue;
            }
            if used > 0 {
                line.push((' ', joining_style(line.last(), word.first())));
                used += 1;
            }
            line.extend_from_slice(word);
            used += w;
        }
        out.push(line);
    }
    out
}

/// Style for the space between two words: continuous inside one styled run
/// (a multi-word link or code span), otherwise without the background and
/// line decorations that would make a styled word look one cell wider.
fn joining_style(before: Option<&Cell>, after: Option<&Cell>) -> Style {
    let before = before.map_or(Style::new(), |c| c.1);
    if Some(before) == after.map(|c| c.1) {
        return before;
    }
    let mut style = before.remove_modifier(Modifier::UNDERLINED | Modifier::CROSSED_OUT);
    style.bg = None;
    style
}

/// Merges runs of equally styled cells into spans. Every character is
/// cleaned again here, so nothing can bypass sanitization.
pub(crate) fn to_line(cells: &[Cell]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut style = None;
    for &(c, s) in cells {
        if style != Some(s) {
            if let Some(prev) = style {
                spans.push(Span::styled(std::mem::take(&mut run), prev));
            }
            style = Some(s);
        }
        run.push(clean(c));
    }
    if let Some(prev) = style {
        spans.push(Span::styled(run, prev));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Vec<Cell>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                l.iter()
                    .map(|c| c.0)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn styled<'a>(lines: &'a [Vec<Cell>], needle: &str) -> &'a Cell {
        for line in lines {
            let s: String = line.iter().map(|c| c.0).collect();
            if let Some(byte) = s.find(needle) {
                return &line[s[..byte].chars().count()];
            }
        }
        panic!("{needle:?} not rendered in {:?}", text(lines));
    }

    #[test]
    fn plain_strips_syntax_noise() {
        assert_eq!(
            plain("## Plan\n**bold** and _it_ with `code` and [link](http://x)\n- item"),
            "Plan bold and it with code and link item"
        );
        assert_eq!(
            plain("… the **portfolio** rule …"),
            "… the portfolio rule …"
        );
        assert_eq!(plain("\u{1b}[31mred\u{7}"), "[31mred");
    }

    #[test]
    fn renders_block_and_inline_elements() {
        let p = Palette::terminal();
        let md = "# Title\n\nSome **bold**, *italic*, ~~gone~~ and `code` here.\n\n> quoted\n\n- one\n- two\n  1. nested\n\n```rust\nfn main() {}\n```\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done";
        let lines = render(md, 40, Style::new(), &p);
        let t = text(&lines);
        assert_eq!(t[0], "Title");
        assert!(styled(&lines, "Title")
            .1
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(
            t.contains(&"Some bold, italic, gone and code here.".to_string()),
            "{t:?}"
        );
        assert!(styled(&lines, "bold")
            .1
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(styled(&lines, "italic")
            .1
            .add_modifier
            .contains(Modifier::ITALIC));
        assert!(styled(&lines, "gone")
            .1
            .add_modifier
            .contains(Modifier::CROSSED_OUT));
        assert_eq!(styled(&lines, "code ").1.bg, Some(p.surface0));
        assert_eq!(
            styled(&lines, " here").1.bg,
            None,
            "no background bleeds past code"
        );
        assert!(t.contains(&"▎ quoted".to_string()), "{t:?}");
        assert!(t.contains(&"• one".to_string()) && t.contains(&"• two".to_string()));
        assert!(t.contains(&"  1. nested".to_string()), "{t:?}");
        assert!(t.contains(&" rust".to_string()));
        let code = lines
            .iter()
            .find(|l| {
                l.iter()
                    .map(|c| c.0)
                    .collect::<String>()
                    .contains("fn main")
            })
            .unwrap();
        assert_eq!(code.len(), 40, "code block is padded to a solid block");
        assert!(code.iter().all(|c| c.1.bg == Some(p.surface0)));
        assert!(t
            .iter()
            .any(|l| l.chars().all(|c| c == '─') && !l.is_empty()));
        assert!(t.contains(&"a │ b".to_string()) && t.contains(&"1 │ 2".to_string()));
        assert!(t.contains(&"• ☑ done".to_string()), "{t:?}");
        assert!(lines
            .iter()
            .all(|l| l.iter().map(|c| cell_width(c.0)).sum::<usize>() <= 40));
    }

    #[test]
    fn raw_html_and_links_are_shown_not_followed() {
        let p = Palette::terminal();
        let lines = render(
            "<script>alert(1)</script>\n\nsee [docs](javascript:evil) <b>x</b>",
            60,
            Style::new(),
            &p,
        );
        let t = text(&lines).join("\n");
        assert!(t.contains("<script>alert(1)</script>"));
        assert!(t.contains("see docs <b>x</b>"));
        assert!(!t.contains("javascript"));
    }

    #[test]
    fn hostile_input_is_neutralized_before_and_after_parsing() {
        let p = Palette::terminal();
        let hostile = "# \u{1b}[2J head\n\n```\n\u{1b}]0;x\u{7}\t雪雪雪雪雪雪雪雪雪雪 e\u{301}\u{301}\n```\n\n> \u{202e}rtl \u{9b}31m\r\n\n".repeat(20)
            + &">".repeat(500)
            + " deep\n\n"
            + &"- ".repeat(300)
            + "list";
        for width in [1, 3, 10, 40] {
            let lines = render(&hostile, width, Style::new(), &p);
            for line in &lines {
                assert!(line.iter().map(|c| cell_width(c.0)).sum::<usize>() <= width.max(2));
                assert!(line.iter().all(|c| !c.0.is_control() && c.0 != '\u{202e}'));
                let rendered = to_line(line);
                assert!(rendered
                    .spans
                    .iter()
                    .all(|s| !s.content.chars().any(char::is_control)));
            }
        }
        assert!(!plain(&hostile)
            .chars()
            .any(|c| c.is_control() || c == '\u{202e}'));
    }
}
