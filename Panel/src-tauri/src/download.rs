//! Shared streaming downloader for every Panel-managed asset.
//!
//! One transfer pipeline: verify a cached file, otherwise resume or restart a
//! `<dest>.part` file over HTTPS, stream it in 128 KiB chunks while reporting
//! progress and honouring cancellation, then verify size + SHA-256 and move the
//! finished file into place atomically.
//!
//! Cancel semantics: an interrupted transfer keeps `.part`, because the bytes
//! already on disk are a valid resume base for the next attempt. A verification
//! failure (size or digest mismatch) deletes `.part`, because poisoned bytes
//! must never be resumed from.
//!
//! Timeouts are the caller's client configuration, not this module's: callers
//! pass a `reqwest::blocking::Client` whose timeout matches their workload
//! (online updates use `DOWNLOAD_TIMEOUT`, the inventory plugin 120s).

use crate::atomic_fs;
use crate::update_core;
use crate::AppError;
use crate::Result;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const CHUNK_BYTES: usize = 128 * 1024;

/// Downloads `url` to `dest`, returning the number of bytes the finished file
/// holds (always `expected_size` on success).
///
/// A `dest` that already matches `expected_size`/`expected_sha256` is reused
/// without any network access. The URL is checked against the trusted GitHub
/// hosts before a request is made.
pub fn download_with_progress(
    client: &reqwest::blocking::Client,
    url: &str,
    expected_size: u64,
    expected_sha256: &str,
    dest: &Path,
    progress: impl Fn(u64, u64),
    cancel: impl Fn() -> bool,
) -> Result<u64> {
    if dest.is_file()
        && update_core::verify_component_file(dest, expected_size, expected_sha256).is_ok()
    {
        return Ok(expected_size);
    }
    update_core::validate_https_github_url(url).map_err(AppError::update)?;
    download_streaming(
        client,
        url,
        expected_size,
        expected_sha256,
        dest,
        progress,
        cancel,
    )
}

