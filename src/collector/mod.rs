pub mod background;
pub mod billing;
pub mod claude_code;
pub mod codex;
pub mod copilot;
pub mod gemini;
pub mod journal;
#[cfg(test)]
mod mutation;
pub mod opencode;
pub mod pricing_refresh;
pub mod registry;
pub mod skipped;
pub mod zen;

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::cli::Cli;
use crate::collector::billing::{detect, BillingSetting, Decision, Signals};
use crate::collector::claude_code::config_json_path;
use crate::model::Usage;
use crate::pricing::{apply_estimated_pricing, PricingEngine};

/// Where every source lives, and how its usage is billed.
///
/// Three functions took the same three paths as separate arguments and each grew a fourth
/// when billing arrived. One struct, extended in one place, threaded everywhere.
#[derive(Clone, Debug)]
pub struct SourceRoots {
    pub db_path: Option<PathBuf>,
    pub journal: PathBuf,
    /// Root of Claude Code's session logs; `None` means the default (see `claude_code`).
    pub claude_dir: Option<PathBuf>,
    pub claude_billing: BillingSetting,
    /// Claude Code's `~/.claude.json`, when the default location is not the right one.
    pub claude_json: Option<PathBuf>,
    /// Codex's home; `None` means `$CODEX_HOME` or `~/.codex` (see `codex`).
    pub codex_dir: Option<PathBuf>,
    pub codex_billing: BillingSetting,
    /// Copilot's home; `None` means `$COPILOT_HOME` or `~/.copilot` (see `copilot`).
    pub copilot_dir: Option<PathBuf>,
    pub copilot_billing: BillingSetting,
    /// Gemini CLI's home; `None` means `~/.gemini` (see `gemini`).
    pub gemini_dir: Option<PathBuf>,
    pub gemini_billing: BillingSetting,
    /// Omarchy's agents-panel records; `None` means the XDG state location (see `omarchy`).
    pub omarchy_dir: Option<PathBuf>,
    /// Whether to read those records at all. On by default: an absent directory is idle.
    pub limits_enabled: bool,
    /// `[collectors.<id>] enabled` overrides, by source id. Absent means the registry default.
    pub source_enabled: std::collections::BTreeMap<String, bool>,
}

impl Default for SourceRoots {
    fn default() -> Self {
        Self {
            db_path: None,
            journal: PathBuf::new(),
            claude_dir: None,
            claude_billing: BillingSetting::Auto,
            claude_json: None,
            codex_dir: None,
            codex_billing: BillingSetting::Auto,
            copilot_dir: None,
            copilot_billing: BillingSetting::Auto,
            gemini_dir: None,
            gemini_billing: BillingSetting::Auto,
            omarchy_dir: None,
            limits_enabled: true,
            source_enabled: Default::default(),
        }
    }
}

impl SourceRoots {
    pub fn new(journal: PathBuf) -> Self {
        Self {
            journal,
            ..Default::default()
        }
    }

    /// The Omarchy records directory in force: explicit, else the XDG state location.
    pub fn omarchy_usage_dir(&self) -> Option<PathBuf> {
        self.omarchy_dir
            .clone()
            .or_else(crate::utils::omarchy_usage_dir)
    }

    /// Whether a source is collected at all, from `[collectors.<id>] enabled`.
    pub fn is_enabled(&self, spec: &registry::SourceSpec) -> bool {
        self.source_enabled
            .get(spec.id)
            .copied()
            .unwrap_or(spec.default_enabled)
    }

    /// Omarchy's records directory *as a billing signal*: the directory in force, unless the
    /// reader is switched off. `main.rs` and every collector construction used to spell this
    /// `.omarchy_usage_dir().filter(|_| cli.limits_enabled)` separately.
    pub fn omarchy_signal_dir(&self) -> Option<PathBuf> {
        self.omarchy_usage_dir().filter(|_| self.limits_enabled)
    }

    /// The plan label Omarchy already derived for an agent, when its record is here.
    pub fn omarchy_tier(&self, agent: &str) -> Option<String> {
        if !self.limits_enabled {
            return None;
        }
        let dir = self.omarchy_usage_dir()?;
        crate::omarchy::tier_label_for(&dir, agent)
    }

