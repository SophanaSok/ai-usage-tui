//! Codex CLI session-log collector.
//!
//! Codex writes one JSONL "rollout" per thread under `$CODEX_HOME/sessions/YYYY/MM/DD/` (and
//! `archived_sessions/` once a thread is archived), one envelope per line:
//! `{"timestamp": "…Z", "type": <kind>, "payload": {…}}`. The kinds this collector reads:
//!
//! - `session_meta` — the thread id and working directory. Written first; a forked thread's
//!   file carries a *second* `session_meta` further down, copied from its ancestor, so only the
//!   first one names the file.
//! - `turn_context` — the model in force from here on. Nothing on a usage line names it.
//! - `event_msg` with `payload.type == "token_count"` — one per model API call, carrying
//!   `info.last_token_usage` (that call) and `info.total_token_usage` (cumulative for the
//!   thread). The last call's figures are what bill; summing the cumulative totals would grow
//!   quadratically. The same event is re-emitted on rate-limit-only updates with unchanged
//!   totals, and after compaction with an *estimate* that has no input or output — both are
//!   skipped.
//!
//! Token conventions, from the CLI's own arithmetic (`TokenUsage::non_cached_input`):
//! `cached_input_tokens` sits inside `input_tokens`, and `reasoning_output_tokens` inside
//! `output_tokens`, so both are split out here rather than counted twice. OpenAI bills prompt
//! cache writes as ordinary input, so `cache_write_input_tokens` stays inside `input` at the
//! input rate rather than becoming a bucket no published rate exists for.
//!
//! A forked thread copies its ancestor's history — timestamps and all — into the new file, so
//! identity is content-based (`codex:<timestamp>:<call tokens>:<running total>`) and the copy
//! deduplicates against the original wherever it is read.
//!
//! **Privacy.** Rollouts hold prompts, tool call arguments and outputs, and reasoning
//! summaries. Only `session_meta`, `turn_context` and the `token_count` block are read; message
//! content is never parsed, retained, or logged. Same invariant as the Claude Code collector.
//!
//! **Compressed rollouts.** With `local_thread_store_compression` on (off by default as of
//! codex-cli 0.155.0), a worker replaces every rollout untouched for seven days with
//! `<name>.jsonl.zst` -- one zstd frame, level 3, no checksum -- and removes the plain file.
//! Measured, not read: the real CLI did it to the captured rollouts in a scratch home, and the
//! collector then reported two of six calls and said nothing about the other four. They are read
//! here through a streaming decoder. A thread that is resumed is decompressed back to `.jsonl`
//! by the CLI before it appends, so a compressed file never grows.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::classify::classify;
use crate::collector::background::Collector;
use crate::collector::billing::Decision;
use crate::collector::billing::{detect, resolve_sticky, BillingSetting, Signals};
use crate::collector::claude_code::{normalize_project_path, session_files};
use crate::collector::opencode::parse_created_at;
use crate::collector::skipped::Skipped;
use crate::helpers::{number, required, string};
use crate::model::{CostStatus, Usage};
use crate::utils::home_dir;
use std::time::Duration;

/// This source's canonical id: the `Collector::name()` it reports, the
/// `[collectors.<id>]` table that configures it, and its key in the source registry.
/// One constant so those can never drift apart.
pub const ID: &str = "codex";

/// Codex's home, `$CODEX_HOME` or `~/.codex`. Session logs live in `sessions/` and
/// `archived_sessions/` beneath it.
pub fn codex_home() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CODEX_HOME") {
        return Some(PathBuf::from(path));
    }
    Some(home_dir()?.join(".codex"))
}

/// Everything a poll must remember about one rollout between reads.
///
/// A byte offset alone is not enough: the model comes from a `turn_context` line and the
/// thread id and directory from `session_meta`, all consumed on an earlier poll. Resuming
/// mid-file with a bare offset would report every later call as `unknown`.
#[derive(Clone, Debug, Default)]
pub struct FileCursor {
    offset: u64,
    session_id: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    /// `total_tokens` of the last `total_token_usage` seen, for the replay guard.
    last_total: Option<u64>,
}

/// Per-file cursors, plus a count of events whose running total did not advance by the call's
/// own figure. Surfaced in the source line: an emission change in the CLI would otherwise
/// under-count silently.
#[derive(Clone, Debug, Default)]
pub struct Cursors {
    files: HashMap<PathBuf, FileCursor>,
    disagreements: u64,
    /// Rollouts and lines the tail had to go around; see `collector::skipped`.
    skipped: Skipped,
}

impl Cursors {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tracked_files(&self) -> usize {
        self.files.len()
    }
}

pub fn load_codex(
    root: Option<&Path>,
    cursors: &mut Cursors,
    decision: &Decision,
) -> Result<(Vec<Usage>, String)> {
    let Some(home) = root.map(Path::to_path_buf).or_else(codex_home) else {
        return Ok((Vec::new(), "Codex: no home directory".into()));
    };
    let roots = [home.join("sessions"), home.join("archived_sessions")];
    if !roots.iter().any(|dir| dir.exists()) {
        return Ok((
            Vec::new(),
            format!("Codex: no session logs at {}", home.display()),
        ));
    }

    let mut usages = Vec::new();
    let mut files = 0usize;
    cursors.skipped.begin_pass();
    for dir in roots.iter().filter(|dir| dir.exists()) {
        for path in rollout_files(dir) {
            files += 1;
            match read_rollout(&path, cursors, decision) {
                Ok(mut found) => usages.append(&mut found),
                // One unreadable or truncated rollout must not sink the whole collector -- but
                // it is counted, because its usage is missing from every total on screen.
                Err(error) => cursors.skipped.unreadable(&path, error),
            }
        }
    }

    let mut source = format!(
        "Codex: {} ({} sessions) · {}",
        home.display(),
        files,
        decision.describe("collectors.codex")
    );
    if cursors.disagreements > 0 {
        source.push_str(&format!(
            " · {} token events disagree with running totals",
            cursors.disagreements
        ));
    }
    if let Some(detail) = cursors.skipped.detail() {
        source.push_str(&format!(" · {detail}"));
    }
    Ok((usages, source))
}

/// Every rollout under `dir`: `*.jsonl`, and `*.jsonl.zst` once the CLI has compressed it.
fn rollout_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_plain(&path) || is_compressed(&path) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn is_plain(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "jsonl")
}

fn is_compressed(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".jsonl.zst"))
}

