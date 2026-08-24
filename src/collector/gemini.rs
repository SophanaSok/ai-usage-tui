//! Gemini CLI usage, read from its OpenTelemetry log file.
//!
//! Unlike Claude Code and Codex, **Gemini CLI persists no usage anywhere by default.** Session
//! token totals live in UI state (`sessionStats.lastPromptTokenCount`) and are gone when the
//! process exits; saved chats under `<project temp>/chats` hold conversation history and an
//! auth type, and no token counts at all. The only durable record is the telemetry log, and it
//! is written only when the user turns it on:
//!
//! ```json
//! // ~/.gemini/settings.json
//! { "telemetry": { "enabled": true, "target": "local", "outfile": "~/.gemini/telemetry.json" } }
//! ```
//!
//! So this collector is idle until the user opts in, which `--doctor` says out loud rather than
//! reporting an empty source. This tool never writes that setting: editing another tool's config
//! is not something anything else here does.
//!
//! # Format
//!
//! Derived from the CLI's own source (`@google/gemini-cli` 0.56.0, `packages/core`), not guessed.
//! Two details would have broken a guessed parser:
//!
//! - `FileExporter.serialize` is `JSON.stringify(data, 2) + "\n"`, so the file is a stream of
//!   **pretty-printed** JSON objects concatenated. It is not JSONL and cannot be split on
//!   newlines.
//! - The interesting record has `attributes["event.name"] == "gemini_cli.api_response"`, with
//!   token counts as sibling attributes. The OTLP SDK wraps it with `body`, `hrTime`,
//!   `severityNumber` and friends, so everything is read out of `attributes`.
//!
//! # Privacy
//!
//! `attributes` may also carry `response_text` — the model's actual output — when the user has
//! `telemetry.logPrompts` on. Only the usage fields and identifiers below are read; nothing
//! else is looked at, and a test plants a credential in `response_text` and fails if it reaches
//! a usage record.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use crate::collector::background::Collector;
use crate::collector::billing::{detect, resolve_sticky, BillingSetting, Signals};
use crate::model::{Billing, Category, CostStatus, Usage};

pub const ID: &str = "gemini";

/// The event this collector cares about. Every other telemetry record is skipped.
const API_RESPONSE: &str = "gemini_cli.api_response";

/// Where the telemetry log lives, when the user has pointed Gemini at one.
///
/// `GEMINI_TELEMETRY_OUTFILE` is Gemini's own environment knob and wins when set: if the user
/// told Gemini to write there, that is where it is. Otherwise the conventional location this
/// project documents, under the Gemini home.
pub fn telemetry_path(root: Option<&Path>) -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GEMINI_TELEMETRY_OUTFILE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    let home = match root {
        Some(root) => root.to_path_buf(),
        None => crate::utils::home_dir()?.join(".gemini"),
    };
    Some(home.join("telemetry.json"))
}

/// Byte offset already consumed, so a poll reads only what was appended.
///
/// The offset advances only past **complete** top-level objects. Gemini appends a multi-line
/// pretty-printed object per record, so a poll can easily land mid-object; consuming to the end
/// of the file would swallow the tail of a record that is still being written.
#[derive(Clone, Debug, Default)]
pub struct Offsets {
    files: HashMap<PathBuf, u64>,
}

impl Offsets {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Split a telemetry stream into complete top-level JSON objects.
///
/// Returns each object's text and the byte offset just past the last complete one. Brace
/// counting is string- and escape-aware: a `{` inside `"body": "API response from {model}"`
/// must not open a nesting level.
fn complete_objects(text: &str) -> (Vec<&str>, usize) {
    let bytes = text.as_bytes();
    let mut objects = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    let mut in_string = false;
    let mut escaped = false;
    let mut consumed = 0usize;

    for (index, &byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(open) = start.take() {
                        objects.push(&text[open..=index]);
                        consumed = index + 1;
                    }
                }
            }
            _ => {}
        }
    }
    (objects, consumed)
}