    pub fn from_cli(cli: &Cli, journal: PathBuf) -> Self {
        Self {
            db_path: cli.db_path.clone(),
            journal,
            claude_dir: cli.claude_dir.clone(),
            claude_billing: cli.claude_billing,
            claude_json: cli.claude_json.clone(),
            codex_dir: cli.codex_dir.clone(),
            codex_billing: cli.codex_billing,
            copilot_dir: cli.copilot_dir.clone(),
            copilot_billing: cli.copilot_billing,
            gemini_dir: cli.gemini_dir.clone(),
            gemini_billing: cli.gemini_billing,
            omarchy_dir: cli.omarchy_dir.clone(),
            limits_enabled: cli.limits_enabled,
            source_enabled: cli.source_enabled.clone(),
        }
    }

    /// Every root pinned at a path that does not exist, for a test that must read nothing.
    ///
    /// Spelled as a whole struct literal, with no `..Default::default()`, on purpose: a source
    /// added later becomes a compile error *here* rather than a `None` that quietly resolves to
    /// the developer's own home directory -- which is how the registry's reachability test came
    /// to read a real `~/.copilot` and `~/.gemini`.
    #[cfg(test)]
    pub(crate) fn nowhere() -> Self {
        let at = |name: &str| Some(PathBuf::from("/nonexistent").join(name));
        Self {
            db_path: at("opencode.db"),
            journal: PathBuf::from("/nonexistent/journal.db"),
            claude_dir: at("claude"),
            claude_billing: BillingSetting::Auto,
            claude_json: at("claude.json"),
            codex_dir: at("codex"),
            codex_billing: BillingSetting::Auto,
            copilot_dir: at("copilot"),
            copilot_billing: BillingSetting::Auto,
            gemini_dir: at("gemini"),
            gemini_billing: BillingSetting::Auto,
            omarchy_dir: at("omarchy"),
            limits_enabled: true,
            source_enabled: Default::default(),
        }
    }

    /// Where Claude Code's config document is for these roots. Derived from an overridden
    /// session-log root, so a test that points at a fixture never resolves the developer's own.
    pub fn claude_json_path(&self) -> Option<PathBuf> {
        config_json_path(self.claude_json.as_deref(), self.claude_dir.as_deref())
    }

    /// Codex's home in force: `--codex-dir`, else `$CODEX_HOME`, else `~/.codex`. The collector
    /// and the limits reader both resolve it here, so they cannot look in different places.
    pub fn codex_home(&self) -> Option<PathBuf> {
        self.codex_dir.clone().or_else(codex::codex_home)
    }

    /// Decide Codex's billing. Codex has no config document this tool will read — its
    /// `auth.json` is a credential file — so the signals are the setting and the environment.
    pub fn codex_decision(&self) -> Decision {
        let tier = self.omarchy_tier("codex");
        detect(
            "codex",
            self.codex_billing,
            &Signals {
                claude_json: None,
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: tier.as_deref(),
            },
        )
    }

    /// Decide Copilot's billing.
    ///
    /// Copilot has no API-key mode: a Pro, Business or Enterprise seat is the only way to use
    /// it, and it bills premium requests against that seat rather than tokens. `detect`'s
    /// unevidenced fallthrough is `PerToken`, which here would let `apply_estimated_pricing`
    /// put list-rate dollars into budgets for money that was never charged — so an unevidenced
    /// decision resolves to `Subscription` instead. An explicit `billing = "api"` is still
    /// honoured, for whoever is on an arrangement this does not know about.
    pub fn copilot_decision(&self) -> Decision {
        let tier = self.omarchy_tier(crate::collector::copilot::ID);
        let decided = detect(
            crate::collector::copilot::ID,
            self.copilot_billing,
            &Signals {
                claude_json: None,
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: tier.as_deref(),
            },
        );
        if decided.is_evidenced() {
            return decided;
        }
        Decision {
            billing: crate::model::Billing::Subscription,
            tier: None,
            reason: "copilot seat",
        }
    }

    /// Decide Gemini CLI's billing.
    ///
    /// The telemetry record carries `auth_type` (`oauth-personal`, `gemini-api-key`, …), which is
    /// a better signal than anything here — but it is per record, and the billing decision is
    /// per source, so the environment decides and a record cannot flip rows already merged.
    pub fn gemini_decision(&self) -> Decision {
        let tier = self.omarchy_tier(crate::collector::gemini::ID);
        detect(
            crate::collector::gemini::ID,
            self.gemini_billing,
            &Signals {
                claude_json: None,
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: tier.as_deref(),
            },
        )
    }

