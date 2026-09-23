//! Layout and drawing. Everything here is a pure function of the state and
//! palette, so it renders identically to a `TestBackend` and a terminal.
use crate::{
    text::{self, matches, query_terms, safe, width},
    theme::Palette,
    BrowserState, Integration, Mode, RoleFilter, RESULT_LIMIT,
};
use agent_history_core::{index::IndexProgress, Agent, EventKind, SearchResult};
use ratatui::{
    layout::{Constraint, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, LineGauge, Paragraph},
    Frame,
};
use std::time::Duration;

/// Below this width the results and preview panes stack vertically.
pub(crate) const SIDE_BY_SIDE_MIN_WIDTH: u16 = 80;
/// Below this body height a stacked layout shows only the focused pane.
const STACKED_MIN_HEIGHT: u16 = 14;
/// Rows per result: header, context, two snippet lines, separator.
const ITEM_HEIGHT: usize = 5;
const SNIPPET_LINES: usize = 2;

pub fn draw(frame: &mut Frame, state: &mut BrowserState, integration: &impl Integration) {
    let p = integration.palette();
    let area = frame.area();
    frame.render_widget(Block::new().style(p.base()), area);
    if area.width < 24 || area.height < 10 {
        let lines = text::wrap("Agent History — enlarge this pane", area.width.into());
        frame.render_widget(
            Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>()).style(p.base()),
            area,
        );
        return;
    }
    let message_height = u16::from(state.error.is_some());
    let [header, search, filters, body, message, keys] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(message_height),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, header, state, integration.title(), &p);
    draw_search(frame, search, state, &p);
    draw_filters(frame, filters, state, &p);

    let (results_area, preview_area) = split_body(body, state.mode);
    if let Some(r) = results_area {
        draw_results(frame, r, state, &p);
    }
    if let Some(r) = preview_area {
        draw_preview(frame, r, state, &p);
    }
    if state.mode == Mode::Action {
        draw_action(frame, body, &integration.action_lines(), &p);
    }
    if let Some(error) = &state.error {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ✗ ", p.error()),
                Span::styled(
                    safe(error, message.width.saturating_sub(3).into()),
                    p.error(),
                ),
            ])),
            message,
        );
    }
    draw_keys(frame, keys, state.mode, integration, &p);
}

/// Side by side when wide enough, stacked when tall enough, otherwise only
/// the pane that has focus.
pub(crate) fn split_body(body: Rect, mode: Mode) -> (Option<Rect>, Option<Rect>) {
    if body.width >= SIDE_BY_SIDE_MIN_WIDTH {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
                .areas(body);
        (Some(left), Some(right))
    } else if body.height >= STACKED_MIN_HEIGHT {
        let [top, bottom] =
            Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body);
        (Some(top), Some(bottom))
    } else if mode == Mode::Preview {
        (None, Some(body))
    } else {
        (Some(body), None)
    }
}

fn pane<'a>(title: &'a str, focused: bool, p: &Palette) -> Block<'a> {
    Block::bordered()
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(p.border(focused))
        .title(Span::styled(format!(" {title} "), p.pane_title(focused)))
}

/// Sanitizes styled spans and truncates them to `width` columns.
fn clip(spans: Vec<Span<'static>>, width: usize) -> (Vec<Span<'static>>, usize) {
    let mut out = Vec::with_capacity(spans.len() + 1);
    let mut used = 0;
    for span in spans {
        let content = safe(&span.content, width - used);
        used += width_of(&content);
        out.push(Span::styled(content, span.style));
        if used >= width {
            break;
        }
    }
    (out, used)
}

/// `clip`, then pads with spaces so a background style covers the row.
fn fit(spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let (mut out, used) = clip(spans, width);
    if used < width {
        out.push(Span::raw(" ".repeat(width - used)));
    }
    Line::from(out)
}

fn width_of(s: &str) -> usize {
    width(s)
}

fn draw_header(frame: &mut Frame, area: Rect, state: &BrowserState, title: &str, p: &Palette) {
    let title = safe(title, area.width.saturating_sub(2).into());
    let room = usize::from(area.width).saturating_sub(width_of(&title) + 4);
    let status = safe(&state.status, room);
    let gap = usize::from(area.width).saturating_sub(width_of(&title) + width_of(&status) + 2);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {title}"),
                Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(gap)),
            Span::styled(status, p.muted()),
            Span::raw(" "),
        ])),
        area,
    );
}

