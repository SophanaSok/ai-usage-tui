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

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("journal path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(250))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS routing_event (
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
            retries INTEGER NOT NULL,
            escalations INTEGER NOT NULL,
            test_result INTEGER,
            review_defects INTEGER NOT NULL,
            created INTEGER NOT NULL
        );",
    )?;
    let has_event_id: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('routing_event') WHERE name = 'event_id')",
        [],
        |row| row.get(0),
    )?;
    if !has_event_id {
        conn.execute("ALTER TABLE routing_event ADD COLUMN event_id TEXT", [])?;
    }
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS routing_event_event_id ON routing_event(event_id)",
        [],
    )?;

    let agent = string(&json, &["agent"]).unwrap_or_else(|| "unknown".to_string());
    let model = string(&json, &["model"]).unwrap_or_else(|| "unknown".to_string());
    let task = string(&json, &["task"]).unwrap_or_default();
    let created = json
        .get("created")
        .and_then(Value::as_i64)
        .unwrap_or_else(now);

    let event_id = format!("routing:{}:{}:{}:{}", agent, model, task, created);

    let category_label = string(&json, &["category"]).unwrap_or_else(|| "UNKNOWN".to_string());
    let cost_status_label =
        string(&json, &["cost_status"]).unwrap_or_else(|| "unavailable".to_string());
    let test_result = json.get("test_result").and_then(|v| match v {
        Value::Bool(b) => Some(*b),
        Value::Null => None,
        _ => v.as_i64().map(|n| n != 0),
    });
    let cost = json.get("cost").and_then(|v| match v {
        Value::Null => None,
        _ => v.as_f64(),
    });

    let inserted = conn.execute(
        "INSERT OR IGNORE INTO routing_event (event_id, task, phase, agent, model, provider, category, cost_status, requests, tokens, cost, retries, escalations, test_result, review_defects, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            event_id,
            task,
            string(&json, &["phase"]).unwrap_or_default(),
            agent,
            model,
            string(&json, &["provider"]).unwrap_or_else(|| "unknown".to_string()),
            category_label,
            cost_status_label,
            stored(number(&json, &["requests"]).max(1)),
            stored(number(&json, &["tokens"])),
            cost,
            number(&json, &["retries"]) as u32,
            number(&json, &["escalations"]) as u32,
            test_result.map(|b| b as i64),
            number(&json, &["review_defects"]) as u32,
            created,
        ],
    )?;
    println!(
        "Recorded {} routing event(s) in {}",
        inserted,
        path.display()
    );
    Ok(())
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
