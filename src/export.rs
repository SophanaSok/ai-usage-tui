use std::fs;

use anyhow::Result;

use crate::cli::Cli;
use crate::collector::{load_usage, SourceRoots};
use crate::helpers::print_line;
use crate::model::{Range, Usage};
use crate::ui::cost_display;
use crate::utils::{format_count, journal_path};

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
        fs::write(path, csv)?;
        print_line(&format!(
            "Wrote usage CSV to {} ({})",
            path.display(),
            source
        ))?;
    } else if cli.json {
        let roots = SourceRoots::from_cli(cli, journal.clone());
        let limits = limits_json(&roots);
        // Filtered once and reused: the escalation block must be derived from exactly the rows
        // the export reports, or the two disagree about the same run.
        let filtered: Vec<Usage> = usages
            .iter()
            .filter(|usage| filter.matches(usage))
            .cloned()
            .collect();
        let escalations = escalations_json(&filtered);
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
                    "created": usage.created,
                    "project": usage.project,
                    "session_id": usage.session_id,
                    // What a subscription row would have cost at list rates; null otherwise.
                    "api_equivalent_cost": usage.api_equivalent_cost,
                })
            })
            .collect();
        print_line(&serde_json::to_string_pretty(&serde_json::json!({
            "source": source,
            "range": cli.range.label(),
            "usage": rows,
            // Present and empty rather than absent when disabled or not on Omarchy, so a
            // consumer can key on it.
            "limits": limits,
            // Derived from the usage above, never from recorded routing events. Always present,
            // for the same reason as `limits`.
            "escalations": escalations,
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
fn escalations_json(filtered: &[Usage]) -> serde_json::Value {
    let engine = crate::pricing::PricingEngine::load();
    let escalations = crate::escalation::derive(filtered, |model| engine.input_rate(model));
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

/// Omarchy's subscription windows, for scripts that want "session window at 92%" without
/// scraping the dashboard. `percent_used` is on the 0..100 scale, like `--check-budgets` `pct`.
fn limits_json(roots: &SourceRoots) -> Vec<serde_json::Value> {
    if !roots.limits_enabled {
        return Vec::new();
    }
    let Some(dir) = roots.omarchy_usage_dir() else {
        return Vec::new();
    };
    let report =
        crate::omarchy::load_limits(&dir, crate::utils::now(), crate::omarchy::STALE_AFTER_SECS);
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
}

impl<'a> UsageFilter<'a> {
    pub fn new(cli: &'a Cli) -> Self {
        Self {
            cutoff: cli.range.cutoff(),
            is_all: cli.range == Range::All,
            provider: cli.provider_filter.as_deref(),
            model: cli.model_filter.as_deref(),
        }
    }

    pub fn matches(&self, usage: &Usage) -> bool {
        (self.is_all || usage.created >= self.cutoff)
            && self
                .provider
                .is_none_or(|provider| usage.provider.eq_ignore_ascii_case(provider))
            && self
                .model
                .is_none_or(|model| usage.model.eq_ignore_ascii_case(model))
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
