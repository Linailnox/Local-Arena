//! Installs and updates the InventorySimulator plugin on request.
//!
//! The plugin is never shipped with the Local Arena package (it is third-party), so the Panel
//! only offers an explicit download action. The flow runs entirely on the shared core:
//! `release_source` picks the release asset, `download` streams it (resumable, cancellable,
//! digest-verified) into the versioned state-root download cache, `update_core` extracts it
//! into a fresh staging directory, and `installer::install_component` commits the whitelisted
//! files through the same locked, journalled, record-merging transaction the other components
//! use. Only its own payload paths are written; the plugin's `configs/` directory holds user
//! data and is left alone (staged copies of it are deleted before anything is committed).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::component_registry::ComponentSource;
use crate::installer::PayloadManifest;
use crate::release_source;
use crate::AppError;

const MARKER_FILE: &str = ".installed.json";
const PLUGIN_DIR: &str = "addons/counterstrikesharp/plugins/InventorySimulator/";
const GAMEDATA_FILE: &str = "addons/counterstrikesharp/gamedata/inventory-simulator.json";
const COMPONENT: &str = "InventorySimulator";
const COMPONENT_ID: &str = "inventory_simulator";
const OWNERSHIP: &str = "third-party";

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

/// The compiled-in registry is the single source of truth for where this component's releases
/// live. The descriptor is looked up by id — the same id the update-progress events and the
/// update manifest field names use — so a registry/id mismatch surfaces as an error at the
/// lookup instead of a panic inside the install task.
fn registry_source() -> Result<(&'static str, release_source::AssetSelector), AppError> {
    let descriptor = crate::component_registry::by_id(COMPONENT_ID)
        .ok_or_else(|| AppError::update(format!("component registry has no {COMPONENT_ID} descriptor")))?;
    match descriptor.source {
        ComponentSource::GitHubRelease { repository, asset } => Ok((repository, asset.selector())),
        ComponentSource::SignedLatestJson { field } => Err(AppError::update(format!(
            "{COMPONENT_ID} registry source changed: SignedLatestJson({field})"
        ))),
    }
}

fn update_root() -> Result<PathBuf, AppError> {
    crate::online_update::update_root()
}

fn download_destination(update_root: &Path, tag: &str, asset_name: &str) -> PathBuf {
    update_root
        .join("downloads")
        .join("inventory-simulator")
        .join(tag)
        .join(asset_name)
}

fn staging_root(update_root: &Path, tag: &str) -> PathBuf {
    update_root
        .join("staging")
        .join("inventory-simulator")
        .join(tag)
}

pub fn resolve_release(tag: Option<&str>) -> Result<Release, AppError> {
    let client = client()?;
    let (repository, selector) = registry_source()?;
    let resolved = match tag {
        Some(tag) => release_source::resolve_tag(&client, repository, tag, &selector)?,
        None => release_source::resolve_latest(&client, repository, &selector)?,
    };
    Ok(Release {
        tag: resolved.tag,
        asset_name: resolved.asset_name,
        url: resolved.url,
        size: resolved.size,
        sha256: resolved.sha256,
    })
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
        // Deliberately network-free: `snapshot` cannot know the latest tag, so a live
        // `needs_update` would read as "up to date" whenever there is no network. The
        // installed-vs-latest comparison happens where a latest tag exists — the frontend
        // comparing the `inventory_plugin_check` release against `installed_version`.
        needs_update: false,
    }
}

fn staged_relative(path: &Path, staging: &Path) -> String {
    path.strip_prefix(staging)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn whitelist_staged(relative: &str) -> bool {
    relative.starts_with(PLUGIN_DIR) || relative == GAMEDATA_FILE
}

/// Deletes every staged file that is not one of the component's owned paths and reports how
/// many whitelisted files survive. User-data copies (`configs/…`) and unrelated archives never
/// reach the game directory.
fn prune_staging(staging: &Path) -> Result<usize, AppError> {
    let mut kept = Vec::new();
    prune_impl(staging, staging, &mut kept)?;
    remove_empty_dirs(staging);
    Ok(kept.len())
}

fn prune_impl(dir: &Path, staging: &Path, kept: &mut Vec<PathBuf>) -> Result<(), AppError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            prune_impl(&path, staging, kept)?;
        } else if whitelist_staged(&staged_relative(&path, staging)) {
            kept.push(path);
        } else {
            let _ = fs::remove_file(&path);
        }
    }
    Ok(())
}

