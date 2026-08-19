//! Claude Code session-log collector.
//!
//! Claude Code writes one JSONL file per session under `~/.claude/projects/<munged-cwd>/`,
//! appending a line per event. Assistant events carry a `message.usage` block with the token
//! counts we need, plus `uuid`/`requestId` for identity and `cwd`/`gitBranch` for attribution.
//!
//! **Privacy.** These files contain full prompts, completions, file contents and anything a
//! tool printed — including secrets read from a `.env`. This collector reads only the `usage`
//! block and a handful of identifiers, and never retains message content. That constraint is
//! the project invariant recorded in `CONTRIBUTING.md` and `docs/architecture.md`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::classify::classify;
use crate::helpers::{number, string};
use crate::model::{CostStatus, Usage};
use crate::utils::home_dir;

/// Root of Claude Code's session logs, overridable for tests and non-standard installs.
pub fn projects_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CLAUDE_PROJECTS_DIR") {
        return Some(PathBuf::from(path));
    }
    if let Ok(config) = std::env::var("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(config).join("projects"));
    }
    Some(home_dir()?.join(".claude").join("projects"))
}

/// Byte offsets already consumed per session file.
///
/// Session logs are append-only and grow without bound, so each poll resumes where the last
/// one stopped rather than re-reading and re-parsing the whole transcript.
#[derive(Debug, Default, Clone)]
pub struct Offsets(HashMap<PathBuf, u64>);

impl Offsets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tracked_files(&self) -> usize {
        self.0.len()
    }
}

pub fn load_claude_code(
    root: Option<&Path>,
    offsets: &mut Offsets,
) -> Result<(Vec<Usage>, String)> {
    let Some(root) = root.map(Path::to_path_buf).or_else(projects_dir) else {
        return Ok((Vec::new(), "Claude Code: no home directory".into()));
    };
    if !root.exists() {
        return Ok((
            Vec::new(),
            format!("Claude Code: no session logs at {}", root.display()),
        ));
    }

    let mut usages = Vec::new();
    let mut files = 0usize;
    for path in session_files(&root) {
        files += 1;
        match read_session(&path, offsets) {
            Ok(mut found) => usages.append(&mut found),
            // One unreadable or truncated transcript must not sink the whole collector.
            Err(_) => continue,
        }
    }

    Ok((
        usages,
        format!("Claude Code: {} ({} sessions)", root.display(), files),
    ))
}

/// Every `*.jsonl` under `root`, one directory deep per project plus any nested layout.
fn session_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn read_session(path: &Path, offsets: &mut Offsets) -> Result<Vec<Usage>> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let resume = offsets.0.get(path).copied().unwrap_or(0);

    // A shrinking file means it was rotated or rewritten; start over rather than reading from
    // a stale offset into the middle of a line.
    let start = if resume > size { 0 } else { resume };
    file.seek(SeekFrom::Start(start))?;

    let mut reader = BufReader::new(file);
    let mut consumed = start;
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
        consumed += bytes as u64;
        if let Some(usage) = parse_line(&line) {
            usages.push(usage);
        }
    }

    offsets.0.insert(path.to_path_buf(), consumed);
    Ok(usages)
}

