use std::{env, fs, path::PathBuf, time::Duration};

use anyhow::Result;
use serde::Deserialize;

use crate::cli::Cli;
use crate::model::Range;
use crate::budget::BudgetsConfig;

#[derive(Debug, Default, Deserialize)]
pub struct ConfigFile {
    pub db: Option<String>,
    pub journal: Option<String>,
    pub refresh_interval: Option<u64>,
    pub days: Option<u64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub collectors: Option<CollectorsConfig>,
    pub budgets: Option<BudgetsConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CollectorsConfig {
    pub opencode: Option<CollectorConfig>,
    pub journal: Option<CollectorConfig>,
    pub zen_pricing: Option<CollectorConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CollectorConfig {
    pub enabled: Option<bool>,
    pub interval: Option<u64>,
}

pub fn config_path() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".config"))
            .join("ai-usage-tui/config.toml"),
    )
}

pub fn apply_config(mut cli: Cli) -> Result<Cli> {
    let path = cli.config_path.clone().or_else(config_path);
    let Some(path) = path else {
        return Ok(cli);
    };
    if !path.exists() {
        if cli.config_path.is_some() {
            return Err(anyhow::anyhow!(
                "config file does not exist: {}",
                path.display()
            ));
        }
        return Ok(cli);
    }
    let contents = fs::read_to_string(&path)?;
    let config: ConfigFile = toml::from_str(&contents)?;
    if cli.db_path.is_none() {
        cli.db_path = config.db.map(PathBuf::from);
    }
    if cli.journal_path.is_none() {
        cli.journal_path = config.journal.map(PathBuf::from);
    }
    if !cli.refresh_interval_set {
        if let Some(seconds) = config.refresh_interval {
            if seconds == 0 {
                return Err(anyhow::anyhow!(
                    "config refresh_interval must be greater than zero"
                ));
            }
            cli.refresh_interval = Duration::from_secs(seconds);
        }
    }
    if !cli.range_set {
        if let Some(days) = config.days {
            if days == 0 {
                return Err(anyhow::anyhow!("config days must be greater than zero"));
            }
            cli.range = Range::Days(days);
        }
    }
    if cli.provider_filter.is_none() {
        cli.provider_filter = config.provider;
    }
    if cli.model_filter.is_none() {
        cli.model_filter = config.model;
    }
    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_values_parse() {
        let config: ConfigFile =
            toml::from_str("refresh_interval = 15\ndays = 14\nprovider = 'ollama'\n").unwrap();
        assert_eq!(config.refresh_interval, Some(15));
        assert_eq!(config.days, Some(14));
        assert_eq!(config.provider.as_deref(), Some("ollama"));
    }
}
