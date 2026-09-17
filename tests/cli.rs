//! End-to-end checks that run the built binary.
//!
//! These exist for behaviour that only appears in a real process: exit codes, and what happens
//! to stdout when the thing reading it goes away.

use std::io::Read;
use std::path::{Path, PathBuf};
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
    hermetic_with(
        command,
        &PathBuf::from(fixture_db()),
        &PathBuf::from(format!(
            "{}/tests/fixtures/no-such-journal.db",
            env!("CARGO_MANIFEST_DIR")
        )),
    )
}

/// `hermetic`, with the two inputs a test sometimes brings its own of.
fn hermetic_with<'a>(command: &'a mut Command, db: &Path, journal: &Path) -> &'a mut Command {
    let nowhere = |name: &str| format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    command
        // `--claude-dir` does NOT pin the config document: `config_json_path` checks
        // `CLAUDE_CONFIG_DIR` *before* it derives one from the session-log root, and
        // `CLAUDE_PROJECTS_DIR` stands in for the flag when it is absent. So on a developer's
        // machine with either exported, a fixture-only run resolved their real `~/.claude.json`.
        // That was survivable while the only reader was billing detection -- it cost a tier
        // label. It is not survivable now that the limits reader consumes the same document:
        // the developer's actual subscription percentages would appear in `--json`.
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_PROJECTS_DIR")
        // Neither the config file nor the pricing cache is a flag every test passes, so both
        // are pinned through the environment: a developer's `config.toml` or a refreshed
        // `zen-pricing.toml` must not change what a fixture-only run prints.
        .env("XDG_CONFIG_HOME", nowhere("no-such-config-home"))
        .env("XDG_DATA_HOME", nowhere("no-such-data-home"))
        .arg("--db")
        .arg(db)
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
        .arg("--copilot-dir")
        .arg(format!(
            "{}/tests/fixtures/no-such-copilot-home",
            env!("CARGO_MANIFEST_DIR")
        ))
        // Gemini reads `~/.gemini/telemetry.json` when left unpinned, so a developer who has
        // switched Gemini's telemetry on saw their own rows in a fixture-only run. Every source
        // in the registry belongs in this list; this one was missed when it shipped.
        .arg("--gemini-dir")
        .arg(format!(
            "{}/tests/fixtures/no-such-gemini-home",
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
        .arg(journal)
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
    // of the diagnosis shows up here. Asked of the registry: this was a list of seven names, and
    // a list kept in a test passes for the eighth source without ever looking for it.
    for id in ai_usage_tui::collector::registry::ids() {
        assert!(
            source_line(&text, id).is_some(),
            "--doctor has no line for {id}:\n{text}"
        );
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
    // The pricing table's state, warnings included: a stale or invalid cache was ignored in
    // favour of bundled rates and said so nowhere.
    assert!(text.contains("PRICING"), "no pricing section:\n{text}");
    assert!(text.contains("models") && text.contains("priced"), "{text}");
    // The absence hint used to name `--refresh-zen`, which writes a file nothing prices from.
    assert!(
        !text.contains("--refresh-zen"),
        "--doctor points at the catalog refresh, not the pricing one:\n{text}"
    );
}

/// A source's own row in `--doctor`'s SOURCES block: the one that carries the path searched.
fn source_line<'a>(doctor: &'a str, id: &str) -> Option<&'a str> {
    doctor
        .lines()
        .skip_while(|line| line.trim() != "SOURCES")
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .find(|line| line.split_whitespace().next() == Some(id))
}

/// `hermetic()` pins every source the registry knows, asked of the binary rather than of a list.
///
/// Every test in this file trusts `hermetic()` to keep the developer's own data out of a
/// fixture-only run, and it is a hand-written list of flags: Copilot and Gemini each shipped
/// outside it, and a machine with their data printed real rows in tests. `--doctor` prints where
/// each source resolved, so the question can be asked directly -- and asked this way it also
/// catches a source reached through an environment variable, which no list of flags could.
#[test]
fn hermetic_pins_every_registered_source() {
    let fixtures = format!("{}/tests/fixtures/", env!("CARGO_MANIFEST_DIR"));
    let output = hermetic(bin().arg("--doctor"))
        .output()
        .expect("run --doctor");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("utf8");

    let ids = ai_usage_tui::collector::registry::ids();
    assert!(ids.len() >= 7, "the registry lost sources: {ids:?}");
    for id in ids {
        let line = source_line(&text, id)
            .unwrap_or_else(|| panic!("--doctor has no line for {id}:\n{text}"));
        assert!(
            line.contains(&fixtures),
            "{id} is not pinned by hermetic(): it resolved outside tests/fixtures, so every test \
             here can read this machine's real data. Pin it in hermetic_with.\n{line}"
        );
    }
    // The config file is read before any source is, and is pinned through the environment.
    let config = text
        .lines()
        .skip_while(|line| line.trim() != "CONFIG")
        .nth(1)
        .expect("a CONFIG section");
    assert!(
        config.contains(&fixtures),
        "the config file is not pinned:\n{config}"
    );
}

/// The arguments of a documented `cargo run --locked -- …` command, continuation lines joined.
fn documented_run_args(text: &str, opening: &str) -> Vec<String> {
    let start = text
        .find(opening)
        .unwrap_or_else(|| panic!("no command starting {opening:?}"));
    let mut command = String::new();
    for line in text[start..].lines() {
        let line = line.trim();
        command.push_str(line.trim_end_matches('\\'));
        command.push(' ');
        if !line.ends_with('\\') {
            break;
        }
    }
    command
        .split_whitespace()
        .skip_while(|word| *word != "--")
        .skip(1)
        .filter(|word| !word.starts_with("{{"))
        .map(str::to_string)
        .collect()
}

/// The two commands the project tells a contributor to run against the fixture pin every source.
///
/// `CONTRIBUTING.md` and `just run` each carry a hand-written list of root flags and call the
/// result hermetic. Both predate Copilot and Gemini and neither was updated for them, so the
/// documented "fixture-only" run printed the reader's own rows. The commands are run as written,
/// with the action swapped for `--doctor`, which says where each source resolved.
#[test]
fn documented_fixture_commands_pin_every_source() {
    let root = env!("CARGO_MANIFEST_DIR");
    let read = |name: &str| std::fs::read_to_string(format!("{root}/{name}")).expect(name);
    for (name, opening) in [
        ("CONTRIBUTING.md", "cargo run --locked -- --json"),
        ("justfile", "cargo run --locked -- --db"),
    ] {
        let args: Vec<String> = documented_run_args(&read(name), opening)
            .into_iter()
            .filter(|arg| arg != "--json")
            .collect();
        assert!(
            args.len() >= 10,
            "{name}: did not find the command: {args:?}"
        );
        let output = bin()
            .current_dir(root)
            .arg("--doctor")
            .args(&args)
            // The config file and the pricing cache are not sources a flag names; pinned here
            // so the run is about the flags.
            .env("XDG_CONFIG_HOME", "/nonexistent/config-home")
            .env("XDG_DATA_HOME", "/nonexistent/data-home")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CLAUDE_PROJECTS_DIR")
            .output()
            .expect("run --doctor");
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).expect("utf8");
        for id in ai_usage_tui::collector::registry::ids() {
            let line = source_line(&text, id)
                .unwrap_or_else(|| panic!("{name}: --doctor has no line for {id}:\n{text}"));
            assert!(
                line.contains("/nonexistent") || line.contains("tests/fixtures/"),
                "{name}: the documented fixture command does not pin {id}, so it reads the \
                 reader's own data:\n{line}"
            );
        }
    }
}

/// A Claude Code session-log root holding one request dated *now*.
///
/// The committed fixtures are years old, so nothing in them falls inside `--today`, a budget's
/// period or a burn window. Anything that has to see current data brings this.
fn recent_claude_home(dir: &Path) -> PathBuf {
    let recent = dir.join("recent").join(".claude").join("projects");
    std::fs::create_dir_all(recent.join("p")).expect("recent claude home");
    std::fs::write(
        recent.join("p").join("s.jsonl"),
        format!(
            "{{\"type\":\"assistant\",\"uuid\":\"u-now\",\"requestId\":\"req_now\",\
             \"timestamp\":\"{}\",\"sessionId\":\"s-now\",\"cwd\":\"/work/app\",\"message\":{{\"id\":\"msg_now\",\
             \"role\":\"assistant\",\"model\":\"claude-sonnet-4-5-20250929\",\
             \"usage\":{{\"input_tokens\":100000,\"output_tokens\":5000}}}}}}\n",
            chrono::Utc::now().to_rfc3339()
        ),
    )
    .expect("write recent session");
    recent
}

/// Every fenced block of a guide, as `(language, body)`.
#[cfg(unix)]
fn fences(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut open: Option<(String, String)> = None;
    for line in text.lines() {
        match (&mut open, line.strip_prefix("```")) {
            (None, Some(language)) => open = Some((language.trim().to_string(), String::new())),
            (Some(_), Some(_)) => found.extend(open.take()),
            (Some((_, body)), None) => {
                body.push_str(line);
                body.push('\n');
            }
            (None, None) => {}
        }
    }
    found
}

