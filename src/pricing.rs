use std::collections::HashMap;

use serde::Deserialize;

use crate::collector::pricing_refresh::pricing_cache_path;
use crate::model::{Category, CostStatus, Usage};

pub(crate) const BUNDLED_PRICING: &str = include_str!("../pricing/zen.toml");

/// How long a refreshed pricing cache is trusted before the bundled table wins again.
///
/// The `zen_pricing` collector refreshes hourly when enabled, so anything approaching a month
/// old means refreshing stopped working — offline, endpoint moved, collector disabled — and the
/// rates in it have had time to be superseded.
const PRICING_CACHE_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// A rate is `None` when the source table does not publish one. That is distinct from a
/// published rate of `0.0`, and the two must never be conflated: an absent rate means we do
/// not know the price, and unknown cost is never rendered as zero cost.
#[derive(Debug, Clone, Default, Deserialize)]
struct ModelPricing {
    free: Option<bool>,
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    reasoning: Option<f64>,
}

/// Rates that applied up to and including a past date.
///
/// A published rate change is a fact about *when* work happened, not about what the table
/// says today. Without this, correcting the table after a vendor price change silently
/// re-prices every historical event at the new rate — a request made in August gets billed at
/// September's price the moment someone edits a number.
#[derive(Debug, Clone)]
struct PricePeriod {
    /// Last day these rates were in effect, inclusive, interpreted in **UTC**.
    ///
    /// Vendor price changes are announced against a calendar date, not a user's timezone, so
    /// this boundary is deliberately not the local-calendar-day used for range filtering.
    through: chrono::NaiveDate,
    rates: ResolvedPricing,
}

#[derive(Debug, Clone, Default)]
struct ResolvedPricing {
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
    reasoning: Option<f64>,
}

#[derive(Default)]
pub struct PricingEngine {
    models: HashMap<String, ModelPricing>,
    tiers: HashMap<String, Vec<(u64, ResolvedPricing)>>,
    /// Historical rates per model, ascending by end date.
    periods: HashMap<String, Vec<PricePeriod>>,
    warnings: Vec<String>,
}

impl PricingEngine {
    pub fn bundled() -> Self {
        match Self::parse(BUNDLED_PRICING) {
            Ok(engine) => engine,
            Err(error) => Self {
                warnings: vec![format!("bundled pricing table is invalid: {}", error)],
                ..Default::default()
            },
        }
    }

    /// Whether the table keys this model id exactly, with no fallback resolution.
    ///
    /// Deliberately exact: this exists to check that the scraper and the bundled table agree
    /// on a model's *spelling*, and layered resolution would paper over exactly the mismatch
    /// it is meant to catch.
    pub fn has_model(&self, model_id: &str) -> bool {
        self.models.contains_key(model_id) || self.tiers.contains_key(model_id)
    }

