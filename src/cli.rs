use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;

use crate::collector::billing::BillingSetting;
use crate::model::Range;

pub struct Cli {
    pub help: bool,
    pub version: bool,
    /// Report what each data source resolved to and exit, without starting the dashboard.
    pub doctor: bool,
    /// Print a shell completion script and exit.
    pub completions: Option<clap_complete::Shell>,
    /// Print the man page in roff and exit.
    pub man: bool,
    /// `[collectors.<id>] enabled` overrides, by source id. Absent means the registry default.
    pub source_enabled: std::collections::BTreeMap<String, bool>,
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
    /// Gemini CLI's home; defaults to `~/.gemini`. Its telemetry log is read from beneath it.
    pub gemini_dir: Option<PathBuf>,
    pub gemini_billing: BillingSetting,
    pub gemini_billing_set: bool,
    /// Omarchy's agents-panel records; defaults to the XDG state location.
    pub omarchy_dir: Option<PathBuf>,
    pub limits_enabled: bool,
    /// Write this tool's usage and budgets as a record Omarchy's agents panel shows, and exit.
    pub omarchy_record: bool,
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
            doctor: false,
            completions: None,
            man: false,
            source_enabled: Default::default(),
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
            gemini_dir: None,
            gemini_billing: BillingSetting::Auto,
            gemini_billing_set: false,
            omarchy_dir: None,
            limits_enabled: true,
            omarchy_record: false,
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

/// The command line, as clap parses it.
///
/// Deliberately a separate struct from [`Cli`], which every consumer already takes. Clap's
/// natural shape is `Option<T>` for "was it given", while `Cli` carries explicit `*_set`
/// booleans that `config::apply_config` reads to decide whether a config value may fill a field
/// in. Converting between them here keeps the migration inside this file: `parse_cli` has the
/// same signature and returns the same `Cli` it always did, so `main.rs`, `config.rs`,
/// `SourceRoots::from_cli` and the UI are untouched and cannot have been changed by accident.
#[derive(clap::Parser, Debug)]
#[command(
    name = "ai-usage-tui",
    version,
    about = "A btop-inspired dashboard for AI token usage.",
    // Last occurrence wins, which is what the hand-rolled parser did: it simply assigned, so a
    // repeated flag overwrote. Clap's default is to reject a repeat, which would break anything
    // that layers defaults and then overrides them -- a wrapper script, a shell alias, or the
    // test harness, which passes --omarchy-dir itself and then again per test.
    args_override_self = true,
    // Clap generates OPTIONS from the fields below, so the flags it accepts and the flags it
    // documents cannot drift. The sections it cannot know about are appended.
    disable_help_flag = false,
)]
struct Args {
    /// Report where each data source was looked for, what was found there, and how billing was
    /// decided, then exit
    #[arg(long, group = "action")]
    doctor: bool,

    /// Load configuration from TOML
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Print a shell completion script and exit
    #[arg(long, value_name = "SHELL", group = "action")]
    completions: Option<clap_complete::Shell>,

    /// Print the man page (roff) and exit
    #[arg(long, group = "action")]
    man: bool,

    // --- data sources ---------------------------------------------------------------------
    /// Override the OpenCode SQLite database path
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,

    /// Override the local usage journal path
    #[arg(long, value_name = "PATH")]
    journal: Option<PathBuf>,

    /// Override the Claude Code session-log directory [default: ~/.claude/projects]
    #[arg(long, value_name = "PATH")]
    claude_dir: Option<PathBuf>,

    /// How Claude Code usage is billed; overrides [collectors.claude_code]
    #[arg(long, value_name = "MODE")]
    claude_billing: Option<BillingSetting>,

    /// Override the Codex home ($CODEX_HOME, else ~/.codex)
    #[arg(long, value_name = "PATH")]
    codex_dir: Option<PathBuf>,

    /// How Codex usage is billed; overrides [collectors.codex]
    #[arg(long, value_name = "MODE")]
    codex_billing: Option<BillingSetting>,

    /// Override the Gemini CLI home (default: ~/.gemini)
    #[arg(long, value_name = "PATH")]
    gemini_dir: Option<PathBuf>,

