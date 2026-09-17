use std::fs;

use anyhow::Result;

use crate::cli::Cli;
use crate::collector::{load_usage, SourceRoots};
use crate::helpers::print_line;
use crate::model::{Range, Usage};
use crate::ui::cost_display;
use crate::utils::{format_count, journal_path};

/// The version of every JSON document this tool prints: `--summary-json`, `--json`,
/// `--routing-json` and `--check-budgets`. Within a version, keys are only ever added -- never removed, renamed or
/// changed in meaning -- so a consumer that ignores unknown keys keeps working. A change that
/// breaks that raises it. See `docs/stability.md`.
pub const JSON_SCHEMA_VERSION: u32 = 1;

pub fn print_once(cli: &Cli) -> Result<()> {
    let journal = cli
        .journal_path
        .clone()
        .or_else(journal_path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine a home directory; pass an explicit path (see --help)"
            )
        })?;
    let (usages, source) = load_usage(&SourceRoots::from_cli(cli, journal.clone()))?;
    let filter = UsageFilter::new(cli);
    if let Some(path) = &cli.csv_path {
        let mut csv = String::from(
            "provider,model,category,cost_status,requests,input_tokens,output_tokens,reasoning_tokens,cache_read_tokens,cache_write_tokens,cost,created,project,session_id,api_equivalent_cost\n",
        );
        for usage in usages.iter().filter(|usage| filter.matches(usage)) {
            let cost = usage
                .cost
                .map(|value| value.to_string())
                .unwrap_or_default();
            let api_equivalent = usage
                .api_equivalent_cost
                .map(|value| value.to_string())
                .unwrap_or_default();
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_field(&usage.provider),
                csv_field(&usage.model),
                usage.category.label(),
                usage.cost_status.label(),
                usage.requests,
                usage.input,
                usage.output,
                usage.reasoning,
                usage.cache_read,
                usage.cache_write,
                csv_field(&cost),
                usage.created,
                // Appended, never inserted: a consumer reading by column index keeps working.
                csv_field(usage.project.as_deref().unwrap_or_default()),
                csv_field(usage.session_id.as_deref().unwrap_or_default()),
                csv_field(&api_equivalent),
            ));
        }
        // `--csv -` is stdout, by the usual convention. CSV is the compact row format -- about a
        // quarter of the JSON's size -- and it could only ever be written to a file, so it was
        // unreachable from a pipeline. Nothing else is printed in that case: the confirmation
        // line would be a row of garbage in the consumer's table.
        if path.as_os_str() == "-" {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            out.write_all(csv.as_bytes())?;
            out.flush()?;
        } else {
            fs::write(path, csv)?;
            print_line(&format!(
                "Wrote usage CSV to {} ({})",
                path.display(),
                source
            ))?;
        }
    } else if cli.json {
        let roots = SourceRoots::from_cli(cli, journal.clone());
        let limits = limits_report_json(&crate::limits::load(&roots, crate::utils::now()));
        // Filtered once and reused: the escalation block must be derived from exactly the rows
        // the export reports, or the two disagree about the same run.
        let filtered: Vec<Usage> = usages
            .iter()
            .filter(|usage| filter.matches(usage))
            .cloned()
            .collect();
        let engine = crate::pricing::PricingEngine::load();
        let escalations = escalations_json(&filtered, &|model| engine.input_rate(model));
        let provenance = provenance_json(&filtered);
        let rows: Vec<_> = filtered
            .iter()
            .map(|usage| {
                serde_json::json!({
                    "provider": usage.provider,
                    "model": usage.model,
                    "category": usage.category.label(),
                    "cost_status": usage.cost_status.label(),
                    "requests": usage.requests,
                    "input_tokens": usage.input,
                    "output_tokens": usage.output,
                    "reasoning_tokens": usage.reasoning,
                    "cache_read_tokens": usage.cache_read,
                    "cache_write_tokens": usage.cache_write,
                    "cost": usage.cost,
                    // How the request was paid for. Decided per source and until now visible only
                    // in `--doctor`'s text, although it is what makes a row `quota`.
                    "billing": usage.billing.label(),
                    // True when the source record lacked a token count it always carries: the
                    // zeros beside it are then "not recorded", and the row is never priced.
                    "incomplete": usage.incomplete,
                    "created": usage.created,
                    "project": usage.project,
                    "session_id": usage.session_id,
                    // What a subscription row would have cost at list rates; null otherwise.
                    "api_equivalent_cost": usage.api_equivalent_cost,
                })
            })
            .collect();
        print_line(&serde_json::to_string_pretty(&serde_json::json!({
            // The machine-readable contract's version; see docs/stability.md.
            "schema_version": JSON_SCHEMA_VERSION,
            "source": source,
            "range": cli.range.label(),
            "usage": rows,
            // Present and empty rather than absent when disabled or not on Omarchy, so a
            // consumer can key on it.
            "limits": limits,
            // Derived from the usage above, never from recorded routing events. Always present,
            // for the same reason as `limits`.
            "escalations": escalations,
            // Where the numbers above came from. Always present, every status always keyed.
            "provenance": provenance,
        }))?)?;
    } else {
        print_line(&format!("{} ({})", source, cli.range.label()))?;
        for usage in usages.iter().filter(|usage| filter.matches(usage)) {
            print_line(&format!(
                "{} / {}: {} tokens [{}]",
                usage.provider,
                usage.model,
                format_count(usage.total_tokens()),
                cost_display(usage)
            ))?;
        }
    }
    Ok(())
}

