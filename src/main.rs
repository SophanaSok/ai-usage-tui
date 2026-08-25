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
    budget::{AlertDispatcher, BudgetEngine},
    cli::{parse_cli, print_help},
    collector::{
        background::{Collector, CollectorHandle},
        journal::{record_ollama, record_routing},
        load_usage,
        pricing_refresh::refresh_pricing,
        registry,
        zen::refresh_zen_catalog,
        SourceRoots,
    },
    config::{apply_config, ConfigFile},
    export::{csv_field, print_once},
    helpers::{is_broken_pipe, print_line},
    ui::run,
};

fn main() -> Result<()> {
    match dispatch() {
        // A downstream reader closing the pipe — `| head`, `| grep -q`, quitting out of
        // `| less` — is a normal way for a command in a pipeline to end, not a failure.
        Err(error) if is_broken_pipe(&error) => Ok(()),
        other => other,
    }
}

fn dispatch() -> Result<()> {
    let parsed_cli = parse_cli(env::args().skip(1))?;
    if parsed_cli.help {
        print_help()?;
        return Ok(());
    }
    // Before the config is read: these describe the CLI itself and must work regardless of
    // whether a config file exists or parses.
    if let Some(shell) = parsed_cli.completions {
        ai_usage_tui::cli::print_completions(shell)?;
        return Ok(());
    }
    if parsed_cli.man {
        ai_usage_tui::cli::print_man()?;
        return Ok(());
    }
    if parsed_cli.version {
        print_line(&format!("ai-usage-tui {}", env!("CARGO_PKG_VERSION")))?;
        return Ok(());
    }
    let (cli, config) = apply_config(parsed_cli)?;
    // Below `apply_config`, not above it: these two used to run first, which meant `--config`
    // was parsed, accepted and then ignored for them alone -- a mistyped path was an error for
    // every other invocation and silently fine for these.
    if cli.refresh_zen {
        let path = refresh_zen_catalog()?;
        print_line(&format!(
            "Cached OpenCode Zen catalog at {}",
            path.display()
        ))?;
        return Ok(());
    }
    if cli.refresh_pricing {
        let path = refresh_pricing()?;
        print_line(&format!(
            "Refreshed Zen pricing table at {}",
            path.display()
        ))?;
        return Ok(());
    }
    if let Some(path) = ai_usage_tui::logging::log_path() {
        ai_usage_tui::logging::info(
            "main",
            &format!("ai-usage-tui {} starting", env!("CARGO_PKG_VERSION")),
        );
        eprintln!("logging to {}", path.display());
    }
    if cli.doctor {
        return doctor(&cli, &config);
    }
    if cli.record_ollama {
        let path = cli
            .journal_path
            .clone()
            .or_else(ai_usage_tui::utils::journal_path)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "could not determine a home directory; pass an explicit path (see --help)"
                )
            })?;
        return record_ollama(&path);
    }
    if cli.record_routing {
        let path = cli
            .journal_path
            .clone()
            .or_else(ai_usage_tui::utils::journal_path)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "could not determine a home directory; pass an explicit path (see --help)"
                )
            })?;
        return record_routing(&path);
    }
    if cli.check_budgets {
        return check_budgets(&cli, &config);
    }
    if cli.omarchy_record {
        return write_omarchy_records(&cli, &config);
    }
    if cli.routing_json || cli.routing_csv_path.is_some() {
        return export_routing(&cli);
    }
    if cli.once || cli.json {
        return print_once(&cli);
    }

    let budget_engine = budget_engine(&config);
    let dispatcher = AlertDispatcher::new(webhook_url(&cli, &config));
    let collector_handle = build_collectors(&cli, &config);
    run_tui(&cli, collector_handle, budget_engine, dispatcher)
}

fn build_collectors(cli: &ai_usage_tui::cli::Cli, config: &ConfigFile) -> Option<CollectorHandle> {
    let journal = cli
        .journal_path
        .clone()
        .or_else(ai_usage_tui::utils::journal_path)?;
    let roots = SourceRoots::from_cli(cli, journal);

    // One list, iterated. This used to be five near-identical blocks that had to be kept in step
    // by hand with the five reads in `collector::load_usage`; see `collector::registry`.
    let collectors: Vec<Box<dyn Collector>> = registry::SOURCES
        .iter()
        .filter(|spec| roots.is_enabled(spec))
        .map(|spec| {
            let interval = config
                .collectors
                .as_ref()
                .and_then(|collectors| collectors.get(spec.id))
                .and_then(|cfg| cfg.interval)
                .unwrap_or(spec.default_interval);
            (spec.collector)(&roots, interval)
        })
        .collect();

    if collectors.is_empty() {
        None
    } else {
        Some(CollectorHandle::spawn(collectors))
    }
}

