//! Claude Code's hooks as a routing harness.
//!
//! `--claude-code-hook` reads the JSON Claude Code hands a `PostToolUse` or
//! `PostToolUseFailure` hook and journals a routing event when — and only when — that payload
//! observed a test run. Everything here was checked against what Claude Code 2.1.245 actually
//! sends, which differs from its reference in two ways that matter:
//!
//! - A Bash command that exits non-zero fires **`PostToolUseFailure`**, with the status in
//!   `error` (`"Exit code 1"`), not `PostToolUse` with a non-zero `exit_code`.
//! - `PostToolUse`'s `tool_response` for Bash carries **no exit code at all** — `stdout`,
//!   `stderr`, `interrupted` and two display flags. Success is the event having fired.
//!
//! So pass and fail are told apart by which hook fired, and the snippet in
//! `contrib/claude-code/` registers the same command for both.
//!
//! What the event says, and where each part comes from:
//!
//! - **`test_result`**: the hook event, gated by `shell::test_runner` — the command line must
//!   contain a recognised runner *and* its exit status must be the runner's own.
//! - **`model`, `requests`, `tokens`, `cost`**: the session transcript the payload names, read
//!   with the collector's own `parse_line`, which takes the `usage` block, the model and the
//!   timestamp from each assistant line and nothing else. The counts are the *attempt's* —
//!   every request in the transcript that no earlier event of this session has attributed —
//!   priced by the same billing decision and rate table the dashboard applies, so a Max
//!   account's attempt is `on quota` here exactly as its rows are there.
//!
//!   The attempt is bounded by a cursor, not a clock, because of a third thing the capture
//!   showed: Claude Code appends the assistant line that issued a tool call *after* the tool and
//!   its hooks have run. At hook time the transcript ends one request early, and that request's
//!   timestamp is earlier than the hook's, so a window by time would lose it from every
//!   attempt. The cursor is the number of requests this session's events have already
//!   attributed (`journal::attributed_requests`); everything past it is this attempt. Each
//!   request is counted once, one run late: the request that issued a test command lands in the
//!   attempt behind the *next* one. The model is the last request in the transcript — the model
//!   in use — which right after a switch is the model before it.
//! - **`event_id`**: `claude-code:{session_id}:{tool_use_id}`, so a re-delivered hook cannot
//!   record the same run twice.
//! - **Nothing else.** No counter is sent. `retries`, `escalations` and `review_defects` stay
//!   `null` — not reported — because a hook cannot count them.

use std::{
    collections::HashSet,
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

use crate::{
    classify::classify,
    collector::{
        billing::Decision,
        claude_code::{normalize_project_path, parse_line},
        journal::{attributed_requests, record_routing_event},
        SourceRoots,
    },
    harness::shell,
    helpers::string,
    model::{accrue, CostStatus},
    pricing::{apply_estimated_pricing, PricingEngine},
    utils::now,
};

/// The `agent` every event from this harness carries, suffixed with the subagent type when the
/// call came from one.
pub const AGENT: &str = "claude-code";

/// A test run the hook payload observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub session_id: String,
    pub tool_use_id: String,
    pub cwd: Option<String>,
    pub agent_type: Option<String>,
    pub agent_id: Option<String>,
    pub transcript_path: Option<PathBuf>,
    pub passed: bool,
}

/// What a payload turned out to be: a test run, or one of the many things a Bash hook fires for
/// that are not one. The reason is printed, so a hook that records nothing says why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Observed(Observation),
    Skipped(String),
}

