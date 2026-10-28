//! Installs and updates the InventorySimulator plugin on request.
//!
//! The plugin is never shipped with the Local Arena package (it is third-party), so the Panel
//! only offers an explicit download action. Only its own two payload paths are written; the
//! plugin's `configs/` directory holds user data and is left alone.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AppError;

const REPOSITORY: &str = "ianlucas/cs2-css-inventory-simulator";
const MARKER_FILE: &str = ".installed.json";
const PLUGIN_DIR: &str = "addons/counterstrikesharp/plugins/InventorySimulator/";
const GAMEDATA_FILE: &str = "addons/counterstrikesharp/gamedata/inventory-simulator.json";
const ASSET_PREFIX: &str = "InventorySimulator-";

#[derive(Clone, Debug, Serialize)]
pub struct InventoryPluginSnapshot {
    pub plugin_present: bool,
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub latest_size: u64,
    pub needs_install: bool,
    pub needs_update: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Release {
    pub tag: String,
    pub asset_name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Marker {
    tag: String,
    sha256: String,
    asset: String,
    installed_at: String,
}

fn marker_path(root: &Path) -> PathBuf {
    root.join(PLUGIN_DIR).join(MARKER_FILE)
}

fn read_marker(root: &Path) -> Option<Marker> {
    let bytes = fs::read(marker_path(root)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn client() -> Result<reqwest::blocking::Client, AppError> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .user_agent("Local-Arena-Panel")
        .build()
        .map_err(|error| AppError::update(error.to_string()))
}

fn pick_asset(name: &str) -> bool {
    name.starts_with(ASSET_PREFIX) && name.ends_with(".zip")
}

fn parse_release(value: &serde_json::Value) -> Result<Release, AppError> {
    let tag = value
        .get("tag_name")
        .and_then(|tag| tag.as_str())
        .ok_or_else(|| AppError::update("Release has no tag_name"))?;
    let asset = value
        .get("assets")
        .and_then(|assets| assets.as_array())
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset
                    .get("name")
                    .and_then(|name| name.as_str())
                    .is_some_and(pick_asset)
            })
        })
        .ok_or_else(|| AppError::update("Release has no InventorySimulator zip asset"))?;
    let digest = asset
        .get("digest")
        .and_then(|digest| digest.as_str())
        .ok_or_else(|| AppError::update("Release asset has no digest"))?;
    let sha256 = digest
        .strip_prefix("sha256:")
        .unwrap_or(digest)
        .to_ascii_lowercase();
    Ok(Release {
        tag: tag.to_string(),
        asset_name: asset
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or_default()
            .to_string(),
        url: asset
            .get("browser_download_url")
            .and_then(|url| url.as_str())
            .unwrap_or_default()
            .to_string(),
        size: asset.get("size").and_then(|size| size.as_u64()).unwrap_or(0),
        sha256,
    })
}

pub fn resolve_release(tag: Option<&str>) -> Result<Release, AppError> {
    let url = match tag {
        Some(tag) => format!("https://api.github.com/repos/{REPOSITORY}/releases/tags/{tag}"),
        None => format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
    };
    let response = client()?
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
    parse_release(&value)
}

