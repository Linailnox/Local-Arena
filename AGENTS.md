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
├── artifacts/         # gitignored release outputs: plugin zips, tauri bundles, latest.json
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
| Build/package/release | `scripts/package.ps1`, `.github/workflows/release.yml` | plugin bundle via pwsh; Panel bundle via tauri-action |
| Downloadable components / update sources | `Panel/src-tauri/src/component_registry.rs`, `scripts/package.ps1` | three peers: upstream suite, LA plugin archive, InventorySimulator; the Panel comes from the Tauri updater, the LA archive from GitHub release assets (`LocalArena-plugin-<tag>-<os>.zip`) |
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
| `update_core` | mod | `Panel/src-tauri/src/update_core.rs` | `online_update.rs` | SemVer `DisplayVersion` (legacy 4-part folded into build metadata), SHA-256/size verification, safe zip extraction |
| `online_update` | mod | `Panel/src-tauri/src/online_update.rs` | component updates | `tauri-plugin-updater` owns Panel checks/installs; `snapshot`/`check` resolve the host `LocalArena-plugin-<tag>-<os>.zip` from `releases/latest` via `release_source`; `prepare_plugin` downloads + verifies + merges with the cached upstream zip |
| `inventory_plugin` | mod | `Panel/src-tauri/src/inventory_plugin.rs` | InventorySimulator | third-party download via `release_source` + `download::download_with_progress`, installed by the shared transaction; the installed version lives in `record.components["inventory_simulator"]` (the old `.installed.json` marker is gone) |
| `match_system.rs` | mod | `Panel/src-tauri/src/` | catalog | parses match_catalog.json (`CATALOG_RELATIVE`) |
| `apply_launch_mode` | fn | `Panel/src-tauri/src/mode_files.rs` | launch modes | rewrites gameinfo.gi from current Steam files |
| `api.ts` | ts | `Panel/src/lib/api.ts` | 94 invoke wrappers | single invoke boundary; `generate_handler!` registers 95 commands (lib.rs 81 + cs2ss_bridge 10 + appearance 4) — `import_cosmetics_inventory` currently has no wrapper |
| `BotHiderApi` | C# api | `addons/BotHider` (submodule) | PMC/TLI ProjectReference | never edit inside; gitlink pin is the version signal |
| `MatchCore` | C# lib | `addons/counterstrikesharp/shared/` | MatchCore.Tests only | `MatchRules` is tests-only now; production MR12 lives in PMC convars (PlusMatchCoordinator.cs:267) |
| `match_catalog.json` | data | `PlusMatchCoordinator/` | schema_version 1 | plugin↔Panel contract, frozen HLTV top-20 |
| `last-completed.json` | data | `<csgo>/.csbip/cs2ss/` | `match_system.rs` | completed-match marker, atomic write, watched → event `cs2ss-match-completed` |
| `component_registry` | mod | `Panel/src-tauri/src/component_registry.rs` | 3 descriptors | compiled-in component set (upstream / la_plugins / inventory_simulator); `check_requirements` preflight — LA bundle requires the upstream `BotHiderApi.dll` |
| `release_source` | mod | `Panel/src-tauri/src/release_source.rs` | `inventory_plugin.rs` | digest-required GitHub release resolution; trusted hosts only; `AssetSelector` |
| `download` | mod | `Panel/src-tauri/src/download.rs` | update pipeline | shared streaming downloader: cache reuse, `.part` resume, progress + cancel, size/sha256 verify, atomic move |
| `package.ps1` | script | `scripts/` | release | plugin-only packager: builds the LA bundle (3 plugins + plus-only manifest), runs the OpenRating gate and `-PluginBundleRoot` verification, writes `LocalArena-plugin-<tag>-<os>.zip` |
| LA plugin bundle | artifact | `artifacts/LocalArena-plugin-<tag>-{windows,linux}.zip` | plugin channel, `-PluginBundleRoot` gate | same plus-only shape on both platforms (3 plugins + plus-only manifest); no upstream runtime, no Panel executable |
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
- NEVER let an upstream path into the plugin bundle — the released `LocalArena-plugin-*.zip` must stay LA-only.
- NEVER add new UI libs in `Panel/src/components/` — compose existing primitives.
- NEVER delete the committed desktop icon set in `Panel/src-tauri/icons/` — `tauri-codegen` needs `icons/icon.png` for any non-Windows target and panics without it.
- NEVER hand-edit generated catalogs (`stickerCatalog.json`, `stickerWeaponIds.json`, `cosmeticPlacements.json`) — regenerate via `scripts/generate-*.mjs`.
- Moving/renaming anything under `addons/` is a cross-repo change — paths hardcoded in ~10 files (`installer/lib/match_system/mode_layout/install_checks.rs`, `installCheckLocalization.ts`, `scripts/*`).

