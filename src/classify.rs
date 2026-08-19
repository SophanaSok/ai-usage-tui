use crate::model::{Category, CostStatus};

/// Providers that bill directly and whose model ids we can resolve against the pricing table.
/// Without this, first-party usage (Anthropic, OpenAI, Google, ...) falls through to
/// `Unknown` and lands in the red UNKNOWN tile even after a cost has been estimated for it.
///
/// Deliberately excludes aggregators and resellers (OpenRouter, Bedrock, Azure) — they do bill,
/// but they namespace model ids (`anthropic/claude-sonnet-4.5`) in a form the pricing engine
/// cannot resolve yet, so calling them PAID would promise a cost figure we cannot produce.
/// Add them once provider-qualified model resolution lands.
const FIRST_PARTY_PAID_PROVIDERS: &[&str] = &[
    "anthropic",
    "openai",
    "google",
    "gemini",
    "mistral",
    "cohere",
    "deepseek",
    "xai",
    "groq",
];

const LOCAL_HOSTS: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "0.0.0.0",
    "::1",
    "ollama",
    "lmstudio",
    "llamacpp",
    "llama.cpp",
    "vllm",
    "local",
];

/// Split an identifier into lowercase alphanumeric tokens.
///
/// Matching on raw substrings misclassifies: `cloudflare` contains "cloud", and a model named
/// `freeform` contains "free" — and being wrongly marked free excludes it from every cost
/// total. Token matching keeps those apart.
fn tokens(value: &str) -> Vec<String> {
    value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

fn has_token(value: &str, needle: &str) -> bool {
    tokens(value).iter().any(|token| token == needle)
}

pub fn classify(provider: &str, model: &str) -> Category {
    let p = provider.to_ascii_lowercase();
    let m = model.to_ascii_lowercase();

    // Cloud routing is decided by an explicit suffix on the model or a `cloud` token on the
    // provider -- never by a bare substring, which would swallow `cloudflare`.
    if m.ends_with(":cloud") || m.ends_with("-cloud") || has_token(&p, "cloud") {
        return Category::Cloud;
    }

    if LOCAL_HOSTS
        .iter()
        .any(|host| has_token(&p, host) || has_token(&m, host) || p.contains(host))
    {
        return Category::Local;
    }

    if is_free_model(&m) {
        return Category::Free;
    }

    if FIRST_PARTY_PAID_PROVIDERS
        .iter()
        .any(|known| has_token(&p, known))
    {
        return Category::Paid;
    }

    Category::Unknown
}

fn is_free_model(model: &str) -> bool {
    if crate::pricing::bundled_free_models().contains(model) {
        return true;
    }
    // Zen names its free tier with a trailing `-free` segment.
    model.ends_with("-free") || has_token(model, "free")
}

pub fn category_from_label(label: &str) -> Category {
    match label {
        "LOCAL" => Category::Local,
        "CLOUD" => Category::Cloud,
        "FREE" => Category::Free,
        "PAID" => Category::Paid,
        _ => Category::Unknown,
    }
}

pub fn cost_status_from_label(label: &str) -> CostStatus {
    match label {
        "reported" => CostStatus::ProviderReported,
        "calculated" => CostStatus::Calculated,
        "estimated" => CostStatus::Estimated,
        "free" => CostStatus::Free,
        "local" => CostStatus::Local,
        _ => CostStatus::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_prioritize_local() {
        assert_eq!(
            classify("ollama", "deepseek-v4-flash-free"),
            Category::Local
        );
    }

    #[test]
    fn cloud_models_are_not_classified_as_local() {
        assert_eq!(classify("ollama", "kimi-k2.7-code:cloud"), Category::Cloud);
    }

    #[test]
    fn free_models_are_detected() {
        assert_eq!(
            classify("opencode", "deepseek-v4-flash-free"),
            Category::Free
        );
    }

    #[test]
    fn unknown_models_are_not_free() {
        assert_eq!(classify("openrouter", "some-model"), Category::Unknown);
    }

    #[test]
    fn anthropic_usage_is_paid_not_unknown() {
        assert_eq!(classify("anthropic", "claude-sonnet-4.6"), Category::Paid);
        assert_eq!(classify("openai", "gpt-5.6-luna"), Category::Paid);
    }

    #[test]
    fn cloudflare_is_not_cloud_routed_usage() {
        // "cloudflare" contains "cloud"; substring matching used to classify it as CLOUD,
        // which suppresses cost estimation entirely.
        assert_ne!(classify("cloudflare", "some-model"), Category::Cloud);
    }

    #[test]
    fn a_model_merely_containing_free_is_not_free() {
        // Being wrongly marked FREE excludes a model from every cost total: invisible spend.
        assert_ne!(classify("openai", "freeform-writer"), Category::Free);
        assert_eq!(classify("opencode", "north-mini-code-free"), Category::Free);
    }

    #[test]
    fn free_models_come_from_the_pricing_table_not_a_second_list() {
        // `big-pickle` carries no "free" marker in its name; it is free because the bundled
        // pricing table says so. Two hand-maintained lists would drift.
        assert_eq!(classify("opencode", "big-pickle"), Category::Free);
    }
}
