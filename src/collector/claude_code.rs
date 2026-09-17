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
use crate::collector::background::Collector;
use crate::collector::billing::Decision;
use crate::collector::billing::{detect, resolve_sticky, BillingSetting, Signals};
use crate::collector::skipped::Skipped;
use crate::helpers::{number, required, string};
use crate::model::{CostStatus, Usage};
use crate::utils::home_dir;
use std::time::Duration;

/// This source's canonical id: the `Collector::name()` it reports, the
/// `[collectors.<id>]` table that configures it, and its key in the source registry.
/// One constant so those can never drift apart.
pub const ID: &str = "claude_code";

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

/// Claude Code's own config document, `~/.claude.json` by default.
///
/// Its `oauthAccount` block is the billing signal (see `collector::billing`). Resolution order:
/// an explicit path; `$CLAUDE_CONFIG_DIR/.claude.json`, where Claude Code keeps it when that
/// variable is set; and otherwise a path derived from the session-log root — `~/.claude/projects`
/// sits two levels below `~/.claude.json`. Deriving from the root is what keeps tests hermetic:
/// a fixture root under `tests/fixtures/` resolves to a file that does not exist rather than to
/// the developer's real account.
pub fn config_json_path(explicit: Option<&Path>, claude_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    if let Ok(config) = std::env::var("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(config).join(".claude.json"));
    }
    let overridden = claude_dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var("CLAUDE_PROJECTS_DIR").ok().map(PathBuf::from));
    if let Some(root) = overridden {
        return Some(root.parent()?.parent()?.join(".claude.json"));
    }
    Some(home_dir()?.join(".claude.json"))
}

/// Byte offsets already consumed per session file.
///
/// Session logs are append-only and grow without bound, so each poll resumes where the last
/// one stopped rather than re-reading and re-parsing the whole transcript.
#[derive(Debug, Default, Clone)]
pub struct Offsets {
    files: HashMap<PathBuf, u64>,
    /// Transcripts and lines the tail had to go around; see `collector::skipped`.
    skipped: Skipped,
}

impl Offsets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tracked_files(&self) -> usize {
        self.files.len()
    }

    pub fn skipped(&self) -> &Skipped {
        &self.skipped
    }
}

/// Every usage row is stamped with `decision.billing`: nothing on a transcript line says how
/// the account pays, so the answer comes from the source-level decision, and the source string
/// says which answer was reached so a wrong one is visible on screen.
pub fn load_claude_code(
    root: Option<&Path>,
    offsets: &mut Offsets,
    decision: &Decision,
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
    offsets.skipped.begin_pass();
    for path in session_files(&root) {
        files += 1;
        match read_session(&path, offsets) {
            Ok(mut found) => {
                for usage in &mut found {
                    usage.billing = decision.billing;
                }
                usages.append(&mut found);
            }
            // One unreadable or truncated transcript must not sink the whole collector -- but it
            // is counted, because its usage is missing from every total on screen.
            Err(error) => offsets.skipped.unreadable(&path, error),
        }
    }

    let mut status = format!(
        "Claude Code: {} ({} sessions) · {}",
        root.display(),
        files,
        decision.describe("collectors.claude_code")
    );
    if let Some(detail) = offsets.skipped.detail() {
        status.push_str(&format!(" · {detail}"));
    }
    Ok((usages, status))
}

/// Every `*.jsonl` under `root`, one directory deep per project plus any nested layout.
pub(crate) fn session_files(root: &Path) -> Vec<PathBuf> {
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
    let resume = offsets.files.get(path).copied().unwrap_or(0);

    // A shrinking file means it was rotated or rewritten; start over rather than reading from
    // a stale offset into the middle of a line.
    let start = if resume > size { 0 } else { resume };
    file.seek(SeekFrom::Start(start))?;

    let mut reader = BufReader::new(file);
    let mut consumed = start;
    let mut usages = Vec::new();
    let mut malformed = 0u64;
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
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(json) => {
                if let Some(usage) = parse_value(&json) {
                    usages.push(usage);
                }
            }
            Err(_) => malformed += 1,
        }
    }

    // Counted only once the offset is saved: a read that fails part-way leaves the offset where
    // it was, and the retry would otherwise count the same lines again.
    offsets.files.insert(path.to_path_buf(), consumed);
    for _ in 0..malformed {
        offsets.skipped.malformed();
    }
    for usage in &usages {
        offsets.skipped.note(usage);
    }
    Ok(usages)
}

