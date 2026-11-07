# PROJECT KNOWLEDGE BASE

**Generated:** 2026-10-29
**Commit:** e0e8f90
**Branch:** next

## OVERVIEW
Local-Arena (Plus): CS2 bot-enhancement suite forked from ed0ard/CS2-Bot-Improver v1.4.5 (d991445). C# CounterStrikeSharp plugins + Tauri 2 desktop Panel (React 18/Vite frontend, Rust backend) + pwsh build/package/verify pipeline.

## STRUCTURE
```
Local-Arena/
├── addons/            # C# CSSharp plugins; BotHider = pinned submodule
├── Panel/             # Tauri 2 app: src/ React frontend, src-tauri/ Rust backend
├── scripts/           # entire build/package/verify pipeline + linux bash wrappers
├── artifacts/         # gitignored release outputs: full zip, panel zip, plugin zips, latest.json
├── cfg/               # upstream-identical tree — deletable, never editable
├── overrides/         # upstream-identical tree — deletable, never editable
└── .cache/package/    # digest-verified upstream zip cache (≈73 MB)
```

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Install/update behavior | `Panel/src-tauri/src/installer.rs`, `upstream_package.rs` | manifest-driven; only manifest entries get copied. Full installs copy the whole `plus-payload-manifest.json`; `install_component` copies only entries whose `ownership` matches, through the same locked transaction |
| Add backend command | `Panel/src-tauri/src/lib.rs` `generate_handler!` (lib.rs:5216) | add matching invoke wrapper in `Panel/src/lib/api.ts` (no `invoke(` outside it); invoke payload keys must be camelCase (`component_id` -> `componentId`), which `tsc` cannot catch |
| Panel UI | `Panel/src/panels/`, `Panel/src/components/` | settings subpages under `panels/settings/` |
| Match rosters | `addons/.../PlusMatchCoordinator/match_catalog.json` | parsed by `Panel/src-tauri/src/match_system.rs`; one edit fans out to install_checks.rs, mode_layout.rs, installer.rs |
| Generated data catalogs | `Panel/src/data/`, `scripts/generate-*.mjs` | generator output pinned by verify — regenerate, never hand-edit |
| Plugin↔Panel runtime contracts | `<csgo>/.csbip/` (`cs2ss/last-completed.json`, `team-lineup.json`, `cs2ss/telemetry.db`) | written by plugins (atomic tmp+move), read by Panel Rust (`match_system.rs`, `cs2ss_bridge.rs`) |
| Build/package/release | `scripts/*.ps1`, `scripts/linux/*.sh` | every gate routes through scripts/ |
| Repo layout change | `scripts/verify-workspace.ps1` | must update allowlists or the gate fails |
| Upstream pin | `scripts/dependencies.json` | v1.4.5 @ d991445 + linuxAsset sha256 |
| Downloadable components / update manifest | `Panel/src-tauri/src/component_registry.rs`, `scripts/assemble-update-manifest.ps1` | three peers: upstream suite, LA plugin archive, InventorySimulator; `latest.json` schema v2 (`la_plugins.{windows,linux}`) |
| i18n key | `Panel/src/i18n/` | EN source of truth + partial overrides |
| Plugin tests | `addons/counterstrikesharp/{plugins,shared}/` | console runners, no xunit |