    /// How Gemini CLI usage is billed; overrides [collectors.gemini]
    #[arg(long, value_name = "MODE")]
    gemini_billing: Option<BillingSetting>,

    /// Override where Omarchy's agents panel keeps its usage records
    #[arg(long, value_name = "PATH")]
    omarchy_dir: Option<PathBuf>,

    // --- range and filters ----------------------------------------------------------------
    /// Show today
    #[arg(long)]
    today: bool,
    /// Show the last 7 days (default)
    #[arg(long)]
    week: bool,
    /// Show the last 30 days
    #[arg(long)]
    month: bool,
    /// Show all available history
    #[arg(long)]
    all: bool,
    /// Show the last N days
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u64).range(1..))]
    days: Option<u64>,

    /// Filter by provider
    #[arg(long, value_name = "NAME")]
    provider: Option<String>,
    /// Filter by model
    #[arg(long, value_name = "NAME")]
    model: Option<String>,

    // --- dashboard and alerts -------------------------------------------------------------
    /// Refresh the dashboard every N seconds [default: 30]
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u64).range(1..))]
    refresh_interval: Option<u64>,

    /// Override the budget alert webhook URL from config
    #[arg(long, value_name = "URL")]
    webhook: Option<String>,

    // --- one-shot actions -------------------------------------------------------------------
    // `action` is a clap group, so at most one may be given. `--once` conflicts separately with
    // the *collection* actions but not with --json/--csv, which set it themselves.
    /// Collect once and print plain text
    #[arg(long, conflicts_with_all = COLLECTION_ACTIONS)]
    once: bool,
    /// Collect once and print JSON
    #[arg(long, group = "action")]
    json: bool,
    /// Collect once and write CSV
    #[arg(long, value_name = "PATH", group = "action")]
    csv: Option<PathBuf>,
    /// Check budget thresholds and print alerts as JSON, exit 1 if any are actionable
    #[arg(long, group = "action")]
    check_budgets: bool,
    /// Export routing analytics as JSON
    #[arg(long, group = "action")]
    routing_json: bool,
    /// Export routing analytics as CSV
    #[arg(long, value_name = "PATH", group = "action")]
    routing_csv: Option<PathBuf>,
    /// Read an Ollama response JSON from stdin and journal it
    #[arg(long, group = "action")]
    record_ollama: bool,
    /// Read a routing event JSON from stdin and journal it
    #[arg(long, group = "action")]
    record_routing: bool,
    /// Refresh the cached OpenCode Zen model catalog
    #[arg(long, group = "action")]
    refresh_zen: bool,
    /// Refresh the Zen pricing table from the docs page
    #[arg(long, group = "action")]
    refresh_pricing: bool,
    /// Write usage and budgets as a record for Omarchy's agents panel
    #[arg(long, group = "action")]
    omarchy_record: bool,
}

/// The actions that collect and exit, which `--once` cannot be combined with.
///
/// `--json` and `--csv` are absent on purpose: they *imply* `--once`, so `--once --json` has
/// always been accepted and still is.
const COLLECTION_ACTIONS: &[&str] = &[
    "record_ollama",
    "refresh_zen",
    "check_budgets",
    "record_routing",
    "omarchy_record",
    "doctor",
];

/// The parser's `Command`, for help, completions and the man page.
pub fn command() -> clap::Command {
    <Args as clap::CommandFactory>::command().after_help(after_help())
}

pub fn parse_cli<I>(args: I) -> Result<Cli>
where
    I: IntoIterator,
    I::Item: Into<String>,
{
    // `try_parse_from` rather than `parse_from`: the caller decides how to report an error, and
    // the tests need a `Result` rather than a process exit. A leading argv[0] is inserted
    // because callers pass only the arguments, as they did to the hand-rolled parser.
    let argv = std::iter::once("ai-usage-tui".to_string()).chain(args.into_iter().map(Into::into));
    match command().try_get_matches_from(argv) {
        Ok(matches) => {
            let args = <Args as clap::FromArgMatches>::from_arg_matches(&matches)?;
            Ok(Cli::from_parts(args, &matches))
        }
        // Clap reports `--help` and `--version` as errors carrying the text to print. They are
        // not failures: `main` has always printed them itself and exited 0, and turning them
        // into an `Err` here would print help to stderr with a non-zero status. Signalled on
        // `Cli` exactly as the hand-rolled parser did.
        Err(error) => match error.kind() {
            clap::error::ErrorKind::DisplayHelp
            | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => Ok(Cli {
                help: true,
                ..Default::default()
            }),
            clap::error::ErrorKind::DisplayVersion => Ok(Cli {
                version: true,
                ..Default::default()
            }),
            _ => Err(error.into()),
        },
    }
}

