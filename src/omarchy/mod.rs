//! Omarchy's agents panel, read-only.
//!
//! Omarchy 4 ships a bar panel that meters every AI coding subscription on the machine: for
//! each agent, one JSON record under `${XDG_STATE_HOME:-~/.local/state}/omarchy/agents/usage/`
//! carrying the plan label, the percentage of each rate-limit window used, and when it resets.
//! Those numbers come from the vendors' own endpoints, with the agent's saved sign-in — a call
//! this tool deliberately never makes and a credential it never reads.
//!
//! This module reads the finished display records instead. Only six fields are consumed:
//! `id`, `name`, `updatedAt`, `ready`, `tierLabel`, `usageStatusText`, and the `limits` list.
//! The record also carries token tallies and help text; none of it is deserialised. Nothing is
//! ever written here, and on a machine without Omarchy the directory is simply absent — that is
//! an idle panel, not an error.

pub mod record;

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Omarchy refreshes its records every 15 minutes by default. Three missed refreshes is the
/// same allowance the collector health machinery gives a source before calling it stale.
pub const STALE_AFTER_SECS: i64 = 3 * 900;

/// Windows this full or fuller are drawn in the alarm colour, matching Omarchy's own panel.
pub const ALARM_FRACTION: f64 = 0.9;

/// The part of an Omarchy usage record this tool reads. Unknown fields are ignored, so the
/// record's token tallies, help text and any later additions never enter this process.
#[derive(Debug, Default, Deserialize)]
pub struct RecordHeader {
    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: Option<String>,
    pub ready: Option<bool>,
    #[serde(rename = "tierLabel")]
    pub tier_label: Option<String>,
    #[serde(rename = "usageStatusText")]
    pub usage_status_text: Option<String>,
    pub limits: Option<Vec<LimitEntry>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LimitEntry {
    pub label: Option<String>,
    /// Present on model-scoped windows, where the label alone would be misread — a model named
    /// "Opus 5 (1M context)" is not a one-minute window. Preferred over `label` when set.
    pub title: Option<String>,
    /// A fraction, 0..1. Negative means unknown; above 1 is clamped.
    pub percent: Option<f64>,
    #[serde(rename = "resetsAt")]
    pub resets_at: Option<String>,
}

pub fn read_record(path: &Path) -> anyhow::Result<RecordHeader> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

/// Omarchy names the Claude Code record `claude`; this tool's collector is `claude_code`.
pub fn record_id_for_agent(agent: &str) -> &str {
    match agent {
        "claude_code" => "claude",
        other => other,
    }
}

/// The plan label Omarchy already derived for an agent, if its record is present and names
/// one. This is what the billing detector consumes: no credential is touched to learn it.
pub fn tier_label_for(dir: &Path, agent: &str) -> Option<String> {
    let path = dir.join(format!("{}.json", record_id_for_agent(agent)));
    let header = read_record(&path).ok()?;
    header
        .tier_label
        .map(|tier| tier.trim().to_string())
        .filter(|tier| !tier.is_empty())
}

/// One rate-limit window as rendered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LimitWindow {
    pub label: String,
    /// Share of the window used, 0..1.
    pub fraction: f64,
    /// Unix timestamp the window resets at, when the record carries one that parses.
    pub resets_at: Option<i64>,
    /// Seconds until the reset relative to the `now` passed to `load_limits`; zero or negative
    /// once the window has rolled over, at which point the percentage describes a period that
    /// is finished.
    pub resets_in_secs: Option<i64>,
}

impl LimitWindow {
    pub fn percent_used(&self) -> f64 {
        self.fraction * 100.0
    }

    pub fn is_alarming(&self) -> bool {
        self.fraction >= ALARM_FRACTION && !self.has_reset()
    }

    pub fn has_reset(&self) -> bool {
        self.resets_in_secs.is_some_and(|secs| secs <= 0)
    }
}

