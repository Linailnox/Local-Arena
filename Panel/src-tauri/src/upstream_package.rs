//! Upstream `CS2BotImprover.zip` package management (plan §6.1–§6.4).
//!
//! The LA package only ships its own content; everything owned by the upstream
//! release is downloaded here, cached under `updates/upstream/<tag>/`, and
//! merged with the LA payload into `updates/merged/<tag>/` before any install.

use crate::{
    AppError, Result, app_storage, atomic_fs, installer, logging, online_update, update_core,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

pub const RELEASES_LATEST: &str =
    "https://api.github.com/repos/ed0ard/CS2-Bot-Improver/releases/latest";
pub const LATEST_DOWNLOAD_FALLBACK: &str =
    "https://github.com/ed0ard/CS2-Bot-Improver/releases/latest/download/CS2BotImprover.zip";
pub const ASSET_NAME: &str = "CS2BotImprover.zip";
/// Upstream zip is ~73 MB today; 400 MB leaves headroom for growth (§6.1).
pub const MAX_ARCHIVE_BYTES: u64 = 400 * 1024 * 1024;

const RELEASE_INFO_CACHE_SECONDS: u64 = 6 * 60 * 60;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const RELEASE_BODY_LIMIT: u64 = 1024 * 1024;
const PROGRESS_EVENT: &str = "upstream://progress";
pub const MERGED_EVENT: &str = "upstream://merged";
const MERGE_STATE_FILE: &str = "merge-state.json";
const SOURCE_FILE: &str = "source.json";
const INSTALLED_STATE_FILE: &str = "upstream-install.json";
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(250);
const DOWNLOAD_BUFFER: usize = 128 * 1024;

static MERGE_MUTEX: Mutex<()> = Mutex::new(());
static RELEASE_INFO: OnceLock<Mutex<Option<CachedReleaseInfo>>> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamRelease {
    pub tag: String,
    pub asset_url: String,
    /// `"sha256:…"` from the GitHub API; `None` on the redirect fallback and
    /// manual imports (decision #5).
    pub digest: Option<String>,
    pub from_api: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct UpstreamReleaseInfo {
    pub release: UpstreamRelease,
    pub cached: bool,
    pub checked_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamCacheEntry {
    pub tag: String,
    pub size: u64,
    pub modified_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamCacheInfo {
    pub entries: Vec<UpstreamCacheEntry>,
    pub total_bytes: u64,
}

/// Mirrors `state/upstream-install.json` (§6.4), written after a successful
/// `installer::install` from a merged payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpstreamInstalledState {
    pub schema_version: u8,
    pub upstream_tag: String,
    pub upstream_source: String,
    pub upstream_installed_at: u64,
    pub la_version: String,
    pub la_installed_at: u64,
    pub target: String,
    pub upstream_panel_exe: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct UpstreamProgress {
    pub tag: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub stage: String,
}

pub struct MergeInput {
    pub upstream_zip: PathBuf,
    pub la_payload: PathBuf,
    pub staging_root: PathBuf,
}

/// Diagnostic record of merge decisions, used by the unit tests and logs.
#[derive(Debug)]
pub struct MergeOutcome {
    pub root: PathBuf,
    /// `(top-level entry, level)` skipped by the manifest whitelist walk.
    pub skipped: Vec<(String, String)>,
}

pub struct CachedUpstream {
    pub release: UpstreamRelease,
    pub zip: PathBuf,
}

#[derive(Clone)]
struct CachedReleaseInfo {
    release: UpstreamRelease,
    checked_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MergeMarker {
    schema_version: u8,
    la_manifest_mtime_unix: u64,
    la_manifest_sha256: String,
    upstream_zip_size: u64,
    upstream_zip_mtime_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SourceRecord {
    schema_version: u8,
    source: String,
    saved_at: u64,
}

fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn modified_unix(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn merge_log(level: &str, event: &str, detail: &str) {
    #[cfg(not(test))]
    if let Ok(root) = app_storage::root() {
        logging::append(&root, level, event, detail);
    }
    #[cfg(test)]
    {
        let _ = (level, event, detail);
    }
}

pub fn upstream_root() -> Result<PathBuf> {
    Ok(online_update::update_root()?.join("upstream"))
}

fn validate_tag(tag: &str) -> Result<()> {
    let valid = !tag.is_empty()
        && tag.len() <= 64
        && tag != "."
        && tag != ".."
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(AppError::invalid(format!("Invalid upstream tag: {tag}")))
    }
}

pub fn cache_zip_path(tag: &str) -> Result<PathBuf> {
    validate_tag(tag)?;
    Ok(upstream_root()?.join(tag).join(ASSET_NAME))
}

pub fn merged_root(tag: &str) -> Result<PathBuf> {
    validate_tag(tag)?;
    Ok(online_update::update_root()?.join("merged").join(tag))
}

/// Canonical prefix of all merged staging roots, used to keep an active-payload
/// pointer from ever pointing a merge at its own output.
pub fn merged_root_prefix() -> Option<PathBuf> {
    fs::canonicalize(online_update::update_root().ok()?.join("merged")).ok()
}

fn write_source_record(directory: &Path, source: &str) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(&SourceRecord {
        schema_version: 1,
        source: source.into(),
        saved_at: unix_time(),
    })
    .map_err(|error| AppError::io(error.to_string()))?;
    atomic_fs::write_replace(&directory.join(SOURCE_FILE), &bytes)
        .map_err(AppError::transaction_io)
}

fn read_source(tag: &str) -> Option<String> {
    let record: SourceRecord = serde_json::from_slice(
        &fs::read(upstream_root().ok()?.join(tag).join(SOURCE_FILE)).ok()?,
    )
    .ok()?;
    (record.schema_version == 1).then_some(record.source)
}

fn cache_entries() -> Result<Vec<UpstreamCacheEntry>> {
    let root = upstream_root()?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(AppError::transaction_io)?
        .flatten()
    {
        let zip = entry.path().join(ASSET_NAME);
        let Ok(metadata) = fs::metadata(&zip) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        entries.push(UpstreamCacheEntry {
            tag: entry.file_name().to_string_lossy().into_owned(),
            size: metadata.len(),
            modified_at: modified_unix(&zip),
        });
    }
    entries.sort_by(|left, right| right.modified_at.cmp(&left.modified_at));
    Ok(entries)
}

pub fn cache_info() -> Result<UpstreamCacheInfo> {
    let entries = cache_entries()?;
    let total_bytes = entries.iter().map(|entry| entry.size).sum();
    Ok(UpstreamCacheInfo {
        entries,
        total_bytes,
    })
}

fn directory_size(path: &Path) -> u64 {
    let mut total = 0_u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(metadata) = fs::metadata(&path) {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}

/// Removes cached upstream zips (and their derived merged staging trees) and
/// returns the number of bytes freed.
pub fn cache_clear(tag: Option<String>) -> Result<usize> {
    let upstream = upstream_root()?;
    let merged = online_update::update_root()?.join("merged");
    let targets: Vec<PathBuf> = match &tag {
        Some(tag) => {
            validate_tag(tag)?;
            vec![upstream.join(tag), merged.join(tag)]
        }
        None => {
            let mut targets = Vec::new();
            for directory in [&upstream, &merged] {
                if let Ok(entries) = fs::read_dir(directory) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            targets.push(path);
                        }
                    }
                }
            }
            targets
        }
    };
    let mut freed = 0_u64;
    for target in targets {
        if !target.exists() {
            continue;
        }
        freed = freed.saturating_add(directory_size(&target));
        if fs::remove_dir_all(&target).is_err() {
            return Err(AppError::transaction_io(std::io::Error::other(format!(
                "Cannot remove the upstream cache directory ({})",
                target.display()
            ))));
        }
    }
    Ok(freed as usize)
}

/// Picks the cached upstream zip a merge should use: the installed tag recorded
/// in §6.4 state when it is still cached, otherwise the newest cache entry.
pub fn select_cached_upstream() -> Result<Option<(String, PathBuf)>> {
    let entries = cache_entries()?;
    if entries.is_empty() {
        return Ok(None);
    }
    if let Ok(Some(state)) = read_installed_state() {
        if validate_tag(&state.upstream_tag).is_ok()
            && entries.iter().any(|entry| entry.tag == state.upstream_tag)
        {
            let tag = state.upstream_tag.clone();
            return Ok(Some((tag.clone(), cache_zip_path(&tag)?)));
        }
    }
    let newest = entries[0].tag.clone();
    Ok(Some((newest.clone(), cache_zip_path(&newest)?)))
}

pub fn resolve_latest() -> Result<UpstreamRelease> {
    let client = online_update::client(REQUEST_TIMEOUT)?;
    let mut response = client
        .get(RELEASES_LATEST)
        .send()
        .map_err(|error| {
            AppError::update(format!("Cannot contact the GitHub release API: {error}"))
        })?;
    update_core::validate_https_github_url(response.url().as_str()).map_err(AppError::update)?;
    let mut body = Vec::new();
    response
        .by_ref()
        .take(RELEASE_BODY_LIMIT + 1)
        .read_to_end(&mut body)
        .map_err(|error| {
            AppError::update(format!("Cannot read the GitHub release response: {error}"))
        })?;
    if body.len() as u64 > RELEASE_BODY_LIMIT {
        return Err(AppError::update(
            "The GitHub release response exceeded the size limit",
        ));
    }
    let body = String::from_utf8_lossy(&body).into_owned();
    release_from_status(response.status(), &body)
}

/// Maps an HTTP status + body onto a release; 403/429 (anonymous rate limit,
/// 60/h/IP) fall back to the `latest/download` redirect without a digest.
pub(crate) fn release_from_status(
    status: reqwest::StatusCode,
    body: &str,
) -> Result<UpstreamRelease> {
    if status.as_u16() == 403 || status.as_u16() == 429 {
        return Ok(fallback_release());
    }
    if !status.is_success() {
        return Err(AppError::update(format!(
            "GitHub release request failed with HTTP {status}"
        )));
    }
    parse_release_document(body)
}

fn fallback_release() -> UpstreamRelease {
    UpstreamRelease {
        tag: "latest".into(),
        asset_url: LATEST_DOWNLOAD_FALLBACK.into(),
        digest: None,
        from_api: false,
    }
}

pub(crate) fn parse_release_document(body: &str) -> Result<UpstreamRelease> {
    #[derive(Deserialize)]
    struct ApiAsset {
        name: String,
        browser_download_url: String,
        #[serde(default)]
        digest: Option<String>,
    }
    #[derive(Deserialize)]
    struct ApiRelease {
        tag_name: String,
        #[serde(default)]
        assets: Vec<ApiAsset>,
    }
    let document: ApiRelease = serde_json::from_str(body)
        .map_err(|error| AppError::update(format!("Invalid GitHub release document: {error}")))?;
    if document.tag_name.trim().is_empty() {
        return Err(AppError::update(
            "The GitHub release document has no tag name",
        ));
    }
    let asset = document
        .assets
        .iter()
        .find(|asset| asset.name == ASSET_NAME)
        .ok_or_else(|| {
            AppError::update(format!(
                "The upstream release {tag} has no {ASSET_NAME} asset",
                tag = document.tag_name
            ))
        })?;
    update_core::validate_https_github_url(&asset.browser_download_url)
        .map_err(AppError::update)?;
    let digest = asset
        .digest
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if let Some(digest) = &digest {
        digest_hex(digest)?;
    }
    Ok(UpstreamRelease {
        tag: document.tag_name,
        asset_url: asset.browser_download_url.clone(),
        digest,
        from_api: true,
    })
}

pub fn release_info(force: bool) -> Result<UpstreamReleaseInfo> {
    let cell = RELEASE_INFO.get_or_init(|| Mutex::new(None));
    let mut guard = cell
        .lock()
        .map_err(|_| AppError::update("Upstream state lock is poisoned"))?;
    if !force {
        if let Some(cached) = guard.as_ref() {
            if unix_time().saturating_sub(cached.checked_at) < RELEASE_INFO_CACHE_SECONDS {
                let cached = cached.clone();
                drop(guard);
                return info_from(cached.release, Some(cached.checked_at));
            }
        }
    }
    let release = resolve_latest()?;
    let checked_at = unix_time();
    *guard = Some(CachedReleaseInfo {
        release: release.clone(),
        checked_at,
    });
    drop(guard);
    info_from(release, Some(checked_at))
}

fn info_from(release: UpstreamRelease, checked_at: Option<u64>) -> Result<UpstreamReleaseInfo> {
    let cached = cache_zip_path(&release.tag)
        .map(|path| zip_is_cached(&path))
        .unwrap_or(false);
    Ok(UpstreamReleaseInfo {
        release,
        cached,
        checked_at,
    })
}

fn zip_is_cached(zip: &Path) -> bool {
    fs::metadata(zip)
        .map(|metadata| metadata.is_file() && metadata.len() > 0 && metadata.len() <= MAX_ARCHIVE_BYTES)
        .unwrap_or(false)
}

fn emit_progress(
    app: Option<&AppHandle>,
    tag: &str,
    downloaded_bytes: u64,
    total_bytes: u64,
    speed_bps: u64,
    stage: &str,
) {
    let Some(app) = app else {
        return;
    };
    let _ = app.emit(
        PROGRESS_EVENT,
        UpstreamProgress {
            tag: tag.into(),
            downloaded_bytes,
            total_bytes,
            speed_bps,
            stage: stage.into(),
        },
    );
}

fn digest_hex(digest: &str) -> Result<&str> {
    let value = digest.trim();
    let hex = value.strip_prefix("sha256:").unwrap_or(value);
    update_core::validate_sha256(hex).map_err(AppError::update)?;
    Ok(hex)
}

pub(crate) fn verify_download_digest(path: &Path, digest: &str) -> Result<()> {
    let hex = digest_hex(digest)?;
    let actual = update_core::sha256_file(path).map_err(AppError::transaction_io)?;
    if !actual.eq_ignore_ascii_case(hex) {
        return Err(AppError::payload(
            "Upstream package digest mismatch: the downloaded file does not match the GitHub release digest",
        ));
    }
    Ok(())
}

/// Size check + optional digest verification, then the atomic move into place.
/// A digest mismatch deletes the temporary `.part` file so a retry can recover.
pub(crate) fn finalize_download(
    app: Option<&AppHandle>,
    part: &Path,
    destination: &Path,
    tag: &str,
    digest: Option<&str>,
) -> Result<()> {
    let size = fs::metadata(part)
        .map_err(|error| AppError::transaction_io(error))?
        .len();
    emit_progress(app, tag, size, size, 0, "verifying");
    if size == 0 || size > MAX_ARCHIVE_BYTES {
        let _ = fs::remove_file(part);
        return Err(AppError::payload(
            "The upstream package size is outside the allowed range",
        ));
    }
    if let Some(digest) = digest {
        if let Err(error) = verify_download_digest(part, digest) {
            let _ = fs::remove_file(part);
            return Err(error);
        }
    }
    atomic_fs::replace(part, destination).map_err(AppError::transaction_io)?;
    emit_progress(app, tag, size, size, 0, "done");
    Ok(())
}

fn release_digest_for(tag: &str) -> Option<String> {
    release_info(false)
        .ok()
        .filter(|info| info.release.tag == tag)
        .and_then(|info| info.release.digest)
}

/// Downloads the latest release into the cache (§6.1.2). `tag` must be the
/// latest tag (or `"latest"`); `url` is the asset or fallback URL.
pub fn download_start(app: &AppHandle, tag: &str, url: &str) -> Result<()> {
    validate_tag(tag)?;
    update_core::validate_https_github_url(url).map_err(AppError::update)?;
    let directory = upstream_root()?.join(tag);
    fs::create_dir_all(&directory).map_err(AppError::transaction_io)?;
    let destination = directory.join(ASSET_NAME);
    let part = directory.join(format!("{ASSET_NAME}.part"));
    let digest = release_digest_for(tag);

    if destination.is_file() {
        let size = fs::metadata(&destination)
            .map_err(AppError::transaction_io)?
            .len();
        let verified = size > 0
            && size <= MAX_ARCHIVE_BYTES
            && digest
                .as_deref()
                .map(|digest| verify_download_digest(&destination, digest).is_ok())
                .unwrap_or(true);
        if verified {
            emit_progress(Some(app), tag, size, size, 0, "done");
            return Ok(());
        }
        fs::remove_file(&destination).map_err(AppError::transaction_io)?;
    }

    let mut offset = if part.is_file() {
        fs::metadata(&part)
            .map_err(AppError::transaction_io)?
            .len()
    } else {
        0
    };
    let client = online_update::client(DOWNLOAD_TIMEOUT)?;
    let mut request = client.get(url);
    if offset > 0 {
        let range = format!("bytes={offset}-");
        request = request.header(reqwest::header::RANGE, reqwest::header::HeaderValue::from_str(&range).map_err(|_| AppError::update("Invalid resume range header"))?);
    }
    let mut response = request.send().map_err(|error| {
        AppError::update(format!("Cannot download the upstream package from GitHub: {error}"))
    })?;
    let status = response.status();
    if !status.is_success() {
        // Keep the .part file: the bytes already on disk remain resumable.
        return Err(AppError::update(format!(
            "Upstream package download failed with HTTP {status}"
        )));
    }
    update_core::validate_https_github_url(response.url().as_str()).map_err(AppError::update)?;
    let resuming = status.as_u16() == 206 && offset > 0;
    if !resuming && offset > 0 {
        // The server ignored the Range header; restart from zero.
        fs::remove_file(&part).map_err(AppError::transaction_io)?;
        offset = 0;
    }
    let total_hint = if resuming {
        response.content_length().map(|remaining| offset + remaining)
    } else {
        response.content_length()
    };
    if let Some(total) = total_hint {
        if total == 0 || total > MAX_ARCHIVE_BYTES {
            let _ = fs::remove_file(&part);
            return Err(AppError::payload(
                "The upstream package size is outside the allowed range",
            ));
        }
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create(true)
        .append(resuming)
        .truncate(!resuming)
        .open(&part)
        .map_err(AppError::transaction_io)?;
    let mut downloaded = if resuming { offset } else { 0 };
    let mut window_start = Instant::now();
    let mut window_bytes = 0_u64;
    let mut last_emit = Instant::now();
    let mut buffer = vec![0_u8; DOWNLOAD_BUFFER];
    loop {
        if online_update::cancelled() {
            return Err(AppError::update("Upstream download was cancelled"));
        }
        let count = response
            .read(&mut buffer)
            .map_err(|error| AppError::update(format!("Upstream download failed: {error}")))?;
        if count == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(count as u64);
        window_bytes = window_bytes.saturating_add(count as u64);
        if downloaded > MAX_ARCHIVE_BYTES {
            let _ = fs::remove_file(&part);
            return Err(AppError::payload(
                "The upstream package exceeded the size limit",
            ));
        }
        output
            .write_all(&buffer[..count])
            .map_err(AppError::transaction_io)?;
        if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
            let elapsed = window_start.elapsed().as_secs_f64();
            let speed_bps = if elapsed > 0.0 {
                (window_bytes as f64 / elapsed) as u64
            } else {
                0
            };
            emit_progress(
                Some(app),
                tag,
                downloaded,
                total_hint.unwrap_or(0),
                speed_bps,
                "downloading",
            );
            window_start = Instant::now();
            window_bytes = 0;
            last_emit = Instant::now();
        }
    }
    output.sync_all().map_err(AppError::transaction_io)?;
    finalize_download(Some(app), &part, &destination, tag, digest.as_deref())
}

/// Resolves the asset URL for `tag` and downloads it. Only the latest release
/// is downloadable; the returned release records the effective cache tag.
pub fn download_by_tag(app: &AppHandle, tag: &str) -> Result<UpstreamRelease> {
    validate_tag(tag)?;
    let info = release_info(false)?;
    let release = if tag == "latest" || tag == info.release.tag {
        info.release
    } else {
        return Err(AppError::invalid(
            "Only the latest upstream release can be downloaded",
        ));
    };
    download_start(app, &release.tag, &release.asset_url)?;
    write_source_record(
        &upstream_root()?.join(&release.tag),
        if release.from_api { "api" } else { "fallback" },
    )?;
    Ok(release)
}

/// Returns a cached upstream zip, downloading the latest release when the
/// cache does not already hold it (§6.5). Falls back to the newest cache entry
/// when the release API is unreachable.
pub fn ensure_cached(app: &AppHandle) -> Result<CachedUpstream> {
    if let Ok(info) = release_info(false) {
        let zip = cache_zip_path(&info.release.tag)?;
        if zip_is_cached(&zip) {
            return Ok(CachedUpstream {
                release: info.release,
                zip,
            });
        }
        let release = download_by_tag(app, &info.release.tag)?;
        let zip = cache_zip_path(&release.tag)?;
        return Ok(CachedUpstream { release, zip });
    }
    if let Some((tag, zip)) = select_cached_upstream()? {
        return Ok(CachedUpstream {
            release: UpstreamRelease {
                tag,
                asset_url: LATEST_DOWNLOAD_FALLBACK.into(),
                digest: None,
                from_api: false,
            },
            zip,
        });
    }
    Err(AppError::payload(
        "Upstream package is not available; download it in Installation Management",
    ))
}

pub fn import_local(app: &AppHandle, source: &str) -> Result<UpstreamRelease> {
    let _ = app;
    import_local_at(Path::new(source), &upstream_root()?)
}

pub(crate) fn import_local_at(source: &Path, cache_root: &Path) -> Result<UpstreamRelease> {
    let metadata = fs::metadata(source)
        .map_err(|_| AppError::invalid("The selected upstream ZIP does not exist"))?;
    if !metadata.is_file() {
        return Err(AppError::invalid("The selected upstream ZIP is not a file"));
    }
    if metadata.len() == 0 || metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(AppError::invalid(
            "The selected upstream ZIP must be between 1 byte and 400 MB",
        ));
    }
    if source
        .extension()
        .and_then(|value| value.to_str())
        .is_none_or(|value| !value.eq_ignore_ascii_case("zip"))
    {
        return Err(AppError::invalid("The selected upstream file must be a .zip"));
    }
    let names = zip_entry_names(source)?;
    if zip_payload_prefix(&names).is_none() {
        return Err(AppError::payload(
            "The selected ZIP is not a CS2BotImprover package: it has no addons/ and cfg/ payload",
        ));
    }
    let tag = names
        .iter()
        .filter(|name| !name.contains('/'))
        .find_map(|name| parse_panel_tag(name))
        .unwrap_or_else(|| "local".into());
    validate_tag(&tag)?;
    let directory = cache_root.join(&tag);
    fs::create_dir_all(&directory).map_err(AppError::transaction_io)?;
    let destination = directory.join(ASSET_NAME);
    let temporary = directory.join(format!("{ASSET_NAME}.import"));
    fs::copy(source, &temporary).map_err(AppError::transaction_io)?;
    if let Err(error) = atomic_fs::replace(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(AppError::transaction_io(error));
    }
    write_source_record(&directory, "manual")?;
    merge_log(
        "INFO",
        "upstream.imported",
        &format!("tag={tag}, source={}", source.display()),
    );
    Ok(UpstreamRelease {
        tag,
        asset_url: source.to_string_lossy().into_owned(),
        digest: None,
        from_api: false,
    })
}

fn zip_entry_names(path: &Path) -> Result<Vec<String>> {
    let file = File::open(path).map_err(AppError::transaction_io)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| AppError::payload(format!("Invalid upstream ZIP: {error}")))?;
    let mut names = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| AppError::payload(format!("Cannot read upstream ZIP entry: {error}")))?;
        names.push(entry.name().to_string());
    }
    Ok(names)
}

