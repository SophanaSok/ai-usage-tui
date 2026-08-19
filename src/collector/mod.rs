pub mod background;
pub mod claude_code;
pub mod journal;
pub mod opencode;
pub mod pricing_refresh;
pub mod zen;

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;

use crate::collector::claude_code::{load_claude_code, Offsets};
use crate::collector::journal::load_journal;
use crate::collector::opencode::load_opencode;
use crate::collector::zen::zen_cache_path;
use crate::model::Usage;
use crate::pricing::{apply_estimated_pricing, PricingEngine};

/// One-shot read of every source.
///
/// `claude_root` overrides Claude Code's session-log directory; production passes `None` to use
/// the default, and tests pass an explicit path so they never read the developer's real
/// transcripts.
pub fn load_usage(
    override_path: Option<&Path>,
    journal: &Path,
    claude_root: Option<&Path>,
) -> Result<(Vec<Usage>, String)> {
    let (mut usages, opencode_source) = load_opencode(override_path)?;
    let journal_usages = load_journal(journal)?;
    let journal_source = if journal.exists() {
        format!("journal: {}", journal.display())
    } else {
        "journal: not initialized".to_string()
    };
    let zen_source = match zen_cache_path() {
        Some(path) if path.exists() => {
            format!("Zen catalog: cached (informational) at {}", path.display())
        }
        _ => "Zen catalog: not cached".to_string(),
    };
    let (claude_usages, claude_source) = load_claude_code(claude_root, &mut Offsets::new())
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
pub fn build_test_journal(dir: &Path) -> std::path::PathBuf {
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