fn run_tui(
    cli: &ai_usage_tui::cli::Cli,
    collector: Option<CollectorHandle>,
    budget_engine: BudgetEngine,
    dispatcher: AlertDispatcher,
) -> Result<()> {
    // Restore the terminal from inside the panic hook, before the default hook prints.
    //
    // `catch_unwind` below already restores it -- but it runs after the unwind has begun, and by
    // then the default hook has written the panic message and location to stderr *while the
    // alternate screen is still up*. `LeaveAlternateScreen` then discards that scrollback, so a
    // user who hit a panic saw a clean prompt and no explanation. The hook runs first, so the
    // message lands on the real screen.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort and deliberately ignoring errors: we are already panicking, and a failed
        // restore must not panic again inside the hook.
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let result = catch_unwind(AssertUnwindSafe(|| {
        run(&mut terminal, cli, collector, budget_engine, dispatcher)
    }));
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    match result {
        Ok(result) => result,
        Err(payload) => resume_unwind(payload),
    }
}

fn budget_engine(config: &ConfigFile) -> BudgetEngine {
    match &config.budgets {
        Some(budgets) => BudgetEngine::from_config(budgets),
        None => BudgetEngine::empty(),
    }
}

/// The alert webhook, with the `--webhook` flag overriding `[budgets] webhook` in config.
fn webhook_url(cli: &ai_usage_tui::cli::Cli, config: &ConfigFile) -> Option<String> {
    cli.webhook_url.clone().or_else(|| {
        config
            .budgets
            .as_ref()
            .and_then(|budgets| budgets.webhook.clone())
    })
}

fn check_budgets(cli: &ai_usage_tui::cli::Cli, config: &ConfigFile) -> Result<()> {
    let budget_engine = budget_engine(config);
    if budget_engine.is_empty() {
        print_line("{\"budgets\": 0, \"alerts\": []}")?;
        return Ok(());
    }

    let journal = cli
        .journal_path
        .clone()
        .or_else(ai_usage_tui::utils::journal_path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine a home directory; set AI_USAGE_JOURNAL_PATH or pass --journal"
            )
        })?;

    let (usages, _) = load_usage(&SourceRoots::from_cli(cli, journal))?;
    let alerts = budget_engine.check(&usages);
    let has_alerts = alerts.iter().any(|a| a.is_actionable());

    let json = serde_json::json!({
        "budgets": budget_engine.budgets().len(),
        "alerts": alerts
            .iter()
            .filter(|a| a.is_actionable())
            .map(|a| a.to_json())
            .collect::<Vec<_>>(),
    });
    print_line(&serde_json::to_string_pretty(&json)?)?;

    if has_alerts {
        // Report a failed dispatch rather than swallowing it: a webhook that silently never
        // fires is indistinguishable from a budget that never trips.
        let mut dispatcher = AlertDispatcher::new(webhook_url(cli, config));
        if let Err(error) = dispatcher.dispatch(&alerts) {
            eprintln!("warning: budget webhook dispatch failed: {}", error);
        }
        // `std::process::exit` here skipped every destructor on the way out, including the
        // collector handle's thread join. Signal the exit code by unwinding instead.
        return Err(BudgetsExceeded(alerts.iter().filter(|a| a.is_actionable()).count()).into());
    }
    Ok(())
}

