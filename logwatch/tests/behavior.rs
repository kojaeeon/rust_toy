//! 공개 모듈을 조합해 파일 입력부터 파싱, 검색, 화면 렌더링까지 검증한다.

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

// 운영 기본 시작 패턴을 재사용하고 테스트마다 이벤트 보관 한도만 바꾼다.
fn parser(max: usize) -> Parser {
    Parser::new(&Multiline::default().start_pattern, max).unwrap()
}
// 기존 내용을 유지하며 바이트를 추가한다. UTF-8 문자의 일부만 쓰는 테스트도 가능하다.
fn append(path: &std::path::Path, data: &[u8]) {
    let mut f = OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(data).unwrap();
    f.flush().unwrap();
}
// 실제 터미널 없이 App의 키 이벤트 처리기를 호출한다.
fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
// 샘플 전체로 여러 줄 경계, 요청 ID, SQL 파라미터와 Java 예외 체인을 확인한다.
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
// SQL의 `<`를 XML 시작으로 오인하지 않고 같은 이벤트의 XML과 SQL을 모두 표시하는지 검증한다.
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
fn xml_in_multiline_jdbc_insert_is_preserved_as_sql() {
    let mut parser = parser(10);
    let lines = [
        "2026-11-20 11:09:15,987 INFO [jdbc.sqlonly.sqlOccurred(741)] SQL::INSERT INTO AUDIT_LOG (MSG_TEXT, STATUS)",
        "VALUES ('<Dataset id=\"result\">",
        "<Row owner=\"O''Brien\">",
        "Parameters: text inside XML",
        "</Row>",
        "</Dataset>', 'OK');",
        "Parameters: ['OK']",
        "<Response><Status>OK</Status></Response>",
    ];
    for (i, line) in lines.iter().enumerate() {
        parser.push(line, i as u64 + 1, 1);
    }
    let event = &parser.events[0];
    let formatted = logwatch::format::detail(event, true);
    assert_eq!(event.kind, Kind::Sql);
    assert!(!formatted.contains("incomplete/invalid"), "{formatted}");
    let xml = "<Dataset id=\"result\">\n<Row owner=\"O''Brien\">\nParameters: text inside XML\n</Row>\n</Dataset>";
    assert!(formatted.contains(xml), "{formatted}");
    assert!(formatted.contains("Parameters: ['OK']"));
    assert!(formatted.contains("<Response>\n  <Status>OK</Status>\n</Response>"));
    assert_eq!(logwatch::format::detail(event, false), lines.join("\n"));
}

#[test]
fn xml_filter_and_detail_include_jdbc_payloads_and_preserve_the_full_event() {
    let mut parser = parser(10);
    let sql = "2026-11-20 11:09:15,987 INFO [jdbc.sqlonly.sqlOccurred(741)] SQL::INSERT INTO audit_log VALUES ('<Dataset><Row>hello</Row></Dataset>', '<Meta id=\"other\"/>')";
    parser.push(sql, 1, 1);
    parser.push("2026-11-20 11:09:16,000 INFO [jdbc.sqlonly.sqlOccurred(741)] SQL::SELECT id FROM audit_log WHERE id < 100", 2, 1);
    parser.push(
        "2026-11-20 11:09:17,000 INFO demo - response: <Response",
        3,
        1,
    );
    parser.push(" status=\"pending\">", 4, 1);
    assert!(parser.events[2].has_xml);
    parser.push("<Child/>", 5, 1);

    assert_eq!(parser.events[0].kind, Kind::Sql);
    assert!(parser.events[0].has_xml);
    assert!(!parser.events[1].has_xml);
    let tail = Tail::new("unused.log".into(), "utf-8").unwrap();
    let mut app = App::new(parser, tail, "unused.log".into(), &Viewer::default());
    for _ in 0..3 {
        key(&mut app, KeyCode::Char('t'));
    }
    assert_eq!(app.filtered, vec![1, 3]);
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Char('x'));
    assert!(
        app.detail
            .contains("<Dataset>\n  <Row>hello</Row>\n</Dataset>")
    );
    assert!(app.detail.contains("<Meta id=\"other\"/>"));
    assert!(!app.detail.contains("INSERT"));
    assert_eq!(app.copy_text().as_deref(), Some(app.detail.as_str()));
    key(&mut app, KeyCode::Char('v'));
    assert_eq!(app.detail, sql);
    assert_eq!(app.copy_text().as_deref(), Some(sql));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('x'));
    assert!(app.detail.contains("incomplete/invalid"));
    assert!(app.detail.contains("<Child/>"));
    let copy = app.copy_text().unwrap();
    assert!(copy.starts_with("<Response"));
    assert!(copy.contains("\n  <Child/>"));
    assert!(!copy.contains("incomplete/invalid"));
}

