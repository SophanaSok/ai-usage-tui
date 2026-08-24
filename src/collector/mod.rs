pub mod background;
pub mod billing;
pub mod claude_code;
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
use crate::collector::journal::load_journal;
use crate::collector::opencode::load_opencode;
use crate::collector::zen::zen_cache_path;
use crate::model::Usage;
use crate::pricing::{apply_estimated_pricing, PricingEngine};

/// Where every source lives, and how its usage is billed.
///
/// Three functions took the same three paths as separate arguments and each grew a fourth
/// when billing arrived. One struct, extended in one place, threaded everywhere.
#[derive(Clone, Debug, Default)]
pub struct SourceRoots {
    pub db_path: Option<PathBuf>,
    pub journal: PathBuf,
    /// Root of Claude Code's session logs; `None` means the default (see `claude_code`).
    pub claude_dir: Option<PathBuf>,
    pub claude_billing: BillingSetting,
    /// Claude Code's `~/.claude.json`, when the default location is not the right one.
    pub claude_json: Option<PathBuf>,
}

impl SourceRoots {
    pub fn new(journal: PathBuf) -> Self {
        Self {
            journal,
            ..Default::default()
        }
    }

    pub fn from_cli(cli: &Cli, journal: PathBuf) -> Self {
        Self {
            db_path: cli.db_path.clone(),
            journal,
            claude_dir: cli.claude_dir.clone(),
            claude_billing: cli.claude_billing,
            claude_json: cli.claude_json.clone(),
        }
    }

    /// Where Claude Code's config document is for these roots. Derived from an overridden
    /// session-log root, so a test that points at a fixture never resolves the developer's own.
    pub fn claude_json_path(&self) -> Option<PathBuf> {
        config_json_path(self.claude_json.as_deref(), self.claude_dir.as_deref())
    }

    /// Decide Claude Code's billing from the evidence available right now.
    pub fn claude_decision(&self) -> Decision {
        let path = self.claude_json_path();
        detect(
            "claude_code",
            self.claude_billing,
            &Signals {
                claude_json: path.as_deref(),
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: None,
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

    let mut seen: HashSet<UsageKey> = usages.iter().map(usage_key).collect();
    for extra in journal_usages.into_iter().chain(claude_usages) {
        if seen.insert(usage_key(&extra)) {
            usages.push(extra);
        }
    }

    let engine = PricingEngine::load();
    apply_estimated_pricing(&mut usages, &engine);

    Ok((
        usages,
        format!(
            "{} | {} | {} | {}",
            opencode_source, claude_source, journal_source, zen_source
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