fn read_rollout(path: &Path, cursors: &mut Cursors, decision: &Decision) -> Result<Vec<Usage>> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut cursor = cursors.files.get(path).cloned().unwrap_or_default();

    let mut usages = Vec::new();
    let mut malformed = 0u64;
    if is_compressed(path) {
        // Cold by construction -- the CLI compresses a rollout nothing has touched for a week and
        // decompresses it before appending -- so it is read once, whole, and its cursor records
        // the compressed size as "done". Decoded as a stream: a long thread is never held in
        // memory, and a frame that fails part-way is an error for the whole file, counted by
        // the caller, rather than a quietly shorter one.
        if cursor.offset == size && cursors.files.contains_key(path) {
            return Ok(Vec::new());
        }
        cursor = FileCursor::default();
        let decoder = ruzstd::decoding::StreamingDecoder::new(file)
            .map_err(|error| anyhow::anyhow!("not a zstd frame: {error}"))?;
        read_lines(
            BufReader::new(decoder),
            &mut cursor,
            path,
            decision,
            &mut cursors.disagreements,
            &mut usages,
            &mut malformed,
        )?;
        cursor.offset = size;
    } else {
        // A shrinking file was rotated or rewritten. Everything remembered about it — not only
        // the offset — describes a file that no longer exists.
        if cursor.offset > size {
            cursor = FileCursor::default();
        }
        let mut file = file;
        file.seek(SeekFrom::Start(cursor.offset))?;
        read_lines(
            BufReader::new(file),
            &mut cursor,
            path,
            decision,
            &mut cursors.disagreements,
            &mut usages,
            &mut malformed,
        )?;
    }

    // Counted only once the cursor is saved: a read that fails part-way keeps the old cursor, and
    // the retry would otherwise count the same lines again.
    cursors.files.insert(path.to_path_buf(), cursor);
    for _ in 0..malformed {
        cursors.skipped.malformed();
    }
    for usage in &usages {
        cursors.skipped.note(usage);
    }
    Ok(usages)
}

/// Feed every complete line of `reader` through the cursor, advancing its offset by what was
/// consumed.
fn read_lines(
    mut reader: impl BufRead,
    cursor: &mut FileCursor,
    path: &Path,
    decision: &Decision,
    disagreements: &mut u64,
    usages: &mut Vec<Usage>,
    malformed: &mut u64,
) -> Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            break;
        }
        // Only advance past complete lines: a partial trailing line is a write in flight and
        // must be re-read next poll, not parsed and skipped.
        if !line.ends_with('\n') {
            break;
        }
        cursor.offset += bytes as u64;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(trimmed) else {
            *malformed += 1;
            continue;
        };
        if let Some(mut usage) = parse_value(&json, cursor, path, disagreements) {
            usage.billing = decision.billing;
            usages.push(usage);
        }
    }
    Ok(())
}

/// Feed one rollout line through the cursor. Context lines update it and yield nothing; a
/// `token_count` yields one usage row.
pub fn parse_line(
    line: &str,
    cursor: &mut FileCursor,
    path: &Path,
    disagreements: &mut u64,
) -> Option<Usage> {
    let json: Value = serde_json::from_str(line.trim()).ok()?;
    parse_value(&json, cursor, path, disagreements)
}

/// `parse_line` over a line already parsed, so the tail parses each line once and can tell a
/// line that is not JSON from one that carries no usage.
fn parse_value(
    json: &Value,
    cursor: &mut FileCursor,
    path: &Path,
    disagreements: &mut u64,
) -> Option<Usage> {
    let kind = json.get("type").and_then(Value::as_str).unwrap_or("");
    let payload = json.get("payload").unwrap_or(json);

    match kind {
        "session_meta" => {
            // First one wins: a fork's file carries its ancestor's meta further down.
            if cursor.session_id.is_none() {
                cursor.session_id = string(payload, &["id", "session_id"]);
            }
            if cursor.cwd.is_none() {
                cursor.cwd = string(payload, &["cwd"]);
            }
            return None;
        }
        "turn_context" => {
            if let Some(model) = string(payload, &["model", "model_slug"]) {
                cursor.model = Some(model);
            }
            if let Some(cwd) = string(payload, &["cwd"]) {
                cursor.cwd = Some(cwd);
            }
            return None;
        }
        _ => {}
    }

    let payload = token_count_payload(json)?;
    // A rate-limit-only update carries `info: null`.
    let info = payload.get("info").filter(|info| info.is_object())?;
    let last = info.get("last_token_usage").filter(|u| u.is_object())?;
    let running_total = info
        .get("total_token_usage")
        .map(|total| number(total, &["total_tokens"]));

    // Cumulative totals that did not move mean nothing new was billed: the same event is
    // re-emitted for rate-limit updates and on resume.
    if let Some(total) = running_total {
        if cursor.last_total == Some(total) {
            return None;
        }
        let call_total = number(last, &["total_tokens"]);
        if let Some(previous) = cursor.last_total {
            if total.saturating_sub(previous) != call_total {
                *disagreements += 1;
            }
        }
        cursor.last_total = Some(total);
    }

    // `input_tokens` and `output_tokens` are on every `last_token_usage` the CLI writes --
    // including the post-compaction estimate, which carries them as zeros. Absent is a format
    // change, not a zero.
    let mut incomplete = false;
    let cache_read = number(last, &["cached_input_tokens"]);
    let input = required(last, &["input_tokens"], &mut incomplete).saturating_sub(cache_read);
    let reasoning = number(last, &["reasoning_output_tokens"]);
    let output = required(last, &["output_tokens"], &mut incomplete).saturating_sub(reasoning);
    // A post-compaction estimate has a total and nothing else; it is not a billed call. One whose
    // fields are gone is a call of unknown size, and is kept and flagged.
    if input == 0 && output == 0 && reasoning == 0 && cache_read == 0 && !incomplete {
        return None;
    }

    let model = cursor.model.clone().unwrap_or_else(|| "unknown".into());
    let provider = "openai".to_string();
    let raw_timestamp = string(json, &["timestamp"]);
    let created = raw_timestamp
        .as_deref()
        .and_then(parse_created_at)
        .unwrap_or(0);
    let event_id = raw_timestamp.map(|ts| {
        format!(
            "codex:{}:{}:{}",
            ts,
            number(last, &["total_tokens"]),
            running_total.unwrap_or(0)
        )
    });

    Some(Usage {
        event_id,
        category: classify(&provider, &model),
        provider,
        model,
        requests: 1,
        input,
        output,
        reasoning,
        cache_read,
        cache_write: 0,
        cost: None,
        // Codex reports no dollar cost; pricing is estimated downstream.
        cost_status: CostStatus::Unavailable,
        billing: Default::default(),
        api_equivalent_cost: None,
        created,
        session_id: cursor
            .session_id
            .clone()
            .or_else(|| session_id_from_filename(path)),
        project: cursor.cwd.as_deref().map(normalize_project_path),
        incomplete,
    })
}