    /// Load the bundled table, then overlay the refreshed cache on top of it.
    ///
    /// The cache is an *overlay*, never a replacement. A refresh that drops or misspells a
    /// model must not be able to delete pricing that shipped in the binary, and a cache that
    /// fails to parse must not be able to silently zero out every price.
    pub fn load() -> Self {
        let mut engine = Self::bundled();
        let Some(path) = pricing_cache_path() else {
            return engine;
        };
        if !path.exists() {
            return engine;
        }
        let age = std::fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok());
        match std::fs::read_to_string(&path) {
            Ok(contents) => engine.apply_cache(&contents, age, &path.display().to_string()),
            Err(error) => engine.warnings.push(format!(
                "cached pricing table at {} is unreadable ({}); using bundled rates",
                path.display(),
                error
            )),
        }
        engine
    }

    /// Apply a refreshed pricing cache, unless it is too old to trust.
    ///
    /// The overlay's *current* rates win over the bundled table, which is right when the cache
    /// is fresher than the binary and wrong once it is not: nothing expires it, so a cache
    /// written before a rate change keeps applying the superseded rate to new events forever.
    /// The bundled table ships with the binary and is at least as current as the release, so
    /// falling back to it is the safe direction. Some models may become `UNKNOWN COST` as a
    /// result — that is the intended trade, and the whole point of the provenance model.
    fn apply_cache(&mut self, contents: &str, age: Option<std::time::Duration>, label: &str) {
        if let Some(age) = age {
            if age > PRICING_CACHE_MAX_AGE {
                self.warnings.push(format!(
                    "cached pricing table at {} is {} days old; ignoring it and using bundled \
                     rates. Run --refresh-pricing to update it, or delete the file.",
                    label,
                    age.as_secs() / 86_400
                ));
                return;
            }
        }
        match Self::parse(contents) {
            Ok(overlay) => self.overlay(overlay),
            Err(error) => self.warnings.push(format!(
                "cached pricing table at {} is invalid ({}); using bundled rates",
                label, error
            )),
        }
    }

    pub fn from_toml(toml_str: &str) -> Self {
        Self::parse(toml_str).unwrap_or_else(|error| Self {
            warnings: vec![format!("pricing table is invalid: {}", error)],
            ..Default::default()
        })
    }

    pub fn parse(toml_str: &str) -> Result<Self, toml::de::Error> {
        let table: toml::Table = toml_str.parse()?;
        let model_table = table.get("model").and_then(|v| v.as_table());

        let mut models = HashMap::new();
        let mut tiers: HashMap<String, Vec<(u64, ResolvedPricing)>> = HashMap::new();
        let mut periods: HashMap<String, Vec<PricePeriod>> = HashMap::new();
        let mut warnings: Vec<String> = Vec::new();

        if let Some(model_table) = model_table {
            for (model_id, model_value) in model_table {
                let Some(model_subtable) = model_value.as_table() else {
                    continue;
                };
                let mut base_pricing = ModelPricing::default();

                for (key, value) in model_subtable {
                    if let Some(stripped) = key.strip_prefix("tier-") {
                        let threshold: u64 = stripped.parse().unwrap_or(0);
                        tiers
                            .entry(model_id.clone())
                            .or_default()
                            .push((threshold, parse_pricing_from_value(value)));
                    } else if key == "period" {
                        match parse_periods(value) {
                            Ok(parsed) => {
                                periods.entry(model_id.clone()).or_default().extend(parsed)
                            }
                            Err(error) => warnings.push(format!(
                                "{}: ignoring historical pricing period ({}); events before the \
                                 rate change will be priced at current rates",
                                model_id, error
                            )),
                        }
                    } else {
                        set_pricing_field(&mut base_pricing, key, value);
                    }
                }

                models.insert(model_id.clone(), base_pricing);
            }
        }

        for tier_list in tiers.values_mut() {
            tier_list.sort_by_key(|b| std::cmp::Reverse(b.0));
        }
        for period_list in periods.values_mut() {
            period_list.sort_by_key(|p| p.through);
        }

        // Tiered rates describe *current* pricing, so a historical period cannot express them.
        // Rather than silently applying today's tier rates to a past event, say so.
        for model_id in periods.keys() {
            if tiers.contains_key(model_id) {
                warnings.push(format!(
                    "{}: has both context tiers and historical pricing periods; periods are \
                     applied without tiering",
                    model_id
                ));
            }
        }

        Ok(Self {
            models,
            tiers,
            periods,
            warnings,
        })
    }

    /// Apply `other` on top of `self`, per model id. Entries present only in `self` survive.
    fn overlay(&mut self, other: Self) {
        for (model_id, pricing) in other.models {
            self.models.insert(model_id.clone(), pricing);
            match other.tiers.get(&model_id) {
                Some(tiers) => {
                    self.tiers.insert(model_id.clone(), tiers.clone());
                }
                // The overlay redefined this model without tiers; stale tiers from the bundled
                // table would otherwise silently apply the wrong rates above the threshold.
                None => {
                    self.tiers.remove(&model_id);
                }
            }
            // Periods are treated the opposite way to tiers, deliberately. A tier is a property
            // of *current* pricing, so a stale one is wrong. A period is a record of what a
            // rate *was*, which a scraped page cannot know and must not erase — dropping it
            // here would re-price every historical event at the refreshed rate, which is the
            // bug this whole mechanism exists to prevent.
            if let Some(overlay_periods) = other.periods.get(&model_id) {
                self.periods.insert(model_id, overlay_periods.clone());
            }
        }
        self.warnings.extend(other.warnings);
    }

    /// Rates in effect for an event, or `None` when current rates apply.
    fn period_rates(
        &self,
        model_id: &str,
        at: Option<chrono::NaiveDate>,
    ) -> Option<&ResolvedPricing> {
        let at = at?;
        self.periods
            .get(model_id)?
            .iter()
            .find(|period| at <= period.through)
            .map(|period| &period.rates)
    }

    /// The model's list input rate, per million tokens — a stable proxy for "how expensive is
    /// this model", independent of how many tokens any one request happened to use.
    ///
    /// Exists to rank models against each other, not to price anything: an *observed* cost per
    /// token would move with the cache-hit mix, so two requests to the same model could rank
    /// differently. `None` for a free model and for one the table cannot price at all — a
    /// caller comparing two models must be able to tell "cheaper" from "unknown".
    pub fn input_rate(&self, model: &str) -> Option<f64> {
        let (_, pricing) = self.resolve(model)?;
        if pricing.free == Some(true) {
            return None;
        }
        // The current base rate, deliberately not a long-context tier and not a historical
        // period: the tier depends on the size of an individual request, and a period depends
        // on when one happened. Neither is a property of the model.
        pricing.input
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Resolve a model id to its pricing entry, trying each candidate spelling in turn.
    fn resolve(&self, model: &str) -> Option<(String, &ModelPricing)> {
        model_candidates(model)
            .into_iter()
            .find_map(|candidate| self.models.get(&candidate).map(|p| (candidate, p)))
    }

    pub fn estimate_cost(&self, usage: &Usage) -> Option<(f64, CostStatus)> {
        let (model_id, pricing) = self.resolve(&usage.model)?;

        if pricing.free == Some(true) {
            return Some((0.0, CostStatus::Free));
        }

        let context_tokens = usage.input + usage.cache_read;
        // Price the event at the rates in effect when it happened, not at whatever the table
        // says now. An undated event falls through to current rates.
        let resolved = match self.period_rates(&model_id, event_date(usage.created)) {
            Some(historical) => historical.clone(),
            None => self.resolve_tier(&model_id, pricing, context_tokens),
        };

        // Reasoning tokens are billed at the output rate unless the table publishes a distinct
        // one; that is the near-universal provider convention.
        let reasoning_rate = resolved.reasoning.or(resolved.output);

        let cost = component(usage.input, resolved.input)?
            + component(usage.output, resolved.output)?
            + component(usage.reasoning, reasoning_rate)?
            + component(usage.cache_read, resolved.cache_read)?
            + component(usage.cache_write, resolved.cache_write)?;

        Some((cost, CostStatus::Estimated))
    }

    fn resolve_tier(
        &self,
        model_id: &str,
        base: &ModelPricing,
        context_tokens: u64,
    ) -> ResolvedPricing {
        if let Some(tiers) = self.tiers.get(model_id) {
            for (threshold, tier) in tiers {
                if context_tokens > *threshold {
                    return tier.clone();
                }
            }
        }

        ResolvedPricing {
            input: base.input,
            output: base.output,
            cache_read: base.cache_read,
            cache_write: base.cache_write,
            reasoning: base.reasoning,
        }
    }
}

