use std::{fs, io, io::Read, path::Path, time::Duration};

use anyhow::Result;
use rusqlite::{params, Connection, OpenFlags};
use serde_json::Value;

use crate::classify::{category_from_label, classify, cost_status_from_label};
use crate::collector::background::Collector;
use crate::collector::opencode::parse_created_at;
use crate::helpers::{number, string};
use crate::model::{Billing, Category, CostStatus, RoutingEvent, Usage};
use crate::utils::now;
use std::path::PathBuf;

/// This source's canonical id: the `Collector::name()` it reports, the
/// `[collectors.<id>]` table that configures it, and its key in the source registry.
/// One constant so those can never drift apart.
pub const ID: &str = "journal";

/// The journal's usage rows. A row that cannot be read is skipped and counted, never silently
/// dropped: the count reaches the source's status line — the `--once` header — and `--doctor`
/// through the source report, and the log through the live collector. Not an error,
/// deliberately: `load_usage` fails as a whole on any source error, and one corrupt Ollama row
/// must not take every command down.
pub fn load_journal(path: &Path) -> Result<Vec<Usage>> {
    let (usages, skipped) = load_journal_counting(path)?;
    if let Some(skipped) = skipped {
        crate::logging::error("journal", &skipped.to_string());
    }
    Ok(usages)
}

/// Rows a read had to skip: how many, and the first reason, for whoever surfaces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkippedRows {
    pub count: usize,
    pub first: String,
}

impl std::fmt::Display for SkippedRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} usage_event row(s) could not be read and were skipped; first: {}",
            self.count, self.first
        )
    }
}

/// `load_journal`, with what was skipped for the caller to surface. The whole table: a one-shot
/// read is a cursor that has seen nothing.
pub(crate) fn load_journal_counting(path: &Path) -> Result<(Vec<Usage>, Option<SkippedRows>)> {
    load_journal_since(path, &mut JournalCursor::default())
}

/// How far a live collector has read, so a poll reads what was recorded since the last one.
///
/// Until this existed the collector ran `SELECT ... FROM usage_event` with no `WHERE` every
/// sixty seconds and left `merge`'s dedup to throw all of it away again -- the defect the
/// roadmap filed against Gemini's `read_to_string`, in SQL.
///
/// `id` is the high-water mark: `INTEGER PRIMARY KEY`, and a new row takes `MAX(id) + 1`. That
/// is only monotonic while the highest row is never deleted -- delete it and its id is handed
/// out again, *below* this cursor, to a row no poll will ever see. `prune` keeps the max-id row
/// for exactly this reason, and says so. `VACUUM` keeps the ids of a table with an explicit
/// `INTEGER PRIMARY KEY`, and rewrites the file in place, so it disturbs neither half.
#[derive(Debug, Default)]
pub(crate) struct JournalCursor {
    /// Every row with an `id` at or below this has been read, mapped or counted as skipped.
    after_id: i64,
    /// Which file that was true of: device and inode. `None` where there is no such thing, so a
    /// journal *replaced* by a bigger one goes unnoticed there until restart; a smaller one is
    /// caught by `MAX(id)` everywhere.
    identity: Option<(u64, u64)>,
    /// Skipped rows so far. Kept here because a poll no longer re-reads them: a count local to
    /// one read would put the warning on the status line for one poll and then take it away
    /// while the row was as unreadable as before.
    skipped: usize,
    first_error: Option<String>,
}

impl JournalCursor {
    fn skipped_rows(&self) -> Option<SkippedRows> {
        (self.skipped > 0).then(|| SkippedRows {
            count: self.skipped,
            first: self.first_error.clone().unwrap_or_default(),
        })
    }
}

fn file_identity(path: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let meta = std::fs::metadata(path).ok()?;
        Some((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// The rows recorded since `cursor`, and everything skipped since it was last reset.
pub(crate) fn load_journal_since(
    path: &Path,
    cursor: &mut JournalCursor,
) -> Result<(Vec<Usage>, Option<SkippedRows>)> {
    if !path.exists() {
        *cursor = JournalCursor::default();
        return Ok((Vec::new(), None));
    }
    let identity = file_identity(path);
    if cursor.identity != identity {
        *cursor = JournalCursor {
            identity,
            ..Default::default()
        };
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(Duration::from_millis(250))?;
    let has_events: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'usage_event')",
        [],
        |row| row.get(0),
    )?;
    if !has_events {
        *cursor = JournalCursor {
            identity,
            ..Default::default()
        };
        return Ok((Vec::new(), None));
    }
    // A table whose highest id is below the cursor is not the table the cursor read: the file
    // was recreated in place, or emptied. One b-tree lookup, not a scan.
    let max_id: i64 =
        conn.query_row("SELECT COALESCE(MAX(id), 0) FROM usage_event", [], |row| {
            row.get(0)
        })?;
    if max_id < cursor.after_id {
        *cursor = JournalCursor {
            identity,
            ..Default::default()
        };
    }
    // A journal written by an older build lacks the columns added since, and this is a read-only
    // path that cannot migrate it. Each is selected only when it actually exists -- probed one by
    // one, because they arrived in different releases and a journal can have any prefix of them.
    // The new ones go *after* `id`: `row_name` below reads `id` by position.
    let has_column = |name: &str| -> rusqlite::Result<bool> {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('usage_event') WHERE name = ?1)",
            [name],
            |row| row.get(0),
        )
    };
    let optional = |name: &str| -> rusqlite::Result<String> {
        Ok(if has_column(name)? {
            name.to_string()
        } else {
            format!("NULL AS {name}")
        })
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT provider, model, category, cost_status, requests, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens, cost, created, {}, id, {}, {}, {} FROM usage_event WHERE id > ?1 ORDER BY id",
        optional("event_id")?,
        optional("session_id")?,
        optional("project")?,
        optional("billing")?,
    ))?;
    let mut rows = stmt.query([cursor.after_id])?;
    let mut usages = Vec::new();
    let mut skipped = 0usize;
    let mut first_error = None;
    let mut position = 0usize;
    let mut highest = cursor.after_id;
    // Driven by hand rather than through `query_map`'s iterator, so the two kinds of failure
    // are told apart: a step error (`next()?`) means the file cannot be read past this point
    // and is the whole read's error, while a row that steps but does not map is one row.
    // Through the iterator, a step error ended the scan after one counted row and everything
    // after it vanished uncounted.
    while let Some(row) = rows.next()? {
        position += 1;
        // Read past it whether or not it maps: a row that cannot be read today cannot be read
        // on the next poll either, and it has been counted.
        if let Ok(id) = row.get::<_, i64>(13) {
            highest = highest.max(id);
        }
        match usage_from_row(row) {
            Ok(usage) => usages.push(usage),
            Err(error) => {
                skipped += 1;
                first_error
                    .get_or_insert_with(|| format!("{}: {error}", row_name(row, 13, position)));
            }
        }
    }
    // Only now, with the scan complete: a step error above returned early and left the cursor
    // where it was, so the rows it did not reach are read again rather than lost.
    cursor.after_id = highest;
    cursor.skipped += skipped;
    if cursor.first_error.is_none() {
        cursor.first_error = first_error;
    }
    Ok((usages, cursor.skipped_rows()))
}

/// One `usage_event` row, or why it could not be read.
fn usage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Usage> {
    let category: String = row.get(2)?;
    let cost_status: String = row.get(3)?;
    let cost: Option<f64> = row.get(10)?;
    let billing: Option<String> = row.get(16)?;
    let subscription = billing.as_deref() == Some(Billing::Subscription.label());
    // A plan-billed event is *stored* as `quota`, so a build that predates the `billing` column
    // reads it as what it is and neither prices nor budgets it. This build knows more: handed
    // back as an unpriced subscription row -- exactly what the native collectors emit -- it takes
    // the one path that derives `quota` and the list-rate `api_equivalent_cost` beside it.
    let cost_status = match cost_status_from_label(&cost_status) {
        CostStatus::Quota if subscription && cost.is_none() => CostStatus::Unavailable,
        status => status,
    };
    {
        Ok(Usage {
            event_id: row.get(12).ok().flatten(),
            provider: row.get(0)?,
            model: row.get(1)?,
            category: category_from_label(&category),
            cost_status,
            billing: if subscription {
                Billing::Subscription
            } else {
                Billing::default()
            },
            api_equivalent_cost: None,
            // SQLite integers are signed 64-bit; rusqlite 0.40 removed the `u64` impls
            // rather than keep silently reinterpreting the top bit. Read as `i64` and clamp
            // — a negative token count is corruption, and zero is the honest reading of it.
            requests: count(row.get(4)?),
            input: count(row.get(5)?),
            output: count(row.get(6)?),
            reasoning: count(row.get(7)?),
            cache_read: count(row.get(8)?),
            cache_write: count(row.get(9)?),
            cost,
            created: row.get(11)?,
            // Only `--record-event` writes these two; every other recorder is handed a bare
            // response, which says nothing about where or in what session it was made.
            session_id: row.get(14)?,
            project: row.get(15)?,
            // The recorders refuse a response with no usage, so a journaled row is whole.
            incomplete: false,
        })
    }
}

/// How a row is named in an error: by its `id`, which is what a user would look for in the
/// file, or — if even that cannot be read — by its position in the scan, in a different word
/// so the two are never confused.
fn row_name(row: &rusqlite::Row<'_>, id_column: usize, position: usize) -> String {
    match row.get::<_, i64>(id_column) {
        Ok(id) => format!("id {id}"),
        Err(_) => format!("position {position}"),
    }
}

/// A token or request count read back from SQLite.
///
/// SQLite has no unsigned integer type, so every counter round-trips through `i64`. A
/// negative value means the row is corrupt; reporting it as a huge positive number — which is
/// what an `as u64` cast would do — would put a fabricated figure in a cost total.
fn count(value: i64) -> u64 {
    value.max(0) as u64
}

/// A counter on its way into SQLite, saturated at the largest value the column can hold.
fn stored(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The JSON values on stdin, and how many lines could not be parsed.
///
/// A recorder is fed either one JSON document or a stream of them, one per line. Server-sent
/// events are the same stream with `data: ` in front and a `[DONE]` sentinel at the end, so both
/// are stripped here rather than by every caller: it makes `curl … | ai-usage-tui --record-usage`
/// work against a streaming endpoint without a `jq` in the middle.
fn read_json_events(input: &str) -> Result<(Vec<Value>, usize)> {
    let mut events = Vec::new();
    let mut invalid_lines = 0;
    for line in input.lines() {
        let line = line.trim();
        let line = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
        if line.is_empty() || line == "[DONE]" {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<Value>(line) {
            events.push(json);
        } else {
            invalid_lines += 1;
        }
    }
    if events.is_empty() {
        // Not a stream, then: a pretty-printed document spread over many lines, where every one
        // of those lines is "invalid" on its own and the count would be noise.
        events.push(serde_json::from_str(input)?);
        invalid_lines = 0;
    }
    Ok((events, invalid_lines))
}

/// The time of recording, for an event that carries no time of its own -- and a note saying so.
///
/// A recorder is piped a response as it completes, so "now" is a fair reading of when it
/// happened, and it is what these three paths have always stamped. But it was stamped in silence,
/// and it is not the same fact: a file of old responses replayed through a recorder lands, all of
/// it, on today. Said once per process on stderr, which for a recorder is the caller's terminal
/// or its hook log, never the dashboard.
fn recorded_now() -> i64 {
    static NOTED: std::sync::Once = std::sync::Once::new();
    NOTED.call_once(|| {
        eprintln!(
            "note: no timestamp in the input; stamped with the time of recording. \
             Replaying old responses this way dates them today."
        );
    });
    now()
}

/// The journal schema this build writes, stored in SQLite's `PRAGMA user_version`.
///
/// Version 1 is the shape both tables have today. The two tables are created lazily by different
/// commands, so each writer still probes and migrates its own table; the version exists to guard
/// the other direction. A journal a *newer* build has written may carry a shape this one would
/// damage by writing into it, and a hook installed from one channel with the dashboard from another
/// is exactly how two builds end up sharing one file -- so a writer refuses a version above this
/// one, by name, instead of guessing. Raise it with any change an older writer must not touch.
pub const JOURNAL_SCHEMA_VERSION: i64 = 1;

/// How long a writer waits for another writer's lock.
///
/// Was 250ms. Writers here are short-lived processes -- parallel subagents fire parallel hooks, a
/// streamed chat pipes into `--record-usage` -- and losing the lock is a lost row and a failing
/// hook, where waiting is a few milliseconds nobody sees.
const WRITER_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The journal's schema version, or the refusal to touch one stamped by a newer build. Every
/// path that changes the file asks this under its write lock: the writers, and `prune`.
fn refuse_newer(conn: &Connection, path: &Path) -> Result<i64> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    anyhow::ensure!(
        version <= JOURNAL_SCHEMA_VERSION,
        "{} was written by a newer ai-usage-tui (journal schema {version}; this build writes \
         {JOURNAL_SCHEMA_VERSION}). Upgrade this copy -- `ai-usage-tui --doctor` shows where it \
         came from -- rather than writing into a journal it may not understand.",
        path.display()
    );
    Ok(version)
}

/// Open the journal for writing, running `migrate` for the caller's table under the write lock.
///
/// `BEGIN IMMEDIATE` takes that lock *before* anything is probed. The migrations were
/// probe-then-`ALTER` with nothing held between the two, so writers that opened an unmigrated
/// journal together all saw the column missing and all but the first failed on "duplicate column
/// name" -- reproduced by `writers_opening_an_unmigrated_journal_together_all_succeed`. Under the
/// lock the second writer waits, then probes a table the first has already migrated.
fn open_for_writing(
    path: &Path,
    migrate: impl FnOnce(&Connection) -> Result<()>,
) -> Result<Connection> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("journal path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let conn = Connection::open(path)?;
    conn.busy_timeout(WRITER_BUSY_TIMEOUT)?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let migrated = (|| -> Result<()> {
        let version = refuse_newer(&conn, path)?;
        migrate(&conn)?;
        if version < JOURNAL_SCHEMA_VERSION {
            conn.execute_batch(&format!("PRAGMA user_version = {JOURNAL_SCHEMA_VERSION}"))?;
        }
        Ok(())
    })();
    match migrated {
        Ok(()) => conn.execute_batch("COMMIT")?,
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(error);
        }
    }
    Ok(conn)
}

