pub mod budget;
pub mod classify;
pub mod cli;
pub mod collector;
pub mod config;
pub mod escalation;
pub mod export;
pub mod helpers;
pub mod logging;
pub mod model;
pub mod omarchy;
pub mod pricing;
pub mod routing;
pub mod ui;
pub mod update;
pub mod utils;

pub use collector::background::{Collector, CollectorHandle};
pub use collector::claude_code::ClaudeCodeCollector;
pub use collector::codex::CodexCollector;
pub use collector::journal::JournalCollector;
pub use collector::opencode::OpenCodeCollector;
pub use collector::pricing_refresh::ZenPricingCollector;

#[cfg(test)]
mod integration_tests {
    use std::fs;
    use tempfile::TempDir;

    use crate::{
        cli::Cli,
        collector::{build_test_journal, load_usage, setup_test_db, SourceRoots},
        export::print_once,
        model::{Category, CostStatus, Range, Usage},
        pricing::{apply_estimated_pricing, PricingEngine},
    };

    #[test]
    fn test_full_pipeline_opencode_to_totals() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = setup_test_db();
        let journal_path = build_test_journal(temp_dir.path());

        let roots = SourceRoots {
            db_path: Some(db_path),
            journal: journal_path,
            claude_dir: Some(temp_dir.path().join("no-claude-logs")),
            codex_dir: Some(temp_dir.path().join("no-codex-home")),
            omarchy_dir: Some(temp_dir.path().join("no-omarchy")),
            ..Default::default()
        };
        let (usages, _source) = load_usage(&roots).unwrap();

        // Both sources must actually contribute; a silently-empty journal used to let this
        // test pass while covering only OpenCode.
        assert!(
            usages.iter().any(|u| u.provider == "opencode"),
            "no OpenCode usage ingested"
        );
        assert_eq!(
            usages.iter().filter(|u| u.model == "gemma3:4b").count(),
            2,
            "two distinct journal events with identical token counts were collapsed"
        );

        let engine = PricingEngine::load();
        let mut usages = usages;
        apply_estimated_pricing(&mut usages, &engine);

        let totals: crate::model::Totals = usages.iter().fold(Default::default(), |mut t, u| {
            t.add(u);
            t
        });