fn draw_search(frame: &mut Frame, area: Rect, state: &BrowserState, p: &Palette) {
    let focused = state.mode == Mode::Query;
    let block = pane("Search", focused, p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let prompt = Span::styled("› ", p.key());
    let room = usize::from(inner.width).saturating_sub(3);
    let line = if state.query.is_empty() {
        let hint = safe("Type words you remember · \"exact phrase\" · prefix*", room);
        Line::from(vec![prompt, Span::styled(hint, p.muted())])
    } else {
        // Keep the end of a long query, where the cursor is, in view.
        let clean: String = state.query.chars().map(text::clean).collect();
        let mut start = 0;
        while width_of(&clean[start..]) > room {
            start += clean[start..].chars().next().map_or(1, char::len_utf8);
        }
        Line::from(vec![
            prompt,
            Span::styled(clean[start..].to_string(), Style::new().fg(p.text)),
        ])
    };
    let typed = if state.query.is_empty() {
        0
    } else {
        width_of(&line.spans[1].content)
    };
    frame.render_widget(Paragraph::new(line), inner);
    if focused {
        let x = inner.x.saturating_add(2 + typed as u16);
        frame.set_cursor_position(Position::new(
            x.min(inner.right().saturating_sub(1)),
            inner.y,
        ));
    }
}

fn draw_filters(frame: &mut Frame, area: Rect, state: &BrowserState, p: &Palette) {
    let mut spans = vec![Span::raw(" ")];
    for filter in RoleFilter::ALL {
        let style = if filter == state.role_filter {
            p.active_tab()
        } else {
            p.inactive_tab()
        };
        spans.push(Span::styled(format!(" {} ", filter.label()), style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled("F2", p.key()));
    let count = match state.results.len() {
        0 if state.query.trim().is_empty() => String::new(),
        n if n >= RESULT_LIMIT => format!("top {n} results "),
        1 => "1 result ".into(),
        n => format!("{n} results "),
    };
    let used: usize = spans.iter().map(|s| width_of(&s.content)).sum();
    let gap = usize::from(area.width).saturating_sub(used + width_of(&count));
    spans.push(Span::raw(" ".repeat(gap)));
    spans.push(Span::styled(count, p.muted()));
    frame.render_widget(Paragraph::new(fit(spans, area.width.into())), area);
}

fn agent_span(agent: Agent, p: &Palette) -> Span<'static> {
    let (name, color) = match agent {
        Agent::Claude => ("Claude", p.peach),
        Agent::Codex => ("Codex", p.teal),
    };
    Span::styled(name, Style::new().fg(color).add_modifier(Modifier::BOLD))
}

fn role_span(kind: EventKind, p: &Palette) -> Span<'static> {
    let (name, color) = role(kind, p);
    Span::styled(name, Style::new().fg(color).add_modifier(Modifier::BOLD))
}

fn role(kind: EventKind, p: &Palette) -> (&'static str, ratatui::style::Color) {
    match kind {
        EventKind::User => ("USER", p.blue),
        EventKind::Assistant => ("ASSISTANT", p.green),
        EventKind::ToolResult => ("TOOL", p.overlay0),
    }
}

fn date(r: &SearchResult) -> String {
    r.timestamp
        .map(|t| {
            let d: chrono::DateTime<chrono::Utc> = t.into();
            d.format("%Y-%m-%d").to_string()
        })
        .unwrap_or_else(|| "unknown date".into())
}

fn context_spans(r: &SearchResult, p: &Palette) -> Vec<Span<'static>> {
    let repo = r
        .repository
        .as_deref()
        .map(|a| a.rsplit('/').next().unwrap_or(a).to_string());
    match (repo, r.branch.clone()) {
        (Some(repo), Some(branch)) => vec![
            Span::styled(repo, Style::new().fg(p.text)),
            Span::styled(" / ", p.muted()),
            Span::styled(branch, Style::new().fg(p.mauve)),
        ],
        (Some(repo), None) => vec![Span::styled(repo, Style::new().fg(p.text))],
        (None, Some(branch)) => vec![Span::styled(branch, Style::new().fg(p.mauve))],
        (None, None) => vec![Span::styled("no repository recorded", p.muted())],
    }
}

