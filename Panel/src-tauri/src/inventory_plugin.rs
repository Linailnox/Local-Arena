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

/// Shared update client (HTTPS-only, 120 s timeout) — the same one the other components use.
fn client() -> Result<reqwest::blocking::Client, AppError> {
    crate::online_update::client(std::time::Duration::from_secs(120))
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

/// The staged InventorySimulator payload the shared transaction commits from.
///
/// This component deletes its staging tree as soon as an install commits, so the resolver answers
/// "what is staged right now" and never pretends a staged tree outlives the install: with nothing
/// staged it reports the install action rather than a stale layout path.
pub fn latest_staging_root() -> Result<PathBuf, AppError> {
    let root = update_root()?
        .join("staging")
        .join("inventory-simulator");
    let mut staged: Vec<PathBuf> = fs::read_dir(&root)
        .map_err(|_| {
            AppError::payload(
                "The InventorySimulator payload has not been staged; install the component first",
            )
        })?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir() && path.join(crate::installer::MANIFEST_FILE).is_file()
        })
        .collect();
    staged.sort();
    staged.pop().ok_or_else(|| {
        AppError::payload(
            "The InventorySimulator payload has not been staged; install the component first",
        )
    })
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

/// The installed version, read from the component record those same transactions write.
///
/// Nothing is read out of the game directory to derive a version: a legacy `.installed.json` a
/// previous Panel left behind is not a source any more, and this component's own version lives in
/// `record.components["inventory_simulator"]` like every other component's.
pub fn snapshot(state_root: &Path, root: &Path) -> InventoryPluginSnapshot {
    let plugin_present = root
        .join(PLUGIN_DIR)
        .join("InventorySimulator.dll")
        .is_file();
    InventoryPluginSnapshot {
        plugin_present,
        installed_version: crate::installer::component_version(
            state_root,
            root,
            crate::installer::ComponentId::InventorySimulator,
        ),
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

/// Builds the synthetic payload manifest the shared transaction commits: one entry per surviving
/// whitelisted file, and nothing else.
///
/// The component's installed version is not a file in the game directory — the transaction records
/// the release tag under `record.components["inventory_simulator"]`, so a marker file (with its own
/// timestamp, kept in step with the payload by hand) is no longer needed.
fn synthetic_manifest(staging: &Path, tag: &str) -> Result<PayloadManifest, AppError> {
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
    // A peer component writes into the game directory under the same rules as the others: never
    // while CS2 holds the files and never while Steam is replacing them.
    crate::ensure_target_not_running(root)?;
    crate::ensure_steam_app_idle(root)?;
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
    let state_root = crate::app_storage::root()?;
    let result = stage_and_install(&staging, &archive, root, &state_root, &release);
    let _ = fs::remove_dir_all(&staging);
    result?;

    Ok(snapshot(&state_root, root))
}

fn stage_and_install(
    staging: &Path,
    archive: &Path,
    root: &Path,
    state_root: &Path,
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
    commit_staged(staging, state_root, root, &release.tag)
}

/// Writes the synthetic manifest for a pruned staging tree and commits it through the shared
/// component transaction — the same locked, journalled install every other component uses. The
/// transaction records the tag under `record.components["inventory_simulator"]`, which is the only
/// place this component's installed version is kept.
fn commit_staged(
    staging: &Path,
    state_root: &Path,
    root: &Path,
    tag: &str,
) -> Result<(), AppError> {
    let manifest = synthetic_manifest(staging, tag)?;
    fs::write(
        staging.join(crate::installer::MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    crate::installer::install_component(
        staging,
        state_root,
        root,
        crate::installer::ComponentId::InventorySimulator,
    )?;
    Ok(())
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

    fn temp_root(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("la-inv-root-{}-{suffix}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A pruned staging tree holding exactly the two whitelisted files.
    fn staged_tree(suffix: &str) -> PathBuf {
        let staging = temp_staging(suffix);
        let dll = staging.join(PLUGIN_DIR).join("InventorySimulator.dll");
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"plugin").unwrap();
        let gamedata = staging.join(GAMEDATA_FILE);
        fs::create_dir_all(gamedata.parent().unwrap()).unwrap();
        fs::write(&gamedata, b"gamedata").unwrap();
        assert_eq!(prune_staging(&staging).unwrap(), 2);
        staging
    }

    #[test]
    fn snapshot_reports_install_state_from_the_record() {
        let root = temp_root("snapshot-target");
        let state = temp_root("snapshot-state");
        let empty = snapshot(&state, &root);
        assert!(!empty.plugin_present);
        assert!(empty.needs_install);
        assert!(empty.installed_version.is_none());

        let staging = staged_tree("snapshot");
        commit_staged(&staging, &state, &root, "3.7.0").unwrap();
        let installed = snapshot(&state, &root);
        assert!(installed.plugin_present);
        assert!(!installed.needs_install);
        assert_eq!(installed.installed_version.as_deref(), Some("3.7.0"));

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&state).unwrap();
        fs::remove_dir_all(&staging).unwrap();
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
    fn synthetic_manifest_carries_exactly_the_whitelisted_files() {
        let staging = temp_staging("manifest");
        let plugin_dll = staging.join(PLUGIN_DIR).join("InventorySimulator.dll");
        fs::create_dir_all(plugin_dll.parent().unwrap()).unwrap();
        fs::write(&plugin_dll, b"plugin").unwrap();
        let gamedata = staging.join(GAMEDATA_FILE);
        fs::create_dir_all(gamedata.parent().unwrap()).unwrap();
        fs::write(&gamedata, b"gamedata").unwrap();
        let unrelated = staging.join("Unrelated.dll");
        fs::write(&unrelated, b"unrelated").unwrap();

        let dll_relative = format!("{PLUGIN_DIR}InventorySimulator.dll");
        let manifest = synthetic_manifest(&staging, "3.7.0").unwrap();

        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.package_version, "3.7.0");
        assert_eq!(
            manifest.entries.len(),
            2,
            "one entry per whitelisted file, and nothing else"
        );
        let paths: Vec<&str> = manifest
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();
        assert_eq!(
            paths,
            vec![GAMEDATA_FILE, dll_relative.as_str()],
            "the manifest covers exactly the whitelisted files"
        );
        assert!(
            manifest
                .entries
                .iter()
                .all(|entry| !entry.path.contains(".installed.json")),
            "the retired marker is not a manifest entry any more"
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

    fn record_path(state: &Path) -> PathBuf {
        let installations = state.join("installations");
        let mut dirs: Vec<PathBuf> = fs::read_dir(&installations)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        dirs.sort();
        dirs.pop().unwrap().join("record.json")
    }

    #[test]
    fn is_version_comes_from_record() {
        // A `.installed.json` an older Panel left behind is not a version source: only the
        // component record is.
        let root = temp_root("version-source-target");
        let state = temp_root("version-source-state");
        let staging = staged_tree("version-source");
        commit_staged(&staging, &state, &root, "3.7.0").unwrap();

        let legacy_marker = root.join(PLUGIN_DIR).join(".installed.json");
        fs::write(
            &legacy_marker,
            serde_json::to_vec(&serde_json::json!({"tag": "9.9.9"})).unwrap(),
        )
        .unwrap();
        assert_eq!(
            snapshot(&state, &root).installed_version.as_deref(),
            Some("3.7.0"),
            "the record wins over a leftover marker"
        );

        let other_state = temp_root("version-source-state-empty");
        assert_eq!(
            snapshot(&other_state, &root).installed_version,
            None,
            "without a record the leftover marker reports nothing at all"
        );

        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&state).unwrap();
        fs::remove_dir_all(&other_state).unwrap();
        fs::remove_dir_all(&staging).unwrap();
    }

    #[test]
    fn task7_happy() {
        let root = temp_root("task7-happy-target");
        let state = temp_root("task7-happy-state");
        let staging = staged_tree("task7-happy");

        commit_staged(&staging, &state, &root, "3.7.2").unwrap();

        assert!(root.join(PLUGIN_DIR).join("InventorySimulator.dll").is_file());
        assert!(root.join(GAMEDATA_FILE).is_file());
        assert!(!root.join(PLUGIN_DIR).join(".installed.json").exists());
        assert_eq!(
            crate::installer::component_version(
                &state,
                &root,
                crate::installer::ComponentId::InventorySimulator
            )
            .as_deref(),
            Some("3.7.2")
        );
        assert_eq!(
            snapshot(&state, &root).installed_version.as_deref(),
            Some("3.7.2")
        );
        println!(
            "task7_happy: installed_version={:?} plugin_present={}",
            snapshot(&state, &root).installed_version,
            snapshot(&state, &root).plugin_present
        );
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&state).unwrap();
        fs::remove_dir_all(&staging).unwrap();
    }

    #[test]
    fn task7_failure() {
        // (a) A staging tree with nothing whitelisted never reaches the game directory.
        let root = temp_root("task7-failure-empty-target");
        let state = temp_root("task7-failure-empty-state");
        let staging = temp_staging("task7-empty");
        fs::write(staging.join("Unrelated.dll"), b"unrelated").unwrap();
        assert_eq!(prune_staging(&staging).unwrap(), 0);
        let empty = commit_staged(&staging, &state, &root, "3.7.0").unwrap_err();
        assert_eq!(empty.code, "E1301", "{}", empty.detail);
        assert!(!root.join(PLUGIN_DIR).join("InventorySimulator.dll").exists());
        assert!(snapshot(&state, &root).installed_version.is_none());
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&state).unwrap();
        fs::remove_dir_all(&staging).unwrap();

        // (b) A conflicting file in the game directory aborts before any write.
        let root = temp_root("task7-failure-conflict-target");
        let state = temp_root("task7-failure-conflict-state");
        let staging = staged_tree("task7-conflict");
        let occupied = root.join(PLUGIN_DIR).join("InventorySimulator.dll");
        fs::create_dir_all(occupied.parent().unwrap()).unwrap();
        fs::write(&occupied, b"another-build").unwrap();
        let conflict = commit_staged(&staging, &state, &root, "3.7.0").unwrap_err();
        assert_eq!(conflict.code, "E1403", "{}", conflict.detail);
        assert_eq!(fs::read(&occupied).unwrap(), b"another-build");
        assert!(snapshot(&state, &root).installed_version.is_none());
        println!(
            "task7_failure: empty={} conflict={} lines={}",
            empty.code,
            conflict.code,
            conflict.detail.lines().count()
        );
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&state).unwrap();
        fs::remove_dir_all(&staging).unwrap();
    }

    #[test]
    fn task7_manual_qa_dump() {
        let root = temp_root("task7-qa-target");
        let state = temp_root("task7-qa-state");
        let staging = staged_tree("task7-qa");
        commit_staged(&staging, &state, &root, "3.7.0").unwrap();

        let bytes = fs::read(record_path(&state)).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            doc["components"]["inventory_simulator"]["version"].as_str(),
            Some("3.7.0")
        );
        assert_eq!(
            doc["components"]["inventory_simulator"]["ownership"].as_str(),
            Some("third-party")
        );
        let paths: Vec<&str> = doc["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        assert!(
            paths.iter().all(|path| !path.contains(".installed.json")),
            "no marker path in the install record: {paths:?}"
        );
        if let Ok(dump) = std::env::var("QA_DUMP") {
            fs::write(&dump, &bytes).unwrap();
            println!(
                "task7_manual_qa_dump: wrote {} bytes to {} (paths={:?})",
                bytes.len(),
                dump,
                paths
            );
        } else {
            println!("task7_manual_qa_dump: QA_DUMP unset, skipped dump");
        }
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&state).unwrap();
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
