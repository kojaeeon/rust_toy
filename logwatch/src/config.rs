use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub viewer: Viewer,
    pub sources: Vec<Source>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
    pub fn load(path: &Path, explicit: bool) -> Result<Self> {
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
        if !["right", "bottom"].contains(&cfg.viewer.layout.as_str()) {
            bail!("viewer.layout must be right or bottom")
        }
        if !(0.2..=0.8).contains(&cfg.viewer.detail_ratio) {
            bail!("detail_ratio must be between 0.2 and 0.8")
        }
        if cfg.viewer.max_events == 0 || cfg.viewer.initial_lines == 0 || cfg.viewer.poll_ms < 20 {
            bail!("max_events and initial_lines must be positive; poll_ms must be >= 20")
        }
        let base = path.parent().unwrap_or(Path::new("."));
        for source in &mut cfg.sources {
            if source.path.is_relative() {
                source.path = base.join(&source.path);
            }
        }
        Ok(cfg)
    }
}
