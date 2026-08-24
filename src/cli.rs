use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

use crate::collector::billing::BillingSetting;
use crate::model::Range;

pub struct Cli {
    pub help: bool,
    pub version: bool,
    pub config_path: Option<PathBuf>,
    pub db_path: Option<PathBuf>,
    pub journal_path: Option<PathBuf>,
    /// Root of Claude Code's session logs; defaults to `~/.claude/projects`.
    pub claude_dir: Option<PathBuf>,
    /// How Claude Code usage is billed: decided automatically unless overridden here or in
    /// `[collectors.claude_code]`.
    pub claude_billing: BillingSetting,
    pub claude_billing_set: bool,
    /// Claude Code's `~/.claude.json`, when it is not at the default location.
    pub claude_json: Option<PathBuf>,
    /// Codex's home; defaults to `$CODEX_HOME` or `~/.codex`.
    pub codex_dir: Option<PathBuf>,
    pub codex_billing: BillingSetting,
    pub codex_billing_set: bool,
    pub range: Range,
    pub range_set: bool,
    pub provider_filter: Option<String>,
    pub model_filter: Option<String>,
    pub once: bool,
    pub json: bool,
    pub csv_path: Option<PathBuf>,
    pub record_ollama: bool,
    pub refresh_zen: bool,
    pub refresh_pricing: bool,
    pub refresh_interval: Duration,
    pub refresh_interval_set: bool,
    pub check_budgets: bool,
    pub webhook_url: Option<String>,
    pub record_routing: bool,
    pub routing_json: bool,
    pub routing_csv_path: Option<PathBuf>,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            help: false,
            version: false,
            config_path: None,
            db_path: None,
            journal_path: None,
            claude_dir: None,
            claude_billing: BillingSetting::Auto,
            claude_billing_set: false,
            claude_json: None,
            codex_dir: None,
            codex_billing: BillingSetting::Auto,
            codex_billing_set: false,
            range: Range::Week,
            range_set: false,
            provider_filter: None,
            model_filter: None,
            once: false,
            json: false,
            csv_path: None,
            record_ollama: false,
            refresh_zen: false,
            refresh_pricing: false,
            refresh_interval: Duration::from_secs(30),
            refresh_interval_set: false,
            check_budgets: false,
            webhook_url: None,
            record_routing: false,
            routing_json: false,
            routing_csv_path: None,
        }
    }
}

