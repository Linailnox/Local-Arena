use crate::{AppError, Result, atomic_fs, mode_layout};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MANIFEST_FILE: &str = "plus-payload-manifest.json";
pub const PANEL_UPDATE_MARKER: &str = "csbip-panel-update.json";

const SUPERSEDED_PAYLOAD_PATHS: &[(&str, &str)] = &[
    (
        "addons/counterstrikesharp/plugins/PlusMatchCoordinator/rating-plus-3.0-proxy-v1.json",
        "addons/counterstrikesharp/plugins/PlusMatchCoordinator/open-rating-3.0-proxy-v1.json",
    ),
];

const PLUS_MARKERS: &[&str] = &[
    "addons/counterstrikesharp/plugins/PlusMatchCoordinator/PlusMatchCoordinator.dll",
    "addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.dll",
    "addons/counterstrikesharp/plugins/OfflineMatchTelemetry/OfflineMatchTelemetry.dll",
];

const UPSTREAM_MARKERS: &[&str] = &[
    "addons/counterstrikesharp/plugins/BotAI/BotAI.dll",
    "addons/counterstrikesharp/plugins/BotAimImprover/BotAimImprover.dll",
    "addons/counterstrikesharp/plugins/BotBuy/BotBuy.dll",
    "addons/counterstrikesharp/plugins/BotRandomizer/BotRandomizer.dll",
    "addons/counterstrikesharp/plugins/BotState/BotState.dll",
    "addons/counterstrikesharp/plugins/NadeSystem/NadeSystem.dll",
    "addons/metamod/BotHider.vdf",
];

const SUITE_OWNED_ROOTS: &[&str] = &[
    "addons/BotHider",
    "addons/BotController",
    "addons/RayTrace",
    "addons/counterstrikesharp/api",
    "addons/counterstrikesharp/bin",
    "addons/counterstrikesharp/dotnet",
    "addons/counterstrikesharp/gamedata",
    "addons/counterstrikesharp/lang",
    "addons/counterstrikesharp/shared",
    "addons/counterstrikesharp/source",
    "addons/counterstrikesharp/plugins/BotAI",
    "addons/counterstrikesharp/plugins/BotAimImprover",
    "addons/counterstrikesharp/plugins/BotBuy",
    "addons/counterstrikesharp/plugins/BotControllerImpl",
    "addons/counterstrikesharp/plugins/BotHiderImpl",
    "addons/counterstrikesharp/plugins/BotRandomizer",
    "addons/counterstrikesharp/plugins/BotState",
    "addons/counterstrikesharp/plugins/NadeSystem",
    "addons/counterstrikesharp/plugins/InventorySimulator",
    "addons/counterstrikesharp/plugins/RayTraceImpl",
    "addons/counterstrikesharp/plugins/RoundDamageRecap",
    "addons/counterstrikesharp/plugins/TeamLineupInjector",
    "addons/counterstrikesharp/plugins/OfflineMatchTelemetry",
    "addons/counterstrikesharp/plugins/disabled/BotAI_for_Linux",
    "addons/counterstrikesharp/plugins/disabled/BotAimImprover_for_Linux",
    "addons/metamod/bin",
    "overrides",
];

const SUITE_OWNED_FILES: &[&str] = &[
    "addons/metamod_x64.vdf",
    "addons/metamod/BotController.vdf",
    "addons/metamod/BotHider.vdf",
    "addons/metamod/RayTrace.vdf",
    "addons/metamod/counterstrikesharp.vdf",
    "addons/metamod/metaplugins.ini",
    "cfg/bot_buy.cfg",
    "cfg/my_bot_ffa_config.cfg",
    "cfg/my_bot_ffa_config_rules_unchanged.cfg",
    "cfg/my_bot_normal_config.cfg",
    "cfg/my_bot_normal_config_rules_unchanged.cfg",
];

const FILE_RETRY_DELAYS: &[Duration] = &[
    Duration::from_millis(80),
    Duration::from_millis(200),
    Duration::from_millis(450),
    Duration::from_millis(900),
    Duration::from_millis(1_400),
    Duration::from_millis(2_200),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationSource {
    Clean,
    ManagedPlus,
    LegacyPlus,
    Upstream,
    MixedUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationKind {
    FreshInstall,
    ManagedUpgrade,
    AdoptLegacyPlus,
    ReplaceUpstream,
    Blocked,
}

fn welcome_story_prompt_eligible(
    release_build: bool,
    full_package: bool,
    migration_kind: MigrationKind,
    repaired: bool,
) -> bool {
    release_build
        && full_package
        && !repaired
        && matches!(
            migration_kind,
            MigrationKind::FreshInstall
                | MigrationKind::AdoptLegacyPlus
                | MigrationKind::ReplaceUpstream
        )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreBaseline {
    SteamOriginal,
    PreMigration,
    ExistingRecord,
    None,
}

fn existing_record_baseline() -> RestoreBaseline {
    RestoreBaseline::ExistingRecord
}
fn managed_source() -> InstallationSource {
    InstallationSource::ManagedPlus
}

/// Stable schema version for `record.json`. Full-payload installs have written `2` since the
/// source/baseline migration; the per-component `components` map lives within this same version,
/// and legacy records (which lack that field) are normalized to it on first read.
const RECORD_SCHEMA_VERSION: u32 = 2;

/// Version sentinel a migrated legacy record carries for a component it never spoke for. It is a
/// literal, never a guessed release tag or timestamp, and callers treat it as "not installed".
pub const UNKNOWN_VERSION: &str = "unknown";

/// Identifies one of the three peer components whose versions `record.json` tracks.
/// `pub` so the upstream / LA-bundle / InventorySimulator install paths (plan todos 3 & 4) can
/// address a component without duplicating string keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentId {
    Upstream,
    LaPlugins,
    InventorySimulator,
}

/// Per-component version entry. `path` is an optional pointer to the LA bundle's download
/// source; migrated legacy records have no such pointer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentRecord {
    pub version: String,
    pub source: String,
    pub installed_at: u64,
    pub ownership: String,
    pub path: Option<String>,
}

impl ComponentRecord {
    /// Placeholder for a component a v1 record never spoke for: the version is the literal
    /// sentinel [`UNKNOWN_VERSION`], never a guessed release tag or timestamp.
    fn unknown() -> Self {
        Self {
            version: UNKNOWN_VERSION.to_string(),
            source: "unknown".to_string(),
            installed_at: 0,
            ownership: "unknown".to_string(),
            path: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PayloadManifest {
    pub schema_version: u32,
    pub package_version: String,
    pub entries: Vec<PayloadEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PayloadEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub component: String,
    pub ownership: String,
    pub restore_policy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstallRecordEntry {
    path: String,
    original_existed: bool,
    original_sha256: Option<String>,
    original_backup: Option<String>,
    installed_sha256: String,
    ownership: String,
    restore_policy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct InstallRecord {
    schema_version: u32,
    installation_id: String,
    target: String,
    package_version: String,
    installed_at: u64,
    #[serde(default = "managed_source")]
    source: InstallationSource,
    #[serde(default = "existing_record_baseline")]
    restore_baseline: RestoreBaseline,
    #[serde(default)]
    migrated_from_version: Option<String>,
    /// Per-component version view (schema 2). `default` keeps every pre-existing fixture and
    /// journal record parseable; a missing `components` key marks a legacy v1 record to migrate.
    #[serde(default)]
    components: BTreeMap<ComponentId, ComponentRecord>,
    entries: Vec<InstallRecordEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TransactionChange {
    path: String,
    existed: bool,
    backup: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TransactionJournal {
    schema_version: u32,
    operation: String,
    target: String,
    transaction_dir: String,
    changes: Vec<TransactionChange>,
    #[serde(default)]
    committed: bool,
    #[serde(default)]
    previous_record: Option<InstallRecord>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallationInspection {
    pub installed: bool,
    pub package_version: Option<String>,
    pub manifest_available: bool,
    pub total: usize,
    pub healthy: usize,
    pub missing: Vec<String>,
    pub corrupt: Vec<String>,
    pub restore_available: bool,
    pub backup_path: Option<String>,
    pub backup_display: Option<String>,
    pub interrupted_transaction: bool,
    pub source: InstallationSource,
    pub source_version: Option<String>,
    pub source_evidence: Vec<String>,
    pub migration_kind: MigrationKind,
    pub restore_baseline: RestoreBaseline,
    pub can_install: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallPlan {
    pub package_version: String,
    pub target: String,
    pub total_files: usize,
    pub new_files: usize,
    pub overwritten_files: usize,
    pub backup_path: String,
    pub required_target_bytes: u64,
    pub available_target_bytes: u64,
    pub required_backup_bytes: u64,
    pub available_backup_bytes: u64,
    pub writable: bool,
    pub source: InstallationSource,
    pub source_version: Option<String>,
    pub source_evidence: Vec<String>,
    pub migration_kind: MigrationKind,
    pub restore_baseline: RestoreBaseline,
    pub can_install: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallTransactionResult {
    pub package_version: String,
    pub installed_files: usize,
    pub backup_path: String,
    pub repaired: bool,
    pub welcome_story_eligible: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct RestoreResult {
    pub restored_files: usize,
    pub removed_files: usize,
    pub preserved_files: usize,
    pub presets_backup: Option<String>,
    pub steam_verify_uri: String,
    pub result_kind: String,
}

#[derive(Clone, Debug)]
struct SourceDetection {
    source: InstallationSource,
    version: Option<String>,
    evidence: Vec<String>,
    migration_kind: MigrationKind,
    restore_baseline: RestoreBaseline,
    can_install: bool,
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|error| AppError::payload(error.to_string()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| AppError::payload(error.to_string()))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn safe_relative(raw: &str) -> Result<PathBuf> {
    // Validate per '/' segment so the manifest's Windows-style rules (absolute
    // paths, parent traversal, drive prefixes, literal backslashes) hold on
    // every platform, then build a real nested path for the host OS instead of
    // treating the whole entry as one backslashed filename on Unix.
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(AppError::payload(format!("Unsafe payload path: {raw}")));
    }
    let mut path = PathBuf::new();
    for segment in trimmed.split('/') {
        let drive_prefix = segment.len() >= 2
            && segment.as_bytes()[0].is_ascii_alphabetic()
            && segment.as_bytes()[1] == b':';
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || drive_prefix
            || segment.contains('\\')
        {
            return Err(AppError::payload(format!("Unsafe payload path: {raw}")));
        }
        path.push(segment);
    }
    Ok(path)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::transaction(error.to_string()))?;
    atomic_fs::write_replace(path, &bytes).map_err(|error| {
        AppError::transaction(format!(
            "Atomic JSON write failed ({}): {error}",
            path.display()
        ))
    })
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    copy_file_for("file copy", source, destination)
}

fn is_retryable_file_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::PermissionDenied
            | io::ErrorKind::NotFound
    ) || matches!(error.raw_os_error(), Some(2 | 3 | 5 | 32 | 33))
}

fn retry_file_io<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut attempt = 0usize;
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt < FILE_RETRY_DELAYS.len() && is_retryable_file_error(&error) => {
                thread::sleep(FILE_RETRY_DELAYS[attempt]);
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn copy_file_for(stage: &str, source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        retry_file_io(|| fs::create_dir_all(parent)).map_err(|error| {
            AppError::transaction(format!(
                "{stage}: cannot create destination directory ({}): {error}",
                parent.display()
            ))
        })?;
    }
    retry_file_io(|| fs::copy(source, destination)).map_err(|error| {
        AppError::transaction(format!(
            "{stage}: cannot copy {} -> {}: {error}",
            source.display(),
            destination.display()
        ))
    })?;
    Ok(())
}

fn installation_id(target: &Path) -> String {
    let normalized = target
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    format!("{:x}", digest)[..16].to_string()
}

fn installation_dir(state_root: &Path, target: &Path) -> PathBuf {
    state_root
        .join("installations")
        .join(installation_id(target))
}

struct TransactionLock {
    file: File,
}

impl Drop for TransactionLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn transaction_lock_path(target: &Path) -> PathBuf {
    let normalized = target
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    std::env::temp_dir()
        .join("CS2BotImproverPlus")
        .join("transaction-locks")
        .join(format!("{:x}.lock", digest))
}

fn open_transaction_lock(target: &Path) -> Result<File> {
    let path = transaction_lock_path(target);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::transaction(format!(
                "Cannot create the installation lock directory ({}): {error}",
                parent.display()
            ))
        })?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&path)
        .map_err(|error| {
            AppError::transaction(format!(
                "Cannot open the installation lock ({}): {error}",
                path.display()
            ))
        })
}

fn lock_contended(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock || matches!(error.raw_os_error(), Some(32 | 33))
}

fn acquire_transaction_lock(target: &Path) -> Result<TransactionLock> {
    let file = open_transaction_lock(target)?;
    FileExt::try_lock_exclusive(&file).map_err(|error| {
        if lock_contended(&error) {
            AppError::transaction(
                "Another CS2BotImproverPlus panel is installing, repairing, or restoring this CS2 directory"
            )
        } else {
            AppError::transaction(format!("Cannot lock this CS2 installation: {error}"))
        }
    })?;
    Ok(TransactionLock { file })
}

fn try_transaction_lock(target: &Path) -> Result<Option<TransactionLock>> {
    let file = open_transaction_lock(target)?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(TransactionLock { file })),
        Err(error) if lock_contended(&error) => Ok(None),
        Err(error) => Err(AppError::transaction(format!(
            "Cannot inspect the installation lock: {error}"
        ))),
    }
}

fn managed_path(target: &Path, raw: &str) -> Option<PathBuf> {
    let canonical = target.join(raw.replace('/', "\\"));
    mode_layout::active_or_disabled(&canonical)
}

fn detect_source(state_root: &Path, target: &Path) -> SourceDetection {
    let directory = installation_dir(state_root, target);
    if let Some(record) = read_record(&directory).filter(is_suite_record) {
        return SourceDetection {
            source: InstallationSource::ManagedPlus,
            version: Some(record.package_version),
            evidence: vec![".csbip installation record".to_string()],
            migration_kind: MigrationKind::ManagedUpgrade,
            restore_baseline: record.restore_baseline,
            can_install: true,
        };
    }

    let plus = PLUS_MARKERS
        .iter()
        .filter(|path| managed_path(target, path).is_some())
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    if !plus.is_empty() {
        return SourceDetection {
            source: InstallationSource::LegacyPlus,
            version: None,
            evidence: plus,
            migration_kind: MigrationKind::AdoptLegacyPlus,
            restore_baseline: RestoreBaseline::PreMigration,
            can_install: true,
        };
    }

    let upstream = UPSTREAM_MARKERS
        .iter()
        .filter(|path| managed_path(target, path).is_some())
        .map(|path| (*path).to_string())
        .collect::<Vec<_>>();
    if upstream.len() >= 3 {
        return SourceDetection {
            source: InstallationSource::Upstream,
            version: None,
            evidence: upstream,
            migration_kind: MigrationKind::ReplaceUpstream,
            restore_baseline: RestoreBaseline::PreMigration,
            can_install: true,
        };
    }
    if !upstream.is_empty() {
        return SourceDetection {
            source: InstallationSource::MixedUnknown,
            version: None,
            evidence: upstream,
            migration_kind: MigrationKind::Blocked,
            restore_baseline: RestoreBaseline::None,
            can_install: false,
        };
    }

    SourceDetection {
        source: InstallationSource::Clean,
        version: None,
        evidence: Vec::new(),
        migration_kind: MigrationKind::FreshInstall,
        restore_baseline: RestoreBaseline::SteamOriginal,
        can_install: true,
    }
}

fn read_manifest_document(payload_root: &Path) -> Result<PayloadManifest> {
    let path = payload_root.join(MANIFEST_FILE);
    let bytes = fs::read(&path).map_err(|_| {
        if payload_root.join(PANEL_UPDATE_MARKER).is_file() {
            AppError::payload(format!(
                "This is the Panel-only online-update component, not the complete installer. Download and extract LocalArena-v{}-windows.zip for a first installation.",
                crate::app_version::display()
            ))
        } else {
            AppError::payload(format!(
                "The complete plugin payload is missing. Keep LocalArena.exe beside addons, cfg, overrides, and {}. Expected: {}",
                MANIFEST_FILE,
                path.display()
            ))
        }
    })?;
    let manifest: PayloadManifest = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::payload(format!("Invalid payload manifest: {error}")))?;
    if manifest.schema_version != 1 || manifest.entries.is_empty() {
        return Err(AppError::payload("Unsupported or empty payload manifest"));
    }
    for entry in &manifest.entries {
        safe_relative(&entry.path)?;
        if entry.sha256.len() != 64 || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AppError::payload(format!(
                "Invalid payload hash: {}",
                entry.path
            )));
        }
    }
    Ok(manifest)
}

fn read_manifest(payload_root: &Path) -> Result<PayloadManifest> {
    let manifest = read_manifest_document(payload_root)?;
    verify_manifest_files(payload_root, &manifest, false)?;
    Ok(manifest)
}

fn same_canonical_directory(left: &Path, right: &Path) -> bool {
    let (Ok(left), Ok(right)) = (fs::canonicalize(left), fs::canonicalize(right)) else {
        return false;
    };
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

fn verify_manifest_files(
    payload_root: &Path,
    manifest: &PayloadManifest,
    allow_modified_preserve_config: bool,
) -> Result<()> {
    for entry in &manifest.entries {
        let relative = safe_relative(&entry.path)?;
        let source = payload_root.join(&relative);
        if !source.is_file() {
            return Err(AppError::payload(format!(
                "Payload verification failed: {}",
                entry.path
            )));
        }
        if allow_modified_preserve_config && entry.restore_policy == "preserve-config" {
            continue;
        }
        if fs::metadata(&source)
            .map_err(AppError::transaction_io)?
            .len()
            != entry.size
            || !sha256(&source)?.eq_ignore_ascii_case(&entry.sha256)
        {
            return Err(AppError::payload(format!(
                "Payload verification failed: {}",
                entry.path
            )));
        }
    }
    Ok(())
}

fn read_manifest_for_target(payload_root: &Path, target: &Path) -> Result<PayloadManifest> {
    let manifest = read_manifest_document(payload_root)?;
    verify_manifest_files(
        payload_root,
        &manifest,
        same_canonical_directory(payload_root, target),
    )?;
    Ok(manifest)
}

pub fn verify_payload(payload_root: &Path) -> Result<PayloadManifest> {
    read_manifest(payload_root)
}

pub fn verify_payload_for_target(
    payload_root: &Path,
    target: &Path,
) -> Result<PayloadManifest> {
    read_manifest_for_target(payload_root, target)
}

fn read_record(directory: &Path) -> Option<InstallRecord> {
    migrate_record_v1_to_v2(directory).ok().flatten()
}

/// Version recorded for one component, or `None` when that component was never installed.
///
/// The install record is the only authority: no payload is inspected, and no other component's
/// version (nor the suite `package_version`) can stand in for a missing row. The migration
/// sentinel [`UNKNOWN_VERSION`] reads as "not installed" rather than as a version string.
pub fn component_version(
    state_root: &Path,
    target: &Path,
    component: ComponentId,
) -> Option<String> {
    let record = read_record(&installation_dir(state_root, target))?;
    let row = record.components.get(&component)?;
    (row.version != UNKNOWN_VERSION).then(|| row.version.clone())
}

/// Payload directory the record points a component's install at, if any.
///
/// This replaces the pointer file the earlier active-payload design wrote into the update cache:
/// the record already describes what is installed, so the directory an update was extracted to
/// belongs in it.
pub fn component_path(
    state_root: &Path,
    target: &Path,
    component: ComponentId,
) -> Option<String> {
    read_record(&installation_dir(state_root, target))?
        .components
        .get(&component)?
        .path
        .clone()
}

/// Points a component's row at the payload directory its download was extracted to, keeping
/// whatever version and install time the row already recorded.
///
/// Only an existing record can be pointed: a machine with nothing installed gets the row from the
/// install that follows, which preserves this pointer (see `install_manifest`). Writing a version
/// here would claim an install that has not happened yet, so the row keeps `UNKNOWN_VERSION` until
/// the transaction commits.
pub fn set_component_path(
    state_root: &Path,
    target: &Path,
    component: ComponentId,
    path: &Path,
) -> Result<bool> {
    let directory = installation_dir(state_root, target);
    let Some(mut record) = read_record(&directory) else {
        return Ok(false);
    };
    let mut row = record
        .components
        .get(&component)
        .cloned()
        .unwrap_or_else(ComponentRecord::unknown);
    row.ownership = component_ownership(component).to_string();
    row.path = Some(
        fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned(),
    );
    record.components.insert(component, row);
    write_json_atomic(&directory.join("record.json"), &record)?;
    Ok(true)
}

/// Reads `record.json` and, on the first read of a legacy v1 record (a single `package_version`
/// and no `components` key), builds the per-component schema-2 view and atomically writes it
/// back. The one known suite version maps to `la_plugins`; `upstream` and `inventory_simulator`
/// get `"unknown"` placeholders rather than fabricated release tags. Every write goes through
/// [`write_json_atomic`], the shared temp + rename helper.
///
/// Presence of the `components` key is the migration trigger: an already-v2 record (complete or
/// partial) is returned untouched with no write, so re-migration leaves the bytes unchanged and
/// never clobbers versions written by component installs. A missing `record.json` yields
/// `Ok(None)` and is never fabricated. Truncated JSON fails with E1401 before any write, so no
/// partial record file can appear. `package_version` is never altered here, which keeps the
/// existing plus-install downgrade guard intact.
fn migrate_record_v1_to_v2(directory: &Path) -> Result<Option<InstallRecord>> {
    let record_path = directory.join("record.json");
    let bytes = match fs::read(&record_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(AppError::transaction_io(error)),
    };
    let document: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        AppError::transaction(format!(
            "Corrupt install record ({}): {error}",
            record_path.display()
        ))
    })?;
    let already_v2 = document.get("components").is_some();
    let mut record: InstallRecord = serde_json::from_value(document).map_err(|error| {
        AppError::transaction(format!(
            "Corrupt install record ({}): {error}",
            record_path.display()
        ))
    })?;
    if already_v2 {
        return Ok(Some(record));
    }
    record.components.insert(
        ComponentId::LaPlugins,
        ComponentRecord {
            version: record.package_version.clone(),
            source: "v1_record".to_string(),
            installed_at: record.installed_at,
            ownership: "plus".to_string(),
            path: None,
        },
    );
    record
        .components
        .insert(ComponentId::Upstream, ComponentRecord::unknown());
    record
        .components
        .insert(ComponentId::InventorySimulator, ComponentRecord::unknown());
    record.schema_version = RECORD_SCHEMA_VERSION;
    write_json_atomic(&record_path, &record)?;
    Ok(Some(record))
}

