//! GitHub Copilot CLI usage.
//!
//! Copilot has written its usage in three different shapes across its life, and only two of
//! them carry numbers worth reporting. This collector reads those two and stops:
//!
//! 1. **`assistant_usage_events`** in the CLI's SQLite store — one row per model request, with
//!    real input/output/cache/reasoning counts and its own timestamp. Preferred whenever the
//!    table exists.
//! 2. **`session.shutdown`** in the legacy `session-state/<id>/events.jsonl` — a cumulative
//!    per-session, per-model aggregate. Used only when there is no usable table.
//!
//! What is deliberately not read: the per-`assistant.message` path, whose input token count is
//! recorded as `0` by the CLI, and the VS Code transcripts, which carry no counts at all. Other
//! tools reconstruct both by dividing a character count by four and then pricing the result.
//! A token count inferred from message length is not a measurement, and a row built from one
//! would flow into budgets and the pricing-coverage figure as though it were. Copilot usage
//! this tool cannot measure is reported as absent, not estimated.
//!
//! Privacy: `events.jsonl` holds prompts, completions and tool arguments, and the CLI store
//! holds the same in its `turns` table. Neither is parsed. Only the usage fields named above
//! are read, and no message content is retained.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::classify::classify;
use crate::collector::background::Collector;
use crate::collector::billing::Decision;
use crate::collector::claude_code::normalize_project_path;
use crate::collector::opencode::{parse_created_at, timestamp_seconds};
use crate::helpers::{number, string};
use crate::model::{CostStatus, Usage};
use crate::utils::home_dir;

/// This source's canonical id: the `Collector::name()` it reports, the
/// `[collectors.<id>]` table that configures it, and its key in the source registry.
/// One constant so those can never drift apart.
pub const ID: &str = "copilot";

/// What every row this collector emits reports as its provider. Tokenised to `github` +
/// `copilot` by `classify`, and `copilot` is in `PAID_PROVIDERS` — the same treatment Claude
/// Code on a plan gets, where the category says who bills and `cost_status` says whether a
/// per-token rate exists.
const PROVIDER: &str = "github-copilot";

/// The CLI's SQLite store, whichever name this build uses.
///
/// The filename has moved between releases, so the candidates are tried in turn and the choice
/// is made on **schema, not filename**: whichever file actually has `assistant_usage_events`
/// wins. A build that renames it again is then a no-op here rather than a silent zero.
const DB_CANDIDATES: &[&str] = &["session-store.db", "session.db", "data.db"];

/// The table this collector exists to read.
const USAGE_TABLE: &str = "assistant_usage_events";

/// Copilot's home, `$COPILOT_HOME` or `~/.copilot`.
pub fn copilot_home() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("COPILOT_HOME") {
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    Some(home_dir()?.join(".copilot"))
}

/// The cumulative token counts one `session.shutdown` reported for one model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Totals {
    requests: u64,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
}

impl Totals {
    /// What arrived since `previous`. Shutdown snapshots are cumulative for a resumed session,
    /// so emitting one whole is emitting everything before it a second time.
    fn since(self, previous: Totals) -> Totals {
        Totals {
            requests: self.requests.saturating_sub(previous.requests),
            input: self.input.saturating_sub(previous.input),
            output: self.output.saturating_sub(previous.output),
            reasoning: self.reasoning.saturating_sub(previous.reasoning),
            cache_read: self.cache_read.saturating_sub(previous.cache_read),
            cache_write: self.cache_write.saturating_sub(previous.cache_write),
        }
    }

    fn tokens(self) -> u64 {
        self.input + self.output + self.reasoning + self.cache_read + self.cache_write
    }
}

/// A resume point for incremental reads.
///
/// Two halves, because the two paths resume differently: a high-water mark on `created_at` for
/// the request table, and a byte offset plus the last cumulative snapshot per session/model for
/// the legacy log.
#[derive(Clone, Debug, Default)]
pub struct Cursor {
    events: Option<i64>,
    legacy_offsets: HashMap<PathBuf, u64>,
    legacy_totals: HashMap<(String, String), Totals>,
}