/// One usage row from a telemetry record, or `None` if it is not an API response.
///
/// `billing` is filled in by the caller from the source-level decision; `auth_type` on the
/// record is a stronger signal than anything else available and is fed into that decision.
pub fn parse_record(value: &Value, decision_billing: Billing) -> Option<Usage> {
    let attributes = value.get("attributes")?.as_object()?;
    if attributes.get("event.name")?.as_str()? != API_RESPONSE {
        return None;
    }
    // A failed call still emits a record. It burned no output tokens worth reporting and would
    // otherwise show up as a free request against the model.
    if let Some(status) = attributes.get("status_code").and_then(Value::as_i64) {
        if !(200..300).contains(&status) {
            return None;
        }
    }

    let model = attributes.get("model")?.as_str()?.trim().to_string();
    if model.is_empty() {
        return None;
    }
    let count = |key: &str| -> u64 {
        attributes
            .get(key)
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0) as u64
    };

    // Google's `cachedContentTokenCount` is a *subset* of `promptTokenCount`, unlike Anthropic's
    // cache-read count which is reported alongside its input. Subtracting keeps the buckets
    // disjoint, so pricing a cache read at the cache rate does not also bill it as fresh input.
    // Saturating, because a record that reports more cache than prompt is malformed and must
    // not wrap into an enormous input count.
    let prompt = count("input_token_count");
    let cache_read = count("cached_content_token_count").min(prompt);
    let input = prompt.saturating_sub(cache_read);

    // `tool_token_count` is deliberately not added: Google counts tool-use prompt tokens inside
    // `promptTokenCount`, so adding it again would bill them twice. `thoughts_token_count` is
    // separate from `candidatesTokenCount` and maps to the reasoning bucket, which prices at the
    // output rate unless a model publishes a distinct one.
    let usage = Usage {
        event_id: event_id(attributes),
        // `gemini` rather than `google`: it is the provider key LiteLLM uses, so
        // `gemini/gemini-2.5-pro` resolves against the bundled pricing table.
        provider: "gemini".to_string(),
        model,
        category: Category::Unknown,
        requests: 1,
        input,
        output: count("output_token_count"),
        reasoning: count("thoughts_token_count"),
        cache_read,
        // Gemini reports no cache-write count; implicit caching is not separately billed.
        cache_write: 0,
        cost: None,
        cost_status: CostStatus::Unavailable,
        billing: decision_billing,
        api_equivalent_cost: None,
        created: created_at(attributes),
        session_id: attributes
            .get("session.id")
            .and_then(Value::as_str)
            .map(str::to_string),
        // Gemini's telemetry records no working directory, so per-project attribution is not
        // available for this source. Left `None` rather than guessed from the process's cwd,
        // which is where the *dashboard* is running, not where the work happened.
        project: None,
    };

    if usage.input == 0 && usage.output == 0 && usage.reasoning == 0 && usage.cache_read == 0 {
        return None;
    }
    Some(usage)
}

/// A stable identity for one API response.
///
/// Deliberately not `prompt_id` alone: one user prompt drives a whole tool-use loop, so several
/// `api_response` records share a `prompt_id`. Keying on it would deduplicate real requests away
/// and under-report spend, which is the failure CONTRIBUTING invariant 3 exists to prevent.
fn event_id(attributes: &serde_json::Map<String, Value>) -> Option<String> {
    let prompt_id = attributes.get("prompt_id").and_then(Value::as_str)?;
    let timestamp = attributes.get("event.timestamp").and_then(Value::as_str)?;
    let total = attributes
        .get("total_token_count")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    Some(format!("gemini:{prompt_id}:{timestamp}:{total}"))
}

/// The record's timestamp, from the ISO-8601 attribute the event carries.
fn created_at(attributes: &serde_json::Map<String, Value>) -> i64 {
    attributes
        .get("event.timestamp")
        .and_then(Value::as_str)
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map(|dt| dt.timestamp())
        .unwrap_or_default()
}

/// Read every usage row in the telemetry log.
pub fn load_gemini(
    root: Option<&Path>,
    offsets: &mut Offsets,
    billing: Billing,
) -> Result<(Vec<Usage>, String)> {
    let Some(path) = telemetry_path(root) else {
        return Ok((Vec::new(), "Gemini CLI: no home directory".into()));
    };
    if !path.exists() {
        return Ok((
            Vec::new(),
            format!(
                "Gemini CLI: telemetry not enabled (no {}); see docs/provider-support.md",
                path.display()
            ),
        ));
    }

    let text = std::fs::read_to_string(&path)?;
    let start = offsets.files.get(&path).copied().unwrap_or(0) as usize;
    // A file that shrank was rotated or truncated; start over rather than reading from a stale
    // offset into the middle of a record.
    let start = if start > text.len() { 0 } else { start };
    let (objects, consumed) = complete_objects(&text[start..]);

    let mut usages = Vec::new();
    let mut unparsed = 0usize;
    for object in objects {
        match serde_json::from_str::<Value>(object) {
            Ok(value) => {
                if let Some(usage) = parse_record(&value, billing) {
                    usages.push(usage);
                }
            }
            Err(_) => unparsed += 1,
        }
    }
    offsets
        .files
        .insert(path.clone(), (start + consumed) as u64);

    let note = if unparsed > 0 {
        format!(" · {unparsed} unreadable record(s)")
    } else {
        String::new()
    };
    let status = format!(
        "Gemini CLI: {} ({} events{})",
        path.display(),
        usages.len(),
        note
    );
    Ok((usages, status))
}