/// Read one hook payload and decide whether it observed a test run.
///
/// Errors only for a payload that is not a Claude Code hook payload at all, or a Bash one with
/// its identity missing. Everything else that is not an observation is `Skipped` with a reason:
/// a hook that fires on every Bash call must not fail on the ordinary ones.
pub fn observe(payload: &Value) -> Result<Outcome> {
    let event = string(payload, &["hook_event_name"])
        .ok_or_else(|| anyhow!("not a Claude Code hook payload: no hook_event_name"))?;
    let passed = match event.as_str() {
        "PostToolUse" => true,
        "PostToolUseFailure" => false,
        other => return Ok(Outcome::Skipped(format!("{other} is not a tool outcome"))),
    };
    if string(payload, &["tool_name"]).as_deref() != Some("Bash") {
        return Ok(Outcome::Skipped("not a Bash call".to_string()));
    }
    let input = payload
        .get("tool_input")
        .ok_or_else(|| anyhow!("Bash payload without tool_input"))?;
    let command = string(input, &["command"])
        .ok_or_else(|| anyhow!("Bash payload without tool_input.command"))?;
    if input.get("run_in_background") == Some(&Value::Bool(true)) {
        return Ok(Outcome::Skipped(
            "the command ran in the background; its outcome is not in this payload".to_string(),
        ));
    }
    let interrupted = if passed {
        payload
            .get("tool_response")
            .and_then(|r| r.get("interrupted"))
            == Some(&Value::Bool(true))
    } else {
        payload.get("is_interrupt") == Some(&Value::Bool(true))
    };
    if interrupted {
        return Ok(Outcome::Skipped(
            "the command was interrupted before it finished".to_string(),
        ));
    }
    let Some(observable) = shell::test_runner(&command) else {
        return Ok(Outcome::Skipped("not a test run".to_string()));
    };
    let speaks = if passed {
        observable.on_success
    } else {
        observable.on_failure
    };
    if !speaks {
        return Ok(Outcome::Skipped(format!(
            "the command line's exit status does not speak for the test runner in it: {command:?}"
        )));
    }
    let session_id = string(payload, &["session_id"])
        .ok_or_else(|| anyhow!("Bash payload without session_id"))?;
    let tool_use_id = string(payload, &["tool_use_id"])
        .ok_or_else(|| anyhow!("Bash payload without tool_use_id"))?;
    Ok(Outcome::Observed(Observation {
        session_id,
        tool_use_id,
        cwd: string(payload, &["cwd"]),
        agent_type: string(payload, &["agent_type"]).filter(|t| !t.is_empty()),
        agent_id: string(payload, &["agent_id"]).filter(|id| !id.is_empty()),
        transcript_path: string(payload, &["transcript_path"]).map(PathBuf::from),
        passed,
    }))
}

/// The attempt behind an observation: who made it and what it consumed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attribution {
    /// The last request in the transcript: the model in use when the hook ran.
    pub model: Option<String>,
    pub requests: u64,
    pub tokens: u64,
    pub cost: Option<f64>,
    pub cost_status: CostStatus,
}

/// Read the transcript and attribute the attempt: every request past the first `skip`, priced
/// as the dashboard would price them.
///
/// One API request is written as several assistant lines sharing a `requestId` — one per
/// content block, each carrying the same usage — so requests are counted once, on that
/// identity, before the cursor is applied or anything is summed, as the collector deduplicates
/// them on merge. `skip` is what this session's earlier events already attributed; see the
/// module documentation for why a count and not a time.
pub fn attribute(
    transcript: &Path,
    skip: u64,
    decision: &Decision,
    engine: &PricingEngine,
) -> Result<Attribution> {
    let file = File::open(transcript)
        .with_context(|| format!("open transcript {}", transcript.display()))?;
    let mut seen = HashSet::new();
    let mut rows = Vec::new();
    let mut model = None;
    let mut position = 0u64;
    for line in BufReader::new(file).split(b'\n') {
        let line = line?;
        let Some(mut usage) = parse_line(&String::from_utf8_lossy(&line)) else {
            continue;
        };
        if let Some(id) = &usage.event_id {
            if !seen.insert(id.clone()) {
                continue;
            }
        }
        model = Some(usage.model.clone());
        position += 1;
        if position <= skip {
            continue;
        }
        usage.billing = decision.billing;
        rows.push(usage);
    }
    apply_estimated_pricing(&mut rows, engine);

    let mut cost = 0.0;
    let mut unpriced = 0;
    let mut quota = 0;
    let mut priced = 0;
    let mut requests = 0;
    let mut tokens = 0;
    for row in &rows {
        requests += row.requests;
        tokens += row.total_tokens();
        if row.cost_status.is_billable() && row.cost.is_some() {
            priced += row.requests;
        }
        accrue(row, &mut cost, &mut unpriced, &mut quota);
    }
    // One status per event, chosen so the aggregate can only understate: any request that should
    // carry a price and does not makes the whole attempt unpriced, and a figure is sent only when
    // every request contributed to it. Plan-billed work is `quota`, which the panel renders as
    // `on quota` rather than as a free run.
    let (cost, cost_status) = if rows.is_empty() || unpriced > 0 {
        (None, CostStatus::Unavailable)
    } else if priced == 0 && quota > 0 {
        (None, CostStatus::Quota)
    } else if priced > 0 && quota == 0 {
        (Some(cost), CostStatus::Estimated)
    } else {
        (None, CostStatus::Unavailable)
    };
    Ok(Attribution {
        model,
        requests,
        tokens,
        cost,
        cost_status,
    })
}

