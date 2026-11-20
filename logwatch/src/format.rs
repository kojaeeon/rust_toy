//! 보관 중인 원문에서 SQL/XML을 찾아 상세 화면에 표시할 새 문자열을 만든다.

use crate::parser::LogEvent;
use quick_xml::{Reader, Writer, events::Event};

// XML 토큰을 순서대로 Writer에 넘겨 들여쓴다. 루트가 정확히 하나이며 깊이가 0으로 끝나야 한다.
fn xml(text: &str) -> Option<String> {
    let mut reader = Reader::from_str(text);
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    let mut depth: usize = 0;
    let mut roots = 0;
    loop {
        // .ok()?는 읽기 오류를 None으로 바꾼다. 호출자가 원문을 보여 줄 수 있게 실패를 알린다.
        let event = reader.read_event().ok()?;
        match &event {
            Event::Start(_) => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.checked_sub(1)?;
            }
            Event::Empty(_) if depth == 0 => roots += 1,
            // 루트 밖에서는 공백만 허용한다. 루트 내부의 텍스트 공백과 엔티티는 그대로 전달된다.
            Event::Text(t) if depth == 0 && !t.as_ref().trim().is_empty() => {
                return None;
            }
            Event::Eof => break,
            _ => {}
        }
        writer.write_event(event).ok()?;
    }
    if depth != 0 || roots != 1 {
        return None;
    }
    String::from_utf8(writer.into_inner()).ok()
}
// 같은 XML을 줄마다 다시 파싱하지 않고, 첫 루트가 끝나는 UTF-8 바이트 오프셋을 찾는다.
fn xml_end(text: &str) -> Option<usize> {
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    loop {
        match reader.read_event().ok()? {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(reader.buffer_position() as usize);
                }
            }
            Event::Empty(_) if depth == 0 => return Some(reader.buffer_position() as usize),
            Event::Eof => return None,
            _ => {}
        }
    }
}
// 대표적인 SQL 시작 토큰을 찾는 휴리스틱이다. 전체 SQL 문법의 유효성은 검사하지 않는다.
fn sql_start(line: &str) -> bool {
    line.split_whitespace().next().is_some_and(|word| {
        ["SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "MERGE"]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword))
    })
}

fn sql_offset(line: &str) -> Option<usize> {
    let payload = |text: &str| {
        let trimmed = text.trim_start();
        for marker in ["SQL::", "SQL:"] {
            if let Some(rest) = trimmed.strip_prefix(marker) {
                let query = rest.trim_start();
                // SQL:: 뒤의 블록 주석도 SQL 포맷에 함께 전달한다.
                return (!query.is_empty()).then_some(text.len() - query.len());
            }
        }
        sql_start(trimmed).then_some(text.len() - trimmed.len())
    };
    payload(line).or_else(|| {
        let prefix = line.find("] ")? + 2;
        payload(&line[prefix..]).map(|offset| prefix + offset)
    })
}

fn xml_offset(line: &str) -> Option<usize> {
    line.match_indices('<').find_map(|(offset, _)| {
        let next = line[offset + 1..].chars().next()?;
        (next.is_alphabetic() || matches!(next, '_' | ':' | '/' | '?' | '!')).then_some(offset)
    })
}

#[derive(Clone, Copy, Default)]
enum SqlContext {
    #[default]
    Statement,
    Quoted(u8),
    BlockComment,
}

impl SqlContext {
    fn scan_line(&mut self, line: &str) {
        let mut bytes = line.bytes().peekable();
        while let Some(byte) = bytes.next() {
            match (*self, byte, bytes.peek()) {
                (Self::Statement, b'-', Some(b'-')) => break,
                (Self::Statement, b'/', Some(b'*')) => {
                    bytes.next();
                    *self = Self::BlockComment;
                }
                (Self::Statement, b'\'' | b'"', _) => *self = Self::Quoted(byte),
                (Self::Quoted(quote), _, _) if byte == quote => {
                    if bytes.next_if_eq(&quote).is_none() {
                        *self = Self::Statement;
                    }
                }
                (Self::BlockComment, b'*', Some(b'/')) => {
                    bytes.next();
                    *self = Self::Statement;
                }
                _ => {}
            }
        }
    }
}
/// 원문 모드에서는 저장한 문자열을 복제한다. 포맷 모드에서는 블록별 표시 형식을 만든다.
pub fn detail(event: &LogEvent, formatted: bool) -> String {
    if !formatted {
        return event.raw.clone();
    }
    // 파싱된 첫 줄 메시지와 원문 두 번째 줄 이후를 빌려 표시 후보로 묶는다.
    let mut parts = vec![event.message.as_str()];
    parts.extend(event.raw.lines().skip(1));
    let mut result = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let line = parts[i];
        // 문장 앞이나 `] ` 접두사 뒤에서 SQL을 먼저 찾는다. 비교 연산자 `<`도 SQL에 등장할 수 있다.
        let query_offset = sql_offset(line);
        if let Some(start) = xml_offset(line).filter(|_| query_offset.is_none()) {
            let prefix = &line[..start];
            let mut end = i + 1;
            let mut block = line[start..].to_string();
            // 다음 SQL 또는 메타데이터 줄 전까지 XML 후보를 모은다.
            while end < parts.len() {
                let extra = parts[end];
                if sql_offset(extra).is_some()
                    || extra.trim() == "SQL:"
                    || ["Parameters:", "Elapsed:", "Rows:"]
                        .iter()
                        .any(|p| extra.trim_start().starts_with(p))
                {
                    break;
                }
                block.push('\n');
                block.push_str(extra);
                end += 1;
            }
            // 첫 XML 루트 뒤의 텍스트와 다른 줄을 다음 블록에서 처리할 수 있게 경계를 되돌린다.
            let mut suffix = String::new();
            if let Some(bytes) = xml_end(&block) {
                let consumed_lines = block[..bytes].bytes().filter(|b| *b == b'\n').count() + 1;
                end = i + consumed_lines;
                suffix = block[bytes..].split('\n').next().unwrap_or("").to_string();
                block.truncate(bytes);
            }
            if !prefix.trim().is_empty() {
                result.push(prefix.trim_end().to_string());
            }
            // XML 파싱에 실패하면 안내와 수집한 블록을 보여 줘 조사에 필요한 내용을 남긴다.
            match xml(&block) {
                Some(value) => result.push(value),
                None => {
                    result.push("[XML incomplete/invalid: original payload below]".into());
                    result.push(block);
                }
            }
            if !suffix.trim().is_empty() {
                result.push(suffix);
            }
            i = end;
        } else {
            // 같은 줄의 요청 ID 같은 접두사는 별도로 보존하고 SQL 부분만 정렬한다.
            if let Some(start) = query_offset {
                if start > 0 {
                    result.push(line[..start].trim().to_string());
                }
                let mut query = line[start..].to_string();
                let mut context = SqlContext::default();
                context.scan_line(&query);
                let mut end = i + 1;
                for extra in &parts[end..] {
                    let trimmed = extra.trim_start();
                    if matches!(context, SqlContext::Statement)
                        && (["Parameters:", "Elapsed:", "Rows:", "SQL:"]
                            .iter()
                            .any(|prefix| trimmed.starts_with(prefix))
                            || xml_offset(trimmed) == Some(0))
                    {
                        break;
                    }
                    query.push('\n');
                    query.push_str(extra);
                    context.scan_line(extra);
                    end += 1;
                }
                // 표시의 가독성을 위한 포맷이다. SQL을 실행하거나 `?`에 바인드 값을 대입하지 않는다.
                result.push(sqlformat::format(
                    &query,
                    &sqlformat::QueryParams::None,
                    &sqlformat::FormatOptions::default(),
                ));
                i = end;
            } else {
                result.push(line.to_string());
                i += 1;
            }
        }
    }
    result.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    // 정상 XML 들여쓰기, 닫는 태그 오류, 한글과 엔티티 및 공백 보존을 함께 검사한다.
    fn xml_formats_and_preserves_invalid() {
        assert!(
            xml("<response><id>1</id></response>")
                .unwrap()
                .contains("\n  <id>")
        );
        assert!(xml("<response><id>1</id>").is_none());
        assert!(xml("<a></b>").is_none());
        assert!(
            xml("<memo>배송 전 연락 &amp; 문 앞 보관</memo>")
                .unwrap()
                .contains("연락 &amp; 문")
        );
        assert!(xml("<memo> </memo>").unwrap().contains("> </memo>"));
    }
}
