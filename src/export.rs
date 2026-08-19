use std::fs;

use anyhow::Result;

use crate::cli::Cli;
use crate::collector::load_usage;
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
    let (usages, source) = load_usage(cli.db_path.as_deref(), &journal, cli.claude_dir.as_deref())?;
    let filter = UsageFilter::new(cli);
    if let Some(path) = &cli.csv_path {
        let mut csv = String::from(
            "provider,model,category,cost_status,requests,input_tokens,output_tokens,reasoning_tokens,cache_read_tokens,cache_write_tokens,cost,created,project,session_id\n",
        );
        for usage in usages.iter().filter(|usage| filter.matches(usage)) {
            let cost = usage
                .cost
                .map(|value| value.to_string())
                .unwrap_or_default();
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
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
            ));
        }
        fs::write(path, csv)?;
        println!("Wrote usage CSV to {} ({})", path.display(), source);
    } else if cli.json {
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
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"source": source, "range": cli.range.label(), "usage": rows})
            )?
        );
    } else {
        println!("{} ({})", source, cli.range.label());
        for usage in usages.iter().filter(|usage| filter.matches(usage)) {
            println!(
                "{} / {}: {} tokens [{}]",
                usage.provider,
                usage.model,
                format_count(usage.total_tokens()),
                cost_display(usage)
            );
        }
    }
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
