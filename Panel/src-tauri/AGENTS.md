# AGENTS.md — src-tauri (Rust backend)

## OVERVIEW
Score 17 — the entire backend; distinct domain from the React frontend. Crate
`cs2-bot-improver-plus-panel` (lib `cs2_bot_improver_plus_panel_lib`, edition 2024).
Flat `src/`: 25 `.rs` files (`lib.rs` + 23 modules + `main.rs`), 20,259 LOC; 24 of them
carry inline `mod tests` (231 `#[test]`s). No `tests/` dir — cross-crate tests live in `core-tests/`.

## WHERE TO LOOK
| Task | Location |
|------|----------|
| Add a Tauri command | `#[tauri::command]` fn (lib.rs 79 / cs2ss_bridge.rs 10 / appearance.rs 4 = 93 total) + register in `generate_handler!` (lib.rs:4730); frontend half in `Panel/src/lib/api.ts` (92 wrappers today — `import_cosmetics_inventory` is unwrapped) |
| App setup / config / merge | `src/lib.rs` (4768 LOC, largest; tests start 3784) |
| Install / repair / restore | `src/installer.rs` (2976; copies only `plus-payload-manifest.json` entries) |
| Component install | `src/installer.rs` — `install_component` (:1021) runs the ownership-filtered manifest subset through the same locked, journalled transaction, skips identical files, merges into the one per-target record and never writes `active-payload.json`; `install_la_plugins_component` (:1063) checks `LA_PLUGIN_BUNDLE.requires` (the upstream `BotHiderApi.dll`) then installs the `"plus"` subset. Record version rule: a full-payload install sets `package_version` from its own manifest; a `"plus"` component install records the freshly installed bundle version unless that would downgrade the existing suite record (`plus_component_package_version`), while a third-party component never lets its tag (`3.7.0`) become the suite version (`is_suite_record` gates the carry-over) — read the comment block above `InstallRecord` in `install_manifest` before touching it |
| LA plugin bundle command | `src/lib.rs` `install_la_plugin_bundle` (:3502) — decides from the update snapshot: with an update available it downloads/verifies/merges via `online_update::prepare_plugin`, otherwise installs from the merged local payload; then `install_la_plugins_component` (no archive extraction of its own); api wrapper `api.installLaPluginBundle` |
| Upstream download + merge | `src/upstream_package.rs` (2249; GitHub digest sha256 verify, `.part` resume, cache `.cache/package/`) |
| Stats telemetry bridge | `src/cs2ss_bridge.rs` (1823; reads `<csgo>/.csbip/cs2ss/telemetry.db`) |
| Match system | `src/match_system.rs` (889) |
| Launch modes / gameinfo.gi | `src/mode_files.rs` (`apply_launch_mode` rewrites from current Steam files) |
| Steam discovery | `src/steam.rs` (cross-platform roots incl. non-Windows) |
| Online update | `src/online_update.rs` (1133) — `snapshot`/`check` read the signed manifest; `host_platform()` keys the LA bundle to `la_plugins.{windows,linux}`, and a release that carries only the other platform errors instead of falling back; `prepare_plugin` downloads + verifies + merges with the cached upstream zip and activates the payload; `client(Duration)` is the one HTTPS-only `reqwest` client every component shares |
| Update manifest schema v2 | `src/update_core.rs` (719) — `components.la_plugins.{windows,linux}`; `parse_update_manifest` converts a v1 `plugin` entry to `la_plugins.windows` (`linux: None`); `validate_v2` requires at least one platform; sha256/signature helpers + `extract_zip_safely` |
| Component registry | `src/component_registry.rs` (242) — compiled-in `UPSTREAM_BUNDLE` / `LA_PLUGIN_BUNDLE` / `INVENTORY_SIMULATOR` (`ALL`); `ComponentSource` = `GitHubRelease { repository, asset: AssetName }` or `SignedLatestJson { field }`; `AssetName::selector()` builds the `release_source::AssetSelector`, so the matching rules live only in `release_source`; `Requirement::BotHiderApi` + fail-closed `check_requirements(target, requires)`. `by_id` is production code: `inventory_plugin::registry_source()` returns `Result` and rejects a descriptor whose source is not a GitHub release |
| GitHub release resolution | `src/release_source.rs` (232) — `resolve_latest` / `resolve_tag`, digest-required, trusted GitHub hosts only, `AssetSelector::{Prefix, ExactName}` |
| Shared streaming download | `src/download.rs` (596) — `download_with_progress`: cache reuse, `.part` resume, progress + cancel, size/sha256 verify, atomic move |
| Install health / diagnostics | `src/install_checks.rs`, `diagnostics.rs` |
| Cosmetics (InventorySimulator) | `src/cosmetics_source.rs` (local↔direct switch), `cosmetics_inventory.rs` (JSON mapping + `_panel` sidecar), `inventory_plugin.rs` (562; third-party, never shipped: `registry_source()` resolves repo + selector from the registry, the shared client and `download_with_progress` fill a per-tag cache, `extract_zip_safely` stages it, and one `installer::install_component` transaction writes only its two payload paths — the `.installed.json` marker is a manifest entry, so it lands with the payload instead of ahead of it) |
| App state root | `src/app_storage.rs` — `.csbip` next to the exe (override `CS2BI_STATE_ROOT`); all writes via `atomic_fs.rs` (temp + replace) |
| Cross-crate core tests | `core-tests/` (re-includes 11 modules via `#[path]`, separate manifest; it does NOT cover `component_registry`/`release_source` — they need `reqwest`, absent from the harness manifest — so the crate currently fails to compile with 5 `E0433`s: 3 pre-existing `cosmetics_source`, 2 added by the component refactor; no gate compiles it, see the `//!` header of `core-tests/src/lib.rs`) |