/// The journal, opened for writing usage and migrated if it predates a column.
///
/// This is the only code that creates `usage_event`; the collector opens the journal read-only and
/// so cannot migrate it.
fn journal_connection(path: &Path) -> Result<Connection> {
    open_for_writing(path, |conn| {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage_event (
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
                created INTEGER NOT NULL,
                session_id TEXT,
                project TEXT,
                billing TEXT
            );",
        )?;
        let has_event_id: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('usage_event') WHERE name = 'event_id')",
            [],
            |row| row.get(0),
        )?;
        if !has_event_id {
            conn.execute("ALTER TABLE usage_event ADD COLUMN event_id TEXT", [])?;
        }
        // What `--record-event` brought. Nullable and added at the end, so this is a change an
        // older build can live with on both sides -- its INSERT and its SELECT name their columns
        // -- which is why `JOURNAL_SCHEMA_VERSION` does not move. Probed one at a time: a journal
        // can hold any prefix of them.
        for column in ["session_id", "project", "billing"] {
            let present: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('usage_event') WHERE name = ?1)",
                [column],
                |row| row.get(0),
            )?;
            if !present {
                conn.execute(
                    &format!("ALTER TABLE usage_event ADD COLUMN {column} TEXT"),
                    [],
                )?;
            }
        }
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS usage_event_event_id ON usage_event(event_id)",
            [],
        )?;
        Ok(())
    })
}

/// One completed response, on its way into the journal.
///
/// The recorders that are handed a bare response fill the first seven fields and leave the rest
/// at their defaults; `--record-event` is the only one with anything to put there.
#[derive(Default)]
struct JournalEvent<'a> {
    event_id: String,
    provider: &'a str,
    model: String,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
    /// A figure the source itself recorded. Never computed here.
    cost: Option<f64>,
    /// How the source says it is billed, when it says. `None` leaves the question to `classify`.
    billing: Option<Billing>,
    session_id: Option<String>,
    project: Option<String>,
}

/// How a local or quota-billed response is costed.
///
/// Local work is a genuine zero and says so; Ollama Cloud is billed against a quota rather than
/// per token, so its cost is unknown-but-not-zero. Anything else is left for the pricing engine.
fn cost_status_for(category: Category) -> CostStatus {
    match category {
        Category::Local => CostStatus::Local,
        // Billed on quota, not per token. This arm was already singled out and then
        // collapsed into the fallback, which is how the distinction got lost.
        Category::Cloud => CostStatus::Quota,
        _ => CostStatus::Unavailable,
    }
}

/// Insert one event, or do nothing if the journal already holds it.
///
/// `cost` is NULL unless the source recorded one: a recorder handed a bare response has no price
/// to record, and a zero here would be a fabricated one. `Category::Local` carries the zero
/// instead, as a status. A recorded cost is `reported`; a plan-billed event is `quota`.
fn insert_event(conn: &Connection, event: &JournalEvent<'_>, created: i64) -> Result<usize> {
    let category = classify(event.provider, &event.model);
    let cost_status = match (event.cost, event.billing) {
        (Some(_), _) => CostStatus::ProviderReported,
        (None, Some(Billing::Subscription)) => CostStatus::Quota,
        (None, _) => cost_status_for(category),
    };
    Ok(conn.execute(
        "INSERT OR IGNORE INTO usage_event (event_id, provider, model, category, cost_status, requests, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens, cost, created, session_id, project, billing) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            event.event_id,
            event.provider,
            event.model,
            category.label(),
            cost_status.label(),
            stored(event.input),
            stored(event.output),
            stored(event.reasoning),
            stored(event.cache_read),
            stored(event.cache_write),
            event.cost,
            created,
            event.session_id,
            event.project,
            event.billing.map(Billing::label),
        ],
    )?)
}

pub fn record_ollama(path: &Path) -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let (events, invalid_lines) = read_json_events(&input)?;
    let conn = journal_connection(path)?;

    let mut recorded = 0;
    let streaming = events.len() > 1;
    let events = if streaming {
        events
            .into_iter()
            .rev()
            .find(|event| event.get("done").and_then(Value::as_bool) == Some(true))
            .into_iter()
            .collect()
    } else {
        events
    };
    if streaming && events.is_empty() {
        return Err(anyhow::anyhow!(
            "Ollama stream did not contain a completed response"
        ));
    }
    for json in events {
        if !streaming && json.get("done").and_then(Value::as_bool) != Some(true) {
            return Err(anyhow::anyhow!(
                "Ollama response is missing done=true; journal only completed responses"
            ));
        }
        if json.get("done").and_then(Value::as_bool) == Some(false) {
            if !streaming {
                return Err(anyhow::anyhow!(
                    "Ollama response is not complete; journal only completed responses"
                ));
            }
            continue;
        }
        let model = string(&json, &["model"]).unwrap_or_else(|| "unknown".to_string());
        let created_at = string(&json, &["created_at"]);
        let event_id = format!(
            "ollama:{}:{}:{}:{}:{}",
            model,
            created_at.as_deref().unwrap_or(""),
            number(&json, &["prompt_eval_count"]),
            number(&json, &["eval_count"]),
            number(&json, &["total_duration"]),
        );
        let created = created_at
            .as_deref()
            .and_then(parse_created_at)
            .unwrap_or_else(recorded_now);
        recorded += insert_event(
            &conn,
            &JournalEvent {
                event_id,
                provider: "ollama",
                model,
                input: number(&json, &["prompt_eval_count"]),
                output: number(&json, &["eval_count"]),
                reasoning: 0,
                cache_read: 0,
                ..Default::default()
            },
            created,
        )?;
    }
    if invalid_lines > 0 {
        eprintln!("Skipped {} malformed Ollama JSON line(s)", invalid_lines);
    }
    crate::helpers::print_line(&format!(
        "Recorded {} Ollama usage event(s) in {}",
        recorded,
        path.display()
    ))?;
    Ok(())
}

/// Journal one completed OpenAI-compatible response, read from stdin.
///
/// This is the path for every local server that speaks `/v1/chat/completions` — llama.cpp's
/// `llama-server`, LM Studio, vLLM — none of which Ollama's format covers. `provider` is required
/// rather than guessed: it is what `classify` reads to decide LOCAL against UNKNOWN COST, and a
/// tool that invents that answer is the thing this project exists not to be.
///
/// Only a response that actually carries `usage` is recorded. A streamed response carries it only
/// when the client asked with `stream_options: {"include_usage": true}`, and the honest outcome
/// when it did not is a loud error, not a row of zeros.
pub fn record_usage(path: &Path, provider: &str) -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let (events, invalid_lines) = read_json_events(&input)?;
    let recorded = record_usage_events(path, provider, &events)?;

    if invalid_lines > 0 {
        eprintln!("Skipped {invalid_lines} malformed JSON line(s)");
    }
    crate::helpers::print_line(&format!(
        "Recorded {} {} usage event(s) in {}",
        recorded,
        provider,
        path.display()
    ))?;
    Ok(())
}

/// The same recording, from values already parsed: the half that has no stdin to read and so can
/// be tested directly.
pub(crate) fn record_usage_events(path: &Path, provider: &str, events: &[Value]) -> Result<usize> {
    if provider.trim().is_empty() {
        return Err(anyhow::anyhow!(
            "--record-usage needs a provider to record under, e.g. llamacpp"
        ));
    }
    // The last event that carries usage: a stream's totals arrive in a final chunk, and a
    // non-streamed reply is the only event there is.
    let Some(json) = events
        .iter()
        .rev()
        .find(|event| event.get("usage").is_some_and(|usage| !usage.is_null()))
    else {
        return Err(anyhow::anyhow!(
            "response carried no usage object; journal only completed responses. A streamed \
             response reports usage only when the request set stream_options.include_usage"
        ));
    };
    let usage = &json["usage"];

    let model = string(json, &["model"]).unwrap_or_else(|| "unknown".to_string());
    // OpenAI counts cached tokens inside `prompt_tokens`; the journal keeps the two apart, so
    // the cached share has to come back out or the row bills the same tokens twice. llama.cpp
    // reports the same split a second time as `timings.cache_n`.
    let prompt = number(usage, &["prompt_tokens"]);
    let cache_read = number(&usage["prompt_tokens_details"], &["cached_tokens"]);
    let created = match number(json, &["created"]) {
        0 => recorded_now(),
        seconds => crate::collector::opencode::timestamp_seconds(seconds as i64),
    };
    // `id` is the server's own idempotency key. Without one, the shape of the response has to
    // serve as its identity, exactly as the Ollama path does.
    let event_id = match string(json, &["id"]) {
        Some(id) => format!("{provider}:{model}:{id}"),
        None => format!(
            "{provider}:{model}:{created}:{prompt}:{}",
            number(usage, &["completion_tokens"])
        ),
    };
    let conn = journal_connection(path)?;
    let recorded = insert_event(
        &conn,
        &JournalEvent {
            event_id,
            provider,
            model,
            input: prompt.saturating_sub(cache_read),
            output: number(usage, &["completion_tokens"]),
            reasoning: number(&usage["completion_tokens_details"], &["reasoning_tokens"]),
            cache_read,
            ..Default::default()
        },
        created,
    )?;
    Ok(recorded)
}

/// The keys `--record-event` reads. Anything else in an event is refused, by name.
const EVENT_KEYS: &[&str] = &[
    "provider",
    "model",
    "input_tokens",
    "output_tokens",
    "reasoning_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "created",
    "event_id",
    "session_id",
    "project",
    "cost",
    "cost_status",
    "billing",
];