/// Splits `line` into plain and matched spans.
fn highlighted(line: &str, terms: &[String], base: Style, p: &Palette) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut at = 0;
    for (start, end) in matches(line, terms) {
        if start > at {
            out.push(Span::styled(line[at..start].to_string(), base));
        }
        out.push(Span::styled(line[start..end].to_string(), p.matched()));
        at = end;
    }
    if at < line.len() || out.is_empty() {
        out.push(Span::styled(line[at..].to_string(), base));
    }
    out
}

fn draw_results(frame: &mut Frame, area: Rect, state: &mut BrowserState, p: &Palette) {
    let focused = state.mode == Mode::Results;
    let mut block = pane("Results", focused, p);
    if !state.results.is_empty() {
        block = block.title_bottom(
            Line::styled(
                format!(" {}/{} ", state.selected + 1, state.results.len()),
                p.muted(),
            )
            .right_aligned(),
        );
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if state.results.is_empty() {
        draw_empty(frame, inner, state, p);
        return;
    }
    let w = usize::from(inner.width);
    let per_page = (usize::from(inner.height) + 1) / ITEM_HEIGHT;
    let per_page = per_page.max(1);
    if state.selected < state.list_offset {
        state.list_offset = state.selected;
    } else if state.selected >= state.list_offset + per_page {
        state.list_offset = state.selected + 1 - per_page;
    }
    state.list_offset = state.list_offset.min(state.results.len() - 1);
    let terms = query_terms(&state.query);
    let mut lines = Vec::new();
    for (i, r) in state
        .results
        .iter()
        .enumerate()
        .skip(state.list_offset)
        .take(per_page)
    {
        let selected = i == state.selected;
        let marker = if selected {
            Span::styled("▌", Style::new().fg(p.accent))
        } else {
            Span::raw(" ")
        };
        let d = date(r);
        let mut head = vec![
            marker.clone(),
            Span::raw(" "),
            agent_span(r.agent, p),
            Span::raw("  "),
            role_span(r.kind, p),
        ];
        let used: usize = head.iter().map(|s| width_of(&s.content)).sum();
        let gap = w.saturating_sub(used + width_of(&d) + 1).max(1);
        head.push(Span::raw(" ".repeat(gap)));
        head.push(Span::styled(d, p.muted()));
        let mut item = vec![fit(head, w)];
        let mut context = vec![marker.clone(), Span::raw("  ")];
        context.extend(context_spans(r, p));
        item.push(fit(context, w));
        let snippet = text::wrap(&r.snippet, w.saturating_sub(4).max(1));
        for k in 0..SNIPPET_LINES {
            let mut spans = vec![marker.clone(), Span::raw("  ")];
            // Snippet text is indented under the header's agent name.
            if let Some(line) = snippet.get(k) {
                spans.extend(highlighted(line, &terms, Style::new().fg(p.subtext0), p));
            }
            item.push(fit(spans, w));
        }
        if selected {
            let style = p.selection(focused);
            item = item.into_iter().map(|l| l.patch_style(style)).collect();
        }
        lines.extend(item);
        lines.push(Line::raw(""));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_empty(frame: &mut Frame, area: Rect, state: &BrowserState, p: &Palette) {
    let w = usize::from(area.width);
    let mut lines = vec![Line::raw("")];
    let push = |lines: &mut Vec<Line<'static>>, s: &str, style: Style| {
        for l in text::wrap(s, w.saturating_sub(2)) {
            lines.push(Line::from(vec![Span::raw(" "), Span::styled(l, style)]));
        }
    };
    if state.query.trim().is_empty() {
        push(&mut lines, "Search your conversations", p.pane_title(true));
        push(
            &mut lines,
            "Type words you remember from a Claude Code or Codex session.",
            p.muted(),
        );
    } else {
        push(&mut lines, "No matching conversations", p.error());
        push(
            &mut lines,
            &format!(
                "Nothing in {} messages matches “{}”.",
                state.role_filter.label().to_lowercase(),
                state.query.trim()
            ),
            Style::new().fg(p.text),
        );
        lines.push(Line::raw(""));
        push(
            &mut lines,
            "Try fewer words, a prefix such as portf*, or F2 to change the role filter.",
            p.muted(),
        );
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// A preview paragraph: either a role heading or a line of message text in
/// the current role.
fn preview_lines(preview: &str, width: usize, terms: &[String], p: &Palette) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut role_color = p.overlay0;
    let mut boundary = true;
    for raw in preview.split('\n') {
        let heading = boundary
            .then(|| {
                [
                    ("User: ", EventKind::User),
                    ("Assistant: ", EventKind::Assistant),
                    ("Tool: ", EventKind::ToolResult),
                ]
                .into_iter()
                .find_map(|(prefix, kind)| raw.strip_prefix(prefix).map(|rest| (kind, rest)))
            })
            .flatten();
        let body = match heading {
            Some((kind, rest)) => {
                let (name, color) = role(kind, p);
                role_color = color;
                out.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(name, Style::new().fg(color).add_modifier(Modifier::BOLD)),
                ]));
                rest
            }
            None => raw,
        };
        boundary = raw.is_empty();
        if raw.starts_with("[Surrounding context is limited]") {
            out.push(Line::styled(format!(" {raw}"), p.muted()));
            continue;
        }
        if body.is_empty() && heading.is_none() {
            out.push(Line::raw(""));
            continue;
        }
        for line in text::wrap(body, width.saturating_sub(4).max(1)) {
            let mut spans = vec![Span::styled(" ▎ ", Style::new().fg(role_color))];
            spans.extend(highlighted(&line, terms, Style::new().fg(p.text), p));
            out.push(Line::from(spans));
        }
    }
    out
}