/// One-shot read for the source registry.
pub(crate) fn read(
    roots: &crate::collector::SourceRoots,
) -> crate::collector::registry::SourceRead {
    let decision = roots.gemini_decision();
    let (usages, status) = load_gemini(
        roots.gemini_dir.as_deref(),
        &mut Offsets::new(),
        decision.billing,
    )
    .unwrap_or_else(|error| (Vec::new(), format!("Gemini CLI: unavailable ({})", error)));
    let path = telemetry_path(roots.gemini_dir.as_deref());
    Ok((
        crate::collector::SourceReport {
            id: ID,
            present: path.as_deref().is_some_and(Path::exists),
            path,
            rows: usages.len(),
            status,
            detail: Some(decision.describe("collectors.gemini")),
        },
        usages,
    ))
}

/// A background collector for the same source.
pub(crate) fn collector(
    roots: &crate::collector::SourceRoots,
    interval_secs: u64,
) -> Box<dyn Collector> {
    Box::new(GeminiCollector {
        root: roots.gemini_dir.clone(),
        interval_secs,
        offsets: Offsets::new(),
        billing: roots.gemini_billing,
        omarchy_dir: roots.omarchy_signal_dir(),
        decision: None,
    })
}

pub struct GeminiCollector {
    pub root: Option<PathBuf>,
    pub interval_secs: u64,
    /// Byte offset per file, so each poll tails only what was appended.
    pub offsets: Offsets,
    pub billing: BillingSetting,
    pub omarchy_dir: Option<PathBuf>,
    pub decision: Option<crate::collector::billing::Decision>,
}

