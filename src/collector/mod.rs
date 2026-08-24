pub mod background;
pub mod billing;
pub mod claude_code;
pub mod codex;
pub mod journal;
pub mod opencode;
pub mod pricing_refresh;
pub mod zen;

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;

use crate::cli::Cli;
use crate::collector::billing::{detect, BillingSetting, Decision, Signals};
use crate::collector::claude_code::{config_json_path, load_claude_code, Offsets};
use crate::collector::codex::{load_codex, Cursors};
use crate::collector::journal::load_journal;
use crate::collector::opencode::load_opencode;
use crate::collector::zen::zen_cache_path;
use crate::model::Usage;
use crate::pricing::{apply_estimated_pricing, PricingEngine};

/// Where every source lives, and how its usage is billed.
///
/// Three functions took the same three paths as separate arguments and each grew a fourth
/// when billing arrived. One struct, extended in one place, threaded everywhere.
#[derive(Clone, Debug)]
pub struct SourceRoots {
    pub db_path: Option<PathBuf>,
    pub journal: PathBuf,
    /// Root of Claude Code's session logs; `None` means the default (see `claude_code`).
    pub claude_dir: Option<PathBuf>,
    pub claude_billing: BillingSetting,
    /// Claude Code's `~/.claude.json`, when the default location is not the right one.
    pub claude_json: Option<PathBuf>,
    /// Codex's home; `None` means `$CODEX_HOME` or `~/.codex` (see `codex`).
    pub codex_dir: Option<PathBuf>,
    pub codex_billing: BillingSetting,
    /// Omarchy's agents-panel records; `None` means the XDG state location (see `omarchy`).
    pub omarchy_dir: Option<PathBuf>,
    /// Whether to read those records at all. On by default: an absent directory is idle.
    pub limits_enabled: bool,
}

impl Default for SourceRoots {
    fn default() -> Self {
        Self {
            db_path: None,
            journal: PathBuf::new(),
            claude_dir: None,
            claude_billing: BillingSetting::Auto,
            claude_json: None,
            codex_dir: None,
            codex_billing: BillingSetting::Auto,
            omarchy_dir: None,
            limits_enabled: true,
        }
    }
}

impl SourceRoots {
    pub fn new(journal: PathBuf) -> Self {
        Self {
            journal,
            ..Default::default()
        }
    }

    /// The Omarchy records directory in force: explicit, else the XDG state location.
    pub fn omarchy_usage_dir(&self) -> Option<PathBuf> {
        self.omarchy_dir
            .clone()
            .or_else(crate::utils::omarchy_usage_dir)
    }

    /// The plan label Omarchy already derived for an agent, when its record is here.
    pub fn omarchy_tier(&self, agent: &str) -> Option<String> {
        if !self.limits_enabled {
            return None;
        }
        let dir = self.omarchy_usage_dir()?;
        crate::omarchy::tier_label_for(&dir, agent)
    }

    pub fn from_cli(cli: &Cli, journal: PathBuf) -> Self {
        Self {
            db_path: cli.db_path.clone(),
            journal,
            claude_dir: cli.claude_dir.clone(),
            claude_billing: cli.claude_billing,
            claude_json: cli.claude_json.clone(),
            codex_dir: cli.codex_dir.clone(),
            codex_billing: cli.codex_billing,
            omarchy_dir: cli.omarchy_dir.clone(),
            limits_enabled: cli.limits_enabled,
        }
    }

    /// Where Claude Code's config document is for these roots. Derived from an overridden
    /// session-log root, so a test that points at a fixture never resolves the developer's own.
    pub fn claude_json_path(&self) -> Option<PathBuf> {
        config_json_path(self.claude_json.as_deref(), self.claude_dir.as_deref())
    }

    /// Decide Codex's billing. Codex has no config document this tool will read — its
    /// `auth.json` is a credential file — so the signals are the setting and the environment.
    pub fn codex_decision(&self) -> Decision {
        let tier = self.omarchy_tier("codex");
        detect(
            "codex",
            self.codex_billing,
            &Signals {
                claude_json: None,
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: tier.as_deref(),
            },
        )
    }

    /// Decide Claude Code's billing from the evidence available right now.
    pub fn claude_decision(&self) -> Decision {
        let path = self.claude_json_path();
        let tier = self.omarchy_tier("claude_code");
        detect(
            "claude_code",
            self.claude_billing,
            &Signals {
                claude_json: path.as_deref(),
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: tier.as_deref(),
            },
        )
    }
}

/// One-shot read of every source.
///
/// Production passes the roots resolved from the CLI and config; tests pass explicit paths so
/// they never read the developer's real transcripts or config.
pub fn load_usage(roots: &SourceRoots) -> Result<(Vec<Usage>, String)> {
    let (mut usages, opencode_source) = load_opencode(roots.db_path.as_deref())?;
    let journal_usages = load_journal(&roots.journal)?;
    let journal_source = if roots.journal.exists() {
        format!("journal: {}", roots.journal.display())
    } else {
        "journal: not initialized".to_string()
    };
    let zen_source = match zen_cache_path() {
        Some(path) if path.exists() => {
            format!("Zen catalog: cached (informational) at {}", path.display())
        }
        _ => "Zen catalog: not cached".to_string(),
    };
    let decision = roots.claude_decision();
    let (claude_usages, claude_source) =
        load_claude_code(roots.claude_dir.as_deref(), &mut Offsets::new(), &decision)
            .unwrap_or_else(|error| (Vec::new(), format!("Claude Code: unavailable ({})", error)));

    let codex_decision = roots.codex_decision();
    let (codex_usages, codex_source) = load_codex(
        roots.codex_dir.as_deref(),
        &mut Cursors::new(),
        &codex_decision,
    )
    .unwrap_or_else(|error| (Vec::new(), format!("Codex: unavailable ({})", error)));

    let mut seen: HashSet<UsageKey> = usages.iter().map(usage_key).collect();
    for extra in journal_usages
        .into_iter()
        .chain(claude_usages)
        .chain(codex_usages)
    {
        if seen.insert(usage_key(&extra)) {
            usages.push(extra);
        }
    }

    let engine = PricingEngine::load();
    apply_estimated_pricing(&mut usages, &engine);

    Ok((
        usages,
        format!(
            "{} | {} | {} | {} | {}",
            opencode_source, claude_source, codex_source, journal_source, zen_source
        ),
    ))
}

