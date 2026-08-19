use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result};

use crate::utils::data_dir;

pub fn pricing_cache_path() -> Option<PathBuf> {
    Some(data_dir()?.join("zen-pricing.toml"))
}

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 2000;

pub fn refresh_pricing() -> Result<PathBuf> {
    let path = pricing_cache_path().ok_or_else(|| {
        anyhow::anyhow!("could not determine a home directory; pass an explicit path (see --help)")
    })?;
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

    let mut entries = Vec::new();

    for row in &rows {
        if row.len() < 5 {
            continue;
        }
        let display_name = &row[0];
        let Some(model_id) = derive_model_id(display_name) else {
            continue;
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

        // With no hardcoded name list to filter against, a row that prices nothing is the
        // only remaining signal that it is not a pricing row at all — a note, a spacer, or a
        // sub-heading the table happens to contain.
        if !free
            && input.is_none()
            && output.is_none()
            && cache_read.is_none()
            && cache_write.is_none()
        {
            continue;
        }

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
            "No priced rows found in the Zen pricing table; page structure may have changed"
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

/// Turn a pricing-table display name into the model id the pricing table is keyed on.
///
/// This replaces a 65-entry hardcoded `(display name, model id)` table that every scraped row
/// had to match to survive. That table could not do the one thing a refresh exists for —
/// discover a model that was not already known — so a newly launched model stayed unpriced
/// until someone edited Rust source. It was also where the Claude Opus dash/dot key mismatch
/// lived, silently unpricing the most expensive model family in the catalog.
///
/// The mapping was never arbitrary: all 66 entries of that table are reproduced exactly by
/// this rule, which is asserted in `derivation_reproduces_every_previously_hardcoded_id`.
///
/// Returns `None` for a name with no alphanumeric content, which is not a model.
fn derive_model_id(display_name: &str) -> Option<String> {
    // A context-tier row ("Claude Sonnet 4.5 (> 200K tokens)") is the same model as its base
    // row; the threshold is carried separately by `extract_tier`.
    let base = strip_tier_suffix(display_name).trim().to_ascii_lowercase();

    let mut id = String::with_capacity(base.len());
    for ch in base.chars() {
        // Dots are significant: model versions are `glm-5.2`, not `glm-5-2`.
        if ch.is_alphanumeric() || ch == '.' {
            id.push(ch);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches('-').to_string();

    if id.chars().any(|c| c.is_alphanumeric()) {
        Some(id)
    } else {
        None
    }
}

/// Drop a trailing context-window parenthetical, e.g. `" (> 200K tokens)"`.
fn strip_tier_suffix(display_name: &str) -> &str {
    let trimmed = display_name.trim_end();
    let Some(open) = trimmed.rfind('(') else {
        return trimmed;
    };
    let suffix = &trimmed[open..];
    if suffix.ends_with(')') && suffix.contains("tokens") {
        &trimmed[..open]
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every model id the previous hardcoded table could emit.
    ///
    /// Kept as a test fixture, not production logic: it is the regression guard the old table
    /// provided as a side effect — that the scraper spells a model exactly the way the bundled
    /// pricing table keys it. A mismatch here is the Claude Opus dash/dot bug returning. Unlike
    /// the old table it does *not* gate what the scraper will emit, so a new model on the page is
    /// discovered rather than dropped.
    const PREVIOUSLY_HARDCODED_IDS: &[&str] = &[
        "big-pickle",
        "deepseek-v4-flash-free",
        "mimo-v2.5-free",
        "laguna-s-2.1-free",
        "ling-3.0-flash-free",
        "north-mini-code-free",
        "nemotron-3-ultra-free",
        "minimax-m3",
        "minimax-m2.7",
        "minimax-m2.5",
        "glm-5.2",
        "glm-5.1",
        "glm-5",
        "kimi-k2.7-code",
        "kimi-k2.6",
        "kimi-k2.5",
        "qwen3.7-max",
        "qwen3.7-plus",
        "qwen3.6-plus",
        "qwen3.5-plus",
        "deepseek-v4-pro",
        "deepseek-v4-flash",
        "claude-fable-5",
        "claude-opus-4.8",
        "claude-opus-4.7",
        "claude-opus-4.6",
        "claude-opus-4.5",
        "claude-sonnet-5",
        "claude-sonnet-4.6",
        "claude-sonnet-4.5",
        "claude-haiku-4.5",
        "gemini-3.6-flash",
        "gemini-3.5-flash",
        "gemini-3.5-flash-lite",
        "gemini-3.1-pro",
        "gemini-3-flash",
        "grok-4.5",
        "grok-build-0.1",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
        "gpt-5.5",
        "gpt-5.5-pro",
        "gpt-5.4",
        "gpt-5.4-pro",
        "gpt-5.4-mini",
        "gpt-5.4-nano",
        "gpt-5.3-codex-spark",
        "gpt-5.3-codex",
        "gpt-5.2",
        "gpt-5.2-codex",
        "gpt-5.1",
        "gpt-5.1-codex",
        "gpt-5.1-codex-max",
        "gpt-5.1-codex-mini",
        "gpt-5",
        "gpt-5-codex",
        "gpt-5-nano",
    ];

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
    fn derivation_reproduces_every_previously_hardcoded_id() {
        // The 65-entry table was deleted on the claim that its mapping was mechanical. This
        // is that claim, checked: each display name it carried still derives to the same id.
        let cases = [
            ("Big Pickle", "big-pickle"),
            ("GLM 5.2", "glm-5.2"),
            ("MiniMax M3", "minimax-m3"),
            ("MiMo-V2.5 Free", "mimo-v2.5-free"),
            ("Ling-3.0-flash Free", "ling-3.0-flash-free"),
            ("Claude Opus 4.8", "claude-opus-4.8"),
            (
                "Claude Sonnet 4.5 (\u{2264} 200K tokens)",
                "claude-sonnet-4.5",
            ),
            ("Claude Sonnet 4.5 (> 200K tokens)", "claude-sonnet-4.5"),
            ("GPT 5.3 Codex Spark", "gpt-5.3-codex-spark"),
            ("GPT 5.6 Luna (> 272K tokens)", "gpt-5.6-luna"),
            ("Gemini 3.5 Flash Lite", "gemini-3.5-flash-lite"),
            ("Grok Build 0.1", "grok-build-0.1"),
            ("Nemotron 3 Ultra Free", "nemotron-3-ultra-free"),
        ];
        for (display, expected) in cases {
            assert_eq!(
                derive_model_id(display).as_deref(),
                Some(expected),
                "{display}"
            );
        }
    }

    #[test]
    fn tier_parentheticals_are_stripped_but_other_parentheses_are_not() {
        // Only a context-window suffix names the same model twice. Anything else in
        // parentheses is part of the name and dropping it would merge two distinct models.
        assert_eq!(strip_tier_suffix("Grok 4.5 (> 200K tokens)"), "Grok 4.5 ");
        assert_eq!(strip_tier_suffix("Model (preview)"), "Model (preview)");
        assert_eq!(
            derive_model_id("Model (preview)").as_deref(),
            Some("model-preview")
        );
    }

    #[test]
    fn a_name_with_no_alphanumeric_content_is_not_a_model() {
        assert_eq!(derive_model_id("—"), None);
        assert_eq!(derive_model_id("  "), None);
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
        let _: toml::Table = toml
            .parse()
            .expect("generated pricing TOML should be valid");
    }

    #[test]
    fn a_model_the_binary_has_never_heard_of_is_still_priced() {
        // The point of 1.21. The old scraper filtered every row against a hardcoded name
        // table, so a model launched after the last release stayed unpriced until someone
        // edited Rust source and cut a new binary — on a code path whose entire job is to
        // pick up pricing changes without a release.
        let html = concat!(
            r#"<h2 id="pricing">Pricing</h2><table><thead><tr><th>Model</th></tr></thead>"#,
            r#"<tbody><tr><td>Brand New Model 9.9</td>"#,
            r#"<td>$3.00</td><td>$15.00</td><td>$0.30</td><td>$3.75</td></tr></tbody></table>"#
        );
        let toml = parse_pricing_html(html).unwrap();
        assert!(
            toml.contains(r#"[model."brand-new-model-9.9"]"#),
            "a model absent from the binary was dropped:\n{toml}"
        );
        assert!(toml.contains("input = 3"));
    }

    #[test]
    fn rows_that_price_nothing_are_not_emitted_as_models() {
        // Without the name-table filter, a non-pricing row in the table would otherwise
        // become a bogus model entry in the cache.
        let html = concat!(
            r#"<h2 id="pricing">Pricing</h2><table><thead><tr><th>Model</th></tr></thead>"#,
            r#"<tbody><tr><td>Coming soon</td><td>-</td><td>-</td><td>-</td><td>-</td></tr>"#,
            r#"<tr><td>GLM 5.2</td><td>$1.40</td><td>$4.40</td><td>$0.26</td><td>-</td></tr></tbody></table>"#
        );
        let toml = parse_pricing_html(html).unwrap();
        assert!(!toml.contains("coming-soon"), "{toml}");
        assert!(toml.contains(r#"[model."glm-5.2"]"#), "{toml}");
    }

    #[test]
    fn every_id_the_fixture_yields_is_spelled_as_the_bundled_table_keys_it() {
        // The refreshed cache overlays the bundled table. An id spelled differently from the
        // bundled key does not correct that model's price — it adds a second, unreachable
        // entry and leaves the real one stale. This is the Claude Opus dash/dot bug's class.
        let html = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/zen_pricing.html"
        ))
        .expect("fixture");
        let toml = parse_pricing_html(&html).expect("fixture parses");
        let scraped: std::collections::HashSet<String> = toml
            .lines()
            .filter_map(|line| line.strip_prefix("[model.\""))
            .filter_map(|rest| rest.split('"').next())
            .map(|id| id.to_string())
            .collect();

        let bundled = crate::pricing::PricingEngine::bundled();
        let missing: Vec<&str> = PREVIOUSLY_HARDCODED_IDS
            .iter()
            .filter(|id| !scraped.contains(**id))
            .copied()
            .collect();
        assert!(
            missing.is_empty(),
            "the fixture no longer yields these ids: {missing:?}"
        );
        for id in PREVIOUSLY_HARDCODED_IDS {
            assert!(
                bundled.has_model(id),
                "scraper emits {id}, which the bundled table does not key"
            );
        }
    }

    #[test]
    fn claude_opus_is_still_priced_after_a_refresh_cycle() {
        use crate::model::Usage;
        use crate::pricing::PricingEngine;

        let html = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/zen_pricing.html"
        ))
        .expect("missing fixture tests/fixtures/zen_pricing.html");
        let refreshed = parse_pricing_html(&html).expect("fixture should parse");

        let engine = PricingEngine::parse(&refreshed).expect("refreshed table should be valid");

        for model in [
            "claude-opus-4.5",
            "claude-opus-4.6",
            "claude-opus-4.7",
            "claude-opus-4.8",
            "claude-sonnet-4.6",
            "claude-sonnet-5",
        ] {
            let usage = Usage {
                model: model.into(),
                input: 1_000_000,
                ..Default::default()
            };
            let priced = engine.estimate_cost(&usage);
            assert!(
                priced.is_some_and(|(cost, _)| cost > 0.0),
                "{} lost its pricing after a refresh cycle",
                model
            );
        }
    }
}