/// The thread UUID from `rollout-<local timestamp>-<uuid>.jsonl`, for a file whose
/// `session_meta` has not been seen. A revert writes `<thread>_<rollout>`; the thread wins.
fn session_id_from_filename(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name
        .strip_suffix(".jsonl.zst")
        .or_else(|| name.strip_suffix(".jsonl"))
        .unwrap_or(name);
    let tail = stem.rsplit('-').take(5).collect::<Vec<_>>();
    if tail.len() != 5 {
        return None;
    }
    let candidate: String = tail.into_iter().rev().collect::<Vec<_>>().join("-");
    let candidate = candidate.split('_').next()?.to_string();
    (candidate.len() == 36).then_some(candidate)
}

/// How many of the most recently written rollouts are searched for `rate_limits`.
///
/// The freshest reading is in whichever thread was used last, and that is nearly always the
/// newest file. More than one is read because Codex keeps a single snapshot per thread and the
/// last header family parsed wins (see `latest_rate_limits`), so a thread on a model with its own
/// limit never writes the default family at all, and the thread before it may hold it.
const RATE_LIMIT_FILES: usize = 3;

/// How much of the end of a rollout is searched.
///
/// This runs on every dashboard refresh and from `--json`, with no state to remember a file by,
/// so it must not re-read a long thread's whole history each time. A `token_count` line is about
/// a kilobyte and one follows every API call, so the last mebibyte holds the thread's newest
/// reading unless the call was followed by more than a mebibyte of tool output -- in which case
/// this finds an older reading or none, and the next API call puts it right.
const RATE_LIMIT_TAIL_BYTES: u64 = 1024 * 1024;

/// The limit id of Codex's default header family (`x-codex-primary-used-percent` and its
/// siblings). An absent `limit_id` means this one: that is the CLI's own default.
pub const DEFAULT_LIMIT_ID: &str = "codex";

/// The slice of a `token_count` event's `rate_limits` block that is read. `credits`,
/// `plan_type` and the rest are not declared, so they are never deserialised.
#[derive(Debug, Deserialize)]
struct RateLimitsBlock {
    limit_id: Option<String>,
    limit_name: Option<String>,
    primary: Option<WindowBlock>,
    secondary: Option<WindowBlock>,
}

#[derive(Debug, Deserialize)]
struct WindowBlock {
    used_percent: Option<f64>,
    window_minutes: Option<i64>,
    /// Unix epoch **seconds**, and an integer: a float or a string here is a format change, and
    /// serde refuses it rather than this guessing at a unit.
    resets_at: Option<i64>,
}

/// One window of one reading, as the CLI wrote it.
#[derive(Clone, Debug, PartialEq)]
pub struct RateLimitWindow {
    /// 0..100, finite and non-negative; anything else was refused.
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<i64>,
    /// The block's `secondary` window rather than its `primary`.
    pub secondary: bool,
}

/// The newest `rate_limits` block found for one limit id.
#[derive(Clone, Debug, PartialEq)]
pub struct RateLimitReading {
    pub limit_id: String,
    pub limit_name: Option<String>,
    /// The event's own `timestamp`, Unix seconds. `None` when the line had none that parses.
    pub at: Option<i64>,
    pub windows: Vec<RateLimitWindow>,
}

/// What `latest_rate_limits` found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RateLimitReadout {
    /// The newest reading per limit id, the default family first.
    pub readings: Vec<RateLimitReading>,
    /// A rollout could not be opened, or a `rate_limits` block was not the shape this reads.
    /// Counted and named by file, never quoted: a rollout line holds the user's prompts.
    pub problems: Vec<String>,
}

/// The newest rate-limit reading Codex wrote, per limit id.
///
/// Every `token_count` event carries the thread's current `rate_limits` snapshot, built from the
/// response's `x-<limit>-primary-*` / `-secondary-*` headers. Measured against codex-cli 0.155.0
/// (`tests/fixtures/codex_capture/`), and not what the source suggests at a
/// glance: **a response can carry several header families and the thread keeps one snapshot, so
/// the last family parsed replaces the others.** A rollout written while a second family was
/// being sent holds that family on every line and the default `codex` family on none. So a
/// reader that takes "the last `rate_limits`" reports some other limit as the account's, and
/// this keys every reading on its `limit_id`.
///
/// API-key use sends no such headers and writes `rate_limits: null`; that is no reading, not a
/// zero.
///
/// **Compressed rollouts are deliberately not searched**, though `load_codex` reads them. The CLI
/// compresses a rollout only once nothing has touched it for seven days, and the longest window
/// it reports is seven days: every reading in a `.jsonl.zst` describes a window that has since
/// reset. There is also no tail to seek to in a zstd frame, so finding one would mean decoding a
/// whole thread on every refresh, from a reader that keeps no state. A home whose newest
/// rollouts are all compressed therefore shows no Codex row, which is the truth about it: no
/// window there is still running.
pub fn latest_rate_limits(home: &Path) -> RateLimitReadout {
    let mut readout = RateLimitReadout::default();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for dir in [home.join("sessions"), home.join("archived_sessions")] {
        for path in session_files(&dir) {
            if let Ok(modified) = std::fs::metadata(&path).and_then(|meta| meta.modified()) {
                files.push((modified, path));
            }
        }
    }
    files.sort_by(|a, b| b.cmp(a));

    let mut newest: HashMap<String, RateLimitReading> = HashMap::new();
    for (_, path) in files.into_iter().take(RATE_LIMIT_FILES) {
        let mut unreadable = 0usize;
        match read_tail_lines(&path, RATE_LIMIT_TAIL_BYTES) {
            Ok(lines) => {
                for line in lines {
                    match rate_limit_reading(&line) {
                        Ok(Some(reading)) => {
                            let slot = newest.get(&reading.limit_id);
                            // `>=`: within one file later lines are newer, and two events can
                            // share a millisecond.
                            if slot.is_none_or(|held| reading.at >= held.at) {
                                newest.insert(reading.limit_id.clone(), reading);
                            }
                        }
                        Ok(None) => {}
                        Err(()) => unreadable += 1,
                    }
                }
            }
            Err(error) => readout
                .problems
                .push(format!("{}: {error}", path.display())),
        }
        if unreadable > 0 {
            readout.problems.push(format!(
                "{}: {unreadable} rate_limits block(s) not in a shape this build reads",
                path.display()
            ));
        }
    }

    let mut readings: Vec<RateLimitReading> = newest.into_values().collect();
    readings.sort_by(|a, b| {
        (a.limit_id != DEFAULT_LIMIT_ID, &a.limit_id)
            .cmp(&(b.limit_id != DEFAULT_LIMIT_ID, &b.limit_id))
    });
    readout.readings = readings;
    readout
}