/// Whether an install record speaks for the Local Arena suite (`plus`/`shared` entries) or was
/// written only by a third-party component install (InventorySimulator on a clean machine).
///
/// A third-party-only record's `package_version` is the component's own release tag
/// ("3.7.0"), so treating it as the suite record would report that tag as the installed suite
/// version: `installed_plugin_version` would hand it to update detection, whose four-part
/// parse fails, and `detect_source` would claim `ManagedPlus` for a machine that never had the
/// suite. Such records exist so the component's entries stay in the one record per target; they
/// never describe the suite. Removing third-party files is `restore_pristine`'s job
/// (via `SUITE_OWNED_ROOTS`), not evidence of a suite install.
fn is_suite_record(record: &InstallRecord) -> bool {
    record
        .entries
        .iter()
        .any(|entry| entry.ownership != "third-party")
}

pub fn inspect(
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
) -> Result<InstallationInspection> {
    inspect_impl(payload_root, state_root, target, true)
}

pub fn inspect_quick(
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
) -> Result<InstallationInspection> {
    inspect_impl(payload_root, state_root, target, false)
}

fn inspect_impl(
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
    verify_hashes: bool,
) -> Result<InstallationInspection> {
    let directory = installation_dir(state_root, target);
    // A third-party-only record is not a suite install: filtering it keeps `installed`,
    // `package_version`, and `restore_available` describing the suite (see `is_suite_record`).
    let record = read_record(&directory).filter(is_suite_record);
    let detection = detect_source(state_root, target);
    let manifest = if verify_hashes {
        read_manifest_for_target(payload_root, target).ok()
    } else {
        read_manifest_document(payload_root).ok()
    };
    let entries: Vec<(String, String, Option<u64>, String)> = if let Some(manifest) = &manifest {
        manifest
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.path.clone(),
                    entry.sha256.clone(),
                    Some(entry.size),
                    entry.restore_policy.clone(),
                )
            })
            .collect()
    } else if let Some(record) = &record {
        record
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.path.clone(),
                    entry.installed_sha256.clone(),
                    None,
                    entry.restore_policy.clone(),
                )
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut healthy = 0;
    let mut missing = Vec::new();
    let mut corrupt = Vec::new();
    for (raw, expected, expected_size, restore_policy) in &entries {
        let canonical = target.join(safe_relative(raw)?);
        let disabled = mode_layout::disabled_path(&canonical);
        let path = if canonical.is_file() {
            canonical
        } else {
            disabled
        };
        if !path.is_file() {
            missing.push(raw.clone());
        } else if restore_policy == "preserve-config" {
            healthy += 1;
        } else if verify_hashes {
            if sha256(&path)?.eq_ignore_ascii_case(expected) {
                healthy += 1;
            } else {
                corrupt.push(raw.clone());
            }
        } else if expected_size.is_none_or(|size| {
            fs::metadata(&path)
                .map(|metadata| metadata.len() == size)
                .unwrap_or(false)
        }) {
            healthy += 1;
        } else {
            corrupt.push(raw.clone());
        }
    }
    Ok(InstallationInspection {
        installed: record.is_some(),
        package_version: record.as_ref().map(|record| record.package_version.clone()),
        manifest_available: manifest.is_some(),
        total: entries.len(),
        healthy,
        missing,
        corrupt,
        restore_available: record.is_some(),
        backup_path: record
            .as_ref()
            .map(|_| directory.to_string_lossy().into_owned()),
        backup_display: record.as_ref().map(|_| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|base| base.to_path_buf()))
                .and_then(|base| {
                    directory
                        .strip_prefix(&base)
                        .ok()
                        .map(|relative| relative.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| directory.to_string_lossy().into_owned())
        }),
        interrupted_transaction: directory.join("journal.json").is_file(),
        source: detection.source,
        source_version: detection.version,
        source_evidence: detection.evidence,
        migration_kind: detection.migration_kind,
        restore_baseline: detection.restore_baseline,
        can_install: detection.can_install,
    })
}

struct Preflight {
    required_target_bytes: u64,
    available_target_bytes: u64,
    required_backup_bytes: u64,
    available_backup_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct InstallSpaceRequirements {
    pub required_target_bytes: u64,
    pub available_target_bytes: u64,
    pub required_backup_bytes: u64,
    pub available_backup_bytes: u64,
}

fn inspect_space(manifest: &PayloadManifest, state_root: &Path, target: &Path) -> Result<Preflight> {
    if !target.is_dir() {
        return Err(AppError::transaction(format!(
            "Installation target is not a directory: {}",
            target.display()
        )));
    }
    fs::create_dir_all(state_root).map_err(AppError::transaction_io)?;
    let probe = target.join(format!(".cs2bi-write-test-{}", unix_time()));
    fs::write(&probe, b"write-test").map_err(|error| {
        AppError::transaction(format!(
            "The CS2 directory is not writable ({}): {error}",
            target.display()
        ))
    })?;
    fs::remove_file(&probe).map_err(|error| {
        AppError::transaction(format!(
            "The CS2 write test could not be cleaned up ({}): {error}",
            probe.display()
        ))
    })?;

    let mut largest_payload = 0;
    let mut backup_bytes: u64 = 0;
    for entry in &manifest.entries {
        largest_payload = largest_payload.max(entry.size);
        let relative = safe_relative(&entry.path)?;
        let mut cursor = target.to_path_buf();
        let components = relative.components().collect::<Vec<_>>();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            cursor.push(component.as_os_str());
            if cursor.exists() && !cursor.is_dir() {
                return Err(AppError::transaction(format!(
                    "A payload parent path is not a directory: {}",
                    cursor.display()
                )));
            }
        }
        let destination = target.join(&relative);
        let existing = if destination.exists() {
            destination.clone()
        } else {
            mode_layout::disabled_path(&destination)
        };
        if existing.exists() && !existing.is_file() {
            return Err(AppError::transaction(format!(
                "A payload file target is occupied by a directory: {}",
                existing.display()
            )));
        }
        if let Ok(metadata) = fs::metadata(existing) {
            backup_bytes = backup_bytes.saturating_add(metadata.len());
        }
    }

    const MARGIN: u64 = 64 * 1024 * 1024;
    let required_target_bytes = largest_payload.saturating_add(MARGIN);
    let required_backup_bytes = backup_bytes.saturating_mul(2).saturating_add(MARGIN);
    let available_target_bytes = fs2::available_space(target).map_err(AppError::transaction_io)?;
    let available_backup_bytes =
        fs2::available_space(state_root).map_err(AppError::transaction_io)?;
    Ok(Preflight {
        required_target_bytes,
        available_target_bytes,
        required_backup_bytes,
        available_backup_bytes,
    })
}

pub fn inspect_space_requirements(
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
) -> Result<InstallSpaceRequirements> {
    let manifest = read_manifest_for_target(payload_root, target)?;
    let preflight = inspect_space(&manifest, state_root, target)?;
    Ok(InstallSpaceRequirements {
        required_target_bytes: preflight.required_target_bytes,
        available_target_bytes: preflight.available_target_bytes,
        required_backup_bytes: preflight.required_backup_bytes,
        available_backup_bytes: preflight.available_backup_bytes,
    })
}

fn preflight(manifest: &PayloadManifest, state_root: &Path, target: &Path) -> Result<Preflight> {
    let preflight = inspect_space(manifest, state_root, target)?;
    if preflight.available_target_bytes < preflight.required_target_bytes {
        return Err(AppError::transaction(format!(
            "Insufficient space in CS2 directory: need {} bytes, available {}",
            preflight.required_target_bytes, preflight.available_target_bytes
        )));
    }
    if preflight.available_backup_bytes < preflight.required_backup_bytes {
        return Err(AppError::transaction(format!(
            "Insufficient space for backups: need {} bytes, available {}",
            preflight.required_backup_bytes, preflight.available_backup_bytes
        )));
    }
    Ok(preflight)
}

pub fn plan(payload_root: &Path, state_root: &Path, target: &Path) -> Result<InstallPlan> {
    let manifest = read_manifest_for_target(payload_root, target)?;
    let detection = detect_source(state_root, target);
    let preflight = preflight(&manifest, state_root, target)?;
    let mut new_files = 0;
    let mut overwritten_files = 0;
    for entry in &manifest.entries {
        let destination = target.join(safe_relative(&entry.path)?);
        if destination.exists() || mode_layout::disabled_path(&destination).exists() {
            overwritten_files += 1;
        } else {
            new_files += 1;
        }
    }
    Ok(InstallPlan {
        package_version: manifest.package_version,
        target: target.to_string_lossy().into_owned(),
        total_files: manifest.entries.len(),
        new_files,
        overwritten_files,
        backup_path: installation_dir(state_root, target)
            .to_string_lossy()
            .into_owned(),
        required_target_bytes: preflight.required_target_bytes,
        available_target_bytes: preflight.available_target_bytes,
        required_backup_bytes: preflight.required_backup_bytes,
        available_backup_bytes: preflight.available_backup_bytes,
        writable: true,
        source: detection.source,
        source_version: detection.version,
        source_evidence: detection.evidence,
        migration_kind: detection.migration_kind,
        restore_baseline: detection.restore_baseline,
        can_install: detection.can_install,
    })
}

fn rollback(directory: &Path, journal: &TransactionJournal) -> Result<()> {
    for change in journal.changes.iter().rev() {
        let target = PathBuf::from(&journal.target).join(safe_relative(&change.path)?);
        if change.existed {
            let backup = change
                .backup
                .as_ref()
                .ok_or_else(|| AppError::transaction("Rollback backup is missing"))?;
            copy_file_for("transaction rollback", &directory.join(backup), &target)?;
        } else if target.is_file() {
            fs::remove_file(&target).map_err(AppError::transaction_io)?;
        }
    }
    Ok(())
}

fn restore_previous_record(directory: &Path, previous: &Option<InstallRecord>) -> Result<()> {
    let record_path = directory.join("record.json");
    if let Some(record) = previous {
        write_json_atomic(&record_path, record)
    } else if record_path.is_file() {
        fs::remove_file(record_path).map_err(AppError::transaction_io)
    } else {
        Ok(())
    }
}

fn recover_incomplete_locked(state_root: &Path, target: &Path) -> Result<bool> {
    let directory = installation_dir(state_root, target);
    let journal_path = directory.join("journal.json");
    if !journal_path.is_file() {
        return Ok(false);
    }
    let journal: TransactionJournal =
        serde_json::from_slice(&fs::read(&journal_path).map_err(AppError::transaction_io)?)
            .map_err(|error| AppError::transaction(error.to_string()))?;
    if !journal.committed {
        rollback(&directory, &journal)?;
        restore_previous_record(&directory, &journal.previous_record)?;
    }
    let _ = fs::remove_dir_all(directory.join(&journal.transaction_dir));
    fs::remove_file(journal_path).map_err(AppError::transaction_io)?;
    Ok(true)
}

pub fn recover_incomplete(state_root: &Path, target: &Path) -> Result<bool> {
    let Some(_transaction_lock) = try_transaction_lock(target)? else {
        // A live writer owns the journal. It is not an interrupted transaction.
        return Ok(false);
    };
    recover_incomplete_locked(state_root, target)
}

/// Installs every entry of one payload manifest — the full-payload path.
///
/// Production installs select a component first ([`install_component`]), so this entry point now
/// exists for the archive/upgrade/migration tests, which need a manifest-complete transaction to
/// exercise `install_manifest`'s `component: None` branch.
#[cfg(test)]
pub fn install(
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
    repaired: bool,
) -> Result<InstallTransactionResult> {
    let _transaction_lock = acquire_transaction_lock(target)?;
    recover_incomplete_locked(state_root, target)?;
    let manifest = read_manifest_for_target(payload_root, target)?;
    install_manifest(&manifest, payload_root, state_root, target, repaired, None)
}

/// The ownership class a component's payload entries carry: the upstream payload is `shared`,
/// the Local Arena bundle is `plus`, and InventorySimulator is the third-party class.
fn component_ownership(component: ComponentId) -> &'static str {
    match component {
        ComponentId::Upstream => "shared",
        ComponentId::LaPlugins => "plus",
        ComponentId::InventorySimulator => "third-party",
    }
}

/// The managed component an install-record entry's `ownership` class belongs to.
fn component_of_ownership(ownership: &str) -> Option<ComponentId> {
    match ownership {
        "shared" => Some(ComponentId::Upstream),
        "plus" => Some(ComponentId::LaPlugins),
        "third-party" => Some(ComponentId::InventorySimulator),
        _ => None,
    }
}

/// Installs one component: the manifest entries whose `ownership` class belongs to
/// `component_id` go through the same locked, journalled, record-merging transaction as
/// [`install`], and the commit records `record.components[component_id]`.
///
/// Before any write the transaction runs [`preflight_component_conflicts`], which fails with
/// E1403 when a payload path would overwrite a file that belongs to another component (or to no
/// managed component) with different bytes, and with E1404 when the Local Arena bundle is
/// installed without the upstream `BotHiderApi.dll` it links against. Both failures leave the
/// target untouched.
///
/// The install record stays one file per target: entries already recorded for other owners are
/// carried over untouched and the component's entries are merged into them. Identical files are
/// skipped, so re-running the same component payload is a no-op. This transaction never writes a
/// payload pointer: the component's own `record.components` row is the pointer.
pub fn install_component(
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
    component_id: ComponentId,
) -> Result<InstallTransactionResult> {
    let _transaction_lock = acquire_transaction_lock(target)?;
    recover_incomplete_locked(state_root, target)?;
    let filtered = preflight_component_conflicts(payload_root, state_root, target, component_id)?;
    // `repaired` selects the skip-identical branch, which is what an incremental component
    // install needs: only missing or differing files are replaced.
    let mut result = install_manifest(
        &filtered,
        payload_root,
        state_root,
        target,
        true,
        Some(component_id),
    )?;
    // A single component is never the full-package first install the welcome story narrates.
    result.welcome_story_eligible = false;
    Ok(result)
}

/// Pre-write gate for one component install: resolves the manifest subset this component owns,
/// checks its dependency on the upstream bundle, and refuses any path that would silently
/// overwrite a file another component already owns.
///
/// A path already registered under the CURRENT component is a normal component update and is
/// excluded from the target-file comparison. Conflict paths are listed one per line as
/// `path: <relative path>` so the error modal can render them as a list. The filtered manifest
/// is returned so the transaction does not read and verify the same payload twice.
pub fn preflight_component_conflicts(
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
    component_id: ComponentId,
) -> Result<PayloadManifest> {
    let manifest = read_manifest_for_target(payload_root, target)?;
    let ownership = component_ownership(component_id);
    let filtered = PayloadManifest {
        schema_version: manifest.schema_version,
        package_version: manifest.package_version,
        entries: manifest
            .entries
            .into_iter()
            .filter(|entry| entry.ownership == ownership)
            .collect(),
    };
    if filtered.entries.is_empty() {
        return Err(AppError::payload(
            "Component has no owned files in this payload",
        ));
    }
    // The LA bundle links against the upstream-managed BotHiderApi; the requirement list comes
    // from the compiled-in descriptor, so this gate cannot drift from it (E1404, not the generic
    // transaction code `<check_requirements>` reports on its own).
    if component_id == ComponentId::LaPlugins {
        crate::component_registry::check_requirements(
            target,
            crate::component_registry::LA_PLUGIN_BUNDLE.requires,
        )
        .map_err(|error| AppError::new("E1404", "installation", error.detail))?;
    }

    let directory = installation_dir(state_root, target);
    let registered_current: BTreeSet<String> = read_record(&directory)
        .map(|record| {
            record
                .entries
                .into_iter()
                .filter(|entry| component_of_ownership(&entry.ownership) == Some(component_id))
                .map(|entry| entry.path)
                .collect()
        })
        .unwrap_or_default();
    let mut conflicts = Vec::new();
    for entry in &filtered.entries {
        if registered_current.contains(&entry.path) {
            continue;
        }
        let destination = target.join(safe_relative(&entry.path)?);
        if !destination.is_file() {
            continue;
        }
        if !sha256(&destination)?.eq_ignore_ascii_case(&entry.sha256) {
            conflicts.push(entry.path.clone());
        }
    }
    if !conflicts.is_empty() {
        let detail = conflicts
            .iter()
            .map(|path| format!("path: {path}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(AppError::new(
            "E1403",
            "installation",
            format!(
                "These files are already installed by another component with different content; repair or remove that component first:\n{detail}"
            ),
        ));
    }
    Ok(filtered)
}

/// Installs the Local Arena plugin bundle. Its dependency on the upstream-managed
/// `BotHiderApi.dll` is enforced by [`preflight_component_conflicts`] from
/// [`crate::component_registry::LA_PLUGIN_BUNDLE`] itself, so this wrapper only names the
/// component and cannot drift from the registry the bundle enforces.
pub fn install_la_plugins_component(
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
) -> Result<InstallTransactionResult> {
    install_component(payload_root, state_root, target, ComponentId::LaPlugins)
}

/// The components an "install everything" run installs, in dependency order: the LA bundle
/// links against the upstream-managed `BotHiderApi.dll` (E1404 otherwise), and
/// InventorySimulator depends on neither.
pub const INSTALL_ORDER: [ComponentId; 3] = [
    ComponentId::Upstream,
    ComponentId::LaPlugins,
    ComponentId::InventorySimulator,
];

/// Runs one independent transaction per component, in order, and stops at the first failure.
///
/// Nothing is caught and retried: a failing component's error is the caller's error, so the
/// components after it are never touched and can never half-apply.
pub fn run_component_steps<T>(
    components: &[ComponentId],
    mut step: impl FnMut(ComponentId) -> Result<T>,
) -> Result<Vec<T>> {
    let mut completed = Vec::with_capacity(components.len());
    for &component in components {
        completed.push(step(component)?);
    }
    Ok(completed)
}

/// `package_version` a `plus` component install records.
///
/// The LA bundle card installs through this path, so the bundle's own version is the one update
/// detection must see; only a downgrade against the previous suite record is refused (see
/// [`install_manifest`]). A version that fails to parse on either side must not block recording,
/// so the manifest version wins there.
fn plus_component_package_version(
    previous: Option<&InstallRecord>,
    manifest_version: &str,
) -> String {
    let Some(previous) = previous.filter(|record| is_suite_record(record)) else {
        return manifest_version.to_string();
    };
    match (
        crate::update_core::DisplayVersion::parse(manifest_version),
        crate::update_core::DisplayVersion::parse(&previous.package_version),
    ) {
        (Ok(installed), Ok(recorded)) if installed < recorded => previous.package_version.clone(),
        _ => manifest_version.to_string(),
    }
}

