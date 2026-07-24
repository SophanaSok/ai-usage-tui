pub mod classify;
pub mod cli;
pub mod collector;
pub mod config;
pub mod export;
pub mod helpers;
pub mod model;
pub mod pricing;
pub mod ui;
pub mod utils;

pub use collector::background::{
    Collector, CollectorHandle, JournalCollector, OpenCodeCollector, ZenPricingCollector,
};

#[cfg(test)]
mod integration_tests {
    use std::fs;
    use tempfile::TempDir;

    use crate::{
        cli::Cli,
        collector::{load_usage, setup_test_db, setup_test_journal},
        export::print_once,
        model::{CostStatus, Range, Usage, Category},
        pricing::{apply_estimated_pricing, PricingEngine},
    };

    #[test]
    fn test_full_pipeline_opencode_to_totals() {
        let db_path = setup_test_db();
        let journal_path = setup_test_journal();
        
        let (usages, _source) = load_usage(Some(&db_path), &journal_path).unwrap();
        
        let engine = PricingEngine::load();
        let mut usages = usages;
        apply_estimated_pricing(&mut usages, &engine);
        
        let totals: crate::model::Totals = usages.iter().fold(Default::default(), |mut t, u| { t.add(u); t });
        
        assert!(totals.tokens() > 0);
        assert!(totals.requests > 0);
    }

    #[test]
    fn test_config_precedence() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");
        
        fs::write(&config_path, r#"
            refresh_interval = 60
            days = 14
            provider = "opencode"
        "#).unwrap();
        
        let cli = Cli {
            config_path: Some(config_path),
            ..Default::default()
        };
        
        let cli = crate::config::apply_config(cli).unwrap();
        
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
    fn test_pricing_engine_skips_local_and_cloud() {
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
        
        assert_eq!(usages[0].cost_status, CostStatus::Unavailable);
        assert_eq!(usages[1].cost_status, CostStatus::Unavailable);
    }

    #[test]
    fn test_export_json_format() {
        let db_path = setup_test_db();
        let journal_path = setup_test_journal();
        
        let (usages, _source) = load_usage(Some(&db_path), &journal_path).unwrap();
        let engine = PricingEngine::load();
        let mut usages = usages;
        apply_estimated_pricing(&mut usages, &engine);
        
        let cli = Cli {
            json: true,
            once: true,
            db_path: Some(db_path),
            ..Default::default()
        };
        
        print_once(&cli).unwrap();
    }

    #[test]
    fn test_export_csv_format() {
        let temp_dir = TempDir::new().unwrap();
        let csv_path = temp_dir.path().join("export.csv");
        
        let db_path = setup_test_db();
        let _journal_path = setup_test_journal();
        
        let cli = Cli {
            csv_path: Some(csv_path.clone()),
            once: true,
            db_path: Some(db_path),
            ..Default::default()
        };
        
        print_once(&cli).unwrap();
        
        let content = fs::read_to_string(&csv_path).unwrap();
        assert!(content.contains("provider,model,category,cost_status"));
    }
}