/// Journal usage events a tool's own adapter has already normalised, read from stdin.
///
/// The other recorders each understand one server's response. This one is for everything else:
/// a tool that logs its own token counts, fed through a few lines of `jq` or a script, arrives
/// here in the terms of `docs/data-model.md` and carries what a bare response cannot -- the
/// project, the session, cache writes, a cost the tool itself recorded, a plan it is billed
/// against.
///
/// It is strict in the way `--record-routing` is, for the same reason: whoever writes the adapter
/// learns from the exit code and nothing else. One event that cannot be read refuses the whole
/// batch, before the journal is opened. And it records **measured counts only** -- an event
/// without `input_tokens` and `output_tokens` is refused, never stored as zero, so a tool that
/// keeps no counts cannot be journaled by estimating them. That is the rule the README's "Why
/// there is no Cursor collector" states; here it is the code's.
pub fn record_event(path: &Path) -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    anyhow::ensure!(
        !input.trim().is_empty(),
        "--record-event read nothing on stdin; it takes one JSON usage event per line"
    );
    let (events, invalid_lines) = read_json_events(&input)?;
    // A recorder of responses skips a line it cannot parse, because only the last chunk of a
    // stream matters. Here every line is usage, and a skipped one is usage silently lost.
    anyhow::ensure!(
        invalid_lines == 0,
        "{invalid_lines} line(s) on stdin are not JSON; nothing was recorded. --record-event \
         takes one usage event per line and refuses the batch rather than dropping part of it"
    );
    // An adapter that collects before it prints sends one array; take it as the batch it is.
    let events: Vec<Value> = events
        .into_iter()
        .flat_map(|event| match event {
            Value::Array(items) => items,
            other => vec![other],
        })
        .collect();
    let (recorded, sent) = record_events(path, &events)?;
    let already = sent - recorded;
    crate::helpers::print_line(&format!(
        "Recorded {recorded} of {sent} usage event(s) in {}{}",
        path.display(),
        if already > 0 {
            format!(" ({already} already journaled)")
        } else {
            String::new()
        }
    ))?;
    Ok(())
}

/// The same recording, from values already parsed. Answers how many were new, and how many sent.
pub(crate) fn record_events(path: &Path, events: &[Value]) -> Result<(usize, usize)> {
    anyhow::ensure!(!events.is_empty(), "--record-event was given no events");
    // Everything is validated before the journal is opened, so a refused batch creates no file
    // and leaves no half of itself behind.
    let parsed = events
        .iter()
        .enumerate()
        .map(|(index, json)| {
            parse_event(json)
                .map_err(|error| anyhow::anyhow!("usage event #{}: {error}", index + 1))
        })
        .collect::<Result<Vec<_>>>()?;

    let conn = journal_connection(path)?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let inserted = (|| -> Result<usize> {
        let mut recorded = 0;
        for event in &parsed {
            recorded += insert_event(&conn, &event.row(), event.created)?;
        }
        Ok(recorded)
    })();
    match inserted {
        Ok(recorded) => {
            conn.execute_batch("COMMIT")?;
            Ok((recorded, parsed.len()))
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// One `--record-event` line, read and found whole.
struct ParsedEvent {
    event_id: String,
    provider: String,
    model: String,
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
    cost: Option<f64>,
    billing: Option<Billing>,
    session_id: Option<String>,
    project: Option<String>,
    created: i64,
}

impl ParsedEvent {
    fn row(&self) -> JournalEvent<'_> {
        JournalEvent {
            event_id: self.event_id.clone(),
            provider: &self.provider,
            model: self.model.clone(),
            input: self.input,
            output: self.output,
            reasoning: self.reasoning,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
            cost: self.cost,
            billing: self.billing,
            session_id: self.session_id.clone(),
            project: self.project.clone(),
        }
    }
}

fn parse_event(json: &Value) -> Result<ParsedEvent> {
    let object = json
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("must be a JSON object, got {json}"))?;
    // A misspelt optional key would otherwise be a count quietly left out of the row.
    if let Some(unknown) = object
        .keys()
        .find(|key| !EVENT_KEYS.contains(&key.as_str()))
    {
        anyhow::bail!(
            "`{unknown}` is not a key this reads; the keys are {}",
            EVENT_KEYS.join(", ")
        );
    }
    let provider = required_text(json, "provider")?;
    let model = required_text(json, "model")?;
    let input = required_count(json, "input_tokens")?;
    let output = required_count(json, "output_tokens")?;
    let reasoning = optional_count(json, "usage event", "reasoning_tokens")?.unwrap_or(0);
    let cache_read = optional_count(json, "usage event", "cache_read_tokens")?.unwrap_or(0);
    let cache_write = optional_count(json, "usage event", "cache_write_tokens")?.unwrap_or(0);
    let session_id = optional_text(json, "session_id")?;
    let project = optional_text(json, "project")?
        .map(|project| crate::collector::claude_code::normalize_project_path(&project));

    let supplied_id = match json.get("event_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) if !id.trim().is_empty() => Some(id.trim().to_string()),
        Some(Value::Number(id)) => Some(id.to_string()),
        Some(other) => anyhow::bail!("`event_id` must be a non-empty string, got {other}"),
    };
    let created = optional_count(json, "usage event", "created")?
        .filter(|seconds| *seconds > 0)
        .map(|seconds| {
            crate::collector::opencode::timestamp_seconds(
                i64::try_from(seconds).unwrap_or(i64::MAX),
            )
        });
    // With neither, the only identity left is the time of recording -- which is different on
    // every run, so replaying the same log would journal it again each time.
    anyhow::ensure!(
        supplied_id.is_some() || created.is_some(),
        "needs `event_id` or `created` (unix seconds): with neither, the same event recorded \
         twice would be counted twice"
    );
    // Identities share one namespace across every source, so an adapter's `1` must not be able
    // to collide with another tool's `1` -- or with a native collector's id.
    let event_id = match &supplied_id {
        Some(id) => format!("event:{provider}:{id}"),
        None => format!(
            "event:{provider}:{model}:{}:{input}:{output}:{}",
            created.unwrap_or_default(),
            session_id.as_deref().unwrap_or_default()
        ),
    };

    let cost = match json.get("cost") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
                .ok_or_else(|| {
                    anyhow::anyhow!("`cost` must be a number of dollars, got {value}")
                })?,
        ),
    };
    let billing = match optional_text(json, "billing")?.as_deref() {
        None => None,
        Some("subscription") => Some(Billing::Subscription),
        Some("per_token") => Some(Billing::PerToken),
        Some(other) => {
            anyhow::bail!("`billing` must be \"subscription\" or \"per_token\", got {other:?}")
        }
    };
    // The status is derived, never taken on trust: `estimated` or `calculated` from an adapter
    // would be this tool vouching for arithmetic it never saw. The one thing an adapter can
    // assert is that the figure is the source's own.
    match optional_text(json, "cost_status")?.as_deref() {
        None => {}
        Some("reported") => anyhow::ensure!(
            cost.is_some(),
            "`cost_status` \"reported\" needs the `cost` that was reported"
        ),
        Some(other) => anyhow::bail!(
            "`cost_status` {other:?} cannot be supplied: the only status an event may assert is \
             \"reported\", with the `cost` the tool itself recorded. Leave it out and the status \
             is derived"
        ),
    }
    anyhow::ensure!(
        !(billing == Some(Billing::Subscription) && cost.is_some()),
        "a \"subscription\" event cannot carry a `cost`: work billed against a plan has no \
         per-request price"
    );

    Ok(ParsedEvent {
        event_id,
        provider,
        model,
        input,
        output,
        reasoning,
        cache_read,
        cache_write,
        cost,
        billing,
        session_id,
        project,
        created: created.unwrap_or_else(recorded_now),
    })
}

/// A string an event must carry. Blank counts as absent: a provider of `""` classifies nothing.
fn required_text(json: &Value, key: &str) -> Result<String> {
    optional_text(json, key)?.ok_or_else(|| anyhow::anyhow!("`{key}` is required"))
}

/// A string an event may carry; absent, `null` and blank are all "not said".
fn optional_text(json: &Value, key: &str) -> Result<Option<String>> {
    match json.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            Ok(Some(text.trim().to_string()).filter(|text| !text.is_empty()))
        }
        Some(other) => Err(anyhow::anyhow!("`{key}` must be a string, got {other}")),
    }
}

/// A token count the source always measures. Absent is refused, not read as `0`: an event with
/// no output count is not an event that produced no output, and priced as one it is a confident,
/// low, wrong number.
fn required_count(json: &Value, key: &str) -> Result<u64> {
    optional_count(json, "usage event", key)?.ok_or_else(|| {
        anyhow::anyhow!(
            "`{key}` is required. Record the count the tool measured; if it measures none, \
             there is nothing to record -- do not estimate one"
        )
    })
}

/// A non-negative whole number as an emitter sent it: `None` when absent or `null`, an error when
/// it is anything else that is not a count. `what` names the kind of event in the error.
fn optional_count(json: &Value, what: &str, key: &str) -> Result<Option<u64>> {
    let Some(value) = json.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_u64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                .map(|f| f as u64)
        })
        .map(Some)
        .ok_or_else(|| {
            anyhow::anyhow!("{what}: `{key}` must be a non-negative integer, got {value}")
        })
}

pub fn load_routing(path: &Path) -> Result<Vec<RoutingEvent>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(Duration::from_millis(250))?;
    let has_events: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'routing_event')",
        [],
        |row| row.get(0),
    )?;
    if !has_events {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT task, phase, agent, model, provider, category, cost_status, requests, tokens, cost, retries, escalations, test_result, review_defects, created, id FROM routing_event",
    )?;
    // Strict, unlike `load_journal`: a row this tool wrote and cannot read back is a corrupt
    // journal, and the two callers already put an error where it is seen — the dashboard's
    // status line, and a refused export rather than a partial table. `filter_map(Result::ok)`
    // dropped the row and reported the rest as the whole.
    let mut rows = stmt.query([])?;
    let mut events = Vec::new();
    let mut position = 0usize;
    while let Some(row) = rows.next()? {
        position += 1;
        match routing_from_row(row) {
            Ok(event) => events.push(event),
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "routing_event {} could not be read ({error}); the journal is corrupt or \
                     was written by a newer version",
                    row_name(row, 15, position)
                ))
            }
        }
    }
    Ok(events)
}

/// How many requests the events whose `event_id` starts with `prefix` have attributed between
/// them: a harness's cursor into a transcript it reads more of each time.
///
/// An event that attributed nothing stores `requests: 1` — the recorder's floor — and `tokens:
/// 0`; a request that was attributed always has tokens, so the zero is what says the event
/// counted no request and must not advance the cursor past one.
///
/// A plain prefix comparison rather than `LIKE`, whose `_` and `%` would be wildcards inside
/// the session id the prefix carries.
///
/// This sum is a cursor over rows written long ago, which is why [`prune`] never deletes a row
/// of a session that still has newer ones. A harness that keeps a cursor under another prefix
/// has to be taught to `prune` as well, or pruning will make it count requests twice.
pub fn attributed_requests(path: &Path, prefix: &str) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(Duration::from_millis(250))?;
    let has_event_id: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('routing_event') WHERE name = 'event_id')",
        [],
        |row| row.get(0),
    )?;
    if !has_event_id {
        return Ok(0);
    }
    let attributed: i64 = conn.query_row(
        "SELECT COALESCE(SUM(CASE WHEN tokens > 0 THEN requests ELSE 0 END), 0) FROM routing_event WHERE substr(event_id, 1, length(?1)) = ?1",
        params![prefix],
        |row| row.get(0),
    )?;
    Ok(count(attributed))
}

/// One `routing_event` row, or why it could not be read.
fn routing_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoutingEvent> {
    let category: String = row.get(5)?;
    let cost_status: String = row.get(6)?;
    let test_result: Option<i64> = row.get(12)?;
    let cost: Option<f64> = row.get(9)?;
    {
        Ok(RoutingEvent {
            task: row.get(0)?,
            phase: row.get(1)?,
            agent: row.get(2)?,
            model: row.get(3)?,
            provider: row.get(4)?,
            category: category_from_label(&category),
            cost_status: cost_status_from_label(&cost_status),
            requests: count(row.get(7)?),
            tokens: count(row.get(8)?),
            cost,
            retries: row.get(10)?,
            escalations: row.get(11)?,
            test_result: test_result.map(|v| v != 0),
            review_defects: row.get(13)?,
            created: row.get(14)?,
        })
    }
}

pub fn record_routing(path: &Path) -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let json: Value = serde_json::from_str(&input)?;
    let inserted = record_routing_event(path, &json)?;
    crate::helpers::print_line(&format!(
        "Recorded {} routing event(s) in {}",
        inserted,
        path.display()
    ))?;
    Ok(())
}

