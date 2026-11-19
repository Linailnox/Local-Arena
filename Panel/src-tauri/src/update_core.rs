use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Read, Seek};
use std::path::{Component, Path, PathBuf};

pub const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 50_000;

/// Release version ordering.
///
/// The release line uses three-part SemVer. Wrapping `semver::Version` keeps the
/// pre-release ordering (`1.5.0-Preview.1 < 1.5.0`) and the existing call sites in
/// `installer` — which compare payload package versions for downgrade guards — intact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayVersion {
    version: semver::Version,
}

impl DisplayVersion {
    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        let value = value.trim().trim_start_matches(['v', 'V']);
        // Legacy records used four numeric parts (`1.4.3.3`). Fold the fourth part
        // into SemVer build metadata so old installs still compare against the
        // three-part release line instead of failing to parse.
        let (core, suffix) = match value.split_once('-') {
            Some((core, suffix)) => (core, Some(suffix)),
            None => (value, None),
        };
        let parts = core.split('.').collect::<Vec<_>>();
        let normalized = match parts.as_slice() {
            [major, minor, patch, build]
                if [major, minor, patch, build]
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())) =>
            {
                match suffix {
                    Some(suffix) => format!("{major}.{minor}.{patch}-{suffix}+{build}"),
                    None => format!("{major}.{minor}.{patch}+{build}"),
                }
            }
            _ => value.to_string(),
        };
        let version = semver::Version::parse(&normalized)
            .map_err(|error| format!("Invalid release version {value}: {error}"))?;
        Ok(Self { version })
    }
}

impl Ord for DisplayVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.version.cmp(&other.version)
    }
}

impl PartialOrd for DisplayVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
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
    use std::io::{Cursor, Write};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "localarena-update-core-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn compares_semver_versions_numerically() {
        assert!(
            DisplayVersion::parse("1.4.10").unwrap() > DisplayVersion::parse("1.4.3").unwrap()
        );
        assert!(
            DisplayVersion::parse("v2.0.0").unwrap() > DisplayVersion::parse("1.99.99").unwrap()
        );
        assert!(DisplayVersion::parse("1.4").is_err());
        assert!(DisplayVersion::parse("1.4.3.fix").is_err());
        assert!(DisplayVersion::parse("1.4.3-Preview.2").is_ok());
        assert!(
            DisplayVersion::parse("1.4.3-Preview.2").unwrap()
                > DisplayVersion::parse("1.4.3-Preview.1").unwrap()
        );
        assert!(
            DisplayVersion::parse("1.4.3").unwrap()
                > DisplayVersion::parse("1.4.3-Preview.2").unwrap()
        );
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
}
