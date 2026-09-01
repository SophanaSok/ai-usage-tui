//! The one list of data sources.
//!
//! There used to be two, wired by hand and independently: `collector::load_usage` read five
//! sources for `--json`, `--csv`, `--check-budgets`, `--omarchy-record` and the dashboard's own
//! refresh, while `main::build_collectors` constructed five background collectors for the
//! dashboard. `CONTRIBUTING.md` documented only the second. A provider added by following it
//! appeared in the dashboard and was silently missing from every export, with nothing to say so.
//!
//! Both paths now iterate [`SOURCES`]. Adding a provider is a new module in `src/collector/`
//! exposing `ID`, `read` and `collector`, plus one entry here — and
//! `every_source_is_reachable_from_both_paths` fails the build if an entry is only half wired.

use anyhow::Result;

use crate::collector::background::Collector;
use crate::collector::{SourceReport, SourceRoots};
use crate::model::Usage;

/// What a source's one-shot read returns: its report, and the rows it found.
pub type SourceRead = Result<(SourceReport, Vec<Usage>)>;

/// A source's one-shot read.
pub type LoadFn = fn(&SourceRoots) -> SourceRead;

/// A source's background-collector constructor, given the roots and a poll interval.
pub type CollectorFn = fn(&SourceRoots, u64) -> Box<dyn Collector>;

/// One data source, described once.
pub struct SourceSpec {
    /// The source's canonical id, owned by its module. Also its `[collectors.<id>]` table and
    /// the `Collector::name()` it reports.
    pub id: &'static str,
    /// What the product calls this source in prose: the README, the crate description and the
    /// GitHub topics. Distinct from `id`, which is the config key -- `claude_code` is a table
    /// name, "Claude Code" is what a reader is looking for.
    ///
    /// `tests/docs.rs` requires an H3 section per label under the README's "Data sources", and
    /// requires every `contributes_rows` label to appear in `CARGO_PKG_DESCRIPTION`, in the
    /// README's headline, and as a topic. So a collector cannot be added without the project
    /// admitting in public that it exists -- which is exactly how Gemini CLI shipped in v0.7.0
    /// and stayed missing from the README's first paragraph.
    pub label: &'static str,
    /// Whether the source is collected without being asked for. Everything that reads local
    /// files is on; anything that reaches the network is opt-in.
    pub default_enabled: bool,
    pub default_interval: u64,
    /// Whether this source contributes usage rows, and so whether
    /// `[collectors.<id>] enabled = false` should also suppress its one-shot read.
    ///
    /// False only for `zen_pricing`, whose read reports whether the pricing cache exists and
    /// produces no rows at all: its `enabled` flag governs the background *network refresh*, and
    /// switching that off must not delete the line that explains why rows are unpriced.
    pub contributes_rows: bool,
    /// Whether `[collectors.<id>] billing` and `config_json` mean anything here. Only the agent
    /// sources that can run on a plan; the rest reject the keys rather than ignoring them.
    pub supports_billing: bool,
    /// One-shot read. Every non-dashboard entry point goes through this, and so does the
    /// dashboard when no background collector is running.
    pub load: LoadFn,
    /// A background collector for the same source, polled by `CollectorHandle`.
    pub collector: CollectorFn,
}