pub fn parse_cli<I>(args: I) -> Result<Cli>
where
    I: IntoIterator,
    I::Item: Into<String>,
{
    let mut cli = Cli::default();
    let mut args = args.into_iter().map(Into::into).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => cli.help = true,
            "--version" | "-V" => cli.version = true,
            "--once" => cli.once = true,
            "--today" => {
                cli.range = Range::Today;
                cli.range_set = true;
            }
            "--week" => {
                cli.range = Range::Week;
                cli.range_set = true;
            }
            "--month" => {
                cli.range = Range::Month;
                cli.range_set = true;
            }
            "--all" => {
                cli.range = Range::All;
                cli.range_set = true;
            }
            "--json" => {
                cli.json = true;
                cli.once = true;
            }
            "--csv" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--csv requires a path"))?;
                cli.csv_path = Some(PathBuf::from(value));
                cli.once = true;
            }
            "--config" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--config requires a path"))?;
                cli.config_path = Some(PathBuf::from(value));
            }
            "--record-ollama" => cli.record_ollama = true,
            "--refresh-zen" => cli.refresh_zen = true,
            "--refresh-pricing" => cli.refresh_pricing = true,
            "--check-budgets" => cli.check_budgets = true,
            "--webhook" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--webhook requires a URL"))?;
                cli.webhook_url = Some(value);
            }
            "--record-routing" => cli.record_routing = true,
            "--routing-json" => {
                cli.routing_json = true;
                cli.once = true;
            }
            "--routing-csv" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--routing-csv requires a path"))?;
                cli.routing_csv_path = Some(PathBuf::from(value));
                cli.once = true;
            }
            "--db" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--db requires a path"))?;
                cli.db_path = Some(PathBuf::from(value));
            }
            "--journal" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--journal requires a path"))?;
                cli.journal_path = Some(PathBuf::from(value));
            }
            "--claude-dir" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--claude-dir requires a path"))?;
                cli.claude_dir = Some(PathBuf::from(value));
            }
            "--claude-billing" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--claude-billing requires a mode"))?;
                cli.claude_billing = parse_billing("--claude-billing", &value)?;
                cli.claude_billing_set = true;
            }
            "--codex-dir" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--codex-dir requires a path"))?;
                cli.codex_dir = Some(PathBuf::from(value));
            }
            "--codex-billing" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--codex-billing requires a mode"))?;
                cli.codex_billing = parse_billing("--codex-billing", &value)?;
                cli.codex_billing_set = true;
            }
            "--days" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--days requires a number"))?;
                let days: u64 = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid day range: {value}"))?;
                if days == 0 {
                    return Err(anyhow::anyhow!("day range must be greater than zero"));
                }
                cli.range = Range::Days(days);
                cli.range_set = true;
            }
            "--provider" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--provider requires a name"))?;
                cli.provider_filter = Some(value);
            }
            "--model" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--model requires a name"))?;
                cli.model_filter = Some(value);
            }
            "--refresh-interval" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--refresh-interval requires seconds"))?;
                let seconds: u64 = value
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid refresh interval: {value}"))?;
                if seconds == 0 {
                    return Err(anyhow::anyhow!(
                        "refresh interval must be greater than zero"
                    ));
                }
                cli.refresh_interval = Duration::from_secs(seconds);
                cli.refresh_interval_set = true;
            }
            other => {
                return Err(anyhow::anyhow!(
                    "unknown option: {other}\nUse --help for usage"
                ))
            }
        }
    }
    let actions = [
        cli.record_ollama,
        cli.refresh_zen,
        cli.refresh_pricing,
        cli.json,
        cli.csv_path.is_some(),
        cli.check_budgets,
        cli.record_routing,
        cli.routing_json,
        cli.routing_csv_path.is_some(),
    ]
    .into_iter()
    .filter(|enabled| *enabled)
    .count();
    if actions > 1
        || (cli.once
            && (cli.record_ollama || cli.refresh_zen || cli.check_budgets || cli.record_routing))
    {
        return Err(anyhow::anyhow!(
            "collection actions and --once/--json/--csv are mutually exclusive"
        ));
    }
    Ok(cli)
}

fn parse_billing(flag: &str, value: &str) -> Result<BillingSetting> {
    match value {
        "auto" => Ok(BillingSetting::Auto),
        "subscription" => Ok(BillingSetting::Subscription),
        "api" => Ok(BillingSetting::Api),
        other => Err(anyhow::anyhow!(
            "invalid {flag} mode: {other} (expected auto, subscription, or api)"
        )),
    }
}

