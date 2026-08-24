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
        let rows: Vec<_> = usages
            .iter()
            .filter(|usage| filter.matches(usage))
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
