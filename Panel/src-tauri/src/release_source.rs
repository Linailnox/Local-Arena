//! Shared GitHub release resolution for components that require a digest.
//!
//! Every consumer of this module needs a verifiable download: the release asset must carry a
//! `digest`, the download URL must be a trusted GitHub host, and the digest must be a well-formed
//! SHA-256. Components that can live with a digest-less fallback (the upstream package download,
//! which tolerates the 403/429 anonymous rate limit and then verifies against its pinned hash)
//! keep their own parser in `upstream_package.rs`; that boundary is deliberate and stays there.

use serde::{Deserialize, Serialize};

use crate::{AppError, Result, update_core};

const API_BASE: &str = "https://api.github.com/repos";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedAsset {
    pub tag: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub asset_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AssetSelector {
    /// Matches a zip archive whose name starts with the given prefix. The `.zip` requirement is
    /// part of the predicate on purpose: release pages also publish `*.zip.sha256` siblings, and a
    /// bare prefix match would hand the checksum file to the downloader.
    Prefix(String),
    /// Matches the asset whose name equals the given name exactly.
    ExactName(String),
}

impl AssetSelector {
    pub fn matches(&self, name: &str) -> bool {
        match self {
            AssetSelector::Prefix(prefix) => name.starts_with(prefix.as_str()) && name.ends_with(".zip"),
            AssetSelector::ExactName(exact) => name == exact,
        }
    }
}

pub fn resolve_latest(
    client: &reqwest::blocking::Client,
    repository: &str,
    selector: &AssetSelector,
) -> Result<ResolvedAsset> {
    let url = format!("{API_BASE}/{repository}/releases/latest");
    fetch_release(client, &url, repository, selector)
}

pub fn resolve_tag(
    client: &reqwest::blocking::Client,
    repository: &str,
    tag: &str,
    selector: &AssetSelector,
) -> Result<ResolvedAsset> {
    let url = format!("{API_BASE}/{repository}/releases/tags/{tag}");
    fetch_release(client, &url, repository, selector)
}

fn fetch_release(
    client: &reqwest::blocking::Client,
    url: &str,
    repository: &str,
    selector: &AssetSelector,
) -> Result<ResolvedAsset> {
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|error| AppError::update(format!("GitHub release request failed: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::update(format!(
            "GitHub release request returned {}",
            response.status()
        )));
    }
    let value: serde_json::Value = response
        .json()
        .map_err(|error| AppError::update(format!("GitHub release JSON is invalid: {error}")))?;
    parse_release_value(&value, repository, selector)
}

