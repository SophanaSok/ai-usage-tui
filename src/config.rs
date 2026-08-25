use std::{fs, path::PathBuf, time::Duration};

use anyhow::Result;
use serde::Deserialize;

use crate::budget::BudgetsConfig;
use crate::cli::Cli;
use crate::collector::billing::BillingSetting;
use crate::model::Range;

/// `deny_unknown_fields` throughout this file, and in `budget.rs`: a key the parser does not
/// recognise is a typo, and silently dropping it is how `# webhook` under the wrong table
/// disabled every budget while the dashboard went on looking healthy. `load_config` already
/// treats a malformed *value* as fatal; an unrecognised *key* now gets the same policy.
/// Whether to look for a newer release, and nothing else.
///
/// Off by default and it stays that way. This is the second thing in the tool that would reach
/// the network -- `zen_pricing` is the first, and it is off by default for the same reason. A
/// tool whose pitch is "reads usage metadata, writes nothing, transmits nothing" does not get to
/// contact a server because it would be convenient.
///
/// Note what is *not* gated on this: `--doctor` always reports which channel this binary came
/// from and how to upgrade it. That is read off the binary's own path and needs no network, so
/// there is nothing to opt in to.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    /// Ask GitHub for the latest release tag when `--doctor` runs. Never automatic, never on the
    /// dashboard's refresh path, and never during any other command.
    #[serde(default)]
    pub check: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    pub db: Option<String>,
    pub journal: Option<String>,
    pub claude_dir: Option<String>,
    pub codex_dir: Option<String>,
    pub gemini_dir: Option<String>,
    pub refresh_interval: Option<u64>,
    pub days: Option<u64>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub collectors: Option<CollectorsConfig>,
    pub budgets: Option<BudgetsConfig>,
    pub omarchy: Option<OmarchyConfig>,
    pub update: Option<UpdateConfig>,
}

/// Omarchy's agents panel: where its records are, and whether to read them.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OmarchyConfig {
    pub dir: Option<String>,
    /// Read the rate-limit windows and plan labels. Default true; the directory being absent
    /// is already silent.
    pub limits: Option<bool>,
    /// Which records `--omarchy-record` writes: `opencode` (default), `ollama`. Never
    /// `claude`, `codex` or `fireworks` — those are Omarchy's own files.
    pub records: Option<Vec<String>>,
    /// Also draw the primary budget as the panel's prepaid ledger. Off by default: the panel
    /// labels it "Prepaid credits … funded", which is a loose description of a soft budget.
    pub balance: Option<bool>,
    /// Which budget is the ledger, as `<scope>/<period>` — `global/monthly` by default.
    pub balance_budget: Option<String>,
}

/// Per-source settings, keyed by the source's registry id.
///
/// This was a fixed struct with one field per source, which meant adding a provider meant
/// editing this file too, and a mistyped table name -- `[collectors.opencodee]` -- parsed into
/// a field nobody read. Keyed off `collector::registry` instead: `validate` rejects an id that
/// is not a source and names the ones that are.
#[derive(Debug, Default, Deserialize)]
#[serde(transparent)]
pub struct CollectorsConfig(pub std::collections::BTreeMap<String, CollectorConfig>);

