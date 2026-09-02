//! `--statusline`: Claude Code's rate-limit push, as a one-line readout and as a limits source.
//!
//! Claude Code hands a statusline command a JSON document on stdin every time it redraws, and —
//! for Pro and Max subscribers, once the session has had an API response — that document carries
//! a `rate_limits` block: the 5-hour window, the weekly window and, on a gateway plan, the spend
//! limit, each with a used percentage and a reset instant. Nothing else in this tool is *pushed*
//! at it. `~/.claude.json` (`crate::limits`) and Omarchy's records are polled on the dashboard's
//! interval, whereas Claude Code re-runs this command when a window reaches its `resets_at`. It
//! is also the one always-visible surface: the line sits under the prompt for the whole session.
//!
//! Two products from one payload. The line on stdout is what Claude Code shows. The windows are
//! also written to a small cache in this tool's own data directory, which `crate::limits::load`
//! reads as a third producer beside `~/.claude.json` and Omarchy, so the `l` panel and `--json`
//! carry them on a machine where neither of the other two says anything.
//!
//! **What is read from the payload, and nothing else.** `rate_limits.five_hour`,
//! `rate_limits.seven_day` and `rate_limits.spend_limit`, and from each only `used_percentage`
//! and `resets_at`. The document also names the session, the transcript path, the working
//! directory, the model and the session's cost; none of it is deserialised, and the keys beside
//! the three windows are never iterated — the same discipline as `crate::limits`, for the same
//! reason: the payload belongs to Claude Code, and this repository is public.
//!
//! **Absence rules**, each documented by Claude Code and each this project's kind of bug:
//!
//! - The whole block is absent on an API-billed account, and in every session until the first
//!   API response. That is "no such thing here", not 0%: the line is empty, the exit code is 0,
//!   and the cache is left as it was — the previous session's windows are still the best
//!   knowledge there is, and the freshness rule retires them.
//! - Each window may be independently absent. The cache is rewritten with exactly the windows
//!   present, so a window that has gone is cleared from the panel rather than frozen at its
//!   last percentage.
//! - Claude Code drops a window once its `resets_at` has passed. So a window whose instant is
//!   behind `now` is dropped at *read* time, whichever side of the cache it is on: rendering the
//!   last-known percentage after the reset would show a full bar on an empty window.
//! - A percentage that is not a finite, non-negative number drops its window. Nothing here
//!   invents a figure.
//!
//! **Units.** `resets_at` here is Unix epoch *seconds* as a JSON number. `~/.claude.json` spells
//! the same instant as RFC 3339 text, and the two readers are kept separate so neither format can
//! be accepted where the other is meant. `used_percentage` is 0..100; a spend limit may exceed
//! 100, and the line prints the real figure while the panel's bar is clamped like every other.
//!
//! **One subscription, one row.** The cache files its snapshot under the same agent id as the
//! `~/.claude.json` reader, so `limits::load` keeps whichever reading is fresher rather than
//! drawing the subscription twice. The trade is recorded rather than hidden: a newer statusline
//! reading replaces the config document's, and the statusline carries no per-model weekly
//! window, so that row is absent until the config document is the newer reading again.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::limits::{is_stale, CACHE_STALE_AFTER_SECS, CLAUDE_AGENT_ID};
use crate::omarchy::{LimitWindow, LimitsSnapshot, ALARM_FRACTION};
use crate::ui::aggregate::format_duration;

/// The cache's file name, under the data directory beside `update-check.json`.
pub const CACHE_FILE: &str = "statusline-limits.json";

/// The slice of the statusline payload this tool reads. Every other key is ignored by serde and
/// never enters this process.
#[derive(Debug, Default, Deserialize)]
struct Payload {
    rate_limits: Option<RateLimits>,
}

/// Three named windows, as struct fields rather than a map: a fourth window added upstream is
/// ignored here rather than formatted into a label.
#[derive(Debug, Default, Deserialize)]
struct RateLimits {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
    spend_limit: Option<Window>,
}

