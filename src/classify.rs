use crate::model::{Category, CostStatus};

pub fn classify(provider: &str, model: &str) -> Category {
    let p = provider.to_ascii_lowercase();
    let m = model.to_ascii_lowercase();
    if p.contains("cloud") || m.contains(":cloud") || m.contains("-cloud") {
        return Category::Cloud;
    }
    let local = [
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
    if local
        .iter()
        .any(|part| p.contains(part) || m.contains(part))
    {
        return Category::Local;
    }
    let free = [
        "free",
        "big-pickle",
        "mimo-v2.5-free",
        "deepseek-v4-flash-free",
        "north-mini-code-free",
        "nemotron-3-ultra-free",
    ];
    if free.iter().any(|part| m.contains(part)) {
        return Category::Free;
    }
    Category::Unknown
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
}