/// Extract usage from one transcript line, or `None` if it carries no billable usage.
pub fn parse_line(line: &str) -> Option<Usage> {
    let json: Value = serde_json::from_str(line.trim()).ok()?;
    let message = json.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let usage = message.get("usage")?;

    let input = number(usage, &["input_tokens", "inputTokens"]);
    let output = number(usage, &["output_tokens", "outputTokens"]);
    let cache_read = number(usage, &["cache_read_input_tokens", "cacheReadInputTokens"]);
    let cache_write = number(
        usage,
        &["cache_creation_input_tokens", "cacheCreationInputTokens"],
    );
    if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 {
        return None;
    }

    let model = string(message, &["model"]).unwrap_or_else(|| "unknown".into());
    // Synthetic assistant turns (e.g. `<synthetic>`) are bookkeeping, not billable requests.
    if model.starts_with('<') {
        return None;
    }

    // Prefer the API request id: retries of one logical request share it, and Claude Code
    // writes the same assistant message across multiple lines when content streams in parts.
    let event_id = string(&json, &["requestId", "request_id"])
        .or_else(|| string(message, &["id"]))
        .or_else(|| string(&json, &["uuid"]));

    let provider = "anthropic".to_string();
    let created = string(&json, &["timestamp"])
        .and_then(|ts| crate::collector::opencode::parse_created_at(&ts))
        .unwrap_or(0);

    Some(Usage {
        event_id,
        category: classify(&provider, &model),
        provider,
        model,
        requests: 1,
        input,
        output,
        reasoning: 0,
        cache_read,
        cache_write,
        cost: None,
        // Claude Code reports no dollar cost; pricing is estimated downstream, and the
        // provenance stays `estimated` rather than pretending the provider told us.
        cost_status: CostStatus::Unavailable,
        created,
        session_id: string(&json, &["sessionId", "session_id"]),
        // The full working directory, not its last segment: `~/a/build` and `~/b/build` are
        // different projects, and collapsing them here would silently merge their costs with
        // no way to tell from the aggregate. The UI shortens this for display.
        project: string(&json, &["cwd"]).map(|cwd| normalize_project_path(&cwd)),
    })
}