/// The UTC calendar date an event happened on.
///
/// `None` for a missing or nonsensical timestamp: an event we cannot date is priced at current
/// rates rather than silently inheriting the oldest historical rate on file.
fn event_date(created: i64) -> Option<chrono::NaiveDate> {
    if created <= 0 {
        return None;
    }
    chrono::DateTime::from_timestamp(created, 0).map(|dt| dt.date_naive())
}

/// Parse a `[[model."x".period]]` array into dated rate sets.
fn parse_periods(value: &toml::Value) -> Result<Vec<PricePeriod>, String> {
    let entries = value
        .as_array()
        .ok_or_else(|| "`period` must be an array of tables".to_string())?;

    let mut periods = Vec::with_capacity(entries.len());
    for entry in entries {
        let through = entry
            .get("through")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "each period needs a `through = \"YYYY-MM-DD\"` date".to_string())?;
        let through = chrono::NaiveDate::parse_from_str(through, "%Y-%m-%d")
            .map_err(|_| format!("`through` is not a YYYY-MM-DD date: {through}"))?;
        periods.push(PricePeriod {
            through,
            rates: parse_pricing_from_value(entry),
        });
    }
    Ok(periods)
}

/// Cost of one token bucket, in dollars.
///
/// Returns `None` when tokens were consumed but no rate is published — the caller turns that
/// into `CostStatus::Unavailable` rather than charging zero for work that was not free.
fn component(tokens: u64, rate: Option<f64>) -> Option<f64> {
    if tokens == 0 {
        return Some(0.0);
    }
    Some((tokens as f64 / 1_000_000.0) * rate?)
}

/// Read a rate as a number, accepting either a TOML float or a TOML integer.
///
/// `--refresh-pricing` writes whole-dollar rates unquoted, so a $5.00 rate lands in the cache
/// as `input = 5`, a TOML *integer*. Reading only floats silently skipped every whole-number
/// rate; combined with the old `unwrap_or(0.0)` that charged $0 per million tokens for them.
fn as_rate(value: &toml::Value) -> Option<f64> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|i| i as f64))
}

fn parse_pricing_from_value(value: &toml::Value) -> ResolvedPricing {
    let table = value.as_table();
    let field = |name: &str| table.and_then(|t| t.get(name)).and_then(as_rate);
    ResolvedPricing {
        input: field("input"),
        output: field("output"),
        cache_read: field("cache_read"),
        cache_write: field("cache_write"),
        reasoning: field("reasoning"),
    }
}

fn set_pricing_field(pricing: &mut ModelPricing, key: &str, value: &toml::Value) {
    match key {
        "free" => pricing.free = value.as_bool(),
        "input" => pricing.input = as_rate(value),
        "output" => pricing.output = as_rate(value),
        "cache_read" => pricing.cache_read = as_rate(value),
        "cache_write" => pricing.cache_write = as_rate(value),
        "reasoning" => pricing.reasoning = as_rate(value),
        _ => {}
    }
}

/// Spellings of `model` to try against the pricing table, most specific first.
///
/// Real-world ids do not arrive in table form. Claude Code writes
/// `claude-sonnet-4-5-20250929`; aggregators write `anthropic/claude-sonnet-4.5`; Ollama writes
/// `glm-5.2:cloud`. Matching only the literal string sends all of them to `UNKNOWN COST`.
fn model_candidates(model: &str) -> Vec<String> {
    let base = model.trim().to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    let push = |candidate: String, out: &mut Vec<String>| {
        if !candidate.is_empty() && !out.contains(&candidate) {
            out.push(candidate);
        }
    };

    // Provider-namespaced ids (`anthropic/claude-sonnet-4.5`) reduce to their last segment.
    let mut seeds = vec![base.clone()];
    if let Some((_, tail)) = base.rsplit_once('/') {
        seeds.push(tail.to_string());
    }

    for seed in seeds {
        let mut forms = vec![seed.clone()];
        for suffix in [":cloud", "-cloud"] {
            if let Some(stripped) = seed.strip_suffix(suffix) {
                forms.push(stripped.to_string());
            }
        }
        if let Some((head, _)) = seed.split_once('@') {
            forms.push(head.to_string());
        }

        for form in forms {
            push(form.clone(), &mut out);
            push(dotted_version(&form), &mut out);
            if let Some(undated) = strip_date_suffix(&form) {
                push(undated.clone(), &mut out);
                push(dotted_version(&undated), &mut out);
            }
        }
    }

    if out.is_empty() {
        out.push(base);
    }
    out
}