/// Where the exported figures came from, by cost status.
///
/// The header has always reduced this to one pricing-coverage percentage; a script had no way to
/// ask the harder question, which is how much of a total a provider actually reported versus how
/// much this tool worked out from a rate table. Derived from the same filtered rows the export
/// reports, so a `--provider` filter narrows both.
///
/// `cost` is null for `quota` and `unavailable` rather than `0.0`. Those rows have no per-token
/// price, and a zero would assert they were free.
pub(crate) fn provenance_json(filtered: &[Usage]) -> serde_json::Value {
    let provenance = crate::model::Provenance::of(filtered);
    serde_json::json!({
        "reported_share": provenance.reported_share(),
        "billable_cost": provenance.billable_cost(),
        "quota_requests": provenance.quota_requests(),
        "unpriced_requests": provenance.unpriced_requests(),
        "by_cost_status": provenance.buckets.iter().map(|(status, bucket)| {
            serde_json::json!({
                "cost_status": status.label(),
                "rows": bucket.rows,
                "requests": bucket.requests,
                "tokens": bucket.tokens,
                "cost": bucket.cost,
                "api_equivalent_cost": bucket.api_equivalent_cost,
            })
        }).collect::<Vec<_>>(),
    })
}

/// Escalations derived from the usage in range — the routing panel's derived block, for scripts.
///
/// This was TUI-only: `--json` carried usage rows and nothing that answered "did sessions move
/// to a pricier model, and what did that cost". Derived from the same filtered rows the export
/// reports, with the same pricing table the dashboard ranks models by, so a script and the
/// dashboard cannot disagree about one run.
///
/// Deliberately *not* merged into `--routing-json`. That export reads recorded `--record-routing`
/// events from the journal and nothing else; these are inferred from usage. The dashboard shows
/// them adjacent and labels them as different things, and a test asserts it — folding one into
/// the other in an export would undo exactly that distinction.
///
/// `escalation_rate` is null rather than 0 when no session had enough information to examine: a
/// rate over zero sessions is not a fact about anything.
///
/// `rate_of` is the pricing table's input rate, passed in so this stays pure and both exports
/// that print the block order models by the same table.
pub(crate) fn escalations_json(
    filtered: &[Usage],
    rate_of: &dyn Fn(&str) -> Option<f64>,
) -> serde_json::Value {
    let escalations = crate::escalation::derive(filtered, rate_of);
    serde_json::json!({
        "sessions_examined": escalations.sessions_examined,
        "sessions_escalated": escalations.sessions_escalated,
        "escalation_rate": escalations.rate(),
        // Model changes that could not be ordered because a rate was missing on one side.
        // Reported so a low escalation count is distinguishable from a blind one.
        "unclassified_changes": escalations.unclassified_changes,
        "transitions": escalations.transitions.iter().map(|transition| {
            serde_json::json!({
                "from": transition.from,
                "to": transition.to,
                // The list rates the two models were ordered by, dollars per million input
                // tokens. Without them the direction rests on the reader knowing which name is
                // the pricier model -- and a reader given only names guessed, and guessed wrong.
                "from_input_rate": rate_of(&transition.from),
                "to_input_rate": rate_of(&transition.to),
                "sessions": transition.sessions,
                // Spend on models pricier than the one the session opened with. A floor, not a
                // total, whenever `unpriced_after` or `quota_after` is non-zero.
                "cost_after": transition.cost_after,
                "unpriced_after": transition.unpriced_after,
                "quota_after": transition.quota_after,
            })
        }).collect::<Vec<_>>(),
    })
}

