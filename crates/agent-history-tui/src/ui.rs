//! Layout and drawing. Everything here is a pure function of the state and
//! palette, so it renders identically to a `TestBackend` and a terminal.
use crate::{
    markdown,
    text::{self, matches, query_terms, safe, width},
    theme::Palette,
    BrowserState, Integration, Mode, RoleFilter, SessionState, RESULT_LIMIT,
};
use agent_history_core::{
    availability::Availability, index::IndexProgress, Agent, EventKind, SearchResult,
};
use ratatui::{
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
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

/// Where the last frame drew each clickable region, so a click resolves
/// against exactly what the user saw.
#[derive(Clone, Debug, Default)]
pub(crate) struct Hits {
    pub(crate) search: Rect,
    pub(crate) tabs: Vec<(Rect, RoleFilter)>,
    /// The whole results pane, border included.
    pub(crate) results: Option<Rect>,
    /// Each visible result's rows, with its index.
    pub(crate) rows: Vec<(Rect, usize)>,
    pub(crate) preview: Option<Rect>,
}

pub fn draw(frame: &mut Frame, state: &mut BrowserState, integration: &impl Integration) {
    let p = integration.palette();
    let area = frame.area();
    state.hits = Hits::default();
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

    if integration.host_draws_title() {
        draw_status_strip(frame, header, state, &p);
    } else {
        draw_header(frame, header, state, integration.title(), &p);
    }
    draw_search(frame, search, state, &p);
    draw_filters(frame, filters, state, &p);

    let (results_area, preview_area) = split_body(body, state.mode);
    if let Some(r) = results_area {
        draw_results(frame, r, state, integration, &p);
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
    draw_keys(
        frame,
        keys,
        state.mode,
        state.mouse_capture,
        integration,
        &p,
    );
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

/// The header row when the host already titles the pane: the index status
/// gets the whole width instead of repeating the host's title.
fn draw_status_strip(frame: &mut Frame, area: Rect, state: &BrowserState, p: &Palette) {
    let label = "Index ";
    let status = safe(
        &state.status,
        usize::from(area.width).saturating_sub(width_of(label) + 2),
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(label, p.key()),
            Span::styled(status, p.muted()),
        ])),
        area,
    );
}

fn draw_search(frame: &mut Frame, area: Rect, state: &mut BrowserState, p: &Palette) {
    state.hits.search = area;
    let focused = state.mode == Mode::Query;
    let block = pane("Search", focused, p);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let prompt = Span::styled(" › ", p.key());
    let room = usize::from(inner.width).saturating_sub(4);
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
        let x = inner.x.saturating_add(3 + typed as u16);
        frame.set_cursor_position(Position::new(
            x.min(inner.right().saturating_sub(1)),
            inner.y,
        ));
    }
}

