//! `logwatch.toml` 스키마, 기본값, 경로 해석과 설정 검증을 담당한다.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

// serde(default)는 생략한 필드를 Default 값으로 채운다. 낯선 키는 설정 오류로 처리한다.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub viewer: Viewer,
    pub sources: Vec<Source>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
// 화면 옵션과 데이터 수집·보관 한도를 함께 정의한다.
pub struct Viewer {
    pub layout: String,
    pub detail_ratio: f64,
    pub follow: bool,
    pub max_events: usize,
    pub initial_lines: usize,
    pub poll_ms: u64,
    pub wrap: bool,
}
impl Default for Viewer {
    fn default() -> Self {
        Self {
            layout: "right".into(),
            detail_ratio: 0.6,
            follow: true,
            max_events: 50000,
            initial_lines: 200,
            poll_ms: 200,
            wrap: false,
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
// 설정에 소스가 여럿 있어도 main이 실행마다 하나를 선택한다.
pub struct Source {
    pub name: String,
    pub path: PathBuf,
    pub encoding: String,
    pub multiline: Multiline,
}
impl Default for Source {
    fn default() -> Self {
        Self {
            name: "default".into(),
            path: "logs/application.log".into(),
            encoding: "utf-8".into(),
            multiline: Multiline::default(),
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
// 이벤트의 시작 줄을 감지한다. Java 헤더에서 필드를 추출하는 정규식은 Parser에 별도로 있다.
pub struct Multiline {
    pub start_pattern: String,
}
impl Default for Multiline {
    fn default() -> Self {
        Self {
            start_pattern: r"^\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}".into(),
        }
    }
}
impl Config {
    /// 파일을 읽고 검증한 뒤 소스의 상대 경로를 설정 파일의 부모 기준으로 해석한다.
    pub fn load(path: &Path, explicit: bool) -> Result<Self> {
        // 기본 파일이 없으면 기본값을 사용한다. --config로 요청한 파일이 없으면 오류다.
        let mut cfg: Self = if path.exists() {
            toml::from_str(
                &fs::read_to_string(path).with_context(|| format!("Read {}", path.display()))?,
            )
            .with_context(|| format!("Invalid TOML: {}", path.display()))?
        } else if explicit {
            bail!("Config does not exist: {}", path.display())
        } else {
            Self::default()
        };
        // 파싱된 타입은 유효해도 화면 배치 문자열과 숫자 범위는 추가 검사가 필요하다.
        if !["right", "bottom"].contains(&cfg.viewer.layout.as_str()) {
            bail!("viewer.layout must be right or bottom")
        }
        if !(0.2..=0.8).contains(&cfg.viewer.detail_ratio) {
            bail!("detail_ratio must be between 0.2 and 0.8")
        }
        if cfg.viewer.max_events == 0 || cfg.viewer.initial_lines == 0 || cfg.viewer.poll_ms < 20 {
            bail!("max_events and initial_lines must be positive; poll_ms must be >= 20")
        }
        // 설정 파일의 소스 경로만 이 기준을 사용한다. CLI 위치 인자는 현재 작업 폴더 기준이다.
        let base = path.parent().unwrap_or(Path::new("."));
        for source in &mut cfg.sources {
            if source.path.is_relative() {
                source.path = base.join(&source.path);
            }
        }
        Ok(cfg)
    }
}