/// The routing event, in the shape `--record-routing` reads from stdin.
pub fn event(observation: &Observation, attribution: &Attribution, created: i64) -> Value {
    let model = attribution
        .model
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    let agent = match &observation.agent_type {
        Some(agent_type) => format!("{AGENT}:{agent_type}"),
        None => AGENT.to_string(),
    };
    let provider = "anthropic";
    json!({
        "event_id": format!("{AGENT}:{}:{}", observation.session_id, observation.tool_use_id),
        "agent": agent,
        "model": model,
        "provider": provider,
        "category": classify(provider, &model).label(),
        "task": observation.cwd.as_deref().map(normalize_project_path).unwrap_or_default(),
        "phase": "test",
        "requests": attribution.requests.max(1),
        "tokens": attribution.tokens,
        "cost": attribution.cost,
        "cost_status": attribution.cost_status.label(),
        "test_result": observation.passed,
        "created": created,
    })
}

/// What `--claude-code-hook` did, for the line it prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// A new event, or `false` when its identity was already in the journal.
    Event {
        inserted: bool,
        passed: bool,
    },
    Skipped(String),
}

/// `--claude-code-hook`: the payload on stdin, the event in the journal.
pub fn record_from_stdin(roots: &SourceRoots) -> Result<()> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    match record(&input, roots)? {
        Recorded::Event {
            inserted: true,
            passed,
        } => println!(
            "Recorded a {} test run in {}",
            if passed { "passing" } else { "failing" },
            roots.journal.display()
        ),
        Recorded::Event {
            inserted: false, ..
        } => println!("Already recorded; nothing to do"),
        Recorded::Skipped(why) => println!("Nothing to record: {why}"),
    }
    Ok(())
}

/// The transcript a subagent's own turns are written to, when the payload came from one.
///
/// Claude Code hands a subagent's hook the **parent's** `transcript_path` and the parent's
/// `session_id`; the subagent's own turns go to
/// `<project>/<session_id>/subagents/agent-<agent_id>.jsonl`, every line marked
/// `isSidechain: true`. Attributing through the payload's path therefore charged a subagent's
/// test run to the parent's requests and the parent's model: one measured run put 3 requests and
/// 65,598 tokens against a `make test` whose agent had spent 2 requests and about 318 tokens,
/// and would have priced it at the parent's model rather than the subagent's.
///
/// Falls back to the payload's path when the file is not there, which is every non-subagent call
/// and any build that lays this out differently.
fn subagent_transcript(observation: &Observation) -> Option<PathBuf> {
    let agent_id = observation.agent_id.as_ref()?;
    let parent = observation.transcript_path.as_ref()?;
    let path = parent
        .parent()?
        .join(&observation.session_id)
        .join("subagents")
        .join(format!("agent-{agent_id}.jsonl"));
    path.is_file().then_some(path)
}