        assert!(totals.tokens() > 0);
        assert!(totals.requests > 0);
    }

    #[test]
    fn test_config_precedence() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        fs::write(
            &config_path,
            r#"
            refresh_interval = 60
            days = 14
            provider = "opencode"
        "#,
        )
        .unwrap();

        let cli = Cli {
            config_path: Some(config_path),
            ..Default::default()
        };

        let (cli, _config) = crate::config::apply_config(cli).unwrap();

        assert_eq!(cli.refresh_interval.as_secs(), 60);
        assert_eq!(cli.range, Range::Days(14));
        assert_eq!(cli.provider_filter, Some("opencode".into()));
    }

    #[test]
    fn test_cli_mutual_exclusion() {
        let result = crate::cli::parse_cli(["--once", "--record-ollama"]);
        assert!(result.is_err());

        let result = crate::cli::parse_cli(["--json", "--refresh-zen"]);
        assert!(result.is_err());

        let result = crate::cli::parse_cli(["--csv", "out.csv", "--refresh-pricing"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_pricing_engine_free_models() {
        let engine = PricingEngine::bundled();

        let usage = Usage {
            model: "big-pickle".into(),
            input: 1_000_000,
            output: 1_000_000,
            category: Category::Free,
            cost_status: CostStatus::Unavailable,
            ..Default::default()
        };

        let (cost, status) = engine.estimate_cost(&usage).unwrap();
        assert_eq!(cost, 0.0);
        assert_eq!(status, CostStatus::Free);
    }

    #[test]
    fn test_pricing_engine_marks_cloud_as_quota_and_leaves_local_alone() {
        let engine = PricingEngine::bundled();
        let mut usages = vec![
            Usage {
                model: "qwen3-coder-agent".into(),
                category: Category::Local,
                cost_status: CostStatus::Unavailable,
                input: 1_000_000,
                ..Default::default()
            },
            Usage {
                model: "glm-5.2:cloud".into(),
                category: Category::Cloud,
                cost_status: CostStatus::Unavailable,
                input: 1_000_000,
                ..Default::default()
            },
        ];

        apply_estimated_pricing(&mut usages, &engine);

        // Local is skipped, not restamped — its status comes from the collector.
        assert_eq!(usages[0].cost_status, CostStatus::Unavailable);
        assert_eq!(usages[1].cost_status, CostStatus::Quota);
        assert_eq!(usages[1].cost, None);
    }

    #[test]
    fn test_export_json_format() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = setup_test_db();
        let journal_path = build_test_journal(temp_dir.path());

        let roots = SourceRoots {
            db_path: Some(db_path.clone()),
            journal: journal_path,
            claude_dir: Some(temp_dir.path().join("no-claude-logs")),
            codex_dir: Some(temp_dir.path().join("no-codex-home")),
            omarchy_dir: Some(temp_dir.path().join("no-omarchy")),
            ..Default::default()
        };
        let (usages, _source) = load_usage(&roots).unwrap();
        let engine = PricingEngine::load();
        let mut usages = usages;
        apply_estimated_pricing(&mut usages, &engine);

        let cli = Cli {
            json: true,
            once: true,
            db_path: Some(db_path),
            claude_dir: Some(temp_dir.path().join("no-claude-logs")),
            codex_dir: Some(temp_dir.path().join("no-codex-home")),
            omarchy_dir: Some(temp_dir.path().join("no-omarchy")),
            ..Default::default()
        };

        print_once(&cli).unwrap();
    }

    #[test]
    fn test_export_csv_format() {
        let temp_dir = TempDir::new().unwrap();
        let csv_path = temp_dir.path().join("export.csv");

        let db_path = setup_test_db();

        let cli = Cli {
            csv_path: Some(csv_path.clone()),
            once: true,
            db_path: Some(db_path),
            claude_dir: Some(temp_dir.path().join("no-claude-logs")),
            codex_dir: Some(temp_dir.path().join("no-codex-home")),
            omarchy_dir: Some(temp_dir.path().join("no-omarchy")),
            ..Default::default()
        };

        print_once(&cli).unwrap();

        let content = fs::read_to_string(&csv_path).unwrap();
        assert!(content.contains("provider,model,category,cost_status"));
    }

    #[test]
    fn subscription_rows_export_as_quota_with_the_counterfactual_appended() {
        // The CSV must keep its first fourteen columns byte-identical for consumers reading by
        // index; the counterfactual is appended, never inserted.
        let temp_dir = TempDir::new().unwrap();
        let projects = temp_dir.path().join(".claude").join("projects").join("p");
        fs::create_dir_all(&projects).unwrap();
        fs::write(
            projects.join("s.jsonl"),
            "{\"type\":\"assistant\",\"uuid\":\"u-1\",\"requestId\":\"req_1\",\"timestamp\":\"2026-08-18T10:00:00Z\",\"sessionId\":\"s1\",\"message\":{\"id\":\"msg_1\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-5-20250929\",\"usage\":{\"input_tokens\":1000,\"output_tokens\":500}}}\n",
        )
        .unwrap();
        let csv_path = temp_dir.path().join("export.csv");
        let cli = Cli {
            csv_path: Some(csv_path.clone()),
            once: true,
            range: Range::All,
            db_path: Some(setup_test_db()),
            journal_path: Some(temp_dir.path().join("journal.db")),
            claude_dir: Some(temp_dir.path().join(".claude").join("projects")),
            codex_dir: Some(temp_dir.path().join("no-codex-home")),
            claude_billing: crate::collector::billing::BillingSetting::Subscription,
            ..Default::default()
        };
        print_once(&cli).unwrap();

        let content = fs::read_to_string(&csv_path).unwrap();
        let header = content.lines().next().unwrap();
        assert!(
            header.starts_with("provider,model,category,cost_status,requests,input_tokens,output_tokens,reasoning_tokens,cache_read_tokens,cache_write_tokens,cost,created,project,session_id"),
            "{header}"
        );
        assert!(header.ends_with(",api_equivalent_cost"), "{header}");
        let row = content
            .lines()
            .find(|line| line.starts_with("anthropic,"))
            .expect("the transcript row is exported");
        let fields: Vec<&str> = row.split(',').collect();
        assert_eq!(fields[3], "quota", "{row}");
        assert_eq!(fields[10], "", "no dollars on a plan-billed row: {row}");
        assert!(
            fields[14].parse::<f64>().is_ok_and(|c| c > 0.0),
            "the list-rate figure is appended: {row}"
        );
    }
}
