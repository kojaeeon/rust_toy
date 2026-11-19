use regex::Regex;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Java,
    Sql,
    Xml,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Java => "JAVA",
            Self::Sql => "SQL",
            Self::Xml => "XML",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Unknown,
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "?",
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}
#[derive(Debug)]
pub struct LogEvent {
    pub id: u64,
    pub generation: u64,
    pub first_line: u64,
    pub last_line: u64,
    pub time: String,
    pub level: Level,
    pub logger: String,
    pub request: String,
    pub message: String,
    pub raw: String,
    pub kind: Kind,
    pub search: String,
}
pub struct Parser {
    start: Regex,
    header: Regex,
    request: Regex,
    sql: Regex,
    pub events: VecDeque<LogEvent>,
    next_id: u64,
    max: usize,
    continuation: bool,
    pub truncated: u64,
}
impl Parser {
    pub fn new(pattern: &str, max: usize) -> Result<Self, regex::Error> {
        Ok(Self {
            start: Regex::new(pattern)?,
            header: Regex::new(
                r"^(?P<time>\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?)\s+(?P<level>TRACE|DEBUG|INFO|WARN|ERROR|FATAL)\s+(?:\[[^\]]*\]\s+)?(?P<logger>.*?)\s+-\s*(?P<message>.*)$",
            )?,
            request: Regex::new(r"(?:requestId|traceId)=([^\s\]]+)")?,
            sql: Regex::new(
                r"(?i)\b(select\s|insert\s+into\b|update\s+\w|delete\s+from\b|with\s+\w+\s+as\b|merge\s+into\b)",
            )?,
            events: VecDeque::new(),
            next_id: 1,
            max,
            continuation: false,
            truncated: 0,
        })
    }
    pub fn boundary(&mut self) {
        self.continuation = false;
    }
    pub fn push(&mut self, line: &str, number: u64, generation: u64) {
        let line = line.trim_start_matches('\u{feff}');
        if self.start.is_match(line) || !self.continuation {
            let caps = self.header.captures(line);
            let part = |name: &str| {
                caps.as_ref()
                    .and_then(|c| c.name(name))
                    .map_or("", |m| m.as_str())
                    .to_string()
            };
            let level = match part("level").as_str() {
                "TRACE" => Level::Trace,
                "DEBUG" => Level::Debug,
                "INFO" => Level::Info,
                "WARN" => Level::Warn,
                "ERROR" | "FATAL" => Level::Error,
                _ => Level::Unknown,
            };
            let message = if caps.is_some() {
                part("message")
            } else {
                line.to_string()
            };
            let request = self
                .request
                .captures(line)
                .map_or(String::new(), |c| c[1].to_string());
            let event = LogEvent {
                id: self.next_id,
                generation,
                first_line: number,
                last_line: number,
                time: part("time"),
                level,
                logger: part("logger"),
                request,
                message,
                raw: line.to_string(),
                kind: Kind::Java,
                search: String::new(),
            };
            self.next_id += 1;
            self.events.push_back(event);
            if self.events.len() > self.max {
                self.events.pop_front();
            }
            self.continuation = true;
        } else if let Some(event) = self.events.back_mut() {
            event.last_line = number;
            if event.raw.len() + line.len() < 256 * 1024 {
                event.raw.push('\n');
                event.raw.push_str(line);
            } else {
                self.truncated += 1;
            }
        }
        if let Some(event) = self.events.back_mut() {
            let lower_logger = event.logger.to_lowercase();
            // An event can contain multiple payload types; the detail formatter keeps all blocks.
            event.kind = if lower_logger.contains("sql")
                || self.sql.is_match(&event.message)
                || event
                    .raw
                    .lines()
                    .skip(1)
                    .any(|l| self.sql.is_match(l.trim()))
            {
                Kind::Sql
            } else if event.message.contains('<')
                || event
                    .raw
                    .lines()
                    .skip(1)
                    .any(|l| l.trim_start().starts_with('<'))
            {
                Kind::Xml
            } else {
                Kind::Java
            };
            event.search = event.raw.to_lowercase();
        }
    }
}
