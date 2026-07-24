use std::collections::HashMap;

use serde::Deserialize;

use crate::collector::pricing_refresh::pricing_cache_path;
use crate::model::{Category, CostStatus, Usage};

const BUNDLED_PRICING: &str = include_str!("../pricing/zen.toml");

#[derive(Debug, Clone, Default, Deserialize)]
struct ModelPricing {
    free: Option<bool>,
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_write: Option<f64>,
}

struct ResolvedPricing {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

pub struct PricingEngine {
    models: HashMap<String, ModelPricing>,
    tiers: HashMap<String, Vec<(u64, ResolvedPricing)>>,
}

impl PricingEngine {
    pub fn bundled() -> Self {
        Self::from_toml(BUNDLED_PRICING)
    }

    pub fn load() -> Self {
        if let Some(path) = pricing_cache_path() {
            if path.exists() {
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    return Self::from_toml(&contents);
                }
            }
        }
        Self::bundled()
    }

    pub fn from_toml(toml_str: &str) -> Self {
        let table: toml::Value =
            toml::from_str(toml_str).unwrap_or(toml::Value::Table(toml::value::Table::new()));
        let model_table = table.get("model").and_then(|v| v.as_table());

        let mut models = HashMap::new();
        let mut tiers: HashMap<String, Vec<(u64, ResolvedPricing)>> = HashMap::new();

        if let Some(model_table) = model_table {
            for (model_id, model_value) in model_table {
                if let Some(model_subtable) = model_value.as_table() {
                    let mut base_pricing = ModelPricing::default();

                    for (key, value) in model_subtable {
                        if let Some(stripped) = key.strip_prefix("tier-") {
                            let threshold: u64 = stripped.parse().unwrap_or(0);
                            let tier_pricing = parse_pricing_from_value(value);
                            tiers
                                .entry(model_id.clone())
                                .or_default()
                                .push((threshold, tier_pricing));
                        } else {
                            set_pricing_field(&mut base_pricing, key, value);
                        }
                    }

                    models.insert(model_id.clone(), base_pricing);
                }
            }
        }

        for tier_list in tiers.values_mut() {
            tier_list.sort_by_key(|b| std::cmp::Reverse(b.0));
        }

        Self { models, tiers }
    }

    pub fn estimate_cost(&self, usage: &Usage) -> Option<(f64, CostStatus)> {
        let model_id = normalize_model_id(&usage.model);
        let pricing = self.models.get(&model_id)?;

        if pricing.free == Some(true) {
            return Some((0.0, CostStatus::Free));
        }

        let context_tokens = usage.input + usage.cache_read;
        let resolved = self.resolve_tier(&model_id, pricing, context_tokens);

        let cost = (usage.input as f64 / 1_000_000.0) * resolved.input
            + (usage.output as f64 / 1_000_000.0) * resolved.output
            + (usage.cache_read as f64 / 1_000_000.0) * resolved.cache_read
            + (usage.cache_write as f64 / 1_000_000.0) * resolved.cache_write;

        Some((cost, CostStatus::Estimated))
    }

    fn resolve_tier(
        &self,
        model_id: &str,
        base: &ModelPricing,
        context_tokens: u64,
    ) -> ResolvedPricing {
        let base_pricing = ResolvedPricing {
            input: base.input.unwrap_or(0.0),
            output: base.output.unwrap_or(0.0),
            cache_read: base.cache_read.unwrap_or(0.0),
            cache_write: base.cache_write.unwrap_or(0.0),
        };

        if let Some(tiers) = self.tiers.get(model_id) {
            for (threshold, tier) in tiers {
                if context_tokens > *threshold {
                    return ResolvedPricing {
                        input: tier.input,
                        output: tier.output,
                        cache_read: tier.cache_read,
                        cache_write: tier.cache_write,
                    };
                }
            }
        }

        base_pricing
    }
}

fn parse_pricing_from_value(value: &toml::Value) -> ResolvedPricing {
    let table = value.as_table();
    ResolvedPricing {
        input: table
            .and_then(|t| t.get("input"))
            .and_then(|v| v.as_float())
            .unwrap_or(0.0),
        output: table
            .and_then(|t| t.get("output"))
            .and_then(|v| v.as_float())
            .unwrap_or(0.0),
        cache_read: table
            .and_then(|t| t.get("cache_read"))
            .and_then(|v| v.as_float())
            .unwrap_or(0.0),
        cache_write: table
            .and_then(|t| t.get("cache_write"))
            .and_then(|v| v.as_float())
            .unwrap_or(0.0),
    }
}

fn set_pricing_field(pricing: &mut ModelPricing, key: &str, value: &toml::Value) {
    match key {
        "free" => pricing.free = value.as_bool(),
        "input" => pricing.input = value.as_float(),
        "output" => pricing.output = value.as_float(),
        "cache_read" => pricing.cache_read = value.as_float(),
        "cache_write" => pricing.cache_write = value.as_float(),
        _ => {}
    }
}

fn normalize_model_id(model: &str) -> String {
    let lower = model.to_ascii_lowercase();
    if let Some(stripped) = lower.strip_suffix(":cloud") {
        stripped.to_string()
    } else if let Some(stripped) = lower.strip_suffix("-cloud") {
        stripped.to_string()
    } else {
        lower
    }
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
        assert_eq!(normalize_model_id("GLM-5.2:cloud"), "glm-5.2");
        assert_eq!(normalize_model_id("gpt-5.6-luna"), "gpt-5.6-luna");
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
}
