//! Subscription rate-limit windows, from every source that reports them.
//!
//! Until this module existed there was exactly one source — Omarchy's agents panel — so
//! `omarchy::load_limits` *was* the limits pipeline, and on any machine without Omarchy the `l`
//! panel had nothing to show. Claude Code writes the same facts about its own subscription into
//! `~/.claude.json`, on every platform, with no configuration, so this reads that too and merges
//! the two into the one `LimitsReport` the panel and `--json` already speak.
//!
//! `omarchy` keeps everything it knows about Omarchy's record format and becomes one producer;
//! this module owns the merge, the shared freshness rule, and the Claude Code reader.
//!
//! **What is read from `~/.claude.json`, and nothing else.** Only
//! `cachedUsageUtilization.fetchedAtMs` and the entries of
//! `cachedUsageUtilization.utilization.limits`, and from each entry only `kind`, `percent`,
//! `resets_at` and `scope.model.display_name`. The document also carries the account identifier,
//! project history, and sibling per-window objects; none of it is deserialised. Two consequences
//! are deliberate and load-bearing:
//!
//! - **The `utilization` map's keys are never iterated.** `limits` is a self-describing array;
//!   the sibling keys beside it are an open, undocumented set. Indexing one array rather than
//!   walking a map is what keeps names this tool has no business republishing out of its output.
//! - **`kind` is a closed vocabulary here.** An entry whose `kind` this module does not
//!   recognise is dropped and counted, never formatted into a label — so a value added upstream
//!   cannot reach the screen, the JSON export or a log line through the data path.
//!
//! The one field that *is* free text is `scope.model.display_name`, which is rendered and
//! exported verbatim as part of a window label. It is a model name the user is already running.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::collector::SourceRoots;
use crate::omarchy::{self, LimitWindow, LimitsReport, LimitsSnapshot};

/// The agent id a Claude Code snapshot is filed under.
///
/// Deliberately `claude`, not `claude_code`: that is what Omarchy names its record
/// (`omarchy::record_id_for_agent`), so when both sources are present the two readings of one
/// subscription collide on the same key by construction rather than through a mapping table.
pub const CLAUDE_AGENT_ID: &str = "claude";

/// How long a `cachedUsageUtilization` block is worth believing.
///
/// Shorter than Omarchy's allowance because the two are refreshed differently: Omarchy's updater
/// runs on a 15-minute timer, while this cache is refreshed by Claude Code itself as it works, so
/// a gap here means the user has not been running it rather than that a timer slipped.
pub const CACHE_STALE_AFTER_SECS: i64 = 30 * 60;

/// A stamp may sit this far in the future before it is treated as unusable rather than fresh.
///
/// Clocks disagree by a little; they do not disagree by half an hour. Anything beyond this is not
/// a skewed clock, it is a unit error — which is the specific failure this constant exists for,
/// see `is_stale`.
pub const CLOCK_SKEW_TOLERANCE_SECS: i64 = 60;

/// Whether a reading of the given age should be treated as describing some earlier moment.
///
/// **Two-sided, and that is the point.** The rule started life inside `omarchy::snapshot` as
/// `age > stale_after`, which is correct for every age a correct reader can produce and silently
/// wrong for the one an incorrect reader produces: this cache stamps `fetchedAtMs` in
/// *milliseconds* while every timestamp in this program is *seconds*, so a reader that forgets to
/// divide computes an age around -1.79e12 — hugely negative, comfortably `<= stale_after`, and
/// therefore *fresh*. A unit bug would have presented as a permanently up-to-date panel showing
/// numbers from an arbitrary moment, which is the worst possible way for it to fail.
///
/// So an age from the future beyond the skew tolerance is stale, exactly as an age from too far
/// in the past is. Undated is stale too: an unknown age is no reason to trust a number.
pub fn is_stale(age_secs: Option<i64>, stale_after: i64) -> bool {
    age_secs.is_none_or(|age| age > stale_after || age < -CLOCK_SKEW_TOLERANCE_SECS)
}