/// Mirrors the `package.ps1` payload probe: the zip must carry `addons/` and
/// `cfg/` either at the root or under a single top-level directory.
fn zip_payload_prefix(names: &[String]) -> Option<String> {
    let has = |prefix: String| names.iter().any(|name| name.starts_with(&prefix));
    if has("addons/".into()) && has("cfg/".into()) {
        return Some(String::new());
    }
    let mut tops = BTreeSet::new();
    for name in names {
        if let Some((top, _)) = name.split_once('/') {
            tops.insert(top.to_string());
        }
    }
    for top in tops {
        let prefix = format!("{top}/");
        if has(format!("{prefix}addons/")) && has(format!("{prefix}cfg/")) {
            return Some(prefix);
        }
    }
    None
}

/// `"Panel v1.4.5.exe"` → `Some("v1.4.5")`; anything else → `None` (import then
/// defaults to the `"local"` tag).
pub fn parse_panel_tag(name: &str) -> Option<String> {
    let rest = name.strip_prefix("Panel ")?.strip_suffix(".exe")?;
    let version = rest.strip_prefix('v')?;
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        || !version.bytes().any(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some(rest.to_string())
}

fn is_panel_exe_name(name: &str) -> bool {
    name.starts_with("Panel ") && name.ends_with(".exe") && !name.contains('/')
}

pub fn merge(input: &MergeInput) -> Result<PathBuf> {
    let outcome = merge_with_options(input, &exe_parent_dir()?)?;
    merge_log(
        "INFO",
        "upstream.merge.skip_summary",
        &outcome
            .skipped
            .iter()
            .map(|(entry, level)| format!("{entry}:{level}"))
            .collect::<Vec<_>>()
            .join(","),
    );
    Ok(outcome.root)
}

fn exe_parent_dir() -> Result<PathBuf> {
    let executable =
        std::env::current_exe().map_err(|error| AppError::payload(error.to_string()))?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| AppError::payload("Panel executable has no parent directory"))
}

