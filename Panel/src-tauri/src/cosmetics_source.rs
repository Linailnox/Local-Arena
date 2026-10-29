//! Source switching for the InventorySimulator plugin.
//!
//! The plugin reads `invsim_file` (default `inventories.json`) inside `Load()`, and that call
//! reaches `Utilities.GetPlayers()` before CounterStrikeSharp natives exist. A *valid* file at
//! the default path therefore aborts the whole plugin load. The Panel keeps its inventory under
//! `.csbip/cosmetics/inventories.json` and points `invsim_file` at it from `autoexec.cfg`, which
//! runs after plugin load and applies the file through `ValueChanged` instead.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic_fs;
use crate::AppError;

pub const PLUGIN_DLL: &str =
    "addons/counterstrikesharp/plugins/InventorySimulator/InventorySimulator.dll";
pub const INVENTORY_RELATIVE: &str = ".csbip/cosmetics/inventories.json";
const CONFIG_INVENTORY_RELATIVE: &str =
    "addons/counterstrikesharp/configs/plugins/InventorySimulator/inventories.json";
const CSGO_INVENTORY_RELATIVE: &str = "inventories.json";
const AUTOEXEC_RELATIVE: &str = "cfg/autoexec.cfg";
const BLOCK_START: &str = "// local-arena-cosmetics:start";
const BLOCK_END: &str = "// local-arena-cosmetics:end";

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
    root.join(INVENTORY_RELATIVE)
}

/// Files the plugin loads during `Load()` when `invsim_file` still has its default value.
pub fn default_inventory_paths(root: &Path) -> [PathBuf; 2] {
    [
        root.join(CONFIG_INVENTORY_RELATIVE),
        root.join(CSGO_INVENTORY_RELATIVE),
    ]
}

fn autoexec_path(root: &Path) -> PathBuf {
    root.join(AUTOEXEC_RELATIVE)
}

pub fn plugin_installed(root: &Path) -> bool {
    root.join(PLUGIN_DLL).is_file()
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
    pub autoexec_linked: bool,
}

fn read_autoexec(root: &Path) -> String {
    fs::read_to_string(autoexec_path(root)).unwrap_or_default()
}

fn strip_block(text: &str) -> (String, bool) {
    let mut kept = String::new();
    let mut inside = false;
    let mut removed = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim_end();
        if trimmed == BLOCK_START {
            inside = true;
            removed = true;
            continue;
        }
        if inside && trimmed == BLOCK_END {
            inside = false;
            continue;
        }
        if inside {
            continue;
        }
        kept.push_str(line);
    }
    (kept, removed)
}