/// The working directory, with a trailing separator removed so `/a/b` and `/a/b/` are one
/// project rather than two.
fn normalize_project_path(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        cwd.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const ASSISTANT_LINE: &str = r#"{"type":"assistant","uuid":"u-1","requestId":"req_abc","sessionId":"sess-1","timestamp":"2026-08-18T10:00:00Z","cwd":"/home/dev/ai-usage-tui","gitBranch":"main","message":{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":1200,"output_tokens":340,"cache_read_input_tokens":8000,"cache_creation_input_tokens":500}}}"#;

    #[test]
    fn an_assistant_line_yields_usage_with_attribution() {
        let usage = parse_line(ASSISTANT_LINE).expect("should parse");
        assert_eq!(usage.provider, "anthropic");
        assert_eq!(usage.model, "claude-sonnet-4-5-20250929");
        assert_eq!(usage.input, 1200);
        assert_eq!(usage.output, 340);
        assert_eq!(usage.cache_read, 8000);
        assert_eq!(usage.cache_write, 500);
        assert_eq!(usage.event_id.as_deref(), Some("req_abc"));
        assert_eq!(usage.session_id.as_deref(), Some("sess-1"));
        assert_eq!(usage.project.as_deref(), Some("/home/dev/ai-usage-tui"));
        assert_eq!(usage.created, 1_787_047_200); // 2026-08-18T10:00:00Z
    }

    #[test]
    fn claude_code_usage_is_priced_by_the_bundled_table() {
        // The dated, dash-versioned id must reach the dotted table entry, or the whole
        // collector produces tokens with no cost attached.
        let mut usage = parse_line(ASSISTANT_LINE).unwrap();
        let engine = crate::pricing::PricingEngine::bundled();
        crate::pricing::apply_estimated_pricing(std::slice::from_mut(&mut usage), &engine);
        assert_eq!(usage.cost_status, crate::model::CostStatus::Estimated);
        assert!(usage.cost.is_some_and(|c| c > 0.0));
    }

    #[test]
    fn non_billable_lines_are_ignored() {
        // User turns, tool results, and zero-token synthetic turns are not requests.
        assert!(
            parse_line(r#"{"type":"user","message":{"role":"user","content":"hi"}}"#).is_none()
        );
        assert!(parse_line("not json").is_none());
        assert!(parse_line(
            r#"{"message":{"role":"assistant","model":"<synthetic>","usage":{"input_tokens":1}}}"#
        )
        .is_none());
        assert!(parse_line(
            r#"{"message":{"role":"assistant","model":"claude-sonnet-4.5","usage":{"input_tokens":0,"output_tokens":0}}}"#
        )
        .is_none());
    }

    #[test]
    fn no_message_content_is_retained() {
        let line = r#"{"type":"assistant","uuid":"u-2","timestamp":"2026-08-18T10:00:00Z","cwd":"/x/secret-repo","message":{"role":"assistant","model":"claude-sonnet-4.5","content":[{"type":"text","text":"AWS_SECRET_ACCESS_KEY=hunter2"}],"usage":{"input_tokens":5,"output_tokens":5}}}"#;
        let usage = parse_line(line).unwrap();
        let rendered = format!("{:?}", usage);
        assert!(
            !rendered.contains("hunter2") && !rendered.contains("AWS_SECRET"),
            "message content leaked into the usage record: {}",
            rendered
        );
    }

    #[test]
    fn appended_lines_are_read_incrementally() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("-home-dev-proj");
        std::fs::create_dir_all(&project).unwrap();
        let log = project.join("session.jsonl");
        std::fs::write(&log, format!("{}\n", ASSISTANT_LINE)).unwrap();

        let mut offsets = Offsets::new();
        let (first, _) = load_claude_code(Some(dir.path()), &mut offsets).unwrap();
        assert_eq!(first.len(), 1);

        // Nothing new: a second poll must do no work rather than re-parsing the transcript.
        let (second, _) = load_claude_code(Some(dir.path()), &mut offsets).unwrap();
        assert!(second.is_empty(), "re-read an unchanged session log");

        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(file, "{}", ASSISTANT_LINE).unwrap();
        let (third, _) = load_claude_code(Some(dir.path()), &mut offsets).unwrap();
        assert_eq!(third.len(), 1, "appended line was not picked up");
    }

    #[test]
    fn a_partially_written_line_is_not_consumed() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("session.jsonl");
        // Simulates catching Claude Code mid-write: no trailing newline yet.
        std::fs::write(&log, ASSISTANT_LINE).unwrap();

        let mut offsets = Offsets::new();
        let (first, _) = load_claude_code(Some(dir.path()), &mut offsets).unwrap();
        assert!(first.is_empty(), "consumed an incomplete line");

        std::fs::write(&log, format!("{}\n", ASSISTANT_LINE)).unwrap();
        let (second, _) = load_claude_code(Some(dir.path()), &mut offsets).unwrap();
        assert_eq!(second.len(), 1, "line was not re-read once complete");
    }

    #[test]
    fn a_truncated_file_restarts_from_the_beginning() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("session.jsonl");
        std::fs::write(&log, format!("{}\n{}\n", ASSISTANT_LINE, ASSISTANT_LINE)).unwrap();

        let mut offsets = Offsets::new();
        let (first, _) = load_claude_code(Some(dir.path()), &mut offsets).unwrap();
        assert_eq!(first.len(), 2);

        std::fs::write(&log, format!("{}\n", ASSISTANT_LINE)).unwrap();
        let (second, _) = load_claude_code(Some(dir.path()), &mut offsets).unwrap();
        assert_eq!(
            second.len(),
            1,
            "rotated file was not re-read from the start"
        );
    }

    #[test]
    fn project_paths_are_kept_whole_and_trailing_separators_normalized() {
        // Keeping only the last segment merged every `build` directory on the machine into
        // one project row, silently summing unrelated work.
        assert_eq!(
            normalize_project_path("/home/dev/ai-usage-tui"),
            "/home/dev/ai-usage-tui"
        );
        assert_eq!(
            normalize_project_path("/home/dev/ai-usage-tui/"),
            "/home/dev/ai-usage-tui"
        );
        assert_eq!(normalize_project_path("C:\\src\\my-app"), "C:\\src\\my-app");
    }

    #[test]
    fn a_missing_projects_directory_is_not_an_error() {
        let mut offsets = Offsets::new();
        let (usages, source) = load_claude_code(
            Some(Path::new("/nonexistent/claude/projects")),
            &mut offsets,
        )
        .unwrap();
        assert!(usages.is_empty());
        assert!(source.contains("no session logs"));
    }
}
