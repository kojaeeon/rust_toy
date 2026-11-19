use crate::{
    app::App,
    parser::{Kind, Level},
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

const MUTED: Color = Color::Rgb(139, 153, 171);
const ACCENT: Color = Color::Rgb(102, 217, 204);
pub fn safe(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}
fn level_color(level: Level) -> Color {
    match level {
        Level::Error => Color::LightRed,
        Level::Warn => Color::Yellow,
        Level::Info => Color::LightGreen,
        _ => MUTED,
    }
}
fn kind_color(kind: Kind) -> Color {
    match kind {
        Kind::Java => Color::LightBlue,
        Kind::Sql => ACCENT,
        Kind::Xml => Color::LightMagenta,
    }
}
fn border(title: String, focus: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if focus { ACCENT } else { Color::DarkGray }))
}
fn highlighted(text: &str) -> Line<'static> {
    let clean = safe(text);
    let trimmed = clean.trim_start();
    let color = if trimmed.starts_with('<') {
        Color::LightMagenta
    } else if trimmed.starts_with("at ")
        || trimmed.starts_with("Caused by:")
        || trimmed.contains("Exception:")
    {
        Color::LightRed
    } else if ["Parameters:", "Elapsed:", "Rows:"]
        .iter()
        .any(|p| trimmed.starts_with(p))
    {
        Color::Yellow
    } else {
        Color::White
    };
    if color != Color::White {
        return Line::styled(clean, Style::default().fg(color));
    }
    let mut spans = vec![];
    let keywords = [
        "SELECT", "FROM", "WHERE", "ORDER", "BY", "INSERT", "INTO", "VALUES", "UPDATE", "SET",
        "DELETE", "JOIN", "LEFT", "RIGHT", "ON", "AND", "OR", "AS", "WITH", "GROUP", "LIMIT",
        "DESC", "ASC", "NULL",
    ];
    for token in clean.split_inclusive(char::is_whitespace) {
        let word = token.trim().to_uppercase();
        spans.push(Span::styled(
            token.to_string(),
            Style::default().fg(if keywords.contains(&word.as_str()) {
                ACCENT
            } else {
                Color::White
            }),
        ));
    }
    Line::from(spans)
}
pub fn draw(frame: &mut Frame, app: &mut App, state: &mut ListState) {
    let area = frame.area();
    if area.width < 50 || area.height < 12 {
        frame.render_widget(
            Paragraph::new("LOGWATCH\nResize terminal to at least 50 columns x 12 rows.\nq: quit")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " LOGWATCH ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(safe(&app.path)),
            Span::styled(
                if app.follow {
                    "  LIVE / FOLLOW"
                } else {
                    "  LIVE / HOLD"
                },
                Style::default().fg(if app.follow {
                    Color::LightGreen
                } else {
                    Color::Yellow
                }),
            ),
        ])),
        rows[0],
    );
    let filter = format!(
        " {}{} | level:{} type:{} request:{} | {} / {} events",
        if app.editing { "SEARCH> " } else { "/ " },
        safe(&app.query),
        ["ALL", "INFO+", "WARN+", "ERROR"][app.level],
        ["ALL", "JAVA", "SQL", "XML"][app.kind],
        app.request.as_deref().unwrap_or("ALL"),
        app.filtered.len(),
        app.parser.events.len()
    );
    frame.render_widget(
        Paragraph::new(filter)
            .style(Style::default().fg(if app.editing { Color::Yellow } else { MUTED }))
            .wrap(Wrap { trim: false }),
        rows[1],
    );
    let bottom = app.bottom || area.width < 95;
    let ratio = (app.ratio * 100.0) as u16;
    let panes = Layout::default()
        .direction(if bottom {
            Direction::Vertical
        } else {
            Direction::Horizontal
        })
        .constraints([
            Constraint::Percentage(100 - ratio),
            Constraint::Percentage(ratio),
        ])
        .split(rows[2]);
    let items: Vec<_> = app
        .filtered
        .iter()
        .filter_map(|id| {
            let offset = id.checked_sub(app.parser.events.front()?.id)? as usize;
            let e = app.parser.events.get(offset)?;
            let time = e.time.get(11..23).unwrap_or(&e.time);
            Some(ListItem::new(vec![
                Line::from(vec![
                    Span::raw(format!("{time} ")),
                    Span::styled(
                        format!("{:5} ", e.level.label()),
                        Style::default().fg(level_color(e.level)),
                    ),
                    Span::styled(e.kind.label(), Style::default().fg(kind_color(e.kind))),
                ]),
                Line::styled(
                    format!("  {}", safe(&e.message)),
                    Style::default().fg(MUTED),
                ),
            ]))
        })
        .collect();
    state.select(
        app.selected
            .and_then(|id| app.filtered.iter().position(|n| *n == id)),
    );
    let list = List::new(items)
        .block(border(" Events ".into(), !app.detail_focus))
        .highlight_symbol("▶ ")
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(36, 54, 66))
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(list, panes[0], state);
    if app.filtered.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching events\nEsc: clear search/request\nf / t: change filters")
                .style(Style::default().fg(MUTED))
                .wrap(Wrap { trim: false }),
            panes[0].inner(ratatui::layout::Margin {
                horizontal: 2,
                vertical: 2,
            }),
        );
    }
    let title = app.selected_event().map_or(" Detail ".to_string(), |e| {
        format!(
            " {} {} | {} | gen:{} lines:{}-{} ",
            e.level.label(),
            e.kind.label(),
            if app.formatted { "FORMATTED" } else { "RAW" },
            e.generation,
            e.first_line,
            e.last_line
        )
    });
    let mut lines = vec![];
    if let Some(e) = app.selected_event() {
        lines.push(Line::styled(
            safe(&format!("{}  {}", e.time, e.logger)),
            Style::default().fg(MUTED),
        ));
        if !e.request.is_empty() {
            lines.push(Line::styled(
                safe(&format!("request: {}", e.request)),
                Style::default().fg(Color::Yellow),
            ));
        }
        lines.push(Line::raw(""));
    }
    lines.extend(app.detail.lines().map(highlighted));
    let text = Text::from(lines);
    let block = border(title, app.detail_focus);
    let width = panes[1].width.saturating_sub(2).max(1);
    let height = panes[1].height.saturating_sub(2);
    let mut detail = Paragraph::new(text).block(block);
    if app.wrap {
        detail = detail.wrap(Wrap { trim: false });
    }
    let max_scroll = detail
        .line_count(width)
        .saturating_sub(height as usize)
        .min(u16::MAX as usize) as u16;
    app.scroll = app.scroll.min(max_scroll);
    detail = detail.scroll((app.scroll, if app.wrap { 0 } else { app.horizontal }));
    frame.render_widget(detail, panes[1]);
    let note = if !app.status.is_empty() {
        safe(&app.status)
    } else {
        format!(
            " gen:{} | decode warnings:{} line truncations:{} event truncations:{} | wrap:{} | Tab: focus {}",
            app.tail.generation,
            app.tail.replacements,
            app.tail.oversized_lines,
            app.parser.truncated,
            app.wrap,
            if app.detail_focus { "detail" } else { "list" }
        )
    };
    frame.render_widget(
        Paragraph::new(note).style(Style::default().fg(MUTED)),
        rows[3],
    );
    frame.render_widget(Paragraph::new(" / search  f level  t type  r request  Space follow  v raw  w wrap  l layout  ? help  q quit").style(Style::default().fg(ACCENT)), rows[4]);
    if app.help {
        let popup = Rect::new(
            area.x + area.width / 10,
            area.y + 1,
            area.width * 8 / 10,
            area.height.saturating_sub(2),
        );
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new("↑/↓ or j/k: select event; disables follow\nHome / End: oldest / newest (resume follow)\nTab: switch list / detail focus\nDetail: ↑↓ scroll, ←→ horizontal, PgUp/PgDn page\n/: edit search (case-insensitive, full raw event)\nEnter/Esc: finish search; Ctrl+U: clear while editing\nEsc outside search: clear search and request filter\nf: ALL → INFO+ → WARN+ → ERROR\nt: ALL → JAVA → SQL → XML\nr: filter by selected request/trace ID; press again to clear\nSpace: follow latest on/off (ingestion always continues)\nv: formatted / raw; w: wrap; l: right / bottom layout\nq or Ctrl+C: quit\n\nRaw bytes decoded by configured encoding. XML folding is not yet supported.\nPress any key to close.")
            .block(border(" Keys ".into(), true)).wrap(Wrap { trim: false }), popup);
    }
}
