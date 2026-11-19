//! TOML 문자열을 읽어 실행 가능한 단축키 규칙으로 변환하고 충돌을 검사한다.

use serde::Deserialize;
use std::collections::HashSet;

// Win32의 MOD_* 값과 일치하는 비트. 조합은 비트 OR(`|`)로 저장한다.
pub const ALT: u32 = 1;
pub const CTRL: u32 = 2;
pub const SHIFT: u32 = 4;
pub const WIN: u32 = 8;

/// 보조키 비트마스크와 가상 키 코드의 쌍. `Eq`와 `Hash`는 중복 검사에 사용한다.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Chord {
    pub modifiers: u32,
    pub key: u16,
}

#[derive(Debug)]
// 각 variant에 해당 동작이 필요로 하는 값만 보관한다.
pub enum Action {
    Text(String),
    Send(Chord),
}

#[derive(Debug)]
// 원래의 trigger 표기도 보존해 등록 실패 시 사용자 입력을 그대로 보여 준다.
pub struct Binding {
    pub chord: Chord,
    pub name: String,
    pub action: Action,
}

// Serde 전용 구조체. 필드 오타는 역직렬화 오류로 처리한다.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    hotkeys: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    trigger: String,
    text: Option<String>,
    send: Option<String>,
}

// 마지막 토큰이 주키다. 뒤에서부터 읽으면 앞쪽의 보조키를 순서와 무관하게 합칠 수 있다.
fn parse_chord(source: &str) -> Result<Chord, String> {
    let mut tokens = source
        .rsplit('+')
        .map(|token| token.trim().to_ascii_uppercase());
    // Option을 Result로 바꾸고 `?`로 실패를 즉시 반환한다. 오류 문구는 실패할 때만 만든다.
    let key = key_code(&tokens.next().unwrap_or_default())
        .ok_or_else(|| format!("Invalid key in {source:?}"))?;
    let mut modifiers = 0;
    for token in tokens {
        let bit = match token.as_str() {
            "ALT" => ALT,
            "CTRL" => CTRL,
            "SHIFT" => SHIFT,
            "WIN" => WIN,
            _ => return Err(format!("Unknown modifier {token:?} in {source:?}")),
        };
        // 이미 켜진 비트를 검사한 다음 현재 보조키를 비트마스크에 추가한다.
        if modifiers & bit != 0 {
            return Err(format!("Repeated modifier in {source:?}"));
        }
        modifiers |= bit;
    }
    Ok(Chord { modifiers, key })
}

// 영문 대문자와 숫자는 ASCII 값이 Win32 가상 키 코드와 같다.
fn key_code(name: &str) -> Option<u16> {
    if name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric() {
        return Some(u16::from(name.as_bytes()[0]));
    }
    // F1~F24는 연속된 코드다. 범위가 맞을 때만 F1의 코드에서 오프셋을 계산한다.
    if let Some(number) = name
        .strip_prefix('F')
        .and_then(|value| value.parse::<u16>().ok())
    {
        return (1..=24).contains(&number).then(|| 0x70 + number - 1);
    }
    Some(match name {
        "BACKSPACE" => 0x08,
        "TAB" => 0x09,
        "ENTER" => 0x0D,
        "ESC" | "ESCAPE" => 0x1B,
        "SPACE" => 0x20,
        "PAGEUP" => 0x21,
        "PAGEDOWN" => 0x22,
        "END" => 0x23,
        "HOME" => 0x24,
        "LEFT" => 0x25,
        "UP" => 0x26,
        "RIGHT" => 0x27,
        "DOWN" => 0x28,
        "INSERT" => 0x2D,
        "DELETE" => 0x2E,
        _ => return None,
    })
}