/// Extract usage from one transcript line, or `None` if it carries no billable usage.
pub fn parse_line(line: &str) -> Option<Usage> {
    parse_value(&serde_json::from_str(line.trim()).ok()?)
}

/// `parse_line` over a line already parsed, so the tail parses each line once and can tell a
/// line that is not JSON from one that simply carries no usage.
fn parse_value(json: &Value) -> Option<Usage> {
    let message = json.get("message")?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let usage = message.get("usage")?;

    // Every assistant `usage` block Claude Code writes carries both of these -- 40,805 of 40,805
    // on the machine this was checked against -- so their absence is a format change, not a
    // zero. Cache counts are optional: older builds and error turns omit them.
    let mut incomplete = false;
    let input = required(usage, &["input_tokens", "inputTokens"], &mut incomplete);
    let output = required(usage, &["output_tokens", "outputTokens"], &mut incomplete);
    let cache_read = number(usage, &["cache_read_input_tokens", "cacheReadInputTokens"]);
    let cache_write = number(
        usage,
        &["cache_creation_input_tokens", "cacheCreationInputTokens"],
    );
    // All zeros *with the fields present* is a turn that used nothing, and is dropped as before.
    // All zeros because the fields are gone is a request whose size is unknown: kept, so that a
    // rename upstream shows as flagged rows rather than as a machine that stopped using Claude.
    if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 && !incomplete {
        return None;
    }

    let model = string(message, &["model"]).unwrap_or_else(|| "unknown".into());
    // Synthetic assistant turns (e.g. `<synthetic>`) are bookkeeping, not billable requests.
    if model.starts_with('<') {
        return None;
    }

    // Prefer the API request id: retries of one logical request share it, and Claude Code
    // writes the same assistant message across multiple lines when content streams in parts.
    let event_id = string(json, &["requestId", "request_id"])
        .or_else(|| string(message, &["id"]))
        .or_else(|| string(json, &["uuid"]));

    let provider = "anthropic".to_string();
    let created = string(json, &["timestamp"])
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
        billing: Default::default(),
        api_equivalent_cost: None,
        created,
        session_id: string(json, &["sessionId", "session_id"]),
        // The full working directory, not its last segment: `~/a/build` and `~/b/build` are
        // different projects, and collapsing them here would silently merge their costs with
        // no way to tell from the aggregate. The UI shortens this for display.
        project: string(json, &["cwd"]).map(|cwd| normalize_project_path(&cwd)),
        incomplete,
    })
}

/// The working directory, with a trailing separator removed so `/a/b` and `/a/b/` are one
/// project rather than two.
pub(crate) fn normalize_project_path(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        cwd.to_string()
    } else {
        trimmed.to_string()
    }
}

pub struct ClaudeCodeCollector {
    pub root: Option<PathBuf>,
    pub interval_secs: u64,
    /// Per-file byte offsets, so each poll tails only what was appended.
    pub offsets: Offsets,
    pub billing: BillingSetting,
    /// Claude Code's `~/.claude.json`, when it is not at the default location.
    pub claude_json: Option<PathBuf>,
    /// Omarchy's records directory, for the plan label its panel already derived; `None`
    /// disables that signal.
    pub omarchy_dir: Option<PathBuf>,
    /// The billing decision in force. Evidence, once found, is kept: Claude Code rewrites its
    /// config document constantly, and a poll that catches it half-written must not flip the
    /// rows it collects to a different status from the rows already merged.
    pub decision: Option<Decision>,
}

impl ClaudeCodeCollector {
    fn resolve_billing(&mut self) -> Decision {
        let path = config_json_path(self.claude_json.as_deref(), self.root.as_deref());
        let tier = self
            .omarchy_dir
            .as_deref()
            .and_then(|dir| crate::omarchy::tier_label_for(dir, "claude_code"));
        let fresh = detect(
            "claude_code",
            self.billing,
            &Signals {
                claude_json: path.as_deref(),
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: tier.as_deref(),
            },
        );
        let decision = resolve_sticky("claude_code", self.decision.take(), fresh);
        self.decision = Some(decision.clone());
        decision
    }
}