/// Write one record per configured id into Omarchy's usage directory.
///
/// Budgets are checked over the tool's whole usage — every source, deduplicated and priced —
/// so the meters agree with `--check-budgets` and the dashboard; only the token tallies are
/// this tab's own rows.
fn write_omarchy_records(cli: &ai_usage_tui::cli::Cli, config: &ConfigFile) -> Result<()> {
    use ai_usage_tui::budget::{BudgetPeriod, BudgetScope};
    use ai_usage_tui::omarchy::record::{build_record, write_record, RecordSpec, ALLOWED_IDS};

    let journal = cli
        .journal_path
        .clone()
        .or_else(ai_usage_tui::utils::journal_path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine a home directory; set AI_USAGE_JOURNAL_PATH or pass --journal"
            )
        })?;
    let roots = SourceRoots::from_cli(cli, journal.clone());
    let dir = roots.omarchy_usage_dir().ok_or_else(|| {
        anyhow::anyhow!("could not determine Omarchy's usage directory; pass --omarchy-dir")
    })?;
    let omarchy = config.omarchy.clone().unwrap_or_default();
    let ids: Vec<String> = omarchy
        .records
        .clone()
        .unwrap_or_else(|| vec!["opencode".to_string()]);

    let (all_usages, _) = load_usage(&roots)?;
    let alerts = budget_engine(config).check(&all_usages);
    let wanted = omarchy
        .balance_budget
        .as_deref()
        .unwrap_or("global/monthly")
        .to_ascii_lowercase();
    let balance = if omarchy.balance.unwrap_or(false) {
        let matches = |alert: &&ai_usage_tui::budget::Alert| {
            format!("{}/{}", alert.scope.label(), alert.period.label()).to_ascii_lowercase()
                == wanted
        };
        alerts
            .iter()
            .find(matches)
            .or_else(|| {
                alerts
                    .iter()
                    .find(|a| a.scope == BudgetScope::Global && a.period == BudgetPeriod::Monthly)
            })
            .or_else(|| alerts.iter().find(|a| a.scope == BudgetScope::Global))
            .or_else(|| alerts.first())
    } else {
        None
    };

    let engine = ai_usage_tui::pricing::PricingEngine::load();
    let now = ai_usage_tui::utils::now();
    for id in &ids {
        anyhow::ensure!(
            ALLOWED_IDS.contains(&id.as_str()),
            "record id {id:?} is not one this tool may write"
        );
        let (mut rows, name) = match id.as_str() {
            "opencode" => (
                ai_usage_tui::collector::opencode::load_opencode(roots.db_path.as_deref())?.0,
                "OpenCode",
            ),
            _ => (
                ai_usage_tui::collector::journal::load_journal(&roots.journal)?
                    .into_iter()
                    .filter(|u| u.provider.eq_ignore_ascii_case("ollama"))
                    .collect(),
                "Ollama",
            ),
        };
        ai_usage_tui::pricing::apply_estimated_pricing(&mut rows, &engine);
        let record = build_record(&RecordSpec {
            id,
            name,
            rows: &rows,
            alerts: &alerts,
            balance,
            now,
        });
        let path = write_record(&dir, &record)?;
        print_line(&format!(
            "Wrote Omarchy record {} ({} requests, {} budget meters)",
            path.display(),
            record.total_prompts,
            record.limits.len()
        ))?;
    }
    Ok(())
}