pub(crate) fn merge_with_options(input: &MergeInput, exe_dir: &Path) -> Result<MergeOutcome> {
    let _merge_lock = MERGE_MUTEX
        .lock()
        .map_err(|_| AppError::payload("Upstream merge lock is poisoned"))?;
    if input.la_payload == input.staging_root {
        return Err(AppError::payload(
            "The merged payload root must not be used as the LA payload source",
        ));
    }
    let la_manifest_path = input.la_payload.join(installer::MANIFEST_FILE);
    let la_manifest: installer::PayloadManifest = serde_json::from_slice(
        &fs::read(&la_manifest_path).map_err(|_| {
            AppError::payload(format!(
                "The LA payload manifest is missing ({})",
                la_manifest_path.display()
            ))
        })?,
    )
    .map_err(|error| AppError::payload(format!("Invalid LA payload manifest: {error}")))?;
    if la_manifest.schema_version != 1 || la_manifest.entries.is_empty() {
        return Err(AppError::payload("Unsupported or empty LA payload manifest"));
    }

    // Step 1: start from an empty staging tree.
    online_update::clear_directory(&input.staging_root)?;

    // Step 2: extract + blocking payload probe (R5).
    let zip_file = File::open(&input.upstream_zip).map_err(|error| {
        AppError::payload(format!(
            "Cannot open the cached upstream package ({}): {error}",
            input.upstream_zip.display()
        ))
    })?;
    update_core::extract_zip_safely(zip_file, &input.staging_root).map_err(AppError::payload)?;
    let payload_dir = locate_payload_dir(&input.staging_root).ok_or_else(|| {
        AppError::payload("Could not locate the upstream game/csgo payload")
    })?;
    if payload_dir != input.staging_root {
        flatten_payload_dir(&input.staging_root, &payload_dir)?;
    }

    // Step 3: post-extract filters (decisions #4 and #8).
    let mut skipped = Vec::new();
    apply_staging_filters(&input.staging_root, exe_dir)?;

    // Step 4: LA overlay — LA files win same-name collisions.
    overlay_la_payload(&input.la_payload, &input.staging_root)?;

    // Step 5: merged manifest.
    let manifest =
        generate_merged_manifest(&la_manifest, &input.staging_root, &mut skipped)?;
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| AppError::io(error.to_string()))?;
    atomic_fs::write_replace(&input.staging_root.join(installer::MANIFEST_FILE), &manifest_bytes)
        .map_err(AppError::transaction_io)?;
    for (entry, level) in &skipped {
        merge_log(level, "upstream.merge.skipped", &format!("entry={entry}"));
    }
    merge_log(
        "INFO",
        "upstream.merge.completed",
        &format!(
            "entries={}, skipped={}, staging={}",
            manifest.entries.len(),
            skipped.len(),
            input.staging_root.display()
        ),
    );

    // Step 6: cache marker keyed on the LA manifest + upstream zip (assumption 6).
    let marker = MergeMarker {
        schema_version: 1,
        la_manifest_mtime_unix: modified_unix(&la_manifest_path),
        la_manifest_sha256: update_core::sha256_file(&la_manifest_path)
            .map_err(AppError::transaction_io)?,
        upstream_zip_size: fs::metadata(&input.upstream_zip)
            .map_err(AppError::transaction_io)?
            .len(),
        upstream_zip_mtime_unix: modified_unix(&input.upstream_zip),
    };
    let marker_bytes =
        serde_json::to_vec_pretty(&marker).map_err(|error| AppError::io(error.to_string()))?;
    atomic_fs::write_replace(&input.staging_root.join(MERGE_STATE_FILE), &marker_bytes)
        .map_err(AppError::transaction_io)?;

    Ok(MergeOutcome {
        root: input.staging_root.clone(),
        skipped,
    })
}

