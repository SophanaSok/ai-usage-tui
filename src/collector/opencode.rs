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

pub fn load_opencode(override_path: Option<&Path>) -> Result<(Vec<Usage>, String)> {
    let path = override_path
        .map(Path::to_path_buf)
        .or_else(db_path)
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    if !path.exists() {
        return Ok((
            Vec::new(),
            format!("No OpenCode database at {}", path.display()),
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
        ));
    }
    let mut stmt = conn.prepare("SELECT data, time_created FROM message")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut usages = Vec::new();
    for row in rows {
        let (raw, created) = row?;
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
                _ => CostStatus::Unavailable,
            }
        };
        let usage = Usage {
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
        };
        usages.push(usage);
    }
    Ok((usages, format!("OpenCode: {}", path.display())))
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
}
