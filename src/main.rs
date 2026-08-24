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
        background::{
            ClaudeCodeCollector, Collector, CollectorHandle, JournalCollector, OpenCodeCollector,
            ZenPricingCollector,
        },
        journal::{record_ollama, record_routing},
        load_usage,
        pricing_refresh::refresh_pricing,
        zen::refresh_zen_catalog,
        SourceRoots,
    },
    config::CollectorConfig,
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
        print_help();
        return Ok(());
    }
    if parsed_cli.version {
        print_line(&format!("ai-usage-tui {}", env!("CARGO_PKG_VERSION")))?;
        return Ok(());
    }
    if parsed_cli.refresh_zen {
        let path = refresh_zen_catalog()?;
        print_line(&format!(
            "Cached OpenCode Zen catalog at {}",
            path.display()
        ))?;
        return Ok(());
    }
    if parsed_cli.refresh_pricing {
        let path = refresh_pricing()?;
        print_line(&format!(
            "Refreshed Zen pricing table at {}",
            path.display()
        ))?;
        return Ok(());
    }
    let (cli, config) = apply_config(parsed_cli)?;
    if let Some(path) = ai_usage_tui::logging::log_path() {
        ai_usage_tui::logging::info(
            "main",
            &format!("ai-usage-tui {} starting", env!("CARGO_PKG_VERSION")),
        );
        eprintln!("logging to {}", path.display());
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

    let collectors_cfg = config.collectors.as_ref();
    let mut collectors: Vec<Box<dyn Collector>> = Vec::new();

    let opencode_cfg = collector_cfg(collectors_cfg, |c| c.opencode.as_ref());
    if opencode_cfg.enabled.unwrap_or(true) {
        collectors.push(Box::new(OpenCodeCollector {
            db_path: cli.db_path.clone(),
            interval_secs: opencode_cfg.interval.unwrap_or(30),
            cursor: Default::default(),
        }));
    }

    // Claude Code's own session logs: the largest source of Anthropic usage on most machines,
    // and invisible to the OpenCode collector.
    let claude_cfg = collector_cfg(collectors_cfg, |c| c.claude_code.as_ref());
    if claude_cfg.enabled.unwrap_or(true) {
        collectors.push(Box::new(ClaudeCodeCollector {
            root: cli.claude_dir.clone(),
            interval_secs: claude_cfg.interval.unwrap_or(30),
            offsets: Default::default(),
            billing: cli.claude_billing,
            claude_json: cli.claude_json.clone(),
            decision: None,
        }));
    }

    let journal_cfg = collector_cfg(collectors_cfg, |c| c.journal.as_ref());
    if journal_cfg.enabled.unwrap_or(true) {
        collectors.push(Box::new(JournalCollector {
            journal_path: journal,
            interval_secs: journal_cfg.interval.unwrap_or(60),
        }));
    }

    let zen_cfg = collector_cfg(collectors_cfg, |c| c.zen_pricing.as_ref());
    if zen_cfg.enabled.unwrap_or(false) {
        collectors.push(Box::new(ZenPricingCollector {
            interval_secs: zen_cfg.interval.unwrap_or(3600),
        }));
    }

    if collectors.is_empty() {
        None
    } else {
        Some(CollectorHandle::spawn(collectors))
    }
}

/// Per-collector settings, defaulted when the section is absent.
fn collector_cfg(
    collectors: Option<&ai_usage_tui::config::CollectorsConfig>,
    pick: impl Fn(&ai_usage_tui::config::CollectorsConfig) -> Option<&CollectorConfig>,
) -> CollectorConfig {
    collectors.and_then(pick).cloned().unwrap_or_default()
}

fn run_tui(
    cli: &ai_usage_tui::cli::Cli,
    collector: Option<CollectorHandle>,
    budget_engine: BudgetEngine,
    dispatcher: AlertDispatcher,
) -> Result<()> {
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
        "alerts": alerts.iter().filter(|a| a.is_actionable()).map(|a| {
            serde_json::json!({
                "scope": a.scope.label(),
                "period": match a.period {
                    ai_usage_tui::budget::BudgetPeriod::Daily => "daily",
                    ai_usage_tui::budget::BudgetPeriod::Monthly => "monthly",
                },
                "level": a.level.label(),
                "spend": a.spend,
                "limit": a.limit,
                "pct": a.pct,
            })
        }).collect::<Vec<_>>(),
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
        let mut csv = String::from(
            "agent,model,provider,tasks,tokens,cost,retries,escalations,test_passes,test_failures,review_defects\n",
        );
        for agg in &aggregates {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
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
                    "cost": agg.cost,
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