    /// Decide Claude Code's billing from the evidence available right now.
    pub fn claude_decision(&self) -> Decision {
        let path = self.claude_json_path();
        let tier = self.omarchy_tier("claude_code");
        detect(
            "claude_code",
            self.claude_billing,
            &Signals {
                claude_json: path.as_deref(),
                env_has: &crate::collector::billing::env_has,
                omarchy_tier: tier.as_deref(),
            },
        )
    }
}

/// One source's contribution to a collection pass: what it found, and where.
///
/// `load_usage` merges these into one list and one status line; `diagnose` prints them one per
/// row. Both walk the same traversal, so the dashboard and `--doctor` can never disagree about
/// which sources exist or where each was looked for.
#[derive(Clone, Debug)]
pub struct SourceReport {
    /// Stable id, matching the `[collectors.<id>]` config table where there is one.
    pub id: &'static str,
    /// Where this source was read from, when it resolves to a single path.
    pub path: Option<PathBuf>,
    /// The one-line status this source already reports for the dashboard header.
    pub status: String,
    /// Rows this source produced, before cross-source deduplication.
    pub rows: usize,
    /// Whether anything was there to read at all. A source that is simply absent is a normal
    /// state on most machines, not a fault.
    pub present: bool,
    /// One line `--doctor` prints under the source: the billing decision, or the rows the read
    /// had to skip and the first reason.
    pub detail: Option<String>,
}

/// Read every source once, keeping each one's rows and report separate.
///
/// One traversal of [`registry::SOURCES`], so the dashboard, the exporters and `--doctor` can
/// never disagree about which sources exist or where each was looked for.
fn collect_sources(roots: &SourceRoots) -> Result<Vec<(SourceReport, Vec<Usage>)>> {
    registry::SOURCES
        .iter()
        .map(|spec| {
            // Only a row-producing source can be switched off here; see `contributes_rows`.
            if spec.contributes_rows && !roots.is_enabled(spec) {
                // Reported rather than skipped: "disabled" is the answer `--doctor` should give
                // when a source is configured off, and the header should not silently lose it.
                return Ok((
                    SourceReport {
                        id: spec.id,
                        path: None,
                        status: format!("{}: disabled", spec.id),
                        rows: 0,
                        present: false,
                        detail: Some(format!(
                            "switched off by [collectors.{}] enabled = false",
                            spec.id
                        )),
                    },
                    Vec::new(),
                ));
            }
            (spec.load)(roots)
        })
        .collect()
}

/// Per-source report without merging, for `--doctor`.
pub fn diagnose(roots: &SourceRoots) -> Result<Vec<SourceReport>> {
    Ok(diagnose_with_usage(roots)?.0)
}

/// `diagnose`, keeping the merged rows as well.
///
/// `--doctor` reports where each number came from, which needs the deduplicated and priced rows
/// and not just the per-source counts. Going through one traversal rather than calling
/// `load_usage` beside `diagnose` matters on a machine with a hundred megabytes of transcripts:
/// the second read cost as much as the first and could disagree with it.
pub fn diagnose_with_usage(roots: &SourceRoots) -> Result<(Vec<SourceReport>, Vec<Usage>)> {
    let sources = collect_sources(roots)?;
    let reports = sources
        .iter()
        .map(|(report, _)| report.clone())
        .collect::<Vec<_>>();
    Ok((reports, merge(sources)))
}

