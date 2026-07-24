use std::{fs, path::PathBuf, time::Duration};

use anyhow::Result;
use serde_json::Value;

use crate::utils::data_dir;

pub fn zen_cache_path() -> Option<PathBuf> {
    Some(data_dir()?.join("zen-models.json"))
}

pub fn refresh_zen_catalog() -> Result<PathBuf> {
    let path = zen_cache_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Zen cache path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let response = client
        .get("https://opencode.ai/zen/v1/models")
        .send()?
        .error_for_status()?;
    let body: Value = response.json()?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(&body)?)?;
    fs::rename(temporary, &path)?;
    Ok(path)
}