/// The complete lines in the last `window` bytes of a file.
///
/// Starting mid-file lands mid-line, so the first line is dropped unless the read began at the
/// start. A last line with no newline is a write in flight and is dropped too. Read as bytes and
/// converted lossily: the window can open inside a multi-byte character, and that line is the
/// one being discarded anyway.
fn read_tail_lines(path: &Path, window: u64) -> std::io::Result<Vec<String>> {
    use std::io::Read;
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(window);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(window).read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.split_inclusive('\n').collect();
    if lines.last().is_some_and(|line| !line.ends_with('\n')) {
        lines.pop();
    }
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    Ok(lines
        .into_iter()
        .map(|line| line.trim().to_string())
        .collect())
}

/// The reading on one rollout line. `Ok(None)` is a line with none -- any other kind of line, or
/// a `token_count` whose `rate_limits` is null. `Err` is a block that is there and could not be
/// read, which the caller counts.
fn rate_limit_reading(line: &str) -> std::result::Result<Option<RateLimitReading>, ()> {
    // Most of a rollout is message content. This keeps it from being parsed at all, which is
    // both the cost and the privacy argument.
    if !line.contains("\"rate_limits\"") {
        return Ok(None);
    }
    let Ok(json) = serde_json::from_str::<Value>(line) else {
        return Ok(None);
    };
    let Some(payload) = token_count_payload(&json) else {
        return Ok(None);
    };
    let Some(block) = payload.get("rate_limits").filter(|block| !block.is_null()) else {
        return Ok(None);
    };
    let block: RateLimitsBlock = serde_json::from_value(block.clone()).map_err(|_| ())?;

    let mut windows = Vec::new();
    for (window, secondary) in [(block.primary, false), (block.secondary, true)] {
        let Some(window) = window else { continue };
        // A window with no percentage has nothing to draw, and one that is negative or not
        // finite is not a percentage.
        let Some(used_percent) = window.used_percent.filter(|p| p.is_finite() && *p >= 0.0) else {
            return Err(());
        };
        windows.push(RateLimitWindow {
            used_percent,
            window_minutes: window.window_minutes,
            resets_at: window.resets_at,
            secondary,
        });
    }
    if windows.is_empty() {
        // Credits only, or a family with nothing in it. Not a window and not a fault.
        return Ok(None);
    }
    let limit_id = block
        .limit_id
        .map(|id| id.trim().to_ascii_lowercase())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| DEFAULT_LIMIT_ID.to_string());
    Ok(Some(RateLimitReading {
        limit_id,
        limit_name: block
            .limit_name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()),
        at: string(&json, &["timestamp"])
            .as_deref()
            .and_then(parse_created_at),
        windows,
    }))
}

/// The payload of a `token_count` event, wherever this writer put it.
fn token_count_payload(json: &Value) -> Option<&Value> {
    let kind = json.get("type").and_then(Value::as_str).unwrap_or("");
    let payload = json.get("payload").unwrap_or(json);
    // Older writers nested the item one level deeper under `response_item`.
    let payload = if kind == "response_item" {
        payload.get("payload").unwrap_or(payload)
    } else {
        payload
    };
    (payload.get("type").and_then(Value::as_str) == Some("token_count")).then_some(payload)
}

pub struct CodexCollector {
    pub root: Option<PathBuf>,
    pub interval_secs: u64,
    /// Per-file cursors: byte offset plus the model, thread and directory in force there.
    pub cursors: Cursors,
    pub billing: BillingSetting,
    pub omarchy_dir: Option<PathBuf>,
    pub decision: Option<Decision>,
}

impl Collector for CodexCollector {
    fn name(&self) -> &str {
        ID
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        let tier = self
            .omarchy_dir
            .as_deref()
            .and_then(|dir| crate::omarchy::tier_label_for(dir, "codex"));
        let fresh = detect(
            "codex",
            self.billing,
            &Signals {
                claude_json: None,
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: tier.as_deref(),
            },
        );
        let decision = resolve_sticky("codex", self.decision.take(), fresh);
        self.decision = Some(decision.clone());
        let (usages, _) = load_codex(self.root.as_deref(), &mut self.cursors, &decision)?;
        Ok(usages)
    }
    fn warning(&self) -> Option<String> {
        self.cursors.skipped.warning()
    }
}

/// One-shot read for the source registry.
pub(crate) fn read(
    roots: &crate::collector::SourceRoots,
) -> crate::collector::registry::SourceRead {
    let decision = roots.codex_decision();
    let (usages, status) = load_codex(roots.codex_dir.as_deref(), &mut Cursors::new(), &decision)
        .unwrap_or_else(|error| (Vec::new(), format!("Codex: unavailable ({})", error)));
    let path = roots.codex_dir.clone().or_else(codex_home);
    Ok((
        crate::collector::SourceReport {
            id: ID,
            present: path.as_deref().is_some_and(Path::exists),
            path,
            rows: usages.len(),
            status,
            detail: Some(decision.describe("collectors.codex")),
        },
        usages,
    ))
}

