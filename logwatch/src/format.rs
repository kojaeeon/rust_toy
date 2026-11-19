use crate::parser::LogEvent;
use quick_xml::{Reader, Writer, events::Event};

fn xml(text: &str) -> Option<String> {
    let mut reader = Reader::from_str(text);
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    let mut depth: usize = 0;
    let mut roots = 0;
    loop {
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
// Find the first complete root in one pass, without reparsing every XML line.
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
fn sql_start(line: &str) -> bool {
    let upper = line.trim().to_uppercase();
    [
        "SELECT ", "INSERT ", "UPDATE ", "DELETE ", "WITH ", "MERGE ", "SELECT", "INSERT",
        "UPDATE", "DELETE",
    ]
    .iter()
    .any(|prefix| {
        if prefix.ends_with(' ') {
            upper.starts_with(prefix)
        } else {
            upper == *prefix
        }
    })
}
pub fn detail(event: &LogEvent, formatted: bool) -> String {
    if !formatted {
        return event.raw.clone();
    }
    let mut parts = vec![event.message.as_str()];
    parts.extend(event.raw.lines().skip(1));
    let mut result = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let line = parts[i];
        let sql_offset = if sql_start(line) {
            Some(0)
        } else {
            line.find("] ")
                .map(|p| p + 2)
                .filter(|p| sql_start(&line[*p..]))
        };
        if let Some(start) = line.find('<').filter(|_| sql_offset.is_none()) {
            let prefix = &line[..start];
            let mut end = i + 1;
            let mut block = line[start..].to_string();
            while end < parts.len() {
                let extra = parts[end];
                if sql_start(extra)
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
            // Stop at a completed XML root so a later payload stays separate.
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
            // A request-id prefix is metadata; retain it while formatting an inline query.
            if let Some(start) = sql_offset {
                if start > 0 {
                    result.push(line[..start].trim().to_string());
                }
                let mut end = i + 1;
                while end < parts.len()
                    && !["Parameters:", "Elapsed:", "Rows:", "SQL:"]
                        .iter()
                        .any(|p| parts[end].trim_start().starts_with(p))
                    && !parts[end].trim_start().starts_with('<')
                {
                    end += 1;
                }
                let mut query = line[start..].to_string();
                for extra in &parts[i + 1..end] {
                    query.push('\n');
                    query.push_str(extra);
                }
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