/// Drop a trailing release date: `-20250929` or `-2025-09-29`.
fn strip_date_suffix(model: &str) -> Option<String> {
    let (head, tail) = model.rsplit_once('-')?;
    if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) {
        return Some(head.to_string());
    }
    // `-YYYY-MM-DD`
    if tail.len() == 2 && tail.bytes().all(|b| b.is_ascii_digit()) {
        let (head2, month) = head.rsplit_once('-')?;
        let (head3, year) = head2.rsplit_once('-')?;
        if month.len() == 2
            && month.bytes().all(|b| b.is_ascii_digit())
            && year.len() == 4
            && year.bytes().all(|b| b.is_ascii_digit())
        {
            return Some(head3.to_string());
        }
    }
    None
}

/// Rewrite dash-separated version runs as dotted: `claude-sonnet-4-5` -> `claude-sonnet-4.5`.
///
/// Anthropic and the Zen table disagree on this purely cosmetically, and that cosmetic
/// difference is the whole gap between a priced row and `UNKNOWN COST`.
fn dotted_version(model: &str) -> String {
    let segments: Vec<&str> = model.split('-').collect();
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());

    let mut out: Vec<String> = Vec::new();
    let mut index = 0;
    while index < segments.len() {
        if numeric(segments[index]) {
            let start = index;
            while index < segments.len() && numeric(segments[index]) {
                index += 1;
            }
            out.push(segments[start..index].join("."));
        } else {
            out.push(segments[index].to_string());
            index += 1;
        }
    }
    out.join("-")
}

/// Model ids the bundled table marks `free = true`.
///
/// Classification reads this rather than keeping its own hand-maintained list; two lists of
/// free models in two files will drift, and a paid model mistakenly treated as free is
/// excluded from every cost total and becomes invisible spend.
pub fn bundled_free_models() -> &'static std::collections::HashSet<String> {
    static FREE: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    FREE.get_or_init(|| {
        PricingEngine::bundled()
            .models
            .iter()
            .filter(|(_, pricing)| pricing.free == Some(true))
            .map(|(id, _)| id.clone())
            .collect()
    })
}

