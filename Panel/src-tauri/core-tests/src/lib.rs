//! Cross-crate harness: the backend modules that hold pure logic are re-included here through
//! `#[path]`, against the `AppError`/`Result` stand-ins below, so their own `#[test]`s run without
//! the Tauri app. A module joins this graph only when everything it pulls is already a dependency
//! of this manifest — `component_registry` (and with it `release_source`) stays out because
//! `release_source` needs the `reqwest` HTTP client plus the `AppError::update` family;
//! `cosmetics_source` is the older gap behind the 3 `E0433`s in `app_storage`/`installer`.
//! Closing either gap means adding that dependency surface here, or splitting the HTTP-free part
//! (`AssetSelector`) into its own module and including that.
#![allow(dead_code)]

use serde::Serialize;

type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Serialize)]
struct AppError {
    code: &'static str,
    category: &'static str,
    detail: String,
}

impl AppError {
    fn new(code: &'static str, category: &'static str, detail: impl Into<String>) -> Self {
        Self { code, category, detail: detail.into() }
    }
    fn invalid(detail: impl Into<String>) -> Self { Self::new("E1002", "validation", detail) }
    fn payload(detail: impl Into<String>) -> Self { Self::new("E1301", "payload", detail) }
    fn transaction(detail: impl Into<String>) -> Self { Self::new("E1401", "installation", detail) }
    fn transaction_io(error: std::io::Error) -> Self { Self::transaction(error.to_string()) }
}

#[path = "../../src/app_storage.rs"]
mod app_storage;
#[path = "../../src/app_version.rs"]
mod app_version;
#[path = "../../src/atomic_fs.rs"]
mod atomic_fs;
#[path = "../../src/diagnostics.rs"]
mod diagnostics;
#[path = "../../src/installer.rs"]
mod installer;
#[path = "../../src/logging.rs"]
mod logging;
#[path = "../../src/mode_files.rs"]
mod mode_files;
#[path = "../../src/mode_layout.rs"]
mod mode_layout;
#[path = "../../src/runtime_state.rs"]
mod runtime_state;
#[path = "../../src/steam.rs"]
mod steam;
#[path = "../../src/update_core.rs"]
mod update_core;
