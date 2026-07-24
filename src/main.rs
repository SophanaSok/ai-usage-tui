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
    budget::BudgetEngine,
    cli::{parse_cli, print_help},
    collector::{
        background::{
            Collector, CollectorHandle, JournalCollector, OpenCodeCollector, ZenPricingCollector,
        },
        journal::{record_ollama, record_routing},
        load_usage,
        pricing_refresh::refresh_pricing,
        zen::refresh_zen_catalog,
    },
    config::{apply_config, CollectorsConfig},
    export::{csv_field, print_once},
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
    if cli.record_routing {
        let path = cli
            .journal_path
            .clone()
            .or_else(ai_usage_tui::utils::journal_path)
            .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        return record_routing(&path);
    }
    if cli.check_budgets {
        return check_budgets(&cli);
    }
    if cli.routing_json || cli.routing_csv_path.is_some() {
        return export_routing(&cli);
    }
    if cli.once || cli.json {
        return print_once(&cli);
    }

    let budget_engine = load_budget_engine(&cli);
    let collector_handle = build_collectors(&cli);
    run_tui(&cli, collector_handle, budget_engine)
}

fn build_collectors(cli: &ai_usage_tui::cli::Cli) -> Option<CollectorHandle> {
    let journal = cli
        .journal_path
        .clone()
        .or_else(ai_usage_tui::utils::journal_path)?;

    let config = load_collector_config(cli);
    let mut collectors: Vec<Box<dyn Collector>> = Vec::new();

    let opencode_cfg = config.opencode.unwrap_or_default();
    if opencode_cfg.enabled.unwrap_or(true) {
        collectors.push(Box::new(OpenCodeCollector {
            db_path: cli.db_path.clone(),
            interval_secs: opencode_cfg.interval.unwrap_or(30),
        }));
    }

    let journal_cfg = config.journal.unwrap_or_default();
    if journal_cfg.enabled.unwrap_or(true) {
        collectors.push(Box::new(JournalCollector {
            journal_path: journal,
            interval_secs: journal_cfg.interval.unwrap_or(60),
        }));
    }

    let zen_cfg = config.zen_pricing.unwrap_or_default();
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

fn load_collector_config(cli: &ai_usage_tui::cli::Cli) -> CollectorsConfig {
    let path = cli
        .config_path
        .clone()
        .or_else(ai_usage_tui::config::config_path);

    let Some(path) = path else {
        return CollectorsConfig::default();
    };
    if !path.exists() {
        return CollectorsConfig::default();
    }
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let config: ai_usage_tui::config::ConfigFile = toml::from_str(&contents).unwrap_or_default();
    config.collectors.unwrap_or_default()
}

fn run_tui(
    cli: &ai_usage_tui::cli::Cli,
    collector: Option<CollectorHandle>,
    budget_engine: BudgetEngine,
) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let result = catch_unwind(AssertUnwindSafe(|| run(&mut terminal, cli, collector, budget_engine)));
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    match result {
        Ok(result) => result,
        Err(payload) => resume_unwind(payload),
    }
}

fn load_budget_engine(cli: &ai_usage_tui::cli::Cli) -> BudgetEngine {
    let config = load_full_config(cli);
    match &config.budgets {
        Some(budgets) => BudgetEngine::from_config(budgets),
        None => BudgetEngine::empty(),
    }
}

fn load_full_config(cli: &ai_usage_tui::cli::Cli) -> ai_usage_tui::config::ConfigFile {
    let path = cli
        .config_path
        .clone()
        .or_else(ai_usage_tui::config::config_path);

    let Some(path) = path else {
        return ai_usage_tui::config::ConfigFile::default();
    };
    if !path.exists() {
        return ai_usage_tui::config::ConfigFile::default();
    }
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    toml::from_str(&contents).unwrap_or_default()
}

fn check_budgets(cli: &ai_usage_tui::cli::Cli) -> Result<()> {
    let budget_engine = load_budget_engine(cli);
    if budget_engine.is_empty() {
        println!("{{\"budgets\": 0, \"alerts\": []}}");
        return Ok(());
    }

    let journal = cli
        .journal_path
        .clone()
        .or_else(ai_usage_tui::utils::journal_path)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;

    let (usages, _) = load_usage(cli.db_path.as_deref(), &journal)?;
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
    println!("{}", serde_json::to_string_pretty(&json)?);

    if has_alerts {
        std::process::exit(1);
    }
    Ok(())
}

fn export_routing(cli: &ai_usage_tui::cli::Cli) -> Result<()> {
    let journal = cli
        .journal_path
        .clone()
        .or_else(ai_usage_tui::utils::journal_path)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;

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
        println!("Wrote routing CSV to {}", path.display());
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
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "source": format!("journal: {}", journal.display()),
                "events": events.len(),
                "aggregates": rows
            }))?
        );
    }
    Ok(())
}