/// Report what each source resolved to, so "the dashboard is empty" is a question with an answer.
///
/// Every line comes from the same traversal the dashboard and the exporters use
/// (`collector::diagnose`), so this can never describe a set of sources the rest of the tool
/// does not actually read. It reads exactly what a normal collection reads, and writes nothing.
fn doctor(cli: &ai_usage_tui::cli::Cli, config: &ConfigFile) -> Result<()> {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "ai-usage-tui {}", env!("CARGO_PKG_VERSION"));

    let journal = cli
        .journal_path
        .clone()
        .or_else(ai_usage_tui::utils::journal_path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine a home directory; set AI_USAGE_JOURNAL_PATH or pass --journal"
            )
        })?;
    let roots = SourceRoots::from_cli(cli, journal);
    let reports = ai_usage_tui::collector::diagnose(&roots)?;

    let _ = writeln!(out, "\nSOURCES");
    let mut found_any = false;
    for report in &reports {
        let mark = if report.present { "found " } else { "absent" };
        let where_ = report
            .path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<no home directory>".to_string());
        let rows = if report.present {
            format!("{:>7} rows", report.rows)
        } else {
            " ".repeat(12)
        };
        found_any |= report.rows > 0;
        let _ = writeln!(out, "  {:<12} {} {}  {}", report.id, mark, rows, where_);
        // Only where there is something to bill: on a machine without Codex, "billing unknown"
        // is noise about a source that is not there.
        if report.present {
            if let Some(detail) = &report.detail {
                let _ = writeln!(out, "  {:<12} {:<19}  {}", "", "", detail);
            }
        }
        if !report.present {
            if let Some(hint) = absence_hint(report.id) {
                let _ = writeln!(out, "  {:<12} {:<19}  {}", "", "", hint);
            }
        }
    }

    let _ = writeln!(out, "\nCONFIG");
    let config_path = cli
        .config_path
        .clone()
        .or_else(ai_usage_tui::config::config_path);
    match &config_path {
        Some(path) if path.exists() => {
            let _ = writeln!(out, "  loaded       {}", path.display());
        }
        Some(path) => {
            let _ = writeln!(out, "  none         {} (not present)", path.display());
            let _ = writeln!(
                out,
                "               copy examples/config.toml there to configure budgets and collectors"
            );
        }
        None => {
            let _ = writeln!(out, "  none         <no config directory>");
        }
    }
    let budgets = config.budgets.as_ref().map_or(0, |b| b.entry.len());
    let _ = writeln!(out, "  budgets      {budgets} configured");
    match ai_usage_tui::logging::log_path() {
        Some(path) => {
            let _ = writeln!(out, "  log          {}", path.display());
        }
        None => {
            let _ = writeln!(out, "  log          off (set AI_USAGE_LOG=1 or a path)");
        }
    }

    // How this copy was installed, and how to upgrade it. Always shown and always offline: it is
    // read off the binary's own path. Seven install channels ship, and until this existed the
    // tool knew nothing about which one it came from -- so a user who installed with brew and
    // upgraded with cargo ended up with two binaries and no idea which was on PATH.
    let _ = writeln!(out, "\nTHIS BUILD");
    let (exe, channel) = ai_usage_tui::update::current_channel();
    match &exe {
        Some(path) => {
            let _ = writeln!(out, "  path         {}", path.display());
        }
        None => {
            let _ = writeln!(out, "  path         <could not be determined>");
        }
    }
    let _ = writeln!(out, "  installed by {}", channel.label());
    match channel.upgrade_command() {
        Some(command) => {
            let _ = writeln!(out, "  upgrade      {command}");
        }
        None => {
            // Naming a command for a location we do not recognise is worse than saying nothing:
            // running it would install a second copy elsewhere on PATH, and the user would be
            // upgrading a binary they are not running.
            let _ = writeln!(
                out,
                "  upgrade      download the release that matches how this was installed:"
            );
            let _ = writeln!(out, "               {}", ai_usage_tui::update::RELEASES_URL);
        }
    }

    // The only part that needs the network, and the only part that is opt-in. Off by default,
    // exactly as `zen_pricing` is, and never reached by any command but this one.
    let check = config.update.as_ref().is_some_and(|u| u.check);
    if check {
        match ai_usage_tui::update::latest_release_tag() {
            Ok(tag) if ai_usage_tui::update::is_newer(env!("CARGO_PKG_VERSION"), &tag) => {
                let _ = writeln!(out, "  latest       {tag} — newer than this build");
            }
            Ok(tag) => {
                let _ = writeln!(out, "  latest       {tag} — up to date");
            }
            // Reported, not swallowed. A check that silently returns nothing is
            // indistinguishable from one that found nothing newer.
            Err(error) => {
                let _ = writeln!(out, "  latest       could not be checked: {error}");
            }
        }
    } else {
        let _ = writeln!(
            out,
            "  latest       not checked (set [update] check = true to look; needs the network)"
        );
    }

    if !found_any {
        let _ = writeln!(
            out,
            "\nNo usage was found in any source. That is the expected result on a machine that has\n\
             not yet run OpenCode, Claude Code, Codex or a journaled Ollama request -- the paths\n\
             above are where each one would be read from."
        );
    }

    print_line(out.trim_end())?;
    Ok(())
}

