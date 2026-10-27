//! Source switching for the InventorySimulator plugin.
//!
//! The plugin has no runtime mode flag: it loads `invsim_file` (default `inventories.json`)
//! from `<csgo>/addons/counterstrikesharp/configs/plugins/InventorySimulator/` or `<csgo>/`,
//! and only calls the hosted service when neither file exists. Mode is therefore expressed by
//! file presence alone; `autoexec.cfg` is never written.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic_fs;
use crate::AppError;

pub const PLUGIN_DLL: &str = "addons/counterstrikesharp/plugins/InventorySimulator/InventorySimulator.dll";
pub const CONFIG_DIR: &str = "addons/counterstrikesharp/configs/plugins/InventorySimulator";
pub const INVENTORY_FILE: &str = "inventories.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CosmeticsSource {
    Remote,
    Local,
}

impl Default for CosmeticsSource {
    fn default() -> Self {
        Self::Remote
    }
}

impl CosmeticsSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Local => "local",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "remote" => Some(Self::Remote),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
}

pub fn inventory_path(root: &Path) -> PathBuf {
    root.join(CONFIG_DIR).join(INVENTORY_FILE)
}

// The plugin accepts this second location too, so a leftover file here would keep local mode
// alive after a switch to remote.
pub fn secondary_inventory_path(root: &Path) -> PathBuf {
    root.join(INVENTORY_FILE)
}

pub fn plugin_installed(root: &Path) -> bool {
    root.join(PLUGIN_DLL).is_file()
}

#[derive(Clone, Debug, Serialize)]
pub struct InventoryFileState {
    pub path: String,
    pub bytes: u64,
    pub valid_json: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CosmeticsSourceState {
    pub effective: CosmeticsSource,
    pub selected: Option<CosmeticsSource>,
    pub plugin_installed: bool,
    pub files: Vec<InventoryFileState>,
    pub mismatch: bool,
}

fn describe(path: &Path) -> Option<InventoryFileState> {
    let metadata = fs::metadata(path).ok()?;
    let bytes = metadata.len();
    let valid_json = bytes > 0
        && fs::read(path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
            .is_some();
    Some(InventoryFileState {
        path: path.to_string_lossy().into_owned(),
        bytes,
        valid_json,
    })
}

pub fn inspect(root: &Path, selected: Option<CosmeticsSource>) -> CosmeticsSourceState {
    let mut files = Vec::new();
    for path in [inventory_path(root), secondary_inventory_path(root)] {
        if let Some(state) = describe(&path) {
            files.push(state);
        }
    }
    let effective = if files.iter().any(|file| file.valid_json) {
        CosmeticsSource::Local
    } else {
        // An empty or broken file makes the plugin log an error and call the hosted service.
        CosmeticsSource::Remote
    };
    CosmeticsSourceState {
        effective,
        selected,
        plugin_installed: plugin_installed(root),
        files,
        mismatch: selected.is_some_and(|value| value != effective),
    }
}

/// `document` must be a non-empty JSON object containing `steam_id`: the plugin throws on
/// empty input, and a document without that key silently falls back to the hosted service.
pub fn write_local(root: &Path, steam_id: &str, document: &str) -> Result<PathBuf, AppError> {
    let trimmed = document.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid("Refusing to write an empty inventories.json"));
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|error| AppError::invalid(format!("Inventory document is not valid JSON: {error}")))?;
    let object = parsed
        .as_object()
        .ok_or_else(|| AppError::invalid("Inventory document must be a JSON object keyed by SteamID64"))?;
    if !object.contains_key(steam_id) {
        return Err(AppError::invalid(format!(
            "Inventory document has no entry for SteamID64 {steam_id}"
        )));
    }

    let path = inventory_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_fs::write_replace(&path, trimmed.as_bytes())?;
    if fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) == 0 {
        let _ = fs::remove_file(&path);
        return Err(AppError::io("Inventory file ended up empty; removed it"));
    }

    // Keep the secondary location clean so remote mode cannot silently stay local later.
    let secondary = secondary_inventory_path(root);
    if secondary.is_file() {
        let backup = secondary.with_extension("json.bak");
        let _ = fs::remove_file(&backup);
        fs::rename(&secondary, &backup)?;
    }
    Ok(path)
}