## CODE MAP
| Symbol | Type | Location | Refs | Role |
|--------|------|----------|------|------|
| `payload_root()` | fn | `Panel/src-tauri/src/lib.rs` | 10+ call sites | exe-adjacent payload dir; install/inspect/restore anchor |
| `installer::install` | mod | `Panel/src-tauri/src/installer.rs` | manifest | copies only `plus-payload-manifest.json` entries |
| `install_component` | fn | `Panel/src-tauri/src/installer.rs` | component install | ownership-filtered subset of the manifest (`"plus"` = LA plugin bundle); merges into the one per-target install record, skips identical files, never writes `active-payload.json` |
| `install_la_plugins_component` | fn | `Panel/src-tauri/src/installer.rs` | `install_component` | BotHiderApi preflight via `component_registry::check_requirements`, then `install_component(.., "plus")` |
| `upstream_package` | mod | `Panel/src-tauri/src/upstream_package.rs` | deps pin | GitHub digest verify, .part resume, merged payload root |
| `update_core` | mod | `Panel/src-tauri/src/update_core.rs` | `online_update.rs` | `latest.json` schema v2 (`components.la_plugins.{windows,linux}`); `parse_update_manifest` converts a v1 `plugin` entry into `la_plugins.windows` |
| `online_update` | mod | `Panel/src-tauri/src/online_update.rs` | component updates | host platform key selects the `la_plugins` archive; a release may publish only one platform |
| `inventory_plugin` | mod | `Panel/src-tauri/src/inventory_plugin.rs` | InventorySimulator | third-party download via `release_source` + `download::download_with_progress`, installed by the shared transaction; the installed version lives in `record.components["inventory_simulator"]` (the old `.installed.json` marker is gone) |
| `match_system.rs` | mod | `Panel/src-tauri/src/` | catalog | parses match_catalog.json (`CATALOG_RELATIVE`) |
| `apply_launch_mode` | fn | `Panel/src-tauri/src/mode_files.rs` | launch modes | rewrites gameinfo.gi from current Steam files |
| `api.ts` | ts | `Panel/src/lib/api.ts` | 94 invoke wrappers | single invoke boundary; `generate_handler!` registers 95 commands (lib.rs 81 + cs2ss_bridge 10 + appearance 4) — `import_cosmetics_inventory` currently has no wrapper |
| `BotHiderApi` | C# api | `addons/BotHider` (submodule) | PMC/TLI ProjectReference | never edit inside; gitlink pin is the version signal |
| `MatchCore` | C# lib | `addons/counterstrikesharp/shared/` | MatchCore.Tests only | `MatchRules` is tests-only now; production MR12 lives in PMC convars (PlusMatchCoordinator.cs:267) |
| `match_catalog.json` | data | `PlusMatchCoordinator/` | schema_version 1 | plugin↔Panel contract, frozen HLTV top-20 |
| `verify-workspace.ps1` | gate | `scripts/` (781 LOC) | CI | LA-only whitelist (3 allowlists :176/:374/:406 move together); `-PluginBundleRoot` verifies the LA-only plugin bundle and is invoked by `package.ps1` before the bundle is zipped; collects ALL failures, exits 1 at end |
| `last-completed.json` | data | `<csgo>/.csbip/cs2ss/` | `match_system.rs` | completed-match marker, atomic write, watched → event `cs2ss-match-completed` |
| `component_registry` | mod | `Panel/src-tauri/src/component_registry.rs` | 3 descriptors | compiled-in component set (upstream / la_plugins / inventory_simulator); `check_requirements` preflight — LA bundle requires the upstream `BotHiderApi.dll` |
| `release_source` | mod | `Panel/src-tauri/src/release_source.rs` | `inventory_plugin.rs` | digest-required GitHub release resolution; trusted hosts only; `AssetSelector` |
| `download` | mod | `Panel/src-tauri/src/download.rs` | update pipeline | shared streaming downloader: cache reuse, `.part` resume, progress + cancel, size/sha256 verify, atomic move |
| `assemble-update-manifest.ps1` | script | `scripts/` | release | builds schema v2 `latest.json` from `artifacts/` by naming convention — panel + `la_plugins.windows` required, `la_plugins.linux` optional |
| LA plugin bundle | artifact | `artifacts/LocalArena-plugin-<tag>-{windows,linux}.zip` | `latest.json`, `-PluginBundleRoot` gate | Windows = full Local Arena payload (3 plugins + 2 bot cfg overlays + full manifest; no upstream runtime — the Panel downloads upstream at install time); Linux = LA-only (3 plugins + 2 bot cfg overlays + plus-only manifest) |
| `LaPluginBundleCard` | tsx | `Panel/src/components/LaPluginBundleCard.tsx` | `InstallationPage` | third of the three peer cards (upstream cache section / `InventoryPluginCard` / this one); `la_plugins` progress events, `install_la_plugin_bundle` command, `laBundle.*` i18n keys |

## CONVENTIONS
- Commits: gitmoji + Conventional Commits (e.g. `✨ feat:`, `🐛 fix:`).
- Plugin tests are console runners (hand-rolled Require/Check, `OutputType=Exe`): `dotnet run --project <Tests> -c Release`. No xunit anywhere.
- Mixed TFMs deliberate: OfflineMatchTelemetry + MatchCore net8.0; PMC/TLI net10.0. Only 3 tracked plugin projects — `PlayerKnifeCustomizer*`/`RoundDamageRecap` dirs on disk are untracked leftovers.
- Panel has NO `bun test`; real gates = `bun run build` (tsc+vite), `test:stickers`, `test:install-gate`.
- i18n: EN map in `keys.ts` is the source of truth; `DICTS` (28 blocks) are partial overrides keyed by Steam codes; new key = two-file workflow (dictionary.ts + keys.ts); new locale also touches `src/data/languages.ts`.