/// Journal one routing event, returning how many rows that inserted: `0` when its identity was
/// already there. Split from the stdin read so the parse and the schema can be tested without one.
pub(crate) fn record_routing_event(path: &Path, json: &Value) -> Result<usize> {
    // Everything that can be refused is refused before the journal is opened: a bad event must
    // not create the file, or rebuild the table, on its way to an error.
    let agent = string(json, &["agent"]).unwrap_or_else(|| "unknown".to_string());
    let model = string(json, &["model"]).unwrap_or_else(|| "unknown".to_string());
    let task = string(json, &["task"]).unwrap_or_default();
    let created = json
        .get("created")
        .and_then(Value::as_i64)
        .unwrap_or_else(recorded_now);

    // The emitter's identity if it gave one. The derived form collapses two events for the same
    // task in the same second into one, and an emitter had no way around that before. An empty
    // string is no identity, as `usage_key` already holds for usage rows: taken literally it
    // would be one identity shared by every event from a template whose variable was unset, and
    // the journal would keep the first and silently ignore the rest.
    let event_id = string(json, &["event_id"])
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| format!("routing:{}:{}:{}:{}", agent, model, task, created));

    let category_label = string(json, &["category"]).unwrap_or_else(|| "UNKNOWN".to_string());
    let cost = json.get("cost").and_then(|v| match v {
        Value::Null => None,
        _ => v.as_f64(),
    });
    // A figure the emitter sent is a figure the emitter reported. The default was `unavailable`
    // whether or not a `cost` came with it, and since the aggregator started classifying by
    // status rather than trusting the number, that default made the figure vanish: the README's
    // own example — `"cost":0.02`, no status — recorded a task the panel then called `unpriced`.
    let cost_status_label = string(json, &["cost_status"]).unwrap_or_else(|| {
        if cost.is_some() {
            "reported"
        } else {
            "unavailable"
        }
        .to_string()
    });
    let test_result = test_result(json)?;
    let retries = counter(json, "retries")?;
    let escalations = counter(json, "escalations")?;
    let review_defects = counter(json, "review_defects")?;
    let requests = quantity(json, "requests")?.max(1);
    let tokens = quantity(json, "tokens")?;

    let conn = open_for_writing(path, |conn| {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS routing_event ({ROUTING_EVENT_COLUMNS});"
        ))?;
        let has_event_id: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('routing_event') WHERE name = 'event_id')",
            [],
            |row| row.get(0),
        )?;
        if !has_event_id {
            conn.execute("ALTER TABLE routing_event ADD COLUMN event_id TEXT", [])?;
        }
        allow_unreported_counters(conn)?;
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS routing_event_event_id ON routing_event(event_id)",
            [],
        )?;
        Ok(())
    })?;

    let inserted = conn.execute(
        "INSERT OR IGNORE INTO routing_event (event_id, task, phase, agent, model, provider, category, cost_status, requests, tokens, cost, retries, escalations, test_result, review_defects, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            event_id,
            task,
            string(json, &["phase"]).unwrap_or_default(),
            agent,
            model,
            string(json, &["provider"]).unwrap_or_else(|| "unknown".to_string()),
            category_label,
            cost_status_label,
            stored(requests),
            stored(tokens),
            cost,
            retries,
            escalations,
            test_result.map(|b| b as i64),
            review_defects,
            created,
        ],
    )?;
    Ok(inserted)
}

/// The table as it is created today. The three counters are nullable: an emitter that reports
/// nothing stores `NULL`, which is not `0`.
const ROUTING_EVENT_COLUMNS: &str = "
            id INTEGER PRIMARY KEY,
            event_id TEXT,
            task TEXT NOT NULL,
            phase TEXT NOT NULL,
            agent TEXT NOT NULL,
            model TEXT NOT NULL,
            provider TEXT NOT NULL,
            category TEXT NOT NULL,
            cost_status TEXT NOT NULL,
            requests INTEGER NOT NULL,
            tokens INTEGER NOT NULL,
            cost REAL,
            retries INTEGER,
            escalations INTEGER,
            test_result INTEGER,
            review_defects INTEGER,
            created INTEGER NOT NULL";

/// Drop the `NOT NULL` from the three counters of a journal written before they were nullable.
///
/// SQLite cannot alter a constraint in place, so the table is rebuilt: the documented way, inside
/// the transaction `open_for_writing` holds, with the index recreated after. It used to open its own
/// deferred `BEGIN`, which a concurrent writer could interleave with before the lock was taken. Rows already there keep their zeros. An omitted
/// field was stored as `0` then, and that is what was recorded — rewriting it as unknown would be
/// inventing in the other direction. Only rows written from here on can say "not reported".
fn allow_unreported_counters(conn: &Connection) -> Result<()> {
    let counters_required: i64 = conn.query_row(
        "SELECT \"notnull\" FROM pragma_table_info('routing_event') WHERE name = 'retries'",
        [],
        |row| row.get(0),
    )?;
    if counters_required == 0 {
        return Ok(());
    }
    const COLUMNS: &str = "id, event_id, task, phase, agent, model, provider, category, cost_status, requests, tokens, cost, retries, escalations, test_result, review_defects, created";
    conn.execute_batch(&format!(
        "CREATE TABLE routing_event_rebuilt ({ROUTING_EVENT_COLUMNS});
         INSERT INTO routing_event_rebuilt ({COLUMNS}) SELECT {COLUMNS} FROM routing_event;
         DROP TABLE routing_event;
         ALTER TABLE routing_event_rebuilt RENAME TO routing_event;
         CREATE UNIQUE INDEX IF NOT EXISTS routing_event_event_id ON routing_event(event_id);"
    ))?;
    Ok(())
}

/// A per-task counter as the emitter sent it.
///
/// Absent or `null` is "not reported", which is not `0`. Anything else must be a non-negative
/// integer: a string or a negative number silently becoming `0` — which is what the old
/// `number()` default did — is a silent failure, and those reach the user as errors (convention 8).
fn counter(json: &Value, key: &str) -> Result<Option<u32>> {
    let refuse = || {
        anyhow::anyhow!(
            "routing event: `{key}` must be a non-negative integer, got {}",
            json[key]
        )
    };
    optional_count(json, "routing event", key)?
        .map(|n| u32::try_from(n).map_err(|_| refuse()))
        .transpose()
}

/// A non-negative whole quantity — `tokens`, `requests` — where absent means `0`.
///
/// The counters above distinguish absent from zero; these do not need to, but they were the two
/// fields still going through `helpers::number`, which maps a string or a negative to `0` and
/// reports success. One rule for the whole event.
fn quantity(json: &Value, key: &str) -> Result<u64> {
    Ok(optional_count(json, "routing event", key)?.unwrap_or(0))
}

/// `test_result` as the emitter sent it: a boolean, `0`/`1`, or `"pass"`/`"fail"`.
///
/// Anything else is refused rather than stored as "unobserved". The old parse mapped an
/// unrecognised value to `null` and reported success, so an emitter writing `"pass"` — which the
/// round-trip test did — recorded nothing and never learned it.
fn test_result(json: &Value) -> Result<Option<bool>> {
    let refuse = |value: &Value| {
        anyhow::anyhow!(
            "routing event: `test_result` must be true, false, 0, 1, \"pass\" or \"fail\", got {value}"
        )
    };
    match json.get("test_result") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(passed)) => Ok(Some(*passed)),
        // `as_f64`, not `as_i64`: the same emitter that sends `retries: 2.0` sends
        // `test_result: 1.0`, and `counter()` accepts the one, so this accepts the other.
        Some(value @ Value::Number(n)) => match n.as_f64() {
            Some(0.0) => Ok(Some(false)),
            Some(1.0) => Ok(Some(true)),
            _ => Err(refuse(value)),
        },
        // Case-insensitive, and only the two words the docs and the error above name.
        Some(value @ Value::String(s)) => match s.to_ascii_lowercase().as_str() {
            "pass" => Ok(Some(true)),
            "fail" => Ok(Some(false)),
            _ => Err(refuse(value)),
        },
        Some(value) => Err(refuse(value)),
    }
}

/// The youngest age `--prune-journal` accepts, in days.
///
/// Budgets read back to the first of the local month and `--month` reads thirty days, so a row
/// younger than this is one the dashboard is still counting. Deleting it would not be retention,
/// it would be a smaller bill.
pub const PRUNE_MIN_DAYS: u64 = 31;

/// What a prune did to one table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TablePrune {
    /// Rows in the table before.
    pub before: u64,
    /// Rows older than the cutoff.
    pub eligible: u64,
    /// Rows deleted. Less than `eligible` by the rows that were kept on purpose: the newest-id
    /// usage row, and routing rows of a session that is still recording.
    pub deleted: u64,
}

/// What `--prune-journal` did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PruneReport {
    /// Rows created before this (Unix seconds) were eligible.
    pub cutoff: i64,
    /// `None` when the table does not exist: each is created by its own writer, on first use.
    pub usage: Option<TablePrune>,
    pub routing: Option<TablePrune>,
    pub bytes_before: u64,
    pub bytes_after: u64,
    /// Whether the space was handed back. A failed `VACUUM` -- a full disk, a busy journal --
    /// leaves the rows deleted and the file the size it was; running the command again retries.
    pub vacuum: std::result::Result<(), String>,
}

/// Delete rows older than `days`, then `VACUUM`. `None` when there is no journal.
///
/// The only place this tool deletes anything a user recorded, and only on the command: the
/// journal is the sole copy of what `--record-*` and the hook wrote, so nothing prunes it on a
/// timer, at startup or from the dashboard. It creates nothing either -- no file, no table, no
/// version stamp -- because "nothing to prune" must not leave a journal behind.
///
/// Three kinds of row older than the cutoff are kept, each for a number that would otherwise go
/// wrong:
///
/// - **Rows of the current local month**, whatever `days` says: the cutoff is never later than
///   `local_month_start`, which is how far back a monthly budget reads.
/// - **The usage row with the highest id.** `id` is `INTEGER PRIMARY KEY` without
///   `AUTOINCREMENT`, so a new row takes `MAX(id) + 1`. Delete the highest and its id is handed
///   out again -- below the [`JournalCursor`] of a dashboard that is open, to a row it will never
///   read. `created` is the caller's, so a replayed old log really can own the highest ids.
/// - **Routing rows of a Claude Code session that has newer rows.** The hook sums a session's
///   rows to know which requests it has already attributed ([`attributed_requests`]); take the
///   old ones away and it attributes those requests again. The session is the event id through
///   its second colon (`claude-code:{session}:`); an id that does not have that shape yields a
///   shorter key, which can only keep more rows, never fewer. Rows under any other id have no
///   cursor and go by `created` alone.
///
/// Undated rows (`created <= 0`) are kept: their age is not known. No index on `created` is
/// added for this -- a full scan is fine for a command run a few times a year, and an index is a
/// schema change every older writer would have to tolerate.
pub fn prune(path: &Path, days: u64, now: i64) -> Result<Option<PruneReport>> {
    anyhow::ensure!(
        days >= PRUNE_MIN_DAYS,
        "refusing to prune rows younger than {PRUNE_MIN_DAYS} days: budgets and --month still read them"
    );
    if !path.exists() {
        return Ok(None);
    }
    let bytes_before = fs::metadata(path)?.len();
    let age = i64::try_from(days.saturating_mul(86_400)).unwrap_or(i64::MAX);
    let cutoff = now
        .saturating_sub(age)
        .min(crate::utils::local_month_start());

    // Opened read-write and *not* create: a path that vanished between the check and here is an
    // error, not a new empty journal.
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(WRITER_BUSY_TIMEOUT)?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let deleted = (|| -> Result<(Option<TablePrune>, Option<TablePrune>)> {
        refuse_newer(&conn, path)?;
        let has_table = |name: &str| -> rusqlite::Result<bool> {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [name],
                |row| row.get(0),
            )
        };
        let counts = |table: &str| -> rusqlite::Result<(u64, u64)> {
            let before: i64 =
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })?;
            let eligible: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE created > 0 AND created < ?1"),
                [cutoff],
                |row| row.get(0),
            )?;
            Ok((count(before), count(eligible)))
        };

        let usage = if has_table("usage_event")? {
            let (before, eligible) = counts("usage_event")?;
            let deleted = conn.execute(
                "DELETE FROM usage_event
                 WHERE created > 0 AND created < ?1
                   AND id < (SELECT MAX(id) FROM usage_event)",
                [cutoff],
            )?;
            Some(TablePrune {
                before,
                eligible,
                deleted: deleted as u64,
            })
        } else {
            None
        };

        let routing = if has_table("routing_event")? {
            let (before, eligible) = counts("routing_event")?;
            let has_event_id: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('routing_event') WHERE name = 'event_id')",
                [],
                |row| row.get(0),
            )?;
            let deleted = if has_event_id {
                // `?2` is `claude-code:`. The session key is that plus everything up to and
                // including the next colon; `instr` is 0 when there is none, leaving `?2` itself.
                let session = "substr(event_id, 1, length(?2) + instr(substr(event_id, length(?2) + 1), ':'))";
                conn.execute(
                    &format!(
                        "DELETE FROM routing_event
                         WHERE created > 0 AND created < ?1
                           AND ( event_id IS NULL
                              OR substr(event_id, 1, length(?2)) <> ?2
                              OR {session} NOT IN (
                                   SELECT {session} FROM routing_event
                                   WHERE created >= ?1 AND substr(event_id, 1, length(?2)) = ?2 ) )"
                    ),
                    params![cutoff, format!("{}:", crate::harness::claude_code::AGENT)],
                )?
            } else {
                // A table from before `event_id`: no hook has ever written to it.
                conn.execute(
                    "DELETE FROM routing_event WHERE created > 0 AND created < ?1",
                    [cutoff],
                )?
            };
            Some(TablePrune {
                before,
                eligible,
                deleted: deleted as u64,
            })
        } else {
            None
        };
        Ok((usage, routing))
    })();
    let (usage, routing) = match deleted {
        Ok(tables) => {
            conn.execute_batch("COMMIT")?;
            tables
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(error);
        }
    };

    // Outside the transaction, where `VACUUM` has to run. Also when nothing was deleted but
    // free pages remain, so that running the command again finishes a `VACUUM` that failed.
    let any_deleted = [usage, routing]
        .iter()
        .flatten()
        .any(|table| table.deleted > 0);
    let free_pages: i64 = conn.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
    let vacuum = if any_deleted || free_pages > 0 {
        conn.execute_batch("VACUUM")
            .map_err(|error| error.to_string())
    } else {
        Ok(())
    };
    drop(conn);
    Ok(Some(PruneReport {
        cutoff,
        usage,
        routing,
        bytes_before,
        bytes_after: fs::metadata(path)?.len(),
        vacuum,
    }))
}