/// TOML 문법 검사에 이어 트리거와 동작의 의미를 검증한다.
pub fn parse(source: &str) -> Result<Vec<Binding>, String> {
    let config: Config = toml::from_str(source).map_err(|error| error.to_string())?;
    if config.hotkeys.is_empty() {
        return Err("Add at least one [[hotkeys]] entry.".into());
    }

    let mut triggers = HashSet::new();
    let mut bindings = Vec::with_capacity(config.hotkeys.len());
    // Vec을 값으로 순회해 Entry를 소유한다. String 필드를 복제 없이 Binding으로 옮길 수 있다.
    for entry in config.hotkeys {
        let chord = parse_chord(&entry.trigger)?;
        // 시스템에 등록하는 트리거에만 보조키를 강제하고 예약된 F12를 막는다.
        if chord.modifiers == 0 || chord.key == 0x7B {
            return Err(format!(
                "Use a modifier and avoid F12 in trigger {:?}.",
                entry.trigger
            ));
        }
        // insert는 이미 같은 키가 있었다면 false. 대소문자와 보조키 순서는 파싱 때 정규화했다.
        if !triggers.insert(chord) {
            return Err(format!("Duplicate trigger {:?}.", entry.trigger));
        }
        // 튜플 패턴으로 text/send 두 필드의 조합을 검사한다. 둘 중 하나만 허용한다.
        let action = match (entry.text, entry.send) {
            (Some(text), None) if !text.is_empty() => Action::Text(text),
            (None, Some(keys)) => Action::Send(parse_chord(&keys)?),
            _ => {
                return Err(format!(
                    "Set exactly one non-empty text or send for {:?}.",
                    entry.trigger
                ))
            }
        };
        bindings.push(Binding {
            chord,
            name: entry.trigger,
            action,
        });
    }
    // 전체 트리거 집합이 완성된 뒤 전송 키와 비교해야 뒤쪽에 정의한 트리거도 잡을 수 있다.
    for binding in &bindings {
        if let Action::Send(output) = &binding.action {
            if triggers.contains(output) {
                return Err(format!(
                    "The output of {:?} is also a registered trigger.",
                    binding.name
                ));
            }
        }
    }
    Ok(bindings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // 배포용 TOML을 컴파일 시 포함해 문서·예제와 실제 파서가 함께 동작하는지 확인한다.
    fn parses_the_shipped_configuration() {
        let bindings = parse(include_str!("../hotkeys.toml")).unwrap();
        assert_eq!(bindings.len(), 3);
        assert_eq!(
            bindings[0].chord,
            Chord {
                modifiers: CTRL | ALT,
                key: b'1'.into()
            }
        );
        assert_eq!(bindings[0].name, "Ctrl+Alt+1");
        assert!(matches!(&bindings[0].action, Action::Text(text) if text == "SELECT * FROM "));
        assert!(matches!(&bindings[1].action, Action::Send(chord) if chord.key == 0x74));
    }

    #[test]
    // 한글과 이모지 보존, 정규화 후 중복, 출력과 트리거의 충돌, 예약 키를 검증한다.
    fn accepts_unicode_text_and_rejects_conflicts() {
        let source = "[[hotkeys]]\ntrigger = 'Alt+Ctrl+K'\ntext = '한글 😀'\n";
        let bindings = parse(source).unwrap();
        assert!(matches!(&bindings[0].action, Action::Text(text) if text == "한글 😀"));
        assert_eq!(bindings[0].chord.modifiers, CTRL | ALT);
        assert!(parse(&format!(
            "{source}[[hotkeys]]\ntrigger = 'ctrl+alt+k'\ntext = 'x'"
        ))
        .is_err());
        assert!(parse(&format!(
            "{source}[[hotkeys]]\ntrigger = 'Ctrl+L'\nsend = 'Ctrl+Alt+K'"
        ))
        .is_err());
        assert!(parse("[[hotkeys]]\ntrigger = 'F5'\ntext = 'x'").is_err());
        assert!(parse("[[hotkeys]]\ntrigger = 'Ctrl+F12'\ntext = 'x'").is_err());
    }
}