/// A merge is reusable while the staging manifest, the LA manifest key, and the
/// upstream zip are all unchanged (§6.3 / assumption 6).
pub fn merged_is_current(la_payload: &Path, staging_root: &Path, upstream_zip: &Path) -> bool {
    if !staging_root.join(installer::MANIFEST_FILE).is_file() {
        return false;
    }
    let Ok(marker_bytes) = fs::read(staging_root.join(MERGE_STATE_FILE)) else {
        return false;
    };
    let Ok(marker) = serde_json::from_slice::<MergeMarker>(&marker_bytes) else {
        return false;
    };
    if marker.schema_version != 1 {
        return false;
    }
    let la_manifest = la_payload.join(installer::MANIFEST_FILE);
    if !la_manifest.is_file()
        || modified_unix(&la_manifest) != marker.la_manifest_mtime_unix
        || update_core::sha256_file(&la_manifest).ok()
            != Some(marker.la_manifest_sha256.clone())
    {
        return false;
    }
    let Ok(zip_metadata) = fs::metadata(upstream_zip) else {
        return false;
    };
    zip_metadata.len() == marker.upstream_zip_size
        && modified_unix(upstream_zip) == marker.upstream_zip_mtime_unix
}

fn has_payload_layout(dir: &Path) -> bool {
    dir.join("addons").is_dir() && dir.join("cfg").is_dir()
}

/// Staging root first, then breadth-first over sorted directories.
fn locate_payload_dir(staging_root: &Path) -> Option<PathBuf> {
    let mut queue = VecDeque::new();
    queue.push_back(staging_root.to_path_buf());
    while let Some(dir) = queue.pop_front() {
        if has_payload_layout(&dir) {
            return Some(dir);
        }
        let mut children = Vec::new();
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    children.push(path);
                }
            }
        }
        children.sort();
        queue.extend(children);
    }
    None
}

