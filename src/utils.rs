use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, path::PathBuf};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn format_clock() -> String {
    let secs = now() % 86_400;
    format!(
        "{:02}:{:02}:{:02} UTC",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

pub fn format_count(value: u64) -> String {
    if value >= 1_000_000_000 {
        format!("{:.1}B", value as f64 / 1_000_000_000.0)
    } else if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

pub fn db_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("OPENCODE_DB_PATH") {
        return Some(PathBuf::from(path));
    }
    let home = env::var_os("HOME")?;
    let data = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".local/share"));
    Some(data.join("opencode/opencode.db"))
}

pub fn data_dir() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(
        env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".local/share"))
            .join("ai-usage-tui"),
    )
}

pub fn journal_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("AI_USAGE_JOURNAL_PATH") {
        return Some(PathBuf::from(path));
    }
    Some(data_dir()?.join("usage.db"))
}