/// The slice of `~/.claude.json` this tool reads. Every other key in the document — the account
/// identifier included — is ignored by serde and never enters this process.
#[derive(Debug, Default, Deserialize)]
struct ConfigDocument {
    #[serde(rename = "cachedUsageUtilization")]
    cached_usage_utilization: Option<CachedUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct CachedUsage {
    /// Unix epoch **milliseconds**. The unit is in the name because the whole rest of this
    /// program counts seconds, and the conversion happens exactly once, in `read_claude_cache`.
    #[serde(rename = "fetchedAtMs")]
    fetched_at_ms: Option<i64>,
    utilization: Option<Utilization>,
}

/// Only `limits` is declared. The sibling per-window objects are intentionally absent from this
/// struct: not declaring them is what guarantees they are never read, never `Debug`-printed and
/// never exported, without this file having to name a single one of them.
#[derive(Debug, Default, Deserialize)]
struct Utilization {
    limits: Option<Vec<LimitEntry>>,
}

#[derive(Debug, Default, Deserialize)]
struct LimitEntry {
    kind: Option<String>,
    /// 0..100. Refused if it is not a finite, non-negative number.
    percent: Option<f64>,
    /// RFC 3339 **text** here. Codex's rollout `rate_limits` and Claude Code's statusline payload
    /// both spell the same instant as epoch *seconds*; this is deserialised as a String precisely
    /// so a number cannot be silently accepted as one unit when it means the other.
    resets_at: Option<String>,
    scope: Option<Scope>,
}

#[derive(Debug, Default, Deserialize)]
struct Scope {
    model: Option<ScopeModel>,
}

#[derive(Debug, Default, Deserialize)]
struct ScopeModel {
    display_name: Option<String>,
}

/// What a recognised `kind` is called on screen.
///
/// A closed match: an unrecognised kind returns `None` and its entry is dropped. The label is
/// this module's word, never the document's, so nothing from an open vocabulary reaches output.
fn window_label(kind: &str, scoped_model: Option<&str>) -> Option<String> {
    match kind {
        "session" => Some("Session (5-hour)".to_string()),
        "weekly_all" => Some("Weekly (all models)".to_string()),
        "weekly_scoped" => Some(match scoped_model {
            Some(model) => format!("Weekly · {model}"),
            // Scoped to something this reader cannot name. Still a real window worth drawing,
            // and saying so beats inventing a scope.
            None => "Weekly (scoped)".to_string(),
        }),
        _ => None,
    }
}

/// What `read_claude_cache` found, beside the snapshot itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CacheReadout {
    pub snapshot: Option<LimitsSnapshot>,
    /// The file exists and parsed as JSON. False means there is nothing here to read, which is
    /// the normal state for anyone who has never run Claude Code.
    pub present: bool,
    /// Entries refused because their `kind` is not one this reader knows, or because they
    /// carried no usable percentage. Counted rather than named: the point of the count is that
    /// a reader noticing "3 windows not shown" can go and look, without this tool republishing
    /// a vocabulary that is not its own.
    pub dropped: usize,
    /// The file is there and could not be used. Distinct from absent, and it reaches the screen:
    /// a config Claude Code rewrote in a way this cannot parse is exactly the thing that must
    /// not fail silently.
    pub problems: Vec<String>,
}