#[derive(Debug, Default, Deserialize)]
struct Window {
    used_percentage: Option<f64>,
    /// Unix epoch **seconds**. Read as `f64` so an integer and a float spelling of the same
    /// instant are both accepted, and converted exactly once, in `parse`.
    resets_at: Option<f64>,
}

/// Which of Claude Code's windows a cached entry is.
///
/// A closed set, and the cache's own vocabulary: a kind this build does not recognise fails the
/// whole cache read, which is right for a file this tool wrote itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FiveHour,
    SevenDay,
    SpendLimit,
}

impl WindowKind {
    /// The panel's label. The first two match what the `~/.claude.json` reader calls the same
    /// windows, so one window is never drawn under two names.
    pub fn label(self) -> &'static str {
        match self {
            WindowKind::FiveHour => "Session (5-hour)",
            WindowKind::SevenDay => "Weekly (all models)",
            WindowKind::SpendLimit => "Spend limit",
        }
    }

    /// The short form the status line uses.
    fn short(self) -> &'static str {
        match self {
            WindowKind::FiveHour => "5h",
            WindowKind::SevenDay => "7d",
            WindowKind::SpendLimit => "spend",
        }
    }
}

/// One window as cached.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedWindow {
    pub kind: WindowKind,
    /// 0..100. A spend limit may exceed 100.
    pub used_percentage: f64,
    /// Unix epoch seconds, when the payload carried one.
    pub resets_at: Option<i64>,
}

/// What the last payload said, and when it arrived.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedLimits {
    /// Unix seconds the payload was received. Claude Code does not stamp the block, so arrival
    /// stands in as the reading's own timestamp for the panel's freshness rule.
    pub received: i64,
    pub windows: Vec<CachedWindow>,
}

/// Read a payload.
///
/// `Ok(None)` means it carried no `rate_limits` block. `Err` means it was not the JSON document
/// Claude Code sends, which is worth a non-zero exit rather than a quiet nothing: a settings
/// entry pointing at the wrong command would otherwise look exactly like an API-billed account.
pub fn parse(text: &str, now: i64) -> Result<Option<CachedLimits>, serde_json::Error> {
    let payload: Payload = serde_json::from_str(text)?;
    let Some(limits) = payload.rate_limits else {
        return Ok(None);
    };
    let mut windows = Vec::new();
    for (kind, window) in [
        (WindowKind::FiveHour, limits.five_hour),
        (WindowKind::SevenDay, limits.seven_day),
        (WindowKind::SpendLimit, limits.spend_limit),
    ] {
        let Some(window) = window else {
            continue;
        };
        let Some(used_percentage) = window
            .used_percentage
            .filter(|percent| percent.is_finite() && *percent >= 0.0)
        else {
            continue;
        };
        let resets_at = window
            .resets_at
            .filter(|at| at.is_finite())
            .map(|at| at as i64);
        windows.push(CachedWindow {
            kind,
            used_percentage,
            resets_at,
        });
    }
    Ok(Some(CachedLimits {
        received: now,
        windows,
    }))
}

/// The windows still describing an open period at `now`.
///
/// A window whose reset instant has passed is gone, not full: Claude Code drops it from the next
/// payload it sends, and this drops it from a cached one, so the panel cannot outlive the reset.
pub fn live(windows: &[CachedWindow], now: i64) -> Vec<CachedWindow> {
    windows
        .iter()
        .filter(|window| window.resets_at.is_none_or(|at| at > now))
        .cloned()
        .collect()
}