fn remove_empty_dirs(dir: &Path) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_empty_dirs(&path);
            let _ = fs::remove_dir(path);
        }
    }
}

fn collect_relative_files(dir: &Path, staging: &Path, collected: &mut Vec<String>) -> Result<(), AppError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_relative_files(&path, staging, collected)?;
        } else {
            collected.push(staged_relative(&path, staging));
        }
    }
    Ok(())
}

/// Writes the install marker into the staging root and builds the synthetic payload manifest
/// the shared transaction will commit: one entry per surviving whitelisted file plus the
/// marker entry last. The transaction therefore installs the marker atomically as its last
/// file — no separate write to the game directory happens.
fn synthetic_manifest(
    staging: &Path,
    tag: &str,
    marker_bytes: &[u8],
) -> Result<PayloadManifest, AppError> {
    let mut relative_paths = Vec::new();
    collect_relative_files(staging, staging, &mut relative_paths)?;
    relative_paths.retain(|relative| whitelist_staged(relative));
    relative_paths.sort();

    let mut entries = Vec::new();
    for relative in relative_paths {
        let path = staging.join(&relative);
        entries.push(serde_json::from_value::<crate::installer::PayloadEntry>(
            serde_json::json!({
                "path": relative,
                "size": fs::metadata(&path)?.len(),
                "sha256": crate::update_core::sha256_file(&path)?,
                "component": COMPONENT,
                "ownership": OWNERSHIP,
                "restore_policy": "restore",
            }),
        )?);
    }

    let marker_relative = format!("{PLUGIN_DIR}{MARKER_FILE}");
    let marker_path = staging.join(&marker_relative);
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&marker_path, marker_bytes)?;
    entries.push(
        serde_json::from_value::<crate::installer::PayloadEntry>(serde_json::json!({
            "path": marker_relative,
            "size": marker_bytes.len() as u64,
            "sha256": crate::update_core::sha256_file(&marker_path)?,
            "component": COMPONENT,
            "ownership": OWNERSHIP,
            "restore_policy": "restore",
        }))?,
    );

    Ok(PayloadManifest {
        schema_version: 1,
        package_version: tag.to_string(),
        entries,
    })
}

pub fn install(
    app: &AppHandle,
    root: &Path,
    tag: Option<&str>,
) -> Result<InventoryPluginSnapshot, AppError> {
    let release = resolve_release(tag)?;
    let update_root = update_root()?;

    // The versioned destination lets a previously downloaded, digest-verified archive be
    // reused without any network, and keeps separate tags from resuming against each other.
    let archive = download_destination(&update_root, &release.tag, &release.asset_name);
    let client = client()?;
    crate::download::download_with_progress(
        &client,
        &release.url,
        release.size,
        &release.sha256,
        &archive,
        |downloaded_bytes, total_bytes| {
            let _ = app.emit(
                "update-progress",
                crate::online_update::UpdateProgress {
                    component: COMPONENT_ID.into(),
                    stage: "downloading".into(),
                    downloaded_bytes,
                    total_bytes,
                },
            );
        },
        // The shared cancel flag: the existing cancel-update command cancels this download.
        crate::online_update::cancelled,
    )?;

    let staging = staging_root(&update_root, &release.tag);
    let result = stage_and_install(&staging, &archive, root, &release);
    let _ = fs::remove_dir_all(&staging);
    result?;

    Ok(snapshot(root))
}

