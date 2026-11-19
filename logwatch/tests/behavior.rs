use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use logwatch::{
    app::App,
    config::{Config, Multiline, Viewer},
    parser::{Kind, Level, Parser},
    tail::Tail,
};
use ratatui::{Terminal, backend::TestBackend, widgets::ListState};
use std::{
    fs::{self, OpenOptions},
    io::Write,
};
use tempfile::tempdir;

fn parser(max: usize) -> Parser {
    Parser::new(&Multiline::default().start_pattern, max).unwrap()
}
fn append(path: &std::path::Path, data: &[u8]) {
    let mut f = OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(data).unwrap();
    f.flush().unwrap();
}
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn sample_groups_payloads_stack_traces_and_requests() {
    let mut p = parser(100);
    for (i, line) in include_str!("../logs/application.log").lines().enumerate() {
        p.push(line, i as u64 + 1, 1);
    }
    assert_eq!(p.events.len(), 25);
    let sql = &p.events[4];
    assert_eq!(sql.kind, Kind::Sql);
    assert_eq!(sql.request, "req-001");
    assert_eq!((sql.first_line, sql.last_line), (5, 15));
    assert!(sql.raw.contains("Parameters: [10042]"));
    let xml = &p.events[5];
    assert_eq!(xml.kind, Kind::Xml);
    assert!(xml.raw.contains("홍길동"));
    let error = p
        .events
        .iter()
        .find(|e| e.message.contains("주문 조회 실패"))
        .unwrap();
    assert_eq!(error.level, Level::Error);
    assert!(error.raw.contains("Caused by:"));
    let bad_xml = p
        .events
        .iter()
        .find(|e| e.message.contains("잘린 XML"))
        .unwrap();
    assert!(logwatch::format::detail(bad_xml, true).contains("incomplete/invalid"));
    assert_eq!(logwatch::format::detail(error, false), error.raw);
}

#[test]
fn mixed_payload_formatting_preserves_xml_text_and_sql_comparison() {
    let mut p = parser(10);
    let lines = [
        "2026-11-20 14:32:00.001 DEBUG [main] Example - [requestId=X] select * from orders where amount < 100",
        "Parameters: []",
        "<response><memo>연락 &amp; 보관</memo></response>",
        "SQL:",
        "SELECT id FROM orders WHERE id = ?",
        "Parameters: [10042]",
    ];
    for (i, line) in lines.iter().enumerate() {
        p.push(line, i as u64 + 1, 1);
    }
    let formatted = logwatch::format::detail(&p.events[0], true);
    assert!(!formatted.contains("incomplete/invalid"));
    assert!(formatted.contains("amount < 100"));
    assert!(formatted.contains("연락 &amp; 보관"));
    assert!(formatted.contains("Parameters: [10042]"));
    assert!(formatted.contains("SQL:"));
    assert_eq!(
        logwatch::format::detail(&p.events[0], false),
        lines.join("\n")
    );
}

#[test]
fn append_waits_for_complete_utf8_and_crlf() {
    let d = tempdir().unwrap();
    let path = d.path().join("app.log");
    fs::write(&path, b"first\r\n").unwrap();
    let mut tail = Tail::new(path.clone(), "utf-8").unwrap();
    assert_eq!(tail.initial(200).unwrap().lines, vec![(1, "first".into())]);
    let bytes = "한글\r\n".as_bytes();
    append(&path, &bytes[..2]);
    assert!(tail.poll().unwrap().lines.is_empty());
    append(&path, &bytes[2..]);
    assert_eq!(tail.poll().unwrap().lines, vec![(2, "한글".into())]);
    assert_eq!(tail.replacements, 0);
    assert!(tail.poll().unwrap().lines.is_empty());
}

#[test]
fn truncate_and_replace_reset_line_numbers_and_pending_bytes() {
    let d = tempdir().unwrap();
    let path = d.path().join("app.log");
    fs::write(&path, b"old long line\npartial").unwrap();
    let mut tail = Tail::new(path.clone(), "utf-8").unwrap();
    tail.initial(200).unwrap();
    fs::write(&path, b"new\n").unwrap();
    let batch = tail.poll().unwrap();
    assert!(batch.reset);
    assert_eq!(batch.lines, vec![(1, "new".into())]);
    let rotated = d.path().join("old.log");
    fs::rename(&path, rotated).unwrap();
    assert!(tail.poll().is_err());
    fs::write(&path, b"replacement longer than before\n").unwrap();
    let batch = tail.poll().unwrap();
    assert!(batch.reset);
    assert_eq!(batch.lines[0].0, 1);
    assert_eq!(tail.generation, 3);
}

#[test]
fn same_length_overwrite_is_detected_by_anchor() {
    let d = tempdir().unwrap();
    let path = d.path().join("app.log");
    fs::write(&path, b"aaa\n").unwrap();
    let mut tail = Tail::new(path.clone(), "utf-8").unwrap();
    tail.initial(200).unwrap();
    fs::write(&path, b"bbb\n").unwrap();
    let batch = tail.poll().unwrap();
    assert!(batch.reset);
    assert_eq!(batch.lines[0].1, "bbb");
}