impl CollectorsConfig {
    pub fn get(&self, id: &str) -> Option<&CollectorConfig> {
        self.0.get(id)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectorConfig {
    pub enabled: Option<bool>,
    pub interval: Option<u64>,
    /// `auto` (default), `subscription`, or `api`. Only meaningful for agent collectors that
    /// can run on a plan; the others reject it rather than ignore it.
    pub billing: Option<BillingSetting>,
    /// The agent's own config document, when it is not where the collector would look.
    pub config_json: Option<String>,
}

impl ConfigFile {
    /// Keys that parse but that the named collector does not act on are an error, not a
    /// no-op: a `billing` line under the wrong table would otherwise sit there looking like
    /// it worked.
    fn validate(&self) -> Result<()> {
        if let Some(records) = self.omarchy.as_ref().and_then(|o| o.records.as_ref()) {
            for id in records {
                if !crate::omarchy::record::ALLOWED_IDS.contains(&id.as_str()) {
                    return Err(anyhow::anyhow!(
                        "[omarchy] records may only contain {}; {id:?} would overwrite or \
                         impersonate an Omarchy agent",
                        crate::omarchy::record::ALLOWED_IDS.join(", ")
                    ));
                }
            }
        }
        let Some(collectors) = &self.collectors else {
            return Ok(());
        };
        for (name, cfg) in &collectors.0 {
            let Some(spec) = crate::collector::registry::find(name) else {
                return Err(anyhow::anyhow!(
                    "[collectors.{name}] is not a data source; the sources are {}",
                    crate::collector::registry::ids().join(", ")
                ));
            };
            if !spec.supports_billing && (cfg.billing.is_some() || cfg.config_json.is_some()) {
                return Err(anyhow::anyhow!(
                    "[collectors.{name}] does not support `billing` or `config_json`; \
                     they apply to [collectors.claude_code] and [collectors.codex]"
                ));
            }
        }
        Ok(())
    }
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
    let config: ConfigFile = toml::from_str(&contents)
        .map_err(|error| anyhow::anyhow!("could not parse {}: {}", path.display(), error))?;
    config
        .validate()
        .map_err(|error| anyhow::anyhow!("{}: {}", path.display(), error))?;
    Ok(config)
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
    if let Some(claude) = config
        .collectors
        .as_ref()
        .and_then(|collectors| collectors.get(crate::collector::claude_code::ID))
    {
        if !cli.claude_billing_set {
            if let Some(billing) = claude.billing {
                cli.claude_billing = billing;
            }
        }
        if cli.claude_json.is_none() {
            cli.claude_json = claude.config_json.clone().map(PathBuf::from);
        }
    }
    if cli.codex_dir.is_none() {
        cli.codex_dir = config.codex_dir.take().map(PathBuf::from);
    }
    if let Some(omarchy) = config.omarchy.as_ref() {
        if cli.omarchy_dir.is_none() {
            cli.omarchy_dir = omarchy.dir.clone().map(PathBuf::from);
        }
        if let Some(limits) = omarchy.limits {
            cli.limits_enabled = limits;
        }
    }
    if cli.gemini_dir.is_none() {
        cli.gemini_dir = config.gemini_dir.take().map(PathBuf::from);
    }
    if let Some(gemini) = config
        .collectors
        .as_ref()
        .and_then(|collectors| collectors.get(crate::collector::gemini::ID))
    {
        if !cli.gemini_billing_set {
            if let Some(billing) = gemini.billing {
                cli.gemini_billing = billing;
            }
        }
    }
    if let Some(codex) = config
        .collectors
        .as_ref()
        .and_then(|collectors| collectors.get(crate::collector::codex::ID))
    {
        if codex.config_json.is_some() {
            return Err(anyhow::anyhow!(
                "[collectors.codex] does not support `config_json`: Codex has no config \
                 document this tool reads"
            ));
        }
        if !cli.codex_billing_set {
            if let Some(billing) = codex.billing {
                cli.codex_billing = billing;
            }
        }
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
    // `[collectors.<id>] enabled` now governs the one-shot paths too, not just the dashboard's
    // background collectors: `--json` used to emit rows from a source the config had switched
    // off. Carried on the Cli so every `SourceRoots::from_cli` site sees it.
    if let Some(collectors) = config.collectors.as_ref() {
        for (id, cfg) in &collectors.0 {
            if let Some(enabled) = cfg.enabled {
                cli.source_enabled.insert(id.clone(), enabled);
            }
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
            config
                .collectors
                .unwrap()
                .get(crate::collector::opencode::ID)
                .unwrap()
                .enabled,
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

    #[test]
    fn the_example_config_puts_the_webhook_under_budgets() {
        // The shipped example once listed `# webhook = …` after `[collectors.zen_pricing]` with
        // no `[budgets]` header, so uncommenting it as written produced
        // `collectors.zen_pricing.webhook` — dropped silently, because no struct here uses
        // `deny_unknown_fields`. The example must parse, and the webhook must land where the
        // README says it does.
        let uncommented: String = include_str!("../examples/config.toml")
            .lines()
            .map(|line| match line.trim_start().strip_prefix("# webhook") {
                Some(rest) => format!("webhook{rest}"),
                None => line.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let config: ConfigFile = toml::from_str(&uncommented).expect("examples/config.toml parses");
        let budgets = config
            .budgets
            .expect("the example configures a [budgets] table");
        assert!(
            budgets.webhook.is_some(),
            "uncommenting `# webhook` must set budgets.webhook, not a key under another table"
        );
        assert!(
            !budgets.entry.is_empty(),
            "the [[budgets.entry]] examples must still parse after the header is added"
        );
    }

    #[test]
    fn billing_parses_and_reaches_the_cli_unless_the_flag_was_given() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[collectors.claude_code]\nbilling = \"subscription\"\nconfig_json = \"/x/.claude.json\"\n",
        )
        .unwrap();
        let (cli, _) = apply_config(cli_with_config(&path)).unwrap();
        assert_eq!(cli.claude_billing, BillingSetting::Subscription);
        assert_eq!(
            cli.claude_json.as_deref(),
            Some(std::path::Path::new("/x/.claude.json"))
        );

        let flagged = Cli {
            claude_billing: BillingSetting::Api,
            claude_billing_set: true,
            ..cli_with_config(&path)
        };
        let (cli, _) = apply_config(flagged).unwrap();
        assert_eq!(
            cli.claude_billing,
            BillingSetting::Api,
            "the flag wins over config"
        );
    }

    #[test]
    fn an_unknown_billing_mode_is_a_parse_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[collectors.claude_code]\nbilling = \"sometimes\"\n").unwrap();
        let error = load_config(&cli_with_config(&path))
            .unwrap_err()
            .to_string();
        assert!(error.contains("could not parse"), "{error}");
    }

    #[test]
    fn a_misspelled_key_is_an_error_rather_than_a_key_that_does_nothing() {
        // Every one of these parsed cleanly and did nothing before `deny_unknown_fields`. The
        // shipped example config carries a comment warning about exactly the `[budgets]` case,
        // because a `webhook` line under the wrong table once disabled every budget silently.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        for (contents, needle) in [
            ("dayz = 14\n", "dayz"),
            ("[budgets]\nwebook = \"https://example.invalid/hook\"\n", "webook"),
            ("[[budgets.entry]]\nscope = \"global\"\nperiod = \"monthly\"\nlimit = 1.0\nwarnn = 50.0\n", "warnn"),
            // `[collectors.*]` is a map keyed by source id, so an unknown table is caught by
            // `validate` rather than by serde -- and the message names the real sources.
            ("[collectors.opencodee]\nenabled = false\n", "opencodee"),
            ("[collectors.opencode]\nenabledd = false\n", "enabledd"),
            ("[omarchy]\nlimit = false\n", "limit"),
        ] {
            fs::write(&path, contents).unwrap();
            let error = match load_config(&cli_with_config(&path)) {
                Ok(_) => panic!("an unknown key parsed silently: {contents:?}"),
                Err(error) => error.to_string(),
            };
            assert!(
                error.contains(needle),
                "the error should name the offending key {needle:?}: {error}"
            );
        }

        // The unknown-source message must be actionable, not just a refusal.
        fs::write(&path, "[collectors.opencodee]\nenabled = false\n").unwrap();
        let error = load_config(&cli_with_config(&path))
            .unwrap_err()
            .to_string();
        assert!(error.contains("is not a data source"), "{error}");
        for id in crate::collector::registry::ids() {
            assert!(
                error.contains(id),
                "the error should list {id:?} as a valid source: {error}"
            );
        }
    }

    #[test]
    fn billing_under_a_collector_that_cannot_use_it_is_rejected_not_ignored() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[collectors.opencode]\nbilling = \"subscription\"\n").unwrap();
        let error = load_config(&cli_with_config(&path))
            .unwrap_err()
            .to_string();
        assert!(error.contains("[collectors.opencode]"), "{error}");
        assert!(error.contains("claude_code"), "{error}");
    }

