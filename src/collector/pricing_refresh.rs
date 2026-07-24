use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result};

use crate::utils::data_dir;

pub fn pricing_cache_path() -> Option<PathBuf> {
    Some(data_dir()?.join("zen-pricing.toml"))
}

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 2000;

pub fn refresh_pricing() -> Result<PathBuf> {
    let path = pricing_cache_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("pricing cache path has no parent directory"))?;
    fs::create_dir_all(parent)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let html = fetch_with_backoff(&client)?;

    let toml_content = parse_pricing_html(&html)
        .context("failed to parse Zen pricing page; page structure may have changed")?;

    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, toml_content)?;
    fs::rename(temporary, &path)?;
    Ok(path)
}

fn fetch_with_backoff(client: &reqwest::blocking::Client) -> Result<String> {
    let mut backoff = INITIAL_BACKOFF_MS;
    let mut last_error = None;

    for attempt in 1..=MAX_RETRIES {
        match client
            .get("https://opencode.ai/docs/zen/")
            .send()
            .and_then(|r| r.error_for_status())
        {
            Ok(response) => return response.text().map_err(Into::into),
            Err(err) => {
                let status = err.status();
                let is_rate_limited = status.is_some_and(|s| s == 429 || s.as_u16() == 529);
                let is_retryable = status.is_none_or(|s| {
                    s.is_server_error() || s == 429 || s.as_u16() == 529 || s == 408
                });

                if !is_retryable || attempt == MAX_RETRIES {
                    return Err(err.into());
                }

                if is_rate_limited {
                    eprintln!(
                        "Zen pricing fetch rate-limited (attempt {}/{}); retrying in {:?} ...",
                        attempt,
                        MAX_RETRIES,
                        Duration::from_millis(backoff)
                    );
                }
                std::thread::sleep(Duration::from_millis(backoff));
                last_error = Some(err);
                backoff *= 2;
            }
        }
    }

    Err(last_error
        .map(|e: reqwest::Error| anyhow::anyhow!(e))
        .unwrap_or_else(|| anyhow::anyhow!("failed to fetch Zen pricing page")))
}

fn parse_pricing_html(html: &str) -> Result<String> {
    let table = extract_pricing_table(html);
    if table.is_empty() {
        return Err(anyhow::anyhow!(
            "No pricing table found in Zen docs page; page structure may have changed"
        ));
    }

    let rows = parse_table_rows(&table);
    if rows.is_empty() {
        return Err(anyhow::anyhow!("No pricing rows found in Zen docs table"));
    }

    let model_names = known_model_names();
    let mut entries = Vec::new();

    for row in &rows {
        if row.len() < 5 {
            continue;
        }
        let display_name = &row[0];
        let model_id = match model_names
            .iter()
            .find(|(name, _)| name == display_name)
            .map(|(_, id)| id.clone())
        {
            Some(id) => id,
            None => continue,
        };

        let tier = extract_tier(display_name);
        let input = parse_price(&row[1]);
        let output = parse_price(&row[2]);
        let cache_read = parse_price(&row[3]);
        let cache_write = parse_price(&row[4]);

        let free = input.is_none()
            && output.is_none()
            && cache_read.is_none()
            && cache_write.is_none()
            && row[1].eq_ignore_ascii_case("free");

        entries.push(PricingEntry {
            model_id,
            free,
            tier,
            input,
            output,
            cache_read,
            cache_write,
        });
    }

    if entries.is_empty() {
        return Err(anyhow::anyhow!(
            "No recognized pricing entries found; model names may have changed"
        ));
    }

    let mut output = String::new();
    output.push_str("# OpenCode Zen pricing table (per 1M tokens)\n");
    output.push_str("# Auto-refreshed from https://opencode.ai/docs/zen/\n\n");

    for entry in &entries {
        if entry.free {
            output.push_str(&format!("[model.\"{}\"]\nfree = true\n\n", entry.model_id));
            continue;
        }
        if let Some(ref tier) = entry.tier {
            output.push_str(&format!("[model.\"{}\".tier-{}]\n", entry.model_id, tier));
        } else {
            output.push_str(&format!("[model.\"{}\"]\n", entry.model_id));
        }
        if let Some(v) = entry.input {
            output.push_str(&format!("input = {}\n", v));
        }
        if let Some(v) = entry.output {
            output.push_str(&format!("output = {}\n", v));
        }
        if let Some(v) = entry.cache_read {
            output.push_str(&format!("cache_read = {}\n", v));
        }
        if let Some(v) = entry.cache_write {
            output.push_str(&format!("cache_write = {}\n", v));
        }
        output.push('\n');
    }

    Ok(output)
}

