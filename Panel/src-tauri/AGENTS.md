# AGENTS.md — src-tauri (Rust backend)

## OVERVIEW
Score 17 — the entire backend; distinct domain from the React frontend. Crate
`cs2-bot-improver-plus-panel` (lib `cs2_bot_improver_plus_panel_lib`, edition 2024).
Flat `src/`: 22 `.rs` files (21 modules + `main.rs`), ~17.6k LOC; 21 modules carry
inline `mod tests` (~178 `#[test]`s). No `tests/` dir — cross-crate tests live in `core-tests/`.

## WHERE TO LOOK
| Task | Location |
|------|----------|
| Add a Tauri command | `#[tauri::command]` fn (lib.rs 78 / cs2ss_bridge.rs 10 / appearance.rs 4 = 92 total) + register in `generate_handler!` (lib.rs:4538); frontend half in `Panel/src/lib/api.ts` |
| App setup / config / merge | `src/lib.rs` (4572 LOC, largest; tests start ~3501) |
| Install / repair / restore | `src/installer.rs` (2285; copies only `plus-payload-manifest.json` entries) |
| Upstream download + merge | `src/upstream_package.rs` (2249; GitHub digest sha256 verify, `.part` resume, cache `.cache/package/`) |
| Stats telemetry bridge | `src/cs2ss_bridge.rs` (1823; reads `<csgo>/.csbip/cs2ss/telemetry.db`) |
| Match system | `src/match_system.rs` (889) |
| Launch modes / gameinfo.gi | `src/mode_files.rs` (`apply_launch_mode` rewrites from current Steam files) |
| Steam discovery | `src/steam.rs` (cross-platform roots incl. non-Windows) |
| Online update | `src/online_update.rs`, `update_core.rs` |
| Install health / diagnostics | `src/install_checks.rs`, `diagnostics.rs` |
| Cosmetics (InventorySimulator) | `src/cosmetics_source.rs` (local↔direct switch), `cosmetics_inventory.rs` (JSON mapping + `_panel` sidecar), `inventory_plugin.rs` (third-party plugin download) |
| App state root | `src/app_storage.rs` — `.csbip` next to the exe (override `CS2BI_STATE_ROOT`); all writes via `atomic_fs.rs` (temp + replace) |
| Cross-crate core tests | `core-tests/` (re-includes 11 modules via `#[path]`, separate manifest) |

## CONVENTIONS
- All modules private `mod` in lib.rs — crate root is the only public surface.
- Windows-only deps (`winreg`, `windows-sys`) under `[target.'cfg(windows)'.dependencies]`; cross-platform behavior via pure functions.
- reqwest uses rustls; no OpenSSL.
- Error model: `AppError { code, category, detail }` (lib.rs:40) with stable `E1xxx` codes — errors carry a code, never prose only.
- Windows file ops retry on a fixed ladder (`installer::FILE_RETRY_DELAYS`, 80 ms → 2.2 s).
- `crate-type = ["rlib"]` deliberate (Cargo.toml comment: unused cdylib over-exports symbols past PE ordinal table).
- Feature `custom-protocol` REQUIRED for any non-`tauri dev` build (Cargo.toml).
- Tauri v2 (`tauri.conf.json` schema 2); frameless (`decorations:false, transparent:true`); NSIS-only bundle.

## ANTI-PATTERNS / TRAPS
- A merge must never consume its own output as the LA overlay source (`la_payload_source`, lib.rs:932).
- `SUITE_OWNED_FILES` (installer.rs:68) = 6 vdf/ini + 5 `cfg/bot_*|my_bot_*` entries — NOT `cfg/gamemode_*`: restore must run against the merged payload root or upstream gamemode cfgs leak.
- `backup/{Online,WithBots}/gameinfo.gi` is machine state — never distribute.
- Linux `cargo build` needs a temp `icons/icon.png` (only icon.ico is committed) — create it, never commit it.
- `scripts/verify-workspace.ps1` greps `lib.rs` for literal markers (`aim_supported,`, `.csbip/aim-runtime.json`, `aim_override_count`, `"normal", "less", "off"`) — renaming those strings fails the gate even if behavior is unchanged.
- Linux test failures are allowlisted by exact test name in `scripts/linux/build.sh` (`KNOWN_WINDOWS_ONLY`, 7 entries) — never widen that list.
- InventorySimulator (`inventory_plugin.rs`) is third-party: never shipped in the package; the Panel only downloads/installs it.
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
