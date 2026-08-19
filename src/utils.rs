use chrono::Datelike;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, path::PathBuf};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn format_clock() -> String {
    chrono::Local::now().format("%H:%M:%S %Z").to_string()
}

/// Start of the current calendar day, in the machine's local timezone.
///
/// Calendar boundaries must be local: a UTC midnight cutoff puts a user's evening work in
/// "tomorrow" and makes the dashboard's daily total disagree with their own sense of the day.
pub fn local_day_start() -> i64 {
    use chrono::{Local, TimeZone};
    let now = Local::now();
    Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .map(|dt| dt.timestamp())
        // A DST transition can make local midnight ambiguous or nonexistent; fall back to a
        // rolling 24h window rather than panicking on one day of the year.
        .unwrap_or_else(|| now.timestamp() - 86_400)
}

/// Start of the current calendar month, in the machine's local timezone.
pub fn local_month_start() -> i64 {
    use chrono::{Local, TimeZone};
    let now = Local::now();
    Local
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .map(|dt| dt.timestamp())
        .unwrap_or_else(|| now.timestamp() - 30 * 86_400)
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

/// The user's home directory.
///
/// Windows does not set `HOME`. Relying on it alone made every path lookup fail there with
/// "HOME is not set" — on a platform for which this project ships Scoop and Chocolatey
/// packages.
pub fn home_dir() -> Option<PathBuf> {
    if let Some(home) = env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home));
    }
    if let Some(profile) = env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(profile));
    }
    let drive = env::var_os("HOMEDRIVE").filter(|v| !v.is_empty())?;
    let path = env::var_os("HOMEPATH").filter(|v| !v.is_empty())?;
    let mut combined = PathBuf::from(drive);
    combined.push(PathBuf::from(path));
    Some(combined)
}

/// Root for user data, honouring `XDG_DATA_HOME`, then `%LOCALAPPDATA%`, then `~/.local/share`.
pub fn data_root() -> Option<PathBuf> {
    if let Some(xdg) = env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg));
    }
    if cfg!(windows) {
        if let Some(local) = env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(local));
        }
    }
    Some(home_dir()?.join(".local").join("share"))
}

/// Root for user config, honouring `XDG_CONFIG_HOME`, then `%APPDATA%`, then `~/.config`.
pub fn config_root() -> Option<PathBuf> {
    if let Some(xdg) = env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg));
    }
    if cfg!(windows) {
        if let Some(roaming) = env::var_os("APPDATA").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(roaming));
        }
    }
    Some(home_dir()?.join(".config"))
}

pub fn db_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("OPENCODE_DB_PATH") {
        return Some(PathBuf::from(path));
    }
    Some(data_root()?.join("opencode").join("opencode.db"))
}

pub fn data_dir() -> Option<PathBuf> {
    Some(data_root()?.join("ai-usage-tui"))
}

pub fn journal_path() -> Option<PathBuf> {
    if let Ok(path) = env::var("AI_USAGE_JOURNAL_PATH") {
        return Some(PathBuf::from(path));
    }
    Some(data_dir()?.join("usage.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_formatting_scales_by_magnitude() {
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1_500), "1.5K");
        assert_eq!(format_count(2_500_000), "2.5M");
        assert_eq!(format_count(3_000_000_000), "3.0B");
    }

    #[test]
    fn local_day_start_precedes_now_by_less_than_a_day() {
        let start = local_day_start();
        assert!(start <= now());
        assert!(now() - start < 86_400 + 3_600);
    }

    #[test]
    fn local_month_start_precedes_local_day_start() {
        assert!(local_month_start() <= local_day_start());
    }

    #[test]
    fn env_overrides_take_precedence_over_derived_paths() {
        // Serialised implicitly: these two vars are read only by the functions under test.
        std::env::set_var("OPENCODE_DB_PATH", "/tmp/explicit-opencode.db");
        std::env::set_var("AI_USAGE_JOURNAL_PATH", "/tmp/explicit-journal.db");
        assert_eq!(
            db_path().unwrap(),
            PathBuf::from("/tmp/explicit-opencode.db")
        );
        assert_eq!(
            journal_path().unwrap(),
            PathBuf::from("/tmp/explicit-journal.db")
        );
        std::env::remove_var("OPENCODE_DB_PATH");
        std::env::remove_var("AI_USAGE_JOURNAL_PATH");
    }
}
