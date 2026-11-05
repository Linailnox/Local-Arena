use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Read, Seek};
use std::path::{Component, Path, PathBuf};

pub const MAX_MANIFEST_BYTES: u64 = 512 * 1024;
pub const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 50_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayVersion {
    parts: [u32; 4],
    preview: Option<u32>,
}

impl DisplayVersion {
    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        let value = value.trim().trim_start_matches(['v', 'V']);
        let (numeric, preview) = match value.split_once('-') {
            Some((numeric, suffix)) => {
                let Some(number) = suffix
                    .to_ascii_lowercase()
                    .strip_prefix("preview.")
                    .map(str::to_owned)
                else {
                    return Err(format!("Unsupported version suffix: {suffix}"));
                };
                if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(format!("Invalid preview version: {suffix}"));
                }
                let number = number
                    .parse::<u32>()
                    .map_err(|_| format!("Preview version is too large: {suffix}"))?;
                (numeric, Some(number))
            }
            None => (value, None),
        };
        let parts = numeric.split('.').collect::<Vec<_>>();
        if parts.len() != 4 {
            return Err(format!("Version must contain four numeric parts: {value}"));
        }
        let mut parsed = [0; 4];
        for (index, part) in parts.into_iter().enumerate() {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(format!("Invalid version component: {part}"));
            }
            parsed[index] = part
                .parse::<u32>()
                .map_err(|_| format!("Version component is too large: {part}"))?;
        }
        Ok(Self {
            parts: parsed,
            preview,
        })
    }
}

impl Ord for DisplayVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts.cmp(&other.parts).then_with(|| match (self.preview, other.preview) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
    }
}