/// Moves the wrapped payload contents up to the staging root; a same-name
/// collision aborts before anything moves (no silent merging).
fn flatten_payload_dir(staging_root: &Path, payload_dir: &Path) -> Result<()> {
    let mut names: Vec<String> = fs::read_dir(payload_dir)
        .map_err(AppError::transaction_io)?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in &names {
        if staging_root.join(name).exists() {
            return Err(AppError::payload(format!(
                "Upstream payload directory contains a conflicting top-level entry: {name}"
            )));
        }
    }
    for name in &names {
        fs::rename(payload_dir.join(name), staging_root.join(name))
            .map_err(AppError::transaction_io)?;
    }
    if let Some(first) = payload_dir
        .strip_prefix(staging_root)
        .ok()
        .and_then(|relative| relative.components().next())
    {
        let wrapper = staging_root.join(first);
        let _ = fs::remove_dir_all(&wrapper);
    }
    Ok(())
}

/// Skips the root `gameinfo.gi` (decision #4) and relocates the upstream
/// `Panel v*.exe` next to the Panel executable (decision #8). `backup/` files
/// and `addons/metamod/RayTrace.vdf` stay in staging untouched (assumption 3).
fn apply_staging_filters(staging_root: &Path, exe_dir: &Path) -> Result<()> {
    let gameinfo = staging_root.join("gameinfo.gi");
    if gameinfo.is_file() {
        fs::remove_file(&gameinfo).map_err(AppError::transaction_io)?;
        merge_log("INFO", "upstream.merge.skip_root_gameinfo", &gameinfo.display().to_string());
    }
    let mut names: Vec<String> = fs::read_dir(staging_root)
        .map_err(AppError::transaction_io)?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        if !is_panel_exe_name(&name) {
            continue;
        }
        let source = staging_root.join(&name);
        let destination = exe_dir.join(&name);
        if destination.exists() {
            let _ = fs::remove_file(&destination);
        }
        if fs::rename(&source, &destination).is_err() {
            // Cross-volume rename fallback.
            fs::copy(&source, &destination).map_err(AppError::transaction_io)?;
            fs::remove_file(&source).map_err(AppError::transaction_io)?;
        }
        merge_log(
            "INFO",
            "upstream.merge.relocated_panel_exe",
            &format!("name={name}, destination={}", destination.display()),
        );
    }
    Ok(())
}

fn overlay_la_payload(la_payload: &Path, staging_root: &Path) -> Result<()> {
    for top in ["addons", "cfg", "overrides"] {
        let source_root = la_payload.join(top);
        if !source_root.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        walk_files(&source_root, &mut files).map_err(AppError::transaction_io)?;
        for file in files {
            let relative = file
                .strip_prefix(&source_root)
                .map_err(|_| AppError::payload("Overlay path escaped the LA payload"))?;
            let destination = staging_root.join(top).join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(AppError::transaction_io)?;
            }
            fs::copy(&file, &destination).map_err(AppError::transaction_io)?;
        }
    }
    Ok(())
}

