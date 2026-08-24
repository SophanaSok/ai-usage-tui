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
        .arg("--all")
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
