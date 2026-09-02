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

/// The geometry the README images are sized for. 132 columns is comfortably wider than the
/// footer's full key line (120 today; `ui::keys::footer_forms` is the source), so the images
/// show every hint rather than the folded form.
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
        eprintln!("usage: render-screenshots <out-dir> --claude-dir D --codex-dir D --omarchy-dir D --journal P --db P [--config P]");
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
            "--copilot-dir" => cli.copilot_dir = Some(value),
            "--gemini-dir" => cli.gemini_dir = Some(value),
            "--omarchy-dir" => cli.omarchy_dir = Some(value),
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
    // prevent. This was not hypothetical: the first run of this renderer did it. Omarchy's
    // records are the fifth source and were the second time: left unpinned, the header carried
    // the author's real rate-limit window and the billing decision read their plan tier, so every
    // Claude row in the demo went `quota` and the images said so. The update cache was the third,
    // and it has no flag because there is nothing to point elsewhere: see `update_notice` below.
    let (Some(journal), Some(_), Some(_), Some(_), Some(_), Some(_), Some(_)) = (
        cli.journal_path.clone(),
        cli.claude_dir.as_ref(),
        cli.codex_dir.as_ref(),
        cli.copilot_dir.as_ref(),
        cli.gemini_dir.as_ref(),
        cli.omarchy_dir.as_ref(),
        cli.db_path.as_ref(),
    ) else {
        eprintln!(
            "--journal, --claude-dir, --codex-dir, --copilot-dir, --gemini-dir, --omarchy-dir and \
             --db are all required: unset,"
        );
        eprintln!(
            "they fall back to this machine's real usage data, which must never reach a README image"
        );
        return ExitCode::FAILURE;
    };

    // Three more inputs have no flag at all, because nothing points them elsewhere: the
    // statusline cache, the update cache and a refreshed pricing cache all resolve from the real
    // `XDG_DATA_HOME` or `HOME`, and `App::new` reads every one of them. The update cache was the
    // third leak into these images (cleared below); the statusline cache would have been the
    // fourth -- the author's own rate-limit window in every header, from the day the statusline
    // entry was installed. So the data root is pinned the way the sources are: required, never
    // inferred. `scripts/render-readme-screenshots.sh` points it at a scratch directory.
    if std::env::var_os("XDG_DATA_HOME").is_none_or(|value| value.is_empty()) {
        eprintln!(
            "XDG_DATA_HOME must name a scratch directory: unset, the statusline, update and \
             pricing caches"
        );
        eprintln!("fall back to this machine's real data directory, which must never reach a README image");
        return ExitCode::FAILURE;
    }

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
    // `App::new` reads the update cache an opted-in `--doctor` leaves behind, so on a machine
    // where one exists every image would carry `↑ vX.Y.Z` in its header — a fact about the
    // author's install, pinned into the README until the next regeneration, and one that would
    // read as a claim about the release being documented. Cleared for the same reason the clock
    // is pinned.
    app.update_notice = None;

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