fn walk_files(root: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(root)?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name().to_os_string());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk_files(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

/// Component mapping copied from `package.ps1` (minus the removed RayTrace
/// branch). Ownership is always `shared` for upstream entries; only
/// `overrides/botprofile.vpk` keeps user edits across repairs.
pub fn upstream_component_for(relative: &str) -> String {
    let segments: Vec<&str> = relative.split('/').collect();
    if segments.len() >= 3
        && segments[0] == "addons"
        && segments[1] == "counterstrikesharp"
        && segments[2] == "plugins"
    {
        if let Some(name) = segments.get(3) {
            return (*name).to_string();
        }
    }
    if segments.first() == Some(&"addons") && segments.get(1) == Some(&"BotHider") {
        return "BotHider".into();
    }
    if segments.first() == Some(&"cfg") {
        return "configuration".into();
    }
    if segments.first() == Some(&"overrides") {
        return "overrides".into();
    }
    "runtime".into()
}

pub fn upstream_restore_policy(relative: &str) -> String {
    if relative == "overrides/botprofile.vpk" {
        "preserve-config".into()
    } else {
        "restore".into()
    }
}

fn generate_merged_manifest(
    la_manifest: &installer::PayloadManifest,
    staging_root: &Path,
    skipped: &mut Vec<(String, String)>,
) -> Result<installer::PayloadManifest> {
    let mut la_paths = BTreeSet::new();
    for entry in &la_manifest.entries {
        la_paths.insert(entry.path.clone());
    }
    let mut upstream_entries = Vec::new();
    let mut top_names: Vec<String> = fs::read_dir(staging_root)
        .map_err(AppError::transaction_io)?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    top_names.sort();
    for name in top_names {
        let path = staging_root.join(&name);
        if matches!(name.as_str(), "addons" | "cfg" | "overrides") && path.is_dir() {
            let mut files = Vec::new();
            walk_files(&path, &mut files).map_err(AppError::transaction_io)?;
            for file in files {
                let relative = format!(
                    "{}/{}",
                    name,
                    file.strip_prefix(&path)
                        .map_err(|_| AppError::payload("Manifest path escaped the staging root"))?
                        .to_string_lossy()
                        .replace('\\', "/")
                );
                if la_paths.contains(&relative) {
                    continue;
                }
                let metadata = fs::metadata(&file).map_err(AppError::transaction_io)?;
                upstream_entries.push(installer::PayloadEntry {
                    path: relative,
                    size: metadata.len(),
                    sha256: update_core::sha256_file(&file)
                        .map_err(AppError::transaction_io)?,
                    component: upstream_component_for(
                        &file
                            .strip_prefix(staging_root)
                            .map(|path| path.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_default(),
                    ),
                    ownership: "shared".into(),
                    restore_policy: upstream_restore_policy(
                        &file
                            .strip_prefix(staging_root)
                            .map(|path| path.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_default(),
                    ),
                });
            }
            continue;
        }
        if name == "backup" {
            // Known upstream top-level entry: kept in staging, excluded from
            // the manifest on purpose (decision #4 revision).
            skipped.push((name, "INFO".into()));
            continue;
        }
        if name == installer::MANIFEST_FILE || name == MERGE_STATE_FILE {
            continue;
        }
        skipped.push((name, "WARN".into()));
    }
    let mut entries = la_manifest.entries.clone();
    entries.extend(upstream_entries);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(installer::PayloadManifest {
        schema_version: 1,
        package_version: la_manifest.package_version.clone(),
        entries,
    })
}

pub fn read_installed_state() -> Result<Option<UpstreamInstalledState>> {
    let path = app_storage::root()?.join(INSTALLED_STATE_FILE);
    let Ok(bytes) = fs::read(&path) else {
        return Ok(None);
    };
    let state: UpstreamInstalledState = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::io(format!("Invalid upstream install state: {error}")))?;
    if state.schema_version != 1 {
        return Err(AppError::io("Unsupported upstream install state schema"));
    }
    Ok(Some(state))
}

/// Called after `installer::install` succeeds from `merged_root`.
pub fn write_installed_state(
    csgo: &Path,
    merged_root: &Path,
) -> Result<UpstreamInstalledState> {
    let tag = merged_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| AppError::payload("The merged payload root has no tag directory"))?;
    validate_tag(&tag)?;
    let source = read_source(&tag).unwrap_or_else(|| "manual".into());
    let manifest: installer::PayloadManifest = serde_json::from_slice(
        &fs::read(merged_root.join(installer::MANIFEST_FILE)).map_err(AppError::transaction_io)?,
    )
    .map_err(|error| AppError::payload(format!("Invalid merged payload manifest: {error}")))?;
    let now = unix_time();
    let state = UpstreamInstalledState {
        schema_version: 1,
        upstream_tag: tag,
        upstream_source: source,
        upstream_installed_at: now,
        la_version: manifest.package_version,
        la_installed_at: now,
        target: csgo.to_string_lossy().into_owned(),
        upstream_panel_exe: locate_relocated_panel_exe()
            .map(|path| path.to_string_lossy().into_owned()),
    };
    let bytes = serde_json::to_vec_pretty(&state).map_err(|error| AppError::io(error.to_string()))?;
    atomic_fs::write_replace(&app_storage::root()?.join(INSTALLED_STATE_FILE), &bytes)
        .map_err(AppError::transaction_io)?;
    Ok(state)
}

/// The relocated `Panel v*.exe` beside the running Panel executable (R6).
pub fn locate_relocated_panel_exe() -> Option<PathBuf> {
    let exe_dir = exe_parent_dir().ok()?;
    let mut names: Vec<String> = fs::read_dir(&exe_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| is_panel_exe_name(name))
        .collect();
    names.sort();
    names.into_iter().map(|name| exe_dir.join(name)).next()
}

pub fn launch_panel(raw: &str) -> Result<()> {
    let path = validate_upstream_panel_path(raw)?;
    let mut command = Command::new(&path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map_err(|error| AppError::launch(format!("Cannot start the upstream Panel: {error}")))?;
    Ok(())
}

pub fn validate_upstream_panel_path(raw: &str) -> Result<PathBuf> {
    let current = std::env::current_exe().map_err(AppError::transaction_io)?;
    validate_upstream_panel_path_in(raw, &current)
}

fn validate_upstream_panel_path_in(raw: &str, current_exe: &Path) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::invalid("The upstream Panel path has no file name"))?;
    if !is_panel_exe_name(name) {
        return Err(AppError::invalid(
            "Only an upstream \"Panel v*.exe\" can be launched",
        ));
    }
    if !path.is_file() {
        return Err(AppError::invalid(
            "The upstream Panel executable does not exist",
        ));
    }
    let canonical = fs::canonicalize(&path)
        .map_err(|error| AppError::invalid(format!("Cannot resolve the upstream Panel path: {error}")))?;
    let parent = current_exe
        .parent()
        .ok_or_else(|| AppError::payload("Panel executable has no parent directory"))?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        AppError::invalid(format!("Cannot resolve the Panel directory: {error}"))
    })?;
    if canonical.parent() != Some(canonical_parent.as_path()) {
        return Err(AppError::invalid(
            "The upstream Panel executable must stay beside LocalArena.exe",
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn test_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cs2bi-upstream-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn payload_entry(
        rel: &str,
        file: &Path,
        component: &str,
        ownership: &str,
        policy: &str,
    ) -> installer::PayloadEntry {
        installer::PayloadEntry {
            path: rel.into(),
            size: fs::metadata(file).unwrap().len(),
            sha256: update_core::sha256_file(file).unwrap(),
            component: component.into(),
            ownership: ownership.into(),
            restore_policy: policy.into(),
        }
    }

    fn build_la_payload(dir: &Path) -> installer::PayloadManifest {
        let knife = dir.join("addons/counterstrikesharp/plugins/PlayerKnifeCustomizer/PlayerKnifeCustomizer.dll");
        fs::create_dir_all(knife.parent().unwrap()).unwrap();
        fs::write(&knife, b"la-plugin").unwrap();
        let cfg = dir.join("cfg/my_bot_normal_config.cfg");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        fs::write(&cfg, b"la-cfg").unwrap();
        let manifest = installer::PayloadManifest {
            schema_version: 1,
            package_version: "1.4.3.3".into(),
            entries: vec![
                payload_entry(
                    "addons/counterstrikesharp/plugins/PlayerKnifeCustomizer/PlayerKnifeCustomizer.dll",
                    &knife,
                    "PlayerKnifeCustomizer",
                    "plus",
                    "restore",
                ),
                payload_entry(
                    "cfg/my_bot_normal_config.cfg",
                    &cfg,
                    "configuration",
                    "plus",
                    "preserve-config",
                ),
            ],
        };
        fs::write(
            dir.join(installer::MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        manifest
    }

    fn standard_upstream_zip(path: &Path) {
        write_zip(
            path,
            &[
                ("Panel v1.4.5.exe", b"panel-exe"),
                ("gameinfo.gi", b"root-gameinfo"),
                (
                    "addons/metamod/RayTrace.vdf",
                    b"github.com/ed0ard/CS2-Bot-Improver",
                ),
                ("addons/counterstrikesharp/plugins/BotAI/BotAI.dll", b"botai"),
                ("addons/BotHider/readme.txt", b"bothider"),
                ("cfg/gamemode_deathmatch.cfg", b"gamemode"),
                ("overrides/botprofile.vpk", b"botprofile"),
                ("backup/Online/gameinfo.gi", b"backup-gameinfo"),
            ],
        );
    }

    struct MergeFixture {
        #[allow(dead_code)]
        base: PathBuf,
        zip: PathBuf,
        la: PathBuf,
        la_manifest: installer::PayloadManifest,
        staging: PathBuf,
        exe_dir: PathBuf,
    }

    fn merge_setup(label: &str) -> MergeFixture {
        let base = test_root(label);
        let zip = base.join("upstream.zip");
        standard_upstream_zip(&zip);
        let la = base.join("la");
        fs::create_dir_all(&la).unwrap();
        let la_manifest = build_la_payload(&la);
        let staging = base.join("merged").join("v1.4.5");
        let exe_dir = base.join("panel-dir");
        fs::create_dir_all(&exe_dir).unwrap();
        MergeFixture {
            base,
            zip,
            la,
            la_manifest,
            staging,
            exe_dir,
        }
    }

    fn run_merge(fixture: &MergeFixture) -> MergeOutcome {
        merge_with_options(
            &MergeInput {
                upstream_zip: fixture.zip.clone(),
                la_payload: fixture.la.clone(),
                staging_root: fixture.staging.clone(),
            },
            &fixture.exe_dir,
        )
        .unwrap()
    }

    fn read_merged_manifest(staging: &Path) -> installer::PayloadManifest {
        serde_json::from_slice(&fs::read(staging.join(installer::MANIFEST_FILE)).unwrap())
            .unwrap()
    }

    fn find_entry<'a>(
        manifest: &'a installer::PayloadManifest,
        path: &str,
    ) -> &'a installer::PayloadEntry {
        manifest
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .unwrap_or_else(|| panic!("manifest has no entry for {path}"))
    }

    #[test]
    fn merge_manifest_inherits_la_entries_verbatim() {
        let fixture = merge_setup("inherit");
        let outcome = run_merge(&fixture);
        let merged = read_merged_manifest(&fixture.staging);

        assert_eq!(merged.schema_version, 1);
        assert_eq!(merged.package_version, fixture.la_manifest.package_version);
        for entry in &fixture.la_manifest.entries {
            let inherited = find_entry(&merged, &entry.path);
            assert_eq!(
                serde_json::to_value(inherited).unwrap(),
                serde_json::to_value(entry).unwrap(),
                "LA entry {} must be inherited verbatim",
                entry.path
            );
        }
        assert!(outcome
            .skipped
            .iter()
            .any(|(name, level)| name == "backup" && level == "INFO"));
        assert!(merged
            .entries
            .iter()
            .any(|entry| entry.path.starts_with("addons/")));
    }

    #[test]
    fn merge_manifest_derives_upstream_component_ownership_and_policy() {
        let fixture = merge_setup("derive");
        run_merge(&fixture);
        let merged = read_merged_manifest(&fixture.staging);

        let expectations = [
            (
                "addons/counterstrikesharp/plugins/BotAI/BotAI.dll",
                "BotAI",
                "restore",
            ),
            ("addons/BotHider/readme.txt", "BotHider", "restore"),
            ("cfg/gamemode_deathmatch.cfg", "configuration", "restore"),
            (
                "overrides/botprofile.vpk",
                "overrides",
                "preserve-config",
            ),
            ("addons/metamod/RayTrace.vdf", "runtime", "restore"),
        ];
        for (path, component, policy) in expectations {
            let entry = find_entry(&merged, path);
            assert_eq!(entry.component, component, "component for {path}");
            assert_eq!(entry.ownership, "shared", "ownership for {path}");
            assert_eq!(entry.restore_policy, policy, "policy for {path}");
        }
    }

    #[test]
    fn merge_manifest_computes_sha256_and_size_at_merge_time() {
        let fixture = merge_setup("hash");
        run_merge(&fixture);
        let merged = read_merged_manifest(&fixture.staging);

        let entry = find_entry(&merged, "addons/counterstrikesharp/plugins/BotAI/BotAI.dll");
        let file = fixture.staging.join(&entry.path);
        assert_eq!(entry.size, fs::metadata(&file).unwrap().len());
        assert_eq!(entry.sha256, update_core::sha256_file(&file).unwrap());
    }

    #[test]
    fn merge_filter_skips_root_gameinfo_and_keeps_backups_out_of_manifest() {
        let fixture = merge_setup("gameinfo");
        run_merge(&fixture);
        let merged = read_merged_manifest(&fixture.staging);

        assert!(!fixture.staging.join("gameinfo.gi").exists());
        assert!(merged.entries.iter().all(|entry| entry.path != "gameinfo.gi"));
        assert!(
            fixture.staging.join("backup/Online/gameinfo.gi").is_file(),
            "backup gameinfo must stay in staging"
        );
        assert!(
            merged
                .entries
                .iter()
                .all(|entry| !entry.path.starts_with("backup/")),
            "backup files must not enter the merged manifest"
        );
    }

    #[test]
    fn merge_filter_relocates_the_upstream_panel_exe() {
        let fixture = merge_setup("panel-exe");
        run_merge(&fixture);

        let relocated = fixture.exe_dir.join("Panel v1.4.5.exe");
        assert_eq!(fs::read(&relocated).unwrap(), b"panel-exe");
        assert!(!fixture.staging.join("Panel v1.4.5.exe").exists());
        assert!(
            read_merged_manifest(&fixture.staging)
                .entries
                .iter()
                .all(|entry| !entry.path.ends_with(".exe"))
        );
    }

    #[test]
    fn merge_filter_keeps_raytrace_vdf_installed() {
        let fixture = merge_setup("raytrace");
        run_merge(&fixture);
        let merged = read_merged_manifest(&fixture.staging);

        assert!(fixture.staging.join("addons/metamod/RayTrace.vdf").is_file());
        assert!(merged
            .entries
            .iter()
            .any(|entry| entry.path == "addons/metamod/RayTrace.vdf"));
    }

    #[test]
    fn merge_filter_warns_and_skips_unknown_top_level_entries() {
        let base = test_root("warn-skip");
        let zip = base.join("upstream.zip");
        write_zip(
            &zip,
            &[
                ("addons/x.dll", b"x"),
                ("cfg/y.cfg", b"y"),
                ("scripts/tool.exe", b"tool"),
            ],
        );
        let la = base.join("la");
        fs::create_dir_all(&la).unwrap();
        build_la_payload(&la);
        let staging = base.join("merged/local");
        let exe_dir = base.join("panel");
        fs::create_dir_all(&exe_dir).unwrap();

        let outcome = merge_with_options(
            &MergeInput {
                upstream_zip: zip,
                la_payload: la,
                staging_root: staging.clone(),
            },
            &exe_dir,
        )
        .unwrap();

        assert!(outcome
            .skipped
            .iter()
            .any(|(name, level)| name == "scripts" && level == "WARN"));
        assert!(staging.join("scripts/tool.exe").is_file());
        let merged = read_merged_manifest(&staging);
        assert!(merged
            .entries
            .iter()
            .all(|entry| !entry.path.starts_with("scripts/")));
    }

    #[test]
    fn payload_probe_flattens_a_wrapped_upstream_payload() {
        let base = test_root("wrapped");
        let zip = base.join("upstream.zip");
        write_zip(
            &zip,
            &[
                ("CS2BotImprover/addons/x.dll", b"x"),
                ("CS2BotImprover/cfg/y.cfg", b"y"),
                ("CS2BotImprover/gameinfo.gi", b"wrapped-gameinfo"),
            ],
        );
        let la = base.join("la");
        fs::create_dir_all(&la).unwrap();
        build_la_payload(&la);
        let staging = base.join("merged/v1.4.5");
        let exe_dir = base.join("panel");
        fs::create_dir_all(&exe_dir).unwrap();

        merge_with_options(
            &MergeInput {
                upstream_zip: zip,
                la_payload: la,
                staging_root: staging.clone(),
            },
            &exe_dir,
        )
        .unwrap();

        assert!(staging.join("addons/x.dll").is_file());
        assert!(staging.join("cfg/y.cfg").is_file());
        assert!(!staging.join("gameinfo.gi").exists());
        assert!(!staging.join("CS2BotImprover").exists());
        let merged = read_merged_manifest(&staging);
        assert!(merged.entries.iter().any(|entry| entry.path == "addons/x.dll"));
        assert!(merged.entries.iter().any(|entry| entry.path == "cfg/y.cfg"));
    }

    #[test]
    fn payload_probe_blocks_a_package_without_addons_and_cfg() {
        let base = test_root("no-payload");
        let zip = base.join("upstream.zip");
        write_zip(&zip, &[("addons/only.txt", b"x"), ("readme.txt", b"r")]);
        let la = base.join("la");
        fs::create_dir_all(&la).unwrap();
        build_la_payload(&la);
        let staging = base.join("merged/local");

        let error = merge_with_options(
            &MergeInput {
                upstream_zip: zip,
                la_payload: la,
                staging_root: staging,
            },
            &base,
        )
        .unwrap_err();
        assert_eq!(error.detail, "Could not locate the upstream game/csgo payload");
    }

    #[test]
    fn payload_probe_blocks_a_cfg_only_package() {
        let base = test_root("cfg-only");
        let zip = base.join("upstream.zip");
        write_zip(&zip, &[("cfg/only.cfg", b"x")]);
        let la = base.join("la");
        fs::create_dir_all(&la).unwrap();
        build_la_payload(&la);
        let staging = base.join("merged/local");

        let error = merge_with_options(
            &MergeInput {
                upstream_zip: zip,
                la_payload: la,
                staging_root: staging,
            },
            &base,
        )
        .unwrap_err();
        assert_eq!(error.detail, "Could not locate the upstream game/csgo payload");
    }

    #[test]
    fn zip_slip_rejects_parent_directory_entries() {
        let base = test_root("zip-slip-parent");
        let zip = base.join("upstream.zip");
        write_zip(&zip, &[("../evil.txt", b"evil"), ("addons/x.dll", b"x"), ("cfg/y.cfg", b"y")]);
        let la = base.join("la");
        fs::create_dir_all(&la).unwrap();
        build_la_payload(&la);
        let staging = base.join("merged/local");

        assert!(merge_with_options(
            &MergeInput {
                upstream_zip: zip,
                la_payload: la,
                staging_root: staging,
            },
            &base,
        )
        .is_err());
        assert!(!base.join("evil.txt").exists());
    }

    #[test]
    fn zip_slip_rejects_windows_drive_entries() {
        let base = test_root("zip-slip-drive");
        let zip = base.join("upstream.zip");
        write_zip(&zip, &[("C:\\evil.txt", b"evil"), ("addons/x.dll", b"x"), ("cfg/y.cfg", b"y")]);
        let la = base.join("la");
        fs::create_dir_all(&la).unwrap();
        build_la_payload(&la);
        let staging = base.join("merged/local");

        assert!(merge_with_options(
            &MergeInput {
                upstream_zip: zip,
                la_payload: la,
                staging_root: staging,
            },
            &base,
        )
        .is_err());
    }

    #[test]
    fn resolve_parses_the_github_release_document() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let body = serde_json::json!({
            "tag_name": "v1.4.5",
            "assets": [
                {
                    "name": "CS2BotImprover.zip",
                    "browser_download_url":
                        "https://github.com/ed0ard/CS2-Bot-Improver/releases/download/v1.4.5/CS2BotImprover.zip",
                    "digest": digest,
                },
                {
                    "name": "other.zip",
                    "browser_download_url":
                        "https://github.com/ed0ard/CS2-Bot-Improver/releases/download/v1.4.5/other.zip",
                },
            ],
        })
        .to_string();

        let release = parse_release_document(&body).unwrap();
        assert_eq!(release.tag, "v1.4.5");
        assert_eq!(
            release.asset_url,
            "https://github.com/ed0ard/CS2-Bot-Improver/releases/download/v1.4.5/CS2BotImprover.zip"
        );
        assert_eq!(release.digest.as_deref(), Some(digest.as_str()));
        assert!(release.from_api);
    }

    #[test]
    fn resolve_falls_back_on_rate_limit_statuses() {
        for status in [
            reqwest::StatusCode::FORBIDDEN,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
        ] {
            let release = release_from_status(status, "").unwrap();
            assert_eq!(release.tag, "latest");
            assert_eq!(release.asset_url, LATEST_DOWNLOAD_FALLBACK);
            assert_eq!(release.digest, None);
            assert!(!release.from_api);
        }
    }

    #[test]
    fn resolve_errors_when_the_asset_is_missing() {
        let body = serde_json::json!({
            "tag_name": "v1.4.5",
            "assets": [{
                "name": "something-else.zip",
                "browser_download_url": "https://github.com/ed0ard/CS2-Bot-Improver/releases/download/v1.4.5/something-else.zip",
            }],
        })
        .to_string();
        assert!(parse_release_document(&body).is_err());
        assert!(parse_release_document("not json").is_err());
    }

    #[test]
    fn digest_match_passes_and_moves_the_part_file() {
        let base = test_root("digest-ok");
        let part = base.join("CS2BotImprover.zip.part");
        let destination = base.join("CS2BotImprover.zip");
        fs::write(&part, b"payload").unwrap();
        let digest = format!("sha256:{}", update_core::sha256_file(&part).unwrap());

        finalize_download(None, &part, &destination, "v1", Some(&digest)).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"payload");
        assert!(!part.exists());
    }

    #[test]
    fn digest_mismatch_errors_and_clears_the_part_file() {
        let base = test_root("digest-bad");
        let part = base.join("CS2BotImprover.zip.part");
        let destination = base.join("CS2BotImprover.zip");
        fs::write(&part, b"poisoned").unwrap();
        let digest = format!("sha256:{}", "0".repeat(64));

        let error =
            finalize_download(None, &part, &destination, "v1", Some(&digest)).unwrap_err();

        assert!(error.detail.contains("digest mismatch"));
        assert!(!part.exists(), "the .part file must be deleted on mismatch");
        assert!(!destination.exists());
    }

    #[test]
    fn digest_none_skips_verification() {
        let base = test_root("digest-none");
        let part = base.join("CS2BotImprover.zip.part");
        let destination = base.join("CS2BotImprover.zip");
        fs::write(&part, b"unchecked-but-https").unwrap();

        finalize_download(None, &part, &destination, "latest", None).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"unchecked-but-https");
        assert!(!part.exists());
    }

    #[test]
    fn version_parse_reads_the_panel_exe_filename() {
        assert_eq!(parse_panel_tag("Panel v1.4.5.exe").as_deref(), Some("v1.4.5"));
        assert_eq!(parse_panel_tag("Panel v2.0.exe").as_deref(), Some("v2.0"));
        assert_eq!(parse_panel_tag("Panel.exe"), None);
        assert_eq!(parse_panel_tag("Panel v1.4.5.txt"), None);
        assert_eq!(parse_panel_tag("readme.txt"), None);
    }

    #[test]
    fn tags_are_restricted_to_path_safe_characters() {
        assert!(validate_tag("v1.4.5").is_ok());
        assert!(validate_tag("local").is_ok());
        assert!(validate_tag("latest").is_ok());
        assert!(validate_tag("..").is_err());
        assert!(validate_tag("a/b").is_err());
        assert!(validate_tag("").is_err());
    }

    #[test]
    fn import_local_copies_the_zip_and_detects_the_tag() {
        let base = test_root("import");
        let source = base.join("manual.zip");
        standard_upstream_zip(&source);
        let cache_root = base.join("cache");

        let release = import_local_at(&source, &cache_root).unwrap();

        assert_eq!(release.tag, "v1.4.5");
        assert!(!release.from_api);
        assert_eq!(release.digest, None);
        assert!(cache_root.join("v1.4.5/CS2BotImprover.zip").is_file());
        assert_eq!(
            fs::read(cache_root.join("v1.4.5/CS2BotImprover.zip")).unwrap(),
            fs::read(&source).unwrap()
        );
        assert!(cache_root.join("v1.4.5").join(SOURCE_FILE).is_file());

        // A zip without the addons+cfg payload layout is rejected.
        let bad = base.join("bad.zip");
        write_zip(&bad, &[("only/addons.txt", b"x")]);
        assert!(import_local_at(&bad, &cache_root).is_err());
    }

    #[test]
    fn import_local_defaults_to_the_local_tag_without_an_exe() {
        let base = test_root("import-local");
        let source = base.join("manual.zip");
        write_zip(
            &source,
            &[("addons/x.dll", b"x"), ("cfg/y.cfg", b"y"), ("Panel.exe", b"nope")],
        );
        let cache_root = base.join("cache");

        let release = import_local_at(&source, &cache_root).unwrap();

        assert_eq!(release.tag, "local");
        assert!(cache_root.join("local/CS2BotImprover.zip").is_file());
    }

    #[test]
    fn merge_state_marker_invalidates_on_la_or_zip_change() {
        let fixture = merge_setup("marker");
        run_merge(&fixture);

        assert!(merged_is_current(&fixture.la, &fixture.staging, &fixture.zip));

        // A changed LA manifest (different sha256) forces a re-merge.
        fs::write(
            fixture.la.join(installer::MANIFEST_FILE),
            serde_json::to_vec(&installer::PayloadManifest {
                schema_version: 1,
                package_version: "1.4.3.4".into(),
                entries: fixture.la_manifest.entries.clone(),
            })
            .unwrap(),
        )
        .unwrap();
        assert!(!merged_is_current(&fixture.la, &fixture.staging, &fixture.zip));

        // A changed upstream zip forces a re-merge as well.
        let mut bytes = fs::read(&fixture.zip).unwrap();
        bytes.push(0);
        fs::write(&fixture.zip, bytes).unwrap();
        assert!(!merged_is_current(&fixture.la, &fixture.staging, &fixture.zip));
    }

    #[test]
    fn launch_validation_requires_a_sibling_panel_v_exe() {
        let base = test_root("launch");
        let exe_dir = base.join("panel");
        fs::create_dir_all(&exe_dir).unwrap();
        let exe = exe_dir.join("LocalArena.exe");
        fs::write(&exe, b"panel").unwrap();
        let upstream = exe_dir.join("Panel v9.9.9.exe");
        fs::write(&upstream, b"upstream").unwrap();
        let outside = base.join("elsewhere");
        fs::create_dir_all(&outside).unwrap();
        let foreign = outside.join("Panel v9.9.9.exe");
        fs::write(&foreign, b"foreign").unwrap();

        let accepted = validate_upstream_panel_path_in(
            upstream.to_str().unwrap(),
            &exe,
        )
        .unwrap();
        assert!(accepted.ends_with("Panel v9.9.9.exe"));

        assert!(validate_upstream_panel_path_in(foreign.to_str().unwrap(), &exe).is_err());
        assert!(validate_upstream_panel_path_in(
            exe_dir.join("not-a-panel.txt").to_string_lossy().as_ref(),
            &exe,
        )
        .is_err());
    }

    #[test]
    fn zip_payload_prefix_requires_addons_and_cfg() {
        assert_eq!(
            zip_payload_prefix(&["addons/a".into(), "cfg/b".into()]),
            Some(String::new())
        );
        assert_eq!(
            zip_payload_prefix(&["wrap/addons/a".into(), "wrap/cfg/b".into()]),
            Some("wrap/".into())
        );
        assert_eq!(zip_payload_prefix(&["addons/a".into()]), None);
        assert_eq!(zip_payload_prefix(&["wrap/addons/a".into(), "other/cfg/b".into()]), None);
    }
}
