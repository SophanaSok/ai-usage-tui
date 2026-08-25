use std::{fs, io, io::Read, path::Path, time::Duration};

use anyhow::Result;
use rusqlite::{params, Connection, OpenFlags};
use serde_json::Value;

use crate::classify::{category_from_label, classify, cost_status_from_label};
use crate::collector::background::Collector;
use crate::collector::opencode::parse_created_at;
use crate::helpers::{number, string};
use crate::model::{Category, CostStatus, RoutingEvent, Usage};
use crate::utils::now;
use std::path::PathBuf;

/// This source's canonical id: the `Collector::name()` it reports, the
/// `[collectors.<id>]` table that configures it, and its key in the source registry.
/// One constant so those can never drift apart.
pub const ID: &str = "journal";

pub fn load_journal(path: &Path) -> Result<Vec<Usage>> {
    if !path.exists() {
        return Ok(Vec::new());
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
        return Ok(Vec::new());
    }
    // A journal written by an older build has no `event_id` column, and this is a read-only
    // path that cannot migrate it. Select the column only when it actually exists.
    let has_event_id: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('usage_event') WHERE name = 'event_id')",
        [],
        |row| row.get(0),
    )?;
    let mut stmt = conn.prepare(if has_event_id {
        "SELECT provider, model, category, cost_status, requests, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens, cost, created, event_id FROM usage_event"
    } else {
        "SELECT provider, model, category, cost_status, requests, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens, cost, created, NULL AS event_id FROM usage_event"
    })?;
    let rows = stmt.query_map([], |row| {
        let category: String = row.get(2)?;
        let cost_status: String = row.get(3)?;
        Ok(Usage {
            event_id: row.get(12).ok().flatten(),
            provider: row.get(0)?,
            model: row.get(1)?,
            category: category_from_label(&category),
            cost_status: cost_status_from_label(&cost_status),
            billing: Default::default(),
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
            cost: row.get(10)?,
            created: row.get(11)?,
            session_id: None,
            project: None,
        })
    })?;
    Ok(rows.filter_map(Result::ok).collect())
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

pub fn record_ollama(path: &Path) -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let mut events = Vec::new();
    let mut invalid_lines = 0;
    for line in input.lines().filter(|line| !line.trim().is_empty()) {
        if let Ok(json) = serde_json::from_str::<Value>(line) {
            events.push(json);
        } else {
            invalid_lines += 1;
        }
    }
    if events.is_empty() {
        let json: Value = serde_json::from_str(&input)?;
        events.push(json);
        invalid_lines = 0;
    }

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("journal path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(250))?;
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
            created INTEGER NOT NULL
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
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS usage_event_event_id ON usage_event(event_id)",
        [],
    )?;

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
        let category = classify("ollama", &model);
        let cost_status = match category {
            Category::Local => CostStatus::Local,
            // Billed on quota, not per token. This arm was already singled out and then
            // collapsed into the fallback, which is how the distinction got lost.
            Category::Cloud => CostStatus::Quota,
            _ => CostStatus::Unavailable,
        };
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
            .unwrap_or_else(now);
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO usage_event (event_id, provider, model, category, cost_status, requests, input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens, cost, created) VALUES (?1, 'ollama', ?2, ?3, ?4, 1, ?5, ?6, 0, 0, 0, NULL, ?7)",
            params![
                event_id,
                model,
                category.label(),
                cost_status.label(),
                stored(number(&json, &["prompt_eval_count"])),
                stored(number(&json, &["eval_count"])),
                created,
            ],
        )?;
        recorded += inserted;
    }
    if invalid_lines > 0 {
        eprintln!("Skipped {} malformed Ollama JSON line(s)", invalid_lines);
    }
    println!(
        "Recorded {} Ollama usage event(s) in {}",
        recorded,
        path.display()
    );
    Ok(())
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
        "SELECT task, phase, agent, model, provider, category, cost_status, requests, tokens, cost, retries, escalations, test_result, review_defects, created FROM routing_event",
    )?;
    let rows = stmt.query_map([], |row| {
        let category: String = row.get(5)?;
        let cost_status: String = row.get(6)?;
        let test_result: Option<i64> = row.get(12)?;
        let cost: Option<f64> = row.get(9)?;
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
    })?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn record_routing(path: &Path) -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let json: Value = serde_json::from_str(&input)?;
    let inserted = record_routing_event(path, &json)?;
    println!(
        "Recorded {} routing event(s) in {}",
        inserted,
        path.display()
    );
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
        .unwrap_or_else(now);

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

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("journal path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(250))?;
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
    allow_unreported_counters(&conn)?;
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS routing_event_event_id ON routing_event(event_id)",
        [],
    )?;

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
/// SQLite cannot alter a constraint in place, so the table is rebuilt: the documented way, in one
/// transaction, with the index recreated after. Rows already there keep their zeros. An omitted
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
        "BEGIN;
         CREATE TABLE routing_event_rebuilt ({ROUTING_EVENT_COLUMNS});
         INSERT INTO routing_event_rebuilt ({COLUMNS}) SELECT {COLUMNS} FROM routing_event;
         DROP TABLE routing_event;
         ALTER TABLE routing_event_rebuilt RENAME TO routing_event;
         CREATE UNIQUE INDEX IF NOT EXISTS routing_event_event_id ON routing_event(event_id);
         COMMIT;"
    ))?;
    Ok(())
}

/// A per-task counter as the emitter sent it.
///
/// Absent or `null` is "not reported", which is not `0`. Anything else must be a non-negative
/// integer: a string or a negative number silently becoming `0` — which is what the old
/// `number()` default did — is a silent failure, and those reach the user as errors (convention 8).
fn counter(json: &Value, key: &str) -> Result<Option<u32>> {
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
        .and_then(|n| u32::try_from(n).ok())
        .map(Some)
        .ok_or_else(|| {
            anyhow::anyhow!("routing event: `{key}` must be a non-negative integer, got {value}")
        })
}

/// A non-negative whole quantity — `tokens`, `requests` — where absent means `0`.
///
/// The counters above distinguish absent from zero; these do not need to, but they were the two
/// fields still going through `helpers::number`, which maps a string or a negative to `0` and
/// reports success. One rule for the whole event.
fn quantity(json: &Value, key: &str) -> Result<u64> {
    let Some(value) = json.get(key) else {
        return Ok(0);
    };
    if value.is_null() {
        return Ok(0);
    }
    value
        .as_u64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                .map(|f| f as u64)
        })
        .ok_or_else(|| {
            anyhow::anyhow!("routing event: `{key}` must be a non-negative integer, got {value}")
        })
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

pub struct JournalCollector {
    pub journal_path: PathBuf,
    pub interval_secs: u64,
}

impl Collector for JournalCollector {
    fn name(&self) -> &str {
        ID
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        load_journal(&self.journal_path)
    }
}

/// One-shot read for the source registry.
pub(crate) fn read(
    roots: &crate::collector::SourceRoots,
) -> crate::collector::registry::SourceRead {
    let usages = load_journal(&roots.journal)?;
    let present = roots.journal.exists();
    Ok((
        crate::collector::SourceReport {
            id: ID,
            present,
            path: Some(roots.journal.clone()),
            rows: usages.len(),
            status: if present {
                format!("journal: {}", roots.journal.display())
            } else {
                "journal: not initialized".to_string()
            },
            detail: None,
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
        // An identity is still unique after the rebuild.
        assert_eq!(
            record_routing_event(&journal, &event("new-task", 2)).expect("record"),
            0
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
}