/// Identity of a usage event for deduplication.
///
/// Token counts alone are not an identity: agent loops routinely emit many requests with
/// byte-identical counts, and keying on shape alone silently discards them, under-reporting
/// real spend. Prefer the source's own id; fall back to shape *plus* timestamp.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UsageKey {
    Event(String),
    Shape {
        provider: String,
        model: String,
        input: u64,
        output: u64,
        reasoning: u64,
        cache_read: u64,
        cache_write: u64,
        created: i64,
    },
}

pub fn usage_key(usage: &Usage) -> UsageKey {
    match &usage.event_id {
        Some(id) if !id.is_empty() => UsageKey::Event(id.clone()),
        _ => UsageKey::Shape {
            provider: usage.provider.clone(),
            model: usage.model.clone(),
            input: usage.input,
            output: usage.output,
            reasoning: usage.reasoning,
            cache_read: usage.cache_read,
            cache_write: usage.cache_write,
            created: usage.created,
        },
    }
}

/// Path to the committed OpenCode fixture database.
///
/// Anchored to the manifest directory: a relative path silently resolves against whatever
/// working directory the test runner happens to use.
#[cfg(test)]
pub fn setup_test_db() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/opencode_test.db"
    ))
}

/// Build a journal database with known contents inside `dir`.
///
/// The previous fixture was a checked-in binary that `.gitignore` excluded, so on a fresh
/// clone it did not exist -- and `load_journal` returns an empty vec for a missing path, which
/// let the pipeline test pass while silently covering nothing. Constructing it here makes the
/// fixture reproducible and the assertions real.
#[cfg(test)]
pub fn build_test_journal(dir: &std::path::Path) -> std::path::PathBuf {
    use rusqlite::params;

    let path = dir.join("journal.db");
    let conn = rusqlite::Connection::open(&path).expect("create journal fixture");
    conn.execute_batch(
        "CREATE TABLE usage_event (
            id INTEGER PRIMARY KEY,
            event_id TEXT,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            category TEXT NOT NULL,
            cost_status TEXT NOT NULL,
            requests INTEGER NOT NULL,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            reasoning_tokens INTEGER NOT NULL,
            cache_read_tokens INTEGER NOT NULL,
            cache_write_tokens INTEGER NOT NULL,
            cost REAL,
            created INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX usage_event_event_id ON usage_event(event_id);",
    )
    .expect("create journal schema");

    let rows = [
        (
            "jrnl-1",
            "ollama",
            "qwen3-coder-agent",
            "LOCAL",
            "local",
            1200_i64,
            340_i64,
        ),
        ("jrnl-2", "ollama", "gemma3:4b", "LOCAL", "local", 800, 210),
        // Same shape as jrnl-2 but a distinct event: must not be deduplicated away.
        ("jrnl-3", "ollama", "gemma3:4b", "LOCAL", "local", 800, 210),
    ];
    for (index, (event_id, provider, model, category, status, input, output)) in
        rows.iter().enumerate()
    {
        conn.execute(
            "INSERT INTO usage_event (event_id, provider, model, category, cost_status, requests,
             input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens,
             cost, created) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, 0, 0, 0, NULL, ?8)",
            params![
                event_id,
                provider,
                model,
                category,
                status,
                input,
                output,
                1_700_000_000_i64 + index as i64 * 60,
            ],
        )
        .expect("seed journal fixture");
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_omarchy_record_decides_billing_when_nothing_else_does() {
        let fixtures = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/omarchy"
        ));
        let dir = tempfile::TempDir::new().unwrap();
        let mut roots = SourceRoots {
            claude_dir: Some(dir.path().join(".claude").join("projects")),
            omarchy_dir: Some(fixtures),
            ..Default::default()
        };
        // Only when no API-key variable intervenes; a developer's shell must not decide.
        if crate::collector::billing::api_env_vars("claude_code")
            .iter()
            .chain(crate::collector::billing::api_env_vars("codex"))
            .any(|name| crate::collector::billing::env_has(name))
        {
            return;
        }
        let claude = roots.claude_decision();
        assert_eq!(claude.billing, crate::model::Billing::Subscription);
        assert_eq!(claude.tier.as_deref(), Some("Max 20x"));
        assert_eq!(claude.reason, "omarchy record");
        assert_eq!(roots.codex_decision().tier.as_deref(), Some("plus"));

        roots.limits_enabled = false;
        assert_eq!(
            roots.claude_decision().reason,
            crate::collector::billing::Decision::REASON_UNKNOWN,
            "disabling the Omarchy reader also removes it as a billing signal"
        );
    }
}