/// One agent's record, reduced to what the panel shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LimitsSnapshot {
    pub agent: String,
    pub name: String,
    pub tier: String,
    /// Omarchy's own status line — "Sign-in expired", "Waiting for auth" — when the limits
    /// could not be fetched. Shown in place of the windows rather than hidden with them.
    pub status_text: String,
    pub updated_at: Option<i64>,
    pub age_secs: Option<i64>,
    /// Older than `stale_after`, or undated: the numbers describe some earlier moment.
    pub stale: bool,
    pub windows: Vec<LimitWindow>,
}

/// Everything found under the usage directory, plus what could not be read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LimitsReport {
    pub dir: PathBuf,
    /// Whether the directory exists at all. Absent is the normal state off Omarchy.
    pub present: bool,
    pub snapshots: Vec<LimitsSnapshot>,
    /// Records that exist but could not be parsed, named so a broken file is not mistaken for
    /// a quiet one.
    pub problems: Vec<String>,
}

impl LimitsReport {
    /// The fullest window across every agent — what a one-line summary should name.
    pub fn binding_window(&self) -> Option<(&LimitsSnapshot, &LimitWindow)> {
        self.snapshots
            .iter()
            .filter(|snapshot| !snapshot.stale)
            .flat_map(|snapshot| snapshot.windows.iter().map(move |w| (snapshot, w)))
            .filter(|(_, window)| !window.has_reset())
            .max_by(|a, b| a.1.fraction.total_cmp(&b.1.fraction))
    }
}

/// Read every record in `dir`. Pure over `now`, so tests never touch the wall clock.
pub fn load_limits(dir: &Path, now: i64, stale_after: i64) -> LimitsReport {
    let mut report = LimitsReport {
        dir: dir.to_path_buf(),
        ..Default::default()
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return report;
    };
    report.present = true;

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            // The updater writes `.agent.XXXXXX` temporaries beside the records before renaming
            // them into place; a half-written temp is not a record.
            path.is_file()
                && path.extension().is_some_and(|ext| ext == "json")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.'))
        })
        .collect();
    paths.sort();

    for path in paths {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("record")
            .to_string();
        let header = match read_record(&path) {
            Ok(header) => header,
            Err(error) => {
                crate::logging::warn("omarchy", &format!("{file_name}: {error}"));
                report.problems.push(format!("{file_name}: {error}"));
                continue;
            }
        };
        if let Some(snapshot) = snapshot(header, &file_name, now, stale_after) {
            report.snapshots.push(snapshot);
        }
    }
    report.snapshots.sort_by(|a, b| a.agent.cmp(&b.agent));
    report
}

fn snapshot(
    header: RecordHeader,
    file_name: &str,
    now: i64,
    stale_after: i64,
) -> Option<LimitsSnapshot> {
    let agent = header
        .id
        .clone()
        .unwrap_or_else(|| file_name.trim_end_matches(".json").to_string());
    let windows: Vec<LimitWindow> = header
        .limits
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            // Negative is Omarchy's "unknown"; a window with no figure has nothing to draw.
            let fraction = entry.percent.filter(|p| p.is_finite() && *p >= 0.0)?;
            let label = entry
                .title
                .or(entry.label)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())?;
            let resets_at = entry.resets_at.as_deref().and_then(parse_timestamp);
            Some(LimitWindow {
                label,
                fraction: fraction.min(1.0),
                resets_at,
                resets_in_secs: resets_at.map(|at| at - now),
            })
        })
        .collect();
    let status_text = header
        .usage_status_text
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    // A record with no windows and nothing to say about why is not worth a row: today that is
    // every agent Omarchy reports a balance for rather than limits.
    if windows.is_empty() && status_text.is_empty() {
        return None;
    }
    let updated_at = header.updated_at.as_deref().and_then(parse_timestamp);
    let age_secs = updated_at.map(|at| now - at);
    Some(LimitsSnapshot {
        name: header.name.unwrap_or_else(|| agent.clone()),
        agent,
        tier: header.tier_label.unwrap_or_default().trim().to_string(),
        status_text,
        updated_at,
        age_secs,
        // Undated or unparsable is stale, not fresh: an unknown age is no reason to trust it.
        // The rule lives in `limits::is_stale` because a second source now shares it, and
        // because it has to reject a *negative* age too -- see that function.
        stale: crate::limits::is_stale(age_secs, stale_after),
        windows,
    })
}