/// The recipes an agent is handed are run, as written, against fixture data.
///
/// `--agent-guide recipes` and `extend` hand an agent shell to adapt. A recipe that names a key
/// the summary no longer has, or pipes `null` into arithmetic, would be copied into a user's
/// status bar by something that cannot tell. So each block that needs only `sh` and `jq` is
/// executed with `ai-usage-tui` on its `PATH` replaced by a shim that pins every source -- the
/// recipe's own flags still apply, because a range flag given later wins.
///
/// What this catches: a recipe that does not parse, that fails on real output, or that prints
/// `null` where it promised a value. What it cannot: a misspelt key whose `null` the recipe then
/// handles politely. The schema guard covers the keys; this covers the scripts.
#[cfg(unix)]
#[test]
fn every_runnable_recipe_runs_against_the_fixture() {
    use std::os::unix::fs::PermissionsExt;

    let Some(jq) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("jq"))
            .find(|candidate| candidate.is_file())
    }) else {
        // CI has `jq`; a contributor's machine may not, and a test that needs an extra program
        // to pass is a worse trade than one that says loudly it did not run.
        assert!(
            std::env::var_os("CI").is_none(),
            "jq is not installed, so the recipes in --agent-guide were not run"
        );
        eprintln!("SKIPPED: jq is not installed, so the recipes in --agent-guide were not run");
        return;
    };

    let dir = scratch("recipes");
    let fixtures = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let recent = recent_claude_home(&dir);
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        "[budgets]\n[[budgets.entry]]\nscope = \"global\"\nperiod = \"monthly\"\nlimit = 0.0001\n",
    )
    .expect("write config");

    let tools = dir.join("bin");
    std::fs::create_dir_all(&tools).expect("bin");
    let shim = tools.join("ai-usage-tui");
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\nexec '{bin}' --config '{config}' --db '{fixtures}/opencode_test.db' \
             --claude-dir '{recent}' --claude-billing api --codex-dir '{fixtures}/codex_home' \
             --copilot-dir '{fixtures}/copilot_home' --gemini-dir /nonexistent \
             --omarchy-dir '{fixtures}/omarchy' --journal '{journal}' \"$@\"\n",
            bin = env!("CARGO_BIN_EXE_ai-usage-tui"),
            config = config.display(),
            recent = recent.display(),
            journal = dir.join("usage.db").display(),
        ),
    )
    .expect("write shim");
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    std::os::unix::fs::symlink(&jq, tools.join("jq")).expect("link jq");

    // A recipe that names a source root or a config would override the pins -- and would be a
    // recipe that only works on its author's machine.
    let pinned: Vec<String> = ai_usage_tui::cli::command()
        .get_arguments()
        .filter_map(|arg| arg.get_long())
        .filter(|long| {
            long.ends_with("-dir") || ["db", "journal", "config", "claude-json"].contains(long)
        })
        .map(|long| format!("--{long}"))
        .collect();
    assert!(pinned.len() >= 8, "{pinned:?}");

    let mut ran = 0;
    for (guide, text) in [
        ("recipes", ai_usage_tui::schema::AGENT_RECIPES),
        ("extend", ai_usage_tui::schema::AGENT_EXTEND),
    ] {
        for (language, body) in fences(text) {
            let first = body.lines().next().unwrap_or_default();
            let needs = first
                .trim_start_matches(['#', '/', ' '])
                .strip_prefix("needs:");
            if guide == "recipes" {
                assert!(
                    needs.is_some(),
                    "a block in the recipes guide does not open with what it needs:\n{body}"
                );
            }
            let runnable = language == "sh"
                && needs.is_some_and(|needs| needs.split(',').all(|need| need.trim() == "jq"));
            if !runnable {
                continue;
            }
            for flag in &pinned {
                assert!(
                    !body.split_whitespace().any(|word| word == flag),
                    "a recipe passes {flag}, which belongs to the user's setup:\n{body}"
                );
            }
            let output = Command::new("sh")
                .arg("-c")
                .arg(&body)
                .current_dir(&dir)
                .env("PATH", format!("{}:/usr/bin:/bin", tools.display()))
                .env("XDG_CONFIG_HOME", "/nonexistent/config-home")
                .env("XDG_DATA_HOME", "/nonexistent/data-home")
                .env_remove("CLAUDE_CONFIG_DIR")
                .env_remove("CLAUDE_PROJECTS_DIR")
                .output()
                .expect("run sh");
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            // A recipe whose exit status is its answer says so on a line of its own.
            let answers_by_exit = body.lines().any(|line| line.starts_with("# exit:"));
            let code = output.status.code();
            assert!(
                code == Some(0) || (answers_by_exit && code == Some(1)),
                "a recipe in --agent-guide {guide} exited {code:?}:\n{body}\n{stderr}"
            );
            assert!(stderr.trim().is_empty(), "{guide}:\n{body}\n{stderr}");
            if !answers_by_exit {
                assert!(
                    !stdout.trim().is_empty(),
                    "{guide}: printed nothing:\n{body}"
                );
            }
            assert!(
                !stdout.contains("null"),
                "{guide}: a recipe printed `null` where it promised a value:\n{body}\n{stdout}"
            );
            ran += 1;
        }
    }
    assert!(
        ran >= 5,
        "only {ran} recipes ran; the guides lost their runnable blocks"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--agent-guide` takes an optional topic, and bare it prints what it always printed: every
/// installed skill and pasted `AGENTS.md` block says "run `--agent-guide`", and they outlive the
/// binary they were written for.
#[test]
fn the_agent_guide_topics_print_their_documents() {
    use clap::ValueEnum;
    let print = |args: &[&str]| {
        let output = bin().args(args).output().expect("run");
        assert!(output.status.success(), "{args:?}");
        String::from_utf8(output.stdout).expect("utf8")
    };
    assert_eq!(print(&["--agent-guide"]), ai_usage_tui::schema::AGENT_GUIDE);
    for topic in ai_usage_tui::schema::GuideTopic::value_variants() {
        let name = topic
            .to_possible_value()
            .expect("named")
            .get_name()
            .to_string();
        assert_eq!(print(&["--agent-guide", &name]), topic.text(), "{name}");
    }
    // A topic given after another flag is still the topic, not a stray argument.
    assert_eq!(
        print(&["--agent-guide=setup"]),
        ai_usage_tui::schema::AGENT_SETUP
    );

    // An unknown topic fails and names the real ones: that error is how an agent on an older
    // binary learns a topic does not exist there yet.
    let output = bin().args(["--agent-guide", "nope"]).output().expect("run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("read") && stderr.contains("setup"),
        "{stderr}"
    );
}

/// A refreshed pricing cache the engine refuses is named, with why, where the user looks.
#[test]
fn doctor_reports_a_pricing_cache_it_could_not_use() {
    let dir = scratch("bad-pricing-cache");
    let data_home = dir.join("data");
    std::fs::create_dir_all(data_home.join("ai-usage-tui")).expect("data home");
    std::fs::write(
        data_home.join("ai-usage-tui").join("zen-pricing.toml"),
        "not = [toml\n",
    )
    .expect("plant a broken cache");

    let output = hermetic(bin().arg("--doctor"))
        // Overrides the pin `hermetic` sets: this test wants the cache read.
        .env("XDG_DATA_HOME", &data_home)
        .output()
        .expect("run --doctor");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        text.contains("warning") && text.contains("is invalid"),
        "the refused cache is not reported:\n{text}"
    );
    assert!(
        text.contains("ignored"),
        "the cache line reads as in use beside a warning that it is not:\n{text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A usage row the journal cannot read is counted where the user looks, not dropped.
#[test]
fn doctor_reports_journal_rows_it_could_not_read() {
    let dir = scratch("corrupt-journal");
    let journal = dir.join("usage.db");
    let conn = rusqlite::Connection::open(&journal).expect("open");
    conn.execute_batch(
        "CREATE TABLE usage_event (
            id INTEGER PRIMARY KEY, event_id TEXT, provider TEXT NOT NULL, model TEXT NOT NULL,
            category TEXT NOT NULL, cost_status TEXT NOT NULL, requests INTEGER NOT NULL,
            input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
            reasoning_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
            cache_write_tokens INTEGER NOT NULL, cost REAL, created INTEGER NOT NULL
        );
        INSERT INTO usage_event (provider, model, category, cost_status, requests, input_tokens,
            output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens, cost, created)
        VALUES ('ollama', 'm', 'LOCAL', 'local', 1, 10, 10, 0, 0, 0, NULL, 1),
               ('ollama', 'm', 'LOCAL', 'local', 1, 10, 10, 0, 0, 0, NULL, 'soon');",
    )
    .expect("plant rows");
    drop(conn);

    let run = |action: &str| {
        let output = hermetic_with(
            bin().arg(action),
            &PathBuf::from(format!(
                "{}/tests/fixtures/no-such.db",
                env!("CARGO_MANIFEST_DIR")
            )),
            &journal,
        )
        .output()
        .expect("run");
        assert!(
            output.status.success(),
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8")
    };

    // `--doctor` carries the first reason beside the source.
    let text = run("--doctor");
    assert!(
        text.contains("could not be read") && text.contains("id 2"),
        "the skipped row is not reported:\n{text}"
    );
    // The source's status line — the `--once` header, and `--json`'s `source` — carries the
    // count.
    let json: serde_json::Value = serde_json::from_str(&run("--json")).expect("json parses");
    let source = json["source"].as_str().unwrap_or_default();
    assert!(
        source.contains("1 row(s) unreadable"),
        "the status line does not carry the count: {source}"
    );
    assert_eq!(
        json["usage"].as_array().map(Vec::len),
        Some(1),
        "the readable row survives"
    );

    let _ = std::fs::remove_dir_all(&dir);
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

/// The other half of the journal's write path: every local server that is not Ollama.
///
/// Both fixtures are real captures from `llama-server` on an author's machine. Until v0.16.0 the
/// only recorder spoke Ollama's format and hardcoded its provider, so a machine running llama.cpp
/// — the common case for a 16GB card — saw an empty dashboard and no hint as to why.
#[test]
fn recording_an_openai_compatible_response_round_trips_and_splits_cached_tokens() {
    let dir = scratch("llamacpp-journal");
    let journal = dir.join("usage.db");
    let fixture = format!(
        "{}/tests/fixtures/llamacpp_chat.json",
        env!("CARGO_MANIFEST_DIR")
    );

    record(&journal, &fixture, "--record-usage=llamacpp");
    let rows = journal_rows(&journal);
    assert_eq!(
        rows.len(),
        1,
        "one response should journal one row: {rows:?}"
    );
    assert_eq!(rows[0]["provider"], "llamacpp");
    assert_eq!(rows[0]["model"], "qwen3.6-35b-a3b");
    // `prompt_tokens` was 11, of which the server had cached 7.
    assert_eq!(rows[0]["input_tokens"], 4, "{rows:?}");
    assert_eq!(rows[0]["cache_read_tokens"], 7, "{rows:?}");
    assert_eq!(rows[0]["output_tokens"], 8);
    assert_eq!(rows[0]["category"], "LOCAL");
    assert_eq!(rows[0]["cost_status"], "local");

    record(&journal, &fixture, "--record-usage=llamacpp");
    assert_eq!(
        journal_rows(&journal).len(),
        1,
        "recording the same response twice double-counted it"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Raw server-sent events, `data:` prefixes and `[DONE]` included, pipe in without a `jq`.
#[test]
fn a_streamed_openai_compatible_response_journals_once_from_its_usage_chunk() {
    let dir = scratch("llamacpp-stream");
    let journal = dir.join("usage.db");
    record(
        &journal,
        &format!(
            "{}/tests/fixtures/llamacpp_stream.sse",
            env!("CARGO_MANIFEST_DIR")
        ),
        "--record-usage=llamacpp",
    );

    let rows = journal_rows(&journal);
    assert_eq!(
        rows.len(),
        1,
        "a stream should journal one row, not one per chunk: {rows:?}"
    );
    assert_eq!(rows[0]["output_tokens"], 24, "{rows:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A streamed response that never carried usage is an error, not a row of zeros.
#[test]
fn a_response_without_usage_fails_loudly() {
    let dir = scratch("llamacpp-no-usage");
    let journal = dir.join("usage.db");
    let chunk = dir.join("chunk.json");
    std::fs::write(
        &chunk,
        br#"{"model":"m","choices":[{"delta":{"content":"hi"}}]}"#,
    )
    .expect("write chunk");

    let output = crate::bin()
        .arg("--record-usage=llamacpp")
        .arg("--journal")
        .arg(&journal)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(&std::fs::read(&chunk).expect("read"))?;
            child.wait_with_output()
        })
        .expect("run");
    assert!(
        !output.status.success(),
        "a response with no usage must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("include_usage"),
        "the error must name the flag that fixes it: {stderr}"
    );
    assert!(!journal.exists(), "nothing should have been written");

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
    // `"pass"` was silently mapped to null before, and this test asserted the three counters
    // beside it and not the result.
    assert_eq!(agg["test_passes"], 1, "{json}");
    // Two retries on one task is one task that retried: 100%, not 200%.
    assert_eq!(agg["retry_rate"], 100.0, "{json}");
    assert_eq!(agg["retries_observed"], 1, "{json}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// An emitter that reports no counters — which an automated one cannot, having nothing to count
/// — must export them as unknown, not as a clean run.
#[test]
fn unreported_routing_counters_export_as_null_not_zero() {
    let dir = scratch("routing-unreported");
    let journal = dir.join("usage.db");
    let event = dir.join("event.json");
    std::fs::write(
        &event,
        r#"{"task":"t-1","agent":"drafter","model":"claude-haiku-4-5","provider":"anthropic",
            "tokens":500,"cost":0.01,"cost_status":"reported"}"#,
    )
    .expect("write event");
    record(&journal, event.to_str().unwrap(), "--record-routing");

    let output = bin()
        .arg("--routing-json")
        .arg("--journal")
        .arg(&journal)
        .output()
        .expect("run --routing-json");
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("routing json parses");
    let agg = json["aggregates"][0].as_object().expect("aggregate object");
    for field in [
        "retries",
        "escalations",
        "review_defects",
        "retry_rate",
        "escalation_rate",
        "defect_rate",
    ] {
        // Present and null — an absent key would also index as null and prove nothing.
        assert!(agg.contains_key(field), "{field} missing from {json}");
        assert!(agg[field].is_null(), "{field} should be null, got {json}");
    }
    for field in [
        "retries_observed",
        "escalations_observed",
        "review_defects_observed",
    ] {
        assert_eq!(agg[field], 0, "{field}: {json}");
    }

    let csv_path = dir.join("routing.csv");
    let output = bin()
        .arg("--routing-csv")
        .arg(&csv_path)
        .arg("--journal")
        .arg(&journal)
        .output()
        .expect("run --routing-csv");
    assert!(output.status.success());
    let csv = std::fs::read_to_string(&csv_path).expect("csv");
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().expect("header").split(',').collect();
    let row: Vec<&str> = lines.next().expect("row").split(',').collect();
    // Existing columns keep their positions; the denominators are appended after everything.
    assert_eq!(
        (header[6], header[7], header[10]),
        ("retries", "escalations", "review_defects"),
        "{csv}"
    );
    assert_eq!(
        (row[6], row[7], row[10]),
        ("", "", ""),
        "an unreported count is an empty field, not 0:\n{csv}"
    );
    assert_eq!(
        header[15..],
        [
            "retries_observed",
            "escalations_observed",
            "review_defects_observed"
        ],
        "{csv}"
    );
    assert_eq!(row[15..], ["0", "0", "0"], "{csv}");

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
    // Through `hermetic_with` rather than its own hand-written list of dirs, which is what let
    // this one drift: it pinned Claude Code, Codex and Omarchy and nothing else, so a developer
    // with a real `~/.copilot` store or Gemini telemetry switched on saw their own rows in a
    // journal-only assertion. The same gap `hermetic_with` closed for `--gemini-dir`, in the one
    // helper that was not using it.
    let mut command = bin();
    command.arg("--json");
    hermetic_with(
        &mut command,
        std::path::Path::new("/nonexistent/opencode.db"),
        journal,
    );
    let output = command.output().expect("run --json");
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

/// `--record-event` is the way in for a tool with no collector: an adapter prints the tool's own
/// usage in this tool's terms. What it has to prove end to end is that the rows arrive *as usage*
/// -- in the project and session views, which the response recorders could never reach -- and
/// that the two statements about money an adapter may make are kept exactly as made.
#[test]
fn a_recorded_event_round_trips_with_project_session_and_cost() {
    let dir = scratch("record-event");
    let journal = dir.join("usage.db");
    let fixture = format!(
        "{}/tests/fixtures/usage_events.ndjson",
        env!("CARGO_MANIFEST_DIR")
    );
    record(&journal, &fixture, "--record-event");
    record(&journal, &fixture, "--record-event");
    let rows = journal_rows(&journal);
    assert_eq!(
        rows.len(),
        2,
        "replaying the adapter's output double-counted it: {rows:?}"
    );

    let plan = rows
        .iter()
        .find(|row| row["model"] == "claude-sonnet-5")
        .expect("the plan row");
    assert_eq!(plan["billing"], "subscription");
    assert_eq!(plan["cost_status"], "quota");
    assert!(
        plan["cost"].is_null(),
        "plan-billed work has no per-request price: {plan}"
    );
    assert!(
        plan["api_equivalent_cost"]
            .as_f64()
            .is_some_and(|cost| cost > 0.0),
        "a subscription row carries the list-rate figure beside it, as a native one does: {plan}"
    );
    assert_eq!(plan["cache_write_tokens"], 50);
    assert_eq!(plan["session_id"], "aider-s1");
    assert_eq!(
        plan["project"], "/work/app",
        "stored as the collectors store it"
    );

    let paid = rows
        .iter()
        .find(|row| row["model"] == "gpt-5")
        .expect("the paid row");
    assert_eq!(paid["cost_status"], "reported");
    assert_eq!(
        paid["cost"], 0.0123,
        "a reported cost is kept, not re-estimated: {paid}"
    );

    // The views a bare response never reached, and the filter spelled with the trailing slash
    // the adapter sent.
    let mut command = bin();
    command.args(["--summary-json", "--project", "/work/app/"]);
    hermetic_with(
        &mut command,
        Path::new("/nonexistent/opencode.db"),
        &journal,
    );
    let output = command.output().expect("run");
    assert!(output.status.success());
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(summary["totals"]["requests"], 2, "{summary}");
    assert_eq!(summary["totals"]["quota_requests"], 1);
    assert_eq!(summary["totals"]["cost"], 0.0123);
    let text = summary.to_string();
    assert!(
        text.contains("aider-s1"),
        "the session is in no view: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A refused batch is refused whole: non-zero, the reason on stderr, and no journal. The adapter's
/// author -- often a model -- has nothing else to learn from.
#[test]
fn a_usage_event_that_cannot_be_read_refuses_the_batch() {
    use std::io::Write;
    let dir = scratch("record-event-refused");
    for (input, expected) in [
        // No counts: the tool measures none, and estimating them is what this refuses.
        (r#"{"provider":"cursor","model":"m","created":1}"#, "do not estimate"),
        // A line that is not JSON, between two that are.
        (
            "{\"provider\":\"a\",\"model\":\"m\",\"input_tokens\":1,\"output_tokens\":1,\"created\":1}\nnot json\n{\"provider\":\"a\",\"model\":\"m\",\"input_tokens\":1,\"output_tokens\":1,\"created\":2}",
            "not JSON",
        ),
        ("", "nothing on stdin"),
    ] {
        let journal = dir.join("usage.db");
        let mut child = bin()
            .args(["--record-event", "--journal"])
            .arg(&journal)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
        let output = child.wait_with_output().expect("wait");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted {input:?}");
        assert!(stderr.contains(expected), "{input:?}: {stderr}");
        assert!(!journal.exists(), "a refused batch created a journal: {input:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Plan-billed events stay out of a budget's dollars and are counted beside them, exactly as a
/// native subscription row is; a reported cost reaches the budget.
#[test]
fn a_subscription_event_stays_out_of_budget_spend_and_a_reported_cost_reaches_it() {
    use std::io::Write;
    let dir = scratch("record-event-budget");
    let journal = dir.join("usage.db");
    let config_home = dir.join("config");
    std::fs::create_dir_all(config_home.join("ai-usage-tui")).expect("config home");
    std::fs::write(
        config_home.join("ai-usage-tui").join("config.toml"),
        "[budgets]\n[[budgets.entry]]\nscope = \"global\"\nperiod = \"monthly\"\nlimit = 100.0\n",
    )
    .expect("write config");
    let now = chrono::Utc::now().timestamp();
    let events = format!(
        "{{\"provider\":\"aider\",\"model\":\"claude-sonnet-5\",\"event_id\":\"p\",\"created\":{now},\"input_tokens\":900000,\"output_tokens\":90000,\"billing\":\"subscription\"}}\n\
         {{\"provider\":\"aider\",\"model\":\"gpt-5\",\"event_id\":\"c\",\"created\":{now},\"input_tokens\":10,\"output_tokens\":3,\"cost\":0.25}}\n"
    );
    let mut child = bin()
        .args(["--record-event", "--journal"])
        .arg(&journal)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(events.as_bytes())
        .unwrap();
    assert!(child.wait().expect("wait").success());

    let mut command = bin();
    command.arg("--summary-json");
    hermetic_with(
        &mut command,
        Path::new("/nonexistent/opencode.db"),
        &journal,
    );
    command.env("XDG_CONFIG_HOME", &config_home);
    let output = command.output().expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let summary: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    let budget = &summary["budgets"][0];
    assert_eq!(budget["spend"], 0.25, "{budget}");
    assert_eq!(budget["quota_requests"], 1, "{budget}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_recorders_survive_a_closed_pipe_after_journaling() {
    // Each recorder confirmed with a bare `println!` *after* writing its row, so a caller that
    // discards stdout by closing it -- a hook runner, `| head -0` -- got a journaled row and a
    // panic, and an exit status that told it the recording failed. The closed pipe is set up
    // before stdin is written, and the child cannot print before stdin closes, so the order is
    // not a race.
    use std::io::Write;
    let fixtures = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    for (flag, fixture, rows) in [
        ("--record-ollama", "ollama_single.json", 1),
        ("--record-usage=llamacpp", "llamacpp_chat.json", 1),
        ("--record-event", "usage_events.ndjson", 2),
    ] {
        let dir = scratch(&format!("closed-pipe-{}", flag.trim_start_matches('-')));
        let journal = dir.join("usage.db");
        let mut child = bin()
            .arg(flag)
            .arg("--journal")
            .arg(&journal)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");

        drop(child.stdout.take());
        let body = std::fs::read(format!("{fixtures}/{fixture}")).expect("read fixture");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(&body)
            .expect("write stdin");

        let mut stderr = String::new();
        if let Some(mut handle) = child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }
        let status = child.wait().expect("wait");

        assert!(
            !stderr.contains("panicked"),
            "{flag} panicked writing to a closed pipe:\n{stderr}"
        );
        assert!(
            status.success(),
            "{flag} on a closed pipe should exit cleanly, got {status}"
        );
        assert_eq!(
            journal_rows(&journal).len(),
            rows,
            "{flag} journaled what it was sent"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn the_flags_that_describe_the_cli_survive_a_closed_pipe_too() {
    // `--json` and `--once` were covered; the four flags that describe the CLI itself were not,
    // and two of them panicked. `print_help` ended with a bare `println!()`, and
    // `print_completions` discarded the write error clap_complete swallows -- so
    // `ai-usage-tui --help | head` aborted with "failed printing to stdout: Broken pipe" while
    // `--man | head`, which returns io::Result, ended cleanly. Three near-identical paths, one
    // of them right.
    //
    // These take no `hermetic()`: they read no source, which is the point of running before the
    // config is loaded.
    for flag in [
        vec!["--help"],
        vec!["--version"],
        vec!["--man"],
        vec!["--completions", "bash"],
    ] {
        let mut child = bin()
            .args(&flag)
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
            "{flag:?} panicked writing to a closed pipe:\n{stderr}"
        );
        assert!(
            status.success(),
            "{flag:?} on a closed pipe should exit cleanly, got {status}"
        );
    }
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
    // Asserted on substance rather than phrasing: the hand-rolled parser said "unknown option",
    // clap says "unexpected argument". What has to hold either way is that the offending flag is
    // named, the user is pointed somewhere useful, and nothing panicked.
    assert!(stderr.contains("--not-a-real-flag"), "{stderr}");
    assert!(stderr.contains("--help"), "{stderr}");
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

/// A Claude Code home whose one session opens on a cheap model and moves to a pricier one.
///
/// Two requests in one `sessionId`, which is what `escalation::derive` needs: a session with a
/// single request cannot show a change, and a row with no session id cannot be placed in a
/// sequence at all.
fn escalating_claude_home(root: &std::path::Path) -> std::path::PathBuf {
    let projects = root.join(".claude").join("projects");
    let project = projects.join("p");
    std::fs::create_dir_all(&project).unwrap();
    let line = |uuid: &str, msg: &str, model: &str, at: &str, out: u64| {
        format!(
            "{{\"type\":\"assistant\",\"uuid\":\"{uuid}\",\"requestId\":\"req_{uuid}\",\
             \"timestamp\":\"{at}\",\"sessionId\":\"s-esc\",\"message\":{{\"id\":\"{msg}\",\
             \"role\":\"assistant\",\"model\":\"{model}\",\
             \"usage\":{{\"input_tokens\":1000,\"output_tokens\":{out}}}}}}}\n"
        )
    };
    std::fs::write(
        project.join("s.jsonl"),
        format!(
            "{}{}",
            line(
                "u-1",
                "msg_1",
                "claude-sonnet-4-5-20250929",
                "2026-08-18T10:00:00Z",
                500
            ),
            line(
                "u-2",
                "msg_2",
                "claude-opus-4-1-20250805",
                "2026-08-18T10:05:00Z",
                900
            ),
        ),
    )
    .unwrap();
    projects
}

/// Derived escalations reach `--json`. They used to be visible only in the dashboard.
#[test]
fn derived_escalations_are_exported() {
    let dir = scratch("escalations");
    let projects = escalating_claude_home(&dir);

    let output = bin()
        .arg("--json")
        .arg("--all")
        .arg("--claude-dir")
        .arg(&projects)
        .arg("--claude-billing")
        .arg("api")
        .arg("--db")
        .arg("/nonexistent/opencode.db")
        .arg("--codex-dir")
        .arg("/nonexistent")
        .arg("--copilot-dir")
        .arg("/nonexistent")
        .arg("--gemini-dir")
        .arg("/nonexistent")
        .arg("--omarchy-dir")
        .arg("/nonexistent")
        .arg("--journal")
        .arg("/nonexistent/journal.db")
        .output()
        .expect("run --json");
    assert!(
        output.status.success(),
        "exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    let escalations = &json["escalations"];

    assert_eq!(escalations["sessions_examined"], 1, "{escalations}");
    assert_eq!(escalations["sessions_escalated"], 1, "{escalations}");
    assert_eq!(escalations["escalation_rate"], 100.0, "{escalations}");

    let transitions = escalations["transitions"].as_array().expect("transitions");
    assert_eq!(transitions.len(), 1, "{escalations}");
    assert_eq!(transitions[0]["from"], "claude-sonnet-4-5-20250929");
    assert_eq!(transitions[0]["to"], "claude-opus-4-1-20250805");
    // The direction is in the numbers, not left to whoever reads the names: a model reading this
    // export called an escalation to a newer, pricier model a "downgrade".
    let from_rate = transitions[0]["from_input_rate"]
        .as_f64()
        .expect("from rate");
    let to_rate = transitions[0]["to_input_rate"].as_f64().expect("to rate");
    assert!(to_rate > from_rate, "{escalations}");
    assert_eq!(transitions[0]["sessions"], 1);
    // Opus output is priced, so the spend after the move is a real figure, not a floor.
    assert!(
        transitions[0]["cost_after"].as_f64().unwrap() > 0.0,
        "{escalations}"
    );
    assert_eq!(transitions[0]["unpriced_after"], 0);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Always present, and a rate over zero sessions is null rather than zero.
///
/// `limits` is emitted present-and-empty for the same reason: a consumer keys on the field
/// rather than having to tell "absent" from "nothing to report".
#[test]
fn the_escalation_block_is_present_even_with_nothing_to_report() {
    let output = hermetic(bin().arg("--json")).output().expect("run");
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    let escalations = &json["escalations"];

    assert!(escalations.is_object(), "the block must always be there");
    assert_eq!(escalations["sessions_examined"], 0);
    assert!(
        escalations["escalation_rate"].is_null(),
        "a rate over zero sessions is not a fact about anything: {escalations}"
    );
    assert_eq!(escalations["transitions"].as_array().map(Vec::len), Some(0));
}

/// The block must be derived from the rows the export reports, not from everything collected.
///
/// This is what fails if the derivation is handed the unfiltered set: the session is Anthropic,
/// so filtering to another provider must empty the escalations too, not just the usage rows.
#[test]
fn escalations_follow_the_same_filter_as_the_rows() {
    let dir = scratch("escalations-filter");
    let projects = escalating_claude_home(&dir);
    let run = |extra: &[&str]| {
        let mut command = bin();
        command
            .arg("--json")
            .arg("--all")
            .arg("--claude-dir")
            .arg(&projects)
            .arg("--claude-billing")
            .arg("api")
            .arg("--db")
            .arg("/nonexistent/opencode.db")
            .arg("--codex-dir")
            .arg("/nonexistent")
            .arg("--copilot-dir")
            .arg("/nonexistent")
            .arg("--gemini-dir")
            .arg("/nonexistent")
            .arg("--omarchy-dir")
            .arg("/nonexistent")
            .arg("--journal")
            .arg("/nonexistent/journal.db");
        for arg in extra {
            command.arg(arg);
        }
        let output = command.output().expect("run");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).expect("valid JSON")
    };

    let unfiltered = run(&[]);
    assert_eq!(unfiltered["escalations"]["sessions_escalated"], 1);

    // Filter to a provider the session is not on: no rows, and so nothing to escalate.
    let filtered = run(&["--provider", "openai"]);
    assert_eq!(filtered["usage"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        filtered["escalations"]["sessions_examined"], 0,
        "escalations were derived from unfiltered usage: {}",
        filtered["escalations"]
    );

    let _ = std::fs::remove_dir_all(&dir);
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
            .arg("--copilot-dir")
            .arg(temp.join("no-copilot-home"))
            .arg("--gemini-dir")
            .arg(temp.join("no-gemini-home"))
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

/// The fixture is a redacted capture of a real Copilot store: the shipping schema, including
/// the autoincrement `id`, the `sessions` table that actually carries `cwd`/`repository`, and
/// `created_at` as RFC 3339 text. Two of its three usage rows share `turn_index` 0 in one
/// session, which is the shape that made a turn-keyed identity lose a request.
#[test]
fn copilot_requests_are_exported_one_row_per_request_with_no_content() {
    let copilot_home = format!("{}/tests/fixtures/copilot_home", env!("CARGO_MANIFEST_DIR"));
    let mut command = bin();
    command
        .arg("--json")
        .arg("--all")
        .arg("--db")
        .arg(fixture_db())
        .arg("--journal")
        .arg(std::env::temp_dir().join(format!("ai-usage-copilot-{}.db", std::process::id())))
        .arg("--copilot-dir")
        .arg(&copilot_home);
    for name in ["claude", "codex", "gemini", "omarchy"] {
        command.arg(format!("--{name}-dir")).arg(format!(
            "{}/tests/fixtures/no-such-{name}-dir",
            env!("CARGO_MANIFEST_DIR")
        ));
    }
    command.env_remove("COPILOT_HOME");
    let output = command.output().expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("FIXTURE_SECRET"),
        "prompt or summary content reached the export:\n{stdout}"
    );

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let rows: Vec<_> = json["usage"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["provider"] == "github-copilot")
        .collect();
    assert_eq!(
        rows.len(),
        3,
        "one row per request, not per turn: {rows:#?}"
    );

    let same_turn: Vec<_> = rows
        .iter()
        .filter(|row| row["session_id"] == "11111111-1111-4111-8111-111111111111")
        .collect();
    assert_eq!(
        same_turn.len(),
        2,
        "both requests of one turn survive: {same_turn:#?}"
    );
    // 13873 - 13440 and 14027 - 13824: the cache buckets come back out of `input_tokens`.
    let mut inputs: Vec<i64> = same_turn
        .iter()
        .map(|row| row["input_tokens"].as_i64().unwrap())
        .collect();
    inputs.sort_unstable();
    assert_eq!(inputs, vec![203, 433]);
    // `cwd` lives on `sessions`, not on the usage table.
    assert_eq!(same_turn[0]["project"], "/home/dev/app");

    for row in &rows {
        assert!(
            row["cost"].is_null(),
            "a seat bills premium requests, not tokens: {row}"
        );
        assert_eq!(row["cost_status"], "quota", "{row}");
    }
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
        .arg("--copilot-dir")
        .arg(format!(
            "{}/tests/fixtures/no-such-copilot-home",
            env!("CARGO_MANIFEST_DIR")
        ))
        .arg("--gemini-dir")
        .arg(format!(
            "{}/tests/fixtures/no-such-gemini-home",
            env!("CARGO_MANIFEST_DIR")
        ))
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

/// `--claude-code-hook` is the third write path: a Claude Code hook payload in, a routing event
/// out, attributed from the transcript the payload names. The payloads are the ones Claude Code
/// 2.1.245 sent, and the failure arrives as `PostToolUseFailure` — the shape that matters.
#[test]
fn a_claude_code_hook_records_the_test_runs_it_observed_and_nothing_else() {
    use std::io::Write;
    let dir = scratch("claude-hook");
    let journal = dir.join("usage.db");
    let transcript = dir.join("t.jsonl");
    // Built as JSON, not spliced into a literal: a Windows path's backslashes are escapes.
    let line = |ts: &str, req: &str, model: &str, output: u64| {
        serde_json::json!({
            "type": "assistant", "timestamp": ts, "requestId": req, "sessionId": "s",
            "cwd": dir.to_string_lossy(),
            "message": {
                "id": "m", "role": "assistant", "model": model,
                "content": [{"type": "tool_use", "name": "Bash", "input": {"command": "cargo test"}}],
                "usage": {"input_tokens": 100, "output_tokens": output}
            }
        })
        .to_string()
            + "\n"
    };
    // Two requests before the first run — the second written twice, as Claude Code does.
    std::fs::write(
        &transcript,
        line("2026-08-25T21:42:04.025Z", "req_1", "claude-sonnet-5", 50)
            + &line("2026-08-25T21:42:05.506Z", "req_2", "claude-opus-5", 80)
            + &line("2026-08-25T21:42:05.819Z", "req_2", "claude-opus-5", 80),
    )
    .expect("write transcript");

    let payload = |event: &str, command: &str, tool_use_id: &str| {
        let mut payload = serde_json::json!({
            "session_id": "s",
            "transcript_path": transcript.to_string_lossy(),
            "cwd": dir.to_string_lossy(),
            "hook_event_name": event,
            "tool_name": "Bash",
            "tool_input": {"command": command, "description": "x"},
            "tool_use_id": tool_use_id,
            "duration_ms": 10
        });
        if event == "PostToolUse" {
            payload["tool_response"] = serde_json::json!({
                "stdout": "", "stderr": "", "interrupted": false, "isImage": false, "noOutputExpected": false
            });
        } else {
            payload["error"] = serde_json::json!("Exit code 1");
            payload["is_interrupt"] = serde_json::json!(false);
        }
        payload.to_string()
    };
    let hook = |body: &str| -> String {
        let mut child = bin()
            .arg("--claude-code-hook")
            .arg("--journal")
            .arg(&journal)
            .arg("--claude-dir")
            .arg(dir.join("no-claude-logs"))
            .arg("--claude-billing")
            .arg("subscription")
            .arg("--omarchy-dir")
            .arg(dir.join("no-omarchy"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        let output = child.wait_with_output().expect("wait");
        assert!(
            output.status.success(),
            "--claude-code-hook exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let aggregates = || -> serde_json::Value {
        let output = bin()
            .arg("--routing-json")
            .arg("--journal")
            .arg(&journal)
            .output()
            .expect("run --routing-json");
        assert!(output.status.success());
        serde_json::from_slice(&output.stdout).expect("routing json parses")
    };

    // An ordinary Bash call records nothing and does not create the journal.
    let out = hook(&payload("PostToolUse", "git status", "toolu_0"));
    assert!(out.starts_with("Nothing to record"), "{out}");
    assert!(
        !journal.exists(),
        "a skipped payload must not touch the journal"
    );

    // A passing run: two requests (not three lines), on quota, by the model that ran it.
    let out = hook(&payload("PostToolUse", "cargo test --locked", "toolu_1"));
    assert!(out.starts_with("Recorded a passing test run"), "{out}");
    let json = aggregates();
    assert_eq!(json["events"], 1, "{json}");
    let agg = &json["aggregates"][0];
    assert_eq!(agg["agent"], "claude-code", "{json}");
    assert_eq!(agg["model"], "claude-opus-5", "{json}");
    assert_eq!(agg["provider"], "anthropic", "{json}");
    assert_eq!(agg["tasks"], 1, "{json}");
    assert_eq!(agg["tokens"], 150 + 180, "{json}");
    assert_eq!(agg["test_passes"], 1, "{json}");
    assert_eq!(agg["quota_tasks"], 1, "{json}");
    assert_eq!(agg["cost_basis"], "quota", "{json}");
    assert_eq!(agg["cost"], serde_json::Value::Null, "{json}");
    for counter in ["retries", "escalations", "review_defects", "retry_rate"] {
        assert_eq!(agg[counter], serde_json::Value::Null, "{counter}: {json}");
    }

    // The same tool call delivered again is the same run.
    let out = hook(&payload("PostToolUse", "cargo test --locked", "toolu_1"));
    assert!(out.starts_with("Already recorded"), "{out}");
    assert_eq!(aggregates()["events"], 1);

    // One more request, then a failing run: the attempt is that request alone.
    let append = |text: String| {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap();
        file.write_all(text.as_bytes()).unwrap();
    };
    append(line(
        "2026-08-25T21:42:06.718Z",
        "req_3",
        "claude-opus-5",
        20,
    ));
    let out = hook(&payload(
        "PostToolUseFailure",
        "cargo test --locked",
        "toolu_2",
    ));
    assert!(out.starts_with("Recorded a failing test run"), "{out}");
    let json = aggregates();
    assert_eq!(json["events"], 2, "{json}");
    let agg = &json["aggregates"][0];
    assert_eq!(agg["tasks"], 2, "{json}");
    assert_eq!(agg["test_passes"], 1, "{json}");
    assert_eq!(agg["test_failures"], 1, "{json}");
    assert_eq!(
        agg["tokens"],
        330 + 120,
        "the second attempt is only req_3: {json}"
    );

    // A failure whose status is not the runner's is not a failure.
    let out = hook(&payload(
        "PostToolUseFailure",
        "cargo build && cargo test",
        "toolu_3",
    ));
    assert!(out.starts_with("Nothing to record"), "{out}");
    assert_eq!(aggregates()["events"], 2);

    // A run with nothing new in the transcript is still a run, attributed to nothing — and it
    // must not move the cursor, or the next request would never be counted.
    let out = hook(&payload("PostToolUse", "cargo test --locked", "toolu_4"));
    assert!(out.starts_with("Recorded a passing test run"), "{out}");
    append(line(
        "2026-08-25T21:42:08.116Z",
        "req_4",
        "claude-opus-5",
        30,
    ));
    let out = hook(&payload("PostToolUse", "cargo test --locked", "toolu_5"));
    assert!(out.starts_with("Recorded a passing test run"), "{out}");
    let json = aggregates();
    assert_eq!(json["events"], 4, "{json}");
    let agg = &json["aggregates"][0];
    assert_eq!(agg["tasks"], 4, "{json}");
    assert_eq!(
        agg["tokens"],
        450 + 130,
        "req_4 belongs to the last attempt; an empty attempt must not have consumed it: {json}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The update cache is what carries an opted-in check's answer to the dashboard, and `--doctor`
/// has to tell the truth about it even when the check itself is off — otherwise a user who
/// turned the check off and still sees a notice in the header has nowhere to find out why.
///
/// No network: the cache is planted, which is exactly the state an earlier opted-in run leaves.
#[test]
fn doctor_reports_a_cached_update_answer_it_did_not_fetch() {
    let dir = scratch("update-cache");
    let data_home = dir.join("data");
    std::fs::create_dir_all(data_home.join("ai-usage-tui")).expect("data home");
    let cache = data_home.join("ai-usage-tui").join("update-check.json");

    let doctor = |tag: &str| -> String {
        std::fs::write(
            &cache,
            format!("{{\"latest\":\"{tag}\",\"checked\":1700000000}}"),
        )
        .expect("plant the cache");
        let output = hermetic(bin().arg("--doctor"))
            // Overrides the pin `hermetic` sets: this test wants the cache read.
            .env("XDG_DATA_HOME", &data_home)
            .output()
            .expect("run --doctor");
        assert!(output.status.success());
        String::from_utf8(output.stdout).expect("utf8")
    };

    // A release this build has not caught up with: reported, and named as what the header shows.
    let text = doctor("v99.0.0");
    assert!(
        text.contains("not checked"),
        "the check should still be off:\n{text}"
    );
    // Both ways of asking are named, not just the config key: the explicit command is what a
    // scheduled check runs, and a user reading this line is the one deciding how to opt in.
    assert!(
        text.contains("--check-update"),
        "the hint does not name the explicit command:\n{text}"
    );
    assert!(
        text.contains("v99.0.0") && text.contains("earlier check"),
        "the cached answer is not reported:\n{text}"
    );
    assert!(
        text.contains("dashboard header shows it"),
        "--doctor does not say the header is showing it:\n{text}"
    );

    // One the build has passed: still disclosed, but not claimed to be on screen.
    let text = doctor("v0.0.1");
    assert!(text.contains("v0.0.1"), "{text}");
    assert!(
        !text.contains("dashboard header shows it"),
        "a superseded cache is claimed to be on screen:\n{text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// The limits panel and `--json` work on a machine with no Omarchy at all.
///
/// This is the gap the feature closes: `limits[]` came only from Omarchy's agents panel, so
/// everywhere else the `l` panel was permanently empty and `--json` carried nothing. Claude Code
/// caches its own subscription utilisation, so there is something to read on every platform.
///
/// The assertions double as the end-to-end privacy sweep. `~/.claude.json` carries an account
/// identifier, project history and sibling keys beside the ones this tool reads; none of it may
/// reach stdout. The fixture plants a credential and a placeholder account id precisely so this
/// can discriminate -- an implementation that dumped the block wholesale would fail here.
#[test]
fn json_limits_come_from_claude_codes_own_cache_without_omarchy() {
    let claude_dir = format!(
        "{}/tests/fixtures/claude_home/.claude/projects",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut command = bin();
    let output = hermetic(&mut command)
        .arg("--json")
        .arg("--claude-dir")
        .arg(&claude_dir)
        .output()
        .expect("run --json");
    assert!(output.status.success(), "--json exited {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is JSON");

    let limits = value["limits"].as_array().expect("limits is an array");
    let claude = limits
        .iter()
        .find(|entry| entry["agent"] == "claude")
        .unwrap_or_else(|| panic!("no claude limits row in {stdout}"));
    let windows = claude["windows"].as_array().expect("windows");
    let labels: Vec<&str> = windows
        .iter()
        .map(|w| w["label"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        labels,
        vec![
            "Session (5-hour)",
            "Weekly (all models)",
            "Weekly · Fixture Model 9"
        ],
        "{stdout}"
    );
    assert_eq!(windows[0]["percent_used"], 11.0, "{stdout}");
    assert_eq!(windows[2]["percent_used"], 88.0, "{stdout}");

    for forbidden in [
        "FIXTURE_SECRET",
        "00000000-0000-0000-0000-000000000000",
        "futureField",
        "a key this reader has never heard of",
        "extra_usage",
        "member_dashboard_available",
        "oauthAccount",
        "/home/fixture/work",
        "a_kind_this_reader_does_not_know",
    ] {
        assert!(
            !stdout.contains(forbidden),
            "{forbidden:?} reached stdout: {stdout}"
        );
    }
}

/// `--statusline` is the fourth stdin action: Claude Code's statusline payload in, one line out,
/// and the windows cached where `limits::load` -- so the `l` panel and `--json` -- will find them.
#[test]
fn a_statusline_payload_becomes_one_line_and_a_row_in_the_limits_export() {
    use std::io::Write;
    let dir = scratch("statusline");
    let data_home = dir.join("data");
    let cache = data_home
        .join("ai-usage-tui")
        .join("statusline-limits.json");

    // The fixture is a capture, so its reset instants are whatever they were on the day. They
    // are moved to an hour and a week from now so the windows are live against the real clock
    // this process reads; nothing else in the payload is touched.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;
    let mut payload: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(format!(
            "{}/tests/fixtures/claude_statusline.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture"),
    )
    .expect("the fixture is JSON");
    payload["rate_limits"]["five_hour"]["resets_at"] = serde_json::json!(now + 3600);
    payload["rate_limits"]["seven_day"]["resets_at"] = serde_json::json!(now + 7 * 86_400);
    // The percentages are whatever the capture said; the assertions below are about the shape of
    // the line and the row, so they take the figures from the fixture rather than repeating them.
    let five = payload["rate_limits"]["five_hour"]["used_percentage"]
        .as_f64()
        .expect("the capture carries a five_hour percentage");
    let seven = payload["rate_limits"]["seven_day"]["used_percentage"]
        .as_f64()
        .expect("the capture carries a seven_day percentage");

    let statusline = |body: &str| -> std::process::Output {
        let mut child = bin()
            .arg("--statusline")
            .env("XDG_DATA_HOME", &data_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
        child.wait_with_output().expect("wait")
    };
    let limits_json = || -> serde_json::Value {
        let output = hermetic(bin().arg("--json"))
            // Overrides the pin `hermetic` sets: this test wants the cache read.
            .env("XDG_DATA_HOME", &data_home)
            .output()
            .expect("run --json");
        assert!(output.status.success());
        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("--json prints JSON");
        json["limits"].clone()
    };

    let output = statusline(&payload.to_string());
    assert!(
        output.status.success(),
        "--statusline exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert_eq!(
        stdout.lines().count(),
        1,
        "one line, for the status bar: {stdout:?}"
    );
    assert!(
        stdout.starts_with(&format!("5h {}% (resets ", five.round())),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("7d {}% (resets ", seven.round())),
        "{stdout}"
    );
    assert!(cache.is_file(), "the windows are cached for the panel");
    let leftovers: Vec<String> = std::fs::read_dir(cache.parent().unwrap())
        .expect("read the data dir")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "temporary-then-rename leaves nothing behind: {leftovers:?}"
    );
    let cached = std::fs::read_to_string(&cache).expect("read cache");
    for forbidden in ["FIXTURE_SECRET", "total_cost_usd", "transcript_path"] {
        assert!(
            !stdout.contains(forbidden),
            "{forbidden} reached stdout: {stdout}"
        );
        assert!(
            !cached.contains(forbidden),
            "{forbidden} reached the cache: {cached}"
        );
    }

    // The panel's source, through the one function it and `--json` share.
    let limits = limits_json();
    let claude = limits
        .as_array()
        .expect("limits is an array")
        .iter()
        .find(|snapshot| snapshot["agent"] == "claude")
        .expect("the subscription is a row")
        .clone();
    assert_eq!(claude["stale"], false, "{claude}");
    let labels: Vec<&str> = claude["windows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|window| window["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["Session (5-hour)", "Weekly (all models)"]);
    let percent = claude["windows"][0]["percent_used"]
        .as_f64()
        .expect("percent_used is a number");
    assert!((percent - five).abs() < 1e-6, "{percent} vs {five}");

    // `--doctor` names the cache, so an empty panel is a question with an answer.
    let output = hermetic(bin().arg("--doctor"))
        .env("XDG_DATA_HOME", &data_home)
        .output()
        .expect("run --doctor");
    assert!(output.status.success());
    let doctor = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        doctor.contains("statusline") && doctor.contains("2 windows"),
        "{doctor}"
    );
    assert!(doctor.contains(&cache.display().to_string()), "{doctor}");

    // No block at all -- an API-billed account, or the moment before the first response. That is
    // "no such thing", not 0%: nothing printed, exit 0, and the cache is left as it was.
    let output = statusline(r#"{"model":{"id":"x"},"cwd":"/tmp"}"#);
    assert!(output.status.success());
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert_eq!(std::fs::read_to_string(&cache).expect("read"), cached);

    // A window gone from the payload is gone from the panel, not frozen at its last figure.
    payload["rate_limits"]
        .as_object_mut()
        .unwrap()
        .remove("seven_day");
    let output = statusline(&payload.to_string());
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        stdout.starts_with(&format!("5h {}%", five.round())) && !stdout.contains("7d"),
        "{stdout}"
    );
    let limits = limits_json();
    assert_eq!(
        limits[0]["windows"].as_array().map(Vec::len),
        Some(1),
        "{limits}"
    );

    // A cache that cannot be written is not an exit code. Claude Code shows stdout only from a
    // command that exited 0, so a non-zero exit here would blank the status line for a failure
    // that has nothing to do with the readout; the failure is said on stderr instead.
    let not_a_dir = dir.join("not-a-dir");
    std::fs::write(&not_a_dir, "a file where the data root should be").expect("plant a file");
    let mut child = bin()
        .arg("--statusline")
        .env("XDG_DATA_HOME", &not_a_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "an unwritable cache must not blank the status line: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).starts_with("5h "),
        "the readout is still the product"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not cached"),
        "the failure is said: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Not the document at all: a settings entry pointing at the wrong command must not look like
    // an API-billed account.
    let output = statusline("not json");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--statusline expects"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `--doctor` names an unreadable statusline cache on its own row. The LIMITS problems added
/// beneath it come from `limits::load`, which reads that cache again -- and printed the same parse
/// error a second time, on an unlabelled row. Caught in review of #102.
#[test]
fn doctor_reports_an_unreadable_statusline_cache_once() {
    let dir = scratch("doctor-statusline-once");
    let data = dir.join("data");
    std::fs::create_dir_all(data.join("ai-usage-tui")).expect("data dir");
    std::fs::write(
        data.join("ai-usage-tui").join("statusline-limits.json"),
        "{ not json",
    )
    .expect("write");

    let output = hermetic(bin().arg("--doctor"))
        .env("XDG_DATA_HOME", &data)
        .output()
        .expect("run --doctor");
    let text = String::from_utf8(output.stdout).expect("utf8");
    let problem = text
        .lines()
        .find_map(|line| line.trim_start().strip_prefix("statusline"))
        .and_then(|rest| rest.trim_start().strip_prefix("unreadable"))
        .map(str::trim)
        .unwrap_or_else(|| {
            panic!("the statusline row does not report the cache as unreadable:\n{text}")
        });
    assert_eq!(
        text.matches(problem).count(),
        1,
        "the parse error is printed more than once:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--doctor` told a user without a config to "copy examples/config.toml there" -- a file no binary
/// install channel ships. The example is in the binary now, and the hint names the command.
#[test]
fn the_example_config_ships_in_the_binary_and_doctor_points_at_it() {
    let output = bin().arg("--print-config").output().expect("run");
    assert!(output.status.success());
    let example = std::fs::read_to_string(format!(
        "{}/examples/config.toml",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("read example");
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), example);

    // It works before the config is read: a user asking for an example is often one whose own
    // config does not parse.
    let dir = scratch("print-config-broken");
    let broken = dir.join("config.toml");
    std::fs::write(&broken, "this is not = [toml").expect("write");
    let output = bin()
        .args(["--print-config", "--config"])
        .arg(&broken)
        .output()
        .expect("run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let doctor = hermetic(bin().arg("--doctor"))
        .output()
        .expect("run --doctor");
    let text = String::from_utf8(doctor.stdout).expect("utf8");
    assert!(text.contains("--print-config"), "{text}");
    assert!(!text.contains("copy examples/config.toml"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every JSON document carries the contract version `docs/stability.md` promises, so a consumer can
/// check what it is reading instead of discovering a change from a missing key.
#[test]
fn every_json_document_carries_its_schema_version() {
    let dir = scratch("schema-version");
    let journal = dir.join("usage.db");
    for flag in [
        "--summary-json",
        "--json",
        "--routing-json",
        "--check-budgets",
    ] {
        let output = hermetic_with(bin().arg(flag), &PathBuf::from(fixture_db()), &journal)
            .output()
            .expect("run");
        assert!(
            output.status.success(),
            "{flag}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json parses");
        assert_eq!(json["schema_version"], 1, "{flag}: {json}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Claude Code home with two projects and two sessions, for the drill-down filters.
fn two_project_claude_home(root: &std::path::Path) -> std::path::PathBuf {
    let projects = root.join(".claude").join("projects");
    for (dir, cwd, session, model, input) in [
        (
            "api",
            "/w/api",
            "s-api",
            "claude-sonnet-4-5-20250929",
            1000u64,
        ),
        ("web", "/w/web", "s-web", "claude-opus-4-1-20250805", 3000),
    ] {
        let project = projects.join(dir);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("s.jsonl"),
            format!(
                "{{\"type\":\"assistant\",\"uuid\":\"u-{dir}\",\"requestId\":\"req_{dir}\",\
                 \"timestamp\":\"2026-08-18T10:00:00Z\",\"sessionId\":\"{session}\",\"cwd\":\"{cwd}\",\
                 \"message\":{{\"id\":\"msg_{dir}\",\"role\":\"assistant\",\"model\":\"{model}\",\
                 \"usage\":{{\"input_tokens\":{input},\"output_tokens\":100,\
                 \"cache_read_input_tokens\":9000,\"cache_creation_input_tokens\":0}}}}}}\n"
            ),
        )
        .unwrap();
    }
    projects
}

/// The binary against a planted Claude Code home and nothing else.
fn with_claude_home(projects: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut command = bin();
    command
        .env("XDG_CONFIG_HOME", "/nonexistent/config-home")
        .env("XDG_DATA_HOME", "/nonexistent/data-home")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_PROJECTS_DIR")
        .args(args)
        .arg("--all")
        .arg("--claude-dir")
        .arg(projects)
        .args(["--claude-billing", "api"])
        .args(["--db", "/nonexistent/opencode.db"])
        .args(["--codex-dir", "/nonexistent"])
        .args(["--copilot-dir", "/nonexistent"])
        .args(["--gemini-dir", "/nonexistent"])
        .args(["--omarchy-dir", "/nonexistent"])
        .args(["--journal", "/nonexistent/journal.db"]);
    let output = command.output().expect("run");
    assert!(
        output.status.success(),
        "{args:?} exited {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// `--json` prints one object per request -- 13 MB on the machine this was written on. The
/// summary is the same run as one compact line a reader with a context window can take whole.
#[test]
fn the_summary_is_one_compact_document_that_adds_up() {
    let dir = scratch("summary-json");
    let projects = two_project_claude_home(&dir);
    let output = with_claude_home(&projects, &["--summary-json"]);
    let text = String::from_utf8(output.stdout).expect("utf8");
    assert_eq!(
        text.trim_end().lines().count(),
        1,
        "compact, not pretty-printed"
    );
    let doc: serde_json::Value = serde_json::from_str(&text).expect("json parses");

    assert_eq!(doc["schema_version"], 1);
    assert_eq!(doc["range"]["label"], "ALL TIME");
    assert!(doc["range"]["since"].is_null(), "all history has no start");
    assert_eq!(doc["totals"]["requests"], 2);
    assert_eq!(doc["totals"]["tokens"], 1000 + 3000 + 200 + 18_000);
    // 18,000 cache reads over 22,000 prompt tokens.
    assert_eq!(doc["totals"]["metrics"]["cache_hit_pct"], 81.82);
    assert!(
        doc["totals"]["metrics"]["reasoning_pct"].is_null(),
        "Claude Code reports no reasoning split, which is not 0% reasoning"
    );

    let by_project = doc["by_project"]["rows"].as_array().expect("rows");
    assert_eq!(by_project.len(), 2);
    assert_eq!(by_project[0]["project"], "/w/web", "largest first");
    assert_eq!(doc["by_session"]["rows"][0]["session_id"], "s-web");
    assert_eq!(doc["by_model"]["total"], 2);
    let rate = |model: &str| {
        doc["by_model"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["model"] == model)
            .and_then(|row| row["list_input_rate"].as_f64())
            .unwrap_or_else(|| panic!("no list rate for {model}"))
    };
    assert!(
        rate("claude-opus-4-1-20250805") > rate("claude-sonnet-4-5-20250929"),
        "the list rate is what says which model is the expensive one"
    );
    // What `--doctor` alone said: which sources were read, and how billing was decided.
    let claude = doc["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .find(|s| s["id"] == "claude_code")
        .expect("claude_code source");
    assert_eq!(claude["rows"], 2);
    assert!(
        claude["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("billing"),
        "{claude}"
    );
    for key in [
        "by_category",
        "by_day",
        "burn",
        "budgets",
        "limits",
        "escalations",
        "provenance",
        "routing",
        "pricing",
    ] {
        assert!(!doc[key].is_null(), "the summary has no {key}");
    }

    // `--top 1` folds the smaller project away and says so; the rows still add up.
    let top = with_claude_home(&projects, &["--summary-json", "--top", "1"]);
    let top: serde_json::Value = serde_json::from_slice(&top.stdout).expect("json parses");
    assert_eq!(top["by_project"]["shown"], 1);
    assert_eq!(top["by_project"]["total"], 2);
    assert_eq!(
        top["by_project"]["rows"][0]["tokens"].as_u64().unwrap()
            + top["by_project"]["other"]["tokens"].as_u64().unwrap(),
        top["totals"]["tokens"].as_u64().unwrap()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The summary names projects and sessions; these are how a reader asks about one of them
/// without pulling every row.
#[test]
fn project_and_session_filters_narrow_every_export() {
    let dir = scratch("drill-down");
    let projects = two_project_claude_home(&dir);

    let rows = |args: &[&str]| -> Vec<serde_json::Value> {
        let output = with_claude_home(&projects, args);
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
        json["usage"].as_array().cloned().unwrap_or_default()
    };
    assert_eq!(rows(&["--json"]).len(), 2);
    let api = rows(&["--json", "--project", "/w/api"]);
    assert_eq!(api.len(), 1);
    assert_eq!(api[0]["session_id"], "s-api");
    // `billing` was in the model and in `--doctor`'s text, and in no export.
    assert_eq!(api[0]["billing"], "per_token");
    assert_eq!(
        rows(&["--json", "--project", "/w/api/"]).len(),
        1,
        "a trailing slash is forgiven"
    );
    assert_eq!(
        rows(&["--json", "--session", "s-web"])[0]["project"],
        "/w/web"
    );
    assert!(rows(&["--json", "--project", "/w/nothing"]).is_empty());

    let summary = with_claude_home(&projects, &["--summary-json", "--session", "s-web"]);
    let summary: serde_json::Value = serde_json::from_slice(&summary.stdout).expect("json");
    assert_eq!(summary["totals"]["requests"], 1);
    assert_eq!(summary["filters"]["session"], "s-web");
    assert_eq!(
        summary["by_model"]["rows"][0]["model"],
        "claude-opus-4-1-20250805"
    );

    // CSV is the compact row format, and `-` finally lets it into a pipeline -- with nothing
    // but the table on stdout.
    let csv = with_claude_home(&projects, &["--csv", "-", "--project", "/w/web"]);
    let csv = String::from_utf8(csv.stdout).expect("utf8");
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "a header and one row, no confirmation line:\n{csv}"
    );
    assert!(lines[0].starts_with("provider,model,"), "{csv}");
    assert!(lines[1].contains("/w/web"), "{csv}");
    assert!(!dir.join("-").exists() && !std::path::Path::new("-").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--routing-json` has always meant all history. A range flag now narrows it -- but only one
/// the caller gave, because the default range everywhere else is a week and applying that
/// unasked would have shrunk every existing script's output.
#[test]
fn routing_json_is_narrowed_by_a_range_flag_only_when_one_is_given() {
    let dir = scratch("routing-range");
    let journal = dir.join("usage.db");
    let record = |created: i64, task: &str| {
        use std::io::Write;
        let mut child = bin()
            .arg("--record-routing")
            .arg("--journal")
            .arg(&journal)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .expect("spawn");
        let event = format!(
            "{{\"agent\":\"a\",\"model\":\"m\",\"provider\":\"p\",\"task\":\"{task}\",\"tokens\":10,\"test_result\":true,\"created\":{created}}}"
        );
        child
            .stdin
            .take()
            .unwrap()
            .write_all(event.as_bytes())
            .unwrap();
        assert!(child.wait().expect("wait").success());
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    record(now - 60, "recent");
    record(now - 90 * 86_400, "old");

    let events = |args: &[&str]| -> serde_json::Value {
        // Not `hermetic_with`: it appends `--all`, and the last range flag wins. This export
        // reads the journal and nothing else, so the journal is all there is to pin.
        let output = bin()
            .env("XDG_CONFIG_HOME", "/nonexistent/config-home")
            .env("XDG_DATA_HOME", "/nonexistent/data-home")
            .arg("--routing-json")
            .args(args)
            .arg("--journal")
            .arg(&journal)
            .output()
            .expect("run");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("json")
    };
    assert_eq!(
        events(&[])["events"],
        2,
        "no range flag: all history, as always"
    );
    assert_eq!(events(&["--week"])["events"], 1);
    assert_eq!(events(&["--all"])["events"], 2);
    // The rate the panel has always shown, no longer left for the reader to divide.
    assert_eq!(events(&[])["aggregates"][0]["success_rate"], 100.0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every key, every enum value and every `null` in every JSON document is in `--schema`.
///
/// The glossary is data, so it can drift from the code that prints the JSON. This runs each
/// document against as much fixture data as there is -- four sources, a routing event, a budget
/// past its limit, Claude Code's cached limits -- and fails on anything the glossary does not
/// describe, naming the path to add.
#[test]
fn every_json_document_is_fully_described_by_the_schema() {
    let dir = scratch("schema-drift");
    let journal = dir.join("usage.db");
    let fixtures = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let config = dir.join("config.toml");
    std::fs::write(
        &config,
        "[budgets]\n[[budgets.entry]]\nscope = \"global\"\nperiod = \"monthly\"\nlimit = 0.0001\n\
         [[budgets.entry]]\nscope = \"provider\"\nname = \"nobody\"\nperiod = \"daily\"\nlimit = 5.0\n",
    )
    .expect("write config");

    use std::io::Write;
    let mut child = bin()
        .args(["--record-routing", "--journal"])
        .arg(&journal)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"agent":"a","model":"m","provider":"p","task":"t","tokens":10,"cost":0.5,"cost_status":"reported","retries":1,"test_result":true}"#)
        .unwrap();
    assert!(child.wait().expect("wait").success());
    // And usage through `--record-event`, so a row with a journaled project, session and
    // subscription billing is among what the glossary has to describe.
    record(
        &journal,
        &format!("{fixtures}/usage_events.ndjson"),
        "--record-event",
    );

    // The committed fixtures are years old, so nothing in them is inside a budget's period. One
    // request dated now, priced per token, is what puts the global budget over its limit.
    let recent = recent_claude_home(&dir);
    let fixture_claude = PathBuf::from(format!("{fixtures}/claude_home/.claude/projects"));

    let run_with = |flag: &str, claude_dir: &Path| -> serde_json::Value {
        let output = bin()
            .env("XDG_CONFIG_HOME", "/nonexistent/config-home")
            .env("XDG_DATA_HOME", "/nonexistent/data-home")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CLAUDE_PROJECTS_DIR")
            .arg(flag)
            .arg("--all")
            .arg("--config")
            .arg(&config)
            .arg("--db")
            .arg(format!("{fixtures}/opencode_test.db"))
            .arg("--claude-dir")
            .arg(claude_dir)
            .args(["--claude-billing", "api"])
            .arg("--codex-dir")
            .arg(format!("{fixtures}/codex_home"))
            .arg("--copilot-dir")
            .arg(format!("{fixtures}/copilot_home"))
            .args(["--gemini-dir", "/nonexistent", "--omarchy-dir"])
            .arg(format!("{fixtures}/omarchy"))
            .arg("--journal")
            .arg(&journal)
            .output()
            .expect("run");
        // `--check-budgets` exits non-zero by design when a budget is over; the document is what
        // is under test.
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("{flag}: {e}\n{}", String::from_utf8_lossy(&output.stderr)))
    };

    let run = |flag: &str| run_with(flag, &fixture_claude);
    // A session that moves to a pricier model, so `escalations.transitions` is not empty.
    let escalating = escalating_claude_home(&dir.join("escalating"));
    assert!(
        !run_with("--summary-json", &escalating)["escalations"]["transitions"]
            .as_array()
            .unwrap()
            .is_empty(),
        "no escalation in the fixture run"
    );

    for (flag, document) in [
        ("--summary-json", run("--summary-json")),
        ("--summary-json", run_with("--summary-json", &recent)),
        ("--summary-json", run_with("--summary-json", &escalating)),
        ("--json", run("--json")),
        ("--json", run_with("--json", &escalating)),
        ("--routing-json", run("--routing-json")),
        ("--check-budgets", run_with("--check-budgets", &recent)),
    ] {
        let problems = ai_usage_tui::schema::unknown(flag, &document);
        assert!(
            problems.is_empty(),
            "{flag} prints what docs/json-glossary.json does not describe:\n  {}",
            problems.join("\n  ")
        );
    }

    // And the fixtures really did exercise the optional parts, or the check above proves little.
    let summary = run("--summary-json");
    assert!(
        summary["budgets"].as_array().is_some_and(|b| b.len() == 2),
        "{}",
        summary["budgets"]
    );
    assert!(
        summary["limits"].as_array().is_some_and(|l| !l.is_empty()),
        "no limits in the fixture run"
    );
    assert_eq!(summary["routing"]["events"], 1);
    assert!(summary["by_model"]["rows"]
        .as_array()
        .is_some_and(|r| r.len() > 3));
    assert!(
        !run_with("--check-budgets", &recent)["alerts"]
            .as_array()
            .unwrap()
            .is_empty(),
        "no budget alert in the fixture run"
    );

    // `--schema` is the same glossary, compact, and needs no config or sources.
    let schema = bin().arg("--schema").output().expect("run --schema");
    assert!(schema.status.success());
    let printed: serde_json::Value = serde_json::from_slice(&schema.stdout).expect("json");
    let embedded: serde_json::Value =
        serde_json::from_str(ai_usage_tui::schema::GLOSSARY).expect("json");
    assert_eq!(printed, embedded);
    let guide = bin()
        .arg("--agent-guide")
        .output()
        .expect("run --agent-guide");
    assert_eq!(
        String::from_utf8_lossy(&guide.stdout),
        ai_usage_tui::schema::AGENT_GUIDE
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The model row the README shows under "Ask an LLM about your usage" is real output.
///
/// It was pasted from a run against the committed fixtures, and this holds it there: a key added
/// to the bucket, or a figure computed differently, fails here until the README says so too.
#[test]
fn the_readmes_summary_sample_is_what_the_binary_prints() {
    // Line endings normalised: a Windows checkout has CRLF, and the fence is found by its newline.
    let readme = std::fs::read_to_string(format!("{}/README.md", env!("CARGO_MANIFEST_DIR")))
        .expect("read README")
        .replace("\r\n", "\n");
    let start = readme
        .find("```json\n{ \"provider\": \"opencode\"")
        .expect("the README's summary sample")
        + "```json\n".len();
    let end = start
        + readme[start..]
            .find("\n```")
            .expect("the sample's closing fence");
    let sample: serde_json::Value =
        serde_json::from_str(&readme[start..end]).expect("the sample is JSON");

    let fixtures = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let output = bin()
        .env("XDG_CONFIG_HOME", "/nonexistent/config-home")
        .env("XDG_DATA_HOME", "/nonexistent/data-home")
        // `config_json_path` checks `CLAUDE_CONFIG_DIR` before the `--claude-dir` override, so
        // with it exported this would read the developer's real `~/.claude.json`.
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDE_PROJECTS_DIR")
        .args(["--summary-json", "--all", "--top", "0", "--db"])
        .arg(format!("{fixtures}/opencode_test.db"))
        // The three committed fixtures the sample was taken from: `share_of_tokens_pct` is a share
        // of all of them.
        .arg("--codex-dir")
        .arg(format!("{fixtures}/codex_home"))
        .arg("--copilot-dir")
        .arg(format!("{fixtures}/copilot_home"))
        .args(["--claude-dir", "/nonexistent"])
        .args(["--gemini-dir", "/nonexistent"])
        .args(["--omarchy-dir", "/nonexistent"])
        .args(["--journal", "/nonexistent/journal.db"])
        .output()
        .expect("run");
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    let row = doc["by_model"]["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| row["model"] == sample["model"] && row["provider"] == sample["provider"])
        .expect("the sampled model is in the fixture");
    assert_eq!(
        &sample, row,
        "README.md shows a row the binary does not print"
    );
}