## CONVENTIONS
- All modules private `mod` in lib.rs — crate root is the only public surface.
- Windows-only deps (`winreg`, `windows-sys`) under `[target.'cfg(windows)'.dependencies]`; cross-platform behavior via pure functions.
- reqwest uses rustls; no OpenSSL.
- Error model: `AppError { code, category, detail }` (lib.rs:43) with stable `E1xxx` codes — errors carry a code, never prose only.
- Windows file ops retry on a fixed ladder (`installer::FILE_RETRY_DELAYS`, 80 ms → 2.2 s).
- `crate-type = ["rlib"]` deliberate (Cargo.toml comment: unused cdylib over-exports symbols past PE ordinal table).
- Feature `custom-protocol` REQUIRED for any non-`tauri dev` build (Cargo.toml).
- Tauri v2 (`tauri.conf.json` schema 2); frameless (`decorations:false, transparent:true`); NSIS-only bundle.

## ANTI-PATTERNS / TRAPS
- A merge must never consume its own output as the LA overlay source (`la_payload_source`, lib.rs:935).
- `SUITE_OWNED_FILES` (installer.rs:68) = 6 vdf/ini + 5 `cfg/bot_*|my_bot_*` entries — NOT `cfg/gamemode_*`: restore must run against the merged payload root or upstream gamemode cfgs leak.
- `backup/{Online,WithBots}/gameinfo.gi` is machine state — never distribute.
- Linux `cargo build` needs a temp `icons/icon.png` (only icon.ico is committed) — create it, never commit it.
- `scripts/verify-workspace.ps1` greps `lib.rs` for literal markers (`aim_supported,`, `.csbip/aim-runtime.json`, `aim_override_count`, `"normal", "less", "off"`) — renaming those strings fails the gate even if behavior is unchanged.
- Linux test failures are allowlisted by exact test name in `scripts/linux/build.sh` (`KNOWN_WINDOWS_ONLY`, 7 entries) — never widen that list.
- InventorySimulator (`inventory_plugin.rs`) is third-party: never shipped in the package; the Panel only downloads/installs it (release resolution via the registry + `release_source.rs`, one transactional install whose manifest carries the `.installed.json` marker, only its two payload paths written).
- The managed component set is compiled in `component_registry.rs` — adding a component is a compile-time change; never introduce a shipped `components.json` (second source of truth, editable after install).
- Never read or derive state from `src-tauri/gen/` (generated Tauri schemas, untracked).

## COMMANDS
```bash
cargo check --manifest-path Panel/src-tauri/Cargo.toml
cargo test  --manifest-path Panel/src-tauri/Cargo.toml
# Linux baseline: ~7 known Windows-semantic failures accepted (scripts/linux/build.sh owns this gate)
cargo test  --manifest-path Panel/src-tauri/core-tests/Cargo.toml   # cross-crate harness, separate manifest

# Local debug build — the WebView inspector (right-click Inspect / Ctrl+Shift+I) is on by default in debug builds; there is no `devtools` feature:
cd Panel && bunx tauri build --debug --no-bundle
# Any non-`tauri dev` build needs the feature:
cargo build --release --features custom-protocol --manifest-path Panel/src-tauri/Cargo.toml
```
