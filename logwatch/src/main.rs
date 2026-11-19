//! CLI와 설정을 합쳐 로그 파일을 선택하고 덤프 또는 실시간 뷰어로 실행한다.

use anyhow::{Context, Result, bail};
// clap 트레이트의 이름을 바꿔 자체 로그 Parser와 구분한다.
use clap::Parser as ClapParser;
use logwatch::{
    app::App,
    config::{Config, Source},
    parser::Parser,
    tail::Tail,
};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};

#[derive(ClapParser)]
#[command(version, about = "Live Java / SQL / XML log viewer for your terminal")]
struct Args {
    /// Log file (overrides configured source path; relative to current directory)
    file: Option<PathBuf>,
    /// TOML configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Select a configured source by name (one file per session)
    #[arg(short, long)]
    source: Option<String>,
    /// Print parsed events once without opening the terminal UI
    #[arg(long)]
    dump: bool,
}
fn main() -> Result<()> {
    let args = Args::parse();
    // as_deref()는 Option<PathBuf>를 Option<&Path>로 빌린다. 명시 설정인지도 따로 전달한다.
    let config = Config::load(
        args.config
            .as_deref()
            .unwrap_or(std::path::Path::new("logwatch.toml")),
        args.config.is_some(),
    )?;
    // --source가 있으면 이름으로 선택하고, 생략하면 첫 설정 또는 기본값을 소유한다.
    let source = if let Some(name) = &args.source {
        config
            .sources
            .into_iter()
            .find(|s| &s.name == name)
            .with_context(|| format!("Unknown source: {name}"))?
    } else {
        config
            .sources
            .into_iter()
            .next()
            .unwrap_or_else(Source::default)
    };
    // 위치 인자로 받은 파일 경로가 소스의 경로에 우선한다. 인코딩과 그룹 규칙은 소스에서 얻는다.
    let path = args.file.unwrap_or(source.path);
    let mut parser = Parser::new(&source.multiline.start_pattern, config.viewer.max_events)
        .context("Invalid multiline start_pattern")?;
    // Tail이 경로를 소유하지만 시작 오류와 화면에도 쓸 경로가 필요해 여기서 복제한다.
    let mut tail = Tail::new(path.clone(), &source.encoding)?;
    let batch = tail
        .initial(config.viewer.initial_lines)
        .with_context(|| format!("Cannot open log file: {}", path.display()))?;
    // Batch의 String은 반복문이 소유하고 Parser에는 한 줄씩 빌려 준다.
    for (number, line) in batch.lines {
        parser.push(&line, number, tail.generation);
    }
    // 스냅샷의 마지막 미완성 줄을 마감하고, 화면 모드로 들어가기 전에 종료한다.
    if args.dump {
        for (number, line) in tail.finish() {
            parser.push(&line, number, tail.generation);
        }
        for event in &parser.events {
            println!(
                "{} {:5} {:4} gen:{} lines:{}-{} {}",
                event.time,
                event.level.label(),
                event.kind.label(),
                event.generation,
                event.first_line,
                event.last_line,
                logwatch::ui::safe(&event.request)
            );
            println!(
                "{}\n",
                logwatch::ui::safe(&logwatch::format::detail(event, true))
            );
        }
        println!("{} events", parser.events.len());
        return Ok(());
    }
    // 대화형 모드에는 입력과 출력 양쪽이 실제 터미널이어야 한다.
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Run in an interactive terminal (ezTerm / PowerShell), or use --dump")
    }
    let mut app = App::new(parser, tail, path.display().to_string(), &config.viewer);
    // ratatui가 터미널을 초기화·복원하고, 클로저가 그 사이의 App 실행을 담당한다.
    ratatui::run(|terminal| app.run(terminal, config.viewer.poll_ms))
}