/// One-shot read of every source.
///
/// Production passes the roots resolved from the CLI and config; tests pass explicit paths so
/// they never read the developer's real transcripts or config.
pub fn load_usage(roots: &SourceRoots) -> Result<(Vec<Usage>, String)> {
    let sources = collect_sources(roots)?;
    let status = sources
        .iter()
        .map(|(report, _)| report.status.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    Ok((merge(sources), status))
}

/// Deduplicate across sources in registry order, then price what is still unpriced.
fn merge(sources: Vec<(SourceReport, Vec<Usage>)>) -> Vec<Usage> {
    // Deduplication is *cross-source* only. OpenCode is the base list and every one of its rows
    // is kept: an OpenCode message without an `id` falls back to the shape key, and an agent
    // loop routinely emits distinct requests with byte-identical counts in the same second --
    // deduplicating those against each other under-reports real spend (CONTRIBUTING invariant
    // 3). Later sources are matched against what is already there, in source order.
    let mut sources = sources.into_iter();
    let mut usages: Vec<Usage> = sources.next().map(|(_, rows)| rows).unwrap_or_default();
    let mut seen: HashMap<UsageKey, usize> = usages
        .iter()
        .enumerate()
        .map(|(index, usage)| (usage_key(usage), index))
        .collect();
    for (_, rows) in sources {
        for usage in rows {
            match seen.entry(usage_key(&usage)) {
                Entry::Vacant(slot) => {
                    slot.insert(usages.len());
                    usages.push(usage);
                }
                Entry::Occupied(slot) => {
                    let held = &mut usages[*slot.get()];
                    if supersedes(&usage, held) {
                        *held = usage;
                    }
                }
            }
        }
    }

    let engine = PricingEngine::load();
    apply_estimated_pricing(&mut usages, &engine);

    usages
}

/// Identity of a usage event for deduplication.
///
/// Token counts alone are not an identity: agent loops routinely emit many requests with
/// byte-identical counts, and keying on shape alone silently discards them, under-reporting
/// real spend. Prefer the source's own id; fall back to shape *plus* timestamp.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UsageKey {
    Event(String),
    Shape {
        provider: String,
        model: String,
        input: u64,
        output: u64,
        reasoning: u64,
        cache_read: u64,
        cache_write: u64,
        created: i64,
    },
}

/// Whether a second reading of one event should replace the one already held.
///
/// A source can write one request more than once while it is still arriving. Claude Code does:
/// an assistant message goes to the transcript a line per content block, every line under the
/// same `requestId`, and in a subagent's transcript the earlier lines carry the count as it
/// stood when they were written -- `output_tokens: 3`, then `190` on the line that closes the
/// message (`tests/fixtures/claude_capture`). Keeping the first line seen, which is what
/// deduplication did, kept the placeholder: measured on the machine this was found on, 916 of
/// 16,523 requests, every one in a subagent, and 7.5% of all output tokens never counted.
///
/// So the reading with more tokens wins. Counts only grow while a response streams, which makes
/// "more" the same as "later" without depending on the order files or polls are read in -- and a
/// forked or resumed transcript replays old lines after new ones. Rows that are equal, which is
/// every other source's duplicates, leave the first in place as before.
pub fn supersedes(incoming: &Usage, held: &Usage) -> bool {
    let tokens = |u: &Usage| {
        u.input
            .saturating_add(u.output)
            .saturating_add(u.reasoning)
            .saturating_add(u.cache_read)
            .saturating_add(u.cache_write)
    };
    tokens(incoming) > tokens(held)
}

pub fn usage_key(usage: &Usage) -> UsageKey {
    match &usage.event_id {
        Some(id) if !id.is_empty() => UsageKey::Event(id.clone()),
        _ => UsageKey::Shape {
            provider: usage.provider.clone(),
            model: usage.model.clone(),
            input: usage.input,
            output: usage.output,
            reasoning: usage.reasoning,
            cache_read: usage.cache_read,
            cache_write: usage.cache_write,
            created: usage.created,
        },
    }
}

/// Path to the committed OpenCode fixture database.
///
/// Anchored to the manifest directory: a relative path silently resolves against whatever
/// working directory the test runner happens to use.
#[cfg(test)]
pub fn setup_test_db() -> std::path::PathBuf {
    std::path::PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/opencode_test.db"
    ))
}