impl Collector for ClaudeCodeCollector {
    fn name(&self) -> &str {
        ID
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_secs)
    }
    fn poll(&mut self) -> Result<Vec<Usage>> {
        let decision = self.resolve_billing();
        let (usages, _) = load_claude_code(self.root.as_deref(), &mut self.offsets, &decision)?;
        Ok(usages)
    }
    fn warning(&self) -> Option<String> {
        self.offsets.skipped.warning()
    }
}

/// One-shot read for the source registry.
///
/// An unreadable transcript tree degrades to zero rows and says so, rather than propagating:
/// one bad session directory must not take the whole dashboard down.
pub(crate) fn read(
    roots: &crate::collector::SourceRoots,
) -> crate::collector::registry::SourceRead {
    let decision = roots.claude_decision();
    let (usages, status) =
        load_claude_code(roots.claude_dir.as_deref(), &mut Offsets::new(), &decision)
            .unwrap_or_else(|error| (Vec::new(), format!("Claude Code: unavailable ({})", error)));
    let path = roots.claude_dir.clone().or_else(projects_dir);
    Ok((
        crate::collector::SourceReport {
            id: ID,
            present: path.as_deref().is_some_and(Path::exists),
            path,
            rows: usages.len(),
            status,
            detail: Some(decision.describe("collectors.claude_code")),
        },
        usages,
    ))
}