/// What `--doctor` says about the journal beyond its usage rows: how big it is, how many routing
/// events it holds -- which no source row counts -- and how far back it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalStats {
    pub bytes: u64,
    pub usage_rows: Option<u64>,
    pub routing_rows: Option<u64>,
    /// The oldest dated row in either table, Unix seconds.
    pub oldest: Option<i64>,
}

pub fn stats(path: &Path) -> Result<Option<JournalStats>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::metadata(path)?.len();
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(Duration::from_millis(250))?;
    let table = |name: &str| -> rusqlite::Result<Option<(u64, Option<i64>)>> {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [name],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        conn.query_row(
            &format!("SELECT COUNT(*), MIN(CASE WHEN created > 0 THEN created END) FROM {name}"),
            [],
            |row| Ok(Some((count(row.get(0)?), row.get(1)?))),
        )
    };
    let usage = table("usage_event")?;
    let routing = table("routing_event")?;
    let oldest = [usage, routing]
        .iter()
        .flatten()
        .filter_map(|(_, oldest)| *oldest)
        .min();
    Ok(Some(JournalStats {
        bytes,
        usage_rows: usage.map(|(rows, _)| rows),
        routing_rows: routing.map(|(rows, _)| rows),
        oldest,
    }))
}

pub struct JournalCollector {
    pub journal_path: PathBuf,
    pub interval_secs: u64,
    /// What the last poll had to skip, for the live status line. The log is off by default,
    /// so a count that reached only the log reached the dashboard's user nowhere.
    pub skipped: Option<String>,
    /// How far the polls have read; see [`JournalCursor`].
    pub(crate) cursor: JournalCursor,
}

impl Collector for JournalCollector {
    fn name(&self) -> &str {
        ID
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        let before = self.cursor.skipped;
        let (usages, skipped) = load_journal_since(&self.journal_path, &mut self.cursor)?;
        // Logged when the count moves, not every poll: the rows are read once now.
        if let Some(skipped) = skipped.as_ref().filter(|_| self.cursor.skipped != before) {
            crate::logging::error(ID, &skipped.to_string());
        }
        self.skipped = skipped.map(|s| format!("{} row(s) unreadable", s.count));
        Ok(usages)
    }
    fn warning(&self) -> Option<String> {
        self.skipped.clone()
    }
}

/// One-shot read for the source registry.
pub(crate) fn read(
    roots: &crate::collector::SourceRoots,
) -> crate::collector::registry::SourceRead {
    let (usages, skipped) = load_journal_counting(&roots.journal)?;
    let present = roots.journal.exists();
    Ok((
        crate::collector::SourceReport {
            id: ID,
            present,
            path: Some(roots.journal.clone()),
            rows: usages.len(),
            // The status line is what the `--once` header shows; a row count that quietly
            // omitted the rows it could not read would be a smaller bill. `--doctor` gets the
            // first reason as well.
            status: match (present, &skipped) {
                (true, Some(skipped)) => format!(
                    "journal: {} ({} row(s) unreadable)",
                    roots.journal.display(),
                    skipped.count
                ),
                (true, None) => format!("journal: {}", roots.journal.display()),
                (false, _) => "journal: not initialized".to_string(),
            },
            detail: skipped.map(|s| s.to_string()),
        },
        usages,
    ))
}

