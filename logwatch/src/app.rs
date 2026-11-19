//! 파일 폴링과 키 입력을 조율하고 필터, 선택, 스크롤 등 화면 상태를 관리한다.

use crate::{
    config::Viewer,
    format,
    parser::{Kind, Level, Parser},
    tail::Tail,
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// Parser는 이벤트 저장, Tail은 파일 읽기, App은 사용자가 보는 상태를 맡는다.
pub struct App {
    pub parser: Parser,
    pub tail: Tail,
    pub path: String,
    // 이벤트를 복제하지 않고 필터 결과와 선택에 고유 ID를 보관한다.
    pub filtered: Vec<u64>,
    pub selected: Option<u64>,
    pub follow: bool,
    pub query: String,
    pub editing: bool,
    pub level: usize,
    pub kind: usize,
    pub request: Option<String>,
    pub detail_focus: bool,
    pub scroll: u16,
    pub horizontal: u16,
    pub formatted: bool,
    pub wrap: bool,
    pub bottom: bool,
    pub ratio: f64,
    pub help: bool,
    pub status: String,
    pub detail: String,
    // 상세 캐시의 키는 (이벤트 ID, 원문 바이트 길이, 포맷 여부)다.
    detail_key: Option<(u64, usize, bool)>,
}
impl App {
    pub fn new(parser: Parser, tail: Tail, path: String, viewer: &Viewer) -> Self {
        let mut app = Self {
            parser,
            tail,
            path,
            filtered: vec![],
            selected: None,
            follow: viewer.follow,
            query: String::new(),
            editing: false,
            level: 0,
            kind: 0,
            request: None,
            detail_focus: false,
            scroll: 0,
            horizontal: 0,
            formatted: true,
            wrap: viewer.wrap,
            bottom: viewer.layout == "bottom",
            ratio: viewer.detail_ratio,
            help: false,
            status: String::new(),
            detail: String::new(),
            detail_key: None,
        };
        app.refresh();
        app
    }
    /// 현재 조건으로 목록을 다시 계산하고 선택 상태와 상세 캐시를 맞춘다.
    pub fn refresh(&mut self) {
        let query = self.query.to_lowercase();
        let min = [Level::Unknown, Level::Info, Level::Warn, Level::Error][self.level];
        let kind = [None, Some(Kind::Java), Some(Kind::Sql), Some(Kind::Xml)][self.kind];
        // 지정하지 않은 유형/요청 ID는 통과시킨다. 레벨, 유형, 요청 ID, 검색어는 AND 조건이다.
        self.filtered = self
            .parser
            .events
            .iter()
            .filter(|e| {
                e.level >= min
                    && kind.is_none_or(|k| e.kind == k)
                    && self.request.as_ref().is_none_or(|r| &e.request == r)
                    && e.search.contains(&query)
            })
            .map(|e| e.id)
            .collect();
        let old = self.selected;
        // 추적 중이면 가장 최근에 매칭된 이벤트로 이동한다. 고정 중에는 기존 선택을 가능한 한 유지한다.
        if self.follow {
            self.selected = self.filtered.last().copied();
        } else if !self.selected.is_some_and(|id| self.filtered.contains(&id)) {
            self.selected = self.filtered.first().copied();
        }
        // 같은 이벤트에 줄이 추가될 때는 위치를 유지한다. 이벤트가 바뀌면 상세 처음부터 보여 준다.
        if old != self.selected {
            self.scroll = 0;
            self.horizontal = 0;
        }
        self.update_detail();
    }
    /// 저장소 안의 이벤트를 빌린다. 필터링이나 보관 한도로 선택이 비어 있을 수 있다.
    pub fn selected_event(&self) -> Option<&crate::parser::LogEvent> {
        self.selected
            .and_then(|id| self.parser.events.iter().find(|e| e.id == id))
    }
    // 원문에 후속 줄이 붙거나 RAW/FORMATTED가 바뀌면 같은 ID도 다시 포맷한다.
    fn update_detail(&mut self) {
        let key = self
            .selected_event()
            .map(|e| (e.id, e.raw.len(), self.formatted));
        if self.detail_key != key {
            self.detail = self
                .selected_event()
                .map(|e| format::detail(e, self.formatted))
                .unwrap_or_default();
            self.detail_key = key;
        }
    }
    /// 읽기 오류는 상태 줄에 남기고 다음 폴링에서 다시 시도한다.
    pub fn poll(&mut self) {
        match self.tail.poll() {
            Ok(batch) => {
                // 먼저 이벤트 경계를 끊은 뒤 새 세대의 줄을 전달해야 이전 로그와 섞이지 않는다.
                if batch.reset {
                    self.parser.boundary();
                }
                let changed = batch.reset || !batch.lines.is_empty();
                for (number, line) in batch.lines {
                    self.parser.push(&line, number, self.tail.generation);
                }
                self.status = if batch.reset {
                    "File replaced/truncated; reading new generation".into()
                } else {
                    String::new()
                };
                if changed {
                    self.refresh();
                }
            }
            Err(error) => self.status = format!("Waiting / read error: {error}"),
        }
    }
    // 수동 탐색은 추적을 끈다. 포화 덧셈과 목록 끝값으로 인덱스를 안전한 범위에 둔다.
    fn navigate(&mut self, delta: isize) {
        self.follow = false;
        if self.filtered.is_empty() {
            return;
        }
        let current = self
            .selected
            .and_then(|id| self.filtered.iter().position(|x| *x == id))
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(delta)
            .min(self.filtered.len() - 1);
        self.selected = Some(self.filtered[next]);
        self.scroll = 0;
        self.horizontal = 0;
        self.update_detail();
    }
    /// 종료가 필요하면 true를 반환한다. 도움말·검색 편집을 일반 모드보다 먼저 해석한다.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        if self.help {
            self.help = false;
            return false;
        }
        // 같은 `q`도 편집 중에는 검색 문자다. 변경 즉시 필터 결과를 갱신한다.
        if self.editing {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.editing = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.refresh();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.clear();
                    self.refresh();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.push(c);
                    self.refresh();
                }
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('?') | KeyCode::F(1) => self.help = true,
            KeyCode::Char('/') => self.editing = true,
            KeyCode::Esc => {
                self.query.clear();
                self.request = None;
                self.refresh();
            }
            KeyCode::Char(' ') => {
                self.follow = !self.follow;
                self.refresh();
            }
            KeyCode::Tab | KeyCode::BackTab => self.detail_focus = !self.detail_focus,
            KeyCode::Char('f') => {
                self.level = (self.level + 1) % 4;
                self.refresh();
            }
            KeyCode::Char('t') => {
                self.kind = (self.kind + 1) % 4;
                self.refresh();
            }
            // 요청 ID를 clone해 App이 소유한다. 필터 기준이 선택 이벤트의 수명에 묶이지 않는다.
            KeyCode::Char('r') => {
                self.request = if self.request.is_some() {
                    None
                } else {
                    self.selected_event()
                        .filter(|e| !e.request.is_empty())
                        .map(|e| e.request.clone())
                };
                self.refresh();
            }
            KeyCode::Char('v') => {
                self.formatted = !self.formatted;
                self.update_detail();
                self.scroll = 0;
                self.horizontal = 0;
            }
            KeyCode::Char('w') => {
                self.wrap = !self.wrap;
                self.scroll = 0;
                self.horizontal = 0;
            }
            KeyCode::Char('l') => self.bottom = !self.bottom,
            // match의 가드로 목록 탐색과 상세 스크롤에 같은 키를 다른 의미로 쓴다.
            KeyCode::Down | KeyCode::Char('j') if !self.detail_focus => self.navigate(1),
            KeyCode::Up | KeyCode::Char('k') if !self.detail_focus => self.navigate(-1),
            KeyCode::PageDown if !self.detail_focus => self.navigate(10),
            KeyCode::PageUp if !self.detail_focus => self.navigate(-10),
            KeyCode::Home if !self.detail_focus => self.navigate(-(self.filtered.len() as isize)),
            KeyCode::End if !self.detail_focus => {
                self.follow = true;
                self.refresh();
            }
            KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(10),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Home => {
                self.scroll = 0;
                self.horizontal = 0;
            }
            KeyCode::Left => self.horizontal = self.horizontal.saturating_sub(4),
            KeyCode::Right => self.horizontal = self.horizontal.saturating_add(4),
            _ => {}
        }
        false
    }
    /// 화면 그리기, 입력 대기, 주기적 파일 읽기를 한 루프에서 수행한다.
    pub fn run(&mut self, terminal: &mut ratatui::DefaultTerminal, poll_ms: u64) -> Result<()> {
        use crossterm::event::{self, Event, KeyEventKind};
        use std::time::{Duration, Instant};
        let interval = Duration::from_millis(poll_ms);
        let mut last = Instant::now();
        let mut list_state = ratatui::widgets::ListState::default();
        loop {
            terminal.draw(|frame| crate::ui::draw(frame, self, &mut list_state))?;
            // 키가 없어도 다음 폴링 시각까지만 기다린다. 이미 시각을 넘겼으면 대기 시간은 0이다.
            let wait = interval.saturating_sub(last.elapsed());
            // 읽을 이벤트가 있을 때만 read하고, 키를 뗀 이벤트는 건너뛴다.
            if event::poll(wait)?
                && let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
                && self.key(key)
            {
                break;
            }
            if last.elapsed() >= interval {
                self.poll();
                last = Instant::now();
            }
        }
        Ok(())
    }
}
