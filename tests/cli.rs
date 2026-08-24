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