## ANTI-PATTERNS (THIS PROJECT)
- NEVER edit inside `addons/BotHider` — pinned submodule; the gitlink is the version signal.
- NEVER edit `cfg/` or `overrides/` — upstream-identical trees are deletable only.
- NEVER let an upstream path into the package — whitelist inversion; any layout change must update `verify-workspace.ps1` allowlists.
- NEVER add new UI libs in `Panel/src/components/` — compose existing primitives.
- NEVER delete the committed desktop icon set in `Panel/src-tauri/icons/` — `tauri-codegen` needs `icons/icon.png` for any non-Windows target and panics without it; `.local-build.ps1` stays local-only.
- NEVER hand-edit generated catalogs (`stickerCatalog.json`, `stickerWeaponIds.json`, `cosmeticPlacements.json`) — regenerate via `scripts/generate-*.mjs`; verify pins sha256/counts.
- Moving/renaming anything under `addons/` is a cross-repo change — paths hardcoded in ~10 files (`installer/lib/match_system/mode_layout/install_checks.rs`, `installCheckLocalization.ts`, `scripts/*`).

## UNIQUE STYLES
- Version default triplicated: `package.ps1:10`, `verify-workspace.ps1:4`, `.github/workflows/release.yml:6` (regex `^\d+\.\d+\.\d+\.\d+(?:-Preview\.\d+)?$`); plus `1.4.3+3` in `Panel/src-tauri/Cargo.toml` AND `tauri.conf.json` — all bump together. PMC `ModuleVersion` 1.4.3.3 tracks it.
- Upstream pinning via GitHub `assets[].digest` sha256 in `dependencies.json`.
- Release assets: `LocalArena-<tag>-<os>.zip` (full), `LocalArena-panel-<tag>-windows.zip`, `LocalArena-plugin-<tag>-{windows,linux}.zip`, `latest.json` (+ `.sig`), `SHA256SUMS.txt`; the Windows plugin asset carries the merged root's `addons|cfg|overrides`, while the Linux one copies only the LA-owned subset.
- Install payload defined by `plus-payload-manifest.json` (walks `addons|cfg|overrides` only).

## COMMANDS
```bash
# full pipeline (pwsh)
pwsh scripts/build.ps1 [-Linux]
pwsh scripts/package.ps1 [-Linux] [-ReleaseVersion x.y.z.w]
pwsh scripts/verify-workspace.ps1 [-Linux]
pwsh scripts/verify-workspace.ps1 -PluginBundleRoot <dir> [-Linux]   # LA-only plugin bundle layout
pwsh scripts/assemble-update-manifest.ps1 -Tag vX.Y.Z.W -OutputDirectory artifacts   # schema v2 latest.json
# Linux gate suite / cross-build
scripts/linux/build.sh [--only pwsh|bun|dotnet|cargo|verify]
scripts/linux/release.sh --check | --version X   # env: LLVM_BIN PWSH XWIN_CACHE
# Panel Rust backend
cargo check --manifest-path Panel/src-tauri/Cargo.toml
# Panel frontend (run in Panel/)
bun run build && bun run test:stickers && bun run test:install-gate
# plugin tests (console runners)
dotnet run --project addons/counterstrikesharp/shared/MatchCore.Tests -c Release
# regenerate sticker catalog (placements generator currently ENOENTs — inputs deleted)
node scripts/generate-sticker-catalog.mjs
```

## NOTES
- `.local-build.ps1` (gitignored, repo root) required for local builds/verify — minimal content: `$DotNet = "$HOME/.dotnet/dotnet"`.
- MatchCore.Tests needs the net8 runtime installed alongside SDK 10.
- Linux cargo baseline: ~7 Windows-semantic test failures are accepted; `scripts/linux/build.sh` owns the full suite.
- Non-`tauri dev` Linux builds need `--features custom-protocol`.
- `Panel/src/data` ships ~7.1 MB JSON in the Vite bundle — `stickerCatalog.json` (4.0 MB) + `skinNames.json` (2.0 MB) dominate; both single-line minified, read via `jq`/`head -c` only.
- `stickerCatalog.json`/`stickerWeaponIds.json`/`agentCatalog.json` are `include_str!`-embedded in Rust (lib.rs :530/:541/:665) — schema changes break `cargo check`, not just tsc.
- `Panel/src/styles/fonts.css` is imported by nothing; `modern` font preset silently falls back to Segoe UI (known dead wire).
- Debug inspector: no devtools feature — debug builds have it by default (`cd Panel && bunx tauri build --debug --no-bundle`).
- `Panel/src/panels/TeamsSection.tsx` is an orphan (no importer) — dead code kept in tree.
- CI: `build.yml` (push to update/build; Node 22 + dotnet 10.0.x; submodules recursive; two jobs — `build` on windows-latest and `linux` on ubuntu-latest), `release.yml` (tags `v1.4.3.3-Preview.*`; per-platform `build` matrix + a `publish` job that re-runs `assemble-update-manifest.ps1`, signs and publishes `latest.json` once every platform archive exists).
- OfflineMatchTelemetry (net8 + Microsoft.Data.Sqlite) ships its `libe_sqlite3.so` via `package.ps1 -Linux`; OMT settles at the winner panel (`_matchSettled`, OfflineMatchTelemetry.cs:545) — once settled nothing may append to the ledger.
