use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use chrono::DateTime;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::classify::classify;
use crate::helpers::{number, string};
use crate::model::{Category, CostStatus, Usage};
use crate::utils::db_path;

/// A resume point for incremental reads: the highest `time_created` already ingested.
///
/// `Cursor::start()` reads everything. Subsequent polls resume from the last high-water mark,
/// inclusively — boundary rows are re-read on purpose and dropped by `event_id` deduplication,
/// which is cheaper and safer than risking rows written within the same clock tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor(Option<i64>);

impl Cursor {
    pub fn start() -> Self {
        Self(None)
    }

    pub fn high_water(self) -> Option<i64> {
        self.0
    }

    fn advance(&mut self, raw_time_created: i64) {
        self.0 = Some(match self.0 {
            Some(current) => current.max(raw_time_created),
            None => raw_time_created,
        });
    }
}

pub fn load_opencode(override_path: Option<&Path>) -> Result<(Vec<Usage>, String)> {
    let (usages, source, _) = load_opencode_since(override_path, Cursor::start())?;
    Ok((usages, source))
}

/// Read messages at or after `cursor`, returning the advanced cursor.
///
/// The collector previously re-read and re-parsed every row of the whole table on every poll,
/// so ingestion cost grew without bound as history accumulated.
pub fn load_opencode_since(
    override_path: Option<&Path>,
    mut cursor: Cursor,
) -> Result<(Vec<Usage>, String, Cursor)> {
    let path = override_path
        .map(Path::to_path_buf)
        .or_else(db_path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine a home directory; set OPENCODE_DB_PATH or pass --db"
            )
        })?;
    if !path.exists() {
        return Ok((
            Vec::new(),
            format!("No OpenCode database at {}", path.display()),
            cursor,
        ));
    }
    let conn = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(Duration::from_millis(250))?;
    let has_messages: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'message')",
        [],
        |row| row.get(0),
    )?;
    if !has_messages {
        return Ok((
            Vec::new(),
            format!("No message table in {}", path.display()),
            cursor,
        ));
    }
    let read = |row: &rusqlite::Row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?));
    let rows: Vec<(String, i64)> = match cursor.high_water() {
        Some(since) => conn
            .prepare("SELECT data, time_created FROM message WHERE time_created >= ?1")?
            .query_map([since], read)?
            .collect::<rusqlite::Result<_>>()?,
        None => conn
            .prepare("SELECT data, time_created FROM message")?
            .query_map([], read)?
            .collect::<rusqlite::Result<_>>()?,
    };
    let mut usages = Vec::new();
    for (raw, created) in rows {
        // Advance on every row, including ones we skip below: a non-assistant row still marks
        // history we have seen, and not advancing past it would re-read it forever.
        cursor.advance(created);
        let json: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let info = json.get("info").unwrap_or(&json);
        if info.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let provider =
            string(info, &["providerID", "provider_id"]).unwrap_or_else(|| "unknown".into());
        let model = string(info, &["modelID", "model_id"]).unwrap_or_else(|| "unknown".into());
        let tokens = info.get("tokens").unwrap_or(&Value::Null);
        let cache = tokens.get("cache").unwrap_or(&Value::Null);
        let mut category = classify(&provider, &model);
        let cost = info.get("cost").and_then(Value::as_f64);
        if category == Category::Unknown && cost.map(|value| value > 0.0).unwrap_or(false) {
            category = Category::Paid;
        }
        let cost_status = if category == Category::Local {
            CostStatus::Local
        } else if category == Category::Free {
            CostStatus::Free
        } else {
            match info.get("cost_source").and_then(Value::as_str) {
                Some("provider_reported") if cost.is_some() => CostStatus::ProviderReported,
                Some("calculated") if cost.is_some() => CostStatus::Calculated,
                Some("estimated") if cost.is_some() => CostStatus::Estimated,
                _ if category == Category::Paid && cost.is_some() => CostStatus::Calculated,
                // Only in the fallback, and only when there is no positive figure: if OpenCode
                // reported real spend for a cloud row, observed data beats the policy rule.
                _ if category == Category::Cloud
                    && !cost.map(|value| value > 0.0).unwrap_or(false) =>
                {
                    CostStatus::Quota
                }
                _ => CostStatus::Unavailable,
            }
        };
        // A recorded `0` on a quota-billed row is OpenCode having no cost data for cloud
        // routes, not an authoritative "this was free". Exporting it as 0 would state the very
        // thing this status exists to avoid stating.
        let cost = if cost_status == CostStatus::Quota {
            None
        } else {
            cost
        };
        let usage = Usage {
            event_id: string(info, &["id", "messageID", "message_id"]),
            provider: provider.clone(),
            model: model.clone(),
            category,
            requests: 1,
            input: number(tokens, &["input", "inputTokens"]),
            output: number(tokens, &["output", "outputTokens"]),
            reasoning: number(tokens, &["reasoning", "reasoningTokens"]),
            cache_read: number(cache, &["read", "readTokens"]),
            cache_write: number(cache, &["write", "writeTokens"]),
            cost,
            cost_status,
            created: timestamp_seconds(created),
            session_id: string(info, &["sessionID", "session_id"]),
            project: None,
        };
        usages.push(usage);
    }
    Ok((usages, format!("OpenCode: {}", path.display()), cursor))
}

pub fn timestamp_seconds(value: i64) -> i64 {
    if value > 100_000_000_000_000 {
        value / 1_000_000
    } else if value > 100_000_000_000 {
        value / 1_000
    } else {
        value
    }
}