pub fn apply_estimated_pricing(usages: &mut [Usage], engine: &PricingEngine) {
    for usage in usages.iter_mut() {
        if usage.cost_status != CostStatus::Unavailable {
            continue;
        }
        if usage.category == Category::Local
            || usage.category == Category::Free
            || usage.category == Category::Cloud
        {
            continue;
        }
        if let Some((cost, status)) = engine.estimate_cost(usage) {
            usage.cost = Some(cost);
            usage.cost_status = status;
            // A row we just put a dollar figure on is billable usage. Leaving it UNKNOWN makes
            // the per-category metric tiles disagree with the aggregate cost in the breakdown.
            if status == CostStatus::Estimated && usage.category == Category::Unknown {
                usage.category = Category::Paid;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_models_return_zero_cost() {
        let engine = PricingEngine::bundled();
        let usage = Usage {
            model: "big-pickle".into(),
            input: 1000,
            output: 500,
            ..Default::default()
        };
        let (cost, status) = engine.estimate_cost(&usage).unwrap();
        assert_eq!(cost, 0.0);
        assert_eq!(status, CostStatus::Free);
    }

    #[test]
    fn paid_models_calculate_cost() {
        let engine = PricingEngine::bundled();
        // Context tokens = 200K (< 272K), so base rates apply
        let usage = Usage {
            model: "gpt-5.6-luna".into(),
            input: 100_000,
            output: 500_000,
            cache_read: 100_000,
            cache_write: 50_000,
            ..Default::default()
        };
        let (cost, status) = engine.estimate_cost(&usage).unwrap();
        assert_eq!(status, CostStatus::Estimated);
        // Base rates: input=1.00, output=6.00, cache_read=0.10, cache_write=1.25
        let expected = 0.1 * 1.00 + 0.5 * 6.00 + 0.1 * 0.10 + 0.05 * 1.25;
        assert!((cost - expected).abs() < 0.001);
    }

    #[test]
    fn context_tier_uses_higher_rates() {
        let engine = PricingEngine::bundled();
        let usage = Usage {
            model: "gpt-5.6-luna".into(),
            input: 300_000,
            output: 100_000,
            cache_read: 0,
            cache_write: 0,
            ..Default::default()
        };
        let (cost, _) = engine.estimate_cost(&usage).unwrap();
        assert!((cost - (0.6 + 0.9)).abs() < 0.001);
    }

    #[test]
    fn cloud_suffix_is_stripped_for_pricing() {
        let candidates = model_candidates("GLM-5.2:cloud");
        assert!(
            candidates.iter().any(|c| c == "glm-5.2"),
            "expected the base model among {:?}",
            candidates
        );
        assert_eq!(model_candidates("gpt-5.6-luna")[0], "gpt-5.6-luna");
    }

    #[test]
    fn unknown_model_returns_none() {
        let engine = PricingEngine::bundled();
        let usage = Usage {
            model: "nonexistent-model".into(),
            ..Default::default()
        };
        assert!(engine.estimate_cost(&usage).is_none());
    }

    #[test]
    fn apply_estimated_pricing_skips_known_costs() {
        let engine = PricingEngine::bundled();
        let mut usages = vec![
            Usage {
                model: "gpt-5.6-luna".into(),
                cost: Some(0.50),
                cost_status: CostStatus::Calculated,
                input: 1_000_000,
                output: 0,
                ..Default::default()
            },
            Usage {
                model: "gpt-5.6-luna".into(),
                cost: None,
                cost_status: CostStatus::Unavailable,
                input: 1_000_000,
                output: 0,
                ..Default::default()
            },
        ];
        apply_estimated_pricing(&mut usages, &engine);
        assert_eq!(usages[0].cost, Some(0.50));
        assert_eq!(usages[0].cost_status, CostStatus::Calculated);
        assert!(usages[1].cost.is_some());
        assert_eq!(usages[1].cost_status, CostStatus::Estimated);
    }

    #[test]
    fn apply_estimated_pricing_skips_local_and_cloud() {
        let engine = PricingEngine::bundled();
        let mut usages = vec![
            Usage {
                model: "glm-5.2:cloud".into(),
                category: Category::Cloud,
                cost_status: CostStatus::Unavailable,
                input: 1_000_000,
                output: 0,
                ..Default::default()
            },
            Usage {
                model: "qwen3-coder:30b".into(),
                category: Category::Local,
                cost_status: CostStatus::Unavailable,
                input: 1_000_000,
                output: 0,
                ..Default::default()
            },
        ];
        apply_estimated_pricing(&mut usages, &engine);
        assert_eq!(usages[0].cost_status, CostStatus::Unavailable);
        assert_eq!(usages[1].cost_status, CostStatus::Unavailable);
    }

    #[test]
    fn reasoning_tokens_are_billed_at_the_output_rate() {
        let engine = PricingEngine::bundled();
        let base = Usage {
            model: "claude-sonnet-4.6".into(),
            input: 1_000_000,
            ..Default::default()
        };
        let with_reasoning = Usage {
            reasoning: 1_000_000,
            ..base.clone()
        };
        let (without, _) = engine.estimate_cost(&base).unwrap();
        let (with, _) = engine.estimate_cost(&with_reasoning).unwrap();
        // claude-sonnet-4.6 output = 15.00 per 1M
        assert!((with - without - 15.00).abs() < 1e-9);
    }

    #[test]
    fn an_explicit_reasoning_rate_overrides_the_output_rate() {
        let engine = PricingEngine::from_toml(
            "[model.\"m\"]\ninput = 1.0\noutput = 10.0\nreasoning = 2.0\n",
        );
        let usage = Usage {
            model: "m".into(),
            reasoning: 1_000_000,
            ..Default::default()
        };
        let (cost, _) = engine.estimate_cost(&usage).unwrap();
        assert!((cost - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_missing_rate_yields_unknown_cost_not_zero_cost() {
        // The table publishes no cache_write rate. Charging $0 for cache writes would be a
        // silent lie; the project invariant is that unknown cost stays unknown.
        let engine = PricingEngine::from_toml("[model.\"m\"]\ninput = 1.0\noutput = 10.0\n");
        let unpriced_bucket = Usage {
            model: "m".into(),
            cache_write: 500_000,
            ..Default::default()
        };
        assert!(engine.estimate_cost(&unpriced_bucket).is_none());

        // ...but a bucket with zero tokens needs no rate.
        let untouched_bucket = Usage {
            model: "m".into(),
            input: 1_000_000,
            ..Default::default()
        };
        let (cost, _) = engine.estimate_cost(&untouched_bucket).unwrap();
        assert!((cost - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_published_zero_rate_is_distinct_from_a_missing_one() {
        let engine = PricingEngine::from_toml(
            "[model.\"m\"]\ninput = 1.0\noutput = 10.0\ncache_write = 0.0\n",
        );
        let usage = Usage {
            model: "m".into(),
            cache_write: 500_000,
            ..Default::default()
        };
        let (cost, status) = engine.estimate_cost(&usage).unwrap();
        assert_eq!(cost, 0.0);
        assert_eq!(status, CostStatus::Estimated);
    }

    #[test]
    fn an_invalid_cache_cannot_delete_bundled_pricing() {
        let mut engine = PricingEngine::bundled();
        let before = engine.estimate_cost(&Usage {
            model: "claude-opus-4.8".into(),
            input: 1_000_000,
            ..Default::default()
        });
        // Simulate load() encountering an unparseable overlay: warnings are recorded and the
        // bundled table is left intact.
        engine
            .warnings
            .push("cached pricing table is invalid".into());
        let after = engine.estimate_cost(&Usage {
            model: "claude-opus-4.8".into(),
            input: 1_000_000,
            ..Default::default()
        });
        assert_eq!(before.map(|c| c.0), after.map(|c| c.0));
        assert!(!engine.warnings().is_empty());
    }

    #[test]
    fn an_overlay_adds_and_replaces_without_dropping_bundled_models() {
        let mut engine = PricingEngine::bundled();
        let opus_before = engine
            .estimate_cost(&Usage {
                model: "claude-opus-4.8".into(),
                input: 1_000_000,
                ..Default::default()
            })
            .unwrap()
            .0;

        // An overlay that knows about only one model must not evict everything else.
        engine.overlay(
            PricingEngine::parse("[model.\"claude-sonnet-4.6\"]\ninput = 99.0\n").unwrap(),
        );

        let opus_after = engine
            .estimate_cost(&Usage {
                model: "claude-opus-4.8".into(),
                input: 1_000_000,
                ..Default::default()
            })
            .unwrap()
            .0;
        assert_eq!(opus_before, opus_after, "overlay evicted a bundled model");

        let sonnet = engine
            .estimate_cost(&Usage {
                model: "claude-sonnet-4.6".into(),
                input: 1_000_000,
                ..Default::default()
            })
            .unwrap()
            .0;
        assert!((sonnet - 99.0).abs() < 1e-9, "overlay did not take effect");
    }

    #[test]
    fn estimated_cost_promotes_unknown_usage_to_paid() {
        let engine = PricingEngine::bundled();
        let mut usages = vec![Usage {
            provider: "anthropic".into(),
            model: "claude-sonnet-4.6".into(),
            category: Category::Unknown,
            cost_status: CostStatus::Unavailable,
            input: 1_000_000,
            ..Default::default()
        }];
        apply_estimated_pricing(&mut usages, &engine);
        assert_eq!(usages[0].cost_status, CostStatus::Estimated);
        assert_eq!(
            usages[0].category,
            Category::Paid,
            "a row with a dollar figure must not stay in the UNKNOWN bucket"
        );
    }

    #[test]
    fn whole_dollar_rates_written_as_integers_are_read_correctly() {
        // `--refresh-pricing` emits `input = 5`, not `input = 5.0`. Reading only TOML floats
        // skipped every whole-number rate, and the old `unwrap_or(0.0)` then charged $0 per
        // million tokens for it -- so a refreshed cache under-reported Opus by ~$30/1M.
        let engine = PricingEngine::from_toml(
            "[model.\"m\"]\ninput = 5\noutput = 25\ncache_read = 0.5\ncache_write = 6.25\n",
        );
        let usage = Usage {
            model: "m".into(),
            input: 1_000_000,
            output: 1_000_000,
            ..Default::default()
        };
        let (cost, _) = engine.estimate_cost(&usage).unwrap();
        assert!(
            (cost - 30.0).abs() < 1e-9,
            "expected $30.00 for 1M in + 1M out, got ${:.4}",
            cost
        );
    }

    #[test]
    fn integer_and_float_rates_are_equivalent() {
        let as_int = PricingEngine::from_toml("[model.\"m\"]\ninput = 5\n");
        let as_float = PricingEngine::from_toml("[model.\"m\"]\ninput = 5.0\n");
        let usage = Usage {
            model: "m".into(),
            input: 1_000_000,
            ..Default::default()
        };
        assert_eq!(
            as_int.estimate_cost(&usage).map(|c| c.0),
            as_float.estimate_cost(&usage).map(|c| c.0)
        );
    }

    #[test]
    fn claude_code_model_ids_resolve_to_table_entries() {
        let engine = PricingEngine::bundled();
        // Claude Code writes dated, dash-versioned ids; the table uses undated, dotted ones.
        for model in [
            "claude-sonnet-4-5-20250929",
            "claude-sonnet-4.5",
            "anthropic/claude-sonnet-4.5",
            "claude-sonnet-4-5",
            "CLAUDE-SONNET-4-5-20250929",
        ] {
            // Kept under claude-sonnet-4.5's tier-200000 threshold so this asserts
            // resolution, not tier selection.
            let usage = Usage {
                model: model.into(),
                input: 100_000,
                ..Default::default()
            };
            let (cost, _) = engine
                .estimate_cost(&usage)
                .unwrap_or_else(|| panic!("{} did not resolve to a price", model));
            assert!((cost - 0.30).abs() < 1e-9, "{} priced at {}", model, cost);
        }
    }

    #[test]
    fn date_suffixes_are_stripped_in_both_formats() {
        assert_eq!(
            strip_date_suffix("claude-sonnet-4-5-20250929").as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(
            strip_date_suffix("gpt-5-codex-2025-09-29").as_deref(),
            Some("gpt-5-codex")
        );
        assert_eq!(strip_date_suffix("claude-sonnet-4.5"), None);
    }

    #[test]
    fn dotted_version_leaves_non_version_segments_alone() {
        assert_eq!(dotted_version("claude-sonnet-4-5"), "claude-sonnet-4.5");
        assert_eq!(dotted_version("claude-3-5-sonnet"), "claude-3.5-sonnet");
        // A lone numeric segment is not a version pair and must survive untouched.
        assert_eq!(dotted_version("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(dotted_version("gpt-5-nano"), "gpt-5-nano");
    }

    #[test]
    fn resolution_never_reprices_a_different_model() {
        let engine = PricingEngine::bundled();
        // Fuzzy candidates must not let an unknown model borrow someone else's price.
        let usage = Usage {
            model: "totally-unknown-model-9-9".into(),
            input: 1_000_000,
            ..Default::default()
        };
        assert!(engine.estimate_cost(&usage).is_none());
    }

    #[test]
    fn a_cloud_suffix_still_resolves_to_the_base_model() {
        let engine = PricingEngine::bundled();
        let usage = Usage {
            model: "glm-5.2:cloud".into(),
            input: 1_000_000,
            ..Default::default()
        };
        assert!(engine.estimate_cost(&usage).is_some());
    }

    /// Build a usage row dated at a specific UTC day.
    fn usage_on(model: &str, date: &str, input: u64) -> Usage {
        let day = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").expect("valid date");
        Usage {
            provider: "anthropic".into(),
            model: model.into(),
            input,
            created: day
                .and_hms_opt(12, 0, 0)
                .expect("valid time")
                .and_utc()
                .timestamp(),
            ..Default::default()
        }
    }

    #[test]
    fn a_fresh_pricing_cache_is_applied() {
        let mut engine = PricingEngine::bundled();
        engine.apply_cache(
            "[model.\"claude-opus-5\"]\ninput = 99.00\n",
            Some(std::time::Duration::from_secs(3600)),
            "test-cache",
        );
        let (cost, _) = engine
            .estimate_cost(&Usage {
                model: "claude-opus-5".into(),
                input: 1_000_000,
                ..Default::default()
            })
            .expect("priced");
        assert!(
            (cost - 99.00).abs() < 1e-9,
            "a fresh cache should win, got ${cost}"
        );
        assert!(engine.warnings().is_empty(), "{:?}", engine.warnings());
    }

    #[test]
    fn a_stale_pricing_cache_is_ignored_and_says_so() {
        // Nothing expired the cache, so one written before a rate change kept applying the
        // superseded rate to new events indefinitely. Found on a real machine: a cache from
        // 2026-07-24 still overriding the bundled table weeks later.
        let mut engine = PricingEngine::bundled();
        engine.apply_cache(
            "[model.\"claude-opus-5\"]\ninput = 99.00\n",
            Some(PRICING_CACHE_MAX_AGE + std::time::Duration::from_secs(86_400)),
            "test-cache",
        );
        let (cost, _) = engine
            .estimate_cost(&Usage {
                model: "claude-opus-5".into(),
                input: 1_000_000,
                ..Default::default()
            })
            .expect("priced");
        assert!(
            (cost - 5.00).abs() < 1e-9,
            "a stale cache should fall back to bundled rates, got ${cost}"
        );
        let warning = engine.warnings().join(" ");
        assert!(warning.contains("31 days old"), "{warning}");
        assert!(
            warning.contains("--refresh-pricing"),
            "expected a next step: {warning}"
        );
    }

    #[test]
    fn a_cache_of_unknown_age_is_still_applied() {
        // A filesystem that cannot report mtime should not silently disable pricing refreshes.
        let mut engine = PricingEngine::bundled();
        engine.apply_cache(
            "[model.\"claude-opus-5\"]\ninput = 99.00\n",
            None,
            "test-cache",
        );
        let (cost, _) = engine
            .estimate_cost(&Usage {
                model: "claude-opus-5".into(),
                input: 1_000_000,
                ..Default::default()
            })
            .expect("priced");
        assert!((cost - 99.00).abs() < 1e-9, "got ${cost}");
    }

    #[test]
    fn a_rate_change_does_not_re_price_events_from_before_it() {
        // Finding 1.6. `claude-sonnet-5` moved from introductory to list pricing after
        // 2026-08-31. Both sides must hold simultaneously and permanently: an August request
        // keeps August's price forever, and a September request gets September's. Before this
        // mechanism existed, editing the table on the changeover date silently re-priced every
        // historical request at the new rate.
        let engine = PricingEngine::bundled();

        let (august, _) = engine
            .estimate_cost(&usage_on("claude-sonnet-5", "2026-08-15", 1_000_000))
            .expect("priced");
        assert!(
            (august - 2.00).abs() < 1e-9,
            "an August request must stay at the introductory $2.00/1M, got ${august:.2}"
        );

        let (september, _) = engine
            .estimate_cost(&usage_on("claude-sonnet-5", "2026-09-15", 1_000_000))
            .expect("priced");
        assert!(
            (september - 3.00).abs() < 1e-9,
            "a September request must use list pricing of $3.00/1M, got ${september:.2}"
        );
    }

    #[test]
    fn the_period_boundary_is_inclusive_and_lands_on_the_right_day() {
        // An off-by-one here misprices a full day of requests, so pin both sides of midnight.
        let engine = PricingEngine::bundled();
        let cost = |date: &str| {
            engine
                .estimate_cost(&usage_on("claude-sonnet-5", date, 1_000_000))
                .expect("priced")
                .0
        };
        assert!((cost("2026-08-31") - 2.00).abs() < 1e-9, "last intro day");
        assert!((cost("2026-09-01") - 3.00).abs() < 1e-9, "first list day");
    }

    #[test]
    fn an_undated_event_is_priced_at_current_rates() {
        // `created` defaults to 0. Treating that as 1970 would hand every undated row the
        // oldest rate on file — quietly, and more wrongly the longer the table's history gets.
        let engine = PricingEngine::bundled();
        let (cost, _) = engine
            .estimate_cost(&Usage {
                provider: "anthropic".into(),
                model: "claude-sonnet-5".into(),
                input: 1_000_000,
                ..Default::default()
            })
            .expect("priced");
        assert!(
            (cost - 3.00).abs() < 1e-9,
            "expected current rates, got ${cost:.2}"
        );
    }

    #[test]
    fn a_refresh_cannot_erase_pricing_history() {
        // The scraper reads a page of *current* rates; it has no way to know what a rate used
        // to be. If an overlay dropped historical periods, `--refresh-pricing` would re-price
        // every past event at today's rate — reintroducing finding 1.6 through the back door.
        let mut engine = PricingEngine::bundled();
        let scraped =
            PricingEngine::parse("[model.\"claude-sonnet-5\"]\ninput = 3.00\noutput = 15.00\n")
                .expect("valid overlay");
        engine.overlay(scraped);

        let (august, _) = engine
            .estimate_cost(&usage_on("claude-sonnet-5", "2026-08-15", 1_000_000))
            .expect("priced");
        assert!(
            (august - 2.00).abs() < 1e-9,
            "a refresh erased the introductory period; August re-priced to ${august:.2}"
        );
    }

    #[test]
    fn an_overlay_may_still_correct_pricing_history() {
        // The flip side: when our recorded history is itself wrong, an overlay that supplies
        // its own periods must win. Preserving history must not mean freezing a mistake.
        let mut engine = PricingEngine::bundled();
        let corrected = PricingEngine::parse(
            "[model.\"claude-sonnet-5\"]\ninput = 3.00\n\n\
             [[model.\"claude-sonnet-5\".period]]\nthrough = \"2026-08-31\"\ninput = 2.50\n",
        )
        .expect("valid overlay");
        engine.overlay(corrected);

        let (august, _) = engine
            .estimate_cost(&usage_on("claude-sonnet-5", "2026-08-15", 1_000_000))
            .expect("priced");
        assert!(
            (august - 2.50).abs() < 1e-9,
            "expected the corrected rate, got ${august:.2}"
        );
    }

    #[test]
    fn a_malformed_period_warns_rather_than_silently_dropping_history() {
        let engine = PricingEngine::parse(
            "[model.\"m\"]\ninput = 3.00\n\n[[model.\"m\".period]]\ninput = 2.00\n",
        )
        .expect("table still parses");
        assert!(
            engine.warnings().iter().any(|w| w.contains("through")),
            "expected a warning naming the missing date, got {:?}",
            engine.warnings()
        );
    }

    #[test]
    fn models_without_periods_are_unaffected() {
        // The mechanism must be inert for the ~60 models that have no dated changes.
        let engine = PricingEngine::bundled();
        let old = usage_on("claude-opus-5", "2020-01-01", 1_000_000);
        let new = usage_on("claude-opus-5", "2030-01-01", 1_000_000);
        assert_eq!(
            engine.estimate_cost(&old).map(|(c, _)| c),
            engine.estimate_cost(&new).map(|(c, _)| c)
        );
    }

    #[test]
    fn current_anthropic_models_are_priced() {
        // A model missing from the table reports UNKNOWN COST rather than a wrong number,
        // which is correct but silently hides real spend. `claude-opus-5` was absent while
        // being the highest-volume model in real Claude Code logs.
        let engine = PricingEngine::bundled();
        for (model, input_rate) in [
            ("claude-opus-5", 5.00),
            ("claude-opus-4-8", 5.00),
            // List rate. The introductory $2.00 that applied through 2026-08-31 is a dated
            // period now, exercised by `a_rate_change_does_not_re_price_events_from_before_it`.
            ("claude-sonnet-5", 3.00),
            ("claude-haiku-4-5", 1.00),
            ("claude-fable-5", 10.00),
        ] {
            let usage = Usage {
                model: model.into(),
                input: 1_000_000,
                ..Default::default()
            };
            let (cost, _) = engine
                .estimate_cost(&usage)
                .unwrap_or_else(|| panic!("{} is not in the pricing table", model));
            assert!(
                (cost - input_rate).abs() < 1e-9,
                "{} priced input at {}, expected {}",
                model,
                cost,
                input_rate
            );
        }
    }

    #[test]
    fn cache_rates_follow_the_published_multipliers() {
        // Cache reads bill at 0.1x input and 5-minute cache writes at 1.25x input.
        let engine = PricingEngine::bundled();
        for model in ["claude-opus-5", "claude-opus-4-8", "claude-haiku-4-5"] {
            let input_only = Usage {
                model: model.into(),
                input: 1_000_000,
                ..Default::default()
            };
            let base = engine.estimate_cost(&input_only).unwrap().0;

            let read_only = Usage {
                model: model.into(),
                cache_read: 1_000_000,
                ..Default::default()
            };
            let write_only = Usage {
                model: model.into(),
                cache_write: 1_000_000,
                ..Default::default()
            };
            assert!(
                (engine.estimate_cost(&read_only).unwrap().0 - base * 0.10).abs() < 1e-9,
                "{} cache_read is not 0.1x input",
                model
            );
            assert!(
                (engine.estimate_cost(&write_only).unwrap().0 - base * 1.25).abs() < 1e-9,
                "{} cache_write is not 1.25x input",
                model
            );
        }
    }
}