/// Read Claude Code's cached subscription utilisation. Pure over `now`; never writes.
pub fn read_claude_cache(
    path: &Path,
    now: i64,
    stale_after: i64,
    tier: Option<String>,
) -> CacheReadout {
    let mut readout = CacheReadout::default();
    let Ok(text) = std::fs::read_to_string(path) else {
        return readout;
    };
    let document: ConfigDocument = match serde_json::from_str(&text) {
        Ok(document) => document,
        Err(error) => {
            // Deliberately naming the path and the parse error and *not* any of the content:
            // this document is the user's config, and a serde error quotes only structure.
            let problem = format!("{}: {error}", path.display());
            crate::logging::warn("limits", &problem);
            readout.problems.push(problem);
            readout.present = true;
            return readout;
        }
    };
    readout.present = true;

    let Some(cached) = document.cached_usage_utilization else {
        // Claude Code is installed but has never recorded a utilisation block — a fresh install,
        // or an account this does not apply to. Nothing to draw and nothing wrong.
        return readout;
    };

    let mut windows: Vec<LimitWindow> = Vec::new();
    for entry in cached
        .utilization
        .and_then(|utilization| utilization.limits)
        .unwrap_or_default()
    {
        let scoped_model = entry
            .scope
            .and_then(|scope| scope.model)
            .and_then(|model| model.display_name)
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        let Some(label) = entry
            .kind
            .as_deref()
            .and_then(|kind| window_label(kind, scoped_model.as_deref()))
        else {
            readout.dropped += 1;
            continue;
        };
        let Some(percent) = entry.percent.filter(|p| p.is_finite() && *p >= 0.0) else {
            readout.dropped += 1;
            continue;
        };
        let resets_at = entry
            .resets_at
            .as_deref()
            .and_then(omarchy::parse_timestamp);
        windows.push(LimitWindow {
            label,
            fraction: (percent / 100.0).min(1.0),
            resets_at,
            resets_in_secs: resets_at.map(|at| at - now),
        });
    }

    // A snapshot with no windows has nothing to draw. Emitting one anyway would put an empty
    // yellow row on the panel *and*, once merged, let a source that understood none of what it
    // read evict a source that understood all of it. The drop count survives in `dropped`.
    if windows.is_empty() {
        return readout;
    }

    let updated_at = cached.fetched_at_ms.map(|ms| ms.div_euclid(1000));
    let age_secs = updated_at.map(|at| now - at);
    readout.snapshot = Some(LimitsSnapshot {
        agent: CLAUDE_AGENT_ID.to_string(),
        name: "Claude Code".to_string(),
        tier: tier.unwrap_or_default(),
        status_text: String::new(),
        updated_at,
        age_secs,
        stale: is_stale(age_secs, stale_after),
        windows,
    });
    readout
}

/// Which of two readings of the same subscription to keep.
///
/// Fresh beats stale, always. Between two fresh readings the newer one wins, and only if they
/// are equally recent does the source rank break the tie, which the caller states as
/// `a_wins_ties`: `load` ranks Claude Code's own config cache first, because it carries the
/// per-model weekly scoping that Omarchy's record flattens away and the statusline never has.
/// Between two stale readings the newer wins. "Whichever was read last" and "whichever number
/// is higher" are both one line away from here and both wrong.
fn prefer<'a>(a: &'a LimitsSnapshot, b: &'a LimitsSnapshot, a_wins_ties: bool) -> bool {
    match (a.stale, b.stale) {
        (false, true) => true,
        (true, false) => false,
        _ => match a.updated_at.cmp(&b.updated_at) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => a_wins_ties,
        },
    }
}

/// Whether Claude Code's files may be read at all.
///
/// `[collectors.claude_code] enabled = false` is the existing switch that means "do not read
/// Claude Code's files". `~/.claude.json` is one of them and the statusline cache holds Claude
/// Code's figures, so both producers honour it rather than becoming a way into the same data that
/// the switch does not cover. `--doctor` asks the same question, so its LIMITS section can say
/// "disabled" where `load` would not have looked, instead of "found".
pub fn claude_enabled(roots: &SourceRoots) -> bool {
    crate::collector::registry::SOURCES
        .iter()
        .find(|spec| spec.id == crate::collector::claude_code::ID)
        .is_none_or(|spec| roots.is_enabled(spec))
}