/// Parses an already-decoded GitHub release document.
pub(crate) fn parse_release_value(
    value: &serde_json::Value,
    repository: &str,
    selector: &AssetSelector,
) -> Result<ResolvedAsset> {
    let tag = value
        .get("tag_name")
        .and_then(|tag| tag.as_str())
        .filter(|tag| !tag.trim().is_empty())
        .ok_or_else(|| AppError::update("Release has no tag_name"))?;
    let asset = value
        .get("assets")
        .and_then(|assets| assets.as_array())
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset
                    .get("name")
                    .and_then(|name| name.as_str())
                    .is_some_and(|name| selector.matches(name))
            })
        })
        .ok_or_else(|| {
            AppError::update(format!(
                "Release has no matching asset for {repository}"
            ))
        })?;
    let digest = asset
        .get("digest")
        .and_then(|digest| digest.as_str())
        .ok_or_else(|| AppError::update("Release asset has no digest"))?;
    let sha256 = digest
        .strip_prefix("sha256:")
        .unwrap_or(digest)
        .trim()
        .to_ascii_lowercase();
    update_core::validate_sha256(&sha256).map_err(AppError::update)?;
    let url = asset
        .get("browser_download_url")
        .and_then(|url| url.as_str())
        .ok_or_else(|| AppError::update("Release asset has no browser_download_url"))?;
    update_core::validate_https_github_url(url).map_err(AppError::update)?;
    let asset_name = asset
        .get("name")
        .and_then(|name| name.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(ResolvedAsset {
        tag: tag.to_string(),
        url: url.to_string(),
        size: asset.get("size").and_then(|size| size.as_u64()).unwrap_or(0),
        sha256,
        asset_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPOSITORY: &str = "ianlucas/cs2-css-inventory-simulator";
    const DOWNLOAD_BASE: &str =
        "https://github.com/ianlucas/cs2-css-inventory-simulator/releases/download/3.7.0/";
    const DIGEST: &str = "A19705A7825CA68379CD09E2B25AFDC2364D6947DB3F3121311954224F573933";

    fn inventory_selector() -> AssetSelector {
        AssetSelector::Prefix("InventorySimulator-".to_string())
    }

    #[test]
    fn prefix_selector_picks_only_the_plugin_zip() {
        let selector = inventory_selector();
        assert!(selector.matches("InventorySimulator-v3.7.0.zip"));
        assert!(!selector.matches("source.zip"));
        assert!(!selector.matches("InventorySimulator-v3.7.0.zip.sha256"));

        let value = serde_json::json!({
            "tag_name": "3.7.0",
            "assets": [
                { "name": "InventorySimulator-v3.7.0.zip.sha256", "browser_download_url": format!("{DOWNLOAD_BASE}InventorySimulator-v3.7.0.zip.sha256"), "size": 64, "digest": format!("sha256:{DIGEST}") },
                { "name": "InventorySimulator-v3.7.0.zip", "browser_download_url": format!("{DOWNLOAD_BASE}InventorySimulator-v3.7.0.zip"), "size": 83624, "digest": format!("sha256:{DIGEST}") }
            ]
        });
        let resolved = parse_release_value(&value, REPOSITORY, &selector).unwrap();
        assert_eq!(resolved.asset_name, "InventorySimulator-v3.7.0.zip");
    }

    #[test]
    fn rejects_asset_without_digest() {
        let value = serde_json::json!({
            "tag_name": "3.7.0",
            "assets": [{
                "name": "InventorySimulator-v3.7.0.zip",
                "browser_download_url": format!("{DOWNLOAD_BASE}InventorySimulator-v3.7.0.zip"),
                "size": 83624
            }]
        });
        assert!(parse_release_value(&value, REPOSITORY, &inventory_selector()).is_err());
    }

    #[test]
    fn rejects_untrusted_download_host() {
        let value = serde_json::json!({
            "tag_name": "3.7.0",
            "assets": [{
                "name": "InventorySimulator-v3.7.0.zip",
                "browser_download_url": "https://example.com/InventorySimulator-v3.7.0.zip",
                "size": 83624,
                "digest": format!("sha256:{DIGEST}")
            }]
        });
        assert!(parse_release_value(&value, REPOSITORY, &inventory_selector()).is_err());
    }

    #[test]
    fn exact_name_selector_picks_the_upstream_zip() {
        let selector = AssetSelector::ExactName("CS2BotImprover.zip".to_string());
        assert!(selector.matches("CS2BotImprover.zip"));
        assert!(!selector.matches("CS2BotImprover.zip.sha256"));

        let value = serde_json::json!({
            "tag_name": "1.4.5",
            "assets": [
                { "name": "CS2BotImprover.zip.sha256", "browser_download_url": "https://github.com/ed0ard/CS2-Bot-Improver/releases/download/1.4.5/CS2BotImprover.zip.sha256", "size": 64, "digest": format!("sha256:{DIGEST}") },
                { "name": "CS2BotImprover.zip", "browser_download_url": "https://github.com/ed0ard/CS2-Bot-Improver/releases/download/1.4.5/CS2BotImprover.zip", "size": 73592318, "digest": format!("sha256:{DIGEST}") }
            ]
        });
        let resolved = parse_release_value(&value, "ed0ard/CS2-Bot-Improver", &selector).unwrap();
        assert_eq!(resolved.asset_name, "CS2BotImprover.zip");
        assert_eq!(resolved.size, 73_592_318);
    }

    #[test]
    fn lowercases_the_release_digest() {
        let value = serde_json::json!({
            "tag_name": "3.7.0",
            "assets": [{
                "name": "InventorySimulator-v3.7.0.zip",
                "browser_download_url": format!("{DOWNLOAD_BASE}InventorySimulator-v3.7.0.zip"),
                "size": 83624,
                "digest": format!("sha256:{DIGEST}")
            }]
        });
        let resolved = parse_release_value(&value, REPOSITORY, &inventory_selector()).unwrap();
        assert_eq!(resolved.sha256, DIGEST.to_ascii_lowercase());
        assert!(resolved.sha256.starts_with("a19705a7"));
    }
}
