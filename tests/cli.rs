//! End-to-end checks that run the built binary.
//!
//! These exist for behaviour that only appears in a real process: exit codes, and what happens
//! to stdout when the thing reading it goes away.

use std::io::Read;
use std::process::{Command, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ai-usage-tui"))
}

fn fixture_db() -> String {
    format!(
        "{}/tests/fixtures/opencode_test.db",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Never read the developer's real `~/.claude/projects`; see `docs/roadmap.md`.
fn hermetic(command: &mut Command) -> &mut Command {
    command
        .arg("--db")
        .arg(fixture_db())
        .arg("--claude-dir")
        .arg(format!(
            "{}/tests/fixtures/no-such-claude-dir",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg("--codex-dir")
        .arg(format!(
            "{}/tests/fixtures/no-such-codex-home",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg("--omarchy-dir")
        .arg(format!(
            "{}/tests/fixtures/no-such-omarchy-dir",
            env!("CARGO_MANIFEST_DIR")
        ))
        // The journal too. Without it these tests read whatever journal the developer's own
        // machine has -- `AI_USAGE_JOURNAL_PATH`, else `$XDG_DATA_HOME/ai-usage-tui/usage.db` --
        // so a machine with any journaled Ollama usage sees `ollama` rows in a run that is
        // supposed to be fixture-only. CI never caught it because a fresh runner has no journal.
        .arg("--journal")
        .arg(format!(
            "{}/tests/fixtures/no-such-journal.db",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg("--all")
}

/// `--doctor` is the answer to "the dashboard is empty and I do not know why".
///
/// It must name every source, say where each was looked for, and never fail just because
/// nothing is installed -- an empty machine is the normal first-run state, not an error.
#[test]
fn doctor_reports_every_source_and_where_it_looked() {
    let missing_claude = format!(
        "{}/tests/fixtures/no-such-claude-dir",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = hermetic(bin().arg("--doctor"))
        .output()
        .expect("run --doctor");

    assert!(
        output.status.success(),
        "--doctor exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("utf8");

    // Every source the collector actually reads, so a source added to the registry and left out
    // of the diagnosis shows up here.
    for id in ["opencode", "claude_code", "codex", "journal", "zen_pricing"] {
        assert!(text.contains(id), "--doctor never mentions {id}:\n{text}");
    }

    // The path searched, not just a verdict: "absent" without a path is not actionable.
    assert!(
        text.contains(&missing_claude),
        "--doctor does not say where Claude Code was looked for:\n{text}"
    );
    assert!(
        text.contains("--claude-dir"),
        "--doctor does not say how to point Claude Code elsewhere:\n{text}"
    );

    // The fixture database is real and has rows, so this run is not the all-absent case.
    assert!(
        text.contains("found"),
        "no source reported as found:\n{text}"
    );
    assert!(text.contains("CONFIG"), "no config section:\n{text}");
}

/// The journal's only write path, end to end. `--record-ollama` had no test at all, and the
/// three fixtures written for it (`ollama_single.json`, `ollama_stream.jsonl`,
/// `opencode_sample.json`) were referenced from nowhere in the tree.
#[test]
fn recording_an_ollama_response_round_trips_through_the_journal_and_is_idempotent() {
    let dir = scratch("ollama-journal");
    let journal = dir.join("usage.db");
    let fixture = format!(
        "{}/tests/fixtures/ollama_single.json",
        env!("CARGO_MANIFEST_DIR")
    );

    record(&journal, &fixture, "--record-ollama");
    let rows = journal_rows(&journal);
    assert_eq!(
        rows.len(),
        1,
        "one response should journal one row: {rows:?}"
    );
    assert_eq!(rows[0]["provider"], "ollama");
    assert_eq!(rows[0]["model"], "qwen3-coder-agent");
    assert_eq!(rows[0]["input_tokens"], 5000);
    assert_eq!(rows[0]["output_tokens"], 6500);
    // Local usage is never billed, and never rendered as a paid zero.
    assert_eq!(rows[0]["category"], "LOCAL");
    assert_eq!(rows[0]["cost_status"], "local");

    // Recording the same response twice must not double-count spend: the insert is
    // `INSERT OR IGNORE` against a unique index on the derived event id.
    record(&journal, &fixture, "--record-ollama");
    let rows = journal_rows(&journal);
    assert_eq!(
        rows.len(),
        1,
        "recording the same response twice double-counted it: {rows:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A streamed response arrives as many JSON lines; only the final one carries the totals.
#[test]
fn a_streamed_ollama_response_journals_once_from_its_final_line() {
    let dir = scratch("ollama-stream");
    let journal = dir.join("usage.db");
    record(
        &journal,
        &format!(
            "{}/tests/fixtures/ollama_stream.jsonl",
            env!("CARGO_MANIFEST_DIR")
        ),
        "--record-ollama",
    );

    let rows = journal_rows(&journal);
    assert_eq!(
        rows.len(),
        1,
        "a stream should journal one row, not one per chunk: {rows:?}"
    );
    // The last line's counts, not the first chunk's partial ones.
    assert_eq!(rows[0]["output_tokens"], 6500, "{rows:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `--record-routing` is the other write path, read back by `--routing-json`.
#[test]
fn recording_a_routing_event_round_trips_through_the_journal() {
    let dir = scratch("routing-journal");
    let journal = dir.join("usage.db");
    let event = dir.join("event.json");
    std::fs::write(
        &event,
        r#"{"task":"t-1","agent":"reviewer","model":"gpt-5.6-sol","provider":"openai",
            "category":"CLOUD","requests":1,"tokens":1234,"cost":0.5,"cost_status":"reported",
            "retries":2,"escalations":1,"test_result":"pass","review_defects":3}"#,
    )
    .expect("write event");
    record(&journal, event.to_str().unwrap(), "--record-routing");

    let output = bin()
        .arg("--routing-json")
        .arg("--journal")
        .arg(&journal)
        .output()
        .expect("run --routing-json");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("routing json parses");

    assert_eq!(json["events"], 1, "{json}");
    let agg = &json["aggregates"][0];
    assert_eq!(agg["agent"], "reviewer", "{json}");
    assert_eq!(agg["tokens"], 1234, "{json}");
    assert_eq!(agg["retries"], 2, "{json}");
    assert_eq!(agg["escalations"], 1, "{json}");
    assert_eq!(agg["review_defects"], 3, "{json}");

    let _ = std::fs::remove_dir_all(&dir);
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ai-usage-tui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Feed a file to a stdin-reading action.
fn record(journal: &std::path::Path, input: &str, flag: &str) {
    use std::io::Write;
    let mut child = bin()
        .arg(flag)
        .arg("--journal")
        .arg(journal)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let body = std::fs::read(input).expect("read input");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&body)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "{flag} exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The journal's rows, as `--json` reports them with every other source switched off.
fn journal_rows(journal: &std::path::Path) -> Vec<serde_json::Value> {
    let output = bin()
        .arg("--json")
        .arg("--all")
        .arg("--journal")
        .arg(journal)
        .arg("--db")
        .arg("/nonexistent/opencode.db")
        .arg("--claude-dir")
        .arg("/nonexistent")
        .arg("--codex-dir")
        .arg("/nonexistent")
        .arg("--omarchy-dir")
        .arg("/nonexistent")
        .output()
        .expect("run --json");
    assert!(
        output.status.success(),
        "--json exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json parses");
    json["usage"].as_array().cloned().unwrap_or_default()
}

/// `[collectors.<id>] enabled = false` governs the exports, not only the dashboard.
///
/// The two paths were wired separately: `main::build_collectors` honoured `enabled` and
/// `collector::load_usage` never saw it, so a source switched off in config still emitted rows
/// from `--json`, `--csv` and `--check-budgets`. The shipped example config even documented the
/// split ("Background collectors (TUI mode only)"). Both paths read one registry now.
#[test]
fn a_disabled_source_is_disabled_for_the_exports_too() {
    let dir = std::env::temp_dir().join(format!("ai-usage-tui-disabled-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let config = dir.join("config.toml");
    std::fs::write(&config, "[collectors.opencode]\nenabled = false\n").expect("write config");

    let with_source = hermetic(bin().arg("--json")).output().expect("run");
    let text = String::from_utf8(with_source.stdout).expect("utf8");
    assert!(
        text.contains("\"provider\""),
        "the fixture database should produce rows:\n{text}"
    );

    let without = hermetic(bin().arg("--json"))
        .arg("--config")
        .arg(&config)
        .output()
        .expect("run");
    let text = String::from_utf8(without.stdout).expect("utf8");
    assert!(
        without.status.success(),
        "exited {}: {}",
        without.status,
        String::from_utf8_lossy(&without.stderr)
    );
    assert!(
        text.contains("opencode: disabled"),
        "the source line should say the source was switched off:\n{text}"
    );
    assert!(
        !text.contains("\"provider\""),
        "rows from a disabled source still reached --json:\n{text}"
    );

    let _ = std::fs::remove_file(&config);
    let _ = std::fs::remove_dir(&dir);
}

/// A one-shot action, like every other one-shot action.
#[test]
fn doctor_does_not_combine_with_the_other_actions() {
    for other in ["--json", "--once", "--check-budgets", "--omarchy-record"] {
        let output = bin().arg("--doctor").arg(other).output().expect("run");
        assert!(
            !output.status.success(),
            "--doctor {other} was accepted; the actions are mutually exclusive"
        );
    }
}

#[test]
fn a_reader_that_closes_the_pipe_is_not_a_crash() {
    // `println!` panics when the write fails, and a closed pipe is a write failure, so
    // `ai-usage-tui --json | head` aborted with "failed printing to stdout: Broken pipe".
    // Closing the read end before the child gets to its first write reproduces that.
    let mut child = hermetic(bin().arg("--json"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    drop(child.stdout.take());

    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }
    let status = child.wait().expect("wait");

    assert!(
        !stderr.contains("panicked"),
        "the process panicked writing to a closed pipe:\n{stderr}"
    );
    assert!(
        status.success(),
        "a closed pipe should be a clean exit, got {status}"
    );
}

#[test]
fn the_text_output_path_also_survives_a_closed_pipe() {
    // The text path prints once per usage row, so it keeps writing after the reader is gone.
    let mut child = hermetic(bin().arg("--once"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");

    drop(child.stdout.take());

    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }
    let status = child.wait().expect("wait");

    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(status.success(), "got {status}");
}

#[test]
fn version_and_help_exit_zero() {
    for flag in ["--version", "--help"] {
        let output = bin().arg(flag).output().expect("run");
        assert!(output.status.success(), "{flag} exited {}", output.status);
        assert!(!output.stdout.is_empty(), "{flag} printed nothing");
    }
}

#[test]
fn an_unknown_flag_is_an_error_not_a_panic() {
    let output = bin().arg("--not-a-real-flag").output().expect("run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown option"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

/// A throwaway Claude Code home: `<home>/.claude/projects/<p>/<s>.jsonl` plus, by construction,
/// `<home>/.claude.json` as the derived config document — so nothing here can reach the
/// developer's own account.
fn claude_home(root: &std::path::Path) -> std::path::PathBuf {
    let projects = root.join(".claude").join("projects");
    let project = projects.join("p");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("s.jsonl"),
        "{\"type\":\"assistant\",\"uuid\":\"u-1\",\"requestId\":\"req_1\",\"timestamp\":\"2026-08-18T10:00:00Z\",\"sessionId\":\"s1\",\"message\":{\"id\":\"msg_1\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-5-20250929\",\"usage\":{\"input_tokens\":1000,\"output_tokens\":500}}}\n",
    )
    .unwrap();
    projects
}

fn anthropic_rows(stdout: &[u8]) -> Vec<serde_json::Value> {
    let json: serde_json::Value = serde_json::from_slice(stdout).expect("valid JSON");
    json["usage"]
        .as_array()
        .expect("usage array")
        .iter()
        .filter(|row| row["provider"] == "anthropic")
        .cloned()
        .collect()
}

#[test]
fn claude_billing_decides_whether_transcript_rows_carry_dollars() {
    let temp = std::env::temp_dir().join(format!("ai-usage-billing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    let projects = claude_home(&temp);
    let journal = temp.join("journal.db");

    let run = |extra: &[&str]| {
        let mut command = bin();
        command
            .arg("--json")
            .arg("--all")
            .arg("--db")
            .arg(fixture_db())
            .arg("--journal")
            .arg(&journal)
            .arg("--claude-dir")
            .arg(&projects)
            .arg("--codex-dir")
            .arg(temp.join("no-codex-home"))
            .arg("--omarchy-dir")
            .arg(temp.join("no-omarchy"))
            .args(extra);
        // The detector consults these; a developer's shell must not decide the test.
        for name in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_PROJECTS_DIR",
        ] {
            command.env_remove(name);
        }
        let output = command.output().expect("run");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        anthropic_rows(&output.stdout)
    };

    // No signal at all (the derived <home>/.claude.json does not exist): per-token, priced.
    let rows = run(&[]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["cost_status"], "estimated", "{}", rows[0]);
    assert!(rows[0]["cost"].is_number());
    assert!(rows[0]["api_equivalent_cost"].is_null());

    // Forced onto a plan: quota, no dollars, counterfactual appended.
    let rows = run(&["--claude-billing", "subscription"]);
    assert_eq!(rows[0]["cost_status"], "quota", "{}", rows[0]);
    assert!(rows[0]["cost"].is_null());
    assert!(rows[0]["api_equivalent_cost"].is_number());

    // Auto-detected from a planted config document at the derived location.
    std::fs::write(
        temp.join(".claude.json"),
        "{\"oauthAccount\":{\"organizationRateLimitTier\":\"default_claude_max_5x\",\"emailAddress\":\"planted@example.invalid\"}}",
    )
    .unwrap();
    let rows = run(&[]);
    assert_eq!(rows[0]["cost_status"], "quota", "{}", rows[0]);

    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn codex_rollouts_are_exported_with_split_buckets_and_no_content() {
    let codex_home = format!("{}/tests/fixtures/codex_home", env!("CARGO_MANIFEST_DIR"));
    let mut command = bin();
    command
        .arg("--json")
        .arg("--all")
        .arg("--db")
        .arg(fixture_db())
        .arg("--journal")
        .arg(std::env::temp_dir().join(format!("ai-usage-codex-{}.db", std::process::id())))
        .arg("--claude-dir")
        .arg(format!(
            "{}/tests/fixtures/no-such-claude-dir",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg("--codex-dir")
        .arg(&codex_home)
        .arg("--omarchy-dir")
        .arg(format!(
            "{}/tests/fixtures/no-such-omarchy-dir",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg("--codex-billing")
        .arg("api");
    for name in ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_HOME"] {
        command.env_remove(name);
    }
    let output = command.output().expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("FIXTURE_SECRET"),
        "rollout content reached the export:\n{stdout}"
    );

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let rows: Vec<_> = json["usage"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["provider"] == "openai")
        .collect();
    // Two billed calls in the live rollout (the repeated and the limit-only events are
    // skipped) plus one in the archived, older-nesting file.
    assert_eq!(rows.len(), 3, "{rows:#?}");
    let first = rows
        .iter()
        .find(|row| row["input_tokens"] == 400)
        .expect("the first call, with cached tokens split out");
    assert_eq!(first["cache_read_tokens"], 800);
    assert_eq!(first["output_tokens"], 240);
    assert_eq!(first["reasoning_tokens"], 100);
    assert_eq!(first["model"], "gpt-5-codex");
    assert_eq!(first["cost_status"], "estimated");
    assert_eq!(first["project"], "/home/fixture/project");
    assert_eq!(first["session_id"], "0198f4c2-7d1e-7a3b-9c11-3e5a6b7c8d90");
    let archived = rows
        .iter()
        .find(|row| row["model"] == "gpt-5.1-codex-max")
        .expect("the archived rollout is scanned too");
    assert_eq!(
        archived["session_id"],
        "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
    );
    assert!(
        json["source"].as_str().unwrap().contains("Codex:"),
        "{}",
        json["source"]
    );
}

#[test]
fn json_carries_omarchy_limits_and_nothing_else_from_the_records() {
    let fixtures = format!("{}/tests/fixtures/omarchy", env!("CARGO_MANIFEST_DIR"));
    let output = hermetic(bin().arg("--json"))
        .arg("--omarchy-dir")
        .arg(&fixtures)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for forbidden in ["authHelpText", "claude auth login", "modelUsage", "345678"] {
        assert!(
            !stdout.contains(forbidden),
            "{forbidden} reached the export:\n{stdout}"
        );
    }
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let limits = json["limits"].as_array().expect("limits array");
    assert_eq!(limits.len(), 1, "{limits:#?}");
    assert_eq!(limits[0]["agent"], "claude");
    assert_eq!(limits[0]["tier"], "Max 20x");
    assert_eq!(limits[0]["windows"][0]["label"], "Session (5-hour)");
    assert_eq!(limits[0]["windows"][0]["percent_used"], 92.0);
    assert_eq!(
        limits[0]["stale"], true,
        "the fixture is dated 2026-08-23 and this is later"
    );

    // Present and empty when there is nothing to read, so a consumer can key on it.
    let output = hermetic(bin().arg("--json")).output().expect("run");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(json["limits"], serde_json::json!([]));
}

#[test]
fn an_omarchy_record_is_written_only_when_asked() {
    let temp = std::env::temp_dir().join(format!("ai-usage-omarchy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let state = temp.join("state");
    let config = temp.join("config.toml");
    std::fs::write(
        &config,
        "[[budgets.entry]]\nscope = \"global\"\nperiod = \"monthly\"\nlimit = 10.0\n",
    )
    .unwrap();

    // The whole default path — not just the flag — must stay write-free: XDG_STATE_HOME is
    // where the record would land if anything wrote one uninvited.
    let output = hermetic(bin().arg("--json"))
        .env("XDG_STATE_HOME", &state)
        .output()
        .expect("run");
    assert!(output.status.success());
    assert!(!state.exists(), "an export must not create Omarchy state");

    let usage_dir = temp.join("usage");
    let output = bin()
        .arg("--omarchy-record")
        .arg("--config")
        .arg(&config)
        .arg("--db")
        .arg(fixture_db())
        .arg("--journal")
        .arg(temp.join("journal.db"))
        .arg("--claude-dir")
        .arg(temp.join("no-claude"))
        .arg("--codex-dir")
        .arg(temp.join("no-codex"))
        .arg("--omarchy-dir")
        .arg(&usage_dir)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Wrote Omarchy record"), "{stdout}");
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(usage_dir.join("opencode.json")).unwrap())
            .unwrap();
    assert_eq!(record["id"], "opencode");
    assert_eq!(record["schemaVersion"], 1);
    assert!(record["totalPrompts"].as_u64().unwrap() > 0, "{record}");
    assert_eq!(record["limits"][0]["title"], "Monthly budget");
    assert!(record.get("balance").is_none(), "balance is opt-in");
    let names: Vec<String> = std::fs::read_dir(&usage_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["opencode.json"], "no temporary left behind");

    let _ = std::fs::remove_dir_all(&temp);
}