fn draw_filters(frame: &mut Frame, area: Rect, state: &mut BrowserState, p: &Palette) {
    let mut spans = vec![Span::raw(" ")];
    let mut x = area.x + 1;
    for filter in RoleFilter::ALL {
        let style = if filter == state.role_filter {
            p.active_tab()
        } else {
            p.inactive_tab()
        };
        let tab = format!(" {} ", filter.label());
        let w = width_of(&tab) as u16;
        let tab_area = Rect::new(x, area.y, w, 1).intersection(area);
        state.hits.tabs.push((tab_area, filter));
        x = x.saturating_add(w + 1);
        spans.push(Span::styled(tab, style));
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
    // Already a short `owner/name` label rather than a path, and the owner
    // distinguishes same-named repositories, so it is shown whole.
    let repo = r.repository.clone();
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

/// `area` less one column of padding on each side.
fn padded(area: Rect) -> Rect {
    if area.width <= 2 {
        return area;
    }
    Rect {
        x: area.x + 1,
        width: area.width - 2,
        ..area
    }
}

/// A dim horizontal rule across `width` columns, optionally led by a label.
fn rule(width: usize, label: Option<Span<'static>>, p: &Palette) -> Line<'static> {
    let mut spans = Vec::new();
    if let Some(label) = label {
        spans.push(label);
        spans.push(Span::raw(" "));
    }
    let used: usize = spans.iter().map(|s| width_of(&s.content)).sum();
    spans.push(Span::styled(
        "─".repeat(width.saturating_sub(used)),
        Style::new().fg(p.surface1),
    ));
    fit(spans, width)
}

/// One row of a result: selection bar, gutter, content, right padding.
/// Every row of a result spans the full pane width, so a selection style
/// patched onto it covers the padding too.
fn result_row(bar: &Span<'static>, content: Vec<Span<'static>>, w: usize) -> Line<'static> {
    let mut spans = vec![bar.clone(), Span::raw(" ")];
    spans.extend(fit(content, w.saturating_sub(3)).spans);
    spans.push(Span::raw(" "));
    fit(spans, w)
}

/// Glyph and color for a session's availability. Each state has its own
/// shape, so it reads without color too.
pub(crate) fn marker(state: Option<SessionState>, p: &Palette) -> (&'static str, Color) {
    match state {
        None => ("·", p.overlay0),
        Some(SessionState::Live) => ("◉", p.teal),
        Some(SessionState::Stored(Availability::OnDisk)) => ("●", p.green),
        Some(SessionState::Stored(Availability::Recoverable)) => ("◐", p.yellow),
        Some(SessionState::Stored(Availability::RepositoryKnown)) => ("○", p.red),
        Some(SessionState::Stored(Availability::TranscriptOnly)) => ("◌", p.overlay0),
    }
}

fn draw_results(
    frame: &mut Frame,
    area: Rect,
    state: &mut BrowserState,
    integration: &impl Integration,
    p: &Palette,
) {
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
    state.hits.results = Some(area);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if state.results.is_empty() {
        draw_empty(frame, padded(inner), state, p);
        return;
    }
    let w = usize::from(inner.width);
    // Items are separated by a rule, so n items take n * ITEM_HEIGHT - 1 rows.
    let per_page = ((usize::from(inner.height) + 1) / ITEM_HEIGHT).max(1);
    if state.list_scrolled {
        // Wheel scrolling moves the view, not the selection.
        state.list_offset = state
            .list_offset
            .min(state.results.len().saturating_sub(per_page));
    } else if state.selected < state.list_offset {
        state.list_offset = state.selected;
    } else if state.selected >= state.list_offset + per_page {
        state.list_offset = state.selected + 1 - per_page;
    }
    state.list_offset = state.list_offset.min(state.results.len() - 1);
    let terms = query_terms(&state.query);
    let content_w = w.saturating_sub(3);
    let mut lines = Vec::new();
    for (i, r) in state
        .results
        .iter()
        .enumerate()
        .skip(state.list_offset)
        .take(per_page)
    {
        if i > state.list_offset {
            let mut separator = vec![Span::raw(" ")];
            separator.extend(rule(w.saturating_sub(2), None, p).spans);
            separator.push(Span::raw(" "));
            lines.push(Line::from(separator));
        }
        let selected = i == state.selected;
        let bar = if selected {
            Span::styled("▌", Style::new().fg(p.accent))
        } else {
            Span::raw(" ")
        };
        let d = date(r);
        let availability = state.availability.state(&r.session_id);
        let (glyph, color) = marker(availability, p);
        let mut head = vec![
            Span::styled(glyph, Style::new().fg(color).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            agent_span(r.agent, p),
            Span::raw("  "),
            role_span(r.kind, p),
        ];
        if let Some(availability) = availability {
            let label = integration.availability_label(availability).to_string();
            let used: usize = head.iter().map(|s| width_of(&s.content)).sum();
            // The label is dropped, never truncated, when the row is narrow.
            if used + 2 + width_of(&label) + 1 + width_of(&d) <= content_w {
                head.push(Span::raw("  "));
                head.push(Span::styled(label, Style::new().fg(color)));
            }
        }
        let used: usize = head.iter().map(|s| width_of(&s.content)).sum();
        let gap = content_w.saturating_sub(used + width_of(&d)).max(1);
        head.push(Span::raw(" ".repeat(gap)));
        head.push(Span::styled(d, p.muted()));
        let mut item = vec![
            result_row(&bar, head, w),
            result_row(&bar, context_spans(r, p), w),
        ];
        let snippet = text::wrap(&markdown::plain(&r.snippet), content_w.max(1));
        for k in 0..SNIPPET_LINES {
            let content = snippet.get(k).map_or_else(Vec::new, |line| {
                highlighted(line, &terms, Style::new().fg(p.subtext0), p)
            });
            item.push(result_row(&bar, content, w));
        }
        if selected {
            let style = p.selection(focused);
            item = item.into_iter().map(|l| l.patch_style(style)).collect();
        }
        let top = inner.y.saturating_add(lines.len() as u16);
        let rows = Rect::new(inner.x, top, inner.width, item.len() as u16).intersection(inner);
        if !rows.is_empty() {
            state.hits.rows.push((rows, i));
        }
        lines.extend(item);
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_empty(frame: &mut Frame, area: Rect, state: &BrowserState, p: &Palette) {
    let w = usize::from(area.width);
    let mut lines = vec![Line::raw("")];
    let push = |lines: &mut Vec<Line<'static>>, s: &str, style: Style| {
        for l in text::wrap(s, w) {
            lines.push(Line::styled(l, style));
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

/// The preview split into messages. Core separates events with a blank line
/// and starts each with its role, so a role prefix counts only at the start
/// of the text or after a blank line.
fn messages(preview: &str) -> Vec<(Option<EventKind>, String)> {
    let mut out: Vec<(Option<EventKind>, String)> = Vec::new();
    let mut boundary = true;
    for raw in preview.split('\n') {
        let heading = if boundary {
            [
                ("User: ", EventKind::User),
                ("Assistant: ", EventKind::Assistant),
                ("Tool: ", EventKind::ToolResult),
            ]
            .into_iter()
            .find_map(|(prefix, kind)| raw.strip_prefix(prefix).map(|rest| (kind, rest)))
        } else {
            None
        };
        boundary = raw.is_empty();
        match heading {
            Some((kind, rest)) => out.push((Some(kind), rest.to_string())),
            None if raw.starts_with("[Surrounding context is limited]") => {
                out.push((None, raw.to_string()))
            }
            None => match out.last_mut() {
                Some((Some(_), body)) => {
                    body.push('\n');
                    body.push_str(raw);
                }
                _ => out.push((None, raw.to_string())),
            },
        }
    }
    for (_, body) in &mut out {
        let trimmed = body.trim_end_matches('\n').len();
        body.truncate(trimmed);
    }
    out
}

/// Patches the matched-term style onto cells whose text matches the query.
fn highlight_cells(cells: &mut [markdown::Cell], terms: &[String], p: &Palette) -> bool {
    let text: String = cells.iter().map(|c| c.0).collect();
    let ranges = matches(&text, terms);
    if ranges.is_empty() {
        return false;
    }
    let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    for (i, cell) in cells.iter_mut().enumerate() {
        if ranges.iter().any(|&(a, b)| (a..b).contains(&starts[i])) {
            cell.1 = cell.1.patch(p.matched());
        }
    }
    true
}

/// Preview lines, each flagged when it contains a matched term. Every
/// message starts with a rule labelled with its role; its text is rendered
/// as markdown behind a gutter in the role's color.
fn preview_lines(
    preview: &str,
    width: usize,
    terms: &[String],
    p: &Palette,
) -> Vec<(Line<'static>, bool)> {
    let mut out = Vec::new();
    for (kind, body) in messages(preview) {
        let Some(kind) = kind else {
            if !body.trim().is_empty() {
                for l in text::wrap(&body, width) {
                    out.push((
                        Line::styled(l, p.muted().add_modifier(Modifier::ITALIC)),
                        false,
                    ));
                }
            }
            continue;
        };
        let (name, color) = role(kind, p);
        if !out.is_empty() {
            out.push((Line::raw(""), false));
        }
        out.push((
            rule(
                width,
                Some(Span::styled(
                    name,
                    Style::new().fg(color).add_modifier(Modifier::BOLD),
                )),
                p,
            ),
            false,
        ));
        let gutter = Span::styled("▎ ", Style::new().fg(color));
        for mut cells in
            markdown::render(&body, width.saturating_sub(2), Style::new().fg(p.text), p)
        {
            let hit = highlight_cells(&mut cells, terms, p);
            let mut spans = vec![gutter.clone()];
            spans.extend(markdown::to_line(&cells).spans);
            out.push((Line::from(spans), hit));
        }
    }
    out
}

fn draw_preview(frame: &mut Frame, area: Rect, state: &mut BrowserState, p: &Palette) {
    state.hits.preview = Some(area);
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
    let content = padded(inner);
    let w = usize::from(content.width);
    let lines: Vec<(Line<'static>, bool)> = if state.selected_result().is_none() {
        vec![
            Line::raw(""),
            Line::styled(
                "The conversation around the selected result appears here.",
                p.muted(),
            ),
        ]
        .into_iter()
        .map(|l| (l, false))
        .collect()
    } else if let Some(error) = &state.preview_error {
        let mut lines = vec![
            Line::raw(""),
            Line::styled("Preview unavailable", p.error()),
        ];
        lines.extend(
            text::wrap(error, w)
                .into_iter()
                .map(|l| Line::styled(l, Style::new().fg(p.text))),
        );
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "Reopen Agent History to re-index changed files.",
            p.muted(),
        ));
        lines.into_iter().map(|l| (l, false)).collect()
    } else {
        preview_lines(&state.preview, w, &query_terms(&state.query), p)
    };
    let visible = usize::from(content.height);
    if state.preview_anchor {
        state.preview_anchor = false;
        if let Some(i) = lines.iter().position(|(_, hit)| *hit) {
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
        .map(|(l, _)| l)
        .collect();
    frame.render_widget(Paragraph::new(shown), content);
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

/// Key hints valid for the focused pane only, plus the mouse toggle, which
/// works everywhere and says what pressing it will do.
pub(crate) fn hints(
    mode: Mode,
    mouse_capture: bool,
    integration: &impl Integration,
) -> Vec<(&'static str, String)> {
    let enter = integration.enter_label().to_lowercase();
    let open = |v: &mut Vec<(&'static str, String)>| {
        if enter == "preview" {
            v.push(("␣/⏎", "preview".into()));
        } else {
            v.push(("␣", "preview".into()));
            v.push(("⏎", enter.clone()));
        }
    };
    let mouse = (
        "F3",
        if mouse_capture {
            "mouse off"
        } else {
            "mouse on"
        }
        .to_string(),
    );
    let mut v = Vec::new();
    match mode {
        Mode::Query => {
            v.push(("↓/tab", "results".into()));
            open(&mut v);
            v.push(("F2", "role".into()));
            v.push(mouse);
            v.push(("esc", "quit".into()));
        }
        Mode::Results => {
            v.push(("↑↓", "move".into()));
            open(&mut v);
            v.push(("F2", "role".into()));
            v.push(mouse);
            v.push(("tab", "preview".into()));
            v.push(("esc", "search".into()));
        }
        Mode::Preview => {
            v.push(("↑↓/pgup/pgdn", "scroll".into()));
            if let Some(label) = integration.preview_enter_label() {
                v.push(("⏎", label.to_lowercase()));
            }
            v.push(("F2", "role".into()));
            v.push(mouse);
            v.push(("tab", "search".into()));
            v.push(("esc", "results".into()));
        }
        Mode::Action => {
            v.push(("esc", "cancel".into()));
            v.push(mouse);
        }
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
    mouse_capture: bool,
    integration: &impl Integration,
    p: &Palette,
) {
    let mut spans = vec![Span::raw(" ")];
    for (key, label) in hints(mode, mouse_capture, integration) {
        spans.push(Span::styled(key, p.key()));
        spans.push(Span::styled(
            format!(" {label}   "),
            Style::new().fg(p.overlay1),
        ));
    }
    frame.render_widget(
        Paragraph::new(fit(spans, area.width.into())).style(p.bar()),
        area,
    );
}

/// `title` is `None` when the host already titles the pane.
pub(crate) fn draw_progress(
    frame: &mut Frame,
    p: &Palette,
    title: Option<&str>,
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
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(p.border(true));
    if let Some(title) = title {
        block = block.title(Span::styled(
            format!(" {} ", safe(title, width.saturating_sub(4).into())),
            p.pane_title(true),
        ));
    }
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
    use std::collections::HashSet;

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
        fn availability_label(&self, state: SessionState) -> &str {
            match state {
                SessionState::Live => "focus running",
                _ => "other",
            }
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
        find_from(buf, needle, 0)
    }

    /// `find`, considering only matches starting at column `min_x` or later.
    fn find_from(buf: &Buffer, needle: &str, min_x: u16) -> Option<(u16, u16)> {
        (0..buf.area.height).find_map(|y| {
            let mut x = min_x;
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
        assert_eq!(buf[(first.0 - 4, first.1)].symbol(), "▌");
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
        let standalone = |m| hints(m, true, &Standalone);
        let resuming = |m| hints(m, true, &Themed(Palette::terminal()));
        assert!(keys(Mode::Query, &standalone).contains(&"␣/⏎"));
        assert!(!keys(Mode::Preview, &standalone).contains(&"⏎"));
        assert!(keys(Mode::Results, &resuming).contains(&"⏎"));
        assert!(keys(Mode::Preview, &resuming).contains(&"⏎"));
        assert_eq!(keys(Mode::Action, &resuming), ["esc", "F3", "^C"]);
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

    fn markdown_store(temp: &TempDir) -> SqliteStore {
        let cwd = temp.path().to_string_lossy();
        let user = format!(
            r#"{{"type":"user","sessionId":"00000000-0000-4000-8000-000000000002","cwd":"{cwd}","message":{{"content":"why is **portfolio** `hidden`?"}}}}"#
        );
        let answer = serde_escape(
            "## Portfolio rules\n\nThe **portfolio** filter uses `DUST`.\n\n- first point\n- second point\n\n> quoted caveat\n\n```rust\nlet portfolio = 1;\n```",
        );
        let assistant = format!(r#"{{"type":"assistant","message":{{"content":"{answer}"}}}}"#);
        crate::tests::fixture_store(temp.path(), &[&user, &assistant])
    }

    fn serde_escape(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    }

    fn markdown_state(store: &SqliteStore) -> BrowserState {
        let mut state = BrowserState {
            query: "portfolio".into(),
            ..Default::default()
        };
        state.refresh(store);
        assert_eq!(state.results.len(), 2);
        state
    }

    /// The results pane's inner rectangle: inside its border.
    fn results_inner(buf: &Buffer) -> Rect {
        let (x, y) = find(buf, " Results ").unwrap();
        let left = x - 1;
        let right = (left + 1..buf.area.width)
            .find(|&x| matches!(buf[(x, y)].symbol(), "┓" | "╮"))
            .unwrap();
        let bottom = (y + 1..buf.area.height)
            .find(|&yy| matches!(buf[(left, yy)].symbol(), "┗" | "╰"))
            .unwrap();
        Rect::new(left + 1, y + 1, right - left - 1, bottom - y - 1)
    }

    #[test]
    fn result_rows_are_padded_separated_and_fully_selected() {
        let temp = TempDir::new("ui-rows").unwrap();
        let (_store, mut state) = searched(&temp);
        state.mode = Mode::Results;
        let buf = render(&mut state, &Standalone, 100, 30);
        let inner = results_inner(&buf);
        let (hx, hy) = find(&buf, "Claude  USER").unwrap();
        let (_, ay) = find(&buf, "Claude  ASSISTANT").unwrap();
        // Header, context and snippet share one gutter: two columns in. The
        // header starts with the availability marker, then the agent.
        assert_eq!(hx, inner.x + 4);
        assert_eq!(buf[(inner.x + 2, hy)].symbol(), "·", "pending marker");
        let snippet_row: String = (inner.x + 2..inner.right())
            .map(|x| buf[(x, hy + 2)].symbol())
            .collect();
        assert!(
            snippet_row.starts_with("rememberable topic"),
            "snippet aligns with the header"
        );
        assert_eq!(
            buf[(inner.x + 2, hy + 1)].symbol(),
            "n",
            "context aligns with the header"
        );
        // The selection covers all four rows edge to edge, padding included.
        for y in hy..hy + 4 {
            for x in [inner.x, inner.x + 1, inner.right() - 1] {
                assert_eq!(buf[(x, y)].bg, Color::DarkGray, "cell {x},{y}");
            }
        }
        // A dim rule separates the items, padded on both sides.
        let sep = hy + 4;
        assert_eq!(ay, sep + 1);
        assert_eq!(buf[(inner.x, sep)].symbol(), " ");
        assert_eq!(buf[(inner.right() - 1, sep)].symbol(), " ");
        for x in inner.x + 1..inner.right() - 1 {
            assert_eq!(buf[(x, sep)].symbol(), "─");
            assert_eq!(buf[(x, sep)].fg, Palette::terminal().surface1);
            assert_eq!(buf[(x, sep)].bg, Color::Reset, "separator is not selected");
        }
        assert_eq!(buf[(inner.x + 1, ay)].bg, Color::Reset);
    }

    #[test]
    fn preview_renders_markdown_with_padding_and_role_rules() {
        let temp = TempDir::new("ui-markdown").unwrap();
        let store = markdown_store(&temp);
        let mut state = markdown_state(&store);
        state.preview_anchor = false;
        let p = Palette::terminal();
        let buf = render(&mut state, &Standalone, 120, 40);
        let (px, _) = find(&buf, " Preview ").unwrap();
        let text: Vec<String> = (0..40).map(|y| row(&buf, y)).collect();
        let all = text.join("\n");
        // Messages open with a rule labelled by role, one column in.
        let (ux, uy) = find_from(&buf, "USER ─", px).unwrap();
        assert_eq!(ux, px + 1, "preview content is padded from the border");
        assert_eq!(buf[(ux, uy)].fg, p.blue);
        assert_eq!(buf[(ux + 6, uy)].fg, p.surface1);
        assert!(find_from(&buf, "ASSISTANT ─", px).is_some());
        // Markdown is rendered, not shown raw.
        assert!(!all.contains("**") && !all.contains("```") && !all.contains("## "));
        let (hx, hy) = find_from(&buf, "Portfolio rules", px).unwrap();
        assert_eq!(buf[(hx, hy)].bg, p.yellow, "query term inside the heading");
        let rules = (hx + 10, hy);
        assert!(buf[rules].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[rules].fg, p.accent);
        let (cx, cy) = find_from(&buf, "DUST", px).unwrap();
        assert_eq!(buf[(cx, cy)].bg, p.surface0, "inline code");
        assert!(find_from(&buf, "• first point", px).is_some());
        assert!(find_from(&buf, "▎ quoted caveat", px).is_some());
        let (kx, ky) = find_from(&buf, "let portfolio = 1;", px).unwrap();
        assert_eq!(buf[(kx, ky)].bg, p.surface0, "code block");
        let right = (px..120)
            .rev()
            .find(|&x| matches!(buf[(x, ky)].symbol(), "│" | "┃"))
            .unwrap();
        assert_eq!(
            buf[(right - 2, ky)].bg,
            p.surface0,
            "code block is a solid block"
        );
        assert_eq!(
            buf[(right - 1, ky)].bg,
            Color::Reset,
            "right padding stays clear"
        );
        // Query terms are still highlighted inside rendered markdown.
        let (bx, by) = find_from(&buf, "portfolio filter", px).unwrap();
        assert_eq!(buf[(bx, by)].bg, p.yellow);
        assert!(buf[(bx, by)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn snippets_strip_markdown_syntax() {
        let temp = TempDir::new("ui-snippet").unwrap();
        let store = markdown_store(&temp);
        let mut state = markdown_state(&store);
        state.mode = Mode::Results;
        let buf = render(&mut state, &Standalone, 120, 40);
        let inner = results_inner(&buf);
        let results: String = (inner.y..inner.bottom())
            .map(|y| {
                (inner.x..inner.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(results.contains("why is portfolio hidden?"), "{results}");
        assert!(!results.contains("**") && !results.contains('`') && !results.contains("##"));
    }

    #[test]
    fn hostile_markdown_cannot_corrupt_the_preview() {
        let temp = TempDir::new("ui-hostile-md").unwrap();
        let (_store, mut state) = searched(&temp);
        let payload = "# \u{1b}[2J\u{1b}]52;c;aGk=\u{7} title\n\n```\n\u{1b}[31m雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪雪 e\u{301}\u{301}\u{301}\u{9b}2J\n```\n\n> \u{202e}evil **\u{1b}[5mblink** `\u{0}`\n\n"
            .repeat(10)
            + &">".repeat(400)
            + " deep\n\n"
            + &"- ".repeat(200)
            + "<script>x</script> [l](javascript:x)";
        state.preview = format!("User: {payload}\n\nAssistant: {payload}");
        state.results[0].snippet = payload.clone();
        for (w, h) in [(120, 40), (61, 40), (30, 12)] {
            for mode in [Mode::Results, Mode::Preview] {
                state.mode = mode;
                state.preview_anchor = true;
                let buf = render(&mut state, &Standalone, w, h);
                for y in 0..h {
                    for x in 0..w {
                        let symbol = buf[(x, y)].symbol();
                        assert!(
                            !symbol.chars().any(|c| c.is_control() || c == '\u{202e}'),
                            "control character at {x},{y}: {symbol:?}"
                        );
                    }
                    if w >= SIDE_BY_SIDE_MIN_WIDTH && y > 5 && y < h - 2 {
                        assert!(
                            matches!(buf[(w - 1, y)].symbol(), "│" | "┃"),
                            "row {y}: {:?}",
                            row(&buf, y)
                        );
                    }
                }
                assert!(!(0..h).any(|y| row(&buf, y).contains("javascript")));
            }
        }
    }

    /// Five results for five distinct sessions, one per availability state.
    fn five_sessions(temp: &TempDir) -> (SqliteStore, BrowserState, Vec<SessionState>) {
        let (store, mut state) = searched(temp);
        let template = state.results[0].clone();
        let states = vec![
            SessionState::Live,
            SessionState::Stored(Availability::OnDisk),
            SessionState::Stored(Availability::Recoverable),
            SessionState::Stored(Availability::RepositoryKnown),
            SessionState::Stored(Availability::TranscriptOnly),
        ];
        state.results = (0..states.len())
            .map(|i| {
                let mut r = template.clone();
                r.session_id.native_id = format!("00000000-0000-4000-8000-00000000009{i}");
                r
            })
            .collect();
        for (r, s) in state.results.iter().zip(&states) {
            match s {
                SessionState::Live => state.availability.set_live([r.session_id.clone()]),
                SessionState::Stored(a) => state.availability.record(r.session_id.clone(), *a),
            }
        }
        (store, state, states)
    }

    /// (glyph, glyph color, header text) of every result row, top to bottom.
    fn headers(buf: &Buffer) -> Vec<(String, Color, String)> {
        let inner = results_inner(buf);
        (inner.y..inner.bottom())
            .filter_map(|y| {
                let text: String = (inner.x + 2..inner.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect();
                text.contains("Claude").then(|| {
                    let cell = &buf[(inner.x + 2, y)];
                    (
                        cell.symbol().to_string(),
                        cell.fg,
                        text.trim_end().to_string(),
                    )
                })
            })
            .collect()
    }

    #[test]
    fn each_availability_state_has_its_own_glyph_color_and_label() {
        let temp = TempDir::new("ui-availability").unwrap();
        let (_store, mut state, states) = five_sessions(&temp);
        state.mode = Mode::Results;
        let buf = render(&mut state, &Standalone, 120, 40);
        let rows = headers(&buf);
        assert_eq!(rows.len(), states.len(), "{rows:?}");
        let p = Palette::terminal();
        for ((glyph, fg, text), s) in rows.iter().zip(&states) {
            let (want, color) = marker(Some(*s), &p);
            assert_eq!((glyph.as_str(), *fg), (want, color), "{s:?}");
            assert!(text.contains(Standalone.availability_label(*s)), "{text}");
            // Standalone wording describes the disk, never a resume.
            assert!(!text.to_lowercase().contains("resum"), "{text}");
        }
        // Distinct in shape, so monochrome terminals and colour-blind users
        // can tell them apart, and distinct in wording.
        let glyphs: HashSet<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(glyphs.len(), states.len());
        let labels: HashSet<&str> = states
            .iter()
            .map(|s| Standalone.availability_label(*s))
            .collect();
        assert_eq!(labels.len(), states.len());

        // An integration supplies its own wording through the same boundary.
        let buf = render(&mut state, &Themed(Palette::terminal()), 120, 40);
        assert!(headers(&buf)[0].2.contains("focus running"));
    }

    #[test]
    fn narrow_rows_keep_the_glyph_and_drop_the_label() {
        let temp = TempDir::new("ui-availability-narrow").unwrap();
        let (_store, mut state, states) = five_sessions(&temp);
        state.mode = Mode::Results;
        let buf = render(&mut state, &Standalone, 36, 60);
        assert_eq!(headers(&buf).len(), states.len());
        for ((glyph, _, text), s) in headers(&buf).iter().zip(&states) {
            assert_eq!(glyph, marker(Some(*s), &Palette::terminal()).0);
            assert!(!text.contains(Standalone.availability_label(*s)), "{text}");
        }
    }

    #[test]
    fn availability_is_computed_once_per_session_not_per_frame() {
        use crate::availability::Worker;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let temp = TempDir::new("ui-availability-once").unwrap();
        let (_store, mut state, _) = five_sessions(&temp);
        state.availability = Default::default();
        let template = state_session(&temp);
        let sessions: Vec<_> = state
            .results
            .iter()
            .map(|r| agent_history_core::Session {
                id: r.session_id.clone(),
                ..template.clone()
            })
            .collect();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let worker = Worker::spawn(sessions, move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Availability::OnDisk
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut frames = 0;
        // The run loop's per-iteration work: drain, request, draw.
        while frames < 50 || state.availability.pending() {
            assert!(
                std::time::Instant::now() < deadline,
                "worker never answered"
            );
            worker.drain(&mut state.availability);
            worker.request(state.availability.wanted(&state.results));
            terminal.draw(|f| draw(f, &mut state, &Standalone)).unwrap();
            frames += 1;
        }
        assert_eq!(calls.load(Ordering::SeqCst), state.results.len());
        let buf = terminal.backend().buffer().clone();
        assert!(headers(&buf).iter().all(|(g, _, _)| g == "●"));
    }

    fn state_session(temp: &TempDir) -> agent_history_core::Session {
        let store = SqliteStore::open(temp.path().join("private/index.sqlite")).unwrap();
        store.sessions().unwrap().remove(0)
    }

    use crate::Mouse;

    fn click(state: &mut BrowserState, store: &SqliteStore, (column, row): (u16, u16)) {
        state.mouse(Mouse::Click { column, row }, store);
    }

    /// The status bar's text.
    fn status_bar(buf: &Buffer) -> String {
        row(buf, buf.area.height - 1)
    }

    /// Asserts `focused` has the heavy accent frame, the other panes do not,
    /// and the status bar shows exactly the hints keyboard focus would.
    fn assert_focus(buf: &Buffer, state: &BrowserState, focused: &str) {
        let p = Palette::terminal();
        for title in ["Search", "Results", "Preview"] {
            let (x, y) = find(buf, &format!(" {title} ")).unwrap();
            let corner = &buf[(x - 1, y)];
            if title == focused {
                assert_eq!(corner.symbol(), "┏", "{title} should be focused");
                assert_eq!(corner.fg, p.accent);
            } else {
                assert_eq!(corner.symbol(), "╭", "{title} should not be focused");
            }
        }
        let bar = status_bar(buf);
        let expected: String = hints(state.mode, state.mouse_capture, &Standalone)
            .iter()
            .map(|(k, l)| format!("{k} {l}   "))
            .collect();
        assert!(
            bar.trim_start().starts_with(expected.trim_end()),
            "status bar {bar:?} should start with {expected:?}"
        );
    }

    #[test]
    fn clicking_anywhere_in_a_pane_focuses_it() {
        let temp = TempDir::new("ui-click-focus").unwrap();
        let (store, mut state) = searched(&temp);
        state.mouse_capture = true;
        let buf = render(&mut state, &Standalone, 120, 36);
        let inner = results_inner(&buf);
        let (sx, sy) = find(&buf, " Search ").unwrap();
        let (px, py) = find(&buf, " Preview ").unwrap();
        // Empty space, borders and titles, not only content.
        let targets = [
            ("Results", (inner.x + 3, inner.bottom() - 1), Mode::Results),
            ("Preview", (px + 20, py + 25), Mode::Preview),
            ("Search", (sx + 80, sy + 1), Mode::Query),
            ("Preview", (buf.area.width - 1, py + 3), Mode::Preview),
            ("Results", (inner.x - 1, inner.y + 10), Mode::Results),
            ("Search", (sx - 1, sy), Mode::Query),
            ("Results", (inner.right() - 1, inner.y + 20), Mode::Results),
        ];
        for (pane, at, mode) in targets {
            let selected = state.selected;
            click(&mut state, &store, at);
            assert_eq!(state.mode, mode, "click at {at:?} should focus {pane}");
            assert_eq!(state.selected, selected, "empty space does not select");
            let buf = render(&mut state, &Standalone, 120, 36);
            assert_focus(&buf, &state, pane);
        }
    }

    #[test]
    fn clicking_a_result_selects_that_row_and_focuses_results() {
        let temp = TempDir::new("ui-click-row").unwrap();
        let (store, mut state, _) = five_sessions(&temp);
        state.mode = Mode::Query;
        let buf = render(&mut state, &Standalone, 120, 40);
        let inner = results_inner(&buf);
        let (_, first) = find(&buf, "Claude  ").unwrap();
        // Rows: 4 lines per result, then one separator line.
        for (dy, expect) in [(0, 0), (3, 0), (5, 1), (8, 1), (10, 2), (15, 3)] {
            state.mode = Mode::Preview;
            click(&mut state, &store, (inner.x + 12, first + dy));
            assert_eq!(state.selected, expect, "row offset {dy}");
            assert_eq!(state.mode, Mode::Results);
            let buf = render(&mut state, &Standalone, 120, 40);
            assert_focus(&buf, &state, "Results");
            assert_eq!(
                buf[(inner.x + 1, first + dy)].bg,
                Color::DarkGray,
                "selected row is highlighted"
            );
        }
        // The separator between results belongs to no result.
        state.selected = 3;
        click(&mut state, &store, (inner.x + 12, first + 4));
        assert_eq!(state.selected, 3);
        assert_eq!(state.mode, Mode::Results);
    }

    #[test]
    fn clicking_a_role_tab_switches_the_filter() {
        let temp = TempDir::new("ui-click-tab").unwrap();
        let (store, mut state) = searched(&temp);
        let buf = render(&mut state, &Standalone, 100, 30);
        let (ux, uy) = find(&buf, " User ").unwrap();
        click(&mut state, &store, (ux + 2, uy));
        assert_eq!(state.role_filter, RoleFilter::User);
        assert!(state.results.iter().all(|r| r.kind == EventKind::User));
        assert_eq!(
            state.mode,
            Mode::Query,
            "a tab does not steal the query's focus"
        );
        let buf = render(&mut state, &Standalone, 100, 30);
        assert_eq!(
            buf[(ux + 1, uy)].bg,
            Color::Blue,
            "clicked tab is highlighted"
        );
        state.mode = Mode::Preview;
        let (ax, ay) = find(&buf, " All ").unwrap();
        click(&mut state, &store, (ax + 1, ay));
        assert_eq!(state.role_filter, RoleFilter::All);
        assert_eq!(state.mode, Mode::Results, "like F2, leaves the preview");
    }

    #[test]
    fn wheel_scrolls_the_pane_under_the_pointer() {
        let temp = TempDir::new("ui-wheel").unwrap();
        let (store, mut state, _) = five_sessions(&temp);
        state.preview = (0..200).map(|i| format!("line {i}\n\n")).collect();
        state.preview_anchor = false;
        state.mode = Mode::Query;
        let buf = render(&mut state, &Standalone, 120, 20);
        let inner = results_inner(&buf);
        let (px, py) = find(&buf, " Preview ").unwrap();
        let wheel_results = Mouse::ScrollDown {
            column: inner.x + 5,
            row: inner.y + 2,
        };
        state.mouse(wheel_results, &store);
        state.mouse(wheel_results, &store);
        render(&mut state, &Standalone, 120, 20);
        assert_eq!(state.list_offset, 2, "list scrolled");
        assert_eq!(state.selected, 0, "selection and preview stay put");
        assert_eq!(state.mode, Mode::Query, "scrolling does not move focus");
        let wheel_preview = Mouse::ScrollDown {
            column: px + 10,
            row: py + 4,
        };
        state.mouse(wheel_preview, &store);
        assert_eq!(state.preview_scroll, 3);
        state.mouse(
            Mouse::ScrollUp {
                column: px + 10,
                row: py + 4,
            },
            &store,
        );
        assert_eq!(state.preview_scroll, 0);
        // Moving the selection makes the list follow it again.
        state.handle(Key::Down, &store).unwrap();
        state.handle(Key::Down, &store).unwrap();
        render(&mut state, &Standalone, 120, 20);
        assert!(!state.list_scrolled);
        assert!(state.list_offset <= state.selected);
    }

    #[test]
    fn mouse_never_touches_the_recovery_screen() {
        let temp = TempDir::new("ui-click-action").unwrap();
        let (store, mut state) = searched(&temp);
        state.mode = Mode::Action;
        render(&mut state, &Themed(Palette::terminal()), 100, 30);
        for at in [(5, 3), (10, 12), (70, 12), (50, 15)] {
            click(&mut state, &store, at);
            state.mouse(
                Mouse::ScrollDown {
                    column: at.0,
                    row: at.1,
                },
                &store,
            );
        }
        assert_eq!(state.mode, Mode::Action);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn status_bar_offers_the_mouse_toggle_in_every_pane() {
        let temp = TempDir::new("ui-toggle-hint").unwrap();
        let (_store, mut state) = searched(&temp);
        for mode in [Mode::Query, Mode::Results, Mode::Preview] {
            state.mode = mode;
            state.mouse_capture = true;
            assert!(status_bar(&render(&mut state, &Standalone, 120, 30)).contains("F3 mouse off"));
            state.mouse_capture = false;
            assert!(status_bar(&render(&mut state, &Standalone, 120, 30)).contains("F3 mouse on"));
        }
    }

    struct HostTitled;
    impl Integration for HostTitled {
        fn title(&self) -> &str {
            "Agent History — Host"
        }
        fn enter_label(&self) -> &str {
            "Resume"
        }
        fn host_draws_title(&self) -> bool {
            true
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
            Vec::new()
        }
    }

    #[test]
    fn a_host_titled_pane_shows_status_instead_of_a_second_title() {
        let temp = TempDir::new("ui-host-title").unwrap();
        let (_store, mut state) = searched(&temp);
        state.status = "412 files · 9120 chunks · 0 failed · 0 malformed · 0.4s".into();
        let buf = render(&mut state, &HostTitled, 100, 30);
        let all: String = (0..30).map(|y| row(&buf, y)).collect();
        assert!(
            !all.contains("Agent History"),
            "the host already shows the title"
        );
        let top = row(&buf, 0);
        assert!(top.starts_with(" Index 412 files · 9120 chunks"), "{top:?}");
        // Standalone keeps its title with the status on the right.
        let buf = render(&mut state, &Standalone, 100, 30);
        let top = row(&buf, 0);
        assert!(top.starts_with(" Agent History (Standalone)"), "{top:?}");
        assert!(top.trim_end().ends_with("0.4s"), "{top:?}");
    }

    #[test]
    fn indexing_progress_omits_the_title_when_the_host_has_one() {
        let draw_with = |title: Option<&str>| {
            let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
            terminal
                .draw(|f| {
                    draw_progress(
                        f,
                        &Palette::terminal(),
                        title,
                        0,
                        std::time::Duration::from_secs(1),
                        None,
                    )
                })
                .unwrap();
            let buf = terminal.backend().buffer().clone();
            (0..20).map(|y| row(&buf, y)).collect::<String>()
        };
        assert!(draw_with(Some("Agent History (Standalone)")).contains("Agent History"));
        let hosted = draw_with(None);
        assert!(!hosted.contains("Agent History"));
        assert!(hosted.contains("Indexing conversations"));
    }
}