/// A background collector for the same source.
pub(crate) fn collector(
    roots: &crate::collector::SourceRoots,
    interval_secs: u64,
) -> Box<dyn Collector> {
    Box::new(ClaudeCodeCollector {
        root: roots.claude_dir.clone(),
        interval_secs,
        offsets: Offsets::new(),
        billing: roots.claude_billing,
        claude_json: roots.claude_json.clone(),
        omarchy_dir: roots.omarchy_signal_dir(),
        decision: None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_billing_decision_with_evidence_survives_a_half_written_config_document() {
        // Claude Code rewrites ~/.claude.json constantly. A poll that catches it mid-write must
        // not flip the rows it collects to per-token while the rows already merged stay quota.
        let dir = tempfile::TempDir::new().unwrap();
        let projects = dir.path().join(".claude").join("projects").join("p");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::write(
            projects.join("s.jsonl"),
            "{\"type\":\"assistant\",\"uuid\":\"u-1\",\"requestId\":\"req_1\",\"timestamp\":\"2026-08-18T10:00:00Z\",\"message\":{\"id\":\"msg_1\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-5-20250929\",\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}}\n",
        )
        .unwrap();
        let config_json = dir.path().join(".claude.json");
        std::fs::write(
            &config_json,
            "{\"oauthAccount\":{\"organizationRateLimitTier\":\"default_claude_max_20x\"}}",
        )
        .unwrap();

        let mut collector = ClaudeCodeCollector {
            root: Some(dir.path().join(".claude").join("projects")),
            interval_secs: 30,
            offsets: Default::default(),
            billing: BillingSetting::Auto,
            claude_json: Some(config_json.clone()),
            omarchy_dir: None,
            decision: None,
        };
        let rows = collector.poll().unwrap();
        assert_eq!(rows[0].billing, crate::model::Billing::Subscription);
        assert_eq!(
            collector.decision.as_ref().and_then(|d| d.tier.as_deref()),
            Some("Max 20x")
        );

        std::fs::write(&config_json, "{\"oauthAccount\":{\"organizationRateLimi").unwrap();
        collector.poll().unwrap();
        assert_eq!(
            collector.decision.as_ref().map(|d| d.billing),
            Some(crate::model::Billing::Subscription),
            "evidence already found is kept over a momentary unreadable file"
        );
    }

    use super::*;
    use std::io::Write;

    use crate::model::Billing;

    /// The decision an API-key user gets, so tests exercise the pre-existing pricing path.
    fn per_token() -> Decision {
        Decision {
            billing: Billing::PerToken,
            tier: None,
            reason: "config",
        }
    }

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

    /// An unreadable transcript and a line that is not JSON both used to vanish: `Err(_) =>
    /// continue` for the file, `.ok()?` for the line, and no count of either. The dashboard
    /// showed smaller totals and a clean status line.
    #[test]
    fn skipped_transcripts_and_lines_reach_the_status_and_the_collector_warning() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("-home-dev-proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("good.jsonl"),
            format!("{ASSISTANT_LINE}\n{{\"type\":\"assistant\",\"message\":\n\n"),
        )
        .unwrap();
        // Not UTF-8, so reading it fails on every platform -- no permissions trick needed.
        let broken = project.join("broken.jsonl");
        std::fs::write(&broken, b"{\"type\":\"assistant\"}\xff\xfe\n").unwrap();

        let mut collector = ClaudeCodeCollector {
            root: Some(dir.path().to_path_buf()),
            interval_secs: 30,
            offsets: Default::default(),
            billing: BillingSetting::Auto,
            claude_json: Some(dir.path().join("no-claude.json")),
            omarchy_dir: None,
            decision: None,
        };
        assert_eq!(
            collector.poll().unwrap().len(),
            1,
            "the readable row survives"
        );
        assert_eq!(
            collector.warning().as_deref(),
            Some("1 file(s) unreadable, 1 malformed record(s) skipped")
        );

        // A second poll retries the broken file and must not count the malformed line again: its
        // offset moved past it.
        collector.poll().unwrap();
        assert_eq!(
            collector.warning().as_deref(),
            Some("1 file(s) unreadable, 1 malformed record(s) skipped")
        );

        // Fixed on disk: the file drops out, the lost line does not.
        std::fs::write(&broken, format!("{ASSISTANT_LINE}\n")).unwrap();
        collector.poll().unwrap();
        assert_eq!(
            collector.warning().as_deref(),
            Some("1 malformed record(s) skipped")
        );

        // The one-shot status names the file, for `--once`, `--json` and `--doctor`.
        std::fs::write(&broken, b"\xff\n").unwrap();
        let (_, status) = load_claude_code(Some(dir.path()), &mut Offsets::new(), &per_token())
            .expect("a bad transcript does not fail the source");
        assert!(
            status.contains("1 file(s) unreadable") && status.contains("broken.jsonl"),
            "{status}"
        );
    }

    /// An absent count used to read as `0`, so `output_tokens` renamed upstream priced every
    /// request as though it had produced nothing; and a turn with no timestamp became 1970,
    /// present in `--all` and silently missing from every other range. Both are kept -- what
    /// they do say is a fact -- flagged, and counted once.
    #[test]
    fn a_missing_count_or_timestamp_is_flagged_not_read_as_zero() {
        let renamed = r#"{"type":"assistant","uuid":"u-r","requestId":"req_r","timestamp":"2026-08-18T10:00:00Z","message":{"id":"m","role":"assistant","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":1200,"completion_tokens":340}}}"#;
        let usage = parse_line(renamed).expect("kept: its input is a fact");
        assert!(usage.incomplete, "output_tokens is absent, not zero");
        assert_eq!((usage.input, usage.output), (1200, 0));

        // Every field gone: a request of unknown size, not a machine that stopped using Claude.
        let all_renamed = r#"{"type":"assistant","uuid":"u-a","requestId":"req_a","timestamp":"2026-08-18T10:00:00Z","message":{"id":"m","role":"assistant","model":"claude-sonnet-4-5-20250929","usage":{"prompt":1200,"completion":340}}}"#;
        assert!(parse_line(all_renamed).is_some_and(|u| u.incomplete && u.requests == 1));
        // Zeros with the fields present are a turn that used nothing, dropped as before.
        let nothing = r#"{"type":"assistant","uuid":"u-z","timestamp":"2026-08-18T10:00:00Z","message":{"id":"m","role":"assistant","model":"claude-sonnet-4-5-20250929","usage":{"input_tokens":0,"output_tokens":0}}}"#;
        assert!(parse_line(nothing).is_none());
        assert!(!parse_line(ASSISTANT_LINE).unwrap().incomplete);

        let undated = ASSISTANT_LINE.replace(r#""timestamp":"2026-08-18T10:00:00Z","#, "");
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("-home-dev-proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("s.jsonl"),
            format!("{renamed}\n{undated}\n{ASSISTANT_LINE}\n"),
        )
        .unwrap();
        let mut offsets = Offsets::new();
        let (rows, status) =
            load_claude_code(Some(dir.path()), &mut offsets, &per_token()).unwrap();
        assert_eq!(rows.len(), 3, "nothing is dropped");
        let expected = "1 record(s) missing a token count, left unpriced, \
                        1 record(s) with no timestamp, in no range but --all";
        assert!(status.ends_with(expected), "{status}");
        load_claude_code(Some(dir.path()), &mut offsets, &per_token()).unwrap();
        assert_eq!(
            offsets.skipped().warning().as_deref(),
            Some(expected),
            "counted once"
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
        let (first, _) = load_claude_code(Some(dir.path()), &mut offsets, &per_token()).unwrap();
        assert_eq!(first.len(), 1);

        // Nothing new: a second poll must do no work rather than re-parsing the transcript.
        let (second, _) = load_claude_code(Some(dir.path()), &mut offsets, &per_token()).unwrap();
        assert!(second.is_empty(), "re-read an unchanged session log");

        let mut file = std::fs::OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(file, "{}", ASSISTANT_LINE).unwrap();
        let (third, _) = load_claude_code(Some(dir.path()), &mut offsets, &per_token()).unwrap();
        assert_eq!(third.len(), 1, "appended line was not picked up");
    }

    #[test]
    fn a_partially_written_line_is_not_consumed() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("session.jsonl");
        // Simulates catching Claude Code mid-write: no trailing newline yet.
        std::fs::write(&log, ASSISTANT_LINE).unwrap();

        let mut offsets = Offsets::new();
        let (first, _) = load_claude_code(Some(dir.path()), &mut offsets, &per_token()).unwrap();
        assert!(first.is_empty(), "consumed an incomplete line");

        std::fs::write(&log, format!("{}\n", ASSISTANT_LINE)).unwrap();
        let (second, _) = load_claude_code(Some(dir.path()), &mut offsets, &per_token()).unwrap();
        assert_eq!(second.len(), 1, "line was not re-read once complete");
    }

    #[test]
    fn a_truncated_file_restarts_from_the_beginning() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("session.jsonl");
        std::fs::write(&log, format!("{}\n{}\n", ASSISTANT_LINE, ASSISTANT_LINE)).unwrap();

        let mut offsets = Offsets::new();
        let (first, _) = load_claude_code(Some(dir.path()), &mut offsets, &per_token()).unwrap();
        assert_eq!(first.len(), 2);

        std::fs::write(&log, format!("{}\n", ASSISTANT_LINE)).unwrap();
        let (second, _) = load_claude_code(Some(dir.path()), &mut offsets, &per_token()).unwrap();
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
            &per_token(),
        )
        .unwrap();
        assert!(usages.is_empty());
        assert!(source.contains("no session logs"));
    }

    #[test]
    fn rows_carry_the_billing_decision_and_the_source_line_names_it() {
        let dir = tempfile::TempDir::new().unwrap();
        let project = dir.path().join("-home-dev-proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("s.jsonl"), format!("{}\n", ASSISTANT_LINE)).unwrap();

        let decision = Decision {
            billing: Billing::Subscription,
            tier: Some("Max 5x".into()),
            reason: "claude.json oauthAccount",
        };
        let (usages, source) =
            load_claude_code(Some(dir.path()), &mut Offsets::new(), &decision).unwrap();
        assert_eq!(usages[0].billing, Billing::Subscription);
        assert!(source.contains("subscription Max 5x"), "{source}");

        let (usages, source) =
            load_claude_code(Some(dir.path()), &mut Offsets::new(), &per_token()).unwrap();
        assert_eq!(usages[0].billing, Billing::PerToken);
        assert!(source.contains("api billing"), "{source}");
    }

    #[test]
    fn the_config_document_is_derived_from_an_overridden_root() {
        // A fixture root must never resolve to the developer's own ~/.claude.json.
        let root = Path::new("/fixtures/home/.claude/projects");
        assert_eq!(
            config_json_path(None, Some(root)).unwrap(),
            Path::new("/fixtures/home/.claude.json")
        );
        assert_eq!(
            config_json_path(Some(Path::new("/explicit.json")), Some(root)).unwrap(),
            Path::new("/explicit.json")
        );
    }
}