## UNIQUE STYLES
- Version default duplicated: `package.ps1:5`, `.github/workflows/release.yml:6` (SemVer `^\d+\.\d+\.\d+(?:-...)?$`); plus the same version in `Panel/package.json`, `Panel/src-tauri/Cargo.toml`, `tauri.conf.json` and PMC `ModuleVersion` — all bump together.
- Upstream pinning via GitHub `assets[].digest` sha256 at download time.
- Release assets: Tauri bundles (Windows NSIS `.exe`, Linux `.AppImage`/`.deb`) + `.sig`, `LocalArena-plugin-<tag>-{windows,linux}.zip`, `latest.json` (Tauri updater schema), `SHA256SUMS.txt`; `tauri-action` creates/updates the release on each matrix leg and merges the `latest.json` platform map.
- Install payload defined by `plus-payload-manifest.json` (walks `addons|cfg|overrides` only).

## COMMANDS
```bash
# plugin bundle (pwsh)
pwsh scripts/package.ps1 [-Linux] [-ReleaseVersion x.y.z]
# Panel bundle locally (run in Panel/; CI uses tauri-action)
bun install --frozen-lockfile
bun tauri build --bundles nsis            # Windows host
bun tauri build --bundles appimage,deb    # Linux host
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
- Match rosters: `PlayerKind.RandomBot` entries have an empty name; PMC adds them with bare `bot_add_ct/t`, while featured opponents keep catalog names. Panel no longer reads `botprofile.db`/`bot_info.json` for rosters; two per-match BotHider toggles (`identity_mode_player`, `use_bot_info_names`, default true) are applied at match start.
- Local plugin builds use the system `dotnet` from PATH; `package.ps1` fails clearly when it is missing.
- MatchCore.Tests needs the net8 runtime installed alongside SDK 10.
- Linux cargo baseline: ~7 Windows-semantic test failures are accepted; run `cargo test --manifest-path Panel/src-tauri/Cargo.toml` directly (no wrapper script).
- Non-`tauri dev` Linux builds need `--features custom-protocol`; `tauri build` enables it.
- `Panel/src/data` ships ~7.1 MB JSON in the Vite bundle — `stickerCatalog.json` (4.0 MB) + `skinNames.json` (2.0 MB) dominate; both single-line minified, read via `jq`/`head -c` only.
- `stickerCatalog.json`/`stickerWeaponIds.json`/`agentCatalog.json` are `include_str!`-embedded in Rust (lib.rs :530/:541/:665) — schema changes break `cargo check`, not just tsc.
- `Panel/src/styles/fonts.css` is imported by nothing; `modern` font preset silently falls back to Segoe UI (known dead wire).
- Debug inspector: no devtools feature — debug builds have it by default (`cd Panel && bunx tauri build --debug --no-bundle`).
- `Panel/src/panels/TeamsSection.tsx` is an orphan (no importer) — dead code kept in tree.
- CI: `release.yml` is the only workflow. Tag push `v*.*.*` builds signed NSIS/AppImage/deb bundles through `tauri-action`, creates the GitHub Release, publishes `latest.json` and uploads `LocalArena-plugin-<tag>-<os>.zip` plus `SHA256SUMS.txt`; `workflow_dispatch` builds the same signed bundles and plugin zips as workflow artifacts by default, or publishes them to the `v<latest tag>` release when `upload_release` is checked.
- OfflineMatchTelemetry (net8 + Microsoft.Data.Sqlite) ships its `libe_sqlite3.so` via `package.ps1 -Linux`; OMT settles at the winner panel (`_matchSettled`, OfflineMatchTelemetry.cs:545) — once settled nothing may append to the ledger.

## Agent skills

### Issue tracker

Issues and specs are local markdown files under `.scratch/<feature-slug>/` (GitHub Issues are disabled on this repo). See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-label vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout: `GLOSSARY.md` + `docs/adr/` at the repo root (created lazily by `/domain-modeling`). See `docs/agents/domain.md`.