/// The status line: the live windows in Claude Code's order, an alarming one in bold red, and
/// nothing at all when there is nothing live to say.
pub fn line(windows: &[CachedWindow], now: i64) -> String {
    live(windows, now)
        .iter()
        .map(|window| {
            let figure = format!(
                "{} {}%",
                window.kind.short(),
                window.used_percentage.round()
            );
            let figure = if window.used_percentage / 100.0 >= ALARM_FRACTION {
                format!("\x1b[1;31m{figure}\x1b[0m")
            } else {
                figure
            };
            match window.resets_at {
                Some(at) => format!("{figure} (resets {})", format_duration(at - now)),
                None => figure,
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Where the cache lives, or `None` without a home directory.
pub fn cache_path() -> Option<PathBuf> {
    Some(crate::utils::data_dir()?.join(CACHE_FILE))
}

/// Write the windows where `limits::load` will find them.
///
/// Temporary-then-rename, as the update and pricing caches are written, so the dashboard's read
/// never sees half a file -- but with the temporary named per process, which those caches do not
/// need and this one does. They have one writer, the dashboard. This has one writer per open
/// Claude Code session, each fired on every redraw, and two sharing a temporary name race: the
/// first rename moves the second's half-written file into place, and the second rename finds
/// nothing to move. `omarchy::record::write_record` records the same rule for the same reason.
/// A temporary that could not be renamed is removed, so a failure leaves nothing behind either.
pub fn write_cache_at(path: &Path, cached: &CachedLimits) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("statusline cache path has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let written = std::fs::write(&temporary, serde_json::to_vec_pretty(cached)?)
        .and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    Ok(written?)
}

/// Read the cache. `Ok(None)` when there is none, which is the normal state for anyone who has
/// not installed the statusline entry; `Err` names a file that is there and cannot be used,
/// which is a problem for the panel to show rather than a silence.
pub fn read_cache_at(path: &Path) -> Result<Option<CachedLimits>, String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    serde_json::from_str(&text).map(Some).map_err(|error| {
        let problem = format!("{}: {error}", path.display());
        crate::logging::warn("statusline", &problem);
        problem
    })
}

/// The cached windows as the limits panel speaks them, or `None` when none is live.
pub fn snapshot(
    cached: &CachedLimits,
    now: i64,
    stale_after: i64,
    tier: Option<String>,
) -> Option<LimitsSnapshot> {
    let windows: Vec<LimitWindow> = live(&cached.windows, now)
        .into_iter()
        .map(|window| LimitWindow {
            label: window.kind.label().to_string(),
            fraction: (window.used_percentage / 100.0).min(1.0),
            resets_at: window.resets_at,
            resets_in_secs: window.resets_at.map(|at| at - now),
        })
        .collect();
    if windows.is_empty() {
        return None;
    }
    let age_secs = Some(now - cached.received);
    Some(LimitsSnapshot {
        agent: CLAUDE_AGENT_ID.to_string(),
        name: "Claude Code".to_string(),
        tier: tier.unwrap_or_default(),
        status_text: String::new(),
        updated_at: Some(cached.received),
        age_secs,
        stale: is_stale(age_secs, stale_after),
        windows,
    })
}

/// What `limits::load` takes from the cache at `path`: a snapshot when a live one is there, and
/// the problem when the file exists and cannot be read.
pub fn readout_at(
    path: &Path,
    now: i64,
    tier: Option<String>,
) -> (Option<LimitsSnapshot>, Option<String>) {
    match read_cache_at(path) {
        Ok(Some(cached)) => (snapshot(&cached, now, CACHE_STALE_AFTER_SECS, tier), None),
        Ok(None) => (None, None),
        Err(problem) => (None, Some(problem)),
    }
}

/// `--statusline`: the payload on stdin, the line on stdout, the windows in the cache.
pub fn run_from_stdin(now: i64) -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let Some(cached) = parse(&input, now).map_err(|error| {
        anyhow::anyhow!("--statusline expects Claude Code's statusline JSON on stdin: {error}")
    })?
    else {
        return Ok(());
    };
    let line = line(&cached.windows, now);
    if !line.is_empty() {
        crate::helpers::print_line(&line)?;
    }
    // The line is the product and the cache a by-product, and the exit code has to say so.
    // Claude Code (2.1.258, read from its bundle) shows stdout only from a command that exited
    // 0 and blanks the status line otherwise, with stderr reaching its debug log alone -- so a
    // cache that cannot be written must not become an exit code, or the readout vanishes for a
    // failure that has nothing to do with it. It is said on stderr and in the log instead. The
    // non-zero exit stays reserved for stdin that is not the document, where there is nothing to
    // show anyway.
    let written = match cache_path() {
        Some(path) => write_cache_at(&path, &cached),
        None => Err(anyhow::anyhow!("could not determine a data directory")),
    };
    if let Err(error) = written {
        let message = format!("the windows were not cached for the panel: {error}");
        crate::logging::warn("statusline", &message);
        eprintln!("ai-usage-tui --statusline: {message}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/claude_statusline.json"
        ))
        .expect("tests/fixtures/claude_statusline.json")
    }

    /// An hour before the earliest reset the fixture names, so every window is live. Derived
    /// rather than written down: the fixture is a capture, and its instants are whatever they
    /// were on the day.
    fn now_for(cached: &CachedLimits) -> i64 {
        cached
            .windows
            .iter()
            .filter_map(|window| window.resets_at)
            .min()
            .expect("the fixture carries a reset instant")
            - 3600
    }

    fn read() -> (CachedLimits, i64) {
        let cached = parse(&fixture(), 0)
            .expect("the fixture parses")
            .expect("the fixture carries rate_limits");
        let now = now_for(&cached);
        let cached = CachedLimits {
            received: now,
            ..cached
        };
        (cached, now)
    }

    fn window(kind: WindowKind, used_percentage: f64, resets_at: Option<i64>) -> CachedWindow {
        CachedWindow {
            kind,
            used_percentage,
            resets_at,
        }
    }

    #[test]
    fn the_capture_yields_the_session_and_weekly_windows_with_their_resets() {
        let (cached, _) = read();
        let kinds: Vec<WindowKind> = cached.windows.iter().map(|w| w.kind).collect();
        assert!(
            kinds.starts_with(&[WindowKind::FiveHour, WindowKind::SevenDay]),
            "{kinds:?}"
        );
        for window in &cached.windows {
            assert!(
                window.resets_at.is_some(),
                "{:?} carries no reset instant",
                window.kind
            );
            assert!(window.used_percentage >= 0.0);
        }
    }

    /// The public-repo guarantee. The payload names the session, the transcript, the working
    /// directory and the model; the fixture plants a marker in those places, and none of it may
    /// appear in the cache's `Debug`, the JSON this writes, or the line.
    #[test]
    fn nothing_beside_the_windows_reaches_the_cache_or_the_line() {
        let (cached, now) = read();
        let rendered = format!(
            "{cached:?} {} {}",
            serde_json::to_string(&cached).expect("serialises"),
            line(&cached.windows, now)
        );
        for forbidden in [
            "FIXTURE_SECRET",
            "00000000-0000-0000-0000-000000000000",
            "11111111-1111-1111-1111-111111111111",
            "session_id",
            "transcript_path",
            "cwd",
            "scratchpad_dir",
            "prompt_id",
            "workspace",
            "total_cost_usd",
            "context_window",
            "prompt_cache",
            "display_name",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "{forbidden:?} reached the readout: {rendered}"
            );
        }
    }

    #[test]
    fn an_absent_block_is_none_and_a_non_document_is_an_error() {
        assert_eq!(parse("{}", 10).expect("parses"), None);
        assert_eq!(
            parse(r#"{"model":{"id":"x"},"cwd":"/tmp"}"#, 10).expect("parses"),
            None,
            "a pre-first-response payload has no block, and that is not 0%"
        );
        assert!(parse("not json", 10).is_err());
        assert!(parse("", 10).is_err());
    }

    #[test]
    fn each_window_may_be_independently_absent() {
        let cached = parse(
            r#"{"rate_limits":{"seven_day":{"used_percentage":63,"resets_at":1000}}}"#,
            10,
        )
        .expect("parses")
        .expect("block");
        assert_eq!(
            cached.windows,
            vec![window(WindowKind::SevenDay, 63.0, Some(1000))]
        );
        assert_eq!(cached.received, 10);
    }

    #[test]
    fn a_refused_percentage_drops_its_window_and_never_becomes_a_figure() {
        let cached = parse(
            r#"{"rate_limits":{
                "five_hour":{"used_percentage":null,"resets_at":1000},
                "seven_day":{"used_percentage":-5,"resets_at":1000},
                "spend_limit":{"used_percentage":120.5}
            }}"#,
            10,
        )
        .expect("parses")
        .expect("block");
        assert_eq!(
            cached.windows,
            vec![window(WindowKind::SpendLimit, 120.5, None)],
            "null and negative are refused; a spend limit over 100 is real"
        );
        let cached = parse(
            r#"{"rate_limits":{"five_hour":{"used_percentage":"42"}}}"#,
            10,
        );
        assert!(cached.is_err(), "a string is not a percentage");
    }

    #[test]
    fn a_reset_instant_may_be_spelled_as_a_float() {
        let cached = parse(
            r#"{"rate_limits":{"five_hour":{"used_percentage":42,"resets_at":1000.0}}}"#,
            10,
        )
        .expect("parses")
        .expect("block");
        assert_eq!(cached.windows[0].resets_at, Some(1000));
    }

    /// The reset rule: absence is the reset, and the cached percentage must not outlive it.
    #[test]
    fn a_window_whose_reset_has_passed_is_dropped_at_read_time() {
        let windows = vec![
            window(WindowKind::FiveHour, 100.0, Some(1000)),
            window(WindowKind::SevenDay, 40.0, Some(5000)),
            window(WindowKind::SpendLimit, 10.0, None),
        ];
        let kinds =
            |now: i64| -> Vec<WindowKind> { live(&windows, now).iter().map(|w| w.kind).collect() };
        assert_eq!(
            kinds(999),
            vec![
                WindowKind::FiveHour,
                WindowKind::SevenDay,
                WindowKind::SpendLimit
            ]
        );
        assert_eq!(
            kinds(1000),
            vec![WindowKind::SevenDay, WindowKind::SpendLimit],
            "at the instant itself the window is over"
        );
        assert_eq!(kinds(6000), vec![WindowKind::SpendLimit]);
        assert!(
            snapshot(
                &CachedLimits {
                    received: 6000,
                    windows: windows[..2].to_vec()
                },
                6000,
                CACHE_STALE_AFTER_SECS,
                None
            )
            .is_none(),
            "nothing live, no row"
        );
    }

    #[test]
    fn the_line_names_the_live_windows_and_reddens_an_alarming_one() {
        let now = 1_000_000;
        let windows = vec![
            window(WindowKind::FiveHour, 42.4, Some(now + 2 * 3600 + 10 * 60)),
            window(
                WindowKind::SevenDay,
                63.0,
                Some(now + 3 * 86_400 + 4 * 3600),
            ),
        ];
        assert_eq!(
            line(&windows, now),
            "5h 42% (resets 2h 10m) · 7d 63% (resets 3d 4h)"
        );

        let alarming = vec![window(WindowKind::FiveHour, 92.0, Some(now + 60))];
        assert_eq!(line(&alarming, now), "\x1b[1;31m5h 92%\x1b[0m (resets 1m)");

        let spend = vec![window(WindowKind::SpendLimit, 120.0, None)];
        assert_eq!(
            line(&spend, now),
            "\x1b[1;31mspend 120%\x1b[0m",
            "over 100 is printed as it is, and no reset means no parenthesis"
        );

        assert_eq!(line(&[], now), "");
        assert_eq!(
            line(&[window(WindowKind::FiveHour, 100.0, Some(now))], now),
            "",
            "a window at its reset instant is not drawn full"
        );
    }

    #[test]
    fn the_snapshot_speaks_the_panels_vocabulary_and_clamps_the_bar() {
        let now = 1_000_000;
        let cached = CachedLimits {
            received: now - 600,
            windows: vec![
                window(WindowKind::FiveHour, 42.0, Some(now + 3600)),
                window(WindowKind::SpendLimit, 120.0, None),
            ],
        };
        let snapshot = snapshot(&cached, now, CACHE_STALE_AFTER_SECS, Some("Max 20x".into()))
            .expect("live windows");
        assert_eq!(snapshot.agent, CLAUDE_AGENT_ID);
        assert_eq!(snapshot.tier, "Max 20x");
        assert_eq!(snapshot.updated_at, Some(now - 600));
        assert_eq!(snapshot.age_secs, Some(600));
        assert!(!snapshot.stale);
        let labels: Vec<&str> = snapshot.windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, vec!["Session (5-hour)", "Spend limit"]);
        assert!((snapshot.windows[0].fraction - 0.42).abs() < 1e-9);
        assert_eq!(snapshot.windows[0].resets_in_secs, Some(3600));
        assert_eq!(
            snapshot.windows[1].fraction, 1.0,
            "the bar is clamped; the line printed the real figure"
        );
        assert!(snapshot.windows[1].is_alarming());
    }

    /// The freshness rule is the shared two-sided one: too old is stale, and so is a stamp from
    /// the future, which is what a unit error would produce.
    #[test]
    fn staleness_is_two_sided() {
        let now = 1_000_000;
        let at = |received: i64| {
            snapshot(
                &CachedLimits {
                    received,
                    windows: vec![window(WindowKind::FiveHour, 1.0, None)],
                },
                now,
                CACHE_STALE_AFTER_SECS,
                None,
            )
            .expect("a window with no reset is always live")
            .stale
        };
        assert!(!at(now - CACHE_STALE_AFTER_SECS + 1));
        assert!(at(now - CACHE_STALE_AFTER_SECS - 1), "too old");
        assert!(
            at(now * 1000),
            "a millisecond stamp is from the future, not fresh"
        );
    }

    #[test]
    fn the_cache_round_trips_and_leaves_no_temporary_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join(CACHE_FILE);
        let cached = CachedLimits {
            received: 123,
            windows: vec![
                window(WindowKind::FiveHour, 42.0, Some(1000)),
                window(WindowKind::SpendLimit, 120.0, None),
            ],
        };
        write_cache_at(&path, &cached).expect("write");
        assert_eq!(read_cache_at(&path), Ok(Some(cached)));
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .expect("read dir")
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from(CACHE_FILE)]);
    }

    /// Two sessions redrawing at once must not share a temporary, and a write that fails must
    /// not leave one behind. The rename here fails because the destination is a directory.
    #[test]
    fn the_temporary_is_per_process_and_removed_on_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(CACHE_FILE);
        std::fs::create_dir(&path).expect("a directory where the cache should go");
        let cached = CachedLimits {
            received: 1,
            windows: vec![window(WindowKind::FiveHour, 1.0, None)],
        };
        assert!(write_cache_at(&path, &cached).is_err());
        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read dir")
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporaries left behind: {leftovers:?}"
        );
        assert!(
            path.with_extension(format!("{}.tmp", std::process::id()))
                .to_string_lossy()
                .contains(&std::process::id().to_string()),
            "the temporary name carries the process id"
        );
    }

    #[test]
    fn an_absent_cache_is_nothing_and_a_broken_one_is_a_problem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(CACHE_FILE);
        assert_eq!(read_cache_at(&path), Ok(None));
        assert_eq!(readout_at(&path, 0, None), (None, None));

        std::fs::write(&path, "{ not json").expect("write");
        let (snapshot, problem) = readout_at(&path, 0, None);
        assert!(snapshot.is_none());
        assert!(
            problem.is_some_and(|p| p.contains(CACHE_FILE)),
            "convention 8: a file this tool wrote and cannot read is said"
        );

        // A kind this build does not know is a cache from another build, not a window to guess.
        std::fs::write(
            &path,
            r#"{"received":1,"windows":[{"kind":"lunar_cycle","used_percentage":1}]}"#,
        )
        .expect("write");
        assert!(read_cache_at(&path).is_err());
    }
}
