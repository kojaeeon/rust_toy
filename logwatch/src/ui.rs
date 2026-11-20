//! App 상태를 Ratatui 위젯으로 그린다. 로그 문자열은 제어 문자를 걸러 화면에 전달한다.

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
use regex::Regex;
use std::sync::LazyLock;

const MUTED: Color = Color::Rgb(139, 153, 171);
const ACCENT: Color = Color::Rgb(102, 217, 204);
/// 줄바꿈과 탭을 보존하면서 ESC 같은 제어 문자를 제거한다.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_xml_styles_tags_and_multiline_attributes_without_changing_text() {
        let input =
            "VALUES ('<Dataset\n id=\"result\"><Row>SELECT</Row></Dataset>')\nWHERE id < 100";
        let lines = highlighted(input);
        let text = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text, input);
        let spans: Vec<_> = lines.iter().flat_map(|line| &line.spans).collect();
        for (token, color) in [
            ("VALUES ", ACCENT),
            ("<Dataset", Color::LightMagenta),
            ("id", Color::Yellow),
            ("\"result\"", Color::LightGreen),
            ("SELECT", Color::White),
            ("</Dataset>", Color::LightMagenta),
            ("< ", Color::White),
        ] {
            assert!(
                spans
                    .iter()
                    .any(|span| span.content == token && span.style.fg == Some(color)),
                "missing expected style for {token:?}"
            );
        }
    }
}
fn kind_color(kind: Kind) -> Color {
    match kind {
        Kind::Java => Color::LightBlue,
        Kind::Sql => ACCENT,
        Kind::Xml => Color::LightMagenta,
    }
}
// 제목을 String으로 넘겨 블록이 소유하게 한다. 반환값은 호출자의 지역 문자열을 빌리지 않는다.
fn border(title: String, focus: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(if focus { ACCENT } else { Color::DarkGray }))
}
// 완성된 태그·선언·주석·CDATA만 찾는다. 'id < 100' 같은 비교 연산자는 매치하지 않는다.
static XML_MARKUP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?s)<!--.*?-->|<!\[CDATA\[.*?\]\]>|"#,
        r#"</[:_\p{L}][:_\p{L}\p{N}.\-]*\s*>|"#,
        r#"<\??[:_\p{L}][:_\p{L}\p{N}.\-]*(?:\s+[:_\p{L}][:_\p{L}\p{N}.\-]*\s*=\s*(?:"[^"]*"|'[^']*'))*\s*[/?]?>"#,
    ))
    .unwrap()
});

static XML_ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"([:_\p{L}][:_\p{L}\p{N}.\-]*)\s*=\s*("[^"]*"|'[^']*')"#).unwrap()
});

fn plain_spans(text: &str, spans: &mut Vec<Span<'static>>) {
    let trimmed = text.trim_start();
    let color = if trimmed.starts_with("at ")
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
        spans.push(Span::styled(text.to_string(), Style::default().fg(color)));
        return;
    }
    let keywords = [
        "SELECT", "FROM", "WHERE", "ORDER", "BY", "INSERT", "INTO", "VALUES", "UPDATE", "SET",
        "DELETE", "JOIN", "LEFT", "RIGHT", "ON", "AND", "OR", "AS", "WITH", "GROUP", "LIMIT",
        "DESC", "ASC", "NULL",
    ];
    // 토큰 경계 공백까지 보존해 서로 다른 색으로 그려도 문장 간격이 유지된다.
    for token in text.split_inclusive(char::is_whitespace) {
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
}