pub fn snapshot(root: &Path) -> InventoryPluginSnapshot {
    let plugin_present = root
        .join(PLUGIN_DIR)
        .join("InventorySimulator.dll")
        .is_file();
    let marker = read_marker(root);
    InventoryPluginSnapshot {
        plugin_present,
        installed_version: marker.as_ref().map(|marker| marker.tag.clone()),
        latest_version: None,
        latest_size: 0,
        needs_install: !plugin_present,
        needs_update: false,
    }
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), AppError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(AppError::update(format!(
            "InventorySimulator archive checksum mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

fn extract_payload(archive: &Path, root: &Path) -> Result<usize, AppError> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| AppError::update(format!("InventorySimulator archive is unreadable: {error}")))?;
    let mut written = 0usize;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| AppError::update(error.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let name = name.to_string_lossy().replace('\\', "/");
        let allowed = name.starts_with(PLUGIN_DIR) || name == GAMEDATA_FILE;
        if !allowed {
            continue;
        }
        let destination = root.join(&name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        let mut handle = fs::File::create(&destination)?;
        handle.write_all(&bytes)?;
        written += 1;
    }
    if written == 0 {
        return Err(AppError::update(
            "InventorySimulator archive contained none of the expected files",
        ));
    }
    Ok(written)
}

pub fn install(root: &Path, tag: Option<&str>) -> Result<InventoryPluginSnapshot, AppError> {
    let release = resolve_release(tag)?;
    let cache = std::env::temp_dir().join("local-arena-inventory-plugin");
    fs::create_dir_all(&cache)?;
    let archive = cache.join(&release.asset_name);

    let mut response = client()?
        .get(&release.url)
        .send()
        .map_err(|error| AppError::update(format!("InventorySimulator download failed: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::update(format!(
            "InventorySimulator download returned {}",
            response.status()
        )));
    }
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::update(error.to_string()))?;
    fs::write(&archive, &bytes)?;

    verify_sha256(&archive, &release.sha256)?;
    extract_payload(&archive, root)?;

    let marker = Marker {
        tag: release.tag.clone(),
        sha256: release.sha256.clone(),
        asset: release.asset_name.clone(),
        installed_at: chrono_like_now(),
    };
    let serialized = serde_json::to_vec_pretty(&marker)?;
    fs::write(marker_path(root), serialized)?;
    let _ = fs::remove_file(&archive);

    Ok(snapshot(root))
}

fn chrono_like_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_only_the_plugin_zip_asset() {
        assert!(pick_asset("InventorySimulator-v3.7.0.zip"));
        assert!(!pick_asset("source.zip"));
        assert!(!pick_asset("InventorySimulator-v3.7.0.zip.sha256"));
    }

    #[test]
    fn parses_release_digest_and_size() {
        let value = serde_json::json!({
            "tag_name": "3.7.0",
            "assets": [
                { "name": "other.txt", "browser_download_url": "u0", "size": 1, "digest": "sha256:aa" },
                {
                    "name": "InventorySimulator-v3.7.0.zip",
                    "browser_download_url": "u1",
                    "size": 83624,
                    "digest": "sha256:A19705A7825CA68379CD09E2B25AFDC2364D6947DB3F3121311954224F573933"
                }
            ]
        });
        let release = parse_release(&value).unwrap();
        assert_eq!(release.tag, "3.7.0");
        assert_eq!(release.size, 83_624);
        assert_eq!(release.url, "u1");
        assert!(release.sha256.starts_with("a19705a7"));
    }

    #[test]
    fn rejects_release_without_digest_or_asset() {
        assert!(parse_release(&serde_json::json!({ "tag_name": "1" })).is_err());
        assert!(parse_release(&serde_json::json!({
            "tag_name": "1",
            "assets": [{ "name": "InventorySimulator-v1.zip", "browser_download_url": "u", "size": 1 }]
        }))
        .is_err());
    }

    #[test]
    fn snapshot_reports_install_state_from_marker() {
        let root = std::env::temp_dir().join(format!("la-inv-snapshot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(PLUGIN_DIR)).unwrap();
        let empty = snapshot(&root);
        assert!(!empty.plugin_present);
        assert!(empty.needs_install);
        assert!(empty.installed_version.is_none());

        fs::write(root.join(PLUGIN_DIR).join("InventorySimulator.dll"), b"dll").unwrap();
        fs::write(
            marker_path(&root),
            serde_json::to_vec(&serde_json::json!({
                "tag": "3.7.0", "sha256": "abc", "asset": "a.zip", "installed_at": "1"
            }))
            .unwrap(),
        )
        .unwrap();
        let installed = snapshot(&root);
        assert!(installed.plugin_present);
        assert!(!installed.needs_install);
        assert_eq!(installed.installed_version.as_deref(), Some("3.7.0"));
        fs::remove_dir_all(&root).unwrap();
    }
}