fn draw_preview(frame: &mut Frame, area: Rect, state: &mut BrowserState, p: &Palette) {
    let focused = state.mode == Mode::Preview;
    let mut block = pane("Preview", focused, p);
    if let Some(r) = state.selected_result() {
        let mut spans = vec![
            Span::raw(" "),
            agent_span(r.agent, p),
            Span::styled(" · ", p.muted()),
        ];
        spans.extend(context_spans(r, p));
        spans.push(Span::styled(format!(" · {} ", date(r)), p.muted()));
        // Leave room for the left title and the corners.
        let (spans, _) = clip(spans, usize::from(area.width).saturating_sub(15));
        block = block.title(Line::from(spans).right_aligned());
    }
    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        frame.render_widget(block, area);
        return;
    }
    let w = usize::from(inner.width);
    let lines: Vec<Line<'static>> = if state.selected_result().is_none() {
        vec![
            Line::raw(""),
            Line::styled(
                " The conversation around the selected result appears here.",
                p.muted(),
            ),
        ]
    } else if let Some(error) = &state.preview_error {
        let mut lines = vec![
            Line::raw(""),
            Line::styled(" Preview unavailable", p.error()),
        ];
        lines.extend(
            text::wrap(error, w.saturating_sub(2))
                .into_iter()
                .map(|l| Line::styled(format!(" {l}"), Style::new().fg(p.text))),
        );
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " Reopen Agent History to re-index changed files.",
            p.muted(),
        ));
        lines
    } else {
        preview_lines(&state.preview, w, &query_terms(&state.query), p)
    };
    let visible = usize::from(inner.height);
    if state.preview_anchor {
        state.preview_anchor = false;
        let terms = query_terms(&state.query);
        if let Some(i) = lines.iter().position(|l| {
            l.spans
                .iter()
                .any(|s| !matches(&s.content, &terms).is_empty())
        }) {
            state.preview_scroll = i.saturating_sub(2);
        }
    }
    let max_scroll = lines.len().saturating_sub(visible);
    state.preview_scroll = state.preview_scroll.min(max_scroll);
    if lines.len() > visible {
        let end = (state.preview_scroll + visible).min(lines.len());
        block = block.title_bottom(
            Line::styled(
                format!(" {}–{} of {} ", state.preview_scroll + 1, end, lines.len()),
                p.muted(),
            )
            .right_aligned(),
        );
    }
    frame.render_widget(block, area);
    let shown: Vec<Line<'static>> = lines
        .into_iter()
        .skip(state.preview_scroll)
        .take(visible)
        .collect();
    frame.render_widget(Paragraph::new(shown), inner);
}