/// How to point a source somewhere else, printed only when it was not where we looked.
fn absence_hint(id: &str) -> Option<&'static str> {
    match id {
        "opencode" => Some("point elsewhere with --db PATH or OPENCODE_DB_PATH"),
        "claude_code" => Some("point elsewhere with --claude-dir PATH or CLAUDE_PROJECTS_DIR"),
        "codex" => Some("point elsewhere with --codex-dir PATH or CODEX_HOME"),
        // The only source that records nothing until the user turns it on, so this hint is the
        // difference between "empty" and "unusable".
        "gemini" => Some(concat!(
            "records nothing until Gemini CLI's telemetry is on. In ~/.gemini/settings.json: ",
            r#"{"telemetry":{"enabled":true,"target":"local","outfile":"~/.gemini/telemetry.json"}}"#,
        )),
        "journal" => {
            Some("written by --record-ollama and --record-routing; nothing to do if unused")
        }
        "zen_pricing" => {
            Some("optional; populate with --refresh-zen (pricing still works without it)")
        }
        _ => None,
    }
}

/// Budget breach as an error, so the non-zero exit runs destructors like any other failure.
#[derive(Debug)]
struct BudgetsExceeded(usize);

impl std::fmt::Display for BudgetsExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} budget threshold(s) exceeded", self.0)
    }
}

impl std::error::Error for BudgetsExceeded {}

fn export_routing(cli: &ai_usage_tui::cli::Cli) -> Result<()> {
    let journal = cli
        .journal_path
        .clone()
        .or_else(ai_usage_tui::utils::journal_path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine a home directory; set AI_USAGE_JOURNAL_PATH or pass --journal"
            )
        })?;

    let events = ai_usage_tui::collector::journal::load_routing(&journal)?;
    let aggregates = ai_usage_tui::routing::aggregate(&events);

    if let Some(path) = &cli.routing_csv_path {
        // The four provenance columns are **appended**, never inserted, so a consumer reading by
        // index keeps working. `cost` keeps its position and its meaning changes from a total to
        // a floor -- which is why `priced_tasks` sits beside it: a reader who takes `cost` alone
        // now has a column that tells them whether they may.
        let mut csv = String::from(
            "agent,model,provider,tasks,tokens,cost,retries,escalations,test_passes,test_failures,review_defects,priced_tasks,unpriced_tasks,quota_tasks,free_tasks\n",
        );
        for agg in &aggregates {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_field(&agg.agent),
                csv_field(&agg.model),
                csv_field(&agg.provider),
                agg.tasks,
                agg.tokens,
                agg.cost,
                agg.retries,
                agg.escalations,
                agg.test_passes,
                agg.test_failures,
                agg.review_defects,
                agg.priced_tasks,
                agg.unpriced_tasks,
                agg.quota_tasks,
                agg.free_tasks,
            ));
        }
        std::fs::write(path, csv)?;
        print_line(&format!("Wrote routing CSV to {}", path.display()))?;
    } else {
        let rows: Vec<_> = aggregates
            .iter()
            .map(|agg| {
                serde_json::json!({
                    "agent": agg.agent,
                    "model": agg.model,
                    "provider": agg.provider,
                    "tasks": agg.tasks,
                    "tokens": agg.tokens,
                    // `null` rather than `0` when nothing was priced, for the same reason the
                    // usage export keeps an unknown cost null: a script that sums this column
                    // must not be handed a zero it cannot distinguish from a free model.
                    "cost": if agg.priced_tasks == 0 { None } else { Some(agg.cost) },
                    "priced_tasks": agg.priced_tasks,
                    "unpriced_tasks": agg.unpriced_tasks,
                    "quota_tasks": agg.quota_tasks,
                    "free_tasks": agg.free_tasks,
                    // What `cost_per_success` is standing on, in the same words the panel uses,
                    // so a script and the dashboard cannot disagree about one aggregate.
                    "cost_per_success": ai_usage_tui::routing::cost_per_success_sort_key(agg),
                    "cost_basis": ai_usage_tui::routing::cost_basis_label(agg),
                    "retries": agg.retries,
                    "escalations": agg.escalations,
                    "test_passes": agg.test_passes,
                    "test_failures": agg.test_failures,
                    "review_defects": agg.review_defects,
                    "retry_rate": ai_usage_tui::routing::retry_rate(agg),
                    "escalation_rate": ai_usage_tui::routing::escalation_rate(agg),
                    "defect_rate": ai_usage_tui::routing::defect_rate(agg),
                })
            })
            .collect();
        print_line(&serde_json::to_string_pretty(&serde_json::json!({
            "source": format!("journal: {}", journal.display()),
            "events": events.len(),
            "aggregates": rows
        }))?)?;
    }
    Ok(())
}