/// Build a journal database with known contents inside `dir`.
///
/// The previous fixture was a checked-in binary that `.gitignore` excluded, so on a fresh
/// clone it did not exist -- and `load_journal` returns an empty vec for a missing path, which
/// let the pipeline test pass while silently covering nothing. Constructing it here makes the
/// fixture reproducible and the assertions real.
#[cfg(test)]
pub fn build_test_journal(dir: &std::path::Path) -> std::path::PathBuf {
    use rusqlite::params;

    let path = dir.join("journal.db");
    let conn = rusqlite::Connection::open(&path).expect("create journal fixture");
    conn.execute_batch(
        "CREATE TABLE usage_event (
            id INTEGER PRIMARY KEY,
            event_id TEXT,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            category TEXT NOT NULL,
            cost_status TEXT NOT NULL,
            requests INTEGER NOT NULL,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            reasoning_tokens INTEGER NOT NULL,
            cache_read_tokens INTEGER NOT NULL,
            cache_write_tokens INTEGER NOT NULL,
            cost REAL,
            created INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX usage_event_event_id ON usage_event(event_id);",
    )
    .expect("create journal schema");

    let rows = [
        (
            "jrnl-1",
            "ollama",
            "qwen3-coder-agent",
            "LOCAL",
            "local",
            1200_i64,
            340_i64,
        ),
        ("jrnl-2", "ollama", "gemma3:4b", "LOCAL", "local", 800, 210),
        // Same shape as jrnl-2 but a distinct event: must not be deduplicated away.
        ("jrnl-3", "ollama", "gemma3:4b", "LOCAL", "local", 800, 210),
    ];
    for (index, (event_id, provider, model, category, status, input, output)) in
        rows.iter().enumerate()
    {
        conn.execute(
            "INSERT INTO usage_event (event_id, provider, model, category, cost_status, requests,
             input_tokens, output_tokens, reasoning_tokens, cache_read_tokens, cache_write_tokens,
             cost, created) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, 0, 0, 0, NULL, ?8)",
            params![
                event_id,
                provider,
                model,
                category,
                status,
                input,
                output,
                1_700_000_000_i64 + index as i64 * 60,
            ],
        )
        .expect("seed journal fixture");
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_omarchy_record_decides_billing_when_nothing_else_does() {
        let fixtures = std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/omarchy"
        ));
        let dir = tempfile::TempDir::new().unwrap();
        let mut roots = SourceRoots {
            claude_dir: Some(dir.path().join(".claude").join("projects")),
            omarchy_dir: Some(fixtures),
            ..Default::default()
        };
        // Only when no API-key variable intervenes; a developer's shell must not decide.
        if crate::collector::billing::api_env_vars("claude_code")
            .iter()
            .chain(crate::collector::billing::api_env_vars("codex"))
            .any(|name| crate::collector::billing::env_has(name))
        {
            return;
        }
        let claude = roots.claude_decision();
        assert_eq!(claude.billing, crate::model::Billing::Subscription);
        assert_eq!(claude.tier.as_deref(), Some("Max 20x"));
        assert_eq!(claude.reason, "omarchy record");
        assert_eq!(roots.codex_decision().tier.as_deref(), Some("plus"));

        roots.limits_enabled = false;
        assert_eq!(
            roots.claude_decision().reason,
            crate::collector::billing::Decision::REASON_UNKNOWN,
            "disabling the Omarchy reader also removes it as a billing signal"
        );
    }
    fn claude_capture_roots() -> SourceRoots {
        let mut roots = SourceRoots::nowhere();
        roots.claude_dir = Some(std::path::PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/claude_capture"
        )));
        roots.claude_billing = BillingSetting::Api;
        roots
    }

    /// The transcript Claude Code 2.1.276 wrote for a session that delegated to a subagent.
    /// Ten assistant lines, five requests; in the subagent's file the first line of each request
    /// carries `output_tokens` 3 and 2, and the second 190 and 77.
    #[test]
    fn a_captured_session_counts_each_request_once_at_its_full_size() {
        let (usages, _) = load_usage(&claude_capture_roots()).unwrap();
        assert_eq!(usages.len(), 5, "{usages:#?}");
        let mut output: Vec<u64> = usages.iter().map(|u| u.output).collect();
        output.sort_unstable();
        assert_eq!(output, vec![52, 77, 91, 166, 190]);
        for usage in &usages {
            assert!(!usage.incomplete);
            assert!(usage.created > 0, "every line is dated");
            assert_eq!(usage.model, "claude-haiku-4-5-20251001");
            assert_eq!(usage.project.as_deref(), Some("/home/user/project"));
            assert_eq!(
                usage.session_id.as_deref(),
                Some("f9945b4e-cf82-4035-8783-8ab7e06ed1e4"),
                "a subagent's requests belong to the parent session"
            );
            assert!(
                usage.cost.is_some_and(|cost| cost > 0.0),
                "priced after the merge"
            );
        }
    }

    #[test]
    fn the_fuller_reading_wins_whichever_is_seen_first() {
        let small = Usage {
            event_id: Some("req_1".into()),
            input: 10,
            output: 3,
            ..Default::default()
        };
        let full = Usage {
            output: 190,
            ..small.clone()
        };
        assert!(supersedes(&full, &small));
        assert!(!supersedes(&small, &full));
        assert!(
            !supersedes(&full, &full.clone()),
            "equal rows leave the first in place"
        );
    }
}