/// Transaction body shared by [`install`] and [`install_component`]. Callers hold the target
/// transaction lock and have already recovered any interrupted transaction.
///
/// `component` marks a subset install: its entries belong to that component's ownership class
/// (`plus` for the LA bundle card, `third-party` for InventorySimulator, `shared` for upstream)
/// and merge into the existing record, which gains `record.components[component]`. `None` is a
/// full-payload install. The recorded `package_version` follows that ownership class — see the
/// record write at the end of the transaction.
fn install_manifest(
    manifest: &PayloadManifest,
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
    repaired: bool,
    component: Option<ComponentId>,
) -> Result<InstallTransactionResult> {
    let detection = detect_source(state_root, target);
    if !detection.can_install {
        return Err(AppError::transaction(
            "A partial or mixed enhanced-plugin installation was detected. Export diagnostics or restore pristine CS2 before installing.",
        ));
    }
    preflight(manifest, state_root, target)?;
    let directory = installation_dir(state_root, target);
    fs::create_dir_all(&directory).map_err(AppError::transaction_io)?;
    let old_record = read_record(&directory);
    let old_entries: BTreeMap<String, InstallRecordEntry> = old_record
        .as_ref()
        .map(|record| {
            record
                .entries
                .iter()
                .cloned()
                .map(|entry| (entry.path.clone(), entry))
                .collect()
        })
        .unwrap_or_default();
    let transaction_name = format!("transaction-{}", unix_time());
    let mut journal = TransactionJournal {
        schema_version: 1,
        operation: if repaired { "repair" } else { "install" }.to_string(),
        target: target.to_string_lossy().into_owned(),
        transaction_dir: transaction_name.clone(),
        changes: Vec::new(),
        committed: false,
        previous_record: old_record.clone(),
    };
    let journal_path = directory.join("journal.json");
    write_json_atomic(&journal_path, &journal)?;
    let transaction_root = directory.join(&transaction_name);
    let original_root = directory.join("original");
    let mut record_entries = old_entries.clone();
    let mut installed_files = 0;

    let outcome = (|| -> Result<()> {
        for entry in &manifest.entries {
            let relative = safe_relative(&entry.path)?;
            let destination = target.join(&relative);
            let preserve_config = entry.restore_policy == "preserve-config";
            let existed = destination.is_file();

            if preserve_config && existed {
                let baseline = if let Some(previous) = old_entries.get(&entry.path) {
                    previous.clone()
                } else {
                    let original = original_root.join(&relative);
                    copy_file_for("preserve-config baseline backup", &destination, &original)?;
                    InstallRecordEntry {
                        path: entry.path.clone(),
                        original_existed: true,
                        original_sha256: Some(sha256(&destination)?),
                        original_backup: Some(format!(
                            "original/{}",
                            entry.path.replace('\\', "/")
                        )),
                        installed_sha256: sha256(&destination)?,
                        ownership: entry.ownership.clone(),
                        restore_policy: entry.restore_policy.clone(),
                    }
                };
                record_entries.insert(
                    entry.path.clone(),
                    InstallRecordEntry {
                        ownership: entry.ownership.clone(),
                        restore_policy: entry.restore_policy.clone(),
                        ..baseline
                    },
                );
                continue;
            }

            if repaired
                && old_entries.contains_key(&entry.path)
                && destination.is_file()
                && sha256(&destination)?.eq_ignore_ascii_case(&entry.sha256)
            {
                continue;
            }
            let packaged_source = payload_root.join(&relative);
            let mirror_source = relative
                .file_name()
                .map(|name| state_root.join("presets").join("current").join(name))
                .filter(|path| preserve_config && path.is_file());
            let source = mirror_source.as_deref().unwrap_or(&packaged_source);
            let source_sha256 = sha256(source)?;
            let transaction_backup = transaction_root.join(&relative);
            if existed {
                copy_file_for(
                    "installation rollback backup",
                    &destination,
                    &transaction_backup,
                )?;
            }
            journal.changes.push(TransactionChange {
                path: entry.path.clone(),
                existed,
                backup: existed
                    .then(|| format!("{transaction_name}/{}", entry.path.replace('\\', "/"))),
            });
            write_json_atomic(&journal_path, &journal)?;

            let baseline = if let Some(previous) = old_entries.get(&entry.path) {
                previous.clone()
            } else if existed {
                let original = original_root.join(&relative);
                copy_file_for("pre-install original backup", &destination, &original)?;
                InstallRecordEntry {
                    path: entry.path.clone(),
                    original_existed: true,
                    original_sha256: Some(sha256(&destination)?),
                    original_backup: Some(format!("original/{}", entry.path.replace('\\', "/"))),
                    installed_sha256: entry.sha256.clone(),
                    ownership: entry.ownership.clone(),
                    restore_policy: entry.restore_policy.clone(),
                }
            } else {
                InstallRecordEntry {
                    path: entry.path.clone(),
                    original_existed: false,
                    original_sha256: None,
                    original_backup: None,
                    installed_sha256: entry.sha256.clone(),
                    ownership: entry.ownership.clone(),
                    restore_policy: entry.restore_policy.clone(),
                }
            };

            let temporary =
                atomic_fs::temporary_path(&destination).map_err(AppError::transaction_io)?;
            let replacement = (|| -> Result<()> {
                copy_file_for("payload staging", source, &temporary)?;
                if !sha256(&temporary)?.eq_ignore_ascii_case(&source_sha256) {
                    return Err(AppError::payload(format!(
                        "Copied payload hash mismatch: {}",
                        entry.path
                    )));
                }
                retry_file_io(|| atomic_fs::sync(&temporary)).map_err(|error| {
                    AppError::transaction(format!(
                        "Payload sync failed ({}): {error}",
                        temporary.display()
                    ))
                })?;
                retry_file_io(|| atomic_fs::replace(&temporary, &destination)).map_err(|error| {
                    AppError::transaction(format!(
                        "Atomic payload replace failed ({} -> {}): {error}",
                        temporary.display(),
                        destination.display()
                    ))
                })
            })();
            if replacement.is_err() {
                let _ = fs::remove_file(&temporary);
            }
            replacement?;

            record_entries.insert(
                entry.path.clone(),
                InstallRecordEntry {
                    installed_sha256: source_sha256,
                    ownership: entry.ownership.clone(),
                    restore_policy: entry.restore_policy.clone(),
                    ..baseline
                },
            );
            installed_files += 1;
        }

        for (superseded_path, replacement_path) in SUPERSEDED_PAYLOAD_PATHS {
            if !manifest.entries.iter().any(|entry| entry.path == *replacement_path) {
                continue;
            }
            let Some(previous) = old_entries.get(*superseded_path) else {
                continue;
            };
            let relative = safe_relative(superseded_path)?;
            let destination = target.join(&relative);
            if !destination.is_file() {
                record_entries.remove(*superseded_path);
                continue;
            }
            if !sha256(&destination)?.eq_ignore_ascii_case(&previous.installed_sha256) {
                continue;
            }

            let transaction_backup = transaction_root.join(&relative);
            copy_file_for(
                "superseded payload rollback backup",
                &destination,
                &transaction_backup,
            )?;
            journal.changes.push(TransactionChange {
                path: (*superseded_path).to_string(),
                existed: true,
                backup: Some(format!(
                    "{transaction_name}/{}",
                    superseded_path.replace('\\', "/")
                )),
            });
            write_json_atomic(&journal_path, &journal)?;

            if previous.original_existed {
                let original = previous.original_backup.as_ref().ok_or_else(|| {
                    AppError::transaction(format!(
                        "Original backup is missing for superseded payload: {superseded_path}"
                    ))
                })?;
                copy_file_for(
                    "superseded payload original restore",
                    &directory.join(original),
                    &destination,
                )?;
            } else {
                fs::remove_file(&destination).map_err(AppError::transaction_io)?;
            }
            record_entries.remove(*superseded_path);
        }
        Ok(())
    })();

    if let Err(error) = outcome {
        let rollback_error = rollback(&directory, &journal).err();
        let record_error = restore_previous_record(&directory, &journal.previous_record).err();
        let _ = fs::remove_dir_all(&transaction_root);
        let _ = fs::remove_file(&journal_path);
        return Err(rollback_error.or(record_error).unwrap_or(error));
    }

    // `package_version` is the suite version `installed_plugin_version` reports and update
    // detection parses. A full-payload install sets it from its own manifest. A component
    // install depends on which ownership it carries:
    //
    // * `plus` is the LA bundle card's path, so the freshly installed bundle's version must be
    //   recorded — preserving the previous one would leave detection at the old version and the
    //   card offering the same update forever. Only a downgrade is refused: the bundle can
    //   install from a local payload that predates the last online update, and recording that
    //   older version would roll update detection backwards.
    // * a third-party component (InventorySimulator) never speaks for the suite. Its tag
    //   ("3.7.0") would break the four-part version parse, so an existing suite record keeps
    //   its version. With no suite record the component's manifest version is the only version
    //   there is; merging `plus` entries into such a record later replaces it (see
    //   `is_suite_record`).
    let package_version = match component {
        None => manifest.package_version.clone(),
        Some(ComponentId::LaPlugins) => {
            plus_component_package_version(old_record.as_ref(), &manifest.package_version)
        }
        Some(_) => old_record
            .as_ref()
            .filter(|previous| is_suite_record(previous))
            .map_or_else(
                || manifest.package_version.clone(),
                |previous| previous.package_version.clone(),
            ),
    };
    // Per-component view: carry forward what earlier installs recorded — `old_record` has already
    // been migrated by `read_record`, so a legacy v1 record's freshly mapped components survive
    // this write — and record this component's own version. The LA row keeps the downgrade
    // guard's answer so the component view can never claim a version the suite record refused;
    // every other component records its own payload version (a third-party tag such as "3.7.0"
    // is correct for its own row, never for the suite).
    let mut components = old_record
        .as_ref()
        .map(|record| record.components.clone())
        .unwrap_or_default();
    if let Some(component_id) = component {
        let version = match component_id {
            ComponentId::LaPlugins => package_version.clone(),
            _ => manifest.package_version.clone(),
        };
        components.insert(
            component_id,
            ComponentRecord {
                version,
                source: "component_install".to_string(),
                installed_at: unix_time(),
                ownership: component_ownership(component_id).to_string(),
                // A download pointer written before this transaction (see
                // `set_component_path`) must survive the row it rewrites, or the local source
                // the next repair resolves would be lost.
                path: old_record
                    .as_ref()
                    .and_then(|record| record.components.get(&component_id))
                    .and_then(|row| row.path.clone()),
            },
        );
    }
    let record = InstallRecord {
        schema_version: 2,
        installation_id: installation_id(target),
        target: target.to_string_lossy().into_owned(),
        package_version,
        installed_at: unix_time(),
        source: old_record
            .as_ref()
            .map_or(detection.source, |record| record.source),
        restore_baseline: old_record
            .as_ref()
            .map_or(detection.restore_baseline, |record| record.restore_baseline),
        migrated_from_version: old_record
            .as_ref()
            .and_then(|record| record.migrated_from_version.clone())
            .or(detection.version),
        components,
        entries: record_entries.into_values().collect(),
    };
    write_json_atomic(&directory.join("record.json"), &record)?;
    journal.committed = true;
    write_json_atomic(&journal_path, &journal)?;
    let _ = fs::remove_dir_all(transaction_root);
    fs::remove_file(journal_path).map_err(AppError::transaction_io)?;
    Ok(InstallTransactionResult {
        package_version: manifest.package_version.clone(),
        installed_files,
        backup_path: directory.to_string_lossy().into_owned(),
        repaired,
        welcome_story_eligible: welcome_story_prompt_eligible(
            cfg!(not(debug_assertions)),
            payload_root.join(MANIFEST_FILE).is_file(),
            detection.migration_kind,
            repaired,
        ),
    })
}

fn cosmetics_inventory_relative() -> String {
    crate::cosmetics_source::INVENTORY_RELATIVE.to_string()
}

fn preserve_cosmetics(state_root: &Path, target: &Path, timestamp: u64) -> Result<Option<PathBuf>> {
    let source = crate::cosmetics_source::inventory_path(target);
    if !source.is_file() {
        return Ok(None);
    }
    let destination = state_root.join("presets").join(timestamp.to_string());
    copy_file(&source, &destination.join("inventories.json"))?;
    Ok(Some(destination))
}

pub fn restore(payload_root: &Path, state_root: &Path, target: &Path) -> Result<RestoreResult> {
    let _transaction_lock = acquire_transaction_lock(target)?;
    recover_incomplete_locked(state_root, target)?;
    let timestamp = unix_time();
    let presets = preserve_cosmetics(state_root, target, timestamp)?;
    let directory = installation_dir(state_root, target);
    fs::create_dir_all(&directory).map_err(AppError::transaction_io)?;
    let previous_record = read_record(&directory);
    let transaction_name = format!("transaction-restore-{timestamp}");
    let transaction_root = directory.join(&transaction_name);
    let journal_path = directory.join("journal.json");
    let mut journal = TransactionJournal {
        schema_version: 1,
        operation: "restore".to_string(),
        target: target.to_string_lossy().into_owned(),
        transaction_dir: transaction_name.clone(),
        changes: Vec::new(),
        committed: false,
        previous_record: previous_record.clone(),
    };
    write_json_atomic(&journal_path, &journal)?;
    let mut restored_files = 0;
    let mut removed_files = 0;
    let mut preserved_files = 0;

    let mut stage = |relative: &Path, raw: &str, destination: &Path| -> Result<()> {
        let existed = destination.is_file();
        let backup = transaction_root.join(relative);
        if existed {
            copy_file(destination, &backup)?;
        }
        journal.changes.push(TransactionChange {
            path: raw.to_string(),
            existed,
            backup: existed.then(|| format!("{transaction_name}/{}", raw.replace('\\', "/"))),
        });
        write_json_atomic(&journal_path, &journal)
    };

    let outcome = (|| -> Result<()> {
        if let Some(record) = &previous_record {
            for entry in record.entries.iter().rev() {
                // Third-party components are not part of the suite: "restore previous" returns
                // the game to its pre-suite state without deleting a plugin the suite never
                // installed. Removing them is `restore_pristine`'s job (via `SUITE_OWNED_ROOTS`).
                if entry.ownership == "third-party" {
                    continue;
                }
                let relative = safe_relative(&entry.path)?;
                let destination = target.join(&relative);
                if entry.original_existed {
                    stage(&relative, &entry.path, &destination)?;
                    let backup = entry.original_backup.as_ref().ok_or_else(|| {
                        AppError::transaction(format!(
                            "Original backup missing from record: {}",
                            entry.path
                        ))
                    })?;
                    copy_file(&directory.join(backup), &destination)?;
                    restored_files += 1;
                } else if destination.is_file() {
                    stage(&relative, &entry.path, &destination)?;
                    if !sha256(&destination)?.eq_ignore_ascii_case(&entry.installed_sha256) {
                        let preserved = directory
                            .join("preserved-modified")
                            .join(timestamp.to_string())
                            .join(&relative);
                        copy_file(&destination, &preserved)?;
                        preserved_files += 1;
                    }
                    fs::remove_file(destination).map_err(AppError::transaction_io)?;
                    removed_files += 1;
                }
            }
            fs::remove_file(directory.join("record.json")).map_err(AppError::transaction_io)?;
        } else {
            let manifest = read_manifest_for_target(payload_root, target)?;
            for entry in manifest
                .entries
                .iter()
                .filter(|entry| entry.ownership == "plus")
            {
                let destination = target.join(safe_relative(&entry.path)?);
                if destination.is_file()
                    && sha256(&destination)?.eq_ignore_ascii_case(&entry.sha256)
                {
                    let relative = safe_relative(&entry.path)?;
                    stage(&relative, &entry.path, &destination)?;
                    fs::remove_file(destination).map_err(AppError::transaction_io)?;
                    removed_files += 1;
                }
            }
        }
        Ok(())
    })();
    drop(stage);

    if let Err(error) = outcome {
        let rollback_error = rollback(&directory, &journal).err();
        let record_error = restore_previous_record(&directory, &journal.previous_record).err();
        let _ = fs::remove_dir_all(&transaction_root);
        let _ = fs::remove_file(&journal_path);
        return Err(rollback_error.or(record_error).unwrap_or(error));
    }

    journal.committed = true;
    write_json_atomic(&journal_path, &journal)?;
    let _ = fs::remove_dir_all(&transaction_root);
    fs::remove_file(&journal_path).map_err(AppError::transaction_io)?;

    Ok(RestoreResult {
        restored_files,
        removed_files,
        preserved_files,
        presets_backup: presets.map(|path| path.to_string_lossy().into_owned()),
        steam_verify_uri: "steam://validate/730".to_string(),
        result_kind: "restore_previous".to_string(),
    })
}

/// Removes one component's installed files, restoring the pre-install copy wherever the record
/// kept one, and drops that component's rows from `record.json`.
///
/// Only paths the record attributes to `component_id` are touched: files another component owns,
/// the plugin's own user data, and the other components' record rows survive. The removal runs in
/// the same journalled transaction shape [`restore`] uses, so an interrupted uninstall rolls the
/// destination files and the record back to the pre-call state. A component that is not installed
/// is a successful no-op.
pub fn uninstall_component(
    state_root: &Path,
    target: &Path,
    component_id: ComponentId,
) -> Result<RestoreResult> {
    let _transaction_lock = acquire_transaction_lock(target)?;
    recover_incomplete_locked(state_root, target)?;
    let directory = installation_dir(state_root, target);
    let empty = || RestoreResult {
        restored_files: 0,
        removed_files: 0,
        preserved_files: 0,
        presets_backup: None,
        steam_verify_uri: "steam://validate/730".to_string(),
        result_kind: "component_uninstall".to_string(),
    };
    let Some(previous_record) = read_record(&directory) else {
        return Ok(empty());
    };
    let owning: Vec<InstallRecordEntry> = previous_record
        .entries
        .iter()
        .filter(|entry| component_of_ownership(&entry.ownership) == Some(component_id))
        .cloned()
        .collect();
    if owning.is_empty() {
        return Ok(empty());
    }

    let timestamp = unix_time();
    let transaction_name = format!("transaction-uninstall-{timestamp}");
    let transaction_root = directory.join(&transaction_name);
    let journal_path = directory.join("journal.json");
    let mut journal = TransactionJournal {
        schema_version: 1,
        operation: "uninstall".to_string(),
        target: target.to_string_lossy().into_owned(),
        transaction_dir: transaction_name.clone(),
        changes: Vec::new(),
        committed: false,
        previous_record: Some(previous_record.clone()),
    };
    write_json_atomic(&journal_path, &journal)?;
    let mut result = empty();

    let outcome = (|| -> Result<()> {
        for entry in owning.iter().rev() {
            let relative = safe_relative(&entry.path)?;
            let destination = target.join(&relative);
            let existed = destination.is_file();
            if existed {
                copy_file(&destination, &transaction_root.join(&relative))?;
            }
            journal.changes.push(TransactionChange {
                path: entry.path.clone(),
                existed,
                backup: existed
                    .then(|| format!("{transaction_name}/{}", entry.path.replace('\\', "/"))),
            });
            write_json_atomic(&journal_path, &journal)?;

            if entry.original_existed {
                let backup = entry.original_backup.as_ref().ok_or_else(|| {
                    AppError::transaction(format!(
                        "Original backup missing from record: {}",
                        entry.path
                    ))
                })?;
                copy_file(&directory.join(backup), &destination)?;
                result.restored_files += 1;
            } else if existed {
                if !sha256(&destination)?.eq_ignore_ascii_case(&entry.installed_sha256) {
                    let preserved = directory
                        .join("preserved-modified")
                        .join(timestamp.to_string())
                        .join(&relative);
                    copy_file(&destination, &preserved)?;
                    result.preserved_files += 1;
                }
                fs::remove_file(&destination).map_err(AppError::transaction_io)?;
                result.removed_files += 1;
            }
        }
        Ok(())
    })();

    if let Err(error) = outcome {
        let rollback_error = rollback(&directory, &journal).err();
        let record_error = restore_previous_record(&directory, &journal.previous_record).err();
        let _ = fs::remove_dir_all(&transaction_root);
        let _ = fs::remove_file(&journal_path);
        return Err(rollback_error.or(record_error).unwrap_or(error));
    }

    let mut record = previous_record;
    record
        .entries
        .retain(|entry| component_of_ownership(&entry.ownership) != Some(component_id));
    record.components.remove(&component_id);
    if record.entries.is_empty() {
        // Nothing this record described is installed any more: a record with no rows would read
        // as "managed install" to `detect_source`, so the file goes rather than lying.
        fs::remove_file(directory.join("record.json")).map_err(AppError::transaction_io)?;
    } else {
        write_json_atomic(&directory.join("record.json"), &record)?;
    }
    journal.committed = true;
    write_json_atomic(&journal_path, &journal)?;
    let _ = fs::remove_dir_all(&transaction_root);
    fs::remove_file(journal_path).map_err(AppError::transaction_io)?;
    Ok(result)
}