#[test]
fn truncated_dataset_keeps_readable_nested_tags_and_the_original_damaged_tail() {
    let raw = "SQL::INSERT INTO SAMPLE VALUES ('<Dataset id=\"sample\"><ColumnInfo><Column id=\"KEY\"/><Column id=\"VALUE\"/></ColumnInfo><Rows><Row><Col>한글 &amp; 값</Col><Col id=\"cut";
    let formatted = logwatch::format::xml_detail(raw, true);
    assert!(formatted.contains("incomplete/invalid"));
    assert!(formatted.contains(
        "  <ColumnInfo>\n    <Column id=\"KEY\"/>\n    <Column id=\"VALUE\"/>\n  </ColumnInfo>"
    ));
    assert!(formatted.contains("      <Col>한글 &amp; 값</Col>"));
    assert!(formatted.ends_with("<Col id=\"cut"));
    assert!(!formatted.contains("</Dataset>"));
}

#[test]
fn jdbc_cte_with_column_list_reaches_detail_end_without_truncation() {
    let mut parser = parser(10);
    let header = "2026-11-20 11:01:58,941 INFO [jdbc.sqlonly.sqlOccurred(741)] SQL::WITH TEMP1 (";
    parser.push(header, 1, 1);
    for n in 1..=70 {
        parser.push(&format!("    COL_{n},"), n + 1, 1);
    }
    parser.push("    FINAL_COLUMN", 72, 1);
    parser.push(") AS (SELECT id FROM orders)", 73, 1);
    parser.push(
        "SELECT id FROM TEMP1 WHERE id < 100 ORDER BY final_marker",
        74,
        1,
    );
    let event = &parser.events[0];
    assert_eq!(parser.events.len(), 1);
    assert_eq!(event.kind, Kind::Sql);
    assert_eq!(event.level, Level::Info);
    assert_eq!(event.logger, "jdbc.sqlonly.sqlOccurred(741)");
    assert_eq!(event.last_line, 74);
    assert_eq!(parser.truncated, 0);
    let formatted = logwatch::format::detail(event, true);
    assert!(formatted.starts_with("SQL::\nWITH"));
    assert!(formatted.contains("final_marker"));
    assert!(logwatch::format::detail(event, false).starts_with(header));

    let tail = Tail::new("unused.log".into(), "utf-8").unwrap();
    let mut app = App::new(parser, tail, "unused.log".into(), &Viewer::default());
    let mut terminal = Terminal::new(TestBackend::new(130, 24)).unwrap();
    let mut state = ListState::default();
    let mut draw = |app: &mut App| {
        terminal
            .draw(|f| logwatch::ui::draw(f, app, &mut state))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    };
    let first = draw(&mut app);
    assert!(first.contains("1-"));
    assert!(!first.contains("final_marker"));
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::End);
    assert!(draw(&mut app).contains("final_marker"));
    assert!(app.scroll > 0);
}

#[test]
// 첫 폴링에는 한글 바이트 일부만 있다. 다음 폴링에서 CRLF까지 오면 온전한 문자열을 기대한다.
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
// 잘림과 파일이 잠시 없는 교체를 차례로 재현하고 세대, 줄 번호, 미완성 줄의 초기화를 검사한다.
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
// 길이가 같아도 읽기 위치 앞의 내용이 바뀌면 새 세대로 감지하는지 확인한다.
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
// 최근 줄만 보관해도 실제 줄 번호를 유지하고, 스냅샷에서만 개행 없는 마지막 줄을 가져온다.
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
// 한국어 인코딩에 이어 큰 줄의 메모리 제한과 비호환 인코딩 거절을 검사한다.
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
// 오래된 이벤트를 제거해도 ID는 유지되고 새 세대의 고아 줄은 이전 이벤트에 붙지 않는다.
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
// 추적 해제 후에도 입력이 쌓이고 검색·요청 ID·최소 레벨 필터가 선택을 올바르게 바꾸는지 검사한다.
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
// 임시 TOML 위치를 기준으로 상대 경로를 해석한다. 허용 범위 위반과 필드 오타는 오류다.
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
// 메모리 화면으로 폭이 다른 창, 결과가 없는 목록, 제어 문자 제거를 확인한다.
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
        // 셀의 표시 문자를 하나의 문자열로 모아 화면에 어떤 글자가 남았는지 확인한다.
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