#[test]
fn initial_window_preserves_actual_line_numbers_and_dump_flushes_final_line() {
    let d = tempdir().unwrap();
    let path = d.path().join("app.log");
    fs::write(&path, b"one\ntwo\nthree\nfour").unwrap();
    let mut tail = Tail::new(path, "utf-8").unwrap();
    assert_eq!(
        tail.initial(2).unwrap().lines,
        vec![(2, "two".into()), (3, "three".into())]
    );
    assert_eq!(tail.finish(), vec![(4, "four".into())]);
}

#[test]
fn cp949_and_oversized_lines_are_bounded() {
    let d = tempdir().unwrap();
    let path = d.path().join("app.log");
    let (encoded, _, _) = encoding_rs::EUC_KR.encode("한글\n");
    fs::write(&path, &encoded).unwrap();
    let mut tail = Tail::new(path.clone(), "euc-kr").unwrap();
    assert_eq!(tail.initial(10).unwrap().lines[0].1, "한글");
    append(&path, &vec![b'x'; 70000]);
    assert!(tail.poll().unwrap().lines.is_empty());
    append(&path, b"\n");
    let batch = tail.poll().unwrap();
    assert!(batch.lines[0].1.len() < 66000);
    assert_eq!(tail.oversized_lines, 1);
    assert!(Tail::new(path, "utf-16").is_err());
}

#[test]
fn ring_eviction_and_rotation_never_merge_old_event() {
    let mut p = parser(2);
    for i in 1..=3 {
        p.push(
            &format!("2026-11-20 14:32:00.001 INFO  [main] Example - event{i}"),
            i,
            1,
        );
    }
    assert_eq!(p.events.len(), 2);
    assert_eq!(p.events[0].id, 2);
    p.boundary();
    p.push("orphan after rotation", 1, 2);
    assert_eq!(p.events.len(), 2);
    assert_eq!(p.events[1].generation, 2);
    assert!(!p.events[0].raw.contains("orphan"));
}

#[test]
fn hold_keeps_selection_while_ingestion_and_search_continue() {
    let d = tempdir().unwrap();
    let path = d.path().join("app.log");
    fs::write(
        &path,
        "2026-11-20 14:32:00.001 INFO  [main] Example - [requestId=A] first\n",
    )
    .unwrap();
    let mut tail = Tail::new(path.clone(), "utf-8").unwrap();
    let batch = tail.initial(200).unwrap();
    let mut p = parser(10);
    for (n, line) in batch.lines {
        p.push(&line, n, 1);
    }
    let mut app = App::new(p, tail, "app.log".into(), &Viewer::default());
    key(&mut app, KeyCode::Char(' '));
    append(
        &path,
        b"2026-11-20 14:32:01.001 ERROR [main] Example - [requestId=B] second\n",
    );
    app.poll();
    assert_eq!(app.selected, Some(1));
    assert_eq!(app.filtered.len(), 2);
    key(&mut app, KeyCode::Char('/'));
    for c in "SECOND".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.filtered, vec![2]);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::End);
    assert!(app.follow);
    key(&mut app, KeyCode::Char('r'));
    assert_eq!(app.request.as_deref(), Some("B"));
    assert_eq!(app.filtered, vec![2]);
    key(&mut app, KeyCode::Char('r'));
    assert_eq!(app.filtered.len(), 2);
    key(&mut app, KeyCode::Char('f'));
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(app.filtered, vec![2]);
}

#[test]
fn config_paths_are_relative_to_toml_and_invalid_settings_fail() {
    let d = tempdir().unwrap();
    let config = d.path().join("viewer.toml");
    fs::write(&config, "[[sources]]\nname='test'\npath='logs/app.log'\n").unwrap();
    assert_eq!(
        Config::load(&config, true).unwrap().sources[0].path,
        d.path().join("logs/app.log")
    );
    fs::write(&config, "[viewer]\nmax_events=0\n").unwrap();
    assert!(Config::load(&config, true).is_err());
    fs::write(&config, "[viewer]\nmax_event=5\n").unwrap();
    assert!(Config::load(&config, true).is_err());
}

#[test]
fn render_wide_narrow_empty_and_detail_scrolling() {
    let d = tempdir().unwrap();
    let path = d.path().join("app.log");
    fs::write(&path, include_str!("../logs/application.log")).unwrap();
    let mut tail = Tail::new(path, "utf-8").unwrap();
    let batch = tail.initial(200).unwrap();
    let mut p = parser(100);
    for (n, line) in batch.lines {
        p.push(&line, n, 1);
    }
    let mut app = App::new(p, tail, "application.log".into(), &Viewer::default());
    app.follow = false;
    app.selected = Some(5);
    app.refresh();
    for (w, h) in [(130, 32), (70, 24), (40, 10)] {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut state = ListState::default();
        terminal
            .draw(|f| logwatch::ui::draw(f, &mut app, &mut state))
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(screen.contains("LOGWATCH"));
        if w == 130 {
            assert!(screen.contains("SELECT"));
            assert!(screen.contains("Parameters:"));
        }
    }
    app.query = "no-such-event".into();
    app.refresh();
    let mut terminal = Terminal::new(TestBackend::new(130, 32)).unwrap();
    let mut state = ListState::default();
    terminal
        .draw(|f| logwatch::ui::draw(f, &mut app, &mut state))
        .unwrap();
    assert!(app.selected.is_none());
    assert_eq!(logwatch::ui::safe("\x1b[31mhello\x07"), "[31mhello");
}