/// Removes every component the install record speaks for, one transaction per component in
/// reverse install order, and totals the per-component results.
///
/// The command layer keeps the pre-flight (target idle, logging) and delegates the removal
/// itself here, so the order that gets tested is the order that ships. Reverse order unwinds the
/// dependency [`INSTALL_ORDER`] encodes: InventorySimulator, then the LA bundle, then upstream.
pub fn restore_all_components(state_root: &Path, target: &Path) -> Result<RestoreResult> {
    let steps = [
        ComponentId::InventorySimulator,
        ComponentId::LaPlugins,
        ComponentId::Upstream,
    ];
    let results = run_component_steps(&steps, |component| {
        uninstall_component(state_root, target, component)
    })?;
    let mut total = RestoreResult {
        restored_files: 0,
        removed_files: 0,
        preserved_files: 0,
        presets_backup: None,
        steam_verify_uri: "steam://validate/730".to_string(),
        result_kind: "restore_all_components".to_string(),
    };
    for result in results {
        total.restored_files += result.restored_files;
        total.removed_files += result.removed_files;
        total.preserved_files += result.preserved_files;
    }
    Ok(total)
}

fn collect_tree_files(
    root: &Path,
    target: &Path,
    files: &mut BTreeMap<String, PathBuf>,
) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(AppError::transaction_io)? {
        let entry = entry.map_err(AppError::transaction_io)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(AppError::transaction_io)?;
        if file_type.is_dir() {
            collect_tree_files(&path, target, files)?;
        } else if file_type.is_file() {
            let relative = path.strip_prefix(target).map_err(|_| {
                AppError::transaction(format!(
                    "Cleanup path escaped the selected CS2 directory: {}",
                    path.display()
                ))
            })?;
            let raw = relative.to_string_lossy().replace('\\', "/");
            files.insert(raw, path);
        }
    }
    Ok(())
}