fn block_value(text: &str) -> Option<String> {
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']).trim();
        if trimmed == BLOCK_START {
            inside = true;
            continue;
        }
        if inside && trimmed == BLOCK_END {
            return None;
        }
        if inside {
            if let Some(rest) = trimmed.strip_prefix("invsim_file") {
                let value = rest.trim().trim_matches('"');
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn write_block(root: &Path, value: &str) -> Result<(), AppError> {
    let path = autoexec_path(root);
    let existing = read_autoexec(root);
    let eol = if existing.contains("\r\n") { "\r\n" } else { "\n" };
    let (mut kept, _) = strip_block(&existing);
    if !kept.is_empty() && !kept.ends_with('\n') {
        kept.push_str(eol);
    }
    kept.push_str(&format!(
        "{BLOCK_START}{eol}invsim_file \"{value}\"{eol}{BLOCK_END}{eol}"
    ));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_fs::write_replace(&path, kept.as_bytes())?;
    Ok(())
}

fn clear_block(root: &Path) -> Result<(), AppError> {
    let path = autoexec_path(root);
    if !path.is_file() {
        return Ok(());
    }
    let existing = read_autoexec(root);
    let (kept, removed) = strip_block(&existing);
    if !removed {
        return Ok(());
    }
    if kept.trim().is_empty() {
        fs::remove_file(&path)?;
    } else {
        atomic_fs::write_replace(&path, kept.as_bytes())?;
    }
    Ok(())
}

fn park_default_files(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    let mut moved = Vec::new();
    for path in default_inventory_paths(root) {
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

pub fn has_valid_inventory(root: &Path) -> bool {
    describe(&inventory_path(root)).is_some_and(|file| file.valid_json)
}

pub fn inspect(root: &Path, selected: Option<CosmeticsSource>) -> CosmeticsSourceState {
    let files = describe(&inventory_path(root)).into_iter().collect::<Vec<_>>();
    let linked = block_value(&read_autoexec(root))
        .is_some_and(|value| Path::new(&value) == inventory_path(root));
    let effective = if linked && files.iter().any(|file| file.valid_json) {
        CosmeticsSource::Local
    } else {
        CosmeticsSource::Remote
    };
    CosmeticsSourceState {
        effective,
        selected,
        plugin_installed: plugin_installed(root),
        files,
        mismatch: selected.is_some_and(|value| value != effective),
        autoexec_linked: linked,
    }
}

/// `document` must be a non-empty JSON object containing `steam_id`: the plugin throws on
/// empty input, and a document without that key would silently fall back to the hosted service.
pub fn write_local(root: &Path, steam_id: &str, document: &str) -> Result<PathBuf, AppError> {
    let trimmed = document.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid("Refusing to write an empty inventories.json"));
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed).map_err(|error| {
        AppError::invalid(format!("Inventory document is not valid JSON: {error}"))
    })?;
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

    park_default_files(root)?;
    write_block(root, &path.to_string_lossy())?;
    Ok(path)
}

pub fn switch_to_remote(root: &Path) -> Result<Vec<PathBuf>, AppError> {
    clear_block(root)?;
    park_default_files(root)
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
    fn effective_is_remote_without_file_and_block() {
        let root = temp_root("remote-empty");
        let state = inspect(&root, Some(CosmeticsSource::Remote));
        assert_eq!(state.effective, CosmeticsSource::Remote);
        assert!(state.files.is_empty());
        assert!(!state.autoexec_linked);
        assert!(!state.mismatch);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn write_local_creates_file_and_autoexec_block() {
        let root = temp_root("local-write");
        let path = write_local(&root, "76561199145436841", DOC).unwrap();
        assert_eq!(path, inventory_path(&root));
        assert!(path.is_file());

        let autoexec = read_autoexec(&root);
        assert!(autoexec.contains(BLOCK_START));
        assert!(autoexec.contains("invsim_file"));

        let state = inspect(&root, Some(CosmeticsSource::Local));
        assert_eq!(state.effective, CosmeticsSource::Local);
        assert!(state.autoexec_linked);
        assert!(!state.mismatch);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn write_local_parks_the_default_path_files() {
        let root = temp_root("park-defaults");
        for path in default_inventory_paths(&root) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, DOC).unwrap();
        }

        write_local(&root, "76561199145436841", DOC).unwrap();

        for path in default_inventory_paths(&root) {
            assert!(!path.exists(), "default path must stay empty: {path:?}");
            assert!(path.with_extension("json.bak").is_file());
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn switch_to_remote_removes_block_and_keeps_user_lines() {
        let root = temp_root("switch-remote");
        write_local(&root, "76561199145436841", DOC).unwrap();
        let autoexec = autoexec_path(&root);
        let user_line = "// user setting\nsv_cheats 1\n";
        let text = read_autoexec(&root);
        fs::write(&autoexec, format!("{user_line}{text}")).unwrap();

        switch_to_remote(&root).unwrap();

        let after = read_autoexec(&root);
        assert!(after.contains("sv_cheats 1"), "user lines must survive");
        assert!(!after.contains(BLOCK_START));
        let state = inspect(&root, Some(CosmeticsSource::Remote));
        assert_eq!(state.effective, CosmeticsSource::Remote);
        assert!(!state.autoexec_linked);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn switching_twice_is_idempotent() {
        let root = temp_root("idempotent");
        write_local(&root, "76561199145436841", DOC).unwrap();
        let first = read_autoexec(&root);
        write_local(&root, "76561199145436841", DOC).unwrap();
        assert_eq!(read_autoexec(&root), first);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn block_must_point_at_the_panel_file_to_count_as_local() {
        let root = temp_root("stale-block");
        let path = write_local(&root, "76561199145436841", DOC).unwrap();
        assert!(path.is_file());
        let autoexec = autoexec_path(&root);
        let text = read_autoexec(&root).replace(
            &path.to_string_lossy().to_string(),
            "/somewhere/else.json",
        );
        fs::write(&autoexec, text).unwrap();

        let state = inspect(&root, Some(CosmeticsSource::Local));
        assert_eq!(state.effective, CosmeticsSource::Remote);
        assert!(state.mismatch);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn write_local_rejects_empty_and_missing_key() {
        let root = temp_root("local-reject");
        assert!(write_local(&root, "76561199145436841", "   ").is_err());
        assert!(write_local(&root, "76561199145436841", "not json").is_err());
        assert!(write_local(&root, "76561199145436841", r#"{"1":{}}"#).is_err());
        assert!(!inventory_path(&root).exists());
        assert!(!autoexec_path(&root).exists());
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
