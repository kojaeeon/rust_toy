use crate::{
    config::Viewer,
    format,
    parser::{Kind, Level, Parser},
    tail::Tail,
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct App {
    pub parser: Parser,
    pub tail: Tail,
    pub path: String,
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
    pub fn refresh(&mut self) {
        let query = self.query.to_lowercase();
        let min = [Level::Unknown, Level::Info, Level::Warn, Level::Error][self.level];
        let kind = [None, Some(Kind::Java), Some(Kind::Sql), Some(Kind::Xml)][self.kind];
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
        if self.follow {
            self.selected = self.filtered.last().copied();
        } else if !self.selected.is_some_and(|id| self.filtered.contains(&id)) {
            self.selected = self.filtered.first().copied();
        }
        if old != self.selected {
            self.scroll = 0;
            self.horizontal = 0;
        }
        self.update_detail();
    }
    pub fn selected_event(&self) -> Option<&crate::parser::LogEvent> {
        self.selected
            .and_then(|id| self.parser.events.iter().find(|e| e.id == id))
    }
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
    pub fn poll(&mut self) {
        match self.tail.poll() {
            Ok(batch) => {
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
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return true;
        }
        if self.help {
            self.help = false;
            return false;
        }
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
    pub fn run(&mut self, terminal: &mut ratatui::DefaultTerminal, poll_ms: u64) -> Result<()> {
        use crossterm::event::{self, Event, KeyEventKind};
        use std::time::{Duration, Instant};
        let interval = Duration::from_millis(poll_ms);
        let mut last = Instant::now();
        let mut list_state = ratatui::widgets::ListState::default();
        loop {
            terminal.draw(|frame| crate::ui::draw(frame, self, &mut list_state))?;
            let wait = interval.saturating_sub(last.elapsed());
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