fn remove_empty_tree(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(path).map_err(AppError::transaction_io)? {
        let entry = entry.map_err(AppError::transaction_io)?;
        if entry
            .file_type()
            .map_err(AppError::transaction_io)?
            .is_dir()
        {
            let _ = remove_empty_tree(&entry.path())?;
        }
    }
    if fs::read_dir(path)
        .map_err(AppError::transaction_io)?
        .next()
        .is_none()
    {
        fs::remove_dir(path).map_err(AppError::transaction_io)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn restore_pristine(
    payload_root: &Path,
    state_root: &Path,
    target: &Path,
) -> Result<RestoreResult> {
    let _transaction_lock = acquire_transaction_lock(target)?;
    recover_incomplete_locked(state_root, target)?;
    let timestamp = unix_time();
    let presets = preserve_cosmetics(state_root, target, timestamp)?;
    let directory = installation_dir(state_root, target);
    fs::create_dir_all(&directory).map_err(AppError::transaction_io)?;
    let previous_record = read_record(&directory);
    let manifest = read_manifest_document(payload_root)?;

    let mut expected = BTreeMap::new();
    for entry in &manifest.entries {
        expected.insert(entry.path.replace('\\', "/"), entry.sha256.clone());
    }
    if let Some(record) = &previous_record {
        for entry in &record.entries {
            expected.insert(
                entry.path.replace('\\', "/"),
                entry.installed_sha256.clone(),
            );
        }
    }

    let mut candidates = BTreeMap::new();
    for raw in expected.keys() {
        let relative = safe_relative(raw)?;
        let canonical = target.join(&relative);
        if canonical.is_file() {
            candidates.insert(
                relative.to_string_lossy().replace('\\', "/"),
                canonical.clone(),
            );
        }
        let disabled = mode_layout::disabled_path(&canonical);
        if disabled.is_file() {
            let disabled_relative = disabled.strip_prefix(target).map_err(|_| {
                AppError::transaction(format!(
                    "Disabled cleanup path escaped the selected CS2 directory: {}",
                    disabled.display()
                ))
            })?;
            candidates.insert(
                disabled_relative.to_string_lossy().replace('\\', "/"),
                disabled,
            );
        }
    }
    for raw in SUITE_OWNED_FILES {
        let relative = safe_relative(raw)?;
        let canonical = target.join(&relative);
        if canonical.is_file() {
            candidates.insert(
                relative.to_string_lossy().replace('\\', "/"),
                canonical.clone(),
            );
        }
        let disabled = mode_layout::disabled_path(&canonical);
        if disabled.is_file() {
            let disabled_relative = disabled.strip_prefix(target).map_err(|_| {
                AppError::transaction(format!(
                    "Disabled cleanup path escaped the selected CS2 directory: {}",
                    disabled.display()
                ))
            })?;
            candidates.insert(
                disabled_relative.to_string_lossy().replace('\\', "/"),
                disabled,
            );
        }
    }
    for raw in SUITE_OWNED_ROOTS {
        collect_tree_files(
            &target.join(raw.replace('/', "\\")),
            target,
            &mut candidates,
        )?;
    }

    let transaction_name = format!("transaction-pristine-{timestamp}");
    let transaction_root = directory.join(&transaction_name);
    let journal_path = directory.join("journal.json");
    let mut journal = TransactionJournal {
        schema_version: 1,
        operation: "restore-pristine".to_string(),
        target: target.to_string_lossy().into_owned(),
        transaction_dir: transaction_name.clone(),
        changes: Vec::new(),
        committed: false,
        previous_record: previous_record.clone(),
    };
    write_json_atomic(&journal_path, &journal)?;
    let preserved_root = directory
        .join("preserved-removed")
        .join(timestamp.to_string());
    let mut removed_files = 0;
    let mut preserved_files = 0;

    let outcome = (|| -> Result<()> {
        for (raw, destination) in &candidates {
            if !destination.is_file() {
                continue;
            }
            let relative = safe_relative(raw)?;
            let backup = transaction_root.join(&relative);
            copy_file(destination, &backup)?;
            journal.changes.push(TransactionChange {
                path: raw.clone(),
                existed: true,
                backup: Some(format!("{transaction_name}/{}", raw.replace('\\', "/"))),
            });
            write_json_atomic(&journal_path, &journal)?;

            let canonical_raw = raw.strip_suffix(".csbip-disabled").unwrap_or(raw);
            let known_unchanged = expected.get(canonical_raw).is_some_and(|hash| {
                sha256(destination)
                    .map(|actual| actual.eq_ignore_ascii_case(hash))
                    .unwrap_or(false)
            });
            if !known_unchanged && raw.as_str() != cosmetics_inventory_relative() {
                copy_file(destination, &preserved_root.join(&relative))?;
                preserved_files += 1;
            }
            fs::remove_file(destination).map_err(AppError::transaction_io)?;
            removed_files += 1;
        }
        Ok(())
    })();

    if let Err(error) = outcome {
        let rollback_error = rollback(&directory, &journal).err();
        let record_error = restore_previous_record(&directory, &journal.previous_record).err();
        let _ = fs::remove_dir_all(&transaction_root);
        let _ = fs::remove_file(&journal_path);
        return Err(rollback_error.or(record_error).unwrap_or(error));
    }

    if directory.join("record.json").is_file() {
        fs::remove_file(directory.join("record.json")).map_err(AppError::transaction_io)?;
    }
    journal.committed = true;
    write_json_atomic(&journal_path, &journal)?;
    let _ = fs::remove_dir_all(&transaction_root);
    fs::remove_file(&journal_path).map_err(AppError::transaction_io)?;
    for raw in SUITE_OWNED_ROOTS.iter().rev() {
        let _ = remove_empty_tree(&target.join(raw.replace('/', "\\")));
    }

    Ok(RestoreResult {
        restored_files: 0,
        removed_files,
        preserved_files,
        presets_backup: presets.map(|path| path.to_string_lossy().into_owned()),
        steam_verify_uri: "steam://validate/730".to_string(),
        result_kind: "pristine".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_story_requires_a_release_first_install_from_the_full_package() {
        for migration_kind in [
            MigrationKind::FreshInstall,
            MigrationKind::AdoptLegacyPlus,
            MigrationKind::ReplaceUpstream,
        ] {
            assert!(welcome_story_prompt_eligible(true, true, migration_kind, false));
            assert!(!welcome_story_prompt_eligible(false, true, migration_kind, false));
            assert!(!welcome_story_prompt_eligible(true, false, migration_kind, false));
            assert!(!welcome_story_prompt_eligible(true, true, migration_kind, true));
        }
        assert!(!welcome_story_prompt_eligible(
            true,
            true,
            MigrationKind::ManagedUpgrade,
            false,
        ));
        assert!(!welcome_story_prompt_eligible(
            true,
            true,
            MigrationKind::Blocked,
            false,
        ));
    }

    fn root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("cs2bi-installer-{name}-{}", unix_time()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn fixture(payload: &Path, target: &Path) {
        fs::create_dir_all(payload.join("cfg")).unwrap();
        fs::create_dir_all(target.join("cfg")).unwrap();
        fs::write(payload.join("cfg/test.cfg"), b"plus").unwrap();
        fs::write(target.join("cfg/test.cfg"), b"steam").unwrap();
        fs::write(target.join("cfg/foreign.cfg"), b"foreign").unwrap();
        let hash = sha256(&payload.join("cfg/test.cfg")).unwrap();
        write_json_atomic(
            &payload.join(MANIFEST_FILE),
            &PayloadManifest {
                schema_version: 1,
                package_version: "1.4.2.4".to_string(),
                entries: vec![PayloadEntry {
                    path: "cfg/test.cfg".to_string(),
                    size: 4,
                    sha256: hash,
                    component: "config".to_string(),
                    ownership: "plus".to_string(),
                    restore_policy: "restore".to_string(),
                }],
            },
        )
        .unwrap();
    }

    fn cosmetics_fixture(payload: &Path, target: &Path) {
        fixture(payload, target);
        let relative = cosmetics_inventory_relative();
        let source = payload.join(&relative);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let bytes = b"packaged-inventory";
        fs::write(&source, bytes).unwrap();
        let mut manifest = read_manifest_document(payload).unwrap();
        manifest.entries.push(PayloadEntry {
            path: relative,
            size: bytes.len() as u64,
            sha256: sha256(&source).unwrap(),
            component: "player-cosmetics".to_string(),
            ownership: "plus".to_string(),
            restore_policy: "preserve-config".to_string(),
        });
        write_json_atomic(&payload.join(MANIFEST_FILE), &manifest).unwrap();
    }

    #[test]
    fn same_root_allows_modified_preserve_config_but_rejects_other_payload_changes() {
        let base = root("same-payload-target");
        let target = base.join("game/csgo");
        let state = base.join("state");
        cosmetics_fixture(&target, &target);
        let mut manifest = read_manifest_document(&target).unwrap();
        let cfg = manifest
            .entries
            .iter_mut()
            .find(|entry| entry.path == "cfg/test.cfg")
            .unwrap();
        cfg.size = fs::metadata(target.join("cfg/test.cfg")).unwrap().len();
        cfg.sha256 = sha256(&target.join("cfg/test.cfg")).unwrap();
        write_json_atomic(&target.join(MANIFEST_FILE), &manifest).unwrap();
        let gun_presets = target.join(cosmetics_inventory_relative());
        fs::create_dir_all(gun_presets.parent().unwrap()).unwrap();
        fs::write(&gun_presets, b"player-modified-guns").unwrap();

        assert!(verify_payload(&target).is_err());
        verify_payload_for_target(&target, &target).unwrap();
        assert!(plan(&target, &state, &target).is_ok());
        assert!(install(&target, &state, &target, false).is_ok());
        assert_eq!(fs::read(&gun_presets).unwrap(), b"player-modified-guns");

        fs::write(target.join("cfg/test.cfg"), b"damaged").unwrap();
        assert!(verify_payload_for_target(&target, &target).is_err());
        fs::remove_dir_all(base).unwrap();
    }

    fn add_payload_file(payload: &Path, raw: &str, bytes: &[u8]) {
        let path = payload.join(raw.replace('/', "\\"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        let mut manifest = read_manifest_document(payload).unwrap();
        manifest.entries.push(PayloadEntry {
            path: raw.to_string(),
            size: bytes.len() as u64,
            sha256: sha256(&path).unwrap(),
            component: "match-rating".to_string(),
            ownership: "plus".to_string(),
            restore_policy: "restore".to_string(),
        });
        write_json_atomic(&payload.join(MANIFEST_FILE), &manifest).unwrap();
    }

    fn marker(target: &Path, raw: &str) {
        let path = target.join(raw.replace('/', "\\"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"marker").unwrap();
    }

    const COMPONENT_SHARED: [&str; 2] = [
        "addons/counterstrikesharp/plugins/BotAI/BotAI.dll",
        "addons/counterstrikesharp/shared/BotHiderApi/BotHiderApi.dll",
    ];
    const COMPONENT_PLUS: [&str; 2] = [
        "addons/counterstrikesharp/plugins/PlusMatchCoordinator/PlusMatchCoordinator.dll",
        "addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.dll",
    ];
    const THIRD_PARTY: &str =
        "addons/counterstrikesharp/plugins/InventorySimulator/InventorySimulator.dll";

    fn write_owned_entry(
        payload: &Path,
        manifest: &mut PayloadManifest,
        raw: &str,
        bytes: &[u8],
        ownership: &str,
    ) {
        let path = payload.join(safe_relative(raw).unwrap());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        manifest.entries.push(PayloadEntry {
            path: raw.to_string(),
            size: bytes.len() as u64,
            sha256: sha256(&path).unwrap(),
            component: if ownership == "plus" {
                "PlusMatchCoordinator".to_string()
            } else {
                "runtime".to_string()
            },
            ownership: ownership.to_string(),
            restore_policy: "restore".to_string(),
        });
    }

    /// Two shared entries always, plus the two Local Arena plugin entries when requested.
    fn component_payload(payload: &Path, include_plus: bool) -> PayloadManifest {
        let mut manifest = PayloadManifest {
            schema_version: 1,
            package_version: "1.4.3.3".to_string(),
            entries: Vec::new(),
        };
        write_owned_entry(
            payload,
            &mut manifest,
            COMPONENT_SHARED[0],
            b"shared-bot-ai",
            "shared",
        );
        write_owned_entry(
            payload,
            &mut manifest,
            COMPONENT_SHARED[1],
            b"shared-bothider-api",
            "shared",
        );
        if include_plus {
            write_owned_entry(
                payload,
                &mut manifest,
                COMPONENT_PLUS[0],
                b"plus-match-coordinator",
                "plus",
            );
            write_owned_entry(
                payload,
                &mut manifest,
                COMPONENT_PLUS[1],
                b"plus-team-lineup-injector",
                "plus",
            );
        }
        write_json_atomic(&payload.join(MANIFEST_FILE), &manifest).unwrap();
        manifest
    }

    /// Target-side path for a manifest entry: `safe_relative` is the one join the installer
    /// itself uses on every platform, so tests must not assume `/` becomes `\\` here.
    fn owned_target(target: &Path, raw: &str) -> PathBuf {
        target.join(safe_relative(raw).unwrap())
    }

    /// A synthetic third-party component payload, shaped like the manifest
    /// `inventory_plugin::synthetic_manifest` commits: its `package_version` is the component's
    /// own release tag, never a suite version.
    fn third_party_payload(payload: &Path) -> PayloadManifest {
        let mut manifest = PayloadManifest {
            schema_version: 1,
            package_version: "3.7.0".to_string(),
            entries: Vec::new(),
        };
        write_owned_entry(payload, &mut manifest, THIRD_PARTY, b"third-party-plugin", "third-party");
        write_json_atomic(&payload.join(MANIFEST_FILE), &manifest).unwrap();
        manifest
    }

    #[test]
    fn distinguishes_clean_legacy_plus_upstream_mixed_and_managed_installs() {
        let base = root("sources");
        let payload = base.join("payload");
        let state = base.join("state");

        let clean = base.join("clean");
        fixture(&payload, &clean);
        assert_eq!(
            inspect(&payload, &state, &clean).unwrap().source,
            InstallationSource::Clean
        );

        let legacy = base.join("legacy");
        fs::create_dir_all(&legacy).unwrap();
        marker(&legacy, PLUS_MARKERS[1]);
        let legacy_inspection = inspect(&payload, &state, &legacy).unwrap();
        assert_eq!(legacy_inspection.source, InstallationSource::LegacyPlus);
        assert_eq!(
            legacy_inspection.migration_kind,
            MigrationKind::AdoptLegacyPlus
        );

        let upstream = base.join("upstream");
        fs::create_dir_all(&upstream).unwrap();
        for raw in UPSTREAM_MARKERS.iter().take(3) {
            marker(&upstream, raw);
        }
        let upstream_inspection = inspect(&payload, &state, &upstream).unwrap();
        assert_eq!(upstream_inspection.source, InstallationSource::Upstream);
        assert_eq!(
            upstream_inspection.migration_kind,
            MigrationKind::ReplaceUpstream
        );

        let mixed = base.join("mixed");
        fs::create_dir_all(&mixed).unwrap();
        marker(&mixed, UPSTREAM_MARKERS[0]);
        let mixed_inspection = inspect(&payload, &state, &mixed).unwrap();
        assert_eq!(mixed_inspection.source, InstallationSource::MixedUnknown);
        assert!(!mixed_inspection.can_install);
        assert!(install(&payload, &state, &mixed, false).is_err());

        install(&payload, &state, &clean, false).unwrap();
        let managed = inspect(&payload, &state, &clean).unwrap();
        assert_eq!(managed.source, InstallationSource::ManagedPlus);
        assert_eq!(managed.source_version.as_deref(), Some("1.4.2.4"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn managed_upgrade_replaces_the_unchanged_rating_plus_model_with_open_rating() {
        let base = root("open-rating-upgrade");
        let old_payload = base.join("old-payload");
        let new_payload = base.join("new-payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&old_payload, &target);
        add_payload_file(&old_payload, SUPERSEDED_PAYLOAD_PATHS[0].0, b"old-model");
        install(&old_payload, &state, &target, false).unwrap();

        fixture(&new_payload, &base.join("new-fixture-target"));
        add_payload_file(&new_payload, SUPERSEDED_PAYLOAD_PATHS[0].1, b"open-model");
        install(&new_payload, &state, &target, false).unwrap();

        assert!(!target.join(SUPERSEDED_PAYLOAD_PATHS[0].0.replace('/', "\\")).exists());
        assert_eq!(
            fs::read(target.join(SUPERSEDED_PAYLOAD_PATHS[0].1.replace('/', "\\"))).unwrap(),
            b"open-model"
        );
        let record = read_record(&installation_dir(&state, &target)).unwrap();
        assert!(!record.entries.iter().any(|entry| entry.path == SUPERSEDED_PAYLOAD_PATHS[0].0));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn managed_upgrade_restores_a_pre_plus_file_at_the_superseded_model_path() {
        let base = root("open-rating-original-restore");
        let old_payload = base.join("old-payload");
        let new_payload = base.join("new-payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&old_payload, &target);
        let old_model = target.join(SUPERSEDED_PAYLOAD_PATHS[0].0.replace('/', "\\"));
        fs::create_dir_all(old_model.parent().unwrap()).unwrap();
        fs::write(&old_model, b"pre-plus-model").unwrap();
        add_payload_file(&old_payload, SUPERSEDED_PAYLOAD_PATHS[0].0, b"old-model");
        install(&old_payload, &state, &target, false).unwrap();

        fixture(&new_payload, &base.join("new-fixture-target"));
        add_payload_file(&new_payload, SUPERSEDED_PAYLOAD_PATHS[0].1, b"open-model");
        install(&new_payload, &state, &target, false).unwrap();

        assert_eq!(fs::read(&old_model).unwrap(), b"pre-plus-model");
        assert!(target.join(SUPERSEDED_PAYLOAD_PATHS[0].1.replace('/', "\\")).is_file());
        let record = read_record(&installation_dir(&state, &target)).unwrap();
        assert!(!record.entries.iter().any(|entry| entry.path == SUPERSEDED_PAYLOAD_PATHS[0].0));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn open_rating_install_preserves_an_untracked_legacy_model() {
        let base = root("open-rating-untracked-file");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&payload, &target);
        let old_model = target.join(SUPERSEDED_PAYLOAD_PATHS[0].0.replace('/', "\\"));
        fs::create_dir_all(old_model.parent().unwrap()).unwrap();
        fs::write(&old_model, b"untracked-model").unwrap();
        add_payload_file(&payload, SUPERSEDED_PAYLOAD_PATHS[0].1, b"open-model");

        install(&payload, &state, &target, false).unwrap();

        assert_eq!(fs::read(old_model).unwrap(), b"untracked-model");
        assert!(target.join(SUPERSEDED_PAYLOAD_PATHS[0].1.replace('/', "\\")).is_file());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn managed_upgrade_preserves_a_modified_legacy_rating_model() {
        let base = root("open-rating-user-file");
        let old_payload = base.join("old-payload");
        let new_payload = base.join("new-payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&old_payload, &target);
        add_payload_file(&old_payload, SUPERSEDED_PAYLOAD_PATHS[0].0, b"old-model");
        install(&old_payload, &state, &target, false).unwrap();
        let old_model = target.join(SUPERSEDED_PAYLOAD_PATHS[0].0.replace('/', "\\"));
        fs::write(&old_model, b"user-modified-model").unwrap();

        fixture(&new_payload, &base.join("new-fixture-target"));
        add_payload_file(&new_payload, SUPERSEDED_PAYLOAD_PATHS[0].1, b"open-model");
        install(&new_payload, &state, &target, false).unwrap();

        assert_eq!(fs::read(old_model).unwrap(), b"user-modified-model");
        assert!(target.join(SUPERSEDED_PAYLOAD_PATHS[0].1.replace('/', "\\")).is_file());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn legacy_plus_adoption_records_a_pre_migration_restore_baseline() {
        let base = root("legacy-baseline");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        cosmetics_fixture(&payload, &target);
        let existing = target.join(PLUS_MARKERS[1].replace('/', "\\"));
        fs::create_dir_all(existing.parent().unwrap()).unwrap();
        fs::write(&existing, b"legacy-player-knives").unwrap();

        install(&payload, &state, &target, false).unwrap();

        let record = read_record(&installation_dir(&state, &target)).unwrap();
        assert_eq!(record.source, InstallationSource::LegacyPlus);
        assert_eq!(record.restore_baseline, RestoreBaseline::PreMigration);
        assert_eq!(fs::read(existing).unwrap(), b"legacy-player-knives");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn pristine_restore_removes_suite_files_but_keeps_third_party_files() {
        let base = root("pristine");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        cosmetics_fixture(&payload, &target);
        install(&payload, &state, &target, false).unwrap();
        let cosmetics = target.join(cosmetics_inventory_relative());
        fs::create_dir_all(cosmetics.parent().unwrap()).unwrap();
        fs::write(&cosmetics, b"player-inventory").unwrap();
        let third_party = target.join("addons/counterstrikesharp/plugins/ThirdParty/keep.dll");
        fs::create_dir_all(third_party.parent().unwrap()).unwrap();
        fs::write(&third_party, b"third-party").unwrap();

        let result = restore_pristine(&payload, &state, &target).unwrap();

        assert!(result.removed_files >= 3);
        assert_eq!(result.result_kind, "pristine");
        assert!(!target.join("cfg/test.cfg").exists());
        assert!(!cosmetics.exists());
        assert_eq!(fs::read(&third_party).unwrap(), b"third-party");
        let presets = PathBuf::from(result.presets_backup.unwrap());
        assert_eq!(
            fs::read(presets.join("inventories.json")).unwrap(),
            b"player-inventory"
        );
        assert!(!inspect(&payload, &state, &target).unwrap().installed);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn install_repair_and_restore_preserve_originals_and_foreign_files() {
        let base = root("roundtrip");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&payload, &target);

        install(&payload, &state, &target, false).unwrap();
        assert_eq!(fs::read(target.join("cfg/test.cfg")).unwrap(), b"plus");
        fs::write(target.join("cfg/test.cfg"), b"broken").unwrap();
        assert_eq!(inspect(&payload, &state, &target).unwrap().corrupt.len(), 1);
        let repaired = install(&payload, &state, &target, true).unwrap();
        assert_eq!(repaired.installed_files, 1);
        let unchanged = install(&payload, &state, &target, true).unwrap();
        assert_eq!(unchanged.installed_files, 0);
        restore(&payload, &state, &target).unwrap();
        assert_eq!(fs::read(target.join("cfg/test.cfg")).unwrap(), b"steam");
        assert_eq!(
            fs::read(target.join("cfg/foreign.cfg")).unwrap(),
            b"foreign"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn modified_preserve_config_is_healthy_and_repair_keeps_player_bytes() {
        let base = root("preserve-modified");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        cosmetics_fixture(&payload, &target);
        install(&payload, &state, &target, false).unwrap();

        let inventory = target.join(cosmetics_inventory_relative());
        fs::create_dir_all(inventory.parent().unwrap()).unwrap();
        fs::write(&inventory, b"player-inventory").unwrap();

        let inspection = inspect(&payload, &state, &target).unwrap();
        assert!(inspection.missing.is_empty());
        assert!(inspection.corrupt.is_empty());
        assert_eq!(inspection.healthy, inspection.total);
        assert_eq!(
            install(&payload, &state, &target, true)
                .unwrap()
                .installed_files,
            0
        );
        assert_eq!(fs::read(&inventory).unwrap(), b"player-inventory");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn first_install_preserves_existing_manual_cosmetics() {
        let base = root("preserve-manual-upgrade");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        cosmetics_fixture(&payload, &target);
        let inventory = target.join(cosmetics_inventory_relative());
        fs::create_dir_all(inventory.parent().unwrap()).unwrap();
        fs::write(&inventory, b"legacy-player-inventory").unwrap();

        install(&payload, &state, &target, false).unwrap();
        assert_eq!(fs::read(&inventory).unwrap(), b"legacy-player-inventory");
        let inspection = inspect(&payload, &state, &target).unwrap();
        assert!(inspection.corrupt.is_empty());
        assert_eq!(inspection.healthy, inspection.total);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn repair_restores_missing_preserve_config_from_current_mirror() {
        let base = root("preserve-mirror");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        cosmetics_fixture(&payload, &target);
        install(&payload, &state, &target, false).unwrap();

        let relative = safe_relative(&cosmetics_inventory_relative()).unwrap();
        let destination = target.join(&relative);
        fs::create_dir_all(state.join("presets/current")).unwrap();
        fs::write(
            state.join("presets/current/inventories.json"),
            b"mirrored-inventory",
        )
        .unwrap();
        fs::remove_file(&destination).unwrap();

        let inspection = inspect(&payload, &state, &target).unwrap();
        assert_eq!(
            inspection.missing,
            vec![relative.to_string_lossy().replace('\\', "/")]
        );
        assert_eq!(
            install(&payload, &state, &target, true)
                .unwrap()
                .installed_files,
            1
        );
        assert_eq!(fs::read(&destination).unwrap(), b"mirrored-inventory");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    #[ignore = "requires CS2BI_REAL_PAYLOAD to point at a packaged payload"]
    fn packaged_payload_install_repair_restore_roundtrip() {
        let payload = PathBuf::from(
            std::env::var("CS2BI_REAL_PAYLOAD").expect("CS2BI_REAL_PAYLOAD is required"),
        );
        let manifest = read_manifest(&payload).unwrap();
        let base = root("packaged-roundtrip");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();

        let original = manifest
            .entries
            .iter()
            .find(|entry| entry.path != cosmetics_inventory_relative())
            .expect("payload needs a restorable file");
        let original_path = target.join(safe_relative(&original.path).unwrap());
        fs::create_dir_all(original_path.parent().unwrap()).unwrap();
        fs::write(&original_path, b"steam-original").unwrap();
        let foreign = target.join("addons/third-party/keep.txt");
        fs::create_dir_all(foreign.parent().unwrap()).unwrap();
        fs::write(&foreign, b"foreign").unwrap();

        let planned = plan(&payload, &state, &target).unwrap();
        assert_eq!(planned.total_files, manifest.entries.len());
        assert_eq!(planned.overwritten_files, 1);
        install(&payload, &state, &target, false).unwrap();
        let installed = inspect(&payload, &state, &target).unwrap();
        assert_eq!(installed.healthy, installed.total);

        let inventory = manifest
            .entries
            .iter()
            .find(|entry| entry.path == cosmetics_inventory_relative())
            .expect("packaged inventory is missing");
        let inventory_path = target.join(safe_relative(&inventory.path).unwrap());
        fs::write(&inventory_path, b"player-modified-inventory").unwrap();
        let customized = inspect(&payload, &state, &target).unwrap();
        assert!(customized.corrupt.is_empty());
        assert_eq!(customized.healthy, customized.total);

        let damaged = manifest
            .entries
            .iter()
            .find(|entry| entry.path != original.path && entry.restore_policy != "preserve-config")
            .expect("payload needs at least two restorable files");
        fs::write(
            target.join(safe_relative(&damaged.path).unwrap()),
            b"damaged",
        )
        .unwrap();
        assert_eq!(inspect(&payload, &state, &target).unwrap().corrupt.len(), 1);
        assert_eq!(
            install(&payload, &state, &target, true)
                .unwrap()
                .installed_files,
            1
        );
        assert_eq!(fs::read(&inventory_path).unwrap(), b"player-modified-inventory");

        let restored = restore(&payload, &state, &target).unwrap();
        assert_eq!(fs::read(&original_path).unwrap(), b"steam-original");
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert!(!inventory_path.exists());
        let preset_backup =
            PathBuf::from(restored.presets_backup.expect("preset backup is required"));
        assert_eq!(
            fs::read(preset_backup.join("inventories.json")).unwrap(),
            b"player-modified-inventory"
        );
        assert!(!inspect(&payload, &state, &target).unwrap().installed);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_parent_directory_payload_paths() {
        assert!(safe_relative("../gameinfo.gi").is_err());
        assert!(safe_relative(r"C:\\game\\csgo").is_err());
    }

    #[test]
    fn panel_only_update_package_has_an_actionable_first_install_error() {
        let base = root("panel-only-marker");
        let payload = base.join("panel-only");
        fs::create_dir_all(&payload).unwrap();
        fs::write(payload.join(PANEL_UPDATE_MARKER), b"{}").unwrap();

        let error = read_manifest_document(&payload).unwrap_err();

        assert_eq!(error.code, "E1301");
        assert!(error.detail.contains("Panel-only online-update component"));
        assert!(error.detail.contains(&format!(
            "LocalArena-v{}-windows.zip",
            crate::app_version::display()
        )));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn active_installation_is_not_rolled_back_by_snapshot_recovery() {
        let base = root("active-install-recovery");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        let mut entries = Vec::new();
        for index in 0..512 {
            let relative = format!("addons/runtime/file-{index:04}.bin");
            let source = payload.join(safe_relative(&relative).unwrap());
            let destination = target.join(safe_relative(&relative).unwrap());
            fs::create_dir_all(source.parent().unwrap()).unwrap();
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            let bytes = vec![(index % 251) as u8; 4096];
            fs::write(&source, &bytes).unwrap();
            fs::write(&destination, b"legacy").unwrap();
            entries.push(PayloadEntry {
                path: relative,
                size: bytes.len() as u64,
                sha256: sha256(&source).unwrap(),
                component: "runtime".to_string(),
                ownership: "shared".to_string(),
                restore_policy: "restore".to_string(),
            });
        }
        write_json_atomic(
            &payload.join(MANIFEST_FILE),
            &PayloadManifest {
                schema_version: 1,
                package_version: "1.4.2.4".to_string(),
                entries,
            },
        )
        .unwrap();

        let install_payload = payload.clone();
        let install_target = target.clone();
        let install_state = state.clone();
        let worker = std::thread::spawn(move || {
            install(&install_payload, &install_state, &install_target, false)
        });

        let journal = installation_dir(&state, &target).join("journal.json");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !journal.is_file() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            journal.is_file(),
            "the installation did not expose an active journal"
        );
        assert!(
            !recover_incomplete(&state, &target).unwrap(),
            "snapshot recovery must not roll back an active installation"
        );

        worker.join().unwrap().unwrap();
        assert!(!journal.exists());
        assert_eq!(inspect(&payload, &state, &target).unwrap().healthy, 512);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn transient_file_disappearance_is_retried_with_contextual_failures() {
        let base = root("transient-copy");
        let source = base.join("late/source.bin");
        let destination = base.join("destination/copied.bin");
        let late_source = source.clone();
        let creator = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(90));
            fs::create_dir_all(late_source.parent().unwrap()).unwrap();
            fs::write(late_source, b"available-after-scan").unwrap();
        });

        copy_file_for("payload staging", &source, &destination).unwrap();
        creator.join().unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"available-after-scan");

        let missing = base.join("permanently-missing.bin");
        let failed_destination = base.join("failed/output.bin");
        let error = copy_file_for("original backup", &missing, &failed_destination).unwrap_err();
        assert!(error.detail.contains("original backup"));
        assert!(
            error
                .detail
                .contains(&missing.to_string_lossy().to_string())
        );
        assert!(
            error
                .detail
                .contains(&failed_destination.to_string_lossy().to_string())
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn interrupted_transaction_restores_files_and_previous_record_state() {
        let base = root("interrupted");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&payload, &target);
        let directory = installation_dir(&state, &target);
        let transaction = directory.join("transaction-test");
        fs::create_dir_all(transaction.join("cfg")).unwrap();
        fs::copy(
            target.join("cfg/test.cfg"),
            transaction.join("cfg/test.cfg"),
        )
        .unwrap();
        fs::write(target.join("cfg/test.cfg"), b"partial").unwrap();
        fs::create_dir_all(&directory).unwrap();
        write_json_atomic(
            &directory.join("journal.json"),
            &TransactionJournal {
                schema_version: 1,
                operation: "install".to_string(),
                target: target.to_string_lossy().into_owned(),
                transaction_dir: "transaction-test".to_string(),
                changes: vec![TransactionChange {
                    path: "cfg/test.cfg".to_string(),
                    existed: true,
                    backup: Some("transaction-test/cfg/test.cfg".to_string()),
                }],
                committed: false,
                previous_record: None,
            },
        )
        .unwrap();

        assert!(recover_incomplete(&state, &target).unwrap());
        assert_eq!(fs::read(target.join("cfg/test.cfg")).unwrap(), b"steam");
        assert!(!directory.join("record.json").exists());
        assert!(!directory.join("journal.json").exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn failed_restore_rolls_back_to_installed_state_and_record() {
        let base = root("restore-rollback");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&payload, &target);
        install(&payload, &state, &target, false).unwrap();
        let directory = installation_dir(&state, &target);
        fs::remove_file(directory.join("original/cfg/test.cfg")).unwrap();

        assert!(restore(&payload, &state, &target).is_err());
        assert_eq!(fs::read(target.join("cfg/test.cfg")).unwrap(), b"plus");
        assert!(directory.join("record.json").is_file());
        assert!(!directory.join("journal.json").exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn component_install_writes_only_owned_files_and_merges_one_record() {
        let base = root("component-subset");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();

        // Seed: a full-package install that only carried the shared half.
        component_payload(&payload, false);
        install(&payload, &state, &target, false).unwrap();
        let directory = installation_dir(&state, &target);
        let seeded = read_record(&directory).unwrap();
        assert_eq!(seeded.entries.len(), 2);
        assert!(
            seeded
                .entries
                .iter()
                .all(|entry| entry.ownership == "shared")
        );

        let shared = owned_target(&target, COMPONENT_SHARED[0]);
        fs::write(&shared, b"player-modified").unwrap();
        let plus = owned_target(&target, COMPONENT_PLUS[0]);
        assert!(!plus.exists());

        // The component payload adds the plus half to the same manifest.
        component_payload(&payload, true);
        let result = install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();

        assert_eq!(result.installed_files, 2);
        assert_eq!(result.package_version, "1.4.3.3");
        assert_eq!(fs::read(&plus).unwrap(), b"plus-match-coordinator");
        assert_eq!(
            fs::read(owned_target(&target, COMPONENT_PLUS[1])).unwrap(),
            b"plus-team-lineup-injector"
        );
        // The shared half is outside the filter: not copied over, player bytes intact.
        assert_eq!(fs::read(&shared).unwrap(), b"player-modified");

        let merged = read_record(&directory).unwrap();
        let mut paths = merged
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        paths.sort_unstable();
        let mut expected = COMPONENT_SHARED
            .iter()
            .chain(COMPONENT_PLUS.iter())
            .copied()
            .collect::<Vec<_>>();
        expected.sort_unstable();
        assert_eq!(paths, expected);
        for entry in &seeded.entries {
            let carried = merged
                .entries
                .iter()
                .find(|candidate| candidate.path == entry.path)
                .expect("the seed entry must survive the component install");
            assert_eq!(carried.installed_sha256, entry.installed_sha256);
            assert_eq!(carried.original_sha256, entry.original_sha256);
            assert_eq!(carried.ownership, "shared");
        }
        // One record file per target, never a per-component fork.
        let names = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["record.json".to_string()]);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn component_install_rejects_a_payload_with_no_owned_files() {
        let base = root("component-empty");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        component_payload(&payload, false);

        let error = install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap_err();

        assert_eq!(error.code, "E1301");
        assert!(error.detail.contains("no owned files"), "{}", error.detail);
        assert!(!installation_dir(&state, &target).exists());
        assert!(fs::read_dir(&target).unwrap().next().is_none());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn la_plugin_component_requires_the_bothider_api_first() {
        let base = root("component-preflight");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        component_payload(&payload, true);

        let error = install_la_plugins_component(&payload, &state, &target).unwrap_err();

        assert_eq!(error.code, "E1404");
        assert!(error.detail.contains("BotHiderApi"), "{}", error.detail);
        assert!(fs::read_dir(&target).unwrap().next().is_none());

        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();
        let result = install_la_plugins_component(&payload, &state, &target).unwrap();

        assert_eq!(result.installed_files, 2);
        assert_eq!(
            fs::read(owned_target(&target, COMPONENT_PLUS[0])).unwrap(),
            b"plus-match-coordinator"
        );
        // The ownership filter still applies: this path never installs the shared half.
        assert!(!owned_target(&target, COMPONENT_SHARED[0]).exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn component_install_rerun_skips_identical_files() {
        let base = root("component-idempotent");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        component_payload(&payload, true);
        // The LA bundle installs only once the upstream-managed BotHiderApi is present; write it
        // unregistered here so the record this test inspects still holds only the plus entries.
        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();

        let first = install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();
        assert_eq!(first.installed_files, 2);
        let plus = owned_target(&target, COMPONENT_PLUS[0]);
        let installed_sha = sha256(&plus).unwrap();

        let second = install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();

        assert_eq!(second.installed_files, 0);
        assert_eq!(sha256(&plus).unwrap(), installed_sha);
        let directory = installation_dir(&state, &target);
        assert!(!directory.join("journal.json").exists());
        assert_eq!(read_record(&directory).unwrap().entries.len(), 2);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn component_install_recovers_an_interrupted_transaction_first() {
        let base = root("component-recovery");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        component_payload(&payload, true);

        // A crashed earlier transaction left a half-written shared file and its journal.
        let directory = installation_dir(&state, &target);
        let transaction = directory.join("transaction-stale");
        let shared = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(shared.parent().unwrap()).unwrap();
        fs::write(&shared, b"partial-write").unwrap();
        fs::create_dir_all(transaction.join("addons/counterstrikesharp/shared/BotHiderApi"))
            .unwrap();
        fs::write(
            transaction.join("addons/counterstrikesharp/shared/BotHiderApi/BotHiderApi.dll"),
            b"steam-original",
        )
        .unwrap();
        write_json_atomic(
            &directory.join("journal.json"),
            &TransactionJournal {
                schema_version: 1,
                operation: "install".to_string(),
                target: target.to_string_lossy().into_owned(),
                transaction_dir: "transaction-stale".to_string(),
                changes: vec![TransactionChange {
                    path: COMPONENT_SHARED[1].to_string(),
                    existed: true,
                    backup: Some(format!("transaction-stale/{}", COMPONENT_SHARED[1])),
                }],
                committed: false,
                previous_record: None,
            },
        )
        .unwrap();

        let result = install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();

        assert_eq!(result.installed_files, 2);
        assert_eq!(fs::read(&shared).unwrap(), b"steam-original");
        assert!(!directory.join("journal.json").exists());
        assert!(!transaction.exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn component_install_preserves_the_suite_record_version() {
        let base = root("component-suite-version");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&payload, &target);
        install(&payload, &state, &target, false).unwrap();
        third_party_payload(&payload);

        let result = install_component(&payload, &state, &target, ComponentId::InventorySimulator).unwrap();

        // The transaction installed the component's tag ...
        assert_eq!(result.package_version, "3.7.0");
        // ... but the merged record still speaks for the suite.
        let record = read_record(&installation_dir(&state, &target)).unwrap();
        assert_eq!(record.package_version, "1.4.2.4");
        assert_eq!(record.entries.len(), 2);
        let inspection = inspect_quick(&payload, &state, &target).unwrap();
        assert_eq!(inspection.package_version.as_deref(), Some("1.4.2.4"));
        assert_eq!(inspection.source, InstallationSource::ManagedPlus);
        assert_eq!(inspection.source_version.as_deref(), Some("1.4.2.4"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn restore_leaves_third_party_files_in_place() {
        let base = root("restore-third-party");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&payload, &target);
        install(&payload, &state, &target, false).unwrap();
        third_party_payload(&payload);
        install_component(&payload, &state, &target, ComponentId::InventorySimulator).unwrap();
        let third_party = owned_target(&target, THIRD_PARTY);

        let result = restore(&payload, &state, &target).unwrap();

        // The suite file returns to its Steam bytes; only that file was touched.
        assert_eq!(fs::read(target.join("cfg/test.cfg")).unwrap(), b"steam");
        assert_eq!(fs::read(&third_party).unwrap(), b"third-party-plugin");
        assert_eq!(result.restored_files, 1);
        assert_eq!(result.removed_files, 0);
        assert!(read_record(&installation_dir(&state, &target)).is_none());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_third_party_only_record_is_not_a_managed_suite_installation() {
        let base = root("third-party-only-record");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        third_party_payload(&payload);
        install_component(&payload, &state, &target, ComponentId::InventorySimulator).unwrap();

        // The component install wrote the only record there is, carrying its own tag.
        let record = read_record(&installation_dir(&state, &target)).unwrap();
        assert_eq!(record.package_version, "3.7.0");
        assert!(record.entries.iter().all(|entry| entry.ownership == "third-party"));

        // Detection must not sell that tag as an installed suite version.
        let inspection = inspect_quick(&payload, &state, &target).unwrap();
        assert!(!inspection.installed);
        assert_eq!(inspection.package_version, None);
        assert_eq!(inspection.source, InstallationSource::Clean);
        assert_eq!(inspection.source_version, None);
        assert!(inspection.can_install);

        // And a restore is a no-op that still leaves the third-party files alone.
        let result = restore(&payload, &state, &target).unwrap();
        assert_eq!(result.restored_files, 0);
        assert_eq!(result.removed_files, 0);
        assert_eq!(
            fs::read(owned_target(&target, THIRD_PARTY)).unwrap(),
            b"third-party-plugin"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_third_party_tag_never_becomes_the_suite_version_of_a_merged_record() {
        let base = root("third-party-then-plus");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();

        // The InventorySimulator card installs first on a machine without a suite record, so the
        // record that exists is third-party-only and carries that component's release tag.
        third_party_payload(&payload);
        install_component(&payload, &state, &target, ComponentId::InventorySimulator).unwrap();
        let directory = installation_dir(&state, &target);
        assert_eq!(read_record(&directory).unwrap().package_version, "3.7.0");

        // The machine already carries the upstream-managed DLL the bundle links against (a
        // legacy suite without a record): the dependency gate requires the file, not a record.
        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();

        // The LA bundle card then merges its `plus` subset into that same record.
        component_payload(&payload, true);
        install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();

        // The record now speaks for the suite, so it must report the bundle's version, not the
        // third-party tag it happened to inherit. A four-part parse of "3.7.0" would silently
        // kill update detection.
        let record = read_record(&directory).unwrap();
        assert_eq!(record.package_version, "1.4.3.3");
        assert!(is_suite_record(&record));
        assert_eq!(record.entries.len(), 3);
        assert!(record
            .entries
            .iter()
            .any(|entry| entry.path == THIRD_PARTY && entry.ownership == "third-party"));
        assert!(record
            .entries
            .iter()
            .any(|entry| entry.path == COMPONENT_PLUS[0] && entry.ownership == "plus"));

        let inspection = inspect_quick(&payload, &state, &target).unwrap();
        assert!(inspection.installed);
        assert_eq!(inspection.package_version.as_deref(), Some("1.4.3.3"));
        assert_eq!(inspection.source, InstallationSource::ManagedPlus);
        assert_eq!(inspection.source_version.as_deref(), Some("1.4.3.3"));
        fs::remove_dir_all(base).unwrap();
    }

    /// Rewrites only the manifest's package version, leaving every entry (and its hash) intact.
    fn set_manifest_version(payload: &Path, version: &str) {
        let mut manifest = read_manifest_document(payload).unwrap();
        manifest.package_version = version.to_string();
        write_json_atomic(&payload.join(MANIFEST_FILE), &manifest).unwrap();
    }

    #[test]
    fn a_plus_component_install_records_the_bundle_version() {
        let base = root("suite-then-plus");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fixture(&payload, &target);
        install(&payload, &state, &target, false).unwrap();
        // The upstream half of a real suite install supplies the DLL the bundle links against.
        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();

        // The bundle card installs through this path, so the newly installed bundle's version is
        // the one update detection must report; keeping the older suite version would leave the
        // card offering the same update forever.
        component_payload(&payload, true);
        install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();

        let record = read_record(&installation_dir(&state, &target)).unwrap();
        assert_eq!(record.package_version, "1.4.3.3");
        assert_eq!(record.entries.len(), 3);
        let inspection = inspect_quick(&payload, &state, &target).unwrap();
        assert_eq!(inspection.package_version.as_deref(), Some("1.4.3.3"));
        assert_eq!(inspection.source, InstallationSource::ManagedPlus);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_plus_component_install_never_records_a_downgrade() {
        let base = root("plus-version-downgrade");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();

        // An online update already brought the suite to 1.4.3.3.
        component_payload(&payload, false);
        set_manifest_version(&payload, "1.4.3.3");
        install(&payload, &state, &target, false).unwrap();

        // The bundle payload installed locally is older: recording it would roll update
        // detection backwards, so the record keeps the newer suite version.
        component_payload(&payload, true);
        set_manifest_version(&payload, "1.4.2.4");
        install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();

        let record = read_record(&installation_dir(&state, &target)).unwrap();
        assert_eq!(record.package_version, "1.4.3.3");
        let inspection = inspect_quick(&payload, &state, &target).unwrap();
        assert_eq!(inspection.package_version.as_deref(), Some("1.4.3.3"));
        assert_eq!(inspection.source, InstallationSource::ManagedPlus);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_plus_component_install_records_a_version_it_cannot_compare() {
        let record = InstallRecord {
            schema_version: 2,
            installation_id: "test".to_string(),
            target: "target".to_string(),
            package_version: "legacy-tag".to_string(),
            installed_at: 0,
            source: InstallationSource::ManagedPlus,
            restore_baseline: RestoreBaseline::ExistingRecord,
            migrated_from_version: None,
            components: BTreeMap::new(),
            entries: vec![InstallRecordEntry {
                path: "cfg/test.cfg".to_string(),
                original_existed: true,
                original_sha256: None,
                original_backup: None,
                installed_sha256: String::new(),
                ownership: "plus".to_string(),
                restore_policy: "restore".to_string(),
            }],
        };

        // A version that fails to parse on either side must not block recording: the manifest
        // version wins instead of the record keeping a version nothing can compare.
        assert_eq!(
            plus_component_package_version(Some(&record), "1.4.3.3"),
            "1.4.3.3"
        );
        assert_eq!(
            plus_component_package_version(Some(&record), "also-not-a-version"),
            "also-not-a-version"
        );
    }

    #[test]
    fn restore_after_a_merge_keeps_the_third_party_component() {
        let base = root("restore-after-merge");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        third_party_payload(&payload);
        install_component(&payload, &state, &target, ComponentId::InventorySimulator).unwrap();
        // The bundle's dependency gate requires the upstream-managed DLL on target.
        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();
        component_payload(&payload, true);
        install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();
        let directory = installation_dir(&state, &target);
        assert!(is_suite_record(&read_record(&directory).unwrap()));

        let result = restore(&payload, &state, &target).unwrap();

        // Only the two suite files the merge added are removed; the component the suite never
        // installed stays on disk, and the record goes with the suite.
        assert_eq!(result.restored_files, 0);
        assert_eq!(result.removed_files, 2);
        assert!(!owned_target(&target, COMPONENT_PLUS[0]).exists());
        assert!(!owned_target(&target, COMPONENT_PLUS[1]).exists());
        assert_eq!(
            fs::read(owned_target(&target, THIRD_PARTY)).unwrap(),
            b"third-party-plugin"
        );
        assert!(read_record(&directory).is_none());
        fs::remove_dir_all(base).unwrap();
    }

    /// Legacy v1 `record.json`: a single `package_version`, no `components` key. Real records
    /// written before this change already carry `schema_version: 2`; the missing `components`
    /// field is what marks them for migration.
    fn write_v1_record(directory: &Path, version: &str) {
        fs::create_dir_all(directory).unwrap();
        let raw = format!(
            r#"{{
  "schema_version": 2,
  "installation_id": "v1-fixture",
  "target": "C:/cs2/game/csgo",
  "package_version": "{version}",
  "installed_at": 1700000000,
  "source": "managed_plus",
  "restore_baseline": "existing_record",
  "migrated_from_version": null,
  "entries": [
    {{
      "path": "cfg/test.cfg",
      "original_existed": true,
      "original_sha256": null,
      "original_backup": null,
      "installed_sha256": "abc123",
      "ownership": "plus",
      "restore_policy": "restore"
    }}
  ]
}}"#
        );
        fs::write(directory.join("record.json"), raw).unwrap();
    }

    /// Reads the record bytes straight from disk (never an in-memory object) for assertions.
    fn read_record_document(directory: &Path) -> serde_json::Value {
        let bytes = fs::read(directory.join("record.json")).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn assert_no_temp_files(directory: &Path) {
        for entry in fs::read_dir(directory).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                !name.contains(".tmp") && !name.starts_with('.'),
                "unexpected temp/partial file left behind: {name}"
            );
        }
    }

    #[test]
    fn record_v2_migrates_v1() {
        let base = root("record-v2-migrates-v1");

        // 1. no record at all -> never fabricate one.
        let none = base.join("none");
        fs::create_dir_all(&none).unwrap();
        assert!(migrate_record_v1_to_v2(&none).unwrap().is_none());
        assert!(!none.join("record.json").exists());
        assert!(read_record(&none).is_none());

        // 2. v1 only -> full per-component migration written atomically.
        let v1 = base.join("v1");
        write_v1_record(&v1, "1.4.3.3");
        let migrated = migrate_record_v1_to_v2(&v1).unwrap().unwrap();
        assert_eq!(migrated.schema_version, 2);
        assert_eq!(migrated.components.len(), 3);
        assert_eq!(
            migrated.components.get(&ComponentId::LaPlugins).unwrap().version,
            "1.4.3.3"
        );
        assert_eq!(
            migrated.components.get(&ComponentId::Upstream).unwrap().version,
            "unknown"
        );
        assert_eq!(
            migrated
                .components
                .get(&ComponentId::InventorySimulator)
                .unwrap()
                .version,
            "unknown"
        );
        let doc = read_record_document(&v1);
        assert_eq!(doc["schema_version"].as_u64(), Some(2));
        assert_eq!(
            doc["components"]["la_plugins"]["version"].as_str(),
            Some("1.4.3.3")
        );
        assert_eq!(
            doc["components"]["upstream"]["version"].as_str(),
            Some("unknown")
        );
        assert_eq!(
            doc["components"]["inventory_simulator"]["version"].as_str(),
            Some("unknown")
        );
        assert_eq!(doc["package_version"].as_str(), Some("1.4.3.3"));
        assert_eq!(doc["restore_baseline"].as_str(), Some("existing_record"));
        assert!(doc["migrated_from_version"].is_null());
        assert_eq!(doc["entries"].as_array().unwrap().len(), 1);
        assert_no_temp_files(&v1);

        // 3. already v2 (three components) -> byte-identical, no write.
        let v2 = base.join("v2");
        let v2_raw = r#"{
  "schema_version": 2,
  "installation_id": "v2-fixture",
  "target": "C:/cs2/game/csgo",
  "package_version": "1.4.3.3",
  "installed_at": 1700000001,
  "source": "managed_plus",
  "restore_baseline": "existing_record",
  "migrated_from_version": null,
  "components": {
    "upstream": {"version": "1.4.5", "source": "upstream_zip", "installed_at": 1, "ownership": "shared", "path": null},
    "la_plugins": {"version": "1.4.4.0", "source": "bundle", "installed_at": 2, "ownership": "plus", "path": "LocalArena-plugin.zip"},
    "inventory_simulator": {"version": "3.7.0", "source": "github", "installed_at": 3, "ownership": "third_party", "path": null}
  },
  "entries": []
}"#;
        fs::create_dir_all(&v2).unwrap();
        fs::write(v2.join("record.json"), v2_raw).unwrap();
        let before = fs::read(v2.join("record.json")).unwrap();
        let record = migrate_record_v1_to_v2(&v2).unwrap().unwrap();
        assert_eq!(record.components.len(), 3);
        assert_eq!(
            record.components.get(&ComponentId::LaPlugins).unwrap().version,
            "1.4.4.0"
        );
        assert_eq!(fs::read(v2.join("record.json")).unwrap(), before);

        // 4. partial components -> treated as already-v2, never clobbered or topped up.
        let partial = base.join("partial");
        let partial_raw = r#"{
  "schema_version": 2,
  "installation_id": "partial-fixture",
  "target": "C:/cs2/game/csgo",
  "package_version": "1.4.3.3",
  "installed_at": 1700000002,
  "source": "managed_plus",
  "restore_baseline": "existing_record",
  "migrated_from_version": null,
  "components": {
    "la_plugins": {"version": "1.4.5.0", "source": "bundle", "installed_at": 4, "ownership": "plus", "path": null}
  },
  "entries": []
}"#;
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("record.json"), partial_raw).unwrap();
        let before = fs::read(partial.join("record.json")).unwrap();
        let record = migrate_record_v1_to_v2(&partial).unwrap().unwrap();
        assert_eq!(record.components.len(), 1);
        assert_eq!(
            record.components.get(&ComponentId::LaPlugins).unwrap().version,
            "1.4.5.0"
        );
        assert_eq!(fs::read(partial.join("record.json")).unwrap(), before);

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn record_v2_migration_is_idempotent() {
        let base = root("record-v2-idempotent");
        write_v1_record(&base, "1.4.3.3");
        migrate_record_v1_to_v2(&base).unwrap().unwrap();
        let once = fs::read(base.join("record.json")).unwrap();
        // Re-migration (direct + via read_record) leaves every byte unchanged.
        migrate_record_v1_to_v2(&base).unwrap().unwrap();
        assert_eq!(fs::read(base.join("record.json")).unwrap(), once);
        let seen = read_record(&base).unwrap();
        assert_eq!(seen.components.len(), 3);
        assert_eq!(fs::read(base.join("record.json")).unwrap(), once);

        // stale_state + misleading_success_output: inject a sentinel into the on-disk record;
        // the next read must see it (reading disk, not a cached object) and must not rewrite it.
        let mut doc = read_record_document(&base);
        doc["components"]["la_plugins"]["source"] =
            serde_json::json!("sentinel-from-disk");
        let sentinel_bytes = serde_json::to_vec_pretty(&doc).unwrap();
        fs::write(base.join("record.json"), &sentinel_bytes).unwrap();
        let reread = read_record(&base).unwrap();
        assert_eq!(
            reread.components.get(&ComponentId::LaPlugins).unwrap().source,
            "sentinel-from-disk"
        );
        assert_eq!(fs::read(base.join("record.json")).unwrap(), sentinel_bytes);
        assert_no_temp_files(&base);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn record_v2_downgrade_guard() {
        let base = root("record-v2-downgrade-guard");
        write_v1_record(&base, "1.4.3.3");
        let migrated = migrate_record_v1_to_v2(&base).unwrap().unwrap();
        // Migration must not disturb package_version: it is the guard's comparison input.
        assert_eq!(migrated.package_version, "1.4.3.3");
        assert!(is_suite_record(&migrated));
        let on_disk = read_record_document(&base);
        assert_eq!(on_disk["package_version"].as_str(), Some("1.4.3.3"));
        // A lower bundle version is still refused; a higher one is still recorded.
        assert_eq!(
            plus_component_package_version(Some(&migrated), "1.4.2.4"),
            "1.4.3.3"
        );
        assert_eq!(
            plus_component_package_version(Some(&migrated), "1.4.4.0"),
            "1.4.4.0"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task1_happy() {
        let base = root("task1-happy");
        write_v1_record(&base, "1.4.3.3");
        // Real read surface: read_record triggers the migration + atomic write-back.
        let record = read_record(&base).expect("v1 record must still be readable");
        assert_eq!(record.schema_version, 2);
        assert_eq!(record.package_version, "1.4.3.3");
        assert_eq!(record.restore_baseline, RestoreBaseline::ExistingRecord);
        assert!(record.migrated_from_version.is_none());
        assert_eq!(record.entries.len(), 1); // InstallRecordEntry fields untouched
        assert_eq!(record.components.len(), 3);
        // misleading_success_output: assert against bytes actually on disk, not the returned object.
        let raw = fs::read(base.join("record.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(doc["schema_version"].as_u64(), Some(2));
        assert_eq!(
            doc["components"]["la_plugins"]["version"].as_str(),
            Some("1.4.3.3")
        );
        assert_eq!(
            doc["components"]["upstream"]["version"].as_str(),
            Some("unknown")
        );
        assert_eq!(
            doc["components"]["inventory_simulator"]["version"].as_str(),
            Some("unknown")
        );
        assert_eq!(
            doc["components"]["la_plugins"]["installed_at"].as_u64(),
            Some(1700000000)
        );
        assert!(doc["components"]["upstream"]["path"].is_null());
        assert_eq!(doc["package_version"].as_str(), Some("1.4.3.3"));
        assert_no_temp_files(&base);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task1_failure() {
        let base = root("task1-failure");
        fs::create_dir_all(&base).unwrap();
        let truncated: &[u8] = b"{\"schema_version\": 2, \"package_version\": \"1.4.3.3\", \"entr";
        let path = base.join("record.json");
        fs::write(&path, truncated).unwrap();

        let error = migrate_record_v1_to_v2(&base).unwrap_err();
        assert_eq!(error.code, "E1401");
        assert!(
            error.detail.contains("Corrupt install record"),
            "{}",
            error.detail
        );
        // mid-operation interrupts: no partial write, the truncated bytes are untouched, and no
        // temp file was left behind.
        assert_eq!(fs::read(&path).unwrap().as_slice(), truncated);
        let names: Vec<String> = fs::read_dir(&base)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["record.json".to_string()]);
        // The production read path stays lenient (returns None) exactly as before this change.
        assert!(read_record(&base).is_none());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task1_manual_qa_dump() {
        let base = root("task1-manual-qa-dump");
        write_v1_record(&base, "1.4.3.3");
        migrate_record_v1_to_v2(&base).unwrap().unwrap();
        // Dump the exact bytes written to disk, not an in-memory serialization.
        let bytes = fs::read(base.join("record.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(doc["schema_version"].as_u64(), Some(2));
        assert_eq!(
            doc["components"]["la_plugins"]["version"].as_str(),
            Some("1.4.3.3")
        );
        assert_eq!(
            doc["components"]["upstream"]["version"].as_str(),
            Some("unknown")
        );
        if let Ok(dump) = std::env::var("QA_DUMP") {
            fs::write(&dump, &bytes).unwrap();
            println!("task1_manual_qa_dump: wrote {} bytes to {}", bytes.len(), dump);
        } else {
            println!("task1_manual_qa_dump: QA_DUMP unset, skipped dump");
        }
        fs::remove_dir_all(base).unwrap();
    }

    /// The LA half of a component payload rewritten with new bytes and a new manifest version.
    fn versioned_component_payload(
        payload: &Path,
        version: &str,
        plus_bytes: (&[u8], &[u8]),
    ) -> PayloadManifest {
        let mut manifest = PayloadManifest {
            schema_version: 1,
            package_version: version.to_string(),
            entries: Vec::new(),
        };
        write_owned_entry(
            payload,
            &mut manifest,
            COMPONENT_SHARED[0],
            b"shared-bot-ai",
            "shared",
        );
        write_owned_entry(
            payload,
            &mut manifest,
            COMPONENT_SHARED[1],
            b"shared-bothider-api",
            "shared",
        );
        write_owned_entry(payload, &mut manifest, COMPONENT_PLUS[0], plus_bytes.0, "plus");
        write_owned_entry(payload, &mut manifest, COMPONENT_PLUS[1], plus_bytes.1, "plus");
        write_json_atomic(&payload.join(MANIFEST_FILE), &manifest).unwrap();
        manifest
    }

    fn collect_files(root: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect_files(&path, out);
            } else {
                out.push(path);
            }
        }
    }

    /// Every target file with its sha256 — the "target unchanged" witness for aborted installs.
    fn snapshot_target(root: &Path) -> BTreeMap<String, String> {
        let mut files = Vec::new();
        collect_files(root, &mut files);
        files
            .into_iter()
            .map(|file| {
                let relative = file
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                (relative, sha256(&file).unwrap())
            })
            .collect()
    }

    #[test]
    fn conflict_aborts_before_any_write() {
        // (a) An existing file nobody registered already occupies a payload path.
        let base = root("component-conflict-unmanaged");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        third_party_payload(&payload);
        let foreign = owned_target(&target, THIRD_PARTY);
        fs::create_dir_all(foreign.parent().unwrap()).unwrap();
        fs::write(&foreign, b"another-plugin-build").unwrap();
        let before = snapshot_target(&target);

        let error = install_component(&payload, &state, &target, ComponentId::InventorySimulator)
            .unwrap_err();

        assert_eq!(error.code, "E1403");
        assert_eq!(error.category, "installation");
        let listed: Vec<&str> = error
            .detail
            .lines()
            .filter(|line| line.starts_with("path: "))
            .collect();
        assert_eq!(listed, vec![format!("path: {THIRD_PARTY}")]);
        // mid-operation interrupts: a refused install writes nothing at all.
        assert_eq!(snapshot_target(&target), before);
        assert!(!installation_dir(&state, &target).exists());
        assert_no_temp_files(&target);
        fs::remove_dir_all(base).unwrap();

        // (b) A path recorded under ANOTHER component conflicts even with the dependency present.
        let base = root("component-conflict-other-component");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();
        third_party_payload(&payload);
        install_component(&payload, &state, &target, ComponentId::InventorySimulator).unwrap();
        let directory = installation_dir(&state, &target);
        let third_party_sha = sha256(&owned_target(&target, THIRD_PARTY)).unwrap();

        versioned_component_payload(
            &payload,
            "1.4.3.3",
            (b"plus-match-coordinator", b"plus-team-lineup-injector"),
        );
        let mut manifest = read_manifest_document(&payload).unwrap();
        write_owned_entry(
            &payload,
            &mut manifest,
            THIRD_PARTY,
            b"la-owned-collision",
            "plus",
        );
        write_json_atomic(&payload.join(MANIFEST_FILE), &manifest).unwrap();
        let record_before = fs::read(directory.join("record.json")).unwrap();
        let before = snapshot_target(&target);

        let error =
            install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap_err();

        assert_eq!(error.code, "E1403");
        let listed: Vec<&str> = error
            .detail
            .lines()
            .filter(|line| line.starts_with("path: "))
            .collect();
        assert_eq!(listed, vec![format!("path: {THIRD_PARTY}")]);
        assert_eq!(snapshot_target(&target), before);
        assert_eq!(fs::read(directory.join("record.json")).unwrap(), record_before);
        assert_eq!(sha256(&owned_target(&target, THIRD_PARTY)).unwrap(), third_party_sha);
        assert!(!owned_target(&target, COMPONENT_PLUS[0]).exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn update_la_plugins_v1_to_v2_succeeds() {
        let base = root("component-la-update");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();

        versioned_component_payload(&payload, "1.4.3.3", (b"pmc-v1", b"tli-v1"));
        let first = install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();
        assert_eq!(first.installed_files, 2);
        assert_eq!(
            fs::read(owned_target(&target, COMPONENT_PLUS[0])).unwrap(),
            b"pmc-v1"
        );

        // Same component, new bytes and a new version: a normal update, never a conflict.
        versioned_component_payload(&payload, "1.4.4.0", (b"pmc-v2-longer", b"tli-v2-longer"));
        let second = install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();

        assert_eq!(second.installed_files, 2);
        assert_eq!(
            fs::read(owned_target(&target, COMPONENT_PLUS[0])).unwrap(),
            b"pmc-v2-longer"
        );
        assert_eq!(
            fs::read(owned_target(&target, COMPONENT_PLUS[1])).unwrap(),
            b"tli-v2-longer"
        );
        let directory = installation_dir(&state, &target);
        let record = read_record(&directory).unwrap();
        assert_eq!(record.package_version, "1.4.4.0");
        assert_eq!(record.components.len(), 1);
        let la = record.components.get(&ComponentId::LaPlugins).unwrap();
        assert_eq!(la.version, "1.4.4.0");
        assert_eq!(la.ownership, "plus");
        // misleading_success_output: the bytes on disk must say the same.
        let doc = read_record_document(&directory);
        assert_eq!(
            doc["components"]["la_plugins"]["version"].as_str(),
            Some("1.4.4.0")
        );
        assert!(!directory.join("journal.json").exists());
        assert_no_temp_files(&directory);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn missing_dependency_reports_E1404() {
        let base = root("component-missing-dependency");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        component_payload(&payload, true);

        let error =
            install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap_err();

        assert_eq!(error.code, "E1404");
        assert_eq!(error.category, "installation");
        assert!(error.detail.contains("BotHiderApi"), "{}", error.detail);
        assert!(fs::read_dir(&target).unwrap().next().is_none());
        assert!(!installation_dir(&state, &target).exists());

        // Once the upstream component supplies the DLL, the same install succeeds.
        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();
        let result = install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();
        assert_eq!(result.installed_files, 2);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn each_component_install_records_its_own_version() {
        let base = root("component-versions");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let directory = installation_dir(&state, &target);

        // upstream: only the upstream row, holding the upstream payload's version.
        component_payload(&payload, false);
        install_component(&payload, &state, &target, ComponentId::Upstream).unwrap();
        let record = read_record(&directory).unwrap();
        assert_eq!(record.components.len(), 1);
        assert_eq!(
            record.components.get(&ComponentId::Upstream).unwrap().version,
            "1.4.3.3"
        );
        assert!(!record.components.contains_key(&ComponentId::LaPlugins));

        // la_plugins: its own row is added, the upstream row is carried over untouched.
        versioned_component_payload(&payload, "1.4.4.0", (b"pmc", b"tli"));
        install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();
        let record = read_record(&directory).unwrap();
        assert_eq!(record.components.len(), 2);
        assert_eq!(
            record.components.get(&ComponentId::Upstream).unwrap().version,
            "1.4.3.3"
        );
        assert_eq!(
            record.components.get(&ComponentId::LaPlugins).unwrap().version,
            "1.4.4.0"
        );

        // inventory_simulator: the third row carries its own tag, never the suite version.
        third_party_payload(&payload);
        install_component(&payload, &state, &target, ComponentId::InventorySimulator).unwrap();
        let record = read_record(&directory).unwrap();
        assert_eq!(record.components.len(), 3);
        let simulator = record
            .components
            .get(&ComponentId::InventorySimulator)
            .unwrap();
        assert_eq!(simulator.version, "3.7.0");
        assert_eq!(simulator.ownership, "third-party");
        assert_eq!(record.package_version, "1.4.4.0");
        let doc = read_record_document(&directory);
        assert_eq!(doc["schema_version"].as_u64(), Some(2));
        assert_eq!(doc["components"]["upstream"]["version"].as_str(), Some("1.4.3.3"));
        assert_eq!(
            doc["components"]["la_plugins"]["version"].as_str(),
            Some("1.4.4.0")
        );
        assert_eq!(
            doc["components"]["inventory_simulator"]["version"].as_str(),
            Some("3.7.0")
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task3_happy() {
        let base = root("task3-happy");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let directory = installation_dir(&state, &target);

        component_payload(&payload, false);
        let upstream = install_component(&payload, &state, &target, ComponentId::Upstream)
            .expect("upstream install");
        versioned_component_payload(&payload, "1.4.4.0", (b"pmc", b"tli"));
        let la = install_component(&payload, &state, &target, ComponentId::LaPlugins)
            .expect("la_plugins install");
        third_party_payload(&payload);
        let simulator =
            install_component(&payload, &state, &target, ComponentId::InventorySimulator)
                .expect("inventory_simulator install");

        let record = read_record(&directory).unwrap();
        println!(
            "task3_happy: files upstream={} la={} simulator={} components={:?} schema={}",
            upstream.installed_files,
            la.installed_files,
            simulator.installed_files,
            record
                .components
                .iter()
                .map(|(id, component)| (format!("{id:?}"), component.version.clone()))
                .collect::<Vec<_>>(),
            record.schema_version
        );
        assert_eq!(record.schema_version, 2);
        assert_eq!(record.components.len(), 3);
        assert_eq!(
            record.components.get(&ComponentId::Upstream).unwrap().version,
            "1.4.3.3"
        );
        assert_eq!(
            record.components.get(&ComponentId::LaPlugins).unwrap().version,
            "1.4.4.0"
        );
        assert_eq!(
            record
                .components
                .get(&ComponentId::InventorySimulator)
                .unwrap()
                .version,
            "3.7.0"
        );
        assert!(!directory.join("journal.json").exists());
        assert_no_temp_files(&directory);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task3_failure() {
        let base = root("task3-failure");

        // Conflict branch: an unregistered file occupies a payload path.
        let payload = base.join("conflict/payload");
        let target = base.join("conflict/target");
        let state = base.join("conflict/state");
        fs::create_dir_all(&target).unwrap();
        third_party_payload(&payload);
        let foreign = owned_target(&target, THIRD_PARTY);
        fs::create_dir_all(foreign.parent().unwrap()).unwrap();
        fs::write(&foreign, b"unmanaged-bytes").unwrap();
        let before = snapshot_target(&target);
        let conflict =
            install_component(&payload, &state, &target, ComponentId::InventorySimulator)
                .expect_err("a conflicting path must abort the install");
        assert_eq!(conflict.code, "E1403");
        assert_eq!(snapshot_target(&target), before);
        assert!(!installation_dir(&state, &target).exists());
        println!(
            "task3_failure: conflict={} lines={} detail=\n{}",
            conflict.code,
            conflict.detail.lines().count(),
            conflict.detail
        );

        // Dependency branch: the LA bundle without the upstream BotHiderApi.
        let payload = base.join("dependency/payload");
        let target = base.join("dependency/target");
        let state = base.join("dependency/state");
        fs::create_dir_all(&target).unwrap();
        component_payload(&payload, true);
        let missing = install_component(&payload, &state, &target, ComponentId::LaPlugins)
            .expect_err("a missing dependency must abort the install");
        assert_eq!(missing.code, "E1404");
        assert!(missing.detail.contains("BotHiderApi"), "{}", missing.detail);
        assert!(fs::read_dir(&target).unwrap().next().is_none());
        assert!(!installation_dir(&state, &target).exists());
        println!("task3_failure: dependency={}", missing.code);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task3_manual_qa_dump() {
        let base = root("task3-manual-qa-dump");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();

        component_payload(&payload, false);
        install_component(&payload, &state, &target, ComponentId::Upstream).unwrap();
        versioned_component_payload(&payload, "1.4.4.0", (b"pmc", b"tli"));
        install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();
        third_party_payload(&payload);
        install_component(&payload, &state, &target, ComponentId::InventorySimulator).unwrap();

        // Dump the exact bytes the transaction wrote to disk, never an in-memory object.
        let bytes = fs::read(installation_dir(&state, &target).join("record.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(doc["schema_version"].as_u64(), Some(2));
        assert_eq!(doc["components"].as_object().unwrap().len(), 3);
        assert_eq!(
            doc["components"]["la_plugins"]["version"].as_str(),
            Some("1.4.4.0")
        );
        if let Ok(dump) = std::env::var("QA_DUMP") {
            fs::write(&dump, &bytes).unwrap();
            println!(
                "task3_manual_qa_dump: wrote {} bytes to {}",
                bytes.len(),
                dump
            );
        } else {
            println!("task3_manual_qa_dump: QA_DUMP unset, skipped dump");
        }
        fs::remove_dir_all(base).unwrap();
    }

    /// One fixture payload per component, laid out the way each component's real payload is:
    /// upstream carries only `shared` entries, the LA bundle only `plus` entries, and
    /// InventorySimulator only its `third-party` entry.
    fn component_payloads(base: &Path) -> BTreeMap<ComponentId, PathBuf> {
        let upstream = base.join("upstream-payload");
        let la = base.join("la-payload");
        let simulator = base.join("simulator-payload");
        component_payload(&upstream, false);
        versioned_component_payload(&la, "1.4.4.0", (b"pmc", b"tli"));
        third_party_payload(&simulator);
        BTreeMap::from([
            (ComponentId::Upstream, upstream),
            (ComponentId::LaPlugins, la),
            (ComponentId::InventorySimulator, simulator),
        ])
    }

    /// The component rows of `record.json` as written on disk: id -> version.
    fn component_versions(directory: &Path) -> BTreeMap<String, String> {
        read_record_document(directory)["components"]
            .as_object()
            .map(|components| {
                components
                    .iter()
                    .map(|(id, row)| {
                        (
                            id.clone(),
                            row["version"].as_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn install_all_stops_on_first_component_failure() {
        // The middle component's payload has no manifest, so the third component must never be
        // attempted: no files, no record row.
        let base = root("task5-stop-on-first-failure");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        let broken = base.join("broken-la");
        fs::create_dir_all(&broken).unwrap();

        let error = run_component_steps(&INSTALL_ORDER, |component| {
            let payload = match component {
                ComponentId::LaPlugins => broken.clone(),
                other => payloads[&other].clone(),
            };
            install_component(&payload, &state, &target, component)
        })
        .expect_err("the run must abort at the broken component");

        assert_eq!(error.code, "E1301", "{}", error.detail);
        assert!(owned_target(&target, COMPONENT_SHARED[0]).is_file());
        assert!(!owned_target(&target, COMPONENT_PLUS[0]).exists());
        assert!(!owned_target(&target, THIRD_PARTY).exists());
        let versions = component_versions(&installation_dir(&state, &target));
        assert_eq!(versions.len(), 1);
        assert_eq!(versions.get("upstream").map(String::as_str), Some("1.4.3.3"));
        assert!(!versions.contains_key("inventory_simulator"));
        assert_no_temp_files(&target);
        println!(
            "install_all_stops_on_first_component_failure: code={} versions={versions:?}",
            error.code
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task5_happy() {
        let base = root("task5-happy");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);

        let results = run_component_steps(&INSTALL_ORDER, |component| {
            install_component(&payloads[&component], &state, &target, component)
        })
        .expect("every component installs through its own transaction");

        assert_eq!(results.len(), 3);
        let directory = installation_dir(&state, &target);
        let versions = component_versions(&directory);
        assert_eq!(versions.len(), 3);
        assert_eq!(versions.get("upstream").map(String::as_str), Some("1.4.3.3"));
        assert_eq!(versions.get("la_plugins").map(String::as_str), Some("1.4.4.0"));
        assert_eq!(
            versions.get("inventory_simulator").map(String::as_str),
            Some("3.7.0")
        );
        assert!(owned_target(&target, COMPONENT_SHARED[0]).is_file());
        assert!(owned_target(&target, COMPONENT_PLUS[1]).is_file());
        assert!(owned_target(&target, THIRD_PARTY).is_file());
        assert!(!directory.join("journal.json").exists());
        assert_no_temp_files(&directory);
        println!(
            "task5_happy: versions={versions:?} files={:?}",
            results.iter().map(|result| result.installed_files).collect::<Vec<_>>()
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task5_failure() {
        let base = root("task5-failure");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        install_component(
            &payloads[&ComponentId::Upstream],
            &state,
            &target,
            ComponentId::Upstream,
        )
        .unwrap();
        let directory = installation_dir(&state, &target);

        // A manifest entry whose staged file is gone must fail before any write.
        let missing = base.join("missing-file");
        versioned_component_payload(&missing, "1.4.5.0", (b"pmc", b"tli"));
        fs::remove_file(missing.join(safe_relative(COMPONENT_PLUS[1]).unwrap())).unwrap();
        let before = snapshot_target(&target);
        let error =
            install_component(&missing, &state, &target, ComponentId::LaPlugins).unwrap_err();
        assert_eq!(error.code, "E1301", "{}", error.detail);
        assert!(
            error.detail.contains("Payload verification failed"),
            "{}",
            error.detail
        );
        assert_eq!(snapshot_target(&target), before);
        assert!(!component_versions(&directory).contains_key("la_plugins"));

        // Uninstalling a component nobody installed is a successful no-op, not an error.
        let noop = uninstall_component(&state, &target, ComponentId::InventorySimulator).unwrap();
        assert_eq!(
            noop.restored_files + noop.removed_files + noop.preserved_files,
            0
        );
        assert!(owned_target(&target, COMPONENT_SHARED[0]).is_file());
        println!(
            "task5_failure: install={}({}) uninstall_noop={}",
            error.code,
            error.detail.lines().next().unwrap_or_default(),
            noop.result_kind
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn uninstall_component_removes_only_its_own_rows() {
        let base = root("uninstall-one-of-three");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        run_component_steps(&INSTALL_ORDER, |component| {
            install_component(&payloads[&component], &state, &target, component)
        })
        .unwrap();
        let directory = installation_dir(&state, &target);

        let removed = uninstall_component(&state, &target, ComponentId::InventorySimulator).unwrap();
        assert!(removed.removed_files >= 1);
        assert!(!owned_target(&target, THIRD_PARTY).exists());
        assert!(owned_target(&target, COMPONENT_PLUS[0]).is_file());
        assert!(owned_target(&target, COMPONENT_SHARED[0]).is_file());
        let versions = component_versions(&directory);
        assert_eq!(versions.len(), 2);
        assert!(!versions.contains_key("inventory_simulator"));

        // Removing the remaining two leaves no record and no managed file behind.
        let rest = run_component_steps(
            &[ComponentId::LaPlugins, ComponentId::Upstream],
            |component| uninstall_component(&state, &target, component),
        )
        .unwrap();
        assert_eq!(rest.len(), 2);
        assert!(!owned_target(&target, COMPONENT_PLUS[0]).exists());
        assert!(!owned_target(&target, COMPONENT_SHARED[0]).exists());
        assert!(!directory.join("record.json").exists());
        assert!(!directory.join("journal.json").exists());
        assert_no_temp_files(&target);
        println!(
            "uninstall_component_removes_only_its_own_rows: removed={} restored={} remaining={}",
            removed.removed_files,
            rest.iter().map(|result| result.removed_files).sum::<usize>(),
            rest.len()
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task5_manual_qa_dump() {
        let base = root("task5-manual-qa-dump");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        run_component_steps(&INSTALL_ORDER, |component| {
            install_component(&payloads[&component], &state, &target, component)
        })
        .unwrap();

        // Dump the exact bytes the transactions wrote, never an in-memory object.
        let bytes = fs::read(installation_dir(&state, &target).join("record.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(doc["components"].as_object().unwrap().len(), 3);
        assert_eq!(
            doc["components"]["upstream"]["version"].as_str(),
            Some("1.4.3.3")
        );
        assert_eq!(
            doc["components"]["la_plugins"]["version"].as_str(),
            Some("1.4.4.0")
        );
        assert_eq!(
            doc["components"]["inventory_simulator"]["version"].as_str(),
            Some("3.7.0")
        );
        // The suite `package_version` mirrors the LA row only; it never stands in for another
        // component's version.
        assert_eq!(doc["package_version"].as_str(), Some("1.4.4.0"));
        assert_ne!(
            doc["package_version"].as_str(),
            doc["components"]["upstream"]["version"].as_str()
        );
        assert_ne!(
            doc["package_version"].as_str(),
            doc["components"]["inventory_simulator"]["version"].as_str()
        );
        if let Ok(dump) = std::env::var("QA_DUMP") {
            fs::write(&dump, &bytes).unwrap();
            println!(
                "task5_manual_qa_dump: wrote {} bytes to {}",
                bytes.len(),
                dump
            );
        } else {
            println!("task5_manual_qa_dump: QA_DUMP unset, skipped dump");
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn installed_plugin_version_reads_component_record() {
        let base = root("task6-component-record-version");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        assert_eq!(
            component_version(&state, &target, ComponentId::LaPlugins),
            None,
            "no record means no version"
        );
        run_component_steps(&INSTALL_ORDER, |component| {
            install_component(&payloads[&component], &state, &target, component)
        })
        .unwrap();

        let la = component_version(&state, &target, ComponentId::LaPlugins);
        assert_eq!(la.as_deref(), Some("1.4.4.0"));
        assert_ne!(
            la.as_deref(),
            Some("3.7.0"),
            "the InventorySimulator release tag must not leak into the LA version"
        );
        assert_eq!(
            component_version(&state, &target, ComponentId::Upstream).as_deref(),
            Some("1.4.3.3")
        );
        assert_eq!(
            component_version(&state, &target, ComponentId::InventorySimulator).as_deref(),
            Some("3.7.0")
        );
        println!("installed_plugin_version_reads_component_record: la={la:?}");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task6_happy() {
        // A legacy record without `components` migrates on read, so the LA version still resolves
        // from the record alone — no payload inspection.
        let base = root("task6-happy");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let directory = installation_dir(&state, &target);
        write_v1_record(&directory, "1.4.3.3");

        assert_eq!(
            component_version(&state, &target, ComponentId::LaPlugins).as_deref(),
            Some("1.4.3.3")
        );
        // The migration sentinel reads as "not installed", never as a version.
        assert_eq!(component_version(&state, &target, ComponentId::Upstream), None);
        assert_eq!(
            component_version(&state, &target, ComponentId::InventorySimulator),
            None
        );
        println!(
            "task6_happy: la={:?} migrated_record_keys={:?}",
            component_version(&state, &target, ComponentId::LaPlugins),
            read_record_document(&directory)["components"]
                .as_object()
                .map(|components| components.keys().cloned().collect::<Vec<_>>())
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task6_failure() {
        let base = root("task6-failure");

        // (a) No record at all.
        let target = base.join("absent/target");
        let state = base.join("absent/state");
        fs::create_dir_all(&target).unwrap();
        assert_eq!(component_version(&state, &target, ComponentId::LaPlugins), None);

        // (b) A corrupt record reports no version instead of panicking or guessing.
        let target = base.join("corrupt/target");
        let state = base.join("corrupt/state");
        fs::create_dir_all(&target).unwrap();
        let directory = installation_dir(&state, &target);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("record.json"), b"{ not a record").unwrap();
        assert_eq!(component_version(&state, &target, ComponentId::LaPlugins), None);

        // (c) A third-party-only record: the IS tag never becomes the LA version.
        let target = base.join("third-party/target");
        let state = base.join("third-party/state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        install_component(
            &payloads[&ComponentId::InventorySimulator],
            &state,
            &target,
            ComponentId::InventorySimulator,
        )
        .unwrap();
        assert_eq!(component_version(&state, &target, ComponentId::LaPlugins), None);
        assert_eq!(
            component_version(&state, &target, ComponentId::InventorySimulator).as_deref(),
            Some("3.7.0")
        );
        println!(
            "task6_failure: absent={:?} corrupt={:?} third_party_la={:?}",
            component_version(&state, &target, ComponentId::Upstream),
            None::<String>,
            component_version(&state, &target, ComponentId::LaPlugins)
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn task6_manual_qa_dump() {
        let base = root("task6-manual-qa-dump");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        run_component_steps(&INSTALL_ORDER, |component| {
            install_component(&payloads[&component], &state, &target, component)
        })
        .unwrap();

        let directory = installation_dir(&state, &target);
        let bytes = fs::read(directory.join("record.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let la = component_version(&state, &target, ComponentId::LaPlugins);
        assert_eq!(la.as_deref(), doc["components"]["la_plugins"]["version"].as_str());
        assert_eq!(doc["components"]["inventory_simulator"]["version"].as_str(), Some("3.7.0"));
        if let Ok(dump) = std::env::var("QA_DUMP") {
            fs::write(&dump, &bytes).unwrap();
            println!(
                "task6_manual_qa_dump: wrote {} bytes to {} (la_plugins={:?})",
                bytes.len(),
                dump,
                la
            );
        } else {
            println!("task6_manual_qa_dump: QA_DUMP unset, skipped dump");
        }
        fs::remove_dir_all(base).unwrap();
    }

    // ---------------------------------------------------------------------------------------
    // Todo 13: the cross-component regression layer. The earlier waves tested their own units;
    // these tests pin the invariants that only hold when all three components share one record
    // and one target. Every assertion reads the filesystem or the record bytes on disk.
    // ---------------------------------------------------------------------------------------

    #[test]
    fn record_v2_migrates_legacy_and_is_idempotent() {
        let base = root("task13-record-v2-idempotent");
        let directory = base.join("installation");
        write_v1_record(&directory, "1.4.3.3");

        // First read through the production path triggers the migration and writes it back.
        let migrated = read_record(&directory).expect("a legacy record must stay readable");
        assert_eq!(migrated.schema_version, 2);
        assert_eq!(migrated.components.len(), 3);
        let doc = read_record_document(&directory);
        assert_eq!(
            doc["components"]["la_plugins"]["version"].as_str(),
            Some("1.4.3.3"),
            "the one known suite version maps to la_plugins"
        );
        assert_eq!(
            doc["components"]["upstream"]["version"].as_str(),
            Some(UNKNOWN_VERSION)
        );
        assert_eq!(
            doc["components"]["upstream"]["source"].as_str(),
            Some("unknown")
        );
        assert_eq!(
            doc["components"]["inventory_simulator"]["version"].as_str(),
            Some(UNKNOWN_VERSION)
        );
        assert_eq!(
            doc["components"]["inventory_simulator"]["source"].as_str(),
            Some("unknown")
        );
        assert_eq!(doc["package_version"].as_str(), Some("1.4.3.3"));

        // Mark the bytes on disk: any rewrite of a migrated record would drop this sentinel.
        let mut sentinel = read_record_document(&directory);
        sentinel["components"]["upstream"]["source"] = serde_json::json!("sentinel-from-disk");
        let sentinel_bytes = serde_json::to_vec_pretty(&sentinel).unwrap();
        fs::write(directory.join("record.json"), &sentinel_bytes).unwrap();

        for round in 0..2 {
            let reread = read_record(&directory).expect("a migrated record must stay readable");
            assert_eq!(reread.components.len(), 3, "round {round}");
            assert_eq!(
                reread
                    .components
                    .get(&ComponentId::Upstream)
                    .unwrap()
                    .source,
                "sentinel-from-disk",
                "round {round}: the read must come from disk, not a cached object"
            );
            assert_eq!(
                fs::read(directory.join("record.json")).unwrap(),
                sentinel_bytes,
                "round {round}: a migrated record must never be rewritten"
            );
        }
        assert_no_temp_files(&directory);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn three_components_coexist_in_one_record() {
        let base = root("task13-three-components-one-record");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        let directory = installation_dir(&state, &target);

        // Three separate transactions, one per component, in dependency order.
        for component in INSTALL_ORDER {
            install_component(&payloads[&component], &state, &target, component).unwrap();
        }

        let doc = read_record_document(&directory);
        let components = doc["components"].as_object().expect("components map");
        assert_eq!(components.len(), 3, "{components:?}");
        assert_eq!(components["upstream"]["version"].as_str(), Some("1.4.3.3"));
        assert_eq!(
            components["la_plugins"]["version"].as_str(),
            Some("1.4.4.0")
        );
        assert_eq!(
            components["inventory_simulator"]["version"].as_str(),
            Some("3.7.0")
        );
        // The suite-level version speaks for the LA bundle only: it is never written over a
        // component's own row, and no component's tag becomes the suite version.
        assert_eq!(doc["package_version"].as_str(), Some("1.4.4.0"));
        assert_ne!(
            components["upstream"]["version"].as_str(),
            doc["package_version"].as_str()
        );
        assert_ne!(
            components["inventory_simulator"]["version"].as_str(),
            doc["package_version"].as_str()
        );

        let mut ownership: Vec<&str> = doc["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["ownership"].as_str().unwrap())
            .collect();
        ownership.sort_unstable();
        assert_eq!(
            ownership,
            vec!["plus", "plus", "shared", "shared", "third-party"]
        );

        // One record file per target: components merge, they never fork a record.
        let mut names: Vec<String> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["record.json".to_string()]);
        assert_no_temp_files(&directory);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn conflict_blocks_write_and_lists_every_path() {
        let base = root("task13-conflict-every-path");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();

        // The bundle's dependency gate runs before the conflict scan, so the upstream DLL has to
        // be present; it is not part of the plus payload and can never be one of the conflicts.
        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();

        versioned_component_payload(&payload, "1.4.4.0", (b"pmc", b"tli"));
        let mut manifest = read_manifest_document(&payload).unwrap();
        let non_conflicting = "addons/counterstrikesharp/plugins/PlusMatchCoordinator/extra.json";
        write_owned_entry(&payload, &mut manifest, non_conflicting, b"extra", "plus");
        write_json_atomic(&payload.join(MANIFEST_FILE), &manifest).unwrap();

        // Two unmanaged files already occupy payload paths with different bytes.
        for raw in COMPONENT_PLUS {
            let destination = owned_target(&target, raw);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(&destination, b"foreign-build").unwrap();
        }
        let before = snapshot_target(&target);

        let error = install_component(&payload, &state, &target, ComponentId::LaPlugins)
            .expect_err("a payload colliding with unmanaged files must abort");

        assert_eq!(error.code, "E1403");
        assert_eq!(error.category, "installation");
        let listed: Vec<&str> = error
            .detail
            .lines()
            .filter(|line| line.starts_with("path: "))
            .collect();
        assert_eq!(
            listed.len(),
            2,
            "one `path:` line per conflicting path, got: {}",
            error.detail
        );
        assert_eq!(
            listed,
            vec![
                format!("path: {}", COMPONENT_PLUS[0]),
                format!("path: {}", COMPONENT_PLUS[1]),
            ]
        );
        // mid-operation interrupts: a refused install writes nothing at all.
        assert_eq!(snapshot_target(&target), before);
        assert!(!owned_target(&target, non_conflicting).exists());
        assert!(!installation_dir(&state, &target).exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn component_update_is_not_a_conflict() {
        let base = root("task13-component-update");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let dll = owned_target(&target, COMPONENT_SHARED[1]);
        fs::create_dir_all(dll.parent().unwrap()).unwrap();
        fs::write(&dll, b"shared-bothider-api").unwrap();

        versioned_component_payload(&payload, "1.4.4.0", (b"pmc-v1", b"tli-v1"));
        install_component(&payload, &state, &target, ComponentId::LaPlugins).unwrap();
        let first_sha = sha256(&owned_target(&target, COMPONENT_PLUS[0])).unwrap();

        // Same component, new bytes: its own registered paths are an update, never a conflict.
        versioned_component_payload(&payload, "1.4.5.0", (b"pmc-v2", b"tli-v2"));
        let updated = install_component(&payload, &state, &target, ComponentId::LaPlugins)
            .expect("a component update must not be reported as E1403");

        assert_eq!(updated.installed_files, 2);
        assert_eq!(
            fs::read(owned_target(&target, COMPONENT_PLUS[0])).unwrap(),
            b"pmc-v2"
        );
        assert_eq!(
            fs::read(owned_target(&target, COMPONENT_PLUS[1])).unwrap(),
            b"tli-v2"
        );
        assert_ne!(
            sha256(&owned_target(&target, COMPONENT_PLUS[0])).unwrap(),
            first_sha
        );
        let doc = read_record_document(&installation_dir(&state, &target));
        assert_eq!(
            doc["components"]["la_plugins"]["version"].as_str(),
            Some("1.4.5.0")
        );
        assert_eq!(doc["package_version"].as_str(), Some("1.4.5.0"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    #[allow(non_snake_case)]
    fn missing_upstream_dependency_reports_E1404() {
        let base = root("task13-missing-upstream-dependency");
        let payload = base.join("payload");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        versioned_component_payload(&payload, "1.4.4.0", (b"pmc", b"tli"));

        let error = install_component(&payload, &state, &target, ComponentId::LaPlugins)
            .expect_err("the LA bundle must refuse to install without its upstream dependency");

        assert_eq!(error.code, "E1404");
        assert_eq!(error.category, "installation");
        assert!(error.detail.contains("BotHiderApi"), "{}", error.detail);
        // Zero writes: no payload file, no state directory, no record row to read back.
        assert!(fs::read_dir(&target).unwrap().next().is_none());
        assert!(!installation_dir(&state, &target).exists());
        assert_eq!(
            component_version(&state, &target, ComponentId::LaPlugins),
            None
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn uninstall_removes_only_its_own_rows() {
        let base = root("task13-uninstall-one-of-three");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        for component in INSTALL_ORDER {
            install_component(&payloads[&component], &state, &target, component).unwrap();
        }
        let directory = installation_dir(&state, &target);

        let removed = uninstall_component(&state, &target, ComponentId::Upstream).unwrap();

        assert_eq!(removed.removed_files, 2);
        assert!(!owned_target(&target, COMPONENT_SHARED[0]).exists());
        assert!(!owned_target(&target, COMPONENT_SHARED[1]).exists());
        // The other two components keep their files and their record rows.
        assert!(owned_target(&target, COMPONENT_PLUS[0]).is_file());
        assert!(owned_target(&target, COMPONENT_PLUS[1]).is_file());
        assert!(owned_target(&target, THIRD_PARTY).is_file());
        let doc = read_record_document(&directory);
        let components = doc["components"].as_object().unwrap();
        assert_eq!(components.len(), 2, "{components:?}");
        assert!(!components.contains_key("upstream"));
        assert_eq!(
            components["la_plugins"]["version"].as_str(),
            Some("1.4.4.0")
        );
        assert_eq!(
            components["inventory_simulator"]["version"].as_str(),
            Some("3.7.0")
        );
        let mut ownership: Vec<&str> = doc["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["ownership"].as_str().unwrap())
            .collect();
        ownership.sort_unstable();
        assert_eq!(ownership, vec!["plus", "plus", "third-party"]);
        assert_no_temp_files(&directory);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn restore_all_removes_all_three() {
        let base = root("task13-restore-all");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        for component in INSTALL_ORDER {
            install_component(&payloads[&component], &state, &target, component).unwrap();
        }
        // A file no component manages must survive the whole teardown.
        let unmanaged = target.join("cfg").join("user_notes.cfg");
        fs::create_dir_all(unmanaged.parent().unwrap()).unwrap();
        fs::write(&unmanaged, b"unmanaged").unwrap();
        let directory = installation_dir(&state, &target);

        let restored = restore_all_components(&state, &target).unwrap();

        assert_eq!(restored.removed_files, 5);
        assert_eq!(restored.restored_files, 0);
        assert_eq!(restored.result_kind, "restore_all_components");
        for raw in COMPONENT_SHARED
            .iter()
            .chain(COMPONENT_PLUS.iter())
            .chain(std::iter::once(&THIRD_PARTY))
        {
            assert!(
                !owned_target(&target, raw).exists(),
                "{raw} must be removed"
            );
        }
        assert_eq!(fs::read(&unmanaged).unwrap(), b"unmanaged");
        assert!(!directory.join("record.json").exists());
        assert!(!directory.join("journal.json").exists());
        assert_no_temp_files(&directory);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn install_all_stops_on_first_failure() {
        let base = root("task13-install-all-stops");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        let directory = installation_dir(&state, &target);

        // Upstream lands first; then an unmanaged file makes the LA transaction conflict, so the
        // inventory component that follows must never be attempted.
        install_component(
            &payloads[&ComponentId::Upstream],
            &state,
            &target,
            ComponentId::Upstream,
        )
        .unwrap();
        let blocked = owned_target(&target, COMPONENT_PLUS[0]);
        fs::create_dir_all(blocked.parent().unwrap()).unwrap();
        fs::write(&blocked, b"unmanaged-plus-path").unwrap();
        let before = snapshot_target(&target);

        let error = run_component_steps(&INSTALL_ORDER, |component| {
            install_component(&payloads[&component], &state, &target, component)
        })
        .expect_err("the run must abort at the LA component");

        assert_eq!(error.code, "E1403", "{}", error.detail);
        // The failing transaction wrote nothing and the inventory component was never started.
        assert_eq!(snapshot_target(&target), before);
        assert!(!owned_target(&target, THIRD_PARTY).exists());
        assert!(!owned_target(&target, COMPONENT_PLUS[1]).exists());
        let versions = component_versions(&directory);
        assert_eq!(versions.len(), 1, "{versions:?}");
        assert_eq!(versions.get("upstream").map(String::as_str), Some("1.4.3.3"));
        assert!(!versions.contains_key("inventory_simulator"));
        assert_no_temp_files(&directory);
        fs::remove_dir_all(base).unwrap();
    }

    /// Final-wave surface test (plan row F3): drives the real entry points through a full
    /// install -> uninstall -> restore-all lifecycle and reads back only the filesystem and the
    /// record bytes on disk. Each phase prints a one-line PASS marker for a human reader.
    #[test]
    fn f3_surface_end_to_end() {
        let base = root("task13-f3-surface");
        let target = base.join("target");
        let state = base.join("state");
        fs::create_dir_all(&target).unwrap();
        let payloads = component_payloads(&base);
        let directory = installation_dir(&state, &target);
        let unmanaged = target.join("cfg").join("user_notes.cfg");
        fs::create_dir_all(unmanaged.parent().unwrap()).unwrap();
        fs::write(&unmanaged, b"unmanaged-user-file").unwrap();

        // Phase 1: three independent component transactions land exactly their own files.
        for component in INSTALL_ORDER {
            let result = install_component(&payloads[&component], &state, &target, component)
                .unwrap_or_else(|error| panic!("{component:?} install failed: {error:?}"));
            assert!(result.installed_files >= 1, "{component:?}");
        }
        let mut expected: BTreeMap<String, String> = BTreeMap::new();
        for raw in COMPONENT_SHARED
            .iter()
            .chain(COMPONENT_PLUS.iter())
            .chain(std::iter::once(&THIRD_PARTY))
        {
            expected.insert((*raw).to_string(), sha256(&owned_target(&target, raw)).unwrap());
        }
        expected.insert(
            "cfg/user_notes.cfg".to_string(),
            sha256(&unmanaged).unwrap(),
        );
        assert_eq!(
            snapshot_target(&target),
            expected,
            "the target must hold exactly the union of the three payloads plus the unmanaged file"
        );
        let doc = read_record_document(&directory);
        let components = doc["components"].as_object().unwrap();
        assert_eq!(components.len(), 3);
        assert_eq!(
            components["la_plugins"]["version"].as_str(),
            Some("1.4.4.0")
        );
        println!(
            "PHASE 1 PASS install: files={} versions={:?}",
            expected.len(),
            component_versions(&directory)
        );

        // Phase 2: uninstalling one component removes only that component's files.
        let removed = uninstall_component(&state, &target, ComponentId::LaPlugins).unwrap();
        assert_eq!(removed.removed_files, 2);
        assert!(!owned_target(&target, COMPONENT_PLUS[0]).exists());
        assert!(!owned_target(&target, COMPONENT_PLUS[1]).exists());
        assert!(owned_target(&target, COMPONENT_SHARED[0]).is_file());
        assert!(owned_target(&target, COMPONENT_SHARED[1]).is_file());
        assert!(owned_target(&target, THIRD_PARTY).is_file());
        assert_eq!(fs::read(&unmanaged).unwrap(), b"unmanaged-user-file");
        let after_uninstall = snapshot_target(&target);
        assert_eq!(after_uninstall.len(), 4, "{after_uninstall:?}");
        let doc = read_record_document(&directory);
        assert_eq!(doc["components"].as_object().unwrap().len(), 2);
        println!(
            "PHASE 2 PASS uninstall: removed={} remaining={:?}",
            removed.removed_files,
            after_uninstall.keys().collect::<Vec<_>>()
        );

        // Phase 3: restore-all clears every managed file and leaves unmanaged content alone.
        let restored = restore_all_components(&state, &target).unwrap();
        assert_eq!(restored.removed_files, 3);
        assert_eq!(restored.result_kind, "restore_all_components");
        let remaining = snapshot_target(&target);
        assert_eq!(
            remaining.keys().collect::<Vec<_>>(),
            vec!["cfg/user_notes.cfg"],
            "only the unmanaged file may survive restore-all"
        );
        assert_eq!(fs::read(&unmanaged).unwrap(), b"unmanaged-user-file");
        assert!(!directory.join("record.json").exists());
        assert!(!directory.join("journal.json").exists());
        assert_no_temp_files(&directory);
        println!(
            "PHASE 3 PASS restore-all: removed={} remaining={:?}",
            restored.removed_files,
            remaining.keys().collect::<Vec<_>>()
        );
        println!("f3_surface_end_to_end: PASS");
        fs::remove_dir_all(base).unwrap();
    }
}