pub fn parse_created_at(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_units_are_normalized() {
        assert_eq!(timestamp_seconds(1_700_000_000), 1_700_000_000);
        assert_eq!(timestamp_seconds(1_700_000_000_000), 1_700_000_000);
        assert_eq!(timestamp_seconds(1_700_000_000_000_000), 1_700_000_000);
    }

    /// Seed one row with an explicit provider/model/cost, for the quota cases below.
    fn seed_row(path: &std::path::Path, provider: &str, model: &str, cost: &str) {
        let conn = rusqlite::Connection::open(path).expect("create db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS message (data TEXT NOT NULL, time_created INTEGER NOT NULL);",
        )
        .expect("create schema");
        let data = format!(
            r#"{{"info":{{"id":"r1","role":"assistant","providerID":"{provider}",
               "modelID":"{model}","cost":{cost},
               "tokens":{{"input":100,"output":50,"cache":{{"read":0,"write":0}}}}}}}}"#
        );
        conn.execute(
            "INSERT INTO message (data, time_created) VALUES (?1, ?2)",
            rusqlite::params![data, 1_700_000_000_i64],
        )
        .expect("insert message");
    }

    #[test]
    fn a_quota_billed_row_carries_no_cost_at_all() {
        // OpenCode records `0` for cloud routes because it has no figure for them, not because
        // the call was free. Keeping that zero exported `cost: 0` — "this cost zero dollars" —
        // which is the exact claim the quota status exists to avoid making.
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("opencode.db");
        seed_row(&db, "ollama", "glm-5.2:cloud", "0");

        let (usages, _) = load_opencode(Some(&db)).expect("load");
        assert_eq!(usages[0].cost_status, CostStatus::Quota);
        assert_eq!(
            usages[0].cost, None,
            "a recorded zero on a quota-billed row is absence of data, not a price"
        );
    }

    #[test]
    fn a_cloud_row_with_real_reported_spend_keeps_its_figure() {
        // The guard on the rule above: observed data beats the "cloud is quota-billed" policy.
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("opencode.db");
        seed_row(&db, "ollama", "glm-5.2:cloud", "0.42");

        let (usages, _) = load_opencode(Some(&db)).expect("load");
        assert_ne!(
            usages[0].cost_status,
            CostStatus::Quota,
            "a cloud row with a real cost is not quota-billed"
        );
        assert_eq!(usages[0].cost, Some(0.42));
    }

    fn seed_db(path: &std::path::Path, rows: &[(&str, i64)]) {
        let conn = rusqlite::Connection::open(path).expect("create db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS message (data TEXT NOT NULL, time_created INTEGER NOT NULL);",
        )
        .expect("create schema");
        for (id, created) in rows {
            let data = format!(
                r#"{{"info":{{"id":"{}","role":"assistant","providerID":"opencode",
                   "modelID":"claude-sonnet-4.6","cost":0.01,
                   "tokens":{{"input":100,"output":50,"cache":{{"read":0,"write":0}}}}}}}}"#,
                id
            );
            conn.execute(
                "INSERT INTO message (data, time_created) VALUES (?1, ?2)",
                rusqlite::params![data, created],
            )
            .expect("insert message");
        }
    }

    #[test]
    fn a_cursor_read_returns_only_new_rows() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("opencode.db");
        seed_db(&db, &[("m1", 1_700_000_000), ("m2", 1_700_000_060)]);

        // Cold start reads everything.
        let (first, _, cursor) = load_opencode_since(Some(&db), Cursor::start()).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(cursor.high_water(), Some(1_700_000_060));

        // A poll with no new rows re-reads only the inclusive boundary, not the whole table.
        // Before the cursor existed this returned all 2 rows -- and all N on a real database.
        let (second, _, cursor) = load_opencode_since(Some(&db), cursor).unwrap();
        assert_eq!(
            second.len(),
            1,
            "second poll re-read more than the boundary"
        );
        assert_eq!(second[0].event_id.as_deref(), Some("m2"));

        // New rows are picked up.
        seed_db(&db, &[("m3", 1_700_000_120)]);
        let (third, _, cursor) = load_opencode_since(Some(&db), cursor).unwrap();
        assert_eq!(third.len(), 2, "expected the boundary row plus the new one");
        assert!(third.iter().any(|u| u.event_id.as_deref() == Some("m3")));
        assert_eq!(cursor.high_water(), Some(1_700_000_120));
    }

    #[test]
    fn boundary_rows_reread_by_the_cursor_are_deduplicated() {
        // The cursor is deliberately inclusive, so correctness depends on `event_id` dedup
        // absorbing the overlap rather than double-counting it.
        use crate::collector::usage_key;
        use std::collections::HashSet;

        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("opencode.db");
        seed_db(&db, &[("m1", 1_700_000_000), ("m2", 1_700_000_060)]);

        let (first, _, cursor) = load_opencode_since(Some(&db), Cursor::start()).unwrap();
        let (second, _, _) = load_opencode_since(Some(&db), cursor).unwrap();

        let mut seen = HashSet::new();
        let unique = first
            .iter()
            .chain(second.iter())
            .filter(|u| seen.insert(usage_key(u)))
            .count();
        assert_eq!(unique, 2, "boundary overlap was double-counted");
    }

    #[test]
    fn event_ids_are_captured_from_opencode_messages() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("opencode.db");
        seed_db(&db, &[("msg_abc", 1_700_000_000)]);
        let (usages, _, _) = load_opencode_since(Some(&db), Cursor::start()).unwrap();
        assert_eq!(usages[0].event_id.as_deref(), Some("msg_abc"));
    }
}