pub fn switch_to_remote(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut moved = Vec::new();
    for path in [inventory_path(root), secondary_inventory_path(root)] {
        if !path.is_file() {
            continue;
        }
        let backup = path.with_extension("json.bak");
        if backup.is_file() {
            fs::remove_file(&backup)?;
        }
        fs::rename(&path, &backup)?;
        moved.push(backup);
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "csbip-cosmetics-source-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    const DOC: &str = r#"{"76561199145436841":{"knives":{"3":{"def":515,"paint":12}}}}"#;

    #[test]
    fn effective_is_remote_when_no_inventory_file_exists() {
        let root = temp_root("remote-empty");
        let state = inspect(&root, Some(CosmeticsSource::Remote));
        assert_eq!(state.effective, CosmeticsSource::Remote);
        assert!(state.files.is_empty());
        assert!(!state.mismatch);
        assert!(!state.plugin_installed);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn empty_file_counts_as_remote_but_is_reported() {
        let root = temp_root("remote-empty-file");
        let path = inventory_path(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"").unwrap();

        let state = inspect(&root, Some(CosmeticsSource::Remote));
        assert_eq!(state.effective, CosmeticsSource::Remote);
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].bytes, 0);
        assert!(!state.files[0].valid_json);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn write_local_creates_valid_file_and_reports_local() {
        let root = temp_root("local-write");
        let path = write_local(&root, "76561199145436841", DOC).unwrap();
        assert_eq!(path, inventory_path(&root));
        assert!(path.is_file());

        let state = inspect(&root, Some(CosmeticsSource::Local));
        assert_eq!(state.effective, CosmeticsSource::Local);
        assert!(state.files.iter().any(|file| file.valid_json));
        assert!(!state.mismatch);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn write_local_rejects_empty_and_missing_key() {
        let root = temp_root("local-reject");
        assert!(write_local(&root, "76561199145436841", "   ").is_err());
        assert!(write_local(&root, "76561199145436841", "not json").is_err());
        assert!(write_local(&root, "76561199145436841", r#"{"1":{}}"#).is_err());
        assert!(!inventory_path(&root).exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn switch_to_remote_parks_both_paths_and_flips_effective() {
        let root = temp_root("switch-remote");
        let primary = inventory_path(&root);
        fs::create_dir_all(primary.parent().unwrap()).unwrap();
        fs::write(&primary, DOC).unwrap();
        let secondary = secondary_inventory_path(&root);
        fs::write(&secondary, DOC).unwrap();

        let moved = switch_to_remote(&root).unwrap();
        assert_eq!(moved.len(), 2);
        assert!(!primary.exists());
        assert!(!secondary.exists());
        assert!(primary.with_extension("json.bak").is_file());
        assert!(secondary.with_extension("json.bak").is_file());

        let state = inspect(&root, Some(CosmeticsSource::Remote));
        assert_eq!(state.effective, CosmeticsSource::Remote);
        assert!(state.files.is_empty());
        assert!(!state.mismatch);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn mismatch_is_reported_when_selection_disagrees_with_disk() {
        let root = temp_root("mismatch");
        write_local(&root, "76561199145436841", DOC).unwrap();
        let state = inspect(&root, Some(CosmeticsSource::Remote));
        assert_eq!(state.effective, CosmeticsSource::Local);
        assert!(state.mismatch);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn parse_accepts_both_values_and_rejects_junk() {
        assert_eq!(CosmeticsSource::parse("Remote"), Some(CosmeticsSource::Remote));
        assert_eq!(CosmeticsSource::parse(" local "), Some(CosmeticsSource::Local));
        assert_eq!(CosmeticsSource::parse("auto"), None);
        assert_eq!(CosmeticsSource::default(), CosmeticsSource::Remote);
    }
}