impl Cli {
    fn from_parts(args: Args, matches: &clap::ArgMatches) -> Self {
        // `--json` and `--csv` collect once and exit; that has always been implied rather than
        // required, so `--json` alone works.
        let once = args.once
            || args.json
            || args.csv.is_some()
            || args.routing_json
            || args.routing_csv.is_some();

        // Last one wins, by the position it appeared at on the command line.
        //
        // Not a clap `group`, which would reject `--week --today` outright, and not a fixed
        // priority either. The hand-rolled parser simply assigned `cli.range` in the order it
        // walked the arguments, so the last range flag given won -- and something that layers a
        // default (`--week` in an alias) and then overrides it (`--today`) depends on that.
        let ranges: [(&str, Range); 5] = [
            ("today", Range::Today),
            ("week", Range::Week),
            ("month", Range::Month),
            ("all", Range::All),
            ("days", Range::Days(args.days.unwrap_or(1))),
        ];
        // `value_source` first, then `index_of`. A boolean flag clap did not see still has a
        // value -- its `false` default -- and `index_of` answers for that too, so ordering alone
        // reported `--all` as given on every invocation.
        let last = ranges
            .iter()
            .filter(|(name, _)| {
                matches.value_source(name) == Some(clap::parser::ValueSource::CommandLine)
            })
            .filter_map(|(name, range)| matches.index_of(name).map(|at| (at, *range)))
            .max_by_key(|(at, _)| *at);
        let range_set = last.is_some();
        let range = last.map_or(Range::Week, |(_, range)| range);

        Self {
            // `--help` and `--version` never reach here: clap handles both and exits. The fields
            // stay so `Cli`'s shape is unchanged for every consumer.
            help: false,
            version: false,
            doctor: args.doctor,
            completions: args.completions,
            man: args.man,
            config_path: args.config,
            db_path: args.db,
            journal_path: args.journal,
            claude_dir: args.claude_dir,
            claude_billing: args.claude_billing.unwrap_or(BillingSetting::Auto),
            claude_billing_set: args.claude_billing.is_some(),
            claude_json: None,
            codex_dir: args.codex_dir,
            codex_billing: args.codex_billing.unwrap_or(BillingSetting::Auto),
            codex_billing_set: args.codex_billing.is_some(),
            gemini_dir: args.gemini_dir,
            gemini_billing: args.gemini_billing.unwrap_or(BillingSetting::Auto),
            gemini_billing_set: args.gemini_billing.is_some(),
            omarchy_dir: args.omarchy_dir,
            limits_enabled: true,
            omarchy_record: args.omarchy_record,
            range,
            range_set,
            provider_filter: args.provider,
            model_filter: args.model,
            once,
            json: args.json,
            csv_path: args.csv,
            record_ollama: args.record_ollama,
            refresh_zen: args.refresh_zen,
            refresh_pricing: args.refresh_pricing,
            refresh_interval: Duration::from_secs(args.refresh_interval.unwrap_or(30)),
            refresh_interval_set: args.refresh_interval.is_some(),
            check_budgets: args.check_budgets,
            webhook_url: args.webhook,
            record_routing: args.record_routing,
            routing_json: args.routing_json,
            routing_csv_path: args.routing_csv,
            source_enabled: Default::default(),
        }
    }
}