fn draw_action(frame: &mut Frame, body: Rect, action_lines: &[String], p: &Palette) {
    let width = body.width.min(76);
    let inner_w = usize::from(width.saturating_sub(4)).max(1);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, l) in action_lines.iter().enumerate() {
        for w in text::wrap(l, inner_w) {
            let style = if i == 0 {
                Style::new().fg(p.peach).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(p.text)
            };
            lines.push(Line::styled(format!(" {w}"), style));
        }
    }
    let height = (lines.len() as u16 + 2).min(body.height);
    let area = Rect {
        x: body.x + (body.width - width) / 2,
        y: body.y + (body.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .border_style(Style::new().fg(p.peach))
        .title(Span::styled(
            " Recovery ",
            Style::new().fg(p.peach).add_modifier(Modifier::BOLD),
        ))
        .style(p.base());
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Key hints valid for the focused pane only.
pub(crate) fn hints(mode: Mode, integration: &impl Integration) -> Vec<(&'static str, String)> {
    let enter = integration.enter_label().to_lowercase();
    let open = |v: &mut Vec<(&'static str, String)>| {
        if enter == "preview" {
            v.push(("␣/⏎", "preview".into()));
        } else {
            v.push(("␣", "preview".into()));
            v.push(("⏎", enter.clone()));
        }
    };
    let mut v = Vec::new();
    match mode {
        Mode::Query => {
            v.push(("↓/tab", "results".into()));
            open(&mut v);
            v.push(("F2", "role".into()));
            v.push(("esc", "quit".into()));
        }
        Mode::Results => {
            v.push(("↑↓", "move".into()));
            open(&mut v);
            v.push(("F2", "role".into()));
            v.push(("tab", "preview".into()));
            v.push(("esc", "search".into()));
        }
        Mode::Preview => {
            v.push(("↑↓/pgup/pgdn", "scroll".into()));
            if let Some(label) = integration.preview_enter_label() {
                v.push(("⏎", label.to_lowercase()));
            }
            v.push(("F2", "role".into()));
            v.push(("tab", "search".into()));
            v.push(("esc", "results".into()));
        }
        Mode::Action => v.push(("esc", "cancel".into())),
    }
    if mode != Mode::Query {
        v.push(("^C", "quit".into()));
    }
    v
}

fn draw_keys(
    frame: &mut Frame,
    area: Rect,
    mode: Mode,
    integration: &impl Integration,
    p: &Palette,
) {
    let mut spans = vec![Span::raw(" ")];
    for (key, label) in hints(mode, integration) {
        spans.push(Span::styled(key, p.key()));
        spans.push(Span::styled(
            format!(" {label}   "),
            Style::new().fg(p.overlay1),
        ));
    }
    frame.render_widget(
        Paragraph::new(fit(spans, area.width.into())).style(Style::new().bg(p.surface_dim)),
        area,
    );
}

pub(crate) fn draw_progress(
    frame: &mut Frame,
    p: &Palette,
    title: &str,
    tick: usize,
    elapsed: Duration,
    progress: Option<&IndexProgress>,
) {
    let area = frame.area();
    frame.render_widget(Block::new().style(p.base()), area);
    let width = area.width.min(64);
    let height = area.height.min(7);
    if width < 8 || height < 3 {
        return;
    }
    let box_area = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(p.border(true))
        .title(Span::styled(
            format!(" {} ", safe(title, width.saturating_sub(4).into())),
            p.pane_title(true),
        ));
    let inner = block.inner(box_area);
    frame.render_widget(block, box_area);
    let w = usize::from(inner.width);
    let spinner = ['◐', '◓', '◑', '◒'][tick % 4];
    let heading = format!("{spinner} Indexing conversations");
    let secs = format!("{:.1}s", elapsed.as_secs_f32());
    let gap = w.saturating_sub(width_of(&heading) + width_of(&secs) + 2);
    let mut lines = vec![
        fit(
            vec![
                Span::raw(" "),
                Span::styled(heading, p.key()),
                Span::raw(" ".repeat(gap)),
                Span::styled(secs, p.muted()),
            ],
            w,
        ),
        // Row 1 is drawn over by the gauge.
        Line::raw(""),
    ];
    let stats = match progress {
        Some(pr) => format!(
            " {} MB read · {} records · {} chunks · {} failed",
            pr.bytes_read / (1024 * 1024),
            pr.records,
            pr.chunks,
            pr.failed_files
        ),
        None => " Preparing conversation index…".into(),
    };
    lines.push(fit(vec![Span::styled(stats, p.muted())], w));
    lines.push(Line::raw(""));
    lines.push(Line::styled(" Ctrl-C cancels", p.muted()));
    frame.render_widget(Paragraph::new(lines), inner);
    if let (Some(pr), true) = (progress, inner.height > 1) {
        let (agent, color) = match pr.agent {
            Agent::Claude => ("Claude", p.peach),
            Agent::Codex => ("Codex", p.teal),
        };
        let ratio = if pr.agent_total_files == 0 {
            0.0
        } else {
            (pr.agent_completed_files as f64 / pr.agent_total_files as f64).clamp(0.0, 1.0)
        };
        let row = Rect {
            x: inner.x + 1,
            y: inner.y + 1,
            width: inner.width.saturating_sub(2),
            height: 1,
        };
        frame.render_widget(
            LineGauge::default()
                .ratio(ratio)
                .label(Span::styled(
                    format!(
                        "{agent} {}/{} files ",
                        pr.agent_completed_files, pr.agent_total_files
                    ),
                    Style::new().fg(color).add_modifier(Modifier::BOLD),
                ))
                .filled_style(Style::new().fg(color))
                .unfilled_style(Style::new().fg(p.surface1)),
            row,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tests::rememberable, Key, Standalone};
    use agent_history_core::{test_support::TempDir, SqliteStore};
    use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};

    struct Themed(Palette);
    impl Integration for Themed {
        fn title(&self) -> &str {
            "Agent History — Test"
        }
        fn enter_label(&self) -> &str {
            "Resume"
        }
        fn preview_enter_label(&self) -> Option<&str> {
            Some("Resume")
        }
        fn palette(&self) -> Palette {
            self.0
        }
        fn handle(
            &mut self,
            _: Key,
            _: &mut BrowserState,
            _: &SqliteStore,
        ) -> agent_history_core::Result<bool> {
            Ok(false)
        }
        fn action_lines(&self) -> Vec<String> {
            vec![
                "The recorded workspace is unavailable.".into(),
                "c/Esc — Cancel".into(),
            ]
        }
    }

    fn render(state: &mut BrowserState, integration: &impl Integration, w: u16, h: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, state, integration)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    /// First cell whose row text contains `needle`, as (x, y) of the match.
    fn find(buf: &Buffer, needle: &str) -> Option<(u16, u16)> {
        (0..buf.area.height).find_map(|y| {
            let mut x = 0;
            let cells: Vec<&str> = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
            while (x as usize) < cells.len() {
                let rest: String = cells[x as usize..].concat();
                if rest.starts_with(needle) {
                    return Some((x, y));
                }
                x += 1;
            }
            None
        })
    }

    fn searched(temp: &TempDir) -> (SqliteStore, BrowserState) {
        let store = rememberable(temp.path());
        let mut state = BrowserState {
            query: "rememberable".into(),
            status: "1 files · 2 chunks".into(),
            ..Default::default()
        };
        state.refresh(&store);
        assert_eq!(state.results.len(), 2);
        (store, state)
    }

    #[test]
    fn selected_result_has_a_contrasting_background() {
        let temp = TempDir::new("ui-select").unwrap();
        let (_store, mut state) = searched(&temp);
        state.mode = Mode::Results;
        let buf = render(&mut state, &Standalone, 100, 30);
        let first = find(&buf, "Claude  USER").expect("first result header");
        let second = find(&buf, "Claude  ASSISTANT").expect("second result header");
        assert_eq!(buf[(first.0 + 2, first.1)].bg, Color::DarkGray);
        assert_eq!(buf[(second.0 + 2, second.1)].bg, Color::Reset);
        assert_eq!(buf[(first.0 - 2, first.1)].symbol(), "▌");
        // The whole row is covered, not only the text.
        assert_eq!(buf[(first.0 + 30, first.1)].bg, Color::DarkGray);

        state.selected = 1;
        let buf = render(&mut state, &Standalone, 100, 30);
        assert_eq!(buf[(first.0 + 2, first.1)].bg, Color::Reset);
        assert_eq!(buf[(second.0 + 2, second.1)].bg, Color::DarkGray);
    }

    #[test]
    fn focused_pane_is_distinguishable() {
        let temp = TempDir::new("ui-focus").unwrap();
        let (_store, mut state) = searched(&temp);
        let accent = Color::Rgb(1, 2, 3);
        let themed = Themed(Palette {
            accent,
            ..Palette::terminal()
        });
        let corner = |buf: &Buffer, title: &str| {
            let (x, y) = find(buf, &format!(" {title} ")).unwrap();
            buf[(x - 1, y)].clone()
        };
        for (mode, focused) in [
            (Mode::Query, "Search"),
            (Mode::Results, "Results"),
            (Mode::Preview, "Preview"),
        ] {
            state.mode = mode;
            let buf = render(&mut state, &themed, 100, 30);
            for title in ["Search", "Results", "Preview"] {
                let cell = corner(&buf, title);
                if title == focused {
                    assert_eq!(cell.symbol(), "┏", "{title} focused in {mode:?}");
                    assert_eq!(cell.fg, accent);
                } else {
                    assert_eq!(cell.symbol(), "╭", "{title} unfocused in {mode:?}");
                    assert_ne!(cell.fg, accent);
                }
            }
        }
    }

    #[test]
    fn query_box_places_the_cursor_after_the_query() {
        let temp = TempDir::new("ui-cursor").unwrap();
        let (_store, mut state) = searched(&temp);
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &mut state, &Standalone)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let (x, y) = find(&buf, "› rememberable").unwrap();
        terminal
            .backend_mut()
            .assert_cursor_position((x + 2 + 12, y));
    }

    #[test]
    fn active_role_tab_is_highlighted() {
        let temp = TempDir::new("ui-tabs").unwrap();
        let (_store, mut state) = searched(&temp);
        for active in RoleFilter::ALL {
            state.role_filter = active;
            let buf = render(&mut state, &Standalone, 100, 30);
            for filter in RoleFilter::ALL {
                let (x, y) = find(&buf, &format!(" {} ", filter.label())).unwrap();
                let bg = buf[(x + 1, y)].bg;
                if filter == active {
                    assert_eq!(bg, Color::Blue, "{filter:?} active");
                } else {
                    assert_eq!(bg, Color::Reset, "{filter:?} inactive");
                }
            }
        }
    }

    #[test]
    fn narrow_panes_stack_and_no_size_panics() {
        let temp = TempDir::new("ui-narrow").unwrap();
        let (_store, mut state) = searched(&temp);
        state.mode = Mode::Results;
        let buf = render(&mut state, &Standalone, 60, 40);
        let (rx, ry) = find(&buf, " Results ").unwrap();
        let (px, py) = find(&buf, " Preview ").unwrap();
        assert_eq!(rx, px, "stacked panes share a left edge");
        assert!(py > ry, "preview sits below results");

        let buf = render(&mut state, &Standalone, 120, 40);
        let (rx, ry) = find(&buf, " Results ").unwrap();
        let (px, py) = find(&buf, " Preview ").unwrap();
        assert_eq!(ry, py, "wide panes sit side by side");
        assert!(px > rx);

        for mode in [Mode::Query, Mode::Results, Mode::Preview, Mode::Action] {
            state.mode = mode;
            for w in (0..=130).step_by(7) {
                for h in (0..=45).step_by(4) {
                    render(&mut state, &Themed(Palette::terminal()), w, h);
                }
            }
        }
    }

    #[test]
    fn empty_states_are_explained() {
        let temp = TempDir::new("ui-empty").unwrap();
        let (store, mut state) = searched(&temp);
        state.query = "nonexistent".into();
        state.refresh(&store);
        assert!(state.results.is_empty());
        let buf = render(&mut state, &Standalone, 100, 30);
        assert!(find(&buf, "No matching conversations").is_some());
        assert!(find(&buf, "“nonexistent”").is_some());
        assert!(find(&buf, "The conversation around the selected result").is_some());

        state.query.clear();
        state.refresh(&store);
        let buf = render(&mut state, &Standalone, 100, 30);
        assert!(find(&buf, "Search your conversations").is_some());
    }

    #[test]
    fn hostile_text_cannot_corrupt_the_frame() {
        let temp = TempDir::new("ui-hostile").unwrap();
        let (_store, mut state) = searched(&temp);
        let hostile = "\u{1b}[2J\u{1b}]0;title\u{7}\r\t雪雪雪雪雪雪 e\u{301}\u{301}\u{301} \u{202e}evil \u{0}\u{9b}31m "
            .repeat(40);
        state.results[0].snippet = hostile.clone();
        state.results[0].branch = Some(hostile.clone());
        state.preview = format!("User: {hostile}\n\nAssistant: {hostile}");
        state.preview_anchor = true;
        state.status = hostile.clone();
        state.query = hostile.clone();
        state.error = Some(hostile);
        for (w, h) in [(100, 30), (61, 40), (37, 12)] {
            for mode in [Mode::Query, Mode::Results, Mode::Preview] {
                state.mode = mode;
                let buf = render(&mut state, &Standalone, w, h);
                for y in 0..h {
                    for x in 0..w {
                        let symbol = buf[(x, y)].symbol();
                        assert!(
                            !symbol.chars().any(|c| c.is_control() || c == '\u{202e}'),
                            "control character at {x},{y}: {symbol:?}"
                        );
                    }
                }
                if w >= SIDE_BY_SIDE_MIN_WIDTH {
                    // Every body row still ends in the preview's right border.
                    let (_, top) = find(&buf, " Results ").unwrap();
                    for y in top + 1..h - 3 {
                        let edge = buf[(w - 1, y)].symbol();
                        assert!(matches!(edge, "│" | "┃"), "row {y}: {:?}", row(&buf, y));
                    }
                }
            }
        }
    }

    #[test]
    fn matched_terms_and_roles_are_colored() {
        let temp = TempDir::new("ui-match").unwrap();
        let (_store, mut state) = searched(&temp);
        let buf = render(&mut state, &Standalone, 100, 30);
        let (x, y) = find(&buf, "rememberable topic").unwrap();
        assert_eq!(buf[(x, y)].bg, Color::Yellow, "matched term");
        assert_eq!(buf[(x + 13, y)].bg, Color::Reset, "unmatched word");
        let (ux, uy) = find(&buf, "USER").unwrap();
        let (ax, ay) = find(&buf, "ASSISTANT").unwrap();
        assert_eq!(buf[(ux, uy)].fg, Color::Blue);
        assert_eq!(buf[(ax, ay)].fg, Color::Green);
    }

    #[test]
    fn key_hints_match_the_focused_pane_and_integration() {
        let keys = |mode, i: &dyn Fn(Mode) -> Vec<(&'static str, String)>| {
            i(mode).into_iter().map(|(k, _)| k).collect::<Vec<_>>()
        };
        let standalone = |m| hints(m, &Standalone);
        let herdr = |m| hints(m, &Themed(Palette::terminal()));
        assert!(keys(Mode::Query, &standalone).contains(&"␣/⏎"));
        assert!(!keys(Mode::Preview, &standalone).contains(&"⏎"));
        assert!(keys(Mode::Results, &herdr).contains(&"⏎"));
        assert!(keys(Mode::Preview, &herdr).contains(&"⏎"));
        assert_eq!(keys(Mode::Action, &herdr), ["esc", "^C"]);
    }

    #[test]
    fn action_screen_overlays_the_integration_lines() {
        let temp = TempDir::new("ui-action").unwrap();
        let (_store, mut state) = searched(&temp);
        state.mode = Mode::Action;
        let buf = render(&mut state, &Themed(Palette::terminal()), 100, 30);
        assert!(find(&buf, " Recovery ").is_some());
        assert!(find(&buf, "The recorded workspace is unavailable.").is_some());
    }
}