/// A background collector for the same source.
pub(crate) fn collector(
    roots: &crate::collector::SourceRoots,
    interval_secs: u64,
) -> Box<dyn Collector> {
    Box::new(CodexCollector {
        root: roots.codex_dir.clone(),
        interval_secs,
        cursors: Cursors::new(),
        billing: roots.codex_billing,
        omarchy_dir: roots.omarchy_signal_dir(),
        decision: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Billing;
    use std::io::Write;

    const META: &str = r#"{"timestamp":"2026-08-18T10:00:00.117Z","type":"session_meta","payload":{"session_id":"0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90","id":"0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90","timestamp":"2026-08-18T10:00:00.117Z","cwd":"/home/dev/proj/","originator":"codex_cli_rs","cli_version":"0.149.0","source":"cli","model_provider":"openai","base_instructions":{"text":"AWS_SECRET_ACCESS_KEY=hunter2"},"history_mode":"legacy"}}"#;
    const TURN: &str = r#"{"timestamp":"2026-08-18T10:00:00.402Z","type":"turn_context","payload":{"cwd":"/home/dev/proj","approval_policy":"on-request","sandbox_policy":{"type":"workspace-write"},"model":"gpt-5-codex","effort":"medium","summary":"auto"}}"#;
    const USER: &str = r#"{"timestamp":"2026-08-18T10:00:00.410Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"AWS_SECRET_ACCESS_KEY=hunter2"}]}}"#;
    const COUNT_1: &str = r#"{"timestamp":"2026-08-18T10:00:04.876Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1200,"cached_input_tokens":800,"cache_write_input_tokens":0,"output_tokens":340,"reasoning_output_tokens":100,"total_tokens":1540},"last_token_usage":{"input_tokens":1200,"cached_input_tokens":800,"cache_write_input_tokens":0,"output_tokens":340,"reasoning_output_tokens":100,"total_tokens":1540},"model_context_window":272000},"rate_limits":{"primary":{"used_percent":12.5,"window_minutes":300,"resets_at":1787422800},"plan_type":"plus"}}}"#;
    const TOOL_OUT: &str = r#"{"timestamp":"2026-08-18T10:00:04.901Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":"AWS_SECRET_ACCESS_KEY=hunter2"}}"#;
    const COUNT_2: &str = r#"{"timestamp":"2026-08-18T10:00:07.334Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2400,"cached_input_tokens":1900,"cache_write_input_tokens":0,"output_tokens":400,"reasoning_output_tokens":110,"total_tokens":2800},"last_token_usage":{"input_tokens":1200,"cached_input_tokens":1100,"cache_write_input_tokens":0,"output_tokens":60,"reasoning_output_tokens":10,"total_tokens":1260},"model_context_window":272000},"rate_limits":null}}"#;
    /// Rate-limit-only refresh: same totals, `info` present but unchanged.
    const COUNT_2_AGAIN: &str = COUNT_2;
    const LIMITS_ONLY: &str = r#"{"timestamp":"2026-08-18T10:00:08.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":13.0}}}}"#;
    const COMPACTION_ESTIMATE: &str = r#"{"timestamp":"2026-08-18T10:00:09.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":2400,"cached_input_tokens":1900,"cache_write_input_tokens":0,"output_tokens":400,"reasoning_output_tokens":110,"total_tokens":2801},"last_token_usage":{"input_tokens":0,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0,"total_tokens":2801},"model_context_window":272000},"rate_limits":null}}"#;

    fn per_token() -> Decision {
        Decision {
            billing: Billing::PerToken,
            tier: None,
            reason: "config",
        }
    }

    fn rollout_path(dir: &Path) -> PathBuf {
        let day = dir.join("sessions").join("2026").join("08").join("18");
        std::fs::create_dir_all(&day).unwrap();
        day.join("rollout-2026-08-18T10-00-00-0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90.jsonl")
    }

    fn parse_all(lines: &[&str]) -> Vec<Usage> {
        let mut cursor = FileCursor::default();
        let mut disagreements = 0;
        let path =
            Path::new("/x/rollout-2026-08-18T10-00-00-0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90.jsonl");
        lines
            .iter()
            .filter_map(|line| parse_line(line, &mut cursor, path, &mut disagreements))
            .collect()
    }

    #[test]
    fn a_token_count_after_context_lines_yields_one_split_usage_row() {
        let rows = parse_all(&[META, TURN, USER, COUNT_1]);
        assert_eq!(rows.len(), 1);
        let u = &rows[0];
        assert_eq!(u.provider, "openai");
        assert_eq!(u.model, "gpt-5-codex");
        assert_eq!(
            u.input, 400,
            "cached tokens sit inside input_tokens and are split out"
        );
        assert_eq!(u.cache_read, 800);
        assert_eq!(
            u.output, 240,
            "reasoning sits inside output_tokens and is split out"
        );
        assert_eq!(u.reasoning, 100);
        assert_eq!(u.cache_write, 0);
        assert_eq!(u.total_tokens(), 1540, "the split never changes the total");
        assert_eq!(
            u.session_id.as_deref(),
            Some("0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90")
        );
        assert_eq!(u.project.as_deref(), Some("/home/dev/proj"));
        assert_eq!(u.created, 1_787_047_204); // 2026-08-18T10:00:04Z
        assert_eq!(
            u.event_id.as_deref(),
            Some("codex:2026-08-18T10:00:04.876Z:1540:1540")
        );
        assert_eq!(u.cost_status, CostStatus::Unavailable);
    }

    #[test]
    fn calls_are_summed_from_last_token_usage_not_from_the_running_total() {
        // Summing `total_token_usage` snapshots grows quadratically; the two rows must add up
        // to the final running total, not to the sum of both snapshots.
        let rows = parse_all(&[META, TURN, COUNT_1, TOOL_OUT, COUNT_2]);
        assert_eq!(rows.len(), 2);
        let total: u64 = rows.iter().map(Usage::total_tokens).sum();
        assert_eq!(total, 2800);
    }

    #[test]
    fn unchanged_totals_limit_only_updates_and_compaction_estimates_are_skipped() {
        let rows = parse_all(&[
            META,
            TURN,
            COUNT_1,
            COUNT_2,
            COUNT_2_AGAIN,
            LIMITS_ONLY,
            COMPACTION_ESTIMATE,
        ]);
        assert_eq!(rows.len(), 2, "{rows:?}");
    }

    #[test]
    fn a_running_total_that_does_not_advance_by_the_call_is_counted_as_a_disagreement() {
        let mut cursor = FileCursor::default();
        let mut disagreements = 0;
        let path = Path::new("/x/r.jsonl");
        for line in [META, TURN, COUNT_1, COMPACTION_ESTIMATE] {
            parse_line(line, &mut cursor, path, &mut disagreements);
        }
        // The estimate advanced the total by 1261 while claiming a call of 2801.
        assert_eq!(disagreements, 1);
    }

    #[test]
    fn no_message_content_is_retained() {
        let rows = parse_all(&[META, TURN, USER, COUNT_1, TOOL_OUT, COUNT_2]);
        let rendered = format!("{rows:?}");
        assert!(
            !rendered.contains("hunter2") && !rendered.contains("AWS_SECRET"),
            "message content leaked into a usage record: {rendered}"
        );
    }

    #[test]
    fn the_first_session_meta_names_a_forked_file_and_copied_history_dedups() {
        // A fork copies the ancestor's meta and token counts verbatim, timestamps included.
        let fork_meta = META.replace(
            "0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90",
            "ffffffff-0000-4000-8000-000000000001",
        );
        let original = parse_all(&[META, TURN, COUNT_1]);
        let forked = parse_all(&[&fork_meta, META, TURN, COUNT_1]);
        assert_eq!(
            forked[0].session_id.as_deref(),
            Some("ffffffff-0000-4000-8000-000000000001")
        );
        assert_eq!(
            forked[0].event_id, original[0].event_id,
            "the copied call must dedup against the original wherever it is read"
        );
    }

    #[test]
    fn a_file_without_session_meta_takes_its_id_from_the_filename() {
        let rows = parse_all(&[TURN, COUNT_1]);
        assert_eq!(
            rows[0].session_id.as_deref(),
            Some("0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90")
        );
        assert_eq!(
            session_id_from_filename(Path::new(
                "rollout-2026-08-18T10-00-00-0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90_abcd.jsonl"
            ))
            .as_deref(),
            Some("0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90"),
            "a revert suffix is not part of the thread id"
        );
        assert_eq!(session_id_from_filename(Path::new("notes.jsonl")), None);
    }

    #[test]
    fn a_call_before_any_turn_context_is_attributed_to_an_unknown_model() {
        let rows = parse_all(&[META, COUNT_1]);
        assert_eq!(rows[0].model, "unknown");
    }

    #[test]
    fn cursor_state_survives_between_polls() {
        // Poll one consumes the context lines only; poll two sees an appended call and must
        // still know the model and the directory. A bare byte offset cannot.
        let dir = tempfile::TempDir::new().unwrap();
        let path = rollout_path(dir.path());
        std::fs::write(&path, format!("{META}\n{TURN}\n")).unwrap();

        let mut cursors = Cursors::new();
        let (first, _) = load_codex(Some(dir.path()), &mut cursors, &per_token()).unwrap();
        assert!(first.is_empty());
        assert_eq!(cursors.tracked_files(), 1);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(file, "{COUNT_1}").unwrap();
        let (second, _) = load_codex(Some(dir.path()), &mut cursors, &per_token()).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].model, "gpt-5-codex");
        assert_eq!(second[0].project.as_deref(), Some("/home/dev/proj"));

        // Nothing new: no work.
        let (third, _) = load_codex(Some(dir.path()), &mut cursors, &per_token()).unwrap();
        assert!(third.is_empty());
    }

    /// An unreadable rollout was `Err(_) => continue` and a line that was not JSON was `.ok()?`,
    /// with no count of either, so a Codex format or encoding change read as less usage.
    #[test]
    fn skipped_rollouts_and_lines_are_counted_once_and_reported() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = rollout_path(dir.path());
        std::fs::write(&path, format!("{META}\n{TURN}\n{{\"type\":\n{COUNT_1}\n")).unwrap();
        let broken = path.with_file_name("rollout-broken.jsonl");
        std::fs::write(&broken, b"{\"type\":\"session_meta\"}\xff\n").unwrap();

        let mut cursors = Cursors::new();
        let (rows, status) = load_codex(Some(dir.path()), &mut cursors, &per_token()).unwrap();
        assert_eq!(rows.len(), 1, "the readable call survives");
        assert!(
            status.contains("1 file(s) unreadable, 1 malformed record(s) skipped")
                && status.contains("rollout-broken.jsonl"),
            "{status}"
        );

        load_codex(Some(dir.path()), &mut cursors, &per_token()).unwrap();
        assert_eq!(
            cursors.skipped.warning().as_deref(),
            Some("1 file(s) unreadable, 1 malformed record(s) skipped"),
            "a retry must not count the malformed line twice"
        );
    }

    /// `output_tokens` gone from `last_token_usage` is a format change: the call is kept and
    /// flagged. The post-compaction estimate carries the fields as zeros and is still dropped.
    #[test]
    fn a_call_missing_a_token_count_is_flagged_and_a_compaction_estimate_is_still_dropped() {
        let renamed = COUNT_1.replacen(
            r#""output_tokens":340,"reasoning_output_tokens":100,"total_tokens":1540},"model_context_window""#,
            r#""completion_tokens":340,"reasoning_output_tokens":100,"total_tokens":1540},"model_context_window""#,
            1,
        );
        assert_ne!(renamed, COUNT_1, "the fixture line changed shape");
        let rows = parse_all(&[META, TURN, &renamed]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].incomplete);
        assert!(parse_all(&[META, TURN, COUNT_1])
            .iter()
            .all(|u| !u.incomplete));
        assert!(parse_all(&[META, TURN, COMPACTION_ESTIMATE]).is_empty());
    }

    #[test]
    fn a_partial_line_waits_and_a_shrunken_file_restarts_with_a_fresh_cursor() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = rollout_path(dir.path());
        std::fs::write(&path, format!("{META}\n{TURN}\n{COUNT_1}")).unwrap();
        let mut cursors = Cursors::new();
        let (first, _) = load_codex(Some(dir.path()), &mut cursors, &per_token()).unwrap();
        assert!(first.is_empty(), "consumed an incomplete line");

        std::fs::write(&path, format!("{META}\n{TURN}\n{COUNT_1}\n{COUNT_2}\n")).unwrap();
        let (second, _) = load_codex(Some(dir.path()), &mut cursors, &per_token()).unwrap();
        assert_eq!(second.len(), 2);

        // Rewritten shorter: start over, forgetting the old model and totals too.
        std::fs::write(&path, format!("{META}\n{COUNT_1}\n")).unwrap();
        let (third, _) = load_codex(Some(dir.path()), &mut cursors, &per_token()).unwrap();
        assert_eq!(third.len(), 1);
        assert_eq!(
            third[0].model, "unknown",
            "the old turn_context must not leak across a rewrite"
        );
    }

    #[test]
    fn rows_carry_the_billing_decision_and_the_source_line_names_it() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            rollout_path(dir.path()),
            format!("{META}\n{TURN}\n{COUNT_1}\n"),
        )
        .unwrap();
        let plan = Decision {
            billing: Billing::Subscription,
            tier: Some("plus".into()),
            reason: "omarchy record",
        };
        let (rows, source) = load_codex(Some(dir.path()), &mut Cursors::new(), &plan).unwrap();
        assert_eq!(rows[0].billing, Billing::Subscription);
        assert!(source.contains("subscription plus"), "{source}");
        assert!(source.starts_with("Codex: "), "{source}");
    }

    #[test]
    fn both_session_roots_are_scanned_and_a_missing_home_is_not_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let archived = dir
            .path()
            .join("archived_sessions")
            .join("2026")
            .join("07")
            .join("01");
        std::fs::create_dir_all(&archived).unwrap();
        std::fs::write(
            archived.join("rollout-2026-07-01T09-00-00-aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee.jsonl"),
            format!("{META}\n{TURN}\n{COUNT_1}\n"),
        )
        .unwrap();
        std::fs::write(
            rollout_path(dir.path()),
            format!("{META}\n{TURN}\n{COUNT_2}\n"),
        )
        .unwrap();
        let (rows, source) =
            load_codex(Some(dir.path()), &mut Cursors::new(), &per_token()).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(source.contains("(2 sessions)"), "{source}");

        let (rows, source) = load_codex(
            Some(Path::new("/nonexistent/codex")),
            &mut Cursors::new(),
            &per_token(),
        )
        .unwrap();
        assert!(rows.is_empty());
        assert!(source.contains("no session logs"), "{source}");
    }

    // ---- compressed rollouts -----------------------------------------------------------

    /// The redacted capture, compressed as the CLI compresses one: `zstd -3 --no-check`, one
    /// frame with its content size. The real `.jsonl.zst` the CLI wrote cannot be committed --
    /// it is the unredacted rollout -- but it was read by this code and gave the same six calls
    /// as the plain files it replaced, and its frame header matches this one field for field.
    fn compressed_home() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/codex_compressed"
        ))
    }

    #[test]
    fn a_compressed_rollout_gives_the_rows_the_plain_one_gave() {
        let plain_home = capture_home(&[CAPTURE_OTHER_FAMILY]);
        let (plain, _) =
            load_codex(Some(plain_home.path()), &mut Cursors::new(), &per_token()).unwrap();
        let mut cursors = Cursors::new();
        let (compressed, status) =
            load_codex(Some(&compressed_home()), &mut cursors, &per_token()).unwrap();
        assert_eq!(plain.len(), 2);
        assert_eq!(
            format!("{compressed:?}"),
            format!("{plain:?}"),
            "same calls, same ids, same thread"
        );
        assert!(status.contains("(1 sessions)"), "{status}");
        assert!(!status.contains("unreadable"), "{status}");

        // Cold files are read once. A second pass finds the cursor and decodes nothing.
        let (again, _) = load_codex(Some(&compressed_home()), &mut cursors, &per_token()).unwrap();
        assert!(again.is_empty());
    }

    #[test]
    fn a_compressed_rollout_that_does_not_decode_is_counted_and_named() {
        let dir = tempfile::TempDir::new().unwrap();
        let day = dir.path().join("sessions/2026/09/18");
        std::fs::create_dir_all(&day).unwrap();
        let source = compressed_home().join(
            "sessions/2026/09/18/rollout-2026-09-18T10-28-44-01a0b590-400c-7d90-9713-4cff3fb43730.jsonl.zst",
        );
        let bytes = std::fs::read(source).unwrap();
        // Cut off mid-frame, and one that was never zstd at all.
        std::fs::write(day.join("rollout-cut.jsonl.zst"), &bytes[..bytes.len() / 2]).unwrap();
        std::fs::write(day.join("rollout-text.jsonl.zst"), format!("{META}\n")).unwrap();

        let (rows, status) =
            load_codex(Some(dir.path()), &mut Cursors::new(), &per_token()).unwrap();
        assert!(
            rows.is_empty(),
            "half a frame is not half a rollout: {rows:#?}"
        );
        assert!(status.contains("2 file(s) unreadable"), "{status}");
    }

    #[test]
    fn a_compressed_file_without_session_meta_still_takes_its_id_from_its_name() {
        let path = Path::new(
            "/x/rollout-2026-09-18T10-28-44-01a0b590-400c-7d90-9713-4cff3fb43730.jsonl.zst",
        );
        assert_eq!(
            session_id_from_filename(path).as_deref(),
            Some("01a0b590-400c-7d90-9713-4cff3fb43730")
        );
    }

    // ---- rate limits -------------------------------------------------------------------

    /// The two rollouts the real CLI wrote (`scripts/codex-standin.py`), by the second in
    /// their names.
    const CAPTURE_OTHER_FAMILY: &str = "10-28-44";
    const CAPTURE_DEFAULT_FAMILY: &str = "10-29-27";

    /// Copy the captured rollouts whose names contain one of `which` into a fresh Codex home,
    /// with modification times one minute apart in the order given. A checkout gives every file
    /// the same instant, more or less, and the reader sorts on it.
    fn capture_home(which: &[&str]) -> tempfile::TempDir {
        let source = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/codex_capture/sessions/2026/09/18"
        ));
        let home = tempfile::tempdir().unwrap();
        let sessions = home.path().join("sessions/2026/09/18");
        std::fs::create_dir_all(&sessions).unwrap();
        for (index, needle) in which.iter().enumerate() {
            let from = std::fs::read_dir(&source)
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .find(|path| path.to_string_lossy().contains(needle))
                .expect("the captured rollout");
            let to = sessions.join(from.file_name().unwrap());
            std::fs::copy(&from, &to).unwrap();
            let at = std::time::UNIX_EPOCH + Duration::from_secs(1_789_752_000 + 60 * index as u64);
            File::options()
                .write(true)
                .open(&to)
                .unwrap()
                .set_modified(at)
                .unwrap();
        }
        home
    }

    #[test]
    fn the_captured_rollouts_give_one_reading_per_limit_id() {
        let home = capture_home(&[CAPTURE_OTHER_FAMILY, CAPTURE_DEFAULT_FAMILY]);
        let readout = latest_rate_limits(home.path());
        assert!(readout.problems.is_empty(), "{:?}", readout.problems);
        let ids: Vec<&str> = readout
            .readings
            .iter()
            .map(|r| r.limit_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["codex", "codex_other"],
            "the default family first"
        );

        let default = &readout.readings[0];
        // The second event of the thread, not the first: 13.0, where the first said 12.0.
        assert_eq!(
            default.windows,
            vec![
                RateLimitWindow {
                    used_percent: 13.0,
                    window_minutes: Some(300),
                    resets_at: Some(1_789_763_367),
                    secondary: false,
                },
                RateLimitWindow {
                    used_percent: 40.5,
                    window_minutes: Some(10_080),
                    resets_at: Some(1_790_098_167),
                    secondary: true,
                },
            ]
        );
        assert_eq!(default.at, parse_created_at("2026-09-18T17:29:27.475Z"));
        assert_eq!(default.limit_name, None);

        let other = &readout.readings[1];
        assert_eq!(other.limit_name.as_deref(), Some("codex_other"));
        assert_eq!(other.windows.len(), 1, "its `secondary` is null");
        assert_eq!(other.windows[0].window_minutes, Some(60));
    }

    /// What the capture found and the source did not suggest. The stand-in sent both header
    /// families on every response of this thread, and the CLI wrote only the second into the
    /// rollout -- on every line. Read as "the thread's rate limits", a 2%-used one-hour window
    /// would be shown as the account's.
    #[test]
    fn a_thread_that_only_wrote_another_family_is_not_read_as_the_default_one() {
        let home = capture_home(&[CAPTURE_OTHER_FAMILY]);
        let readout = latest_rate_limits(home.path());
        let ids: Vec<&str> = readout
            .readings
            .iter()
            .map(|r| r.limit_id.as_str())
            .collect();
        assert_eq!(ids, vec!["codex_other"]);
    }

    #[test]
    fn a_newer_file_does_not_hide_a_family_it_never_wrote() {
        // The other-family thread is the newer file here; the default family is still found in
        // the one before it.
        let home = capture_home(&[CAPTURE_DEFAULT_FAMILY, CAPTURE_OTHER_FAMILY]);
        let ids: Vec<String> = latest_rate_limits(home.path())
            .readings
            .into_iter()
            .map(|r| r.limit_id)
            .collect();
        assert_eq!(ids, vec!["codex", "codex_other"]);
    }

    #[test]
    fn a_null_block_is_no_reading_and_an_absent_limit_id_is_the_default_family() {
        assert_eq!(rate_limit_reading(COUNT_2), Ok(None), "rate_limits: null");
        assert_eq!(rate_limit_reading(USER), Ok(None));
        let reading = rate_limit_reading(COUNT_1)
            .unwrap()
            .expect("a primary window");
        assert_eq!(reading.limit_id, DEFAULT_LIMIT_ID);
        assert_eq!(reading.windows[0].used_percent, 12.5);
        // `info: null`, which yields no usage row, still carries a reading.
        let limits_only = rate_limit_reading(LIMITS_ONLY).unwrap().expect("a reading");
        assert_eq!(limits_only.windows[0].window_minutes, None);
    }

    #[test]
    fn a_block_in_another_shape_is_counted_not_read() {
        let float_reset = r#"{"timestamp":"2026-08-18T10:00:08.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":13.0,"resets_at":1787422800.5}}}}"#;
        let text_percent = r#"{"timestamp":"2026-08-18T10:00:08.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":"13"}}}}"#;
        let negative = r#"{"timestamp":"2026-08-18T10:00:08.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":-1.0}}}}"#;
        let no_percent = r#"{"timestamp":"2026-08-18T10:00:08.000Z","type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"window_minutes":300}}}}"#;
        for line in [float_reset, text_percent, negative, no_percent] {
            assert_eq!(rate_limit_reading(line), Err(()), "{line}");
        }

        let home = tempfile::tempdir().unwrap();
        let path = rollout_path(home.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{float_reset}\n{COUNT_1}\n{negative}\n")).unwrap();
        let readout = latest_rate_limits(home.path());
        assert_eq!(
            readout.readings.len(),
            1,
            "the readable block is still read"
        );
        assert_eq!(readout.problems.len(), 1);
        assert!(
            readout.problems[0].contains("2 rate_limits block(s)"),
            "{:?}",
            readout.problems
        );
        assert!(
            !readout.problems[0].contains("used_percent"),
            "a problem quotes no line"
        );
    }

    #[test]
    fn the_tail_drops_the_line_it_opened_inside_and_the_one_still_being_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tail.jsonl");
        std::fs::write(&path, "first line\nsecond\nthird\nfourth, unfinished").unwrap();
        // The whole file: only the unfinished line goes.
        assert_eq!(
            read_tail_lines(&path, 1024).unwrap(),
            vec!["first line", "second", "third"]
        );
        // A window opening inside "second": that line goes as well.
        assert_eq!(read_tail_lines(&path, 28).unwrap(), vec!["third"]);
        // A window opening inside a multi-byte character is not an error.
        std::fs::write(&path, "caf\u{e9}\nlast\n").unwrap();
        assert_eq!(read_tail_lines(&path, 6).unwrap(), vec!["last"]);
    }

    /// Stated, not an oversight: see `latest_rate_limits`. A compressed rollout is a week cold,
    /// and no window in it is still running.
    #[test]
    fn a_compressed_rollout_is_not_searched_for_windows() {
        let readout = latest_rate_limits(&compressed_home());
        assert_eq!(readout, RateLimitReadout::default());
    }

    #[test]
    fn a_home_with_no_rollouts_has_no_reading_and_no_problem() {
        let readout = latest_rate_limits(Path::new("/nonexistent/codex-home"));
        assert_eq!(readout, RateLimitReadout::default());
    }

    #[test]
    fn codex_usage_is_priced_by_the_bundled_table_and_the_split_is_cost_neutral() {
        // Reasoning is billed at the output rate when no distinct rate is published
        // (pricing.rs), so splitting it out changes the buckets, never the bill.
        let mut split = parse_all(&[META, TURN, COUNT_1]).remove(0);
        let mut unsplit = split.clone();
        unsplit.output += unsplit.reasoning;
        unsplit.reasoning = 0;
        let engine = crate::pricing::PricingEngine::bundled();
        crate::pricing::apply_estimated_pricing(std::slice::from_mut(&mut split), &engine);
        crate::pricing::apply_estimated_pricing(std::slice::from_mut(&mut unsplit), &engine);
        assert_eq!(split.cost_status, CostStatus::Estimated);
        assert!(split.cost.is_some_and(|c| c > 0.0));
        assert_eq!(split.cost, unsplit.cost);
    }
}