    #[test]
    fn codex_settings_parse_but_a_config_document_is_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "codex_dir = \"/x/codex\"\n[collectors.codex]\nenabled = false\ninterval = 45\nbilling = \"api\"\n",
        )
        .unwrap();
        let (cli, config) = apply_config(cli_with_config(&path)).unwrap();
        assert_eq!(
            cli.codex_dir.as_deref(),
            Some(std::path::Path::new("/x/codex"))
        );
        assert_eq!(cli.codex_billing, BillingSetting::Api);
        let collectors = config.collectors.unwrap();
        let codex = collectors.get(crate::collector::codex::ID).unwrap();
        assert_eq!(codex.enabled, Some(false));
        assert_eq!(codex.interval, Some(45));

        // Codex's only document is a credential file; there is nothing to point at.
        fs::write(
            &path,
            "[collectors.codex]\nconfig_json = \"/x/auth.json\"\n",
        )
        .unwrap();
        let error = match apply_config(cli_with_config(&path)) {
            Ok(_) => panic!("a config document for Codex was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("[collectors.codex]"), "{error}");
    }

    #[test]
    fn the_omarchy_table_sets_the_records_dir_and_can_disable_limits() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[omarchy]\ndir = \"/x/usage\"\nlimits = false\n").unwrap();
        let (cli, _) = apply_config(cli_with_config(&path)).unwrap();
        assert_eq!(
            cli.omarchy_dir.as_deref(),
            Some(std::path::Path::new("/x/usage"))
        );
        assert!(!cli.limits_enabled);
        let flagged = Cli {
            omarchy_dir: Some(PathBuf::from("/flag")),
            ..cli_with_config(&path)
        };
        let (cli, _) = apply_config(flagged).unwrap();
        assert_eq!(
            cli.omarchy_dir.as_deref(),
            Some(std::path::Path::new("/flag"))
        );
    }

    #[test]
    fn omarchy_records_are_an_allowlist() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[omarchy]\nrecords = [\"opencode\", \"claude\"]\n").unwrap();
        let error = load_config(&cli_with_config(&path))
            .unwrap_err()
            .to_string();
        assert!(error.contains("claude"), "{error}");
        fs::write(
            &path,
            "[omarchy]\nrecords = [\"opencode\", \"ollama\"]\nbalance = true\n",
        )
        .unwrap();
        let config = load_config(&cli_with_config(&path)).unwrap();
        assert_eq!(config.omarchy.unwrap().balance, Some(true));
    }
}
