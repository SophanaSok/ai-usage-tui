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
    if cli.claude_code_hook {
        let path = cli
            .journal_path
            .clone()
            .or_else(ai_usage_tui::utils::journal_path)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "could not determine a home directory; pass an explicit path (see --help)"
                )
            })?;
        return ai_usage_tui::harness::claude_code::record_from_stdin(&SourceRoots::from_cli(
            &cli, path,
        ));
    }
    if cli.statusline {
        // Runs on every redraw of Claude Code's status line, so it reads stdin and the clock and
        // touches nothing else: no sources, no journal, no config document.
        return ai_usage_tui::statusline::run_from_stdin(ai_usage_tui::utils::now());
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
    let (reports, usages) = ai_usage_tui::collector::diagnose_with_usage(&roots)?;

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

    // Cursor is the one agent people ask about that is deliberately absent from SOURCES.
    // Saying nothing reads as an oversight; saying this reads as the decision it is.
    if cursor_is_installed() {
        let _ = writeln!(
            out,
            "  {:<12} installed, not read -- Cursor keeps no reliable local token counts",
            "cursor"
        );
        for line in [
            "its own team calls the stored counts best-effort and points at the web",
            "dashboard; the only way to make a row is to guess one from message length",
        ] {
            let _ = writeln!(out, "  {:<12} {:<19}  {}", "", "", line);
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

    // The pricing table's own state. `PricingEngine::load` collected warnings about the cache —
    // unreadable, invalid, too old to trust — and nothing printed them, so the fallback to
    // bundled rates happened in silence and an `UNKNOWN COST` row had no explanation anywhere.
    let _ = writeln!(out, "\nPRICING");
    let engine = ai_usage_tui::pricing::PricingEngine::load();
    // The count is the loaded table's: the bundled one, plus the cache below when it is in use.
    let _ = writeln!(out, "  models       {} priced", engine.model_count());
    match ai_usage_tui::collector::pricing_refresh::pricing_cache_path() {
        Some(path) if path.exists() => {
            let days = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .map(|age| age.as_secs() / 86_400);
            // A cache the engine refused — too old, unreadable, invalid — names its path in a
            // warning; a cache line that read as "in use" beside that warning was two answers.
            let shown = path.display().to_string();
            let disposition = if engine.warnings().iter().any(|w| w.contains(&shown)) {
                "ignored — see warning"
            } else {
                "in use"
            };
            match days {
                Some(days) => {
                    let _ = writeln!(
                        out,
                        "  cache        {shown} ({days} days old; {disposition})"
                    );
                }
                None => {
                    let _ = writeln!(out, "  cache        {shown} ({disposition})");
                }
            }
        }
        Some(path) => {
            let _ = writeln!(
                out,
                "  cache        none at {} (--refresh-pricing writes it; bundled rates work \
                 without it)",
                path.display()
            );
        }
        None => {
            let _ = writeln!(out, "  cache        <no data directory>");
        }
    }
    if engine.warnings().is_empty() {
        let _ = writeln!(out, "  warnings     none");
    } else {
        for warning in engine.warnings() {
            let _ = writeln!(out, "  warning      {warning}");
        }
    }

    // Where the numbers came from. A total is only as good as its worst-provenance row, and
    // until this existed the only way to see that split was the header's single coverage
    // percentage -- which says how much is priced, not how much was actually reported.
    let provenance = ai_usage_tui::model::Provenance::of(&usages);
    let _ = writeln!(out, "\nPROVENANCE");
    for (status, bucket) in &provenance.buckets {
        if bucket.rows == 0 {
            continue;
        }
        let money = match (bucket.cost, bucket.api_equivalent_cost) {
            (Some(cost), _) => format!("${cost:.2}"),
            // Never rendered as a dollar total: this is what the work would have cost on an
            // API key, not what anyone was charged.
            (None, Some(equivalent)) => format!("${equivalent:.2} at list rates"),
            (None, None) => "no price exists".to_string(),
        };
        let _ = writeln!(
            out,
            "  {:<12} {:>7} requests  {money}",
            status.label(),
            bucket.requests
        );
    }
    match provenance.reported_share() {
        Some(share) => {
            let _ = writeln!(
                out,
                "  {:<12} {share:.0}% of ${:.2} was reported by the provider; the rest is worked \
                 out from rate tables",
                "share",
                provenance.billable_cost()
            );
        }
        None => {
            let _ = writeln!(
                out,
                "  {:<12} no billable spend in range, so no share to report",
                "share"
            );
        }
    }

    // Where the `l` panel's rows come from. None of the three is a usage collector, so none is
    // in SOURCES, and an empty panel was a question with no answer here.
    let _ = writeln!(out, "\nLIMITS");
    if roots.limits_enabled {
        let mark = |present: bool| if present { "found " } else { "absent" };
        let named = |path: &Option<std::path::PathBuf>| {
            path.as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<no home directory>".to_string())
        };
        let omarchy = roots.omarchy_usage_dir();
        let _ = writeln!(
            out,
            "  {:<12} {} {:<12} {}",
            "omarchy",
            mark(omarchy.as_deref().is_some_and(|d| d.is_dir())),
            "",
            named(&omarchy)
        );
        let claude = ai_usage_tui::limits::claude_cache_path(&roots);
        let _ = writeln!(
            out,
            "  {:<12} {} {:<12} {}",
            "claude_code",
            mark(claude.as_deref().is_some_and(|p| p.is_file())),
            "",
            named(&claude)
        );
        let cache = ai_usage_tui::statusline::cache_path();
        match cache
            .as_deref()
            .map(ai_usage_tui::statusline::read_cache_at)
        {
            Some(Ok(Some(cached))) => {
                let now = ai_usage_tui::utils::now();
                let live = ai_usage_tui::statusline::live(&cached.windows, now).len();
                let _ = writeln!(
                    out,
                    "  {:<12} {} {:>4} windows {}",
                    "statusline",
                    mark(true),
                    live,
                    named(&cache)
                );
                let _ = writeln!(
                    out,
                    "  {:<12} {:<19}  received {} ago",
                    "",
                    "",
                    ai_usage_tui::ui::aggregate::format_duration(now - cached.received)
                );
            }
            Some(Err(problem)) => {
                let _ = writeln!(out, "  {:<12} unreadable  {problem}", "statusline");
            }
            _ => {
                let _ = writeln!(
                    out,
                    "  {:<12} {} {:<12} {}",
                    "statusline",
                    mark(false),
                    "",
                    named(&cache)
                );
                let _ = writeln!(
                    out,
                    "  {:<12} {:<19}  fed by Claude Code's status line; see \
                     contrib/claude-code/statusline-settings.json",
                    "", ""
                );
            }
        }
    } else {
        let _ = writeln!(out, "  {:<12} disabled ([omarchy] limits = false)", "panel");
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
            Ok(tag) => {
                if ai_usage_tui::update::is_newer(env!("CARGO_PKG_VERSION"), &tag) {
                    let _ = writeln!(out, "  latest       {tag} — newer than this build");
                } else {
                    let _ = writeln!(out, "  latest       {tag} — up to date");
                }
                // The answer outlives this command: the dashboard reads it at startup and puts
                // a newer release in its header. Without this the check was printed once and
                // forgotten, so a user who never ran --doctor never learned a release existed.
                // Cached either way -- an "up to date" answer is what stops a stale cache from
                // going on claiming an update after the upgrade that satisfied it.
                match ai_usage_tui::update::write_check_cache(&ai_usage_tui::update::CachedCheck {
                    latest: tag,
                    checked: ai_usage_tui::utils::now(),
                }) {
                    Ok(path) => {
                        let _ = writeln!(out, "  cached       {}", path.display());
                    }
                    Err(error) => {
                        let _ = writeln!(out, "  cached       could not be written: {error}");
                    }
                }
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
        // What the dashboard is showing, which is not nothing just because the check is off: an
        // answer cached by an earlier opted-in run stays in the header until the upgrade that
        // satisfies it. A user turning the check off and still seeing a notice deserves to find
        // out from here where it came from.
        if let Some(cached) = ai_usage_tui::update::read_check_cache() {
            let _ = writeln!(
                out,
                "  cached       {} from an earlier check{}",
                cached.latest,
                if ai_usage_tui::update::is_newer(env!("CARGO_PKG_VERSION"), &cached.latest) {
                    " — the dashboard header shows it"
                } else {
                    ""
                }
            );
        }
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

/// Whether Cursor is on this machine, for the `--doctor` note explaining why it is not a source.
///
/// Cursor stores its sessions in `state.vscdb`, where `tokenCount.inputTokens` and
/// `.outputTokens` are written as `{0,0}` on current builds -- Cursor's own staff describe the
/// field as best-effort and unreliable, and direct users to the web dashboard for real figures.
/// Every tool that shows Cursor spend therefore reconstructs it, ultimately by dividing a
/// character count by four. That is a guess wearing a number's clothes, and pricing it would put
/// invented dollars into budgets and the coverage figure. So there is no Cursor collector, and
/// this is where that choice is stated out loud.
fn cursor_is_installed() -> bool {
    let Some(home) = ai_usage_tui::utils::home_dir() else {
        return false;
    };
    [
        home.join(".cursor"),
        home.join(".config/Cursor"),
        home.join("Library/Application Support/Cursor"),
        home.join("AppData/Roaming/Cursor"),
    ]
    .iter()
    .any(|path| path.exists())
}

/// How to point a source somewhere else, printed only when it was not where we looked.
fn absence_hint(id: &str) -> Option<&'static str> {
    match id {
        "opencode" => Some("point elsewhere with --db PATH or OPENCODE_DB_PATH"),
        "claude_code" => Some("point elsewhere with --claude-dir PATH or CLAUDE_PROJECTS_DIR"),
        "codex" => Some("point elsewhere with --codex-dir PATH or CODEX_HOME"),
        "copilot" => Some("point elsewhere with --copilot-dir PATH or COPILOT_HOME"),
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
            Some("optional; --refresh-pricing writes it (bundled rates work without it)")
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
        // `retries`, `escalations` and `review_defects` keep their positions and are empty
        // rather than `0` when no task reported one; the three `_observed` denominators are
        // appended after everything else, for the same reason the four before them were.
        let sum_or_empty = |count: ai_usage_tui::model::ObservedCount| {
            count.sum().map(|n| n.to_string()).unwrap_or_default()
        };
        let mut csv = String::from(
            "agent,model,provider,tasks,tokens,cost,retries,escalations,test_passes,test_failures,review_defects,priced_tasks,unpriced_tasks,quota_tasks,free_tasks,retries_observed,escalations_observed,review_defects_observed\n",
        );
        for agg in &aggregates {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_field(&agg.agent),
                csv_field(&agg.model),
                csv_field(&agg.provider),
                agg.tasks,
                agg.tokens,
                agg.cost,
                sum_or_empty(agg.retries),
                sum_or_empty(agg.escalations),
                agg.test_passes,
                agg.test_failures,
                sum_or_empty(agg.review_defects),
                agg.priced_tasks,
                agg.unpriced_tasks,
                agg.quota_tasks,
                agg.free_tasks,
                agg.retries.observed,
                agg.escalations.observed,
                agg.review_defects.observed,
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
                    // `null`, not `0`, when no task reported the count — and the number of
                    // tasks that did, so a script has the denominator the rate was taken over.
                    // The rates are the share of those tasks affected: a percentage, bounded,
                    // rather than `retries / tasks`, which exceeded 100% on the first task that
                    // retried twice.
                    "retries": agg.retries.sum(),
                    "escalations": agg.escalations.sum(),
                    "test_passes": agg.test_passes,
                    "test_failures": agg.test_failures,
                    "review_defects": agg.review_defects.sum(),
                    "retries_observed": agg.retries.observed,
                    "escalations_observed": agg.escalations.observed,
                    "review_defects_observed": agg.review_defects.observed,
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
