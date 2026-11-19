use anyhow::{Context, Result, bail};
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
    let config = Config::load(
        args.config
            .as_deref()
            .unwrap_or(std::path::Path::new("logwatch.toml")),
        args.config.is_some(),
    )?;
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
    let path = args.file.unwrap_or(source.path);
    let mut parser = Parser::new(&source.multiline.start_pattern, config.viewer.max_events)
        .context("Invalid multiline start_pattern")?;
    let mut tail = Tail::new(path.clone(), &source.encoding)?;
    let batch = tail
        .initial(config.viewer.initial_lines)
        .with_context(|| format!("Cannot open log file: {}", path.display()))?;
    for (number, line) in batch.lines {
        parser.push(&line, number, tail.generation);
    }
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
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Run in an interactive terminal (ezTerm / PowerShell), or use --dump")
    }
    let mut app = App::new(parser, tail, path.display().to_string(), &config.viewer);
    ratatui::run(|terminal| app.run(terminal, config.viewer.poll_ms))
}