impl Cursor {
    pub fn start() -> Self {
        Self::default()
    }

    pub fn high_water(&self) -> Option<i64> {
        self.events
    }

    fn advance(&mut self, created_at: i64) {
        self.events = Some(match self.events {
            Some(current) => current.max(created_at),
            None => created_at,
        });
    }
}

/// SQLite integers are signed; a negative token count is corruption and zero is the honest
/// reading of it.
fn count(value: Option<i64>) -> u64 {
    value.unwrap_or(0).max(0) as u64
}

/// Whether `table` exists in this database.
fn has_table(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

/// Whether `column` exists on `table`. Copilot has added token buckets over time, and this is a
/// read-only path that cannot migrate an older store — so an absent column is selected as NULL
/// rather than making the whole query fail.
fn has_column(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let sql = format!("SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1)");
    conn.query_row(&sql, [column], |row| row.get(0))
}

/// The store to read, chosen by schema rather than by name.
fn find_usage_db(root: &Path) -> Option<PathBuf> {
    for name in DB_CANDIDATES {
        let path = root.join(name);
        if !path.exists() {
            continue;
        }
        let Ok(conn) = open_read_only(&path) else {
            continue;
        };
        if has_table(&conn, USAGE_TABLE).unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

fn open_read_only(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(Duration::from_millis(250))?;
    Ok(conn)
}

/// `created_at` is an integer on some builds and an RFC 3339 string on others. Both are read;
/// neither is guessed at.
fn created_at_seconds(row: &rusqlite::Row<'_>, index: usize) -> Option<i64> {
    if let Ok(Some(raw)) = row.get::<_, Option<i64>>(index) {
        return Some(timestamp_seconds(raw));
    }
    row.get::<_, Option<String>>(index)
        .ok()
        .flatten()
        .as_deref()
        .and_then(parse_created_at)
}

/// One `assistant_usage_events` row.
///
/// `input_tokens` is inclusive of the cache buckets, so they are subtracted back out: `Usage`'s
/// token fields are disjoint and every total sums all five. Reasoning comes out of output for
/// the same reason. This is the Codex convention, and `total_tokens()` is unchanged by it.
fn usage_from_event_row(row: &rusqlite::Row<'_>, decision: &Decision) -> Option<Usage> {
    let session_id: Option<String> = row.get(0).ok().flatten();
    let model: String = row
        .get::<_, Option<String>>(1)
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".into());

    let cache_read = count(row.get(4).ok().flatten());
    let cache_write = count(row.get(5).ok().flatten());
    let reasoning = count(row.get(6).ok().flatten());
    let input = count(row.get(2).ok().flatten())
        .saturating_sub(cache_read)
        .saturating_sub(cache_write);
    let output = count(row.get(3).ok().flatten()).saturating_sub(reasoning);

    if input == 0 && output == 0 && reasoning == 0 && cache_read == 0 && cache_write == 0 {
        return None;
    }

    let created = created_at_seconds(row, 7).unwrap_or(0);
    let cwd: Option<String> = row.get(8).ok().flatten();
    let repository: Option<String> = row.get(9).ok().flatten();
    let turn_index: Option<i64> = row.get(10).ok().flatten();

    let event_id = session_id.as_ref().map(|session| match turn_index {
        Some(turn) => format!("copilot:{session}:{turn}"),
        // No turn index on this build: the timestamp plus the call's own totals is still a
        // content-derived identity, which is what dedup needs.
        None => format!("copilot:{session}:{created}:{input}:{output}"),
    });

    Some(Usage {
        event_id,
        category: classify(PROVIDER, &model),
        provider: PROVIDER.to_string(),
        model,
        requests: 1,
        input,
        output,
        reasoning,
        cache_read,
        cache_write,
        // Copilot publishes no per-request dollar figure this tool will trust: a seat bills
        // premium requests, not tokens. Left open for `apply_estimated_pricing`, which turns a
        // subscription row into `Quota` and parks the list rate in `api_equivalent_cost`.
        cost: None,
        cost_status: CostStatus::Unavailable,
        billing: decision.billing,
        api_equivalent_cost: None,
        created,
        session_id,
        project: cwd.as_deref().map(normalize_project_path).or(repository),
    })
}

/// Read the request table at or after `cursor`.
fn load_events(
    path: &Path,
    cursor: &mut Cursor,
    decision: &Decision,
) -> Result<(Vec<Usage>, usize)> {
    let conn = open_read_only(path)?;
    // Every column the collector wants, with the ones a given build may not have selected as
    // NULL so the positional indices above are stable whatever the store's age.
    let optional = [
        "cache_read_tokens",
        "cache_write_tokens",
        "reasoning_tokens",
        "cwd",
        "repository",
        "turn_index",
    ];
    let mut selected: Vec<String> = vec![
        "session_id".into(),
        "model".into(),
        "input_tokens".into(),
        "output_tokens".into(),
    ];
    for name in optional {
        if has_column(&conn, USAGE_TABLE, name)? {
            selected.push(name.to_string());
        } else {
            selected.push(format!("NULL AS {name}"));
        }
    }
    // `created_at` is required, but it sits at index 7 — after the three optional token
    // buckets — so it is spliced into place rather than appended.
    selected.insert(7, "created_at".into());

    let columns = selected.join(", ");
    let (sql, bind): (String, Vec<i64>) = match cursor.high_water() {
        Some(since) => (
            format!("SELECT {columns} FROM {USAGE_TABLE} WHERE created_at >= ?1"),
            vec![since],
        ),
        None => (format!("SELECT {columns} FROM {USAGE_TABLE}"), Vec::new()),
    };

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(bind))?;
    let mut usages = Vec::new();
    let mut skipped = 0usize;
    // Driven by hand rather than through `query_map`: a step error means the store cannot be
    // read past this point and is the whole read's error, while a row that steps but does not
    // map is one skipped row.
    while let Some(row) = rows.next()? {
        // Advance on every row, including ones dropped below: a row with no tokens still marks
        // history we have seen, and not advancing past it would re-read it forever.
        if let Some(created) = created_at_seconds(row, 7) {
            cursor.advance(created);
        }
        match usage_from_event_row(row, decision) {
            Some(usage) => usages.push(usage),
            None => skipped += 1,
        }
    }
    Ok((usages, skipped))
}

/// Every legacy session log under `session-state/`.
fn legacy_logs(root: &Path) -> Vec<PathBuf> {
    let state = root.join("session-state");
    let Ok(entries) = fs::read_dir(&state) else {
        return Vec::new();
    };
    let mut logs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path().join("events.jsonl"))
        .filter(|path| path.is_file())
        .collect();
    logs.sort();
    logs
}

/// The cumulative totals a `session.shutdown` line reports, by model.
fn shutdown_totals(line: &str) -> Option<Vec<(String, Totals)>> {
    let json: Value = serde_json::from_str(line).ok()?;
    if string(&json, &["type", "event"]).as_deref() != Some("session.shutdown") {
        return None;
    }
    let metrics = json.get("data")?.get("modelMetrics")?.as_object()?;
    let mut out = Vec::new();
    for (model, entry) in metrics {
        let usage = entry.get("usage").unwrap_or(&Value::Null);
        out.push((
            model.clone(),
            Totals {
                requests: entry
                    .get("requests")
                    .map(|r| number(r, &["count"]))
                    .unwrap_or(0),
                input: number(usage, &["inputTokens", "input_tokens"]),
                output: number(usage, &["outputTokens", "output_tokens"]),
                reasoning: number(usage, &["reasoningTokens", "reasoning_tokens"]),
                cache_read: number(usage, &["cacheReadTokens", "cache_read_tokens"]),
                cache_write: number(usage, &["cacheWriteTokens", "cache_write_tokens"]),
            },
        ));
    }
    Some(out)
}

/// Tail the legacy logs, emitting the delta each new shutdown snapshot represents.
fn load_legacy(root: &Path, cursor: &mut Cursor, decision: &Decision) -> Result<Vec<Usage>> {
    let mut usages = Vec::new();
    for path in legacy_logs(root) {
        // One unreadable or half-written log must not sink the whole source.
        let Ok(file) = fs::File::open(&path) else {
            continue;
        };
        let Ok(size) = file.metadata().map(|meta| meta.len()) else {
            continue;
        };
        let offset = cursor.legacy_offsets.entry(path.clone()).or_insert(0);
        if *offset > size {
            *offset = 0;
        }
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(*offset)).is_err() {
            continue;
        }
        let session = path
            .parent()
            .and_then(|dir| dir.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut line = String::new();
        loop {
            line.clear();
            let Ok(bytes) = reader.read_line(&mut line) else {
                break;
            };
            if bytes == 0 {
                break;
            }
            // A partial trailing line is a write in flight; re-read it next poll.
            if !line.ends_with('\n') {
                break;
            }
            *offset += bytes as u64;
            let Some(totals) = shutdown_totals(&line) else {
                continue;
            };
            for (model, cumulative) in totals {
                let key = (session.clone(), model.clone());
                let previous = cursor.legacy_totals.get(&key).copied().unwrap_or_default();
                let delta = cumulative.since(previous);
                cursor.legacy_totals.insert(key, cumulative);
                if delta.tokens() == 0 {
                    continue;
                }
                // Shutdown metrics use the same inclusive convention as the request table:
                // `inputTokens` contains the cache buckets and `outputTokens` contains
                // reasoning. Subtract them back out so the five buckets stay disjoint, exactly
                // as `usage_from_event_row` does -- a delta of inclusive totals is still
                // inclusive.
                let input = delta
                    .input
                    .saturating_sub(delta.cache_read)
                    .saturating_sub(delta.cache_write);
                let output = delta.output.saturating_sub(delta.reasoning);
                usages.push(Usage {
                    // Content-derived, so a log re-read from the start dedups against what is
                    // already merged instead of doubling it.
                    event_id: Some(format!(
                        "copilot:shutdown:{session}:{model}:{}",
                        cumulative.tokens()
                    )),
                    category: classify(PROVIDER, &model),
                    provider: PROVIDER.to_string(),
                    model,
                    requests: delta.requests.max(1),
                    input,
                    output,
                    reasoning: delta.reasoning,
                    cache_read: delta.cache_read,
                    cache_write: delta.cache_write,
                    cost: None,
                    cost_status: CostStatus::Unavailable,
                    billing: decision.billing,
                    api_equivalent_cost: None,
                    created: shutdown_time(&line).unwrap_or(0),
                    session_id: Some(session.clone()),
                    project: None,
                });
            }
        }
    }
    Ok(usages)
}

fn shutdown_time(line: &str) -> Option<i64> {
    let json: Value = serde_json::from_str(line).ok()?;
    if let Some(raw) = string(&json, &["timestamp", "time"]) {
        if let Some(seconds) = parse_created_at(&raw) {
            return Some(seconds);
        }
    }
    let raw = number(&json, &["timestamp", "timestampMs", "time"]);
    (raw > 0).then(|| timestamp_seconds(raw as i64))
}

/// Read Copilot usage, preferring the request table and falling back to the legacy log.
pub fn load_copilot_since(
    root: Option<&Path>,
    cursor: &mut Cursor,
    decision: &Decision,
) -> Result<(Vec<Usage>, String)> {
    let Some(root) = root.map(Path::to_path_buf).or_else(copilot_home) else {
        return Ok((
            Vec::new(),
            "could not determine a home directory; set COPILOT_HOME or pass --copilot-dir".into(),
        ));
    };
    if !root.exists() {
        return Ok((
            Vec::new(),
            format!("No Copilot directory at {}", root.display()),
        ));
    }
    if let Some(db) = find_usage_db(&root) {
        let (usages, skipped) = load_events(&db, cursor, decision)?;
        let mut status = format!("GitHub Copilot: {}", db.display());
        if skipped > 0 {
            status.push_str(&format!(" ({skipped} row(s) with no tokens skipped)"));
        }
        return Ok((usages, status));
    }
    let logs = legacy_logs(&root);
    if logs.is_empty() {
        return Ok((
            Vec::new(),
            format!(
                "No {USAGE_TABLE} table and no session logs under {}",
                root.display()
            ),
        ));
    }
    let usages = load_legacy(&root, cursor, decision)?;
    Ok((
        usages,
        format!(
            "GitHub Copilot: {} legacy session log(s) under {}",
            logs.len(),
            root.join("session-state").display()
        ),
    ))
}

pub struct CopilotCollector {
    pub root: Option<PathBuf>,
    pub interval_secs: u64,
    pub cursor: Cursor,
    pub decision: Decision,
}

impl Collector for CopilotCollector {
    fn name(&self) -> &str {
        ID
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        let (usages, _) =
            load_copilot_since(self.root.as_deref(), &mut self.cursor, &self.decision)?;
        Ok(usages)
    }
}

/// One-shot read for the source registry.
pub(crate) fn read(
    roots: &crate::collector::SourceRoots,
) -> crate::collector::registry::SourceRead {
    let path = roots.copilot_dir.clone().or_else(copilot_home);
    let decision = roots.copilot_decision();
    let mut cursor = Cursor::start();
    let (usages, status) =
        load_copilot_since(roots.copilot_dir.as_deref(), &mut cursor, &decision)?;
    Ok((
        crate::collector::SourceReport {
            id: ID,
            present: path.as_deref().is_some_and(Path::exists),
            path,
            rows: usages.len(),
            status,
            detail: Some(decision.describe("copilot")),
        },
        usages,
    ))
}

/// A background collector for the same source.
pub(crate) fn collector(
    roots: &crate::collector::SourceRoots,
    interval_secs: u64,
) -> Box<dyn Collector> {
    Box::new(CopilotCollector {
        root: roots.copilot_dir.clone(),
        interval_secs,
        cursor: Cursor::start(),
        decision: roots.copilot_decision(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Billing;

    fn decision() -> Decision {
        Decision {
            billing: Billing::Subscription,
            tier: None,
            reason: "test",
        }
    }

    /// One seeded request row.
    struct Row<'a> {
        session: &'a str,
        model: &'a str,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
        created: i64,
        turn: i64,
    }

    /// A Copilot home with a CLI store, seeded with one request row per entry.
    fn seed_store(root: &Path, name: &str, rows: &[Row<'_>]) {
        fs::create_dir_all(root).expect("create root");
        let conn = Connection::open(root.join(name)).expect("create db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS assistant_usage_events (
                 session_id TEXT, model TEXT, input_tokens INTEGER, output_tokens INTEGER,
                 cache_read_tokens INTEGER, cache_write_tokens INTEGER,
                 reasoning_tokens INTEGER, created_at INTEGER, cwd TEXT,
                 repository TEXT, turn_index INTEGER);",
        )
        .expect("create schema");
        for row in rows {
            conn.execute(
                "INSERT INTO assistant_usage_events (session_id, model, input_tokens, \
                 output_tokens, cache_read_tokens, cache_write_tokens, reasoning_tokens, \
                 created_at, cwd, repository, turn_index) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '/home/dev/app', 'dev/app', ?9)",
                rusqlite::params![
                    row.session,
                    row.model,
                    row.input,
                    row.output,
                    row.cache_read,
                    row.cache_write,
                    row.reasoning,
                    row.created,
                    row.turn
                ],
            )
            .expect("insert row");
        }
    }

    #[test]
    fn cache_and_reasoning_come_out_of_input_and_output() {
        // `input_tokens` is inclusive of the cache buckets. Leaving them folded in would count
        // every cached token twice in `total_tokens`, and price it twice.
        let dir = tempfile::tempdir().unwrap();
        seed_store(
            dir.path(),
            "session-store.db",
            &[Row {
                session: "s1",
                model: "claude-sonnet-5",
                input: 1000,
                output: 300,
                cache_read: 600,
                cache_write: 100,
                reasoning: 50,
                created: 1_700_000_000,
                turn: 0,
            }],
        );
        let mut cursor = Cursor::start();
        let (usages, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");

        assert_eq!(usages.len(), 1);
        let usage = &usages[0];
        assert_eq!(usage.input, 300, "cache read and write must leave input");
        assert_eq!(usage.output, 250, "reasoning must leave output");
        assert_eq!(usage.cache_read, 600);
        assert_eq!(usage.cache_write, 100);
        assert_eq!(usage.reasoning, 50);
        assert_eq!(
            usage.total_tokens(),
            1300,
            "the split must not change what was reported in total"
        );
    }

    #[test]
    fn rows_carry_session_project_and_a_stable_event_id() {
        let dir = tempfile::tempdir().unwrap();
        seed_store(
            dir.path(),
            "session-store.db",
            &[Row {
                session: "s1",
                model: "gpt-5.6",
                input: 100,
                output: 20,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
                created: 1_700_000_000,
                turn: 3,
            }],
        );
        let mut cursor = Cursor::start();
        let (usages, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert_eq!(usages[0].event_id.as_deref(), Some("copilot:s1:3"));
        assert_eq!(usages[0].session_id.as_deref(), Some("s1"));
        assert_eq!(usages[0].project.as_deref(), Some("/home/dev/app"));
        assert_eq!(usages[0].provider, PROVIDER);
    }

    #[test]
    fn copilot_rows_are_subscription_billed_and_never_carry_a_dollar_figure() {
        // A seat bills premium requests, not tokens. A cost here would be money that was never
        // charged, and it would reach budgets.
        let dir = tempfile::tempdir().unwrap();
        seed_store(
            dir.path(),
            "session-store.db",
            &[Row {
                session: "s1",
                model: "claude-sonnet-5",
                input: 100,
                output: 20,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
                created: 1_700_000_000,
                turn: 0,
            }],
        );
        let mut cursor = Cursor::start();
        let (mut usages, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");

        assert_eq!(usages[0].billing, Billing::Subscription);
        assert_eq!(usages[0].cost, None);

        // And it stays that way through pricing, which parks the list rate as a counterfactual.
        crate::pricing::apply_estimated_pricing(
            &mut usages,
            &crate::pricing::PricingEngine::bundled(),
        );
        assert_eq!(usages[0].cost_status, CostStatus::Quota);
        assert_eq!(usages[0].cost, None, "a quota row must not gain a price");
    }

    #[test]
    fn a_cursor_read_returns_only_new_rows_and_the_boundary_dedups() {
        use crate::collector::usage_key;
        use std::collections::HashSet;

        let dir = tempfile::tempdir().unwrap();
        seed_store(
            dir.path(),
            "session-store.db",
            &[
                Row {
                    session: "s1",
                    model: "gpt-5.6",
                    input: 100,
                    output: 20,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                    created: 1_700_000_000,
                    turn: 0,
                },
                Row {
                    session: "s1",
                    model: "gpt-5.6",
                    input: 200,
                    output: 40,
                    cache_read: 0,
                    cache_write: 0,
                    reasoning: 0,
                    created: 1_700_000_060,
                    turn: 1,
                },
            ],
        );
        let mut cursor = Cursor::start();
        let (first, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert_eq!(first.len(), 2);
        assert_eq!(cursor.high_water(), Some(1_700_000_060));

        let (second, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert_eq!(
            second.len(),
            1,
            "second poll re-read more than the boundary"
        );

        let mut seen = HashSet::new();
        let unique = first
            .iter()
            .chain(second.iter())
            .filter(|u| seen.insert(usage_key(u)))
            .count();
        assert_eq!(unique, 2, "boundary overlap was double-counted");
    }

    #[test]
    fn a_row_with_no_tokens_still_advances_the_cursor() {
        // Not advancing past a zero row re-reads it on every poll, forever.
        let dir = tempfile::tempdir().unwrap();
        seed_store(
            dir.path(),
            "session-store.db",
            &[Row {
                session: "s1",
                model: "gpt-5.6",
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                reasoning: 0,
                created: 1_700_000_500,
                turn: 0,
            }],
        );
        let mut cursor = Cursor::start();
        let (usages, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert!(
            usages.is_empty(),
            "a row with no tokens is not a billed call"
        );
        assert_eq!(cursor.high_water(), Some(1_700_000_500));
    }

    #[test]
    fn an_older_store_missing_the_newer_columns_still_reads() {
        // Copilot has added token buckets over time and this path cannot migrate anything.
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path()).unwrap();
        let conn = Connection::open(dir.path().join("session.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE assistant_usage_events (session_id TEXT, model TEXT, \
             input_tokens INTEGER, output_tokens INTEGER, created_at INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assistant_usage_events VALUES ('s1', 'gpt-5.6', 100, 20, 1700000000)",
            [],
        )
        .unwrap();

        let mut cursor = Cursor::start();
        let (usages, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].input, 100);
        assert_eq!(usages[0].cache_read, 0);
    }

    #[test]
    fn an_absent_home_is_a_status_not_an_error() {
        let mut cursor = Cursor::start();
        let (usages, status) = load_copilot_since(
            Some(Path::new("/nonexistent/copilot-home")),
            &mut cursor,
            &decision(),
        )
        .expect("a missing source must not fail the read");
        assert!(usages.is_empty());
        assert!(status.contains("No Copilot directory"), "{status}");
    }

    #[test]
    fn a_store_without_the_usage_table_is_reported_not_guessed_at() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path()).unwrap();
        let conn = Connection::open(dir.path().join("data.db")).unwrap();
        conn.execute_batch("CREATE TABLE turns (session_id TEXT, user_message TEXT);")
            .unwrap();

        let mut cursor = Cursor::start();
        let (usages, status) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert!(usages.is_empty());
        assert!(status.contains("No assistant_usage_events"), "{status}");
    }

    fn write_legacy(root: &Path, session: &str, lines: &[String]) {
        let dir = root.join("session-state").join(session);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        let mut body = fs::read_to_string(&path).unwrap_or_default();
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }
        fs::write(path, body).unwrap();
    }

    fn shutdown_line(model: &str, input: u64, output: u64, requests: u64) -> String {
        format!(
            r#"{{"type":"session.shutdown","timestamp":"2026-08-18T10:00:00Z","data":{{"modelMetrics":{{"{model}":{{"requests":{{"count":{requests}}},"usage":{{"inputTokens":{input},"outputTokens":{output},"cacheReadTokens":0,"cacheWriteTokens":0,"reasoningTokens":0}}}}}}}}}}"#
        )
    }

    #[test]
    fn the_legacy_log_is_read_only_when_there_is_no_store() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy(
            dir.path(),
            "abc",
            &[shutdown_line("claude-sonnet-5", 1000, 200, 4)],
        );
        let mut cursor = Cursor::start();
        let (usages, status) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].input, 1000);
        assert_eq!(usages[0].output, 200);
        assert_eq!(usages[0].requests, 4);
        assert_eq!(usages[0].session_id.as_deref(), Some("abc"));
        assert!(status.contains("legacy session log"), "{status}");
    }

    #[test]
    fn legacy_aggregates_split_cache_out_of_input_too() {
        // The request table's convention is the shutdown aggregate's convention. Both unit
        // tests above used `cacheReadTokens: 0`, so the legacy path double-counted every
        // cached token until an end-to-end run on a fixture with cache showed it.
        let dir = tempfile::tempdir().unwrap();
        let line = r#"{"type":"session.shutdown","timestamp":"2026-08-18T10:00:00Z","data":{"modelMetrics":{"claude-sonnet-5":{"requests":{"count":7},"usage":{"inputTokens":31000,"outputTokens":4200,"cacheReadTokens":12000,"cacheWriteTokens":1000,"reasoningTokens":200}}}}}"#
            .to_string();
        write_legacy(dir.path(), "abc", &[line]);
        let mut cursor = Cursor::start();
        let (usages, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");

        let usage = &usages[0];
        assert_eq!(usage.input, 18000, "cache buckets must leave input");
        assert_eq!(usage.output, 4000, "reasoning must leave output");
        assert_eq!(usage.cache_read, 12000);
        assert_eq!(usage.cache_write, 1000);
        assert_eq!(
            usage.total_tokens(),
            35200,
            "the split must not change what Copilot reported in total"
        );
    }

    #[test]
    fn a_session_that_never_shut_down_contributes_nothing() {
        // The per-message path is where other tools get their numbers, by dividing a character
        // count by four. A session with no shutdown aggregate is unmeasured, and says so.
        let dir = tempfile::tempdir().unwrap();
        write_legacy(
            dir.path(),
            "live",
            &[
                r#"{"type":"session.start","data":{"context":{"cwd":"/home/dev/app"}}}"#.to_string(),
                r#"{"type":"assistant.message","data":{"outputTokens":900,"content":"aaaaaaaaaaaaaaaaaaaa"}}"#.to_string(),
            ],
        );
        let mut cursor = Cursor::start();
        let (usages, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert!(
            usages.is_empty(),
            "an unmeasured session must not be reconstructed"
        );
    }

    #[test]
    fn a_resumed_sessions_second_snapshot_is_a_delta_not_a_repeat() {
        // Shutdown metrics are cumulative. Emitting the second snapshot whole would report the
        // first session's tokens twice.
        let dir = tempfile::tempdir().unwrap();
        write_legacy(
            dir.path(),
            "abc",
            &[shutdown_line("claude-sonnet-5", 1000, 200, 4)],
        );
        let mut cursor = Cursor::start();
        let (first, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert_eq!(first[0].input, 1000);

        write_legacy(
            dir.path(),
            "abc",
            &[shutdown_line("claude-sonnet-5", 2500, 450, 9)],
        );
        let (second, _) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].input, 1500, "expected the delta, not the total");
        assert_eq!(second[0].output, 250);
        assert_eq!(second[0].requests, 5);

        let total: u64 = first.iter().chain(second.iter()).map(|u| u.input).sum();
        assert_eq!(
            total, 2500,
            "the two reads must sum to what Copilot reported"
        );
    }

    #[test]
    fn no_message_content_reaches_a_usage_row() {
        // Both stores hold prompts and completions. A secret in one must not survive the read.
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path()).unwrap();
        let conn = Connection::open(dir.path().join("session-store.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE assistant_usage_events (session_id TEXT, model TEXT, \
             input_tokens INTEGER, output_tokens INTEGER, created_at INTEGER);
             CREATE TABLE turns (session_id TEXT, user_message TEXT, assistant_response TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO assistant_usage_events VALUES ('s1', 'gpt-5.6', 100, 20, 1700000000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turns VALUES ('s1', 'export AWS_SECRET_ACCESS_KEY=hunter2', 'ok')",
            [],
        )
        .unwrap();

        let mut cursor = Cursor::start();
        let (usages, status) =
            load_copilot_since(Some(dir.path()), &mut cursor, &decision()).expect("load");
        let rendered = format!("{usages:?}{status}");
        assert!(!rendered.contains("hunter2"), "message content leaked");
        assert!(!rendered.contains("AWS_SECRET_ACCESS_KEY"));
    }
}