fn stage_and_install(
    staging: &Path,
    archive: &Path,
    root: &Path,
    release: &Release,
) -> Result<(), AppError> {
    let _ = fs::remove_dir_all(staging);
    fs::create_dir_all(staging).map_err(AppError::transaction_io)?;

    let file = fs::File::open(archive)?;
    if let Err(error) = crate::update_core::extract_zip_safely(file, staging) {
        return Err(AppError::update(error));
    }
    if prune_staging(staging)? == 0 {
        return Err(AppError::update(
            "InventorySimulator archive contained none of the expected files",
        ));
    }

    let marker = Marker {
        tag: release.tag.clone(),
        sha256: release.sha256.clone(),
        asset: release.asset_name.clone(),
        installed_at: chrono_like_now(),
    };
    let marker_bytes = serde_json::to_vec_pretty(&marker)?;
    let manifest = synthetic_manifest(staging, &release.tag, &marker_bytes)?;
    fs::write(
        staging.join(crate::installer::MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    let staging_dir = staging.to_path_buf();
    let state_root = crate::app_storage::root()?;
    // One locked, journalled transaction installs every manifest entry — payload files and
    // the marker — into the game directory and merges the install record. It never touches
    // `active-payload.json`, which belongs to full-package installs.
    crate::installer::install_component(&staging_dir, &state_root, root, OWNERSHIP)?;
    Ok(())
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
        let (_, selector) = registry_source().unwrap();
        assert!(selector.matches("InventorySimulator-v3.7.0.zip"));
        assert!(!selector.matches("source.zip"));
        assert!(!selector.matches("InventorySimulator-v3.7.0.zip.sha256"));
    }

    #[test]
    fn parses_release_digest_and_size() {
        let value = serde_json::json!({
            "tag_name": "3.7.0",
            "assets": [
                { "name": "other.txt", "browser_download_url": "https://github.com/ianlucas/cs2-css-inventory-simulator/releases/download/3.7.0/other.txt", "size": 1, "digest": "sha256:aa" },
                {
                    "name": "InventorySimulator-v3.7.0.zip",
                    "browser_download_url": "https://github.com/ianlucas/cs2-css-inventory-simulator/releases/download/3.7.0/InventorySimulator-v3.7.0.zip",
                    "size": 83624,
                    "digest": "sha256:A19705A7825CA68379CD09E2B25AFDC2364D6947DB3F3121311954224F573933"
                }
            ]
        });
        let (repository, selector) = registry_source().unwrap();
        let release = release_source::parse_release_value(&value, repository, &selector).unwrap();
        assert_eq!(release.tag, "3.7.0");
        assert_eq!(release.size, 83_624);
        assert_eq!(
            release.url,
            "https://github.com/ianlucas/cs2-css-inventory-simulator/releases/download/3.7.0/InventorySimulator-v3.7.0.zip"
        );
        assert!(release.sha256.starts_with("a19705a7"));
    }

    #[test]
    fn rejects_release_without_digest_or_asset() {
        let (repository, selector) = registry_source().unwrap();
        assert!(
            release_source::parse_release_value(
                &serde_json::json!({ "tag_name": "1" }),
                repository,
                &selector
            )
            .is_err()
        );
        assert!(
            release_source::parse_release_value(
                &serde_json::json!({
                    "tag_name": "1",
                    "assets": [{
                        "name": "InventorySimulator-v1.zip",
                        "browser_download_url": "https://github.com/ianlucas/cs2-css-inventory-simulator/releases/download/1/InventorySimulator-v1.zip",
                        "size": 1
                    }]
                }),
                repository,
                &selector
            )
            .is_err()
        );
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

    fn temp_staging(suffix: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("la-inv-stage-{}-{suffix}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sha256_bytes(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(bytes);
        format!("{:x}", hash.finalize())
    }

    #[test]
    fn staged_files_outside_the_whitelist_are_deleted() {
        let staging = temp_staging("whitelist");
        let plugin_dll = staging
            .join(PLUGIN_DIR)
            .join("InventorySimulator.dll");
        fs::create_dir_all(plugin_dll.parent().unwrap()).unwrap();
        fs::write(&plugin_dll, b"plugin").unwrap();
        let gamedata = staging.join(GAMEDATA_FILE);
        fs::create_dir_all(gamedata.parent().unwrap()).unwrap();
        fs::write(&gamedata, b"gamedata").unwrap();
        // The conflicting `configs/` copy and `Unrelated.dll` must both vanish.
        let config = staging.join("configs/plugins/InventorySimulator/inventories.json");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, b"user data").unwrap();
        let unrelated = staging.join("Unrelated.dll");
        fs::write(&unrelated, b"unrelated").unwrap();

        let kept = prune_staging(&staging).unwrap();
        assert_eq!(kept, 2);
        assert!(plugin_dll.is_file(), "the plugin dll survives");
        assert!(gamedata.is_file(), "the gamedata file survives");
        assert!(!config.exists(), "configs/ user data is never installed");
        assert!(!unrelated.exists(), "unrelated files are deleted");
        fs::remove_dir_all(&staging).unwrap();
    }

    #[test]
    fn synthetic_manifest_carries_exactly_the_whitelisted_files_and_marker() {
        let staging = temp_staging("manifest");
        let plugin_dll = staging.join(PLUGIN_DIR).join("InventorySimulator.dll");
        fs::create_dir_all(plugin_dll.parent().unwrap()).unwrap();
        fs::write(&plugin_dll, b"plugin").unwrap();
        let gamedata = staging.join(GAMEDATA_FILE);
        fs::create_dir_all(gamedata.parent().unwrap()).unwrap();
        fs::write(&gamedata, b"gamedata").unwrap();
        let unrelated = staging.join("Unrelated.dll");
        fs::write(&unrelated, b"unrelated").unwrap();

        let marker_bytes = b"{}marker-bytes";
        let marker_relative = format!("{PLUGIN_DIR}{MARKER_FILE}");
        let dll_relative = format!("{PLUGIN_DIR}InventorySimulator.dll");
        let manifest = synthetic_manifest(&staging, "3.7.0", marker_bytes).unwrap();

        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.package_version, "3.7.0");
        assert_eq!(manifest.entries.len(), 3);
        let paths: Vec<&str> = manifest
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec![
                GAMEDATA_FILE,
                dll_relative.as_str(),
                marker_relative.as_str(),
            ],
            "the manifest covers exactly the whitelisted files plus the marker"
        );
        for entry in &manifest.entries {
            assert_eq!(entry.ownership, "third-party");
            assert_eq!(entry.restore_policy, "restore");
        }
        let dll_entry = manifest
            .entries
            .iter()
            .find(|entry| entry.path.ends_with("InventorySimulator.dll"))
            .unwrap();
        assert_eq!(dll_entry.sha256, sha256_bytes(b"plugin"));
        fs::remove_dir_all(&staging).unwrap();
    }

    #[test]
    fn marker_entry_is_last_and_matches_marker_bytes() {
        let staging = temp_staging("marker");
        let gamedata = staging.join(GAMEDATA_FILE);
        fs::create_dir_all(gamedata.parent().unwrap()).unwrap();
        fs::write(&gamedata, b"gamedata").unwrap();

        let marker_bytes = serde_json::to_vec_pretty(&serde_json::json!({
            "tag": "3.7.2", "sha256": "abc", "asset": "a.zip", "installed_at": "1"
        }))
        .unwrap();
        let manifest =
            synthetic_manifest(&staging, "3.7.2", &marker_bytes).unwrap();

        let last = manifest.entries.last().unwrap();
        assert_eq!(last.path, format!("{PLUGIN_DIR}{MARKER_FILE}"));
        assert_eq!(last.sha256, sha256_bytes(&marker_bytes));
        assert_eq!(last.size, marker_bytes.len() as u64);
        let staged_marker = fs::read(staging.join(PLUGIN_DIR).join(MARKER_FILE)).unwrap();
        assert_eq!(staged_marker, marker_bytes);
        fs::remove_dir_all(&staging).unwrap();
    }

    #[test]
    fn download_and_staging_paths_are_versioned_by_tag() {
        let update_root = Path::new("/tmp/la-inv-paths");
        let old = download_destination(update_root, "3.7.0", "InventorySimulator-v3.7.0.zip");
        let new = download_destination(update_root, "3.7.2", "InventorySimulator-v3.7.2.zip");
        assert_ne!(old, new, "tags must not share a resumed `.part` base");
        assert!(old.starts_with(update_root.join("downloads/inventory-simulator/3.7.0")));
        assert!(new.starts_with(update_root.join("downloads/inventory-simulator/3.7.2")));
        let old_staging = staging_root(update_root, "3.7.0");
        let new_staging = staging_root(update_root, "3.7.2");
        assert_ne!(old_staging, new_staging);
        assert!(old_staging.starts_with(update_root.join("staging/inventory-simulator/3.7.0")));
    }
}
