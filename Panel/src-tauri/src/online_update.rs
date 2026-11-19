//! Panel and component updates.
//!
//! The Panel itself is updated by `tauri-plugin-updater`: the plugin reads the
//! Tauri `latest.json` published with the release, verifies the bundle signature
//! against the embedded public key, and installs the NSIS/AppImage payload.
//! Component payloads (the LA plugin archive) cannot be expressed as Tauri
//! bundles, so they come straight from the release assets through
//! [`release_source`] — the same digest-verified path InventorySimulator uses.

use crate::{
    AppError, Result, app_storage, app_version, atomic_fs, component_registry,
    installer, release_source, update_core,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

const CACHE_SECONDS: u64 = 6 * 60 * 60;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

static CANCELLED: AtomicBool = AtomicBool::new(false);
static RUNTIME: OnceLock<Mutex<RuntimeState>> = OnceLock::new();

#[derive(Clone, Debug, Default)]
struct RuntimeState {
    busy: bool,
    progress: Option<UpdateProgress>,
    last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateComponentState {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub compatible: bool,
    pub status: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OnlineUpdateSnapshot {
    pub checked_at: Option<u64>,
    pub release_version: Option<String>,
    pub release_notes_url: Option<String>,
    pub panel: UpdateComponentState,
    pub la_plugins: UpdateComponentState,
    pub busy: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateProgress {
    pub component: String,
    pub stage: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateResult {
    pub component: String,
    pub version: String,
    pub installed: bool,
    pub restart_required: bool,
    pub rollback_succeeded: Option<bool>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateBatchResult {
    pub panel: Option<UpdateResult>,
    pub la_plugins: Option<UpdateResult>,
    /// One entry per component, in install order (upstream, la_plugins, inventory_simulator).
    pub components: Vec<UpdateResult>,
    pub restart_required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PanelCheckCache {
    schema_version: u8,
    checked_at: u64,
    latest_version: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PluginCheckCache {
    schema_version: u8,
    checked_at: u64,
    asset: release_source::ResolvedAsset,
}

fn runtime() -> &'static Mutex<RuntimeState> {
    RUNTIME.get_or_init(|| Mutex::new(RuntimeState::default()))
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn update_root() -> Result<PathBuf> {
    let root = app_storage::root()?.join("updates");
    fs::create_dir_all(&root).map_err(AppError::transaction_io)?;
    Ok(root)
}

pub(crate) fn client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(format!("LocalArena/{}", app_version::display()))
        .https_only(true)
        .build()
        .map_err(|error| AppError::update(format!("Cannot initialize update client: {error}")))
}

fn is_fresh(checked_at: u64) -> bool {
    unix_time().saturating_sub(checked_at) < CACHE_SECONDS
}

fn panel_cache_path() -> Result<PathBuf> {
    Ok(update_root()?.join("panel-check.json"))
}

fn read_panel_cache() -> Result<Option<PanelCheckCache>> {
    let path = panel_cache_path()?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(AppError::transaction_io(error)),
    };
    let cache: PanelCheckCache = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::update(format!("Invalid Panel update cache: {error}")))?;
    if cache.schema_version != 1 {
        return Ok(None);
    }
    Ok(Some(cache))
}

fn write_panel_cache(latest_version: Option<String>, checked_at: u64) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(&PanelCheckCache {
        schema_version: 1,
        checked_at,
        latest_version,
    })
    .map_err(|error| AppError::update(error.to_string()))?;
    atomic_fs::write_replace(&panel_cache_path()?, &bytes).map_err(AppError::transaction_io)
}

fn plugin_cache_path() -> Result<PathBuf> {
    Ok(update_root()?.join("plugin-check.json"))
}

fn read_plugin_cache() -> Result<Option<PluginCheckCache>> {
    let path = plugin_cache_path()?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(AppError::transaction_io(error)),
    };
    let cache: PluginCheckCache = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::update(format!("Invalid plugin update cache: {error}")))?;
    if cache.schema_version != 1 {
        return Ok(None);
    }
    Ok(Some(cache))
}

fn write_plugin_cache(asset: &release_source::ResolvedAsset, checked_at: u64) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(&PluginCheckCache {
        schema_version: 1,
        checked_at,
        asset: asset.clone(),
    })
    .map_err(|error| AppError::update(error.to_string()))?;
    atomic_fs::write_replace(&plugin_cache_path()?, &bytes).map_err(AppError::transaction_io)
}

fn plugin_selector() -> release_source::AssetSelector {
    component_registry::AssetName::LaPluginBundle.selector()
}

fn resolve_plugin_release() -> Result<Option<release_source::ResolvedAsset>> {
    let client = client(REQUEST_TIMEOUT)?;
    release_source::resolve_latest_optional(
        &client,
        component_registry::LA_PLUGIN_REPOSITORY,
        &plugin_selector(),
    )
}

fn plugin_asset(force: bool) -> Result<Option<release_source::ResolvedAsset>> {
    if !force {
        if let Some(cache) = read_plugin_cache()? {
            if is_fresh(cache.checked_at) {
                return Ok(Some(cache.asset));
            }
        }
    }
    let asset = resolve_plugin_release()?;
    if let Some(asset) = asset.as_ref() {
        write_plugin_cache(asset, unix_time())?;
    }
    Ok(asset)
}

fn version_from_tag(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

fn newer(latest: &str, current: &str) -> bool {
    match (
        update_core::DisplayVersion::parse(current),
        update_core::DisplayVersion::parse(latest),
    ) {
        (Ok(current), Ok(latest)) => latest > current,
        _ => false,
    }
}

fn component_state(
    current: &str,
    latest: Option<&str>,
    compatible: bool,
    total_bytes: u64,
    selected_progress: Option<&UpdateProgress>,
    error: Option<String>,
) -> UpdateComponentState {
    let update_available = latest.is_some_and(|latest| newer(latest, current));
    let (status, downloaded_bytes, total_bytes) = if let Some(progress) = selected_progress {
        (
            progress.stage.clone(),
            progress.downloaded_bytes,
            progress.total_bytes,
        )
    } else if error.is_some() {
        ("error".into(), 0, total_bytes)
    } else if update_available {
        (
            if compatible {
                "available"
            } else {
                "panel-required"
            }
            .into(),
            0,
            total_bytes,
        )
    } else {
        ("current".into(), 0, total_bytes)
    };
    UpdateComponentState {
        current_version: current.into(),
        latest_version: latest.map(str::to_owned),
        update_available,
        compatible,
        status,
        downloaded_bytes,
        total_bytes,
        error,
    }
}

/// Version of the installed Local-Arena plugin bundle.
///
/// The caller (`lib.rs::installed_plugin_version`) only reports a version when the
/// merged payload actually carries an install record, so `None` means "nothing is
/// installed". The Panel's own version must never stand in for it: on a clean machine
/// that would report the bundle as installed and offer "Update" instead of "Install".
fn la_plugins_current(installed: Option<&str>) -> Option<String> {
    installed.map(str::to_owned)
}

pub fn snapshot(plugin_version: Option<&str>) -> Result<OnlineUpdateSnapshot> {
    let state = runtime()
        .lock()
        .map_err(|_| AppError::update("Update state lock is poisoned"))?
        .clone();
    let panel_cache = read_panel_cache()?;
    let plugin_cache = read_plugin_cache()?;
    let checked_at = [panel_cache.as_ref().map(|cache| cache.checked_at), plugin_cache.as_ref().map(|cache| cache.checked_at)]
        .into_iter()
        .flatten()
        .max();

    let panel_latest = panel_cache
        .as_ref()
        .and_then(|cache| cache.latest_version.as_deref());
    let plugin_latest = plugin_cache
        .as_ref()
        .map(|cache| version_from_tag(&cache.asset.tag));
    let plugin_size = plugin_cache
        .as_ref()
        .map(|cache| cache.asset.size)
        .unwrap_or(0);

    let panel_progress = state
        .progress
        .as_ref()
        .filter(|value| value.component == "panel");
    let la_plugins_progress = state
        .progress
        .as_ref()
        .filter(|value| value.component == "la_plugins");
    // Empty means "not installed" — see `la_plugins_current`.
    let plugin_current_version = la_plugins_current(plugin_version).unwrap_or_default();
    Ok(OnlineUpdateSnapshot {
        checked_at,
        release_version: panel_latest.map(str::to_owned),
        release_notes_url: panel_latest.map(|version| {
            format!(
                "https://github.com/{}/releases/tag/v{}",
                component_registry::LA_PLUGIN_REPOSITORY,
                version.trim_start_matches(['v', 'V'])
            )
        }),
        panel: component_state(
            app_version::display(),
            panel_latest,
            true,
            0,
            panel_progress,
            state.last_error.clone(),
        ),
        la_plugins: component_state(
            &plugin_current_version,
            plugin_latest.as_deref(),
            true,
            plugin_size,
            la_plugins_progress,
            state.last_error.clone(),
        ),
        busy: state.busy,
        error: state.last_error,
    })
}

async fn panel_update(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>> {
    let updater = app
        .updater()
        .map_err(|error| AppError::update(format!("Updater is not configured: {error}")))?;
    updater
        .check()
        .await
        .map_err(|error| AppError::update(format!("Cannot check the Panel update feed: {error}")))
}

pub async fn check(
    app: &AppHandle,
    force: bool,
    plugin_version: Option<&str>,
) -> Result<OnlineUpdateSnapshot> {
    let panel_cache = read_panel_cache()?;
    let panel_needs_check = force
        || panel_cache
            .as_ref()
            .is_none_or(|cache| !is_fresh(cache.checked_at));
    if panel_needs_check {
        let checked_at = unix_time();
        let latest = match panel_update(app).await? {
            Some(update) => update.version,
            None => app_version::display().to_string(),
        };
        write_panel_cache(Some(latest), checked_at)?;
    }

    let plugin_cache = read_plugin_cache()?;
    let plugin_needs_check = force
        || plugin_cache
            .as_ref()
            .is_none_or(|cache| !is_fresh(cache.checked_at));
    if plugin_needs_check {
        let asset = tauri::async_runtime::spawn_blocking(resolve_plugin_release)
            .await
            .map_err(|error| AppError::update(format!("Plugin release task failed: {error}")))??;
        if let Some(asset) = asset {
            write_plugin_cache(&asset, unix_time())?;
        }
    }

    if let Ok(mut state) = runtime().lock() {
        state.last_error = None;
    }
    snapshot(plugin_version)
}

pub fn record_check_error(error: &AppError) {
    if let Ok(mut state) = runtime().lock() {
        state.last_error = Some(error.detail.clone());
    }
}

fn set_progress(app: &AppHandle, value: UpdateProgress) {
    if let Ok(mut state) = runtime().lock() {
        state.progress = Some(value.clone());
    }
    let _ = app.emit("update-progress", value);
}

pub(crate) struct OperationGuard;
impl OperationGuard {
    pub(crate) fn acquire() -> Result<Self> {
        let mut state = runtime()
            .lock()
            .map_err(|_| AppError::update("Update state lock is poisoned"))?;
        if state.busy {
            return Err(AppError::update(
                "Another install or update operation is already running",
            ));
        }
        state.busy = true;
        state.last_error = None;
        CANCELLED.store(false, Ordering::Release);
        Ok(Self)
    }
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = runtime().lock() {
            state.busy = false;
            state.progress = None;
        }
    }
}

/// Installs the signed Tauri bundle for the Panel.
///
/// The Tauri updater verifies the bundle signature before extraction and installs
/// it through the platform installer (NSIS on Windows, AppImage replacement on
/// Linux). The caller restarts the application afterwards.
pub async fn install_panel(app: &AppHandle) -> Result<UpdateResult> {
    let update = panel_update(app)
        .await?
        .ok_or_else(|| AppError::update("The Panel is already up to date"))?;
    let version = update.version.clone();
    let progress_app = app.clone();
    let mut downloaded = 0_u64;
    update
        .download_and_install(
            move |chunk_length, content_length| {
                downloaded += chunk_length as u64;
                set_progress(
                    &progress_app,
                    UpdateProgress {
                        component: "panel".into(),
                        stage: "downloading".into(),
                        downloaded_bytes: downloaded,
                        total_bytes: content_length.unwrap_or(0),
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|error| AppError::update(format!("Cannot install the Panel update: {error}")))?;
    write_panel_cache(Some(version.clone()), unix_time())?;
    Ok(UpdateResult {
        component: "panel".into(),
        version,
        installed: true,
        restart_required: true,
        rollback_succeeded: None,
        detail: "Panel update installed; restart required".into(),
    })
}

pub(crate) fn clear_directory(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(AppError::transaction_io)?;
    }
    fs::create_dir_all(path).map_err(AppError::transaction_io)
}

/// Shared cancellation flag for downloads driven by `cancel()`; the upstream
/// package downloader polls the same flag so one cancel button covers both.
pub(crate) fn cancelled() -> bool {
    CANCELLED.load(Ordering::Acquire)
}

fn find_payload_root(extracted: &Path) -> Option<PathBuf> {
    if extracted.join(installer::MANIFEST_FILE).is_file() {
        return Some(extracted.to_path_buf());
    }
    fs::read_dir(extracted)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path())
        .find(|path| path.join(installer::MANIFEST_FILE).is_file())
}

pub fn prepare_plugin(app: &AppHandle, target: &Path) -> Result<(String, PathBuf)> {
    let asset = plugin_asset(false)?.ok_or_else(|| {
        AppError::update("This release has no LA plugin archive for this platform")
    })?;
    let version = version_from_tag(&asset.tag);
    let directory = update_root()?.join("downloads").join(&version);
    fs::create_dir_all(&directory).map_err(AppError::transaction_io)?;
    let archive = directory.join("la_plugins.zip");
    // The shared downloader reuses a verified archive, resumes `<dest>.part`,
    // and honours the same cancellation flag as the rest of the Panel.
    crate::download::download_with_progress(
        &client(DOWNLOAD_TIMEOUT)?,
        &asset.url,
        asset.size,
        &asset.sha256,
        &archive,
        |downloaded_bytes, total_bytes| {
            set_progress(
                app,
                UpdateProgress {
                    component: "la_plugins".into(),
                    stage: "downloading".into(),
                    downloaded_bytes,
                    total_bytes,
                },
            );
        },
        cancelled,
    )?;

    set_progress(
        app,
        UpdateProgress {
            component: "la_plugins".into(),
            stage: "extracting".into(),
            downloaded_bytes: asset.size,
            total_bytes: asset.size,
        },
    );
    let directory = update_root()?.join("payloads").join(&version);
    clear_directory(&directory)?;
    let file = fs::File::open(archive).map_err(AppError::transaction_io)?;
    if let Err(error) = update_core::extract_zip_safely(file, &directory) {
        let _ = fs::remove_dir_all(&directory);
        return Err(AppError::update(error));
    }
    let plugin_root = find_payload_root(&directory)
        .ok_or_else(|| AppError::payload("Plugin update ZIP has no payload manifest"))?;
    let manifest = installer::verify_payload(&plugin_root)?;
    if manifest.package_version != version {
        return Err(AppError::payload(
            "Plugin payload version does not match the published release",
        ));
    }
    // Decision #7: a plugin update is installed from an upstream + LA merge.
    // The overlay source is the freshly downloaded plugin payload, not the
    // possibly older package beside the executable.
    let upstream = crate::upstream_package::ensure_cached(app)?;
    let staging_root = crate::upstream_package::merged_root(&upstream.release.tag)?;
    let merged = crate::upstream_package::merge(&crate::upstream_package::MergeInput {
        upstream_zip: upstream.zip,
        la_payload: plugin_root.clone(),
        staging_root,
    })?;
    installer::verify_payload(&merged)?;
    // The pure LA payload root is what later local installs, repairs and merges resolve from —
    // never the merge's own output. It is recorded on the component's row, replacing the pointer
    // file the earlier design kept in the update cache; a machine with no record yet is skipped,
    // and the install that follows records its own row.
    installer::set_component_path(
        &app_storage::root()?,
        target,
        installer::ComponentId::LaPlugins,
        &plugin_root,
    )?;
    cleanup_cache(Some(&version))?;
    Ok((version, merged))
}

/// Path of the pointer file this design retired.
fn legacy_active_payload_path() -> Result<PathBuf> {
    Ok(update_root()?.join("active-payload.json"))
}

/// Deletes the retired payload pointer.
///
/// The LA payload directory now lives in the install record, so a Panel that finds the old file
/// simply removes it. It is never parsed: a half-written or hand-edited pointer from an older
/// build must not be able to redirect an install, and its absence is the normal case.
pub fn retire_legacy_active_payload() -> Result<bool> {
    let path = legacy_active_payload_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(AppError::transaction_io(error)),
    }
}

pub fn cancel() {
    CANCELLED.store(true, Ordering::Release);
}

pub fn cleanup_cache(active_version: Option<&str>) -> Result<usize> {
    let root = update_root()?;
    // Files written by the retired signed-manifest updater.
    for legacy in ["latest.json", "latest.json.sig", "cache.json"] {
        let _ = fs::remove_file(root.join(legacy));
    }
    let mut versions = fs::read_dir(root.join("payloads"))
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            (
                entry
                    .metadata()
                    .and_then(|value| value.modified())
                    .unwrap_or(UNIX_EPOCH),
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .collect::<Vec<_>>();
    versions.sort_by_key(|value| std::cmp::Reverse(value.0));
    let mut keep = versions
        .iter()
        .find(|value| active_version == Some(value.1.as_str()))
        .map(|value| vec![value.2.clone()])
        .unwrap_or_default();
    for (_, _, path) in &versions {
        if keep.len() >= 2 { break; }
        if !keep.contains(path) { keep.push(path.clone()); }
    }
    let mut removed = 0;
    for (_, _, path) in versions {
        if !keep.contains(&path) && fs::remove_dir_all(path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}-{}", std::process::id(), unix_time()))
    }

    #[test]
    fn operation_guard_rejects_overlap_and_releases_on_drop() {
        let first = OperationGuard::acquire().unwrap();
        assert!(OperationGuard::acquire().is_err());
        drop(first);
        assert!(OperationGuard::acquire().is_ok());
    }

    #[test]
    fn legacy_active_payload_file_is_deleted() {
        // The retired pointer is removed on the next run and never parsed: a half-written or
        // hand-edited one from an older build must not be able to redirect an install, and its
        // absence is the normal case.
        let _guard = app_storage::TEST_STATE_ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let base = test_root("localarena-legacy-active-payload");
        unsafe { std::env::set_var("CS2BI_STATE_ROOT", &base); }
        let updates = base.join("updates");
        fs::create_dir_all(&updates).unwrap();
        assert!(
            !retire_legacy_active_payload().unwrap(),
            "an absent pointer is not an error"
        );

        let pointer = legacy_active_payload_path().unwrap();
        fs::write(&pointer, b"{ half written").unwrap();
        assert!(retire_legacy_active_payload().unwrap());
        assert!(!pointer.exists(), "the legacy pointer is gone");
        assert!(!retire_legacy_active_payload().unwrap(), "deletion is idempotent");

        fs::remove_dir_all(&base).unwrap();
        unsafe { std::env::remove_var("CS2BI_STATE_ROOT"); }
    }

    #[test]
    fn la_plugins_current_never_substitutes_the_panel_version() {
        // A clean machine has no install record: the version stays unknown rather than
        // borrowing the Panel's own version (which would report the bundle as installed).
        assert_eq!(la_plugins_current(None), None);
        let current = la_plugins_current(None).unwrap_or_default();
        assert!(current.is_empty());
        assert_ne!(current, app_version::display());
    }

    #[test]
    fn la_plugins_current_reports_the_installed_version_verbatim() {
        assert_eq!(
            la_plugins_current(Some("1.5.0")),
            Some("1.5.0".to_owned())
        );
    }

    #[test]
    fn version_from_tag_strips_the_v_prefix() {
        assert_eq!(version_from_tag("v1.5.0"), "1.5.0");
        assert_eq!(version_from_tag("1.5.0"), "1.5.0");
        assert_eq!(version_from_tag("v1.5.0-Preview.1"), "1.5.0-Preview.1");
    }

    #[test]
    fn newer_uses_semver_ordering() {
        assert!(newer("1.5.0", "1.4.9"));
        assert!(newer("1.5.0", "1.5.0-Preview.2"));
        assert!(!newer("1.5.0-Preview.1", "1.5.0"));
        assert!(!newer("1.5.0", "1.5.0"));
        assert!(!newer("1.5.0", ""));
    }

    #[test]
    fn snapshot_without_a_probe_reports_no_update() {
        let _guard = app_storage::TEST_STATE_ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let base = test_root("localarena-snapshot-clean");
        unsafe { std::env::set_var("CS2BI_STATE_ROOT", &base); }
        let snapshot = snapshot(None).unwrap();
        assert_eq!(snapshot.panel.current_version, app_version::display());
        assert!(!snapshot.panel.update_available);
        assert!(snapshot.la_plugins.current_version.is_empty());
        assert!(!snapshot.la_plugins.update_available);
        assert!(snapshot.la_plugins.latest_version.is_none());
        fs::remove_dir_all(&base).ok();
        unsafe { std::env::remove_var("CS2BI_STATE_ROOT"); }
    }

    #[test]
    fn cleanup_keeps_only_active_and_one_previous_payload() {
        let _guard = app_storage::TEST_STATE_ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let base = test_root("localarena-update-cleanup");
        unsafe { std::env::set_var("CS2BI_STATE_ROOT", &base); }
        let payloads = base.join("updates/payloads");
        for version in ["1.4.2", "1.4.3", "1.5.0"] {
            fs::create_dir_all(payloads.join(version)).unwrap();
        }
        assert_eq!(cleanup_cache(Some("1.4.2")).unwrap(), 1);
        let remaining = fs::read_dir(&payloads).unwrap().flatten().count();
        assert_eq!(remaining, 2);
        assert!(payloads.join("1.4.2").is_dir());
        fs::remove_dir_all(&base).unwrap();
        unsafe { std::env::remove_var("CS2BI_STATE_ROOT"); }
    }

    #[test]
    fn component_state_flags_only_real_upgrades() {
        let state = component_state("1.4.9", Some("1.5.0"), true, 100, None, None);
        assert!(state.update_available);
        assert_eq!(state.status, "available");
        assert_eq!(state.total_bytes, 100);

        let current = component_state("1.5.0", Some("1.5.0"), true, 100, None, None);
        assert!(!current.update_available);
        assert_eq!(current.status, "current");
    }
}