impl PartialOrd for DisplayVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteComponent {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub min_panel_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteComponents {
    pub panel: RemoteComponent,
    pub plugin: RemoteComponent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteUpdateManifest {
    pub schema_version: u8,
    pub release_version: String,
    pub published_at: String,
    pub release_notes_url: String,
    pub components: RemoteComponents,
}

impl RemoteUpdateManifest {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.schema_version != 1 {
            return Err("Unsupported update manifest schema".into());
        }
        DisplayVersion::parse(&self.release_version)?;
        validate_https_github_url(&self.release_notes_url)?;
        for component in [&self.components.panel, &self.components.plugin] {
            DisplayVersion::parse(&component.version)?;
            DisplayVersion::parse(&component.min_panel_version)?;
            validate_https_github_url(&component.url)?;
            if component.size == 0 || component.size > MAX_ARCHIVE_BYTES {
                return Err(format!(
                    "Update component size is outside the allowed range: {}",
                    component.size
                ));
            }
            validate_sha256(&component.sha256)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaPluginsComponents {
    pub windows: Option<RemoteComponent>,
    pub linux: Option<RemoteComponent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteComponentsV2 {
    pub panel: RemoteComponent,
    pub la_plugins: LaPluginsComponents,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteUpdateManifestV2 {
    pub schema_version: u8,
    pub release_version: String,
    pub published_at: String,
    pub release_notes_url: String,
    pub components: RemoteComponentsV2,
}

impl RemoteUpdateManifestV2 {
    pub fn validate(&self) -> std::result::Result<(), String> {
        validate_v2(self)
    }
}

fn validate_component(component: &RemoteComponent) -> std::result::Result<(), String> {
    DisplayVersion::parse(&component.version)?;
    DisplayVersion::parse(&component.min_panel_version)?;
    validate_https_github_url(&component.url)?;
    if component.size == 0 || component.size > MAX_ARCHIVE_BYTES {
        return Err(format!(
            "Update component size is outside the allowed range: {}",
            component.size
        ));
    }
    validate_sha256(&component.sha256)
}

pub fn validate_v2(manifest: &RemoteUpdateManifestV2) -> std::result::Result<(), String> {
    if manifest.schema_version != 2 {
        return Err("Unsupported update manifest schema".into());
    }
    DisplayVersion::parse(&manifest.release_version)?;
    validate_https_github_url(&manifest.release_notes_url)?;
    validate_component(&manifest.components.panel)?;
    let la_plugins = &manifest.components.la_plugins;
    if la_plugins.windows.is_none() && la_plugins.linux.is_none() {
        return Err("Update manifest must publish at least one LA plugin archive".into());
    }
    for component in [&la_plugins.windows, &la_plugins.linux].into_iter().flatten() {
        validate_component(component)?;
    }
    Ok(())
}

pub fn parse_update_manifest(
    bytes: &[u8],
) -> std::result::Result<RemoteUpdateManifestV2, String> {
    #[derive(Deserialize)]
    struct SchemaPeek {
        schema_version: u8,
    }

    let peek: SchemaPeek = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid update manifest: {error}"))?;
    match peek.schema_version {
        2 => {
            let manifest: RemoteUpdateManifestV2 = serde_json::from_slice(bytes)
                .map_err(|error| format!("Invalid update manifest: {error}"))?;
            manifest.validate()?;
            Ok(manifest)
        }
        1 => {
            let manifest: RemoteUpdateManifest = serde_json::from_slice(bytes)
                .map_err(|error| format!("Invalid update manifest: {error}"))?;
            manifest.validate()?;
            // Schema v1 only ever published a Windows LA plugin archive.
            Ok(RemoteUpdateManifestV2 {
                schema_version: 2,
                release_version: manifest.release_version,
                published_at: manifest.published_at,
                release_notes_url: manifest.release_notes_url,
                components: RemoteComponentsV2 {
                    panel: manifest.components.panel,
                    la_plugins: LaPluginsComponents {
                        windows: Some(manifest.components.plugin),
                        linux: None,
                    },
                },
            })
        }
        _ => Err("Unsupported update manifest schema".into()),
    }
}

pub fn validate_https_github_url(value: &str) -> std::result::Result<(), String> {
    let Some(rest) = value.strip_prefix("https://") else {
        return Err("Update URLs must use HTTPS".into());
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.contains(':') {
        return Err("Update URL has an invalid authority".into());
    }
    let host = authority.to_ascii_lowercase();
    let accepted = host == "github.com"
        || host == "api.github.com"
        || host == "objects.githubusercontent.com"
        || host.ends_with(".githubusercontent.com");
    if !accepted {
        return Err(format!("Update URL host is not trusted: {host}"));
    }
    Ok(())
}

pub fn validate_sha256(value: &str) -> std::result::Result<(), String> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("SHA-256 must contain 64 hexadecimal characters".into())
    }
}

pub fn verify_manifest_signature(
    manifest: &[u8],
    signature_base64: &str,
    public_key_base64: &str,
) -> std::result::Result<(), String> {
    let engine = base64::engine::general_purpose::STANDARD;
    let public = engine
        .decode(public_key_base64.trim())
        .map_err(|_| "Embedded update public key is invalid Base64")?;
    let signature = engine
        .decode(signature_base64.trim())
        .map_err(|_| "Update manifest signature is invalid Base64")?;
    let public: [u8; 32] = public
        .try_into()
        .map_err(|_| "Embedded update public key must be 32 bytes")?;
    let signature = Signature::from_slice(&signature)
        .map_err(|_| "Update manifest signature must be 64 bytes")?;
    let key =
        VerifyingKey::from_bytes(&public).map_err(|_| "Embedded update public key is invalid")?;
    key.verify(manifest, &signature)
        .map_err(|_| "Update manifest signature verification failed".into())
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn verify_component_file(
    path: &Path,
    expected_size: u64,
    expected_hash: &str,
) -> std::result::Result<(), String> {
    validate_sha256(expected_hash)?;
    let metadata =
        fs::metadata(path).map_err(|error| format!("Cannot read downloaded update: {error}"))?;
    if metadata.len() != expected_size {
        return Err(format!(
            "Downloaded update size mismatch: expected {expected_size}, got {}",
            metadata.len()
        ));
    }
    let actual =
        sha256_file(path).map_err(|error| format!("Cannot hash downloaded update: {error}"))?;
    if !actual.eq_ignore_ascii_case(expected_hash) {
        return Err(format!(
            "Downloaded update SHA-256 mismatch: expected {expected_hash}, got {actual}"
        ));
    }
    Ok(())
}

pub fn replace_file_with_backup(
    staged: &Path,
    target: &Path,
    backup: &Path,
) -> std::result::Result<(), String> {
    if !staged.is_file() {
        return Err("Staged update file is missing".into());
    }
    if !target.is_file() {
        return Err("Current Panel executable is missing".into());
    }
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create Panel backup directory: {error}"))?;
    }
    fs::copy(target, backup).map_err(|error| format!("Cannot back up current Panel: {error}"))?;
    let temporary = crate::atomic_fs::temporary_path(target).map_err(|error| error.to_string())?;
    let result = (|| -> std::result::Result<(), String> {
        fs::copy(staged, &temporary)
            .map_err(|error| format!("Cannot stage Panel replacement: {error}"))?;
        crate::atomic_fs::sync(&temporary)
            .map_err(|error| format!("Cannot flush Panel replacement: {error}"))?;
        crate::atomic_fs::replace(&temporary, target)
            .map_err(|error| format!("Cannot replace Panel executable: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn install_file_with_backup(
    staged: &Path,
    current: &Path,
    target: &Path,
    backup: &Path,
) -> std::result::Result<(), String> {
    if current == target {
        return replace_file_with_backup(staged, target, backup);
    }
    if !staged.is_file() {
        return Err("Staged update file is missing".into());
    }
    if !current.is_file() {
        return Err("Current Panel executable is missing".into());
    }
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create Panel backup directory: {error}"))?;
    }
    fs::copy(current, backup).map_err(|error| format!("Cannot back up current Panel: {error}"))?;
    let temporary = crate::atomic_fs::temporary_path(target).map_err(|error| error.to_string())?;
    let result = (|| -> std::result::Result<(), String> {
        fs::copy(staged, &temporary)
            .map_err(|error| format!("Cannot stage Panel replacement: {error}"))?;
        crate::atomic_fs::sync(&temporary)
            .map_err(|error| format!("Cannot flush Panel replacement: {error}"))?;
        crate::atomic_fs::replace(&temporary, target)
            .map_err(|error| format!("Cannot install Panel executable: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn safe_entry_path(name: &str) -> std::result::Result<PathBuf, String> {
    if name.contains('\0') || name.contains('\\') || name.starts_with('/') {
        return Err(format!("Unsafe ZIP entry path: {name}"));
    }
    let path = Path::new(name);
    if path.as_os_str().is_empty() {
        return Err("ZIP entry has an empty path".into());
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => safe.push(value),
            Component::CurDir => {}
            _ => return Err(format!("Unsafe ZIP entry path: {name}")),
        }
    }
    if safe.as_os_str().is_empty() {
        return Err(format!("Unsafe ZIP entry path: {name}"));
    }
    Ok(safe)
}

pub fn extract_zip_safely<R: Read + Seek>(
    reader: R,
    destination: &Path,
) -> std::result::Result<u64, String> {
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|error| format!("Invalid update ZIP: {error}"))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("Update ZIP contains too many entries".into());
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("Cannot create update directory: {error}"))?;
    let mut extracted = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Cannot read ZIP entry: {error}"))?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!(
                "ZIP symbolic links are not allowed: {}",
                entry.name()
            ));
        }
        extracted = extracted
            .checked_add(entry.size())
            .ok_or_else(|| "Update ZIP extracted size overflow".to_string())?;
        if extracted > MAX_EXTRACTED_BYTES {
            return Err("Update ZIP exceeds the extracted size limit".into());
        }
        let relative = safe_entry_path(entry.name())?;
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output)
                .map_err(|error| format!("Cannot create ZIP directory: {error}"))?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create ZIP directory: {error}"))?;
        }
        let mut file = File::create(&output)
            .map_err(|error| format!("Cannot create extracted file: {error}"))?;
        io::copy(&mut entry, &mut file)
            .map_err(|error| format!("Cannot extract update file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Cannot flush extracted update file: {error}"))?;
    }
    Ok(extracted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::{Cursor, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "cs2bi-update-core-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn compares_four_part_versions_numerically() {
        assert!(
            DisplayVersion::parse("1.4.2.10").unwrap() > DisplayVersion::parse("1.4.2.3").unwrap()
        );
        assert!(
            DisplayVersion::parse("v2.0.0.0").unwrap()
                > DisplayVersion::parse("1.99.99.99").unwrap()
        );
        assert!(DisplayVersion::parse("1.4.2").is_err());
        assert!(DisplayVersion::parse("1.4.2.fix").is_err());
        assert!(DisplayVersion::parse("1.4.2.5-Preview.2").is_ok());
        assert!(
            DisplayVersion::parse("1.4.2.5-Preview.2").unwrap()
                > DisplayVersion::parse("1.4.2.5-Preview.1").unwrap()
        );
        assert!(
            DisplayVersion::parse("1.4.2.5").unwrap()
                > DisplayVersion::parse("1.4.2.5-Preview.2").unwrap()
        );
        assert!(DisplayVersion::parse("1.4.2.5-beta.2").is_err());
    }

    #[test]
    fn verifies_ed25519_manifest_and_rejects_changes() {
        let signing = SigningKey::from_bytes(&[7_u8; 32]);
        let manifest = br#"{"schema_version":1}"#;
        let signature = signing.sign(manifest);
        let engine = base64::engine::general_purpose::STANDARD;
        let public = engine.encode(signing.verifying_key().to_bytes());
        let signature = engine.encode(signature.to_bytes());
        verify_manifest_signature(manifest, &signature, &public).unwrap();
        assert!(verify_manifest_signature(b"changed", &signature, &public).is_err());
    }

    #[test]
    fn component_verification_checks_size_and_hash() {
        let directory = root();
        fs::create_dir_all(&directory).unwrap();
        let file = directory.join("component.zip");
        fs::write(&file, b"payload").unwrap();
        let hash = sha256_file(&file).unwrap();
        verify_component_file(&file, 7, &hash).unwrap();
        assert!(verify_component_file(&file, 8, &hash).is_err());
        assert!(verify_component_file(&file, 7, &"0".repeat(64)).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parses_and_validates_v2_manifest_with_panel_and_la_plugins() {
        let manifest = serde_json::json!({
            "schema_version": 2,
            "release_version": "1.4.3.4",
            "published_at": "2026-11-05T00:00:00Z",
            "release_notes_url": "https://github.com/numakkiyu/Local-Arena/releases/tag/v1.4.3.4",
            "components": {
                "panel": component_json("1.4.3.4"),
                "la_plugins": {
                    "windows": component_json("1.4.3.4"),
                    "linux": component_json("1.4.3.4"),
                },
            },
        });
        let parsed = parse_update_manifest(manifest.to_string().as_bytes()).unwrap();
        assert_eq!(parsed.schema_version, 2);
        assert_eq!(parsed.release_version, "1.4.3.4");
        assert_eq!(parsed.components.panel.version, "1.4.3.4");
        let windows = parsed.components.la_plugins.windows.unwrap();
        assert_eq!(windows.size, 1024);
        let linux = parsed.components.la_plugins.linux.unwrap();
        assert_eq!(linux.version, "1.4.3.4");
    }

    #[test]
    fn converts_v1_manifest_to_v2_la_plugins_windows() {
        let manifest = serde_json::json!({
            "schema_version": 1,
            "release_version": "1.4.3.3",
            "published_at": "2026-10-31T00:00:00Z",
            "release_notes_url": "https://github.com/numakkiyu/Local-Arena/releases/tag/v1.4.3.3",
            "components": {
                "panel": component_json("1.4.3.3"),
                "plugin": component_json("1.4.3.2"),
            },
        });
        let parsed = parse_update_manifest(manifest.to_string().as_bytes()).unwrap();
        assert_eq!(parsed.schema_version, 2);
        assert_eq!(parsed.release_version, "1.4.3.3");
        assert_eq!(parsed.components.panel.version, "1.4.3.3");
        assert_eq!(parsed.components.panel.sha256, "a".repeat(64));
        assert_eq!(parsed.components.panel.min_panel_version, "1.4.3.0");
        let windows = parsed.components.la_plugins.windows.unwrap();
        assert_eq!(windows.version, "1.4.3.2");
        assert_eq!(
            windows.url,
            component_json("1.4.3.2")["url"].as_str().unwrap()
        );
        assert!(parsed.components.la_plugins.linux.is_none());
    }

    #[test]
    fn rejects_v1_manifest_without_plugin_component() {
        let manifest = serde_json::json!({
            "schema_version": 1,
            "release_version": "1.4.3.3",
            "published_at": "2026-10-31T00:00:00Z",
            "release_notes_url": "https://github.com/numakkiyu/Local-Arena/releases/tag/v1.4.3.3",
            "components": {
                "panel": component_json("1.4.3.3"),
            },
        });
        assert!(parse_update_manifest(manifest.to_string().as_bytes()).is_err());
    }

    #[test]
    fn rejects_unsupported_schema_and_invalid_v2_components() {
        let base = serde_json::json!({
            "schema_version": 2,
            "release_version": "1.4.3.4",
            "published_at": "2026-11-05T00:00:00Z",
            "release_notes_url": "https://github.com/numakkiyu/Local-Arena/releases/tag/v1.4.3.4",
            "components": {
                "panel": component_json("1.4.3.4"),
                "la_plugins": {
                    "windows": component_json("1.4.3.4"),
                    "linux": null,
                },
            },
        });
        parse_update_manifest(base.to_string().as_bytes()).unwrap();

        let mut future = base.clone();
        future["schema_version"] = serde_json::json!(3);
        assert!(parse_update_manifest(future.to_string().as_bytes()).is_err());

        let mut empty = base.clone();
        empty["components"]["la_plugins"]["windows"] = serde_json::Value::Null;
        assert!(parse_update_manifest(empty.to_string().as_bytes()).is_err());

        let mut bad_hash = base.clone();
        bad_hash["components"]["panel"]["sha256"] = serde_json::json!("not-a-sha256");
        assert!(parse_update_manifest(bad_hash.to_string().as_bytes()).is_err());

        let mut bad_url = base.clone();
        bad_url["components"]["panel"]["url"] = serde_json::json!("http://example.com/panel.zip");
        assert!(parse_update_manifest(bad_url.to_string().as_bytes()).is_err());

        let mut bad_min = base.clone();
        bad_min["components"]["panel"]["min_panel_version"] = serde_json::json!("1.4");
        assert!(parse_update_manifest(bad_min.to_string().as_bytes()).is_err());
    }

    fn component_json(version: &str) -> serde_json::Value {
        serde_json::json!({
            "version": version,
            "url": "https://github.com/numakkiyu/Local-Arena/releases/download/v1.4.3.3/LocalArena-panel-v1.4.3.3-windows.zip",
            "size": 1024,
            "sha256": "a".repeat(64),
            "min_panel_version": "1.4.3.0",
        })
    }

    fn zip_with(name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut output);
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
            zip.finish().unwrap();
        }
        output.into_inner()
    }

    #[test]
    fn safe_zip_extracts_files() {
        let directory = root();
        extract_zip_safely(Cursor::new(zip_with("payload/file.txt", b"ok")), &directory).unwrap();
        assert_eq!(fs::read(directory.join("payload/file.txt")).unwrap(), b"ok");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn safe_zip_rejects_parent_and_windows_paths() {
        let directory = root();
        assert!(
            extract_zip_safely(Cursor::new(zip_with("../escape.txt", b"bad")), &directory).is_err()
        );
        assert!(
            extract_zip_safely(Cursor::new(zip_with("C:\\escape.txt", b"bad")), &directory)
                .is_err()
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn panel_replacement_creates_backup_and_is_atomic() {
        let directory = root();
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("LocalArena.exe");
        let staged = directory.join("new.exe");
        let backup = directory.join("backup/previous.exe");
        fs::write(&target, b"old").unwrap();
        fs::write(&staged, b"new").unwrap();
        replace_file_with_backup(&staged, &target, &backup).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"new");
        assert_eq!(fs::read(backup).unwrap(), b"old");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_panel_stage_leaves_current_executable_unchanged() {
        let directory = root();
        fs::create_dir_all(&directory).unwrap();
        let target = directory.join("LocalArena.exe");
        fs::write(&target, b"old").unwrap();
        assert!(
            replace_file_with_backup(
                &directory.join("missing.exe"),
                &target,
                &directory.join("backup.exe")
            )
            .is_err()
        );
        assert_eq!(fs::read(target).unwrap(), b"old");
        assert!(!directory.join("backup.exe").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn panel_replacement_migrates_legacy_executable_name() {
        let directory = root();
        fs::create_dir_all(&directory).unwrap();
        let current = directory.join("CS2BotImproverPlus.exe");
        let target = directory.join("LocalArena.exe");
        let staged = directory.join("new.exe");
        let backup = directory.join("backup/previous.exe");
        fs::write(&current, b"old").unwrap();
        fs::write(&staged, b"new").unwrap();
        install_file_with_backup(&staged, &current, &target, &backup).unwrap();
        assert_eq!(fs::read(target).unwrap(), b"new");
        assert_eq!(fs::read(current).unwrap(), b"old");
        assert_eq!(fs::read(backup).unwrap(), b"old");
        fs::remove_dir_all(directory).unwrap();
    }
}