struct PricingEntry {
    model_id: String,
    free: bool,
    tier: Option<String>,
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

fn extract_pricing_table(html: &str) -> String {
    let pricing_idx = html.find("id=\"pricing\"").or_else(|| html.find("Pricing"));
    let Some(start) = pricing_idx else {
        return String::new();
    };
    let after = &html[start..];

    let Some(table_start) = after.find("<table>") else {
        return String::new();
    };
    let Some(table_end) = after[table_start..].find("</table>") else {
        return String::new();
    };
    after[table_start..table_start + table_end + 8].to_string()
}

fn parse_table_rows(table_html: &str) -> Vec<Vec<String>> {
    let Some(tbody_start) = table_html
        .find("<tbody>")
        .or_else(|| table_html.find("<tbody "))
    else {
        return Vec::new();
    };
    let tbody = &table_html[tbody_start..];

    let mut rows = Vec::new();
    for tr_content in tbody.split("<tr>").skip(1) {
        let row_end = tr_content.find("</tr>").unwrap_or(tr_content.len());
        let row_html = &tr_content[..row_end];
        let cells = parse_td_cells(row_html);
        if !cells.is_empty() {
            rows.push(cells);
        }
    }
    rows
}

fn parse_td_cells(row_html: &str) -> Vec<String> {
    let mut cells = Vec::new();
    for td_content in row_html.split("<td>").skip(1) {
        let cell_end = td_content.find("</td>").unwrap_or(td_content.len());
        let cell_html = &td_content[..cell_end];
        let text = strip_tags(cell_html);
        cells.push(text.trim().to_string());
    }
    cells
}

fn strip_tags(html: &str) -> String {
    // Protect literal angle-bracket entities from the tag stripper, then restore.
    let guarded = html
        .replace("&amp;", "\0AMP\0")
        .replace("&lt;", "\0LT\0")
        .replace("&gt;", "\0GT\0")
        .replace("&quot;", "\0QUOT\0")
        .replace("&#39;", "\0APOS\0")
        .replace("&nbsp;", "\0NBSP\0");

    let mut result = String::with_capacity(guarded.len());
    let mut in_tag = false;
    for ch in guarded.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' && in_tag {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }

    result
        .replace("\0AMP\0", "&")
        .replace("\0LT\0", "<")
        .replace("\0GT\0", ">")
        .replace("\0QUOT\0", "\"")
        .replace("\0APOS\0", "'")
        .replace("\0NBSP\0", " ")
        .trim()
        .to_string()
}

fn parse_price(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed == "-"
        || trimmed == "—"
        || trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("free")
    {
        return None;
    }
    trimmed.trim_start_matches('$').parse::<f64>().ok()
}

fn extract_tier(display_name: &str) -> Option<String> {
    if let Some(start) = display_name.find("(> ") {
        let rest = &display_name[start + 3..];
        if let Some(end) = rest.find(" tokens)") {
            let threshold_str = &rest[..end];
            let expanded = threshold_str
                .replace("K", "000")
                .replace("M", "000000")
                .replace(',', "");
            let threshold: u64 = expanded.parse().unwrap_or(0);
            return Some(threshold.to_string());
        }
    }
    None
}

fn known_model_names() -> Vec<(String, String)> {
    vec![
        ("Big Pickle".into(), "big-pickle".into()),
        (
            "DeepSeek V4 Flash Free".into(),
            "deepseek-v4-flash-free".into(),
        ),
        ("MiMo-V2.5 Free".into(), "mimo-v2.5-free".into()),
        ("Laguna S 2.1 Free".into(), "laguna-s-2.1-free".into()),
        ("Ling-3.0-flash Free".into(), "ling-3.0-flash-free".into()),
        ("North Mini Code Free".into(), "north-mini-code-free".into()),
        (
            "Nemotron 3 Ultra Free".into(),
            "nemotron-3-ultra-free".into(),
        ),
        ("MiniMax M3".into(), "minimax-m3".into()),
        ("MiniMax M2.7".into(), "minimax-m2.7".into()),
        ("MiniMax M2.5".into(), "minimax-m2.5".into()),
        ("GLM 5.2".into(), "glm-5.2".into()),
        ("GLM 5.1".into(), "glm-5.1".into()),
        ("GLM 5".into(), "glm-5".into()),
        ("Kimi K2.7 Code".into(), "kimi-k2.7-code".into()),
        ("Kimi K2.6".into(), "kimi-k2.6".into()),
        ("Kimi K2.5".into(), "kimi-k2.5".into()),
        ("Qwen3.7 Max".into(), "qwen3.7-max".into()),
        ("Qwen3.7 Plus".into(), "qwen3.7-plus".into()),
        ("Qwen3.6 Plus".into(), "qwen3.6-plus".into()),
        ("Qwen3.5 Plus".into(), "qwen3.5-plus".into()),
        ("DeepSeek V4 Pro".into(), "deepseek-v4-pro".into()),
        ("DeepSeek V4 Flash".into(), "deepseek-v4-flash".into()),
        ("Claude Fable 5".into(), "claude-fable-5".into()),
        ("Claude Opus 4.8".into(), "claude-opus-4-8".into()),
        ("Claude Opus 4.7".into(), "claude-opus-4-7".into()),
        ("Claude Opus 4.6".into(), "claude-opus-4-6".into()),
        ("Claude Opus 4.5".into(), "claude-opus-4-5".into()),
        ("Claude Sonnet 5".into(), "claude-sonnet-5".into()),
        ("Claude Sonnet 4.6".into(), "claude-sonnet-4-6".into()),
        (
            "Claude Sonnet 4.5 (\u{2264} 200K tokens)".into(),
            "claude-sonnet-4.5".into(),
        ),
        (
            "Claude Sonnet 4.5 (> 200K tokens)".into(),
            "claude-sonnet-4.5".into(),
        ),
        ("Claude Haiku 4.5".into(), "claude-haiku-4.5".into()),
        ("Gemini 3.6 Flash".into(), "gemini-3.6-flash".into()),
        ("Gemini 3.5 Flash".into(), "gemini-3.5-flash".into()),
        (
            "Gemini 3.5 Flash Lite".into(),
            "gemini-3.5-flash-lite".into(),
        ),
        (
            "Gemini 3.1 Pro (\u{2264} 200K tokens)".into(),
            "gemini-3.1-pro".into(),
        ),
        (
            "Gemini 3.1 Pro (> 200K tokens)".into(),
            "gemini-3.1-pro".into(),
        ),
        ("Gemini 3 Flash".into(), "gemini-3-flash".into()),
        ("Grok 4.5 (\u{2264} 200K tokens)".into(), "grok-4.5".into()),
        ("Grok 4.5 (> 200K tokens)".into(), "grok-4.5".into()),
        ("Grok Build 0.1".into(), "grok-build-0.1".into()),
        (
            "GPT 5.6 Sol (\u{2264} 272K tokens)".into(),
            "gpt-5.6-sol".into(),
        ),
        ("GPT 5.6 Sol (> 272K tokens)".into(), "gpt-5.6-sol".into()),
        (
            "GPT 5.6 Terra (\u{2264} 272K tokens)".into(),
            "gpt-5.6-terra".into(),
        ),
        (
            "GPT 5.6 Terra (> 272K tokens)".into(),
            "gpt-5.6-terra".into(),
        ),
        (
            "GPT 5.6 Luna (\u{2264} 272K tokens)".into(),
            "gpt-5.6-luna".into(),
        ),
        ("GPT 5.6 Luna (> 272K tokens)".into(), "gpt-5.6-luna".into()),
        ("GPT 5.5 (\u{2264} 272K tokens)".into(), "gpt-5.5".into()),
        ("GPT 5.5 (> 272K tokens)".into(), "gpt-5.5".into()),
        ("GPT 5.5 Pro".into(), "gpt-5.5-pro".into()),
        ("GPT 5.4 (\u{2264} 272K tokens)".into(), "gpt-5.4".into()),
        ("GPT 5.4 (> 272K tokens)".into(), "gpt-5.4".into()),
        ("GPT 5.4 Pro".into(), "gpt-5.4-pro".into()),
        ("GPT 5.4 Mini".into(), "gpt-5.4-mini".into()),
        ("GPT 5.4 Nano".into(), "gpt-5.4-nano".into()),
        ("GPT 5.3 Codex Spark".into(), "gpt-5.3-codex-spark".into()),
        ("GPT 5.3 Codex".into(), "gpt-5.3-codex".into()),
        ("GPT 5.2".into(), "gpt-5.2".into()),
        ("GPT 5.2 Codex".into(), "gpt-5.2-codex".into()),
        ("GPT 5.1".into(), "gpt-5.1".into()),
        ("GPT 5.1 Codex".into(), "gpt-5.1-codex".into()),
        ("GPT 5.1 Codex Max".into(), "gpt-5.1-codex-max".into()),
        ("GPT 5.1 Codex Mini".into(), "gpt-5.1-codex-mini".into()),
        ("GPT 5".into(), "gpt-5".into()),
        ("GPT 5 Codex".into(), "gpt-5-codex".into()),
        ("GPT 5 Nano".into(), "gpt-5-nano".into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_tags_removes_html() {
        assert_eq!(strip_tags("<p>hello</p>"), "hello");
        assert_eq!(strip_tags("<code dir=\"auto\">$1.00</code>"), "$1.00");
    }

    #[test]
    fn parse_price_handles_dollar_and_dash() {
        assert_eq!(parse_price("$1.00"), Some(1.0));
        assert_eq!(parse_price("$0.26"), Some(0.26));
        assert_eq!(parse_price("-"), None);
        assert_eq!(parse_price("Free"), None);
        assert_eq!(parse_price(""), None);
    }

    #[test]
    fn extract_tier_detects_threshold() {
        assert_eq!(
            extract_tier("GPT 5.6 Luna (> 272K tokens)"),
            Some("272000".to_string())
        );
        assert_eq!(extract_tier("GPT 5.6 Luna"), None);
    }

    #[test]
    fn known_models_are_complete() {
        let models = known_model_names();
        assert!(models.len() >= 58);
    }

    #[test]
    fn parse_table_rows_extracts_cells() {
        let html = r#"<tbody><tr><td>Big Pickle</td><td>Free</td><td>Free</td><td>Free</td><td>-</td></tr><tr><td>GLM 5.2</td><td>$1.40</td><td>$4.40</td><td>$0.26</td><td>-</td></tr></tbody>"#;
        let rows = parse_table_rows(html);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], "Big Pickle");
        assert_eq!(rows[1][0], "GLM 5.2");
        assert_eq!(rows[1][1], "$1.40");
    }

    #[test]
    fn parse_pricing_html_handles_tiered_rows() {
        let html = r#"<h2 id="pricing">Pricing</h2><table><thead><tr><th>Model</th><th>Input</th><th>Output</th><th>Cached Read</th><th>Cached Write</th></tr></thead><tbody><tr><td>Big Pickle</td><td>Free</td><td>Free</td><td>Free</td><td>-</td></tr><tr><td>GPT 5.6 Luna (≤ 272K tokens)</td><td>$1.00</td><td>$6.00</td><td>$0.10</td><td>$1.25</td></tr><tr><td>GPT 5.6 Luna (> 272K tokens)</td><td>$2.00</td><td>$9.00</td><td>$0.20</td><td>$2.50</td></tr></tbody></table>"#;
        let toml = parse_pricing_html(html).unwrap();
        assert!(toml.contains("[model.\"big-pickle\"]\nfree = true"));
        assert!(toml.contains("[model.\"gpt-5.6-luna\"]"));
        assert!(toml.contains("[model.\"gpt-5.6-luna\".tier-272000]"));
        assert!(toml.contains("input = 1\n"));
        assert!(toml.contains("input = 2\n"));
    }

    #[test]
    fn parse_fixture_produces_valid_toml() {
        let html = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/zen_pricing.html"
        ))
        .expect("missing fixture tests/fixtures/zen_pricing.html; run curl to capture it");
        let toml = parse_pricing_html(&html).expect("fixture should parse successfully");
        assert!(toml.contains("[model.\"big-pickle\"]\nfree = true"));
        assert!(toml.contains("[model.\"gpt-5.6-luna\"]"));
        assert!(toml.contains("[model.\"gpt-5.6-luna\".tier-272000]"));
        assert!(toml.contains("[model.\"claude-sonnet-4.5\".tier-200000]"));
        // Ensure the parsed output is valid TOML.
        let _: toml::Value = toml
            .parse()
            .expect("generated pricing TOML should be valid");
    }
}
