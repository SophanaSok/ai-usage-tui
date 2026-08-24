//! Render every panel to SVG, off-screen.
//!
//! Run through `scripts/render-readme-screenshots.sh`, which builds the demo dataset first and
//! rasterises the output to PNG. See `src/ui/svg.rs` for why the README images are generated
//! rather than photographed.
//!
//!   cargo run --example render-screenshots -- <out-dir> --claude-dir <dir> --journal <path> \
//!       --config <path> [--db <path>]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use ai_usage_tui::budget::BudgetEngine;
use ai_usage_tui::cli::Cli;
use ai_usage_tui::config::load_config;
use ai_usage_tui::model::Range;
use ai_usage_tui::ui::{render_svg, App, Panel};

/// The geometry the README images are sized for. 132 columns is comfortably above the 110-column
/// threshold where the footer switches to its compact form, so the images show the full key line.
const COLS: u16 = 132;
const ROWS: u16 = 38;

/// Written into the header in place of the wall clock. Without it, two runs over identical data
/// produce different bytes and every regeneration shows up as a diff.
const FIXED_CLOCK: &str = "14:07:22";

const PANELS: [(&str, Panel); 7] = [
    ("dashboard", Panel::Models),
    ("budgets", Panel::Budgets),
    ("routing", Panel::Routing),
    ("projects", Panel::Projects),
    ("timeseries", Panel::TimeSeries),
    ("burn", Panel::Burn),
    ("sessions", Panel::Sessions),
];

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(out_dir) = args.next().map(PathBuf::from) else {
        eprintln!("usage: render-screenshots <out-dir> [--claude-dir D] [--journal P] [--config P] [--db P]");
        return ExitCode::FAILURE;
    };

    let mut cli = Cli {
        range: Range::All,
        refresh_interval: Duration::from_secs(60),
        ..Cli::default()
    };
    while let Some(flag) = args.next() {
        let Some(value) = args.next().map(PathBuf::from) else {
            eprintln!("{flag} needs a value");
            return ExitCode::FAILURE;
        };
        match flag.as_str() {
            "--claude-dir" => cli.claude_dir = Some(value),
            "--codex-dir" => cli.codex_dir = Some(value),
            "--journal" => cli.journal_path = Some(value),
            "--config" => cli.config_path = Some(value),
            "--db" => cli.db_path = Some(value),
            other => {
                eprintln!("unknown flag {other}");
                return ExitCode::FAILURE;
            }
        }
    }

    // Every source is required, and none of them may be inferred. Left unset, the collectors
    // discover the machine's real OpenCode database and Claude Code transcripts, and the images
    // become a picture of the author's own spend — the exact thing the demo fixture exists to
    // prevent. This was not hypothetical: the first run of this renderer did it.
    let (Some(journal), Some(_), Some(_), Some(_)) = (
        cli.journal_path.clone(),
        cli.claude_dir.as_ref(),
        cli.codex_dir.as_ref(),
        cli.db_path.as_ref(),
    ) else {
        eprintln!("--journal, --claude-dir, --codex-dir and --db are all required: unset, they");
        eprintln!(
            "fall back to this machine's real usage data, which must never reach a README image"
        );
        return ExitCode::FAILURE;
    };

    let budgets = match load_config(&cli) {
        Ok(config) => config
            .budgets
            .as_ref()
            .map_or_else(BudgetEngine::empty, BudgetEngine::from_config),
        Err(error) => {
            eprintln!("could not read {:?}: {error}", cli.config_path);
            return ExitCode::FAILURE;
        }
    };

    let mut app = App::new(
        ai_usage_tui::collector::SourceRoots::from_cli(&cli, journal),
        cli.range,
        cli.refresh_interval,
        None,
        None,
        None,
        budgets,
        None,
    );
    app.last_refresh = FIXED_CLOCK.to_string();

    if app.usages.is_empty() {
        eprintln!("no usage loaded — the images would all be empty; check --claude-dir");
        return ExitCode::FAILURE;
    }

    if let Err(error) = std::fs::create_dir_all(&out_dir) {
        eprintln!("could not create {}: {error}", out_dir.display());
        return ExitCode::FAILURE;
    }
    for (name, panel) in PANELS {
        app.panel = panel;
        let path = out_dir.join(format!("{name}.svg"));
        if let Err(error) = std::fs::write(&path, render_svg(&app, COLS, ROWS)) {
            eprintln!("could not write {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
        println!("{}", path.display());
    }
    ExitCode::SUCCESS
}
