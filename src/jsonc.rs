use std::path::Path;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

/// Parse a JSONC file into any deserializable type.
///
/// Strips comments using `jsonc-parser`, then deserializes with `serde_json`.
pub fn parse_file<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    let json_str = jsonc_parser::parse_to_serde_value(&content, &Default::default())
        .map_err(|e| anyhow::anyhow!("JSONC parse error in {}: {}", path.display(), e))?
        .ok_or_else(|| anyhow::anyhow!("Empty JSONC file: {}", path.display()))?;

    serde_json::from_value(json_str)
        .with_context(|| format!("Failed to deserialize: {}", path.display()))
}