/// Every limit window this machine can report, from every source.
///
/// Pure over `now` so tests never touch the wall clock, and the single entry point for both the
/// dashboard and `--json` — which previously ran two independent reads of the same directory and
/// could disagree about one run.
pub fn load(roots: &SourceRoots, now: i64) -> LimitsReport {
    if !roots.limits_enabled {
        return LimitsReport::default();
    }

    let mut report = match roots.omarchy_usage_dir() {
        Some(dir) => omarchy::load_limits(&dir, now, omarchy::STALE_AFTER_SECS),
        None => LimitsReport::default(),
    };

    if claude_enabled(roots) {
        let tier = roots.omarchy_tier(crate::collector::claude_code::ID);
        if let Some(path) = roots.claude_json_path() {
            let readout = read_claude_cache(&path, now, CACHE_STALE_AFTER_SECS, tier.clone());
            report.problems.extend(readout.problems);
            if let Some(snapshot) = readout.snapshot {
                merge(&mut report, snapshot, true);
            }
        }
        // The third producer: what Claude Code last pushed to `--statusline`. It is this tool's
        // own file, but it holds Claude Code's figures, so the same switch covers it -- a user
        // who turned Claude Code off does not expect its subscription to keep appearing.
        if let Some(path) = crate::statusline::cache_path() {
            let (snapshot, problem) = crate::statusline::readout_at(&path, now, tier);
            report.problems.extend(problem);
            if let Some(snapshot) = snapshot {
                // Loses a tie to whatever is already there. The config cache merged first and
                // may carry a per-model weekly window; a statusline reading stamped the same
                // second cannot, and must not replace it.
                merge(&mut report, snapshot, false);
            }
        }
    }

    report.snapshots.sort_by(|a, b| a.agent.cmp(&b.agent));
    report
}

/// Fold one snapshot into a report, keyed on the agent it describes.
///
/// The key is normalised through `omarchy::record_id_for_agent` rather than compared raw: the
/// agent id on an Omarchy snapshot comes from the record's own `id` field, which this tool does
/// not control, and the two namespaces are already known to differ — that function exists because
/// of it. Comparing raw strings would file one subscription as two agents the day a record spells
/// itself `claude_code`.
///
/// `incoming_wins_ties` is the source rank for the equal-timestamp case, and it is the caller's
/// to state because `merge` cannot tell the producers apart: the `~/.claude.json` reading passes
/// `true`, the statusline reading `false`. This used to be a hardcoded `true`, which was right
/// while the config cache was the only caller and wrong the moment a second one appeared -- a
/// statusline reading stamped the same second would have evicted the snapshot carrying the
/// per-model weekly window with one that cannot carry it.
fn merge(report: &mut LimitsReport, incoming: LimitsSnapshot, incoming_wins_ties: bool) {
    let key = omarchy::record_id_for_agent(&incoming.agent).to_string();
    match report
        .snapshots
        .iter_mut()
        .find(|existing| omarchy::record_id_for_agent(&existing.agent) == key)
    {
        Some(existing) => {
            if prefer(&incoming, existing, incoming_wins_ties) {
                // The tier is a display label, and the record that carries limits is not always
                // the record that named the plan. Keep whichever side actually has one.
                let tier = if incoming.tier.is_empty() {
                    existing.tier.clone()
                } else {
                    incoming.tier.clone()
                };
                *existing = LimitsSnapshot { tier, ..incoming };
            }
        }
        None => report.snapshots.push(incoming),
    }
}