pub fn print_help() {
    println!(
        "ai-usage-tui {}
A btop-inspired dashboard for AI token usage.

USAGE:
    ai-usage-tui [OPTIONS]

OPTIONS:
    -h, --help    Show this help message
    -V, --version Show the version
    --once        Collect once and exit
    --json        Collect once and print JSON
    --csv PATH    Collect once and write CSV
    --config PATH Load configuration from TOML
    --db PATH     Override the OpenCode SQLite database path
    --journal PATH
                  Override the local usage journal path
    --claude-dir PATH
                  Override the Claude Code session-log directory
    --claude-billing MODE               How Claude Code usage is billed: auto (default),
                                        subscription, or api. Overrides [collectors.claude_code]
    --codex-dir PATH                    Override the Codex home ($CODEX_HOME, else ~/.codex);
                                        sessions/ and archived_sessions/ are read beneath it
    --codex-billing MODE                How Codex usage is billed: auto (default), subscription,
                                        or api. Overrides [collectors.codex]
                  (default: ~/.claude/projects)
    --days N       Show the last N days
    --today        Show today
    --week         Show the last 7 days
    --month        Show the last 30 days
    --all          Show all available history
    --provider NAME
                  Filter by provider
    --model NAME   Filter by model
    --record-ollama
                  Read an Ollama response JSON from stdin and journal it
    --refresh-zen  Refresh the cached OpenCode Zen model catalog and exit
    --refresh-pricing
                  Refresh the Zen pricing table from the docs page and exit
    --refresh-interval N
                  Refresh the dashboard every N seconds (default: 30)
    --check-budgets
                  Check budget thresholds and print alerts as JSON, exit 1 if any
    --webhook URL  Override the budget alert webhook URL from config
    --record-routing
                  Read a routing event JSON from stdin and journal it
    --routing-json
                  Export routing analytics as JSON and exit
    --routing-csv PATH
                  Export routing analytics as CSV and exit

ENVIRONMENT:
    OPENCODE_DB_PATH    Override the OpenCode SQLite database path
    CLAUDE_PROJECTS_DIR Override the Claude Code session-log directory
    CLAUDE_CONFIG_DIR   Claude Code's own config root; session logs are read
                        from $CLAUDE_CONFIG_DIR/projects. CLAUDE_PROJECTS_DIR
                        wins when both are set.
    CODEX_HOME          Codex's home; session logs are read from sessions/ and
                        archived_sessions/ beneath it
    AI_USAGE_JOURNAL_PATH
                        Override the local usage journal path
    AI_USAGE_LOG        Write diagnostics to a file. Set to 1 for the default
                        location under the data directory, or to a path.
                        Collector errors are invisible on stderr while the
                        dashboard holds the alternate screen.
    XDG_CONFIG_HOME     Base directory for the default config path
    XDG_DATA_HOME       Base directory for default database, journal, and
                        cache paths

KEYS:
    1              Today
    2              Last 7 days
    3              Last 30 days
    4              All time
    r              Refresh data
    b              Toggle budgets panel
    t              Toggle routing panel
    p              Toggle per-project cost panel
    g              Toggle the spend-over-time graph
    w              Toggle the burn-rate panel
    s              Toggle the sessions panel
    j / Down       Select next model
    k / Up         Select previous model
    ?              Key reference overlay
    q / Esc        Quit (also Ctrl-C)

CATEGORIES:
    LOCAL          Local provider usage
    CLOUD          Hosted/cloud-routed usage without authoritative cost
    FREE           Explicitly free model usage
    PAID           Usage with known cost
    UNKNOWN        Usage without pricing metadata

EXAMPLES:
    ai-usage-tui
    OPENCODE_DB_PATH=/path/to/opencode.db ai-usage-tui",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_and_version_flags_are_detected() {
        assert!(parse_cli(["--help"]).unwrap().help);
        assert!(parse_cli(["-h"]).unwrap().help);
        assert!(parse_cli(["--version"]).unwrap().version);
    }

    #[test]
    fn invalid_options_are_rejected() {
        assert!(parse_cli(["--not-a-real-option"]).is_err());
        assert!(parse_cli(["--refresh-interval", "0"]).is_err());
        assert!(parse_cli(["--once", "--record-ollama"]).is_err());
    }

    #[test]
    fn day_range_and_filters_parse() {
        let cli = parse_cli(["--days", "14", "--provider", "ollama", "--model", "model"]).unwrap();
        assert_eq!(cli.range.label(), "14 DAYS");
        assert!(cli.range_set);
        assert_eq!(cli.provider_filter.as_deref(), Some("ollama"));
        assert_eq!(cli.model_filter.as_deref(), Some("model"));
    }

    #[test]
    fn explicit_default_flags_are_marked() {
        let cli = parse_cli(["--week", "--refresh-interval", "30"]).unwrap();
        assert!(cli.range_set);
        assert!(cli.refresh_interval_set);
    }

    #[test]
    fn claude_billing_accepts_the_three_modes_and_nothing_else() {
        assert_eq!(
            parse_cli(["--claude-billing", "subscription"])
                .unwrap()
                .claude_billing,
            BillingSetting::Subscription
        );
        let api = parse_cli(["--claude-billing", "api"]).unwrap();
        assert_eq!(api.claude_billing, BillingSetting::Api);
        assert!(api.claude_billing_set);
        assert_eq!(
            parse_cli(["--claude-billing", "auto"])
                .unwrap()
                .claude_billing,
            BillingSetting::Auto
        );
        let error = match parse_cli(["--claude-billing", "sometimes"]) {
            Ok(_) => panic!("an unknown mode parsed"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("invalid --claude-billing mode"), "{error}");
        assert!(parse_cli(["--claude-billing"]).is_err());
    }

    #[test]
    fn codex_flags_parse() {
        let cli = parse_cli(["--codex-dir", "/x", "--codex-billing", "subscription"]).unwrap();
        assert_eq!(cli.codex_dir.as_deref(), Some(std::path::Path::new("/x")));
        assert_eq!(cli.codex_billing, BillingSetting::Subscription);
        assert!(cli.codex_billing_set);
        assert!(parse_cli(["--codex-dir"]).is_err());
        assert!(parse_cli(["--codex-billing", "maybe"]).is_err());
    }
}