/// Every source, in the order they are read and merged.
///
/// Order is load-bearing in two ways. It is the order of the status line the dashboard header
/// shows, and it is the deduplication order: the first entry is the base list and later entries
/// are matched against what is already there (see `load_usage`).
pub const SOURCES: &[SourceSpec] = &[
    SourceSpec {
        id: crate::collector::opencode::ID,
        label: "OpenCode",
        contributes_rows: true,
        supports_billing: false,
        default_enabled: true,
        default_interval: 30,
        load: crate::collector::opencode::read,
        collector: crate::collector::opencode::collector,
    },
    // Claude Code's own session logs: the largest source of Anthropic usage on most machines,
    // and invisible to the OpenCode collector.
    SourceSpec {
        id: crate::collector::claude_code::ID,
        label: "Claude Code",
        contributes_rows: true,
        supports_billing: true,
        default_enabled: true,
        default_interval: 30,
        load: crate::collector::claude_code::read,
        collector: crate::collector::claude_code::collector,
    },
    // Codex CLI rollouts: the OpenAI counterpart of the Claude Code logs, likewise invisible to
    // the OpenCode collector.
    SourceSpec {
        id: crate::collector::codex::ID,
        label: "Codex CLI",
        contributes_rows: true,
        supports_billing: true,
        default_enabled: true,
        default_interval: 30,
        load: crate::collector::codex::read,
        collector: crate::collector::codex::collector,
    },
    // GitHub Copilot CLI. Reads the request table its CLI store keeps, falling back to the
    // legacy session log's shutdown aggregates. Placed after Codex so that a machine running
    // both keeps the agents' own records as the base list.
    SourceSpec {
        id: crate::collector::copilot::ID,
        label: "GitHub Copilot",
        contributes_rows: true,
        supports_billing: true,
        default_enabled: true,
        default_interval: 30,
        load: crate::collector::copilot::read,
        collector: crate::collector::copilot::collector,
    },
    // Gemini CLI. Idle unless the user has switched its local telemetry on -- it persists no
    // usage otherwise -- which `--doctor` reports rather than showing an empty source.
    SourceSpec {
        id: crate::collector::gemini::ID,
        label: "Gemini CLI",
        contributes_rows: true,
        supports_billing: true,
        default_enabled: true,
        default_interval: 30,
        load: crate::collector::gemini::read,
        collector: crate::collector::gemini::collector,
    },
    SourceSpec {
        id: crate::collector::journal::ID,
        label: "Ollama",
        contributes_rows: true,
        supports_billing: false,
        default_enabled: true,
        default_interval: 60,
        load: crate::collector::journal::read,
        collector: crate::collector::journal::collector,
    },
    // Off by default: the only source here that reaches the network.
    SourceSpec {
        id: crate::collector::pricing_refresh::ID,
        label: "Pricing tables",
        contributes_rows: false,
        supports_billing: false,
        default_enabled: false,
        default_interval: 3600,
        load: crate::collector::pricing_refresh::read,
        collector: crate::collector::pricing_refresh::collector,
    },
];

/// The spec for an id, if it names a source.
pub fn find(id: &str) -> Option<&'static SourceSpec> {
    SOURCES.iter().find(|spec| spec.id == id)
}

/// Every source id, for error messages that should name the valid options.
pub fn ids() -> Vec<&'static str> {
    SOURCES.iter().map(|spec| spec.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_source_is_reachable_from_both_paths() {
        // The bug this replaces: a provider wired into the background collectors and not into
        // `load_usage` worked in the dashboard and was invisible to every export, silently.
        let roots = SourceRoots {
            journal: std::path::PathBuf::from("/nonexistent/journal.db"),
            db_path: Some(std::path::PathBuf::from("/nonexistent/opencode.db")),
            claude_dir: Some(std::path::PathBuf::from("/nonexistent/claude")),
            codex_dir: Some(std::path::PathBuf::from("/nonexistent/codex")),
            omarchy_dir: Some(std::path::PathBuf::from("/nonexistent/omarchy")),
            ..Default::default()
        };
        assert!(!SOURCES.is_empty());
        for spec in SOURCES {
            let (report, _rows) =
                (spec.load)(&roots).unwrap_or_else(|e| panic!("{} load failed: {e}", spec.id));
            assert_eq!(
                report.id, spec.id,
                "{} reports a different id from the one it is registered under",
                spec.id
            );
            let collector = (spec.collector)(&roots, spec.default_interval);
            assert_eq!(
                collector.name(),
                spec.id,
                "{}'s Collector::name() disagrees with its registry id",
                spec.id
            );
            assert_eq!(
                collector.interval(),
                std::time::Duration::from_secs(spec.default_interval)
            );
        }
    }

    #[test]
    fn ids_are_unique_and_resolvable() {
        let mut seen = std::collections::HashSet::new();
        for id in ids() {
            assert!(seen.insert(id), "duplicate source id {id:?}");
            assert!(find(id).is_some());
        }
        assert!(find("not-a-source").is_none());
    }

    /// `detect` reaches its "api key in environment" branch only through `api_env_vars`, whose
    /// fallback arm is `_ => &[]`. An id that does not match therefore does not fail loudly --
    /// it silently makes an exported API key invisible and reports the account as
    /// subscription-billed. These are the two sources that can be billed either way.
    #[test]
    fn billing_capable_sources_resolve_their_api_key_variables() {
        for id in [
            crate::collector::claude_code::ID,
            crate::collector::codex::ID,
        ] {
            assert!(
                !crate::collector::billing::api_env_vars(id).is_empty(),
                "api_env_vars({id:?}) fell through to the empty arm; billing detection for this \
                 source can no longer see an API key in the environment"
            );
        }
    }
}
