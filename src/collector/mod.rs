pub mod background;
pub mod journal;
pub mod opencode;
pub mod pricing_refresh;
pub mod zen;

use std::path::Path;

use anyhow::Result;

use crate::collector::journal::load_journal;
use crate::collector::opencode::load_opencode;
use crate::collector::zen::zen_cache_path;
use crate::model::Usage;
use crate::pricing::{apply_estimated_pricing, PricingEngine};

pub fn load_usage(override_path: Option<&Path>, journal: &Path) -> Result<(Vec<Usage>, String)> {
    let (mut usages, opencode_source) = load_opencode(override_path)?;
    let journal_usages = load_journal(journal)?;
    let journal_source = if journal.exists() {
        format!("journal: {}", journal.display())
    } else {
        "journal: not initialized".to_string()
    };
    let zen_source = match zen_cache_path() {
        Some(path) if path.exists() => {
            format!("Zen catalog: cached (informational) at {}", path.display())
        }
        _ => "Zen catalog: not cached".to_string(),
    };
    let existing_usage = usages.clone();
    usages.extend(journal_usages.into_iter().filter(|journal_usage| {
        !existing_usage
            .iter()
            .any(|usage| usage_key(usage) == usage_key(journal_usage))
    }));

    let engine = PricingEngine::load();
    apply_estimated_pricing(&mut usages, &engine);

    Ok((
        usages,
        format!("{} | {} | {}", opencode_source, journal_source, zen_source),
    ))
}

pub fn usage_key(usage: &Usage) -> (String, String, u64, u64, u64, u64, u64) {
    (
        usage.provider.clone(),
        usage.model.clone(),
        usage.input,
        usage.output,
        usage.reasoning,
        usage.cache_read,
        usage.cache_write,
    )
}

#[cfg(test)]
pub fn setup_test_db() -> std::path::PathBuf {
    std::path::PathBuf::from("tests/fixtures/opencode_test.db")
}

#[cfg(test)]
pub fn setup_test_journal() -> std::path::PathBuf {
    std::path::PathBuf::from("tests/fixtures/test_journal.db")
}