/// Subscription windows from every source that reports them -- Omarchy's agents panel and
/// Claude Code's own cached utilisation -- for scripts that want "session window at 92%" without
/// scraping the dashboard. `percent_used` is on the 0..100 scale, like `--check-budgets` `pct`.
///
/// Callers get the report from `limits::load` rather than reading Omarchy directly, so a script
/// and the dashboard cannot disagree about one run: they previously ran two independent reads.
pub(crate) fn limits_report_json(report: &crate::omarchy::LimitsReport) -> Vec<serde_json::Value> {
    report
        .snapshots
        .iter()
        .map(|snapshot| {
            serde_json::json!({
                "agent": snapshot.agent,
                "name": snapshot.name,
                "tier": snapshot.tier,
                "status": snapshot.status_text,
                "updated_at": snapshot.updated_at,
                "age_secs": snapshot.age_secs,
                "stale": snapshot.stale,
                "windows": snapshot.windows.iter().map(|window| serde_json::json!({
                    "label": window.label,
                    "percent_used": window.percent_used(),
                    "resets_at": window.resets_at,
                    "resets_in_secs": window.resets_in_secs,
                })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// `--summary-json`: gather everything `summary::build` needs, and print the document compactly.
///
/// One line, not pretty-printed like the other exports: whitespace is about a fifth of a pretty
/// document's tokens, and this one exists for a reader that pays for every token. `| jq .` is the
/// human view.
pub fn print_summary(cli: &Cli, budgets: &crate::budget::BudgetEngine) -> Result<()> {
    let journal = cli
        .journal_path
        .clone()
        .or_else(journal_path)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "could not determine a home directory; pass an explicit path (see --help)"
            )
        })?;
    let roots = SourceRoots::from_cli(cli, journal.clone());
    let now = crate::utils::now();
    let (sources, usages) = crate::collector::diagnose_with_usage(&roots)?;
    let filter = UsageFilter::new(cli);
    let filtered: Vec<Usage> = usages
        .iter()
        .filter(|usage| filter.matches(usage))
        .cloned()
        .collect();
    let engine = crate::pricing::PricingEngine::load();
    let routing_events: Vec<_> = crate::collector::journal::load_routing(&journal)?
        .into_iter()
        .filter(|event| filter.in_range(event.created))
        .collect();
    let document = crate::summary::build(&crate::summary::Inputs {
        schema_version: JSON_SCHEMA_VERSION,
        now,
        scope: crate::summary::Scope {
            range: cli.range.label(),
            since: filter.since(),
            provider: cli.provider_filter.clone(),
            model: cli.model_filter.clone(),
            project: cli.project_filter.clone(),
            session: cli.session_filter.clone(),
        },
        top: cli.top,
        usages: &filtered,
        sources: &sources,
        pricing_models: engine.model_count(),
        pricing_warnings: engine.warnings(),
        // All usage, not the filtered rows: how fast tokens are going *now* does not depend on
        // which month the rest of the document is about. The dashboard does the same.
        burn: &crate::ui::aggregate::burn_rate(&usages, 3600, now),
        // Budgets carry their own period, so they too are checked against everything.
        budgets: &budgets.check(&usages),
        limits: &crate::limits::load(&roots, now),
        routing_events: &routing_events,
        input_rate: &|model| engine.input_rate(model),
    });
    print_line(&serde_json::to_string(&document)?)?;
    Ok(())
}

pub fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn matches_cli_filters(usage: &Usage, cli: &Cli) -> bool {
    UsageFilter::new(cli).matches(usage)
}

/// A filter with the range cutoff resolved once.
///
/// `Range::cutoff()` reads the clock (and for `Today`, the local timezone). Calling it from
/// inside a filter predicate meant one clock lookup per usage row per pass.
pub struct UsageFilter<'a> {
    cutoff: i64,
    is_all: bool,
    provider: Option<&'a str>,
    model: Option<&'a str>,
    project: Option<&'a str>,
    session: Option<&'a str>,
}

impl<'a> UsageFilter<'a> {
    pub fn new(cli: &'a Cli) -> Self {
        Self {
            cutoff: cli.range.cutoff(),
            is_all: cli.range == Range::All,
            provider: cli.provider_filter.as_deref(),
            model: cli.model_filter.as_deref(),
            project: cli.project_filter.as_deref(),
            session: cli.session_filter.as_deref(),
        }
    }

    /// When the range starts, or `None` for all history.
    pub fn since(&self) -> Option<i64> {
        (!self.is_all).then_some(self.cutoff)
    }

    /// Whether a timestamp falls in the range, for things that are not usage rows.
    pub fn in_range(&self, created: i64) -> bool {
        self.is_all || created >= self.cutoff
    }

    pub fn matches(&self, usage: &Usage) -> bool {
        (self.is_all || usage.created >= self.cutoff)
            && self
                .provider
                .is_none_or(|provider| usage.provider.eq_ignore_ascii_case(provider))
            && self
                .model
                .is_none_or(|model| usage.model.eq_ignore_ascii_case(model))
            // Exact, and case-sensitive: these are a path and an id, copied from an export. The
            // trailing separator is forgiven because the collectors strip it before storing.
            && self.project.is_none_or(|project| {
                let wanted = project.trim_end_matches(['/', '\\']);
                match usage.project.as_deref() {
                    Some(actual) => actual == wanted || actual == project,
                    None => project == crate::summary::UNATTRIBUTED,
                }
            })
            && self
                .session
                .is_none_or(|session| usage.session_id.as_deref() == Some(session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_fields_are_escaped() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("model,one"), "\"model,one\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
