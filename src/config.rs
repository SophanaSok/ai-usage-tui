use std::{fs, path::PathBuf, time::Duration};

use anyhow::Result;
use serde::Deserialize;

use crate::budget::BudgetsConfig;
use crate::cli::Cli;
use crate::model::Range;

#[derive(Debug, Default, Deserialize)]
pub struct ConfigFile {
    pub db: Option<String>,
    pub journal: Option<String>,
    pub claude_dir: Option<String>,
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
    pub claude_code: Option<CollectorConfig>,
    pub journal: Option<CollectorConfig>,
    pub zen_pricing: Option<CollectorConfig>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CollectorConfig {
    pub enabled: Option<bool>,
    pub interval: Option<u64>,
}

pub fn config_path() -> Option<PathBuf> {
    Some(
        crate::utils::config_root()?
            .join("ai-usage-tui")
            .join("config.toml"),
    )
}

/// Read the config file once, or report why it could not be read.
///
/// The file used to be parsed three separate times with three different error policies:
/// `apply_config` hard-errored, while the collector and budget loaders both
/// `unwrap_or_default()`, so a typo in `[budgets]` silently disabled every budget while the
/// same typo in `[collectors]` was reported. One read, one policy.
pub fn load_config(cli: &Cli) -> Result<ConfigFile> {
    let Some(path) = cli.config_path.clone().or_else(config_path) else {
        return Ok(ConfigFile::default());
    };
    if !path.exists() {
        // An explicit `--config` that does not exist is a mistake worth stopping for; a
        // missing default is just an unconfigured install.
        if cli.config_path.is_some() {
            return Err(anyhow::anyhow!(
                "config file does not exist: {}",
                path.display()
            ));
        }
        return Ok(ConfigFile::default());
    }
    let contents = fs::read_to_string(&path)
        .map_err(|error| anyhow::anyhow!("could not read {}: {}", path.display(), error))?;
    toml::from_str(&contents)
        .map_err(|error| anyhow::anyhow!("could not parse {}: {}", path.display(), error))
}

/// Fill in CLI defaults from the config file, returning the parsed config alongside so callers
/// do not have to read it again.
pub fn apply_config(mut cli: Cli) -> Result<(Cli, ConfigFile)> {
    let mut config = load_config(&cli)?;
    if cli.db_path.is_none() {
        cli.db_path = config.db.take().map(PathBuf::from);
    }
    if cli.journal_path.is_none() {
        cli.journal_path = config.journal.take().map(PathBuf::from);
    }
    if cli.claude_dir.is_none() {
        cli.claude_dir = config.claude_dir.take().map(PathBuf::from);
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
        cli.provider_filter = config.provider.take();
    }
    if cli.model_filter.is_none() {
        cli.model_filter = config.model.take();
    }
    Ok((cli, config))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli_with_config(path: &std::path::Path) -> Cli {
        Cli {
            config_path: Some(path.to_path_buf()),
            ..Default::default()
        }
    }

    #[test]
    fn a_malformed_config_is_reported_rather_than_discarded() {
        // Silently defaulting here is how a typo in `[budgets]` turned every configured
        // budget off while the dashboard carried on looking healthy.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "days = \"not a number\"\n").unwrap();
        let error = load_config(&cli_with_config(&path))
            .unwrap_err()
            .to_string();
        assert!(error.contains("could not parse"), "{error}");
    }

    #[test]
    fn an_explicit_missing_config_is_an_error_but_a_missing_default_is_not() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("absent.toml");
        assert!(load_config(&cli_with_config(&missing)).is_err());
    }

    #[test]
    fn one_read_populates_both_the_cli_and_the_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "days = 14\n[collectors.opencode]\nenabled = false\n[budgets]\nwebhook = \"https://example.invalid/hook\"\n",
        )
        .unwrap();
        let (cli, config) = apply_config(cli_with_config(&path)).unwrap();
        assert_eq!(cli.range.label(), "14 DAYS");
        assert_eq!(
            config.collectors.unwrap().opencode.unwrap().enabled,
            Some(false)
        );
        assert_eq!(
            config.budgets.unwrap().webhook.as_deref(),
            Some("https://example.invalid/hook")
        );
    }

    #[test]
    fn config_file_values_parse() {
        let config: ConfigFile =
            toml::from_str("refresh_interval = 15\ndays = 14\nprovider = 'ollama'\n").unwrap();
        assert_eq!(config.refresh_interval, Some(15));
        assert_eq!(config.days, Some(14));
        assert_eq!(config.provider.as_deref(), Some("ollama"));
    }
}