// XML과 SQL이 같은 줄에 있어도 태그, 속성명, 속성값마다 표시 스타일을 적용한다.
fn highlighted(text: &str) -> Vec<Line<'static>> {
    let clean = safe(text);
    let mut lines = vec![Vec::new()];
    let mut append = |text: &str, color: Option<Color>| {
        for (index, part) in text.split('\n').enumerate() {
            if index > 0 {
                lines.push(Vec::new());
            }
            let spans = lines.last_mut().unwrap();
            if part.is_empty() {
                continue;
            }
            if let Some(color) = color {
                spans.push(Span::styled(part.to_string(), Style::default().fg(color)));
            } else {
                plain_spans(part, spans);
            }
        }
    };
    let tag_color = Some(Color::LightMagenta);
    let mut offset = 0;
    let mut depth = 0usize;
    for found in XML_MARKUP.find_iter(&clean) {
        append(
            &clean[offset..found.start()],
            (depth > 0).then_some(Color::White),
        );
        let tag = found.as_str();
        if tag.starts_with("<!--") || tag.starts_with("<![CDATA[") {
            append(tag, Some(MUTED));
        } else {
            let mut tag_offset = 0;
            for attribute in XML_ATTRIBUTE.captures_iter(tag) {
                let name = attribute.get(1).unwrap();
                let value = attribute.get(2).unwrap();
                append(&tag[tag_offset..name.start()], tag_color);
                append(name.as_str(), Some(Color::Yellow));
                append(&tag[name.end()..value.start()], tag_color);
                append(value.as_str(), Some(Color::LightGreen));
                tag_offset = value.end();
            }
            append(&tag[tag_offset..], tag_color);
            if tag.starts_with("</") {
                depth = depth.saturating_sub(1);
            } else if !tag.starts_with("<?") && !tag.ends_with("/>") {
                depth += 1;
            }
        }
        offset = found.end();
    }
    append(&clean[offset..], (depth > 0).then_some(Color::White));
    lines.into_iter().map(Line::from).collect()
}
pub fn draw(frame: &mut Frame, app: &mut App, state: &mut ListState) {
    // 화면이 너무 작으면 본문을 그리지 않고 크기 안내를 표시한다.
    let area = frame.area();
    if area.width < 50 || area.height < 12 {
        frame.render_widget(
            Paragraph::new("LOGWATCH\nResize terminal to at least 50 columns x 12 rows.\nq: quit")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    // 제목, 필터, 두 본문 패널, 상태, 단축키 안내 순으로 세로 공간을 배분한다.
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
    // 설정이 좌우 배치여도 95열보다 좁으면 위아래로 표시한다.
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
            // ID가 연속으로 붙고 오래된 이벤트는 앞에서만 빠지므로 첫 ID와의 차이가 큐 인덱스다.
            let offset = id.checked_sub(app.parser.events.front()?.id)? as usize;
            let e = app.parser.events.get(offset)?;
            // get은 범위나 UTF-8 경계가 맞지 않으면 None을 반환한다. 이 경우 원래 시간을 사용한다.
            let time = e.time.get(11..23).unwrap_or(&e.time);
            Some(ListItem::new(vec![
                Line::from(vec![
                    Span::raw(format!("{time} ")),
                    Span::styled(
                        format!("{:5} ", e.level.label()),
                        Style::default().fg(level_color(e.level)),
                    ),
                    Span::styled(e.kind.label(), Style::default().fg(kind_color(e.kind))),
                    Span::styled(
                        if e.has_xml && e.kind != Kind::Xml {
                            " +XML"
                        } else {
                            ""
                        },
                        Style::default().fg(kind_color(Kind::Xml)),
                    ),
                ]),
                Line::styled(
                    format!("  {}", safe(&e.message)),
                    Style::default().fg(MUTED),
                ),
            ]))
        })
        .collect();
    // 위젯에 필요한 값은 이벤트 고유 ID가 아니라 필터 결과의 행 인덱스다.
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
    // 상세 제목에 세대와 원본 파일 줄 범위를 표시해 원본에서 같은 이벤트를 찾을 수 있게 한다.
    let title = app.selected_event().map_or(" Detail ".to_string(), |e| {
        format!(
            " {} {} | {} | gen:{} lines:{}-{} ",
            e.level.label(),
            e.kind.label(),
            if app.xml_only {
                "XML ONLY"
            } else if app.formatted {
                "FORMATTED"
            } else {
                "RAW"
            },
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
    if !app.detail.is_empty() {
        lines.extend(highlighted(&app.detail));
    }
    let text = Text::from(lines);
    // 테두리 2칸을 뺀 내부 크기에서 줄바꿈 후 줄 수를 계산한다.
    let width = panes[1].width.saturating_sub(2).max(1);
    let height = panes[1].height.saturating_sub(2);
    let mut detail = Paragraph::new(text);
    if app.wrap {
        detail = detail.wrap(Wrap { trim: false });
    }
    let total_lines = detail.line_count(width);
    let max_scroll = total_lines
        .saturating_sub(height as usize)
        .min(u16::MAX as usize) as u16;
    // 창 크기나 줄바꿈이 변해도 스크롤이 본문 끝을 넘지 않게 제한한다.
    app.scroll = app.scroll.min(max_scroll);
    let title = if max_scroll > 0 && height > 0 {
        let first = app.scroll as usize + 1;
        let last = (app.scroll as usize + height as usize).min(total_lines);
        format!(" {first}-{last}/{total_lines} |{}", title)
    } else {
        title
    };
    detail = detail
        .block(border(title, app.detail_focus))
        .scroll((app.scroll, if app.wrap { 0 } else { app.horizontal }));
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
    frame.render_widget(Paragraph::new(" / search  f level  t type  r request  Space follow  v raw  x XML  w wrap  l layout  ? help  q quit").style(Style::default().fg(ACCENT)), rows[4]);
    // 도움말을 덮어 그리기 전에 겹치는 영역의 배경을 지운다.
    if app.help {
        let popup = Rect::new(
            area.x + area.width / 10,
            area.y + 1,
            area.width * 8 / 10,
            area.height.saturating_sub(2),
        );
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new("↑/↓ or j/k: select event; disables follow\nHome / End: oldest / newest (resume follow)\nTab: switch list / detail focus\nDetail: ↑↓ scroll, ←→ horizontal, PgUp/PgDn page, Home/End first/last\n/: edit search (case-insensitive, full raw event)\nEnter/Esc: finish search; Ctrl+U: clear while editing\nEsc outside search: clear search and request filter\nf: ALL → INFO+ → WARN+ → ERROR\nt: ALL → JAVA → SQL → XML (includes SQL with XML)\nr: filter by selected request/trace ID; press again to clear\nSpace: follow latest on/off (ingestion always continues)\nv: formatted / raw; x: XML only; w: wrap; l: right / bottom layout\nq or Ctrl+C: quit\n\nRaw bytes decoded by configured encoding. XML folding is not yet supported.\nPress any key to close.")
            .block(border(" Keys ".into(), true)).wrap(Wrap { trim: false }), popup);
    }
}
