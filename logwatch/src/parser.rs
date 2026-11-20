//! 여러 로그 줄을 이벤트로 묶고 Java 헤더, 요청 ID, 본문 유형을 추출한다.

use crate::format::xml_offset;
use regex::Regex;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 목록에서 한 이벤트당 대표 유형 하나를 표시한다. 본문에는 여러 유형이 함께 있을 수 있다.
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
// 선언 순서로 비교하는 Ord를 파생한다. UI의 INFO+, WARN+ 같은 최소 레벨 비교에 쓰인다.
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
// id는 전체 실행에서 증가한다. generation과 줄 번호는 파일에서의 위치를 표현한다.
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
    // 디코딩 후 줄을 연결한 텍스트. 표시용 포맷과 검색용 소문자 문자열은 이 값에서 만든다.
    pub raw: String,
    pub kind: Kind,
    pub has_xml: bool,
    pub search: String,
}
pub struct Parser {
    start: Regex,
    header: Regex,
    jdbc_header: Regex,
    request: Regex,
    sql: Regex,
    // 뒤에 이벤트를 추가하고 보관 한도를 넘으면 앞에서 빼기 좋은 양방향 큐다.
    pub events: VecDeque<LogEvent>,
    next_id: u64,
    max: usize,
    continuation: bool,
    pub truncated: u64,
}
impl Parser {
    /// 이벤트 경계, 표준 Java 헤더, 요청 ID, SQL 분류 정규식을 한 번 컴파일한다.
    pub fn new(pattern: &str, max: usize) -> Result<Self, regex::Error> {
        Ok(Self {
            start: Regex::new(pattern)?,
            header: Regex::new(
                r"^(?P<time>\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?)\s+(?P<level>TRACE|DEBUG|INFO|WARN|ERROR|FATAL)\s+(?:\[[^\]]*\]\s+)?(?P<logger>.*?)\s+-\s*(?P<message>.*)$",
            )?,
            jdbc_header: Regex::new(
                r"^(?P<time>\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?)\s+(?P<level>TRACE|DEBUG|INFO|WARN|ERROR|FATAL)\s+\[(?P<logger>[^\]]+)\]\s+(?P<message>SQL::?.*)$",
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
    /// 이전 세대의 이벤트는 보관하되 다음 줄을 기존 이벤트에 붙이지 않는다.
    pub fn boundary(&mut self) {
        self.continuation = false;
    }
    pub fn push(&mut self, line: &str, number: u64, generation: u64) {
        // 줄 앞의 BOM을 제거한다. 시작 패턴에 맞거나 이어 붙일 이벤트가 없으면 새 이벤트다.
        let line = line.trim_start_matches('\u{feff}');
        if self.start.is_match(line) || !self.continuation {
            // 헤더가 일치하지 않아도 줄 자체는 보관한다. 이름 있는 캡처를 소유 문자열로 복사한다.
            // JDBC SQL 로그는 대괄호 안에 로거를 기록하고 " - " 구분자 없이 SQL:을 붙인다.
            let caps = self
                .jdbc_header
                .captures(line)
                .or_else(|| self.header.captures(line));
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
            // 첫 줄에서 requestId/traceId 중 처음 매칭된 값을 얻는다.
            let request = self
                .request
                .captures(line)
                .map_or(String::new(), |c| c[1].to_string());
            let has_xml = xml_offset(line).is_some();
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
                has_xml,
                search: String::new(),
            };
            self.next_id += 1;
            self.events.push_back(event);
            // VecDeque의 앞부분만 제거한다. 남은 이벤트의 고유 ID는 바뀌지 않는다.
            if self.events.len() > self.max {
                self.events.pop_front();
            }
            self.continuation = true;
        } else if let Some(event) = self.events.back_mut() {
            // 마지막 이벤트를 가변으로 빌려 후속 줄을 추가한다. 본문 한도를 넘겨도 줄 번호는 갱신한다.
            event.last_line = number;
            if event.raw.len() + line.len() < 256 * 1024 {
                event.raw.push('\n');
                event.raw.push_str(line);
                event.has_xml |= xml_offset(&event.raw).is_some();
            } else {
                self.truncated += 1;
            }
        }
        if let Some(event) = self.events.back_mut() {
            let lower_logger = event.logger.to_lowercase();
            // SQL 단서가 먼저 적용된다. XML이나 스택이 섞여도 detail()은 본문 전체를 처리한다.
            event.kind = if lower_logger.contains("sql")
                || event.message.starts_with("SQL:")
                || self.sql.is_match(&event.message)
                || event
                    .raw
                    .lines()
                    .skip(1)
                    .any(|l| self.sql.is_match(l.trim()))
            {
                Kind::Sql
            } else if event.has_xml {
                Kind::Xml
            } else {
                Kind::Java
            };
            // 질의마다 전체 본문을 소문자로 바꾸지 않도록 이벤트가 갱신될 때 검색 문자열을 만든다.
            event.search = event.raw.to_lowercase();
        }
    }
}