/// The transfer half of [`download_with_progress`]: resume/restart `.part`,
/// stream, verify, and replace `dest`. The caller has already accepted `url`.
fn download_streaming(
    client: &reqwest::blocking::Client,
    url: &str,
    expected_size: u64,
    expected_sha256: &str,
    dest: &Path,
    progress: impl Fn(u64, u64),
    cancel: impl Fn() -> bool,
) -> Result<u64> {
    if expected_size == 0 || expected_size > update_core::MAX_ARCHIVE_BYTES {
        return Err(AppError::update(
            "Update size is outside the allowed range",
        ));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(AppError::transaction_io)?;
    }
    let part = part_path(dest);
    let mut offset = match fs::metadata(&part) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        _ => 0,
    };
    if offset > expected_size {
        // A leftover longer than the expected payload cannot be a resume base.
        let _ = fs::remove_file(&part);
        offset = 0;
    }

    let mut request = client.get(url);
    if offset > 0 {
        let range = format!("bytes={offset}-");
        request = request.header(
            reqwest::header::RANGE,
            reqwest::header::HeaderValue::from_str(&range)
                .map_err(|_| AppError::update("Invalid download resume range"))?,
        );
    }
    let mut response = request.send().map_err(|error| {
        AppError::update(format!("Cannot download the update from GitHub: {error}"))
    })?;
    let status = response.status();
    if !status.is_success() {
        // Keep `.part`: the bytes already on disk remain resumable.
        return Err(AppError::update(format!(
            "Update download failed with HTTP {status}"
        )));
    }
    let resuming = status.as_u16() == 206 && offset > 0;
    if !resuming {
        // Either a fresh transfer, or the server ignored the Range header and
        // sent the whole body: restart the `.part` file from zero.
        offset = 0;
        if let Some(length) = response.content_length() {
            if length != expected_size {
                return Err(AppError::update(
                    "Update download Content-Length did not match the expected size",
                ));
            }
        }
    }

    let mut output = OpenOptions::new()
        .write(true)
        .create(true)
        .append(resuming)
        .truncate(!resuming)
        .open(&part)
        .map_err(AppError::transaction_io)?;
    let mut downloaded = offset;
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    loop {
        let count = response
            .read(&mut buffer)
            .map_err(|error| AppError::update(format!("Update download failed: {error}")))?;
        if count == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(count as u64);
        if downloaded > expected_size || downloaded > update_core::MAX_ARCHIVE_BYTES {
            drop(output);
            let _ = fs::remove_file(&part);
            return Err(AppError::update(
                "Downloaded update exceeded the expected size",
            ));
        }
        output
            .write_all(&buffer[..count])
            .map_err(AppError::transaction_io)?;
        progress(downloaded, expected_size);
        if cancel() {
            // Keep `.part`: a cancelled transfer is resumable.
            return Err(AppError::update("Update download was cancelled"));
        }
    }
    output.sync_all().map_err(AppError::transaction_io)?;
    drop(output);

    if let Err(error) = update_core::verify_component_file(&part, expected_size, expected_sha256) {
        // Poisoned bytes are not a resumable base.
        let _ = fs::remove_file(&part);
        return Err(AppError::update(error));
    }
    atomic_fs::replace(&part, dest).map_err(AppError::transaction_io)?;
    Ok(expected_size)
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    struct TestServer {
        address: SocketAddr,
        requests: Arc<Mutex<Vec<String>>>,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl TestServer {
        fn start(handler: impl Fn(&mut TcpStream, &str) + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let worker = {
                let requests = Arc::clone(&requests);
                let stop = Arc::clone(&stop);
                let handler = Arc::new(handler);
                thread::spawn(move || {
                    while let Ok((mut stream, _)) = listener.accept() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let request = read_request(&mut stream);
                        requests.lock().unwrap().push(request.clone());
                        handler(&mut stream, &request);
                    }
                })
            };
            Self {
                address,
                requests,
                stop,
                worker: Some(worker),
            }
        }

        fn url(&self, path: &str) -> String {
            format!("http://{}{path}", self.address)
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        fn shutdown(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            // Unblock the worker's accept so it can observe the stop flag.
            let _ = TcpStream::connect(self.address);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            if self.worker.is_some() {
                self.shutdown();
            }
        }
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];
        while let Ok(count) = stream.read(&mut chunk) {
            if count == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..count]);
            if buffer.windows(4).any(|window| window == b"\r\n\r\n") || buffer.len() > 64 * 1024 {
                break;
            }
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }

    fn respond(stream: &mut TcpStream, status: &str, content_length: usize, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hash = Sha256::new();
        hash.update(bytes);
        format!("{:x}", hash.finalize())
    }

    fn test_client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    }

    fn temp_dir(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("la-dl-{}-{suffix}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fresh_download_streams_verifies_and_moves_into_place() {
        let dir = temp_dir("fresh");
        let body = b"local-arena-fresh-payload-".repeat(4096);
        let digest = sha256_hex(&body);
        let mut server = TestServer::start({
            let body = body.clone();
            move |stream, _request| respond(stream, "200 OK", body.len(), &body)
        });
        let dest = dir.join("payload.zip");
        let observed = Arc::new(Mutex::new(Vec::new()));
        let downloaded = download_streaming(
            &test_client(),
            &server.url("/asset.zip"),
            body.len() as u64,
            &digest,
            &dest,
            {
                let observed = Arc::clone(&observed);
                move |done, total| observed.lock().unwrap().push((done, total))
            },
            || false,
        )
        .unwrap();

        assert_eq!(downloaded, body.len() as u64);
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert!(!part_path(&dest).exists());
        let progress = observed.lock().unwrap();
        assert!(
            progress
                .iter()
                .any(|(done, total)| *done == body.len() as u64 && *total == body.len() as u64),
            "progress must report the finished transfer: {progress:?}"
        );
        drop(progress);
        server.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn interrupted_transfer_keeps_part_and_resumes_with_range() {
        let dir = temp_dir("resume");
        let body: Vec<u8> = (0..200_000_u32).map(|index| (index % 251) as u8).collect();
        let digest = sha256_hex(&body);
        let half = body.len() / 2;
        let dest = dir.join("payload.zip");

        let mut truncated = TestServer::start({
            let body = body.clone();
            move |stream, _request| {
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&body[..half]).unwrap();
                stream.flush().unwrap();
                // Returning closes the connection mid-body: a truncated transfer.
            }
        });
        let first = download_streaming(
            &test_client(),
            &truncated.url("/asset.zip"),
            body.len() as u64,
            &digest,
            &dest,
            |_, _| {},
            || false,
        );
        assert!(first.is_err(), "a truncated body must fail the download");
        let part = part_path(&dest);
        assert!(part.exists(), "a transfer interruption keeps .part");
        assert_eq!(fs::metadata(&part).unwrap().len(), half as u64);
        assert!(!dest.exists());
        truncated.shutdown();

        let mut resumed = TestServer::start({
            let body = body.clone();
            move |stream, request| {
                let request = request.to_ascii_lowercase();
                let start = request
                    .split_once("range: bytes=")
                    .and_then(|(_, rest)| rest.split_once('-'))
                    .and_then(|(start, _)| start.trim().parse::<usize>().ok());
                match start {
                    Some(start) => {
                        respond(stream, "206 Partial Content", body.len() - start, &body[start..]);
                    }
                    None => respond(stream, "200 OK", body.len(), &body),
                }
            }
        });
        let downloaded = download_streaming(
            &test_client(),
            &resumed.url("/asset.zip"),
            body.len() as u64,
            &digest,
            &dest,
            |_, _| {},
            || false,
        )
        .unwrap();

        assert_eq!(downloaded, body.len() as u64);
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert!(!part_path(&dest).exists());
        let expected_range = format!("range: bytes={half}-");
        let requests = resumed.requests();
        assert!(
            requests
                .iter()
                .any(|request| request.to_ascii_lowercase().contains(&expected_range)),
            "resume request must carry Range: bytes={half}-; got {requests:?}"
        );
        resumed.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn digest_mismatch_deletes_the_part_file() {
        let dir = temp_dir("digest");
        let expected = b"expected-bytes".repeat(512);
        let served = b"tampered-bytes".repeat(512);
        assert_eq!(expected.len(), served.len());
        let digest = sha256_hex(&expected);
        let mut server = TestServer::start({
            let served = served.clone();
            move |stream, _request| respond(stream, "200 OK", served.len(), &served)
        });
        let dest = dir.join("payload.zip");
        let error = download_streaming(
            &test_client(),
            &server.url("/asset.zip"),
            expected.len() as u64,
            &digest,
            &dest,
            |_, _| {},
            || false,
        )
        .unwrap_err();

        assert!(
            format!("{error:?}").contains("SHA-256"),
            "digest mismatch must be reported: {error:?}"
        );
        assert!(!part_path(&dest).exists(), "poisoned .part must be deleted");
        assert!(!dest.exists());
        server.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_keeps_the_part_file_resumable() {
        let dir = temp_dir("cancel");
        let body = vec![7_u8; 300_000];
        let digest = sha256_hex(&body);
        let mut server = TestServer::start({
            let body = body.clone();
            move |stream, _request| respond(stream, "200 OK", body.len(), &body)
        });
        let dest = dir.join("payload.zip");
        let error = download_streaming(
            &test_client(),
            &server.url("/asset.zip"),
            body.len() as u64,
            &digest,
            &dest,
            |_, _| {},
            || true,
        )
        .unwrap_err();

        assert!(
            format!("{error:?}").contains("cancelled"),
            "cancellation must be reported: {error:?}"
        );
        let part = part_path(&dest);
        assert!(part.exists(), "a cancelled transfer keeps .part");
        assert!(fs::metadata(&part).unwrap().len() > 0);
        assert!(!dest.exists());
        server.shutdown();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verified_cache_is_reused_without_network() {
        let dir = temp_dir("cache");
        let body = b"already-downloaded".repeat(64);
        let digest = sha256_hex(&body);
        let dest = dir.join("payload.zip");
        fs::write(&dest, &body).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("http://{}/asset.zip", listener.local_addr().unwrap());

        let downloaded = download_with_progress(
            &test_client(),
            &url,
            body.len() as u64,
            &digest,
            &dest,
            |_, _| {},
            || false,
        )
        .unwrap();

        assert_eq!(downloaded, body.len() as u64);
        listener.set_nonblocking(true).unwrap();
        assert!(
            listener.accept().is_err(),
            "a verified cache hit must not contact the server"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn untrusted_url_is_rejected_before_transfer() {
        let dir = temp_dir("url");
        let dest = dir.join("payload.zip");
        let digest = "a".repeat(64);
        let error = download_with_progress(
            &test_client(),
            "http://127.0.0.1:9/asset.zip",
            1024,
            &digest,
            &dest,
            |_, _| {},
            || false,
        )
        .unwrap_err();

        assert!(
            format!("{error:?}").contains("HTTPS"),
            "untrusted hosts must be rejected: {error:?}"
        );
        assert!(!dest.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
