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
//! Files compressed to `.jsonl.zst` by the CLI's history compaction are not read.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::classify::classify;
use crate::collector::billing::Decision;
use crate::collector::claude_code::{normalize_project_path, session_files};
use crate::collector::opencode::parse_created_at;
use crate::helpers::{number, string};
use crate::model::{CostStatus, Usage};
use crate::utils::home_dir;

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
    for dir in roots.iter().filter(|dir| dir.exists()) {
        for path in session_files(dir) {
            files += 1;
            match read_rollout(&path, cursors, decision) {
                Ok(mut found) => usages.append(&mut found),
                // One unreadable or truncated rollout must not sink the whole collector.
                Err(_) => continue,
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
    Ok((usages, source))
}

fn read_rollout(path: &Path, cursors: &mut Cursors, decision: &Decision) -> Result<Vec<Usage>> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut cursor = cursors.files.get(path).cloned().unwrap_or_default();

    // A shrinking file was rotated or rewritten. Everything remembered about it — not only
    // the offset — describes a file that no longer exists.
    if cursor.offset > size {
        cursor = FileCursor::default();
    }
    file.seek(SeekFrom::Start(cursor.offset))?;

    let mut reader = BufReader::new(file);
    let mut usages = Vec::new();
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
        if let Some(mut usage) = parse_line(&line, &mut cursor, path, &mut cursors.disagreements) {
            usage.billing = decision.billing;
            usages.push(usage);
        }
    }

    cursors.files.insert(path.to_path_buf(), cursor);
    Ok(usages)
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
    let kind = json.get("type").and_then(Value::as_str).unwrap_or("");
    let payload = json.get("payload").unwrap_or(&json);

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

    // Older writers nested the item one level deeper under `response_item`.
    let payload = if kind == "response_item" {
        payload.get("payload").unwrap_or(payload)
    } else {
        payload
    };
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
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

    let cache_read = number(last, &["cached_input_tokens"]);
    let input = number(last, &["input_tokens"]).saturating_sub(cache_read);
    let reasoning = number(last, &["reasoning_output_tokens"]);
    let output = number(last, &["output_tokens"]).saturating_sub(reasoning);
    // A post-compaction estimate has a total and nothing else; it is not a billed call.
    if input == 0 && output == 0 && reasoning == 0 && cache_read == 0 {
        return None;
    }

    let model = cursor.model.clone().unwrap_or_else(|| "unknown".into());
    let provider = "openai".to_string();
    let raw_timestamp = string(&json, &["timestamp"]);
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
    })
}

/// The thread UUID from `rollout-<local timestamp>-<uuid>.jsonl`, for a file whose
/// `session_meta` has not been seen. A revert writes `<thread>_<rollout>`; the thread wins.
fn session_id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let tail = stem.rsplit('-').take(5).collect::<Vec<_>>();
    if tail.len() != 5 {
        return None;
    }
    let candidate: String = tail.into_iter().rev().collect::<Vec<_>>().join("-");
    let candidate = candidate.split('_').next()?.to_string();
    (candidate.len() == 36).then_some(candidate)
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