impl Collector for GeminiCollector {
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
            .and_then(|dir| crate::omarchy::tier_label_for(dir, ID));
        let fresh = detect(
            ID,
            self.billing,
            &Signals {
                claude_json: None,
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: tier.as_deref(),
            },
        );
        let decision = resolve_sticky(ID, self.decision.take(), fresh);
        self.decision = Some(decision.clone());
        let (usages, _) = load_gemini(self.root.as_deref(), &mut self.offsets, decision.billing)?;
        Ok(usages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record shaped exactly as `ApiResponseEvent.toLogRecord` builds it, wrapped the way the
    /// OTLP SDK's file exporter writes it.
    fn record(prompt_id: &str, timestamp: &str, extra: &str) -> String {
        format!(
            r#"{{
  "hrTime": [ 1756000000, 0 ],
  "severityNumber": 9,
  "body": "API response from gemini-2.5-pro. Status: 200. Duration: 1200ms.",
  "attributes": {{
    "session.id": "sess-1",
    "event.name": "gemini_cli.api_response",
    "event.timestamp": "{timestamp}",
    "model": "gemini-2.5-pro",
    "duration_ms": 1200,
    "input_token_count": 1000,
    "output_token_count": 200,
    "cached_content_token_count": 400,
    "thoughts_token_count": 50,
    "tool_token_count": 30,
    "total_token_count": 1250,
    "prompt_id": "{prompt_id}",
    "auth_type": "oauth-personal",
    "status_code": 200,
    "finish_reasons": [ "STOP" ]{extra}
  }}
}}
"#
        )
    }

    #[test]
    fn an_api_response_becomes_one_usage_row() {
        let value: Value = serde_json::from_str(&record("p-1", "2026-08-24T10:00:00.000Z", ""))
            .expect("fixture parses");
        let usage = parse_record(&value, Billing::PerToken).expect("a usage row");

        assert_eq!(usage.provider, "gemini");
        assert_eq!(usage.model, "gemini-2.5-pro");
        assert_eq!(usage.requests, 1);
        // Cache is a subset of the prompt count upstream, so the buckets are made disjoint.
        assert_eq!(usage.cache_read, 400);
        assert_eq!(
            usage.input, 600,
            "cached tokens must not also count as input"
        );
        assert_eq!(usage.output, 200);
        assert_eq!(usage.reasoning, 50, "thoughts map to the reasoning bucket");
        assert_eq!(usage.cache_write, 0);
        assert_eq!(usage.session_id.as_deref(), Some("sess-1"));
        assert_eq!(usage.created, 1787565600, "2026-08-24T10:00:00Z");
    }

    /// Several API responses share one `prompt_id` during a tool-use loop. Keying identity on it
    /// alone would deduplicate real requests away and under-report spend.
    #[test]
    fn responses_sharing_a_prompt_id_keep_separate_identities() {
        let first: Value =
            serde_json::from_str(&record("p-1", "2026-08-24T10:00:00.000Z", "")).unwrap();
        let second: Value =
            serde_json::from_str(&record("p-1", "2026-08-24T10:00:03.000Z", "")).unwrap();
        let a = parse_record(&first, Billing::PerToken).unwrap();
        let b = parse_record(&second, Billing::PerToken).unwrap();

        assert!(a.event_id.is_some());
        assert_ne!(
            a.event_id, b.event_id,
            "two responses in one prompt must not collapse into one row"
        );
        assert_ne!(
            crate::collector::usage_key(&a),
            crate::collector::usage_key(&b)
        );
    }

    /// The privacy boundary: `response_text` is present whenever the user has `logPrompts` on.
    #[test]
    fn model_output_never_reaches_a_usage_record() {
        let planted = r#","response_text":"sk-not-a-real-key-abcdef0123456789""#;
        let value: Value =
            serde_json::from_str(&record("p-1", "2026-08-24T10:00:00.000Z", planted)).unwrap();
        let usage = parse_record(&value, Billing::PerToken).expect("a usage row");
        let rendered = format!("{usage:?}");
        assert!(
            !rendered.contains("sk-not-a-real-key"),
            "model output reached the usage record: {rendered}"
        );
    }

    #[test]
    fn records_that_are_not_api_responses_are_skipped() {
        for body in [
            r#"{"attributes":{"event.name":"gemini_cli.user_prompt","prompt_length":42}}"#,
            r#"{"attributes":{"event.name":"gemini_cli.tool_call","function_name":"read_file"}}"#,
            r#"{"body":"no attributes at all"}"#,
        ] {
            let value: Value = serde_json::from_str(body).unwrap();
            assert!(parse_record(&value, Billing::PerToken).is_none(), "{body}");
        }
    }

    /// A failed call still emits a record; counting it would show a free request.
    #[test]
    fn a_failed_call_is_not_counted_as_usage() {
        let value: Value = serde_json::from_str(
            r#"{"attributes":{"event.name":"gemini_cli.api_response","model":"gemini-2.5-pro",
                "status_code":429,"input_token_count":10,"output_token_count":0,
                "event.timestamp":"2026-08-24T10:00:00.000Z","prompt_id":"p"}}"#,
        )
        .unwrap();
        assert!(parse_record(&value, Billing::PerToken).is_none());
    }

    /// The file is concatenated *pretty-printed* JSON, so it cannot be split on newlines — and a
    /// poll can land mid-record while the CLI is still writing one.
    #[test]
    fn only_complete_objects_are_consumed() {
        let whole = record("p-1", "2026-08-24T10:00:00.000Z", "");
        let partial = &whole[..whole.len() / 2];
        let stream = format!("{whole}{partial}");

        let (objects, consumed) = complete_objects(&stream);
        assert_eq!(objects.len(), 1, "the half-written record must not be read");
        assert_eq!(
            consumed,
            whole.trim_end().len(),
            "the offset must stop at the end of the last complete record"
        );
        serde_json::from_str::<Value>(objects[0]).expect("the complete object is valid JSON");
    }

    /// A `{` inside a string must not open a nesting level.
    #[test]
    fn braces_inside_strings_do_not_confuse_the_splitter() {
        let stream = r#"{"body":"a { b } c","attributes":{"event.name":"x"}}
{"body":"escaped \" quote { still fine","attributes":{"event.name":"y"}}
"#;
        let (objects, _) = complete_objects(stream);
        assert_eq!(objects.len(), 2);
        for object in objects {
            serde_json::from_str::<Value>(object).expect("valid JSON");
        }
    }

    #[test]
    fn a_missing_telemetry_file_is_not_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let (usages, source) =
            load_gemini(Some(dir.path()), &mut Offsets::new(), Billing::PerToken).unwrap();
        assert!(usages.is_empty());
        assert!(source.contains("telemetry not enabled"), "{source}");
    }

    #[test]
    fn a_poll_reads_only_what_was_appended() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("telemetry.json");
        std::fs::write(&path, record("p-1", "2026-08-24T10:00:00.000Z", "")).unwrap();

        let mut offsets = Offsets::new();
        let (first, _) = load_gemini(Some(dir.path()), &mut offsets, Billing::PerToken).unwrap();
        assert_eq!(first.len(), 1);

        // Nothing new: a second poll must not re-report the same request.
        let (again, _) = load_gemini(Some(dir.path()), &mut offsets, Billing::PerToken).unwrap();
        assert!(again.is_empty(), "the same record was reported twice");

        let mut appended = std::fs::read_to_string(&path).unwrap();
        appended.push_str(&record("p-2", "2026-08-24T10:00:05.000Z", ""));
        std::fs::write(&path, appended).unwrap();
        let (third, _) = load_gemini(Some(dir.path()), &mut offsets, Billing::PerToken).unwrap();
        assert_eq!(third.len(), 1, "only the appended record");
        assert!(third[0].event_id.as_deref().unwrap().contains("p-2"));
    }
}