/// Omarchy writes Python `isoformat()` output with an offset, or a trailing `Z`. Both are
/// RFC 3339 once the `Z` is accepted, which `chrono` does.
///
/// Shared with `crate::limits`, whose Claude Code cache spells reset instants the same way.
/// Returning Unix **seconds** is the contract every caller depends on.
pub fn parse_timestamp(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/omarchy"
        ))
    }

    /// 2026-08-23T10:10:00Z: ten minutes after the fixture's `updatedAt`.
    const NOW: i64 = 1_787_479_800;

    #[test]
    fn a_record_is_reduced_to_its_windows_and_nothing_it_must_not_read() {
        let report = load_limits(&fixtures(), NOW, STALE_AFTER_SECS);
        assert!(report.present);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        // codex.json has no limits and no status: no row. claude.json has three usable windows;
        // the negative "Unknown window" is Omarchy's unknown and is dropped.
        assert_eq!(report.snapshots.len(), 1, "{:?}", report.snapshots);
        let claude = &report.snapshots[0];
        assert_eq!(claude.agent, "claude");
        assert_eq!(claude.name, "Claude Code");
        assert_eq!(claude.tier, "Max 20x");
        assert!(!claude.stale);
        assert_eq!(claude.age_secs, Some(600));
        let labels: Vec<&str> = claude.windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Session (5-hour)",
                "Weekly (7-day)",
                "Opus 5 (1M context) Weekly"
            ],
            "file order, title preferred over label"
        );
        assert!((claude.windows[0].fraction - 0.92).abs() < 1e-9);
        assert_eq!(claude.windows[0].resets_in_secs, Some(2 * 3600 + 3 * 60));

        let rendered = format!("{report:?}");
        for forbidden in ["authHelpText", "claude auth login", "modelUsage", "345678"] {
            assert!(
                !rendered.contains(forbidden),
                "{forbidden} was read: {rendered}"
            );
        }
    }

    #[test]
    fn a_percent_above_one_is_clamped_and_a_reset_that_passed_is_not_alarming() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("claude.json"),
            r#"{"id":"claude","updatedAt":"2026-08-23T10:00:00Z","limits":[{"label":"Over","percent":1.4,"resetsAt":"2026-08-27T09:00:00Z"},{"label":"Done","percent":0.95,"resetsAt":"2026-08-23T09:00:00Z"}]}"#,
        )
        .unwrap();
        let report = load_limits(dir.path(), NOW, STALE_AFTER_SECS);
        let windows = &report.snapshots[0].windows;
        assert_eq!(windows[0].fraction, 1.0);
        assert!(windows[0].is_alarming());
        assert!(windows[1].has_reset());
        assert!(
            !windows[1].is_alarming(),
            "a finished window is not an alarm"
        );
        let (_, binding) = report.binding_window().unwrap();
        assert_eq!(binding.label, "Over");
    }

    #[test]
    fn staleness_is_judged_by_updated_at_not_by_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let write = |updated_at: &str| {
            std::fs::write(
                dir.path().join("claude.json"),
                format!(
                    r#"{{"id":"claude","updatedAt":"{updated_at}","limits":[{{"label":"S","percent":0.5}}]}}"#
                ),
            )
            .unwrap();
            load_limits(dir.path(), NOW, STALE_AFTER_SECS).snapshots[0].stale
        };
        assert!(!write("2026-08-23T10:00:00+00:00"));
        assert!(write("2026-08-23T09:14:59Z"), "2701 seconds old");
        // `unwrap_or(fresh)` on the parse would pass a garbage date as current.
        assert!(write("garbage"));
        std::fs::write(
            dir.path().join("claude.json"),
            r#"{"id":"claude","limits":[{"label":"S","percent":0.5}]}"#,
        )
        .unwrap();
        let report = load_limits(dir.path(), NOW, STALE_AFTER_SECS);
        assert!(report.snapshots[0].stale, "undated is stale");
        assert!(
            report.binding_window().is_none(),
            "stale windows do not bind"
        );
    }

    #[test]
    fn a_status_only_record_is_shown_not_hidden() {
        // "Sign-in expired" with no windows is the state a user most needs to see.
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("claude.json"),
            r#"{"id":"claude","name":"Claude Code","updatedAt":"2026-08-23T10:00:00Z","usageStatusText":"Sign-in expired","limits":[]}"#,
        )
        .unwrap();
        let report = load_limits(dir.path(), NOW, STALE_AFTER_SECS);
        assert_eq!(report.snapshots.len(), 1);
        assert_eq!(report.snapshots[0].status_text, "Sign-in expired");
        assert!(report.snapshots[0].windows.is_empty());
    }

    #[test]
    fn discovery_keeps_only_regular_json_records() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::copy(
            fixtures().join("claude.json"),
            dir.path().join("claude.json"),
        )
        .unwrap();
        std::fs::write(dir.path().join(".claude.Ab12Cd"), "{\"id\":\"cl").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a record").unwrap();
        std::fs::create_dir(dir.path().join("subdir.json")).unwrap();
        let report = load_limits(dir.path(), NOW, STALE_AFTER_SECS);
        assert_eq!(report.snapshots.len(), 1);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
    }

    #[test]
    fn an_absent_directory_is_idle_and_a_malformed_record_is_named() {
        let dir = tempfile::TempDir::new().unwrap();
        let report = load_limits(&dir.path().join("does-not-exist"), NOW, STALE_AFTER_SECS);
        assert!(!report.present);
        assert!(report.snapshots.is_empty());
        assert!(report.problems.is_empty(), "absence is not a problem");

        std::fs::write(dir.path().join("claude.json"), "{not json").unwrap();
        let report = load_limits(dir.path(), NOW, STALE_AFTER_SECS);
        assert!(report.present);
        assert!(report.snapshots.is_empty());
        assert_eq!(report.problems.len(), 1);
        assert!(
            report.problems[0].starts_with("claude.json: "),
            "{:?}",
            report.problems
        );
    }

    #[test]
    fn snapshots_are_sorted_by_agent_and_timestamps_accept_both_suffixes() {
        let dir = tempfile::TempDir::new().unwrap();
        for (id, ts) in [
            ("codex", "2026-08-23T10:00:00Z"),
            ("claude", "2026-08-23T10:00:00.000000+00:00"),
        ] {
            std::fs::write(
                dir.path().join(format!("{id}.json")),
                format!(
                    r#"{{"id":"{id}","updatedAt":"{ts}","limits":[{{"label":"S","percent":0.2,"resetsAt":"{ts}"}}]}}"#
                ),
            )
            .unwrap();
        }
        let report = load_limits(dir.path(), NOW, STALE_AFTER_SECS);
        let ids: Vec<&str> = report.snapshots.iter().map(|s| s.agent.as_str()).collect();
        assert_eq!(ids, ["claude", "codex"]);
        assert!(report
            .snapshots
            .iter()
            .all(|s| s.updated_at == Some(NOW - 600)));
    }

    #[test]
    fn the_tier_label_is_available_to_the_billing_detector_by_collector_name() {
        assert_eq!(
            tier_label_for(&fixtures(), "claude_code").as_deref(),
            Some("Max 20x")
        );
        assert_eq!(
            tier_label_for(&fixtures(), "codex").as_deref(),
            Some("plus")
        );
        assert_eq!(tier_label_for(&fixtures(), "fireworks"), None);
        assert_eq!(
            tier_label_for(Path::new("/nonexistent/omarchy"), "claude_code"),
            None
        );
    }
}