/// The parts of `--help` clap cannot generate from the argument definitions.
///
/// `KEYS` is rendered from `ui::keys::BINDINGS`, the one table the dispatch and the `?` overlay
/// also read, so the three cannot disagree.
fn after_help() -> String {
    let keys = crate::ui::keys::rows()
        .map(|(keys, what)| format!("    {keys:<14} {what}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "ENVIRONMENT:
    OPENCODE_DB_PATH    Override the OpenCode SQLite database path
    CLAUDE_PROJECTS_DIR Override the Claude Code session-log directory
    CLAUDE_CONFIG_DIR   Claude Code's own config root; session logs are read
                        from $CLAUDE_CONFIG_DIR/projects. CLAUDE_PROJECTS_DIR
                        wins when both are set.
    CODEX_HOME          Codex's home; session logs are read from sessions/ and
                        archived_sessions/ beneath it
    GEMINI_TELEMETRY_OUTFILE
                        Gemini CLI's own telemetry output path; read from there
                        when set
    AI_USAGE_JOURNAL_PATH
                        Override the local usage journal path
    AI_USAGE_LOG        Write diagnostics to a file. Set to 1 for the default
                        location under the data directory, or to a path.
                        Collector errors are invisible on stderr while the
                        dashboard holds the alternate screen.
    XDG_CONFIG_HOME     Base directory for the default config path
    XDG_DATA_HOME       Base directory for default database, journal, and
                        cache paths
    XDG_STATE_HOME      Base directory for Omarchy's agents-panel records
                        (omarchy/agents/usage beneath it)

KEYS:
{keys}

CATEGORIES:
    LOCAL          Local provider usage
    CLOUD          Hosted/cloud-routed usage without authoritative cost
    FREE           Explicitly free model usage
    PAID           Usage from a provider that bills per token
    UNKNOWN        Usage whose provider is not recognised as billing per token

EXAMPLES:
    ai-usage-tui
    OPENCODE_DB_PATH=/path/to/opencode.db ai-usage-tui"
    )
}

/// Write a shell completion script to stdout.
///
/// Generated from the same `Command` that parses the arguments, so a flag cannot exist without
/// completing — which is exactly what a hand-written completion script cannot promise.
pub fn print_completions(shell: clap_complete::Shell) {
    let mut command = command();
    let name = command.get_name().to_string();
    clap_complete::generate(shell, &mut command, name, &mut std::io::stdout());
}

/// Write the man page (roff) to stdout.
pub fn print_man() -> std::io::Result<()> {
    clap_mangen::Man::new(command()).render(&mut std::io::stdout())
}

/// Print the full help, as `--help` does.
///
/// Kept because `main.rs` still has a `help` branch, and because a caller that wants the text
/// without exiting the process should not have to go through clap's error path.
pub fn print_help() {
    let _ = command().print_help();
    println!();
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
        // Clap words this differently from the hand-rolled parser, and lists the valid values
        // rather than only naming them in prose. Assert on what has to be true for the message
        // to be useful, not on its exact phrasing.
        assert!(error.contains("--claude-billing"), "{error}");
        assert!(error.contains("sometimes"), "{error}");
        for mode in ["auto", "subscription", "api"] {
            assert!(
                error.contains(mode),
                "the valid modes should be listed: {error}"
            );
        }
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

    /// Range flags combine, and the last one given wins.
    ///
    /// The hand-rolled parser assigned `cli.range` as it walked the arguments, so a later flag
    /// overwrote an earlier one. A clap `group` would reject the combination instead, and a fixed
    /// priority order would make `--week --today` and `--today --week` mean the same thing. Both
    /// break layering a default in an alias and overriding it on the command line.
    #[test]
    fn the_last_range_flag_given_wins() {
        assert_eq!(
            parse_cli(["--week", "--today"]).unwrap().range.label(),
            "TODAY"
        );
        assert_eq!(
            parse_cli(["--today", "--week"]).unwrap().range.label(),
            "7 DAYS",
            "order must matter, so this is not a fixed priority"
        );
        assert_eq!(
            parse_cli(["--today", "--all"]).unwrap().range.label(),
            "ALL TIME"
        );
        assert_eq!(
            parse_cli(["--all", "--days", "14"]).unwrap().range.label(),
            "14 DAYS"
        );
        assert_eq!(
            parse_cli(["--days", "14", "--month"])
                .unwrap()
                .range
                .label(),
            "30 DAYS"
        );
        // And `range_set` is true whenever any of them was given, which is what stops a config
        // `days` value from overriding an explicit flag.
        assert!(parse_cli(["--week", "--today"]).unwrap().range_set);
        assert!(!parse_cli([] as [&str; 0]).unwrap().range_set);
    }

    /// The mutual-exclusion rule is two rules, not one, and the second is asymmetric.
    ///
    /// At most one action may be given. Separately, `--once` conflicts with the *collection*
    /// actions — but the hand-rolled parser's list for that second rule omitted
    /// `--refresh-pricing`, so `--once --refresh-pricing` was accepted. Transcribed rather than
    /// tidied: an ArgGroup covering all eleven would silently "fix" the asymmetry and reject a
    /// combination that has always worked.
    #[test]
    fn the_once_conflict_keeps_its_asymmetry() {
        // Rejected: --once with a collection action.
        for action in [
            "--record-ollama",
            "--refresh-zen",
            "--check-budgets",
            "--record-routing",
            "--omarchy-record",
            "--doctor",
        ] {
            assert!(
                parse_cli(["--once", action]).is_err(),
                "--once {action} should be rejected"
            );
        }
        // Accepted, and deliberately so: --refresh-pricing is not in that second list.
        assert!(
            parse_cli(["--once", "--refresh-pricing"]).is_ok(),
            "--once --refresh-pricing has always been accepted"
        );
    }

    /// At most one action, however they are spelled.
    #[test]
    fn two_actions_are_always_rejected() {
        for pair in [
            ["--json", "--csv"],
            ["--json", "--doctor"],
            ["--check-budgets", "--routing-json"],
            ["--refresh-zen", "--refresh-pricing"],
            ["--record-ollama", "--record-routing"],
        ] {
            let args = if pair[1] == "--csv" {
                vec![pair[0], pair[1], "/tmp/x.csv"]
            } else {
                vec![pair[0], pair[1]]
            };
            assert!(
                parse_cli(args.clone()).is_err(),
                "{args:?} should be rejected"
            );
        }
    }

    /// The export flags imply `--once`; that is why `--json` alone works.
    #[test]
    fn the_export_flags_imply_collect_once() {
        for flag in ["--json", "--routing-json"] {
            assert!(
                parse_cli([flag]).unwrap().once,
                "{flag} should imply --once"
            );
        }
        assert!(parse_cli(["--csv", "/tmp/x.csv"]).unwrap().once);
        assert!(parse_cli(["--routing-csv", "/tmp/x.csv"]).unwrap().once);
        // And a bare --once is not an export.
        let cli = parse_cli(["--once"]).unwrap();
        assert!(cli.once && !cli.json && cli.csv_path.is_none());
    }

    /// A repeated flag takes the last value, as the hand-rolled parser did by simply assigning.
    /// Clap rejects repeats by default, which would break layering a default then overriding it.
    #[test]
    fn a_repeated_flag_takes_the_last_value() {
        let cli = parse_cli(["--db", "/first.db", "--db", "/second.db"]).unwrap();
        assert_eq!(
            cli.db_path.as_deref(),
            Some(std::path::Path::new("/second.db"))
        );
        let cli = parse_cli(["--days", "7", "--days", "30"]).unwrap();
        assert_eq!(cli.range.label(), "30 DAYS");
    }

    /// Zero was rejected by the hand-rolled parser and must still be.
    #[test]
    fn zero_is_not_a_range_or_an_interval() {
        assert!(parse_cli(["--days", "0"]).is_err());
        assert!(parse_cli(["--refresh-interval", "0"]).is_err());
        assert!(parse_cli(["--days", "1"]).is_ok());
        assert!(parse_cli(["--refresh-interval", "1"]).is_ok());
    }

    #[test]
    fn omarchy_record_is_a_single_purpose_action() {
        assert!(parse_cli(["--omarchy-record"]).unwrap().omarchy_record);
        assert!(parse_cli(["--omarchy-record", "--json"]).is_err());
        assert!(parse_cli(["--omarchy-record", "--once"]).is_err());
        let cli = parse_cli(["--omarchy-record", "--omarchy-dir", "/x"]).unwrap();
        assert_eq!(cli.omarchy_dir.as_deref(), Some(std::path::Path::new("/x")));
    }
}