/// A background collector for the same source.
pub(crate) fn collector(
    roots: &crate::collector::SourceRoots,
    interval_secs: u64,
) -> Box<dyn Collector> {
    Box::new(JournalCollector {
        journal_path: roots.journal.clone(),
        interval_secs,
        skipped: None,
        cursor: JournalCursor::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A journal in a scratch directory that is removed when the test ends, panicking or not.
    struct Scratch {
        journal: PathBuf,
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Some(dir) = self.journal.parent() {
                let _ = fs::remove_dir_all(dir);
            }
        }
    }

    fn scratch_journal(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "ai-usage-tui-journal-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        Scratch {
            journal: dir.join("usage.db"),
        }
    }

    fn event(task: &str, created: i64) -> Value {
        json!({"agent": "a", "model": "m", "task": task, "tokens": 10, "created": created})
    }

    #[test]
    fn an_omitted_counter_is_stored_as_unreported_not_zero() {
        // Restore the bug by storing `number(json, "retries") as u32`: both rows read back as
        // `0`, and an agent whose harness never counted retries reports a 0% retry rate.
        let scratch = scratch_journal("unreported");
        let journal = scratch.journal.clone();
        record_routing_event(&journal, &event("said-nothing", 1)).expect("record");
        let mut zeros = event("said-zero", 2);
        zeros["retries"] = json!(0);
        zeros["escalations"] = json!(0);
        zeros["review_defects"] = json!(0);
        record_routing_event(&journal, &zeros).expect("record");

        let mut events = load_routing(&journal).expect("load");
        events.sort_by_key(|e| e.created);
        assert_eq!(events[0].retries, None, "omitted is not zero");
        assert_eq!(events[0].escalations, None);
        assert_eq!(events[0].review_defects, None);
        assert_eq!(events[1].retries, Some(0), "zero is not omitted");
        assert_eq!(events[1].escalations, Some(0));
        assert_eq!(events[1].review_defects, Some(0));
    }

    #[test]
    fn a_test_result_string_is_recorded_and_junk_is_refused() {
        // `"pass"` used to be mapped to null and reported as success: the round-trip CLI test
        // sent exactly that and asserted everything except the field it dropped.
        let scratch = scratch_journal("test-result");
        let journal = scratch.journal.clone();
        let mut passed = event("t1", 1);
        passed["test_result"] = json!("pass");
        let mut failed = event("t2", 2);
        failed["test_result"] = json!("FAIL");
        record_routing_event(&journal, &passed).expect("record");
        record_routing_event(&journal, &failed).expect("record");
        let mut events = load_routing(&journal).expect("load");
        events.sort_by_key(|e| e.created);
        assert_eq!(events[0].test_result, Some(true));
        assert_eq!(events[1].test_result, Some(false));

        // The same emitter that sends `retries: 2.0` sends `test_result: 1.0`.
        let mut float = event("t3", 3);
        float["test_result"] = json!(1.0);
        record_routing_event(&journal, &float).expect("record");
        assert_eq!(
            load_routing(&journal).expect("load")[2].test_result,
            Some(true)
        );

        // Only the two documented words, and only 0 or 1: anything else is refused, not stored
        // as "unobserved" under a success message.
        for junk in [
            json!("maybe"),
            json!("passed"),
            json!("true"),
            json!(2),
            json!(-1),
        ] {
            let mut e = event("t4", 4);
            e["test_result"] = junk.clone();
            let error = record_routing_event(&journal, &e).expect_err("junk refused");
            assert!(error.to_string().contains("test_result"), "{junk}: {error}");
        }
        assert_eq!(
            load_routing(&journal).expect("load").len(),
            3,
            "nothing was stored"
        );
    }

    #[test]
    fn a_counter_that_is_not_a_count_is_refused() {
        let scratch = scratch_journal("bad-counter");
        let journal = scratch.journal.clone();
        for bad in [
            json!("three"),
            json!(-1),
            json!(1.5),
            json!(true),
            json!(4_294_967_296_u64),
            json!(4_294_967_296.0),
        ] {
            let mut e = event("t", 1);
            e["retries"] = bad.clone();
            let error = record_routing_event(&journal, &e).expect_err("refused");
            assert!(error.to_string().contains("retries"), "{bad}: {error}");
        }
        // A refused event must not have created the journal on its way to the error.
        assert!(!journal.exists(), "a refused event created the journal");
        // `tokens` and `requests` follow the same rule; they were the last two fields going
        // through `helpers::number`, which turned a string into 0 and reported success.
        let mut e = event("t", 1);
        e["tokens"] = json!("lots");
        let error = record_routing_event(&journal, &e).expect_err("refused");
        assert!(error.to_string().contains("tokens"), "{error}");
        assert!(!journal.exists(), "a refused quantity created the journal");
        // An integral float is a count an emitter in a loosely typed language will send.
        let mut e = event("t", 1);
        e["retries"] = json!(2.0);
        record_routing_event(&journal, &e).expect("record");
        assert_eq!(load_routing(&journal).expect("load")[0].retries, Some(2));
    }

    #[test]
    fn a_cost_sent_without_a_status_is_a_reported_cost_not_an_unpriced_one() {
        // Restore the bug by defaulting `cost_status` to `unavailable` unconditionally: the
        // event carries `0.02`, the aggregator files it under `unpriced_tasks`, and the panel
        // renders the README's own example as `unpriced`.
        let scratch = scratch_journal("cost-status");
        let journal = scratch.journal.clone();
        let mut priced = event("t1", 1);
        priced["cost"] = json!(0.02);
        let mut explicit = event("t2", 2);
        explicit["cost"] = json!(0.02);
        explicit["cost_status"] = json!("quota");
        record_routing_event(&journal, &priced).expect("record");
        record_routing_event(&journal, &explicit).expect("record");
        record_routing_event(&journal, &event("t3", 3)).expect("record");

        let mut events = load_routing(&journal).expect("load");
        events.sort_by_key(|e| e.created);
        assert_eq!(events[0].cost_status, CostStatus::ProviderReported);
        assert_eq!(events[0].cost, Some(0.02));
        assert_eq!(
            events[1].cost_status,
            CostStatus::Quota,
            "an explicit status is never overridden"
        );
        assert_eq!(
            events[2].cost_status,
            CostStatus::Unavailable,
            "no figure and no status is still unknown"
        );
    }

    #[test]
    fn a_supplied_event_id_is_the_identity() {
        // The derived identity collapses two events for one task in the same second, and an
        // emitter had no way around it: the field was read from nowhere.
        let scratch = scratch_journal("event-id");
        let journal = scratch.journal.clone();
        let mut first = event("t", 1);
        first["event_id"] = json!("run-1");
        let mut second = event("t", 1);
        second["event_id"] = json!("run-2");
        assert_eq!(record_routing_event(&journal, &first).expect("record"), 1);
        assert_eq!(record_routing_event(&journal, &second).expect("record"), 1);
        assert_eq!(
            record_routing_event(&journal, &first).expect("record"),
            0,
            "the same identity is ignored, not duplicated"
        );
        assert_eq!(load_routing(&journal).expect("load").len(), 2);

        // An empty string is no identity. Taken literally it is one identity shared by every
        // event from a template whose `$RUN_ID` was unset, and the journal keeps the first and
        // reports `Recorded 0 routing event(s)` for the rest, forever, with exit 0.
        let mut blank_a = event("u", 5);
        blank_a["event_id"] = json!("");
        let mut blank_b = event("v", 5);
        blank_b["event_id"] = json!("");
        assert_eq!(record_routing_event(&journal, &blank_a).expect("record"), 1);
        assert_eq!(
            record_routing_event(&journal, &blank_b).expect("record"),
            1,
            "a second event with an empty event_id was swallowed by the first"
        );
    }

    #[test]
    fn a_journal_written_before_counters_were_nullable_is_rebuilt_in_place() {
        // The table as v0.9.0 created it: an emitter that reported nothing was stored as 0.
        let scratch = scratch_journal("migrate");
        let journal = scratch.journal.clone();
        let conn = Connection::open(&journal).expect("open");
        conn.execute_batch(
            "CREATE TABLE routing_event (
                id INTEGER PRIMARY KEY,
                event_id TEXT,
                task TEXT NOT NULL, phase TEXT NOT NULL, agent TEXT NOT NULL,
                model TEXT NOT NULL, provider TEXT NOT NULL, category TEXT NOT NULL,
                cost_status TEXT NOT NULL, requests INTEGER NOT NULL, tokens INTEGER NOT NULL,
                cost REAL, retries INTEGER NOT NULL, escalations INTEGER NOT NULL,
                test_result INTEGER, review_defects INTEGER NOT NULL, created INTEGER NOT NULL
            );
            CREATE UNIQUE INDEX routing_event_event_id ON routing_event(event_id);
            INSERT INTO routing_event (event_id, task, phase, agent, model, provider, category,
                cost_status, requests, tokens, cost, retries, escalations, test_result,
                review_defects, created)
            VALUES ('old', 'old-task', '', 'a', 'm', 'p', 'UNKNOWN', 'unavailable', 1, 10, NULL,
                0, 0, NULL, 0, 1);",
        )
        .expect("old schema");
        drop(conn);

        assert_eq!(
            record_routing_event(&journal, &event("new-task", 2)).expect("record into old journal"),
            1
        );
        let mut events = load_routing(&journal).expect("load");
        events.sort_by_key(|e| e.created);
        assert_eq!(events.len(), 2, "the old row survived the rebuild");
        assert_eq!(
            events[0].retries,
            Some(0),
            "what was recorded stays recorded; a stored zero is not rewritten as unknown"
        );
        assert_eq!(
            events[1].retries, None,
            "the new row can say it was not reported"
        );
        // An identity is still unique once the recorder has run on the rebuilt table. (The
        // recorder's own `CREATE UNIQUE INDEX IF NOT EXISTS` would satisfy this too; the
        // oldest-shape test below observes the index the rebuild itself creates.)
        assert_eq!(
            record_routing_event(&journal, &event("new-task", 2)).expect("record"),
            0
        );
    }

    #[test]
    fn a_routing_row_that_cannot_be_read_fails_the_read_rather_than_vanishing() {
        // Restore the bug with `filter_map(Result::ok)`: the row is dropped and the read reports
        // the rest as the whole. A negative count cannot become a `u32`.
        let scratch = scratch_journal("corrupt-routing");
        let journal = scratch.journal.clone();
        record_routing_event(&journal, &event("good", 1)).expect("record");
        record_routing_event(&journal, &event("bad", 2)).expect("record");
        let conn = Connection::open(&journal).expect("open");
        conn.execute(
            "UPDATE routing_event SET retries = -1 WHERE task = 'bad'",
            [],
        )
        .expect("corrupt a row");
        drop(conn);

        let error = load_routing(&journal).expect_err("a corrupt row is an error");
        assert!(error.to_string().contains("id 2"), "{error}");
    }

    #[test]
    fn a_usage_row_that_cannot_be_read_is_counted_and_the_rest_survive() {
        // Not strict, deliberately: `load_usage` fails as a whole on any source error, and one
        // corrupt Ollama row must not take every command down. But the count reaches the
        // source report, so `--doctor` says it — restore the bug and `skipped` is `None`.
        let scratch = scratch_journal("corrupt-usage");
        let journal = scratch.journal.clone();
        let conn = Connection::open(&journal).expect("open");
        conn.execute_batch(
            "CREATE TABLE usage_event (
                id INTEGER PRIMARY KEY, event_id TEXT, provider TEXT NOT NULL, model TEXT NOT NULL,
                category TEXT NOT NULL, cost_status TEXT NOT NULL, requests INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                reasoning_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                cache_write_tokens INTEGER NOT NULL, cost REAL, created INTEGER NOT NULL
            );
            INSERT INTO usage_event (provider, model, category, cost_status, requests,
                input_tokens, output_tokens, reasoning_tokens, cache_read_tokens,
                cache_write_tokens, cost, created)
            VALUES ('ollama', 'm', 'LOCAL', 'local', 1, 10, 10, 0, 0, 0, NULL, 1),
                   ('ollama', 'm', 'LOCAL', 'local', 1, 10, 10, 0, 0, 0, NULL, 'soon');",
        )
        .expect("plant rows");
        drop(conn);

        let (usages, skipped) = load_journal_counting(&journal).expect("read");
        assert_eq!(usages.len(), 1, "the readable row survives");
        let skipped = skipped.expect("the unreadable row is counted");
        assert_eq!(skipped.count, 1);
        assert!(skipped.first.contains("id 2"), "{}", skipped.first);

        // The live dashboard reads the count off the collector, not the log — which is off by
        // default, so a count that reached only the log reached its user nowhere.
        let mut collector = JournalCollector {
            journal_path: journal.clone(),
            interval_secs: 60,
            skipped: None,
            cursor: JournalCursor::default(),
        };
        assert_eq!(collector.poll().expect("poll").len(), 1);
        assert_eq!(
            collector.warning().as_deref(),
            Some("1 row(s) unreadable"),
            "the collector must carry the count to the status line"
        );
        // The next poll reads nothing -- the cursor is past both rows -- and the row is as
        // unreadable as it was. Bug: a count local to one read, so the warning showed for one
        // poll and then the status line went clean.
        assert!(collector.poll().expect("poll").is_empty());
        assert_eq!(collector.warning().as_deref(), Some("1 row(s) unreadable"));
    }

    fn cursor_event(id: &str) -> Value {
        json!({
            "provider": "aider", "model": "claude-sonnet-5", "event_id": id,
            "input_tokens": 1200, "output_tokens": 300, "created": 1_758_000_000
        })
    }

    fn journal_collector(journal: &Path) -> JournalCollector {
        JournalCollector {
            journal_path: journal.to_path_buf(),
            interval_secs: 60,
            skipped: None,
            cursor: JournalCursor::default(),
        }
    }

    /// Bug: `SELECT ... FROM usage_event` with no `WHERE`, every sixty seconds, for `merge` to
    /// throw away again.
    #[test]
    fn a_poll_reads_only_what_was_recorded_since_the_last_one() {
        let scratch = scratch_journal("cursor");
        let journal = scratch.journal.clone();
        let mut collector = journal_collector(&journal);
        assert!(collector.poll().expect("no journal yet").is_empty());

        record_events(&journal, &[cursor_event("c1"), cursor_event("c2")]).expect("record");
        assert_eq!(collector.poll().expect("poll").len(), 2);
        assert!(
            collector.poll().expect("poll").is_empty(),
            "a poll with nothing new re-read the table"
        );

        record_events(&journal, &[cursor_event("c3")]).expect("record");
        let third = collector.poll().expect("poll");
        assert_eq!(third.len(), 1, "only the row recorded since");
        assert!(third[0].event_id.as_deref().unwrap().ends_with("c3"));

        // The one-shot read is a cursor that has seen nothing: every row, every time.
        assert_eq!(load_journal(&journal).expect("read").len(), 3);
        assert_eq!(load_journal(&journal).expect("read").len(), 3);
    }

    /// Bug: a cursor left pointing into a journal that is no longer there -- deleted and
    /// started again -- hiding every row below the old high-water mark.
    #[test]
    fn a_replaced_journal_is_read_from_the_start() {
        let scratch = scratch_journal("cursor-replaced");
        let journal = scratch.journal.clone();
        let mut collector = journal_collector(&journal);
        record_events(
            &journal,
            &[cursor_event("r1"), cursor_event("r2"), cursor_event("r3")],
        )
        .expect("record");
        assert_eq!(collector.poll().expect("poll").len(), 3);

        fs::remove_file(&journal).expect("delete the journal");
        assert!(collector.poll().expect("absent").is_empty());
        record_events(&journal, &[cursor_event("r4")]).expect("record");
        let rows = collector.poll().expect("poll");
        assert_eq!(
            rows.len(),
            1,
            "id 1 of the new journal is below the old cursor"
        );

        // Emptied in place, same file: caught by MAX(id), with or without an inode to compare.
        let conn = Connection::open(&journal).expect("open");
        conn.execute_batch("DELETE FROM usage_event;")
            .expect("empty");
        drop(conn);
        assert!(collector.poll().expect("poll").is_empty());
        record_events(&journal, &[cursor_event("r5")]).expect("record");
        assert_eq!(collector.poll().expect("poll").len(), 1, "id 1 again");
    }

    const DAY: i64 = 86_400;

    fn dated_usage(id: &str, created: i64) -> Value {
        let mut event = cursor_event(id);
        event["created"] = json!(created);
        event
    }

    fn hook_event(id: &str, created: i64) -> Value {
        let mut event = event(id, created);
        event["event_id"] = json!(id);
        event
    }

    fn routing_ids(journal: &Path) -> Vec<String> {
        let conn = Connection::open(journal).expect("open");
        let mut stmt = conn
            .prepare("SELECT COALESCE(event_id, task) FROM routing_event ORDER BY id")
            .expect("prepare");
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .map(|id| id.expect("row"))
            .collect();
        ids
    }

    fn usage_row_ids(journal: &Path) -> Vec<i64> {
        let conn = Connection::open(journal).expect("open");
        let mut stmt = conn
            .prepare("SELECT id FROM usage_event ORDER BY id")
            .expect("prepare");
        let ids = stmt
            .query_map([], |row| row.get::<_, i64>(0))
            .expect("query")
            .map(|id| id.expect("row"))
            .collect();
        ids
    }

    /// Bug: pruning routing rows by age alone. The hook sums a session's rows to know which
    /// requests it has already attributed; without the old ones it attributes them again, and
    /// the panel shows requests that were never made.
    #[test]
    fn a_prune_never_takes_rows_from_a_session_that_is_still_recording() {
        let scratch = scratch_journal("prune-live");
        let journal = scratch.journal.clone();
        let now = crate::utils::now();
        let old = now - 100 * DAY;
        for event in [
            hook_event("claude-code:live:main:t1", old),
            hook_event("claude-code:live:Explore:t2", old + 1),
            hook_event("claude-code:live:main:t3", now - 60),
            hook_event("claude-code:finished:main:t1", old + 2),
            hook_event("claude-code:finished:main:t2", old + 3),
            hook_event("my-harness-7", old + 4),
            event("no-id-of-its-own", old + 5),
            hook_event("my-harness-8", now - 120),
        ] {
            assert_eq!(record_routing_event(&journal, &event).expect("record"), 1);
        }
        let conn = Connection::open(&journal).expect("open");
        conn.execute_batch(
            "INSERT INTO routing_event (event_id, task, phase, agent, model, provider, category,
                cost_status, requests, tokens, created)
             VALUES ('undated', 'undated', '', 'a', 'm', 'p', 'UNKNOWN', 'unavailable', 1, 10, 0);",
        )
        .expect("an undated row");
        drop(conn);
        let live_main = "claude-code:live:main:";
        assert_eq!(attributed_requests(&journal, live_main).unwrap(), 2);

        let report = prune(&journal, 31, now).expect("prune").expect("a journal");
        assert_eq!(
            report.routing,
            Some(TablePrune {
                before: 9,
                eligible: 6,
                deleted: 4
            })
        );
        assert_eq!(report.usage, None, "no usage was ever recorded here");
        assert_eq!(report.vacuum, Ok(()));
        assert_eq!(
            routing_ids(&journal),
            [
                "claude-code:live:main:t1",
                "claude-code:live:Explore:t2",
                "claude-code:live:main:t3",
                "my-harness-8",
                "undated",
            ],
            "the live session whole -- its subagent's rows too -- and the row of unknown age"
        );
        assert_eq!(
            attributed_requests(&journal, live_main).unwrap(),
            2,
            "the hook's cursor moved: it would attribute those requests again"
        );
        assert_eq!(
            attributed_requests(&journal, "claude-code:finished:main:").unwrap(),
            0
        );
        // Nothing left to do: no rows, no free pages, no second VACUUM.
        let again = prune(&journal, 31, now).unwrap().unwrap();
        assert_eq!(again.routing.unwrap().deleted, 0);
        assert_eq!(again.bytes_before, again.bytes_after);
    }

    /// Bug: deleting the highest-id row. SQLite hands that id out again, below the cursor of a
    /// dashboard that is open, to a row it will never read.
    #[test]
    fn a_prune_keeps_the_newest_id_so_an_open_dashboard_misses_nothing() {
        let scratch = scratch_journal("prune-max-id");
        let journal = scratch.journal.clone();
        let now = crate::utils::now();
        // A replayed old log: the *oldest* events own the *highest* ids.
        record_events(
            &journal,
            &[
                dated_usage("recent", now - 60),
                dated_usage("old-1", now - 200 * DAY),
                dated_usage("old-2", now - 199 * DAY),
            ],
        )
        .expect("record");
        let mut collector = journal_collector(&journal);
        assert_eq!(collector.poll().expect("poll").len(), 3);

        let report = prune(&journal, 90, now).unwrap().unwrap();
        assert_eq!(
            report.usage,
            Some(TablePrune {
                before: 3,
                eligible: 2,
                deleted: 1
            })
        );
        assert_eq!(
            usage_row_ids(&journal),
            [1, 3],
            "ids survive VACUUM, and the highest stays"
        );

        record_events(&journal, &[dated_usage("after-the-prune", now)]).expect("record");
        assert_eq!(usage_row_ids(&journal), [1, 3, 4], "no id was reused");
        let seen = collector.poll().expect("poll");
        assert_eq!(
            seen.len(),
            1,
            "the row recorded after the prune reached the dashboard"
        );
        assert!(seen[0]
            .event_id
            .as_deref()
            .unwrap()
            .ends_with("after-the-prune"));
    }

    /// Bug: a row the dashboard still counts deleted because `DAYS` said so. A monthly budget
    /// reads back to the first of the local month.
    #[test]
    fn a_prune_refuses_young_rows_and_never_reaches_into_this_month() {
        let scratch = scratch_journal("prune-floor");
        let journal = scratch.journal.clone();
        let now = crate::utils::now();
        record_events(
            &journal,
            &[dated_usage("a", now - 40 * DAY), dated_usage("b", now)],
        )
        .expect("record");
        let error = prune(&journal, 30, now).unwrap_err().to_string();
        assert!(error.contains("31"), "{error}");
        assert_eq!(usage_row_ids(&journal), [1, 2], "a refusal deletes nothing");

        let report = prune(&journal, 31, now).unwrap().unwrap();
        assert!(report.cutoff <= crate::utils::local_month_start());
        assert!(report.cutoff <= now - 31 * DAY);

        // The clock is a parameter, so the clause can be reached: forty days into a month that
        // never gets that long, thirty-one days back is the 9th -- and the cutoff is still the 1st.
        let month_start = crate::utils::local_month_start();
        let report = prune(&journal, 31, month_start + 40 * DAY)
            .unwrap()
            .unwrap();
        assert_eq!(
            report.cutoff, month_start,
            "rows of the current month were eligible: a monthly budget still reads them"
        );
    }

    /// Bug: "nothing to prune" leaving a journal behind -- a file, a table, a version stamp.
    #[test]
    fn a_prune_creates_nothing() {
        let scratch = scratch_journal("prune-creates-nothing");
        let journal = scratch.journal.clone();
        let now = crate::utils::now();
        assert_eq!(prune(&journal, 31, now).expect("no journal"), None);
        assert!(!journal.exists(), "a prune created the journal");
        assert_eq!(stats(&journal).expect("stats"), None);

        // Only the routing writer has ever run: there is no `usage_event`, and there still is not.
        record_routing_event(&journal, &event("t", now)).expect("record");
        let schema = |journal: &Path| -> Vec<String> {
            let conn = Connection::open(journal).expect("open");
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master ORDER BY name")
                .expect("prepare");
            let names = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .expect("query")
                .map(|name| name.expect("row"))
                .collect();
            names
        };
        let before = schema(&journal);
        let report = prune(&journal, 31, now).unwrap().unwrap();
        assert_eq!(report.usage, None);
        assert_eq!(report.routing.unwrap().deleted, 0);
        assert_eq!(schema(&journal), before);

        let found = stats(&journal).unwrap().unwrap();
        assert_eq!((found.usage_rows, found.routing_rows), (None, Some(1)));
        assert_eq!(found.oldest, Some(now));
        assert!(found.bytes > 0);
    }

    /// Bug: a prune writing into a journal whose shape it may not understand.
    #[test]
    fn a_prune_refuses_a_journal_from_a_newer_build() {
        let scratch = scratch_journal("prune-newer");
        let journal = scratch.journal.clone();
        let now = crate::utils::now();
        record_events(
            &journal,
            &[dated_usage("old", now - 200 * DAY), dated_usage("new", now)],
        )
        .expect("record");
        let conn = Connection::open(&journal).expect("open");
        conn.execute_batch("PRAGMA user_version = 99;")
            .expect("stamp");
        drop(conn);
        let error = prune(&journal, 31, now).unwrap_err().to_string();
        assert!(error.contains("newer ai-usage-tui"), "{error}");
        assert_eq!(usage_row_ids(&journal), [1, 2], "rows intact");
    }

    /// Bug: a `VACUUM` that failed once -- a full disk -- never retried, because the rerun finds
    /// no rows to delete and stops there.
    #[test]
    fn a_rerun_reclaims_the_space_a_failed_vacuum_left() {
        let scratch = scratch_journal("prune-freelist");
        let journal = scratch.journal.clone();
        let now = crate::utils::now();
        let events: Vec<Value> = (0..3000)
            .map(|n| dated_usage(&format!("bulk-{n}"), now))
            .collect();
        record_events(&journal, &events).expect("record");
        // Rows gone, pages still in the file: what a prune whose VACUUM failed leaves.
        let conn = Connection::open(&journal).expect("open");
        conn.execute_batch("DELETE FROM usage_event WHERE id < 3000;")
            .expect("delete");
        drop(conn);

        let report = prune(&journal, 31, now).unwrap().unwrap();
        assert_eq!(report.usage.unwrap().deleted, 0);
        assert_eq!(report.vacuum, Ok(()));
        assert!(
            report.bytes_after < report.bytes_before,
            "free pages were left in the file: {} -> {}",
            report.bytes_before,
            report.bytes_after
        );
    }

    #[test]
    fn the_oldest_journal_shape_is_migrated_in_the_right_order() {
        // Before `event_id` existed at all: no column, no index. The `ALTER TABLE` that adds the
        // column has to run before the rebuild, whose `SELECT` names it.
        let scratch = scratch_journal("migrate-oldest");
        let journal = scratch.journal.clone();
        let conn = Connection::open(&journal).expect("open");
        conn.execute_batch(
            "CREATE TABLE routing_event (
                id INTEGER PRIMARY KEY,
                task TEXT NOT NULL, phase TEXT NOT NULL, agent TEXT NOT NULL,
                model TEXT NOT NULL, provider TEXT NOT NULL, category TEXT NOT NULL,
                cost_status TEXT NOT NULL, requests INTEGER NOT NULL, tokens INTEGER NOT NULL,
                cost REAL, retries INTEGER NOT NULL, escalations INTEGER NOT NULL,
                test_result INTEGER, review_defects INTEGER NOT NULL, created INTEGER NOT NULL
            );
            INSERT INTO routing_event (task, phase, agent, model, provider, category,
                cost_status, requests, tokens, cost, retries, escalations, test_result,
                review_defects, created)
            VALUES ('old-task', '', 'a', 'm', 'p', 'UNKNOWN', 'unavailable', 1, 10, NULL,
                2, 0, NULL, 0, 1);",
        )
        .expect("oldest schema");
        drop(conn);

        assert_eq!(
            record_routing_event(&journal, &event("new-task", 2)).expect("record"),
            1
        );
        let mut events = load_routing(&journal).expect("load");
        events.sort_by_key(|e| e.created);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].retries, Some(2));
        assert_eq!(events[1].retries, None);
        let conn = Connection::open(&journal).expect("open");
        let indexed: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'routing_event_event_id')",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert!(indexed, "the identity index was not created by the rebuild");
    }

    /// Both fixtures are real captures from a `llama-server` on this machine, not hand-written
    /// shapes: the cached-token split below is only interesting because a real server reports it.
    fn fixture(name: &str) -> Vec<Value> {
        let path =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")).join(name);
        let text = fs::read_to_string(path).expect("fixture");
        read_json_events(&text).expect("parse").0
    }

    #[test]
    fn a_cached_prompt_is_not_billed_twice() {
        // OpenAI counts cached tokens inside `prompt_tokens`; the journal keeps them apart. Add
        // the cached share to `input` as well and this row reports 11 input tokens for 4.
        let scratch = scratch_journal("llamacpp-cache");
        let journal = scratch.journal.clone();
        assert_eq!(
            record_usage_events(&journal, "llamacpp", &fixture("llamacpp_chat.json"))
                .expect("record"),
            1
        );

        let rows = load_journal(&journal).expect("load");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.provider, "llamacpp");
        assert_eq!(row.model, "qwen3.6-35b-a3b");
        assert_eq!(
            row.input, 4,
            "11 prompt tokens less the 7 the server had cached"
        );
        assert_eq!(row.cache_read, 7);
        assert_eq!(row.output, 8);
        assert_eq!(row.reasoning, 0, "llama.cpp reports no reasoning count");
        assert_eq!(row.created, 1_789_525_959);
    }

    #[test]
    fn a_local_server_costs_a_genuine_zero_and_says_so() {
        let scratch = scratch_journal("llamacpp-local");
        let journal = scratch.journal.clone();
        record_usage_events(&journal, "llamacpp", &fixture("llamacpp_chat.json")).expect("record");

        let rows = load_journal(&journal).expect("load");
        assert_eq!(rows[0].category, Category::Local);
        assert_eq!(rows[0].cost_status, CostStatus::Local);
        assert_eq!(
            rows[0].cost, None,
            "a recorded zero would be an invented price"
        );
    }

    #[test]
    fn a_stream_is_recorded_from_its_final_chunk() {
        // Server-sent events, `data:` prefixes and `[DONE]` included, exactly as curl writes them.
        let scratch = scratch_journal("llamacpp-stream");
        let journal = scratch.journal.clone();
        let events = fixture("llamacpp_stream.sse");
        assert!(
            events.len() > 1,
            "the fixture is a stream, not one document"
        );
        assert_eq!(
            record_usage_events(&journal, "llamacpp", &events).expect("record"),
            1,
            "one response, however many chunks carried it"
        );

        let rows = load_journal(&journal).expect("load");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input, 13);
        assert_eq!(rows[0].output, 24);
        assert_eq!(rows[0].cache_read, 0);
    }

    #[test]
    fn replaying_a_response_records_it_once() {
        let scratch = scratch_journal("llamacpp-replay");
        let journal = scratch.journal.clone();
        let events = fixture("llamacpp_chat.json");
        record_usage_events(&journal, "llamacpp", &events).expect("record");
        assert_eq!(
            record_usage_events(&journal, "llamacpp", &events).expect("record"),
            0,
            "the server's own id keys the event; a wrapper that records twice must not inflate it"
        );
        assert_eq!(load_journal(&journal).expect("load").len(), 1);
    }

    #[test]
    fn a_response_without_usage_is_refused_rather_than_recorded_as_zero() {
        // What a streamed request gets when it did not ask for stream_options.include_usage.
        let scratch = scratch_journal("llamacpp-no-usage");
        let journal = scratch.journal.clone();
        let chunk = json!({"model": "qwen3.6-35b-a3b", "choices": [{"delta": {"content": "hi"}}]});
        let error = record_usage_events(&journal, "llamacpp", &[chunk])
            .expect_err("a chunk with no usage is not a completed response");
        assert!(
            error.to_string().contains("include_usage"),
            "the error has to name the flag that fixes it: {error}"
        );
        assert!(load_journal(&journal).expect("load").is_empty());
    }

    #[test]
    fn an_empty_provider_is_refused() {
        // `--record-usage=` would otherwise file the row under "", which classifies as neither
        // local nor priced and reads as a mystery in every panel.
        let scratch = scratch_journal("empty-provider");
        let journal = scratch.journal.clone();
        let error = record_usage_events(&journal, "  ", &fixture("llamacpp_chat.json"))
            .expect_err("a blank provider is not a provider");
        assert!(error.to_string().contains("provider"), "{error}");
    }

    #[test]
    fn a_provider_that_is_not_local_keeps_its_cost_unknown() {
        // The recorder is provider-agnostic on purpose, and only `classify` decides what a
        // provider means. A hosted one must not inherit the local zero.
        let scratch = scratch_journal("hosted");
        let journal = scratch.journal.clone();
        record_usage_events(&journal, "openrouter", &fixture("llamacpp_chat.json"))
            .expect("record");
        let rows = load_journal(&journal).expect("load");
        assert_eq!(rows[0].cost_status, CostStatus::Unavailable);
        assert_eq!(rows[0].cost, None);
    }
}

