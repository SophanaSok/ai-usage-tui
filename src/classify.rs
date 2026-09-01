use crate::model::{Category, CostStatus};

/// Providers that bill per token. Matched as whole tokens of the provider id.
///
/// Without this, billable usage falls through to `Unknown` and lands in the red UNKNOWN tile.
/// Note what this does *not* do: it never causes a row to be priced. A row is priced when the
/// pricing table can resolve it, and one that gets a figure is promoted to `Paid` regardless of
/// what is listed here (`pricing::apply_estimated_pricing`). This list decides the category of
/// rows we *cannot* price — the difference between "real spend, rate unknown" and "no idea what
/// this is".
///
/// The aggregators were excluded until provider-qualified model resolution landed, on the
/// grounds that calling them PAID promised a figure we could not produce. Both halves of that
/// have changed: the pricing table now carries provider-qualified keys for them (106 for
/// OpenRouter, 111 for Azure, 82 for Bedrock), and a PAID row with no rate is a state the tool
/// already reports honestly — it counts against the pricing-coverage figure rather than being
/// rendered as `$0.00`.
///
/// Kept as an explicit list rather than derived from the pricing table's key prefixes, which
/// looked more principled and is not: `google` has no keys at all (LiteLLM spells it `gemini`
/// and `vertex_ai`), `ollama` has 29 and is emphatically not billable, and LiteLLM's
/// `fireworks_ai` does not match the `fireworks-ai` a collector records. Matching those up
/// needs fuzzy token comparison, and a token like `ai` matches nearly anything.
const PAID_PROVIDERS: &[&str] = &[
    // First-party.
    "anthropic",
    "openai",
    "google",
    "gemini",
    "mistral",
    "cohere",
    "deepseek",
    "xai",
    "groq",
    // Aggregators, resellers and clouds. All bill per token, and all have provider-qualified
    // rates in the bundled table.
    "openrouter",
    "bedrock",
    "azure",
    "vertex",
    "fireworks",
    "deepinfra",
    "together",
    "togetherai",
    "perplexity",
    // GitHub bills Copilot by seat and premium request rather than per token, but it is still a
    // vendor that charges for the work. Category says who bills; `cost_status` says whether a
    // per-token rate exists, and for Copilot it is `Quota` -- the same pairing Claude Code on a
    // plan already gets.
    "copilot",
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

    // After the local and free checks, deliberately: `ollama` has rates in the bundled pricing
    // table and several hosts carry a provider name that also appears here.
    if PAID_PROVIDERS.iter().any(|known| has_token(&p, known)) {
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
        "quota" => CostStatus::Quota,
        // An unrecognised label degrades to the pessimistic status rather than a confident one.
        // This match has no exhaustiveness check, so a new variant added without a case here is
        // silent — `every_cost_status_round_trips_through_its_label` is the only guard.
        _ => CostStatus::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The aggregators bill per token and the table can now price them.
    #[test]
    fn aggregators_and_clouds_are_billable_not_unknown() {
        for (provider, model) in [
            ("openrouter", "anthropic/claude-3.5-sonnet"),
            (
                "amazon-bedrock",
                "anthropic.claude-sonnet-4-5-20250929-v1:0",
            ),
            ("azure", "gpt-4o"),
            ("google-vertex", "gemini-2.5-pro"),
            ("fireworks-ai", "llama-v3p1-70b-instruct"),
            ("deepinfra", "meta-llama/Meta-Llama-3.1-70B-Instruct"),
            ("togetherai", "meta-llama/Llama-3-70b-chat-hf"),
            ("perplexity", "sonar"),
        ] {
            assert_eq!(
                classify(provider, model),
                Category::Paid,
                "{provider} should be billable"
            );
        }
    }

    /// The ordering that keeps the previous line honest.
    ///
    /// `ollama` has 29 provider-qualified entries in the bundled pricing table, so anything that
    /// decided billability from the table alone would call a local model paid. The local and free
    /// checks run first, and this fails if that order is ever changed.
    #[test]
    fn local_and_free_still_win_over_the_paid_list() {
        assert_eq!(classify("ollama", "qwen3-coder"), Category::Local);
        assert_eq!(classify("openai", "localhost-proxy"), Category::Local);
        assert_eq!(
            classify("openrouter", "deepseek-v4-flash-free"),
            Category::Free
        );
        // `:cloud` is decided before everything else.
        assert_eq!(classify("ollama", "glm-5.2:cloud"), Category::Cloud);
    }

    /// Token matching, not substring: the reason `cloudflare` is not "cloud" and `freeform` is
    /// not "free".
    #[test]
    fn provider_names_are_matched_as_whole_tokens() {
        assert_ne!(classify("cloudflare", "some-model"), Category::Cloud);
        assert_ne!(classify("someprovider", "freeform-model"), Category::Free);
        // A provider that merely contains a listed name as a substring is not a match.
        assert_eq!(
            classify("notopenairelated", "some-model"),
            Category::Unknown
        );
    }

    /// Nothing here should make a row *priced*; that is the pricing engine's decision.
    #[test]
    fn an_unrecognised_provider_is_still_unknown() {
        assert_eq!(classify("some-startup", "their-model"), Category::Unknown);
    }

    /// Every `CostStatus`, so the round-trip test below fails to compile if a variant is added
    /// without being considered here.
    const ALL_STATUSES: [CostStatus; 7] = [
        CostStatus::ProviderReported,
        CostStatus::Calculated,
        CostStatus::Estimated,
        CostStatus::Free,
        CostStatus::Local,
        CostStatus::Quota,
        CostStatus::Unavailable,
    ];

    #[test]
    fn every_cost_status_round_trips_through_its_label() {
        // `cost_status_from_label` has a `_` fallback, so a new variant missing its arm is
        // silent: the status is written to the journal correctly and read back as Unavailable.
        // This is the only guard on that match.
        for status in ALL_STATUSES {
            assert_eq!(
                cost_status_from_label(status.label()),
                status,
                "{} did not survive a write/read cycle",
                status.label()
            );
        }
    }

    #[test]
    fn an_unrecognised_label_degrades_to_the_pessimistic_status() {
        // An older binary reading a newer journal must not claim a cost state it cannot verify.
        assert_eq!(
            cost_status_from_label("some-future-status"),
            CostStatus::Unavailable
        );
    }

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
        // The point of this test is that an unrecognised *model* is not free. It used to assert
        // `openrouter` + unknown model was `Unknown`, which encoded the old policy of excluding
        // aggregators from the paid list; that policy has changed deliberately. What must not
        // change is that an unrecognised model never becomes FREE, which would exclude it from
        // every cost total.
        assert_ne!(classify("openrouter", "some-model"), Category::Free);
        assert_ne!(classify("some-startup", "some-model"), Category::Free);
        // OpenRouter bills, so a model we cannot price there is billable-with-unknown-cost.
        assert_eq!(classify("openrouter", "some-model"), Category::Paid);
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
