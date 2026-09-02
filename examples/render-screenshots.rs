//! Render every panel to SVG, off-screen -- and, given a key script, the frames of the demo.
//!
//! Run through `scripts/render-readme-screenshots.sh`, which builds the demo dataset first and
//! rasterises the output to PNG and the frames to a GIF. See `src/ui/svg.rs` for why the README
//! images are generated rather than photographed.
//!
//!   cargo run --example render-screenshots -- <out-dir> --claude-dir <dir> --journal <path> \
//!       --config <path> [--db <path>] [--script "t,hold,>,o,p,down,enter,esc,l,?"]
//!
//! The script is the demo: one token per key, replayed through the same dispatch the dashboard's
//! event loop uses (`App::apply`), with a frame written after each. A single character is looked
//! up in the key table; `enter`, `esc`, `down` and `up` are the keys the loop handles beside the
//! table; `hold` writes the previous frame again, which the GIF assembler turns into a pause.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use ai_usage_tui::budget::BudgetEngine;
use ai_usage_tui::cli::Cli;
use ai_usage_tui::config::load_config;
use ai_usage_tui::model::Range;
use ai_usage_tui::ui::keys::{action_for, Action};
use ai_usage_tui::ui::{render_svg, App, Flow, Panel};

/// The geometry the README images are sized for. 132 columns is comfortably wider than the
/// footer's full key line (120 today; `ui::keys::footer_forms` is the source), so the images
/// show every hint rather than the folded form.
const COLS: u16 = 132;
const ROWS: u16 = 38;

/// Written into the header in place of the wall clock. Without it, two runs over identical data
/// produce different bytes and every regeneration shows up as a diff.
const FIXED_CLOCK: &str = "14:07:22";

const PANELS: [(&str, Panel); 8] = [
    ("dashboard", Panel::Models),
    ("budgets", Panel::Budgets),
    ("routing", Panel::Routing),
    ("projects", Panel::Projects),
    ("timeseries", Panel::TimeSeries),
    ("burn", Panel::Burn),
    ("sessions", Panel::Sessions),
    ("limits", Panel::Limits),
];

/// One token of the demo script.
enum Step {
    Key(Action),
    Hold,
}

/// Parse the script, refusing anything the key table does not know. A typo that silently did
/// nothing would leave a frame missing from the demo with no sign of which one.
fn parse_script(script: &str) -> Result<Vec<Step>, String> {
    script
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| match token {
            "hold" => Ok(Step::Hold),
            "enter" => Ok(Step::Key(Action::DrillIn)),
            "esc" => Ok(Step::Key(Action::Back)),
            "down" => Ok(Step::Key(Action::SelectNext)),
            "up" => Ok(Step::Key(Action::SelectPrev)),
            _ => {
                let mut chars = token.chars();
                match (chars.next(), chars.next()) {
                    (Some(key), None) => action_for(key)
                        .map(Step::Key)
                        .ok_or_else(|| format!("`{key}` is not a dashboard key")),
                    _ => Err(format!("`{token}` is neither a key nor a script word")),
                }
            }
        })
        .collect()
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(out_dir) = args.next().map(PathBuf::from) else {
        eprintln!("usage: render-screenshots <out-dir> --claude-dir D --codex-dir D --omarchy-dir D --journal P --db P [--config P] [--script S]");
        return ExitCode::FAILURE;
    };

    let mut cli = Cli {
        range: Range::All,
        refresh_interval: Duration::from_secs(60),
        ..Cli::default()
    };
    let mut script = None;
    while let Some(flag) = args.next() {
        let Some(value) = args.next() else {
            eprintln!("{flag} needs a value");
            return ExitCode::FAILURE;
        };
        let path = PathBuf::from(&value);
        match flag.as_str() {
            "--claude-dir" => cli.claude_dir = Some(path),
            "--codex-dir" => cli.codex_dir = Some(path),
            "--copilot-dir" => cli.copilot_dir = Some(path),
            "--gemini-dir" => cli.gemini_dir = Some(path),
            "--omarchy-dir" => cli.omarchy_dir = Some(path),
            "--journal" => cli.journal_path = Some(path),
            "--config" => cli.config_path = Some(path),
            "--db" => cli.db_path = Some(path),
            "--script" => script = Some(value),
            other => {
                eprintln!("unknown flag {other}");
                return ExitCode::FAILURE;
            }
        }
    }
    let steps = match script.as_deref().map(parse_script) {
        Some(Ok(steps)) => steps,
        Some(Err(error)) => {
            eprintln!("--script: {error}");
            return ExitCode::FAILURE;
        }
        None => Vec::new(),
    };

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
    // inferred. `scripts/render-readme-screenshots.sh` points it at a scratch directory, and
    // seeds that directory's statusline cache from the fixture so the limits panel has something
    // invented to show.
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
    let write = |path: &PathBuf, svg: String| -> bool {
        if let Err(error) = std::fs::write(path, svg) {
            eprintln!("could not write {}: {error}", path.display());
            return false;
        }
        println!("{}", path.display());
        true
    };
    for (name, panel) in PANELS {
        app.panel = panel;
        if !write(
            &out_dir.join(format!("{name}.svg")),
            render_svg(&app, COLS, ROWS),
        ) {
            return ExitCode::FAILURE;
        }
    }

    // The demo starts where the dashboard starts, then replays the script. A frame is written
    // before the first key so the GIF opens on the model list rather than mid-gesture. `Quit`
    // ends the script early: nothing after it would be a frame a user could see.
    app.panel = Panel::Models;
    app.selected = 0;
    app.show_help = false;
    let mut frame = 0usize;
    let mut last = render_svg(&app, COLS, ROWS);
    if steps.is_empty() {
        return ExitCode::SUCCESS;
    }
    if !write(&out_dir.join(format!("frame-{frame:03}.svg")), last.clone()) {
        return ExitCode::FAILURE;
    }
    for step in steps {
        frame += 1;
        match step {
            Step::Hold => {}
            Step::Key(action) => {
                // `Refresh` would re-read the sources -- harmless here, but the frame would not
                // change, and a script that leans on it is asking for the clock.
                if app.apply(action) == Flow::Quit {
                    break;
                }
                last = render_svg(&app, COLS, ROWS);
            }
        }
        if !write(&out_dir.join(format!("frame-{frame:03}.svg")), last.clone()) {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
