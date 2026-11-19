use crate::{AppError, Result, atomic_fs};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const STATE_DIRECTORY: &str = "localarena";
#[cfg(not(test))]
static RESOLVED_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);
#[cfg(test)]
pub(crate) static TEST_STATE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UiMemory {
    pub schema_version: u32,
    pub saved_at: u64,
    pub entries: BTreeMap<String, String>,
}

fn unix_time() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_fs::write_replace(path, bytes).map_err(AppError::transaction_io)
}

#[cfg(windows)]
fn config_base_directory() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|home| PathBuf::from(home).join("AppData").join("Roaming"))
        })
}

#[cfg(not(windows))]
fn config_base_directory() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

fn ensure_writable(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    let probe = path.join(format!(
        ".state-write-probe-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
    ));
    atomic_fs::write_replace(&probe, b"state-write-probe")?;
    let result = fs::read(&probe).and_then(|bytes| {
        if bytes == b"state-write-probe" {
            Ok(())
        } else {
            Err(std::io::Error::other("state write probe returned unexpected data"))
        }
    });
    let cleanup = fs::remove_file(&probe);
    result.and(cleanup)
}

fn resolve_root() -> Result<PathBuf> {
    if let Some(override_path) = std::env::var_os("CS2BI_STATE_ROOT") {
        let path = PathBuf::from(override_path);
        ensure_writable(&path).map_err(|error| {
            AppError::transaction(format!(
                "CS2BI_STATE_ROOT is not writable ({}): {error}",
                path.display()
            ))
        })?;
        return Ok(path);
    }

    let base = config_base_directory().ok_or_else(|| {
        AppError::transaction(
            "Cannot locate the per-user configuration directory (APPDATA / XDG_CONFIG_HOME / HOME)",
        )
    })?;
    let path = base.join(STATE_DIRECTORY);
    ensure_writable(&path).map_err(|error| {
        AppError::transaction(format!(
            "The per-user state directory is not writable ({}): {error}",
            path.display()
        ))
    })?;
    Ok(path)
}

pub fn root() -> Result<PathBuf> {
    #[cfg(test)]
    {
        resolve_root()
    }
    #[cfg(not(test))]
    {
        let mut resolved = RESOLVED_ROOT.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(path) = resolved.as_ref() {
            return Ok(path.clone());
        }
        let path = resolve_root()?;
        *resolved = Some(path.clone());
        Ok(path)
    }
}

pub fn panel_config_path() -> Result<PathBuf> {
    Ok(root()?.join("config").join("panel.json"))
}

pub fn read_ui_memory() -> Result<UiMemory> {
    let path = root()?.join("memory").join("ui-state.json");
    if !path.is_file() {
        return Ok(UiMemory { schema_version: 1, ..UiMemory::default() });
    }
    let bytes = fs::read(&path).map_err(AppError::transaction_io)?;
    let memory = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::invalid(format!("Invalid Panel UI memory: {error}")))?;
    Ok(memory)
}

pub fn write_ui_memory(entries: BTreeMap<String, String>) -> Result<UiMemory> {
    let memory = UiMemory { schema_version: 1, saved_at: unix_time(), entries };
    let bytes = serde_json::to_vec_pretty(&memory)
        .map_err(|error| AppError::transaction(error.to_string()))?;
    write_atomic(&root()?.join("memory").join("ui-state.json"), &bytes)?;
    Ok(memory)
}

fn copy_atomic(source: &Path, destination: &Path) -> Result<()> {
    let bytes = fs::read(source).map_err(AppError::transaction_io)?;
    write_atomic(destination, &bytes)
}

pub fn mirror_cosmetics(csgo: &Path) -> Result<usize> {
    let source = crate::cosmetics_source::inventory_path(csgo);
    let destination = root()?.join("presets").join("current");
    let mut copied = 0;
    if source.is_file() {
        copy_atomic(&source, &destination.join("inventories.json"))?;
        copied = 1;
    }
    if copied > 0 {
        let metadata = serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": 2,
            "saved_at": unix_time(),
            "source": csgo.to_string_lossy(),
            "files_copied": copied,
        })).map_err(|error| AppError::transaction(error.to_string()))?;
        write_atomic(&destination.join("mirror.json"), &metadata)?;
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_directory_is_the_branded_config_directory() {
        assert_eq!(STATE_DIRECTORY, "localarena");
        let base = std::env::temp_dir().join(format!("localarena-config-{}", unix_time()));
        let resolved = base.join(STATE_DIRECTORY);
        ensure_writable(&resolved).unwrap();
        assert_eq!(resolved.file_name().and_then(|name| name.to_str()), Some("localarena"));
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn ui_memory_and_cosmetics_are_stored_under_the_state_root() {
        let _guard = TEST_STATE_ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let base = std::env::temp_dir().join(format!("cs2bi-storage-{}", unix_time()));
        unsafe { std::env::set_var("CS2BI_STATE_ROOT", &base); }
        let mut entries = BTreeMap::new();
        entries.insert("cs2bi.cosmeticsTeam".to_string(), "t".to_string());
        write_ui_memory(entries.clone()).unwrap();
        assert_eq!(read_ui_memory().unwrap().entries, entries);

        let csgo = base.join("fake-csgo");
        let cosmetics = csgo.join(crate::cosmetics_source::INVENTORY_RELATIVE);
        fs::create_dir_all(cosmetics.parent().unwrap()).unwrap();
        fs::write(&cosmetics, b"{}").unwrap();
        assert_eq!(mirror_cosmetics(&csgo).unwrap(), 1);
        assert!(base.join("presets/current/inventories.json").is_file());
        fs::remove_dir_all(&base).unwrap();
        unsafe { std::env::remove_var("CS2BI_STATE_ROOT"); }
    }
}
