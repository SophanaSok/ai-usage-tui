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
        .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let (usages, source) = load_usage(cli.db_path.as_deref(), &journal)?;
    if let Some(path) = &cli.csv_path {
        let mut csv = String::from(
            "provider,model,category,cost_status,requests,input_tokens,output_tokens,reasoning_tokens,cache_read_tokens,cache_write_tokens,cost,created\n",
        );
        for usage in usages
            .iter()
            .filter(|usage| matches_cli_filters(usage, cli))
        {
            let cost = usage
                .cost
                .map(|value| value.to_string())
                .unwrap_or_default();
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{}\n",
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
            ));
        }
        fs::write(path, csv)?;
        println!("Wrote usage CSV to {} ({})", path.display(), source);
    } else if cli.json {
        let rows: Vec<_> = usages
            .iter()
            .filter(|usage| matches_cli_filters(usage, cli))
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
        for usage in usages
            .iter()
            .filter(|usage| matches_cli_filters(usage, cli))
        {
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
    (usage.created >= cli.range.cutoff() || cli.range == Range::All)
        && cli
            .provider_filter
            .as_ref()
            .map(|provider| usage.provider.eq_ignore_ascii_case(provider))
            .unwrap_or(true)
        && cli
            .model_filter
            .as_ref()
            .map(|model| usage.model.eq_ignore_ascii_case(model))
            .unwrap_or(true)
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