#[cfg(test)]
mod concurrent_writer_tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Barrier};

    /// A journal from before `event_id`, which every writer migrates on open.
    fn pre_event_id_journal(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_event (
                id INTEGER PRIMARY KEY, provider TEXT NOT NULL, model TEXT NOT NULL,
                category TEXT NOT NULL, cost_status TEXT NOT NULL, requests INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                reasoning_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                cache_write_tokens INTEGER NOT NULL, cost REAL, created INTEGER NOT NULL);",
        )
        .unwrap();
    }

    /// The journal as v0.18.0 wrote it: `event_id` and its index, none of `--record-event`'s
    /// columns, one row, stamped with the schema version.
    fn pre_record_event_journal(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_event (
                id INTEGER PRIMARY KEY, event_id TEXT, provider TEXT NOT NULL, model TEXT NOT NULL,
                category TEXT NOT NULL, cost_status TEXT NOT NULL, requests INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                reasoning_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                cache_write_tokens INTEGER NOT NULL, cost REAL, created INTEGER NOT NULL);
             CREATE UNIQUE INDEX usage_event_event_id ON usage_event(event_id);
             INSERT INTO usage_event (event_id, provider, model, category, cost_status, requests,
                input_tokens, output_tokens, reasoning_tokens, cache_read_tokens,
                cache_write_tokens, cost, created)
             VALUES ('ollama:old', 'ollama', 'qwen3', 'LOCAL', 'local', 1, 7, 3, 0, 0, 0, NULL, 100);
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }

    fn usage_event(id: &str) -> Value {
        json!({
            "provider": "aider", "model": "claude-sonnet-5", "event_id": id,
            "input_tokens": 1200, "output_tokens": 300, "created": 1_758_000_000
        })
    }

    #[test]
    fn a_recorded_event_carries_what_a_bare_response_cannot() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        let mut event = usage_event("a1");
        event["cache_write_tokens"] = json!(50);
        event["reasoning_tokens"] = json!(20);
        event["project"] = json!("/work/app/");
        event["session_id"] = json!("s1");
        assert_eq!(record_events(&path, &[event]).unwrap(), (1, 1));

        let rows = load_journal(&path).unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!((row.input, row.output), (1200, 300));
        assert_eq!((row.cache_write, row.reasoning), (50, 20));
        assert_eq!(row.session_id.as_deref(), Some("s1"));
        // Normalised as the collectors do it, or `--project /work/app` would not find the row.
        assert_eq!(row.project.as_deref(), Some("/work/app"));
        assert_eq!(row.cost, None);
    }

    /// The rule the README states for Cursor, as code: a tool that keeps no counts cannot be
    /// journaled, because an absent count is refused rather than read as `0` -- and `0` output
    /// tokens prices as a confident, low, wrong number.
    #[test]
    fn an_event_without_token_counts_is_refused_not_stored_as_zero() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        for missing in ["input_tokens", "output_tokens"] {
            let mut event = usage_event("a1");
            event.as_object_mut().unwrap().remove(missing);
            let error = record_events(&path, &[event]).unwrap_err().to_string();
            assert!(error.contains(missing), "{error}");
            assert!(error.contains("do not estimate"), "{error}");
        }
        for junk in [json!("1200"), json!(-1), json!(1.5)] {
            let mut event = usage_event("a1");
            event["output_tokens"] = junk;
            assert!(record_events(&path, &[event]).is_err());
        }
        assert!(!path.exists(), "a refused event must not create a journal");
        // An explicit zero is a measurement, and is kept.
        let mut event = usage_event("a1");
        event["output_tokens"] = json!(0);
        assert_eq!(record_events(&path, &[event]).unwrap(), (1, 1));
    }

    #[test]
    fn one_bad_event_refuses_the_whole_batch_and_creates_no_journal() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        let mut bad = usage_event("a2");
        bad["cach_read_tokens"] = json!(4);
        let error = record_events(&path, &[usage_event("a1"), bad])
            .unwrap_err()
            .to_string();
        // Named by position and by key, because whoever wrote the adapter reads only this.
        assert!(
            error.contains("#2") && error.contains("cach_read_tokens"),
            "{error}"
        );
        assert!(
            !path.exists(),
            "the good half of a refused batch was written"
        );
    }

    #[test]
    fn an_event_with_neither_id_nor_time_is_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        let mut event = usage_event("a1");
        event.as_object_mut().unwrap().remove("event_id");
        event.as_object_mut().unwrap().remove("created");
        let error = record_events(&path, &[event.clone()])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("event_id") && error.contains("created"),
            "{error}"
        );
        // Either one is an identity.
        event["created"] = json!(1_758_000_000);
        assert_eq!(record_events(&path, &[event.clone()]).unwrap(), (1, 1));
        assert_eq!(record_events(&path, &[event]).unwrap(), (0, 1));
    }

    #[test]
    fn a_reported_cost_needs_a_cost_and_a_subscription_forbids_one() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        let with = |key: &str, value: Value| {
            let mut event = usage_event("a1");
            event[key] = value;
            event
        };
        let refused = |event: Value| record_events(&path, &[event]).unwrap_err().to_string();

        assert!(refused(with("cost_status", json!("reported"))).contains("needs the `cost`"));
        // The adapter cannot make this tool vouch for arithmetic it never saw.
        for status in ["estimated", "calculated", "free", "local", "quota"] {
            assert!(refused(with("cost_status", json!(status))).contains("cannot be supplied"));
        }
        let mut both = with("billing", json!("subscription"));
        both["cost"] = json!(0.5);
        assert!(refused(both).contains("cannot carry a `cost`"));
        assert!(refused(with("cost", json!(-1.0))).contains("`cost`"));
        assert!(refused(with("billing", json!("plan"))).contains("`billing`"));
        assert!(!path.exists());

        // A cost the tool recorded is `reported`, with or without saying so.
        assert_eq!(
            record_events(&path, &[with("cost", json!(0.25))]).unwrap(),
            (1, 1)
        );
        let row = &load_journal(&path).unwrap()[0];
        assert_eq!(row.cost, Some(0.25));
        assert_eq!(row.cost_status, CostStatus::ProviderReported);
    }

    /// Plan-billed work is *stored* as `quota` so an older build neither prices nor budgets it,
    /// and *read* as the unpriced subscription row a native collector emits -- the shape the
    /// pricing pass turns into `quota` with a list-rate figure beside it.
    #[test]
    fn a_subscription_event_is_stored_as_quota_and_read_as_a_subscription_row() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        let mut event = usage_event("a1");
        event["billing"] = json!("subscription");
        record_events(&path, &[event]).unwrap();

        let stored: String = Connection::open(&path)
            .unwrap()
            .query_row("SELECT cost_status FROM usage_event", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, "quota");
        let row = &load_journal(&path).unwrap()[0];
        assert_eq!(row.billing, Billing::Subscription);
        assert_eq!(row.cost_status, CostStatus::Unavailable);
        assert_eq!(row.cost, None);
    }

    /// Identities are one namespace across every source, so an adapter's `1` is stored under its
    /// provider: two tools that both count from one are two events, not one.
    #[test]
    fn a_supplied_event_id_is_namespaced_by_provider() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        let mut other = usage_event("1");
        other["provider"] = json!("othertool");
        assert_eq!(
            record_events(&path, &[usage_event("1"), other]).unwrap(),
            (2, 2)
        );
        let ids: Vec<Option<String>> = load_journal(&path)
            .unwrap()
            .into_iter()
            .map(|row| row.event_id)
            .collect();
        assert!(ids.contains(&Some("event:aider:1".to_string())), "{ids:?}");
        assert!(
            ids.contains(&Some("event:othertool:1".to_string())),
            "{ids:?}"
        );
    }

    #[test]
    fn replaying_a_batch_records_nothing_new() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        let batch = [usage_event("a1"), usage_event("a2"), usage_event("a1")];
        // The third is the first again: counted as sent, not as recorded.
        assert_eq!(record_events(&path, &batch).unwrap(), (2, 3));
        assert_eq!(record_events(&path, &batch).unwrap(), (0, 3));
        assert_eq!(load_journal(&path).unwrap().len(), 2);
    }

    /// The three columns are additive. A v0.18.0 journal gains them on the first write and keeps
    /// its row; read *before* any write -- which is what a dashboard does to a journal only an
    /// older hook has touched -- it reads as it always did.
    #[test]
    fn a_journal_without_the_new_columns_is_migrated_and_an_old_shape_still_reads() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        pre_record_event_journal(&path);

        let before = load_journal(&path).unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(
            (before[0].project.clone(), before[0].session_id.clone()),
            (None, None)
        );
        assert_eq!(before[0].billing, Billing::PerToken);

        let mut event = usage_event("a1");
        event["project"] = json!("/work/app");
        record_events(&path, &[event]).unwrap();
        let after = load_journal(&path).unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].event_id.as_deref(), Some("ollama:old"));
        assert_eq!(after[1].project.as_deref(), Some("/work/app"));

        // And an older build's writer, which names thirteen columns, still writes into it.
        Connection::open(&path)
            .unwrap()
            .execute(
                "INSERT INTO usage_event (event_id, provider, model, category, cost_status, requests, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens, cost, created) VALUES ('ollama:older-writer', 'ollama', 'qwen3', 'LOCAL', 'local', 1, 1, 1, 0, 0, 0, NULL, 200)",
                [],
            )
            .unwrap();
        assert_eq!(load_journal(&path).unwrap().len(), 3);
        let version: i64 = Connection::open(&path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            version, 1,
            "an additive change must not lock older writers out"
        );
    }

    #[test]
    fn a_write_stamps_the_schema_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        journal_connection(&path).unwrap();
        let version: i64 = Connection::open(&path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, JOURNAL_SCHEMA_VERSION);
    }

    /// A journal from a newer build may have a shape this one would damage. Refusing names why and
    /// what to do, and leaves the file as it was.
    #[test]
    fn a_journal_from_a_newer_build_is_refused_and_left_untouched() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("usage.db");
        Connection::open(&path)
            .unwrap()
            .execute_batch(&format!(
                "PRAGMA user_version = {}",
                JOURNAL_SCHEMA_VERSION + 1
            ))
            .unwrap();

        for error in [
            journal_connection(&path).err(),
            record_routing_event(&path, &serde_json::json!({"agent": "a", "model": "m"})).err(),
        ] {
            let message = error.expect("a newer journal is refused").to_string();
            assert!(message.contains("newer ai-usage-tui"), "{message}");
        }
        let tables: i64 = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0, "the refused writers created nothing");
    }

    /// Parallel subagents fire parallel hooks, each opening the journal to write. The migration
    /// was probe-then-`ALTER` with no lock held between the two, so writers that probed together
    /// all saw the column missing, and every one but the first died on "duplicate column name".
    #[test]
    fn writers_opening_an_unmigrated_journal_together_all_succeed() {
        for round in 0..20 {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("usage.db");
            // Both shapes a writer can still meet: the oldest, and the one v0.18.0 left, which
            // has `event_id` and lacks what `--record-event` added.
            if round % 2 == 0 {
                pre_event_id_journal(&path);
            } else {
                pre_record_event_journal(&path);
            }
            let writers = 8;
            let start = Arc::new(Barrier::new(writers));
            let handles: Vec<_> = (0..writers)
                .map(|_| {
                    let path = path.clone();
                    let start = Arc::clone(&start);
                    std::thread::spawn(move || {
                        start.wait();
                        journal_connection(&path)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    })
                })
                .collect();
            for handle in handles {
                let outcome = handle.join().unwrap();
                assert!(
                    outcome.is_ok(),
                    "round {round}: a writer failed: {outcome:?}"
                );
            }
        }
    }
}