/// The whole of the hook, on one payload. The fast path — every Bash call that is not a test
/// run — decides from the payload alone: no journal, no transcript, no rate table.
pub fn record(input: &str, roots: &SourceRoots) -> Result<Recorded> {
    let payload: Value =
        serde_json::from_str(input).context("the hook payload on stdin is not JSON")?;
    let observation = match observe(&payload)? {
        Outcome::Observed(observation) => observation,
        Outcome::Skipped(why) => return Ok(Recorded::Skipped(why)),
    };

    // A subagent's attempt is its own, and so is its cursor: keying both on the session alone
    // made one counter serve the parent and every subagent under it.
    let prefix = match &observation.agent_id {
        Some(agent_id) => format!("{AGENT}:{}:{agent_id}:", observation.session_id),
        None => format!("{AGENT}:{}:", observation.session_id),
    };
    let skip = attributed_requests(&roots.journal, &prefix)?;
    let decision = roots.claude_decision();
    let engine = PricingEngine::load();
    let transcript =
        subagent_transcript(&observation).or_else(|| observation.transcript_path.clone());
    let attribution = match &transcript {
        Some(path) => match attribute(path, skip, &decision, &engine) {
            Ok(attribution) => attribution,
            // The run was observed whether or not the attempt can be attributed; recording it
            // with nothing counted is the smaller loss, and the log says which it was.
            Err(error) => {
                crate::logging::warn(
                    "harness",
                    &format!(
                        "claude-code: test run {} recorded without attribution: {error}",
                        observation.tool_use_id
                    ),
                );
                Attribution::default()
            }
        },
        None => {
            crate::logging::warn(
                "harness",
                &format!(
                    "claude-code: test run {} recorded without attribution: the payload named no transcript",
                    observation.tool_use_id
                ),
            );
            Attribution::default()
        }
    };
    if attribution.requests == 0 {
        crate::logging::warn(
            "harness",
            &format!(
                "claude-code: test run {} attributed to no request; the transcript had none past the {skip} already attributed",
                observation.tool_use_id
            ),
        );
    }

    let event = event(&observation, &attribution, now());
    let inserted = record_routing_event(&roots.journal, &event)?;
    Ok(Recorded::Event {
        inserted: inserted > 0,
        passed: observation.passed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Billing;

    /// The `PostToolUse` payload Claude Code 2.1.245 sent for a Bash call, verbatim apart from
    /// the paths. Note what is absent: any exit code.
    fn success(command: &str) -> Value {
        json!({
            "session_id": "6319b6c9-cad9-4969-a499-0086c596a220",
            "transcript_path": "/tmp/t.jsonl",
            "cwd": "/home/me/proj/",
            "prompt_id": "97f95a1e-50ea-414b-8f76-6d6b1d2142a2",
            "permission_mode": "default",
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command, "description": "Run tests"},
            "tool_response": {"stdout": "", "stderr": "", "interrupted": false, "isImage": false, "noOutputExpected": false},
            "tool_use_id": "toolu_01Jig6ud9ip8ehgTZQ1qgJvE",
            "duration_ms": 117
        })
    }

    /// The `PostToolUseFailure` payload for `false`, verbatim apart from the paths.
    fn failure(command: &str) -> Value {
        json!({
            "session_id": "6319b6c9-cad9-4969-a499-0086c596a220",
            "transcript_path": "/tmp/t.jsonl",
            "cwd": "/home/me/proj",
            "prompt_id": "97f95a1e-50ea-414b-8f76-6d6b1d2142a2",
            "permission_mode": "default",
            "hook_event_name": "PostToolUseFailure",
            "tool_name": "Bash",
            "tool_input": {"command": command, "description": "Run tests"},
            "tool_use_id": "toolu_017jk7QupzJNHf9e1BimihU4",
            "error": "Exit code 1",
            "is_interrupt": false,
            "duration_ms": 18
        })
    }

    fn observed(outcome: Outcome) -> Observation {
        match outcome {
            Outcome::Observed(o) => o,
            Outcome::Skipped(why) => panic!("skipped: {why}"),
        }
    }

    #[test]
    fn the_event_that_fired_is_the_result() {
        let pass = observed(observe(&success("cargo test")).unwrap());
        assert!(pass.passed);
        assert_eq!(pass.session_id, "6319b6c9-cad9-4969-a499-0086c596a220");
        assert_eq!(pass.tool_use_id, "toolu_01Jig6ud9ip8ehgTZQ1qgJvE");
        assert_eq!(
            pass.transcript_path.as_deref(),
            Some(Path::new("/tmp/t.jsonl"))
        );

        let fail = observed(observe(&failure("cargo test")).unwrap());
        assert!(!fail.passed);
    }

    #[test]
    fn an_ordinary_bash_call_is_skipped_with_a_reason() {
        for command in ["ls -la", "git status", "grep -rn \"cargo test\" ."] {
            assert_eq!(
                observe(&success(command)).unwrap(),
                Outcome::Skipped("not a test run".to_string()),
                "{command}"
            );
        }
    }

    /// `cargo test | tail` succeeds when `tail` does; `cargo build && cargo test` fails when the
    /// build does. Neither status is the runner's, and neither is recorded.
    #[test]
    fn a_status_that_is_not_the_runners_is_skipped() {
        assert!(matches!(
            observe(&success("cargo test 2>&1 | tail -20")).unwrap(),
            Outcome::Skipped(why) if why.starts_with("not a test run")
        ));
        assert!(matches!(
            observe(&failure("cargo build && cargo test")).unwrap(),
            Outcome::Skipped(why) if why.contains("does not speak")
        ));
        // ...but the same chain's success is the runner's.
        assert!(observed(observe(&success("cargo build && cargo test")).unwrap()).passed);
    }

    #[test]
    fn other_tools_and_other_events_are_skipped() {
        let mut edit = success("cargo test");
        edit["tool_name"] = json!("Edit");
        assert_eq!(
            observe(&edit).unwrap(),
            Outcome::Skipped("not a Bash call".to_string())
        );
        let mut stop = success("cargo test");
        stop["hook_event_name"] = json!("Stop");
        assert!(matches!(observe(&stop).unwrap(), Outcome::Skipped(_)));
    }

    #[test]
    fn an_interrupted_or_backgrounded_run_has_no_outcome() {
        let mut interrupted = success("cargo test");
        interrupted["tool_response"]["interrupted"] = json!(true);
        assert!(
            matches!(observe(&interrupted).unwrap(), Outcome::Skipped(why) if why.contains("interrupted"))
        );

        let mut cancelled = failure("cargo test");
        cancelled["is_interrupt"] = json!(true);
        assert!(
            matches!(observe(&cancelled).unwrap(), Outcome::Skipped(why) if why.contains("interrupted"))
        );

        let mut background = success("cargo test");
        background["tool_input"]["run_in_background"] = json!(true);
        assert!(
            matches!(observe(&background).unwrap(), Outcome::Skipped(why) if why.contains("background"))
        );
    }

    #[test]
    fn a_payload_that_is_not_a_hooks_is_an_error_not_a_skip() {
        assert!(observe(&json!({"foo": 1})).is_err());
        let mut anonymous = success("cargo test");
        anonymous.as_object_mut().unwrap().remove("tool_use_id");
        assert!(observe(&anonymous).is_err());
    }

    #[test]
    fn the_event_carries_its_identity_and_nothing_it_did_not_measure() {
        let observation = observed(observe(&success("cargo test")).unwrap());
        let attribution = Attribution {
            model: Some("claude-opus-5".into()),
            requests: 3,
            tokens: 4200,
            cost: None,
            cost_status: CostStatus::Quota,
        };
        let event = event(&observation, &attribution, 1_700_000_000);
        assert_eq!(
            event["event_id"],
            "claude-code:6319b6c9-cad9-4969-a499-0086c596a220:toolu_01Jig6ud9ip8ehgTZQ1qgJvE"
        );
        assert_eq!(event["agent"], "claude-code");
        assert_eq!(event["model"], "claude-opus-5");
        assert_eq!(event["provider"], "anthropic");
        assert_eq!(event["category"], "PAID");
        // The trailing separator is dropped, as the Projects panel drops it.
        assert_eq!(event["task"], "/home/me/proj");
        assert_eq!(event["phase"], "test");
        assert_eq!(event["requests"], 3);
        assert_eq!(event["tokens"], 4200);
        assert_eq!(event["cost"], Value::Null);
        assert_eq!(event["cost_status"], "quota");
        assert_eq!(event["test_result"], true);
        assert_eq!(event["created"], 1_700_000_000);
        for counter in ["retries", "escalations", "review_defects"] {
            assert!(event.get(counter).is_none(), "{counter} must not be sent");
        }
    }

    #[test]
    fn a_subagents_run_is_attributed_to_it() {
        let mut payload = success("cargo test");
        payload["agent_id"] = json!("a-1");
        payload["agent_type"] = json!("code-reviewer");
        let observation = observed(observe(&payload).unwrap());
        let event = event(&observation, &Attribution::default(), 0);
        assert_eq!(event["agent"], "claude-code:code-reviewer");
        assert_eq!(event["model"], "unknown");
        assert_eq!(event["requests"], 1);
        assert_eq!(event["cost_status"], "unavailable");
    }

    /// The layout Claude Code actually writes, captured from a real subagent session: the
    /// payload names the **parent's** transcript and the parent's `session_id`, while the
    /// subagent's own turns are at `<dir>/<session_id>/subagents/agent-<agent_id>.jsonl`.
    fn subagent_layout() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let parent = dir.path().join("t.jsonl");
        std::fs::write(&parent, TRANSCRIPT).unwrap();
        let nested = dir.path().join("s").join("subagents");
        std::fs::create_dir_all(&nested).unwrap();
        // One request, an order of magnitude smaller than the parent's: a subagent charged the
        // parent's transcript is charged the parent's context.
        std::fs::write(
            nested.join("agent-a-1.jsonl"),
            concat!(
                r#"{"type":"assistant","timestamp":"2026-08-25T21:30:43.693Z","requestId":"sub_1","isSidechain":true,"sessionId":"s","cwd":"/p","message":{"id":"m1","role":"assistant","model":"claude-haiku-4-5-20251001","content":[{"type":"thinking","thinking":"x"}],"usage":{"input_tokens":10,"output_tokens":3}}}"#,
                "\n",
            ),
        )
        .unwrap();
        (dir, parent)
    }

    #[test]
    fn a_subagents_attempt_is_read_from_the_subagents_own_transcript() {
        // Measured against a real session before this was fixed: a subagent's `make test` was
        // recorded as 3 requests and 65,598 tokens -- the parent's -- when the agent that ran it
        // had spent 2 requests and about 318. The model was the parent's last model too, so an
        // Opus parent would have priced a Haiku subagent's attempt at Opus.
        let (_dir, parent) = subagent_layout();
        let mut payload = success("cargo test");
        payload["agent_id"] = json!("a-1");
        payload["agent_type"] = json!("general-purpose");
        payload["session_id"] = json!("s");
        payload["transcript_path"] = json!(parent.to_str().unwrap());
        let observation = observed(observe(&payload).unwrap());

        let resolved = subagent_transcript(&observation).expect("the subagent's own transcript");
        let attribution =
            attribute(&resolved, 0, &subscription(), &PricingEngine::bundled()).unwrap();
        assert_eq!(
            attribution.model.as_deref(),
            Some("claude-haiku-4-5-20251001"),
            "the subagent's model, not the parent's"
        );
        assert_eq!(attribution.requests, 1);
        assert_eq!(attribution.tokens, 13, "10 + 3, not the parent's 1430");
    }

    #[test]
    fn a_call_that_is_not_a_subagents_keeps_the_payloads_transcript() {
        // No `agent_id`, so nothing to look beside: the parent's own runs are unaffected.
        let (_dir, parent) = subagent_layout();
        let mut payload = success("cargo test");
        payload["session_id"] = json!("s");
        payload["transcript_path"] = json!(parent.to_str().unwrap());
        let observation = observed(observe(&payload).unwrap());
        assert_eq!(observation.agent_id, None);
        assert_eq!(subagent_transcript(&observation), None);
    }

    #[test]
    fn a_subagent_with_no_transcript_of_its_own_falls_back() {
        // A build that lays subagents out differently degrades to the payload's path rather
        // than to no attribution at all.
        let (_dir, parent) = transcript();
        let mut payload = success("cargo test");
        payload["agent_id"] = json!("nobody");
        payload["session_id"] = json!("s");
        payload["transcript_path"] = json!(parent.to_str().unwrap());
        let observation = observed(observe(&payload).unwrap());
        assert_eq!(subagent_transcript(&observation), None);
    }

    fn subscription() -> Decision {
        Decision {
            billing: Billing::Subscription,
            tier: Some("Max 20x".into()),
            reason: "config",
        }
    }

    fn api() -> Decision {
        Decision {
            billing: Billing::PerToken,
            tier: None,
            reason: "config",
        }
    }

    /// A transcript as Claude Code writes one: each request as two assistant lines sharing a
    /// `requestId` and a usage block, the second carrying the tool call. The content is a
    /// planted credential, which must never reach the event.
    const TRANSCRIPT: &str = concat!(
        r#"{"type":"user","timestamp":"2026-08-25T21:30:41.689Z","sessionId":"s","message":{"role":"user","content":"AKIA_PLANTED_SECRET_KEY"}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-25T21:30:43.693Z","requestId":"req_1","sessionId":"s","cwd":"/p","message":{"id":"m1","role":"assistant","model":"claude-sonnet-5","content":[{"type":"thinking","thinking":"AKIA_PLANTED_SECRET_KEY"}],"usage":{"input_tokens":100,"output_tokens":50}}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-25T21:30:44.028Z","requestId":"req_1","sessionId":"s","cwd":"/p","message":{"id":"m1","role":"assistant","model":"claude-sonnet-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}],"usage":{"input_tokens":100,"output_tokens":50}}}"#,
        "\n",
        r#"{"type":"user","timestamp":"2026-08-25T21:30:44.156Z","sessionId":"s","message":{"role":"user","content":[{"type":"tool_result","content":"AKIA_PLANTED_SECRET_KEY"}]}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-25T21:31:45.582Z","requestId":"req_2","sessionId":"s","cwd":"/p","message":{"id":"m2","role":"assistant","model":"claude-opus-5","content":[{"type":"thinking","thinking":"x"}],"usage":{"input_tokens":200,"output_tokens":80,"cache_read_input_tokens":1000}}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-25T21:31:46.039Z","requestId":"req_2","sessionId":"s","cwd":"/p","message":{"id":"m2","role":"assistant","model":"claude-opus-5","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}],"usage":{"input_tokens":200,"output_tokens":80,"cache_read_input_tokens":1000}}}"#,
        "\n",
    );

    fn transcript() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, TRANSCRIPT).unwrap();
        (dir, path)
    }

    #[test]
    fn the_attempt_is_every_request_past_the_cursor_counted_once_per_request_id() {
        let (_dir, path) = transcript();
        let engine = PricingEngine::bundled();

        // First run in the session: everything so far. Two requests, not four lines.
        let whole = attribute(&path, 0, &subscription(), &engine).unwrap();
        assert_eq!(whole.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(whole.requests, 2, "{whole:?}");
        assert_eq!(whole.tokens, 150 + 1280, "{whole:?}");
        assert_eq!(whole.cost_status, CostStatus::Quota);
        assert_eq!(whole.cost, None);

        // Second run, with two requests already attributed: nothing is left, and the request
        // counted twice by a cursor over *lines* would be `req_1`'s second line.
        let rest = attribute(&path, 2, &subscription(), &engine).unwrap();
        assert_eq!(rest.requests, 0, "{rest:?}");

        // One attributed: the other, whichever second it fell in.
        let second = attribute(&path, 1, &subscription(), &engine).unwrap();
        assert_eq!(second.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(second.requests, 1, "{second:?}");
        assert_eq!(second.tokens, 1280, "{second:?}");
    }

    #[test]
    fn a_per_token_account_gets_a_figure_from_the_rate_table() {
        let (_dir, path) = transcript();
        let engine = PricingEngine::bundled();
        let priced = attribute(&path, 0, &api(), &engine).unwrap();
        assert_eq!(priced.cost_status, CostStatus::Estimated, "{priced:?}");
        assert!(priced.cost.is_some_and(|c| c > 0.0), "{priced:?}");
    }

    #[test]
    fn an_attempt_with_nothing_in_it_is_unpriced_not_free() {
        let (_dir, path) = transcript();
        let engine = PricingEngine::bundled();
        let empty = attribute(&path, 5, &subscription(), &engine).unwrap();
        // The model is still the last one that spoke; the counts are honestly nothing.
        assert_eq!(empty.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(empty.requests, 0);
        assert_eq!(empty.cost_status, CostStatus::Unavailable);
        assert_eq!(empty.cost, None);
    }

    /// Invariant 2: the transcript carries source and secrets; the event carries none of it.
    #[test]
    fn no_message_content_reaches_the_event() {
        let (_dir, path) = transcript();
        let attribution = attribute(&path, 0, &subscription(), &PricingEngine::bundled()).unwrap();
        let mut payload = success("cargo test");
        payload["transcript_path"] = json!(path.to_str().unwrap());
        let observation = observed(observe(&payload).unwrap());
        let event = event(&observation, &attribution, 0);
        assert!(
            !event.to_string().contains("PLANTED"),
            "content leaked into the event: {event}"
        );
        assert!(!format!("{attribution:?}").contains("PLANTED"));
    }

    #[test]
    fn a_missing_transcript_is_an_error_the_caller_can_see() {
        let error = attribute(
            Path::new("/nonexistent/t.jsonl"),
            0,
            &subscription(),
            &PricingEngine::bundled(),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("/nonexistent/t.jsonl"),
            "{error}"
        );
    }
}