/// Where the Claude Code cache would be read from, for `--doctor` and the panel's empty state.
pub fn claude_cache_path(roots: &SourceRoots) -> Option<PathBuf> {
    roots.claude_json_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10 minutes after the fixture's `fetchedAtMs`, and before either reset instant.
    const NOW: i64 = 1_787_479_800;
    /// What the fixture's `fetchedAtMs` means once it is read as milliseconds.
    const FETCHED_AT_SECS: i64 = 1_787_479_200;

    fn fixture() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/claude_home/.claude.json"
        ))
    }

    fn read() -> CacheReadout {
        read_claude_cache(&fixture(), NOW, CACHE_STALE_AFTER_SECS, None)
    }

    #[test]
    fn the_three_known_kinds_are_read_and_the_unknown_one_is_dropped() {
        let readout = read();
        assert!(readout.present);
        assert!(readout.problems.is_empty());
        let snapshot = readout.snapshot.expect("the fixture carries windows");
        let labels: Vec<&str> = snapshot.windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Session (5-hour)",
                "Weekly (all models)",
                "Weekly · Fixture Model 9"
            ]
        );
        assert_eq!(readout.dropped, 1, "the unrecognised kind is counted");
    }

    /// The whole reason `is_stale` is two-sided.
    ///
    /// `fetchedAtMs` is milliseconds against a seconds clock. A reader that forgets to divide
    /// computes an age near -1.79e12, which the original one-sided `age > stale_after` rule
    /// scored as *fresh* -- so the panel would have shown an arbitrary moment's numbers, forever,
    /// with no staleness marker. This asserts the sign is caught, not just the magnitude.
    #[test]
    fn a_millisecond_stamp_read_as_seconds_is_stale_not_fresh() {
        let age_if_unconverted = NOW - 1_787_479_200_000;
        assert!(age_if_unconverted < 0, "the bug produces a negative age");
        assert!(
            is_stale(Some(age_if_unconverted), CACHE_STALE_AFTER_SECS),
            "a stamp from the far future must not read as fresh"
        );
        assert!(
            !is_stale(Some(CACHE_STALE_AFTER_SECS - 1), CACHE_STALE_AFTER_SECS),
            "a recent age is still fresh"
        );
        assert!(is_stale(None, CACHE_STALE_AFTER_SECS), "undated is stale");
        assert!(
            !is_stale(Some(-CLOCK_SKEW_TOLERANCE_SECS + 1), CACHE_STALE_AFTER_SECS),
            "a little clock skew is tolerated"
        );
    }

    #[test]
    fn fetched_at_ms_is_converted_to_seconds_once() {
        let snapshot = read().snapshot.expect("snapshot");
        assert_eq!(snapshot.updated_at, Some(FETCHED_AT_SECS));
        assert_eq!(snapshot.age_secs, Some(NOW - FETCHED_AT_SECS));
        assert!(!snapshot.stale, "10 minutes old is fresh");
    }

    #[test]
    fn percentages_become_fractions_and_reset_instants_become_seconds() {
        let snapshot = read().snapshot.expect("snapshot");
        let session = &snapshot.windows[0];
        assert!((session.fraction - 0.11).abs() < 1e-9);
        assert_eq!(session.percent_used().round() as u64, 11);
        assert_eq!(session.resets_in_secs, Some(3 * 3600));
        let scoped = &snapshot.windows[2];
        assert!((scoped.fraction - 0.88).abs() < 1e-9);
        assert!(!scoped.is_alarming(), "88% is under the 90% alarm");
    }

    /// The public-repo guarantee, asserted rather than argued.
    ///
    /// Nothing from the document may reach a value this program can print. The fixture plants a
    /// credential in `projects.history`, a placeholder account id in both places the real one
    /// appears, and several sibling keys beside `limits` -- none may appear in the readout's own
    /// `Debug`, which is what a log line or a panic message would render.
    #[test]
    fn no_document_content_reaches_the_readout() {
        let readout = read();
        let rendered = format!("{readout:?}");
        for forbidden in [
            "FIXTURE_SECRET",
            "00000000-0000-0000-0000-000000000000",
            "futureField",
            "a key this reader has never heard of",
            "extra_usage",
            "member_dashboard_available",
            "oauthAccount",
            "/home/fixture/work",
            // The unrecognised kind: dropped through a closed match, never formatted.
            "a_kind_this_reader_does_not_know",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "{forbidden:?} reached the readout: {rendered}"
            );
        }
    }

    #[test]
    fn an_absent_file_is_absent_not_broken() {
        let readout = read_claude_cache(
            Path::new("/nonexistent/.claude.json"),
            NOW,
            CACHE_STALE_AFTER_SECS,
            None,
        );
        assert!(!readout.present);
        assert!(readout.snapshot.is_none());
        assert!(readout.problems.is_empty(), "absent is not a problem");
    }

    #[test]
    fn an_unparsable_file_is_a_problem_not_a_silence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".claude.json");
        std::fs::write(&path, "{ not json").expect("write");
        let readout = read_claude_cache(&path, NOW, CACHE_STALE_AFTER_SECS, None);
        assert!(readout.present, "the file is there");
        assert!(readout.snapshot.is_none());
        assert_eq!(readout.problems.len(), 1, "convention 8: it must be said");
    }

    /// A cache that understood none of what it read must not evict a source that understood all
    /// of it, and must not draw an empty row.
    #[test]
    fn a_snapshot_with_no_recognised_windows_is_not_emitted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{"cachedUsageUtilization":{"fetchedAtMs":1787479200000,
               "utilization":{"limits":[{"kind":"renamed_upstream","percent":50}]}}}"#,
        )
        .expect("write");
        let readout = read_claude_cache(&path, NOW, CACHE_STALE_AFTER_SECS, None);
        assert!(readout.present);
        assert!(readout.snapshot.is_none(), "no windows, no row");
        assert_eq!(readout.dropped, 1);
    }

    fn snapshot_at(agent: &str, updated_at: i64, stale: bool) -> LimitsSnapshot {
        LimitsSnapshot {
            agent: agent.to_string(),
            name: agent.to_string(),
            updated_at: Some(updated_at),
            stale,
            windows: vec![LimitWindow {
                label: "Session (5-hour)".to_string(),
                fraction: 0.5,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn fresh_beats_stale_and_newer_beats_older() {
        let fresh = snapshot_at("claude", 100, false);
        let stale = snapshot_at("claude", 900, true);
        assert!(prefer(&fresh, &stale, true), "fresh wins even when older");
        assert!(!prefer(&stale, &fresh, true));

        let newer = snapshot_at("claude", 900, false);
        let older = snapshot_at("claude", 100, false);
        assert!(
            prefer(&newer, &older, false),
            "newer wins between two fresh"
        );
        assert!(
            !prefer(&older, &newer, true),
            "the richer source does not outrank a materially newer reading"
        );
    }

    /// The merge key is normalised, so one subscription cannot become two rows.
    ///
    /// Omarchy's agent id comes from its record's own `id` field, which this tool does not
    /// control -- `record_id_for_agent` exists because the two namespaces already differ.
    #[test]
    fn a_claude_code_record_merges_with_the_cache_rather_than_doubling_it() {
        let mut report = LimitsReport {
            snapshots: vec![snapshot_at("claude_code", 100, true)],
            ..Default::default()
        };
        merge(&mut report, snapshot_at(CLAUDE_AGENT_ID, 900, false), true);
        assert_eq!(report.snapshots.len(), 1, "one subscription, one row");
        assert!(!report.snapshots[0].stale, "the fresh reading won");
    }

    #[test]
    fn a_different_agent_is_a_separate_row() {
        let mut report = LimitsReport {
            snapshots: vec![snapshot_at("codex", 100, false)],
            ..Default::default()
        };
        merge(&mut report, snapshot_at(CLAUDE_AGENT_ID, 100, false), true);
        assert_eq!(report.snapshots.len(), 2);
    }

    /// The equal-timestamp case, caught in review of the change that added the statusline
    /// producer: merged with the config cache's own "I win ties" rank, a statusline reading
    /// stamped the same second replaced the snapshot carrying the per-model weekly window with
    /// one that cannot carry it.
    #[test]
    fn a_statusline_reading_does_not_evict_the_config_cache_on_a_tie() {
        let mut richer = snapshot_at(CLAUDE_AGENT_ID, 900, false);
        richer.windows.push(LimitWindow {
            label: "Weekly · Fixture Model 9".to_string(),
            fraction: 0.5,
            ..Default::default()
        });
        let mut report = LimitsReport {
            snapshots: vec![richer],
            ..Default::default()
        };
        merge(&mut report, snapshot_at(CLAUDE_AGENT_ID, 900, false), false);
        assert_eq!(report.snapshots.len(), 1);
        assert_eq!(
            report.snapshots[0].windows.len(),
            2,
            "the per-model window survived the tie"
        );

        // Freshness still outranks richness: a materially newer statusline reading wins.
        merge(&mut report, snapshot_at(CLAUDE_AGENT_ID, 901, false), false);
        assert_eq!(report.snapshots[0].windows.len(), 1);
    }

    /// A tier is a display label, and the record carrying the limits is not always the one that
    /// named the plan.
    #[test]
    fn a_winning_snapshot_without_a_tier_keeps_the_one_already_known() {
        let mut existing = snapshot_at("claude", 100, true);
        existing.tier = "Max 20x".to_string();
        let mut report = LimitsReport {
            snapshots: vec![existing],
            ..Default::default()
        };
        merge(&mut report, snapshot_at(CLAUDE_AGENT_ID, 900, false), true);
        assert_eq!(report.snapshots[0].tier, "Max 20x");
    }
}
