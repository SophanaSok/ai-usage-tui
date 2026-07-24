use std::env;
use std::io::stdout;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use ai_usage_tui::{
    cli::{parse_cli, print_help},
    collector::{journal::record_ollama, pricing_refresh::refresh_pricing, zen::refresh_zen_catalog},
    config::apply_config,
    export::print_once,
    ui::run,
};

fn main() -> Result<()> {
    let parsed_cli = parse_cli(env::args().skip(1))?;
    if parsed_cli.help {
        print_help();
        return Ok(());
    }
    if parsed_cli.version {
        println!("ai-usage-tui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if parsed_cli.refresh_zen {
        let path = refresh_zen_catalog()?;
        println!("Cached OpenCode Zen catalog at {}", path.display());
        return Ok(());
    }
    if parsed_cli.refresh_pricing {
        let path = refresh_pricing()?;
        println!("Refreshed Zen pricing table at {}", path.display());
        return Ok(());
    }
    let cli = apply_config(parsed_cli)?;
    if cli.record_ollama {
        let path = cli
            .journal_path
            .clone()
            .or_else(ai_usage_tui::utils::journal_path)
            .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        return record_ollama(&path);
    }
    if cli.once || cli.json {
        return print_once(&cli);
    }

    run_tui(&cli)
}

fn run_tui(cli: &ai_usage_tui::cli::Cli) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let result = catch_unwind(AssertUnwindSafe(|| run(&mut terminal, cli)));
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    match result {
        Ok(result) => result,
        Err(payload) => resume_unwind(payload),
    }
}
