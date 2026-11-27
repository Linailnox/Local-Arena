# scripts/ KNOWLEDGE BASE

**Earned:** score 17 (reference centrality: 34 filename refs repo-wide, 46 pipeline symbols; 13 files + own configs) — distinct domain: the repo's entire build/package/verify pipeline lives only here.

## OVERVIEW
PowerShell plugin build/package/verify pipeline + Node generators + Panel TS tests; the Panel itself is built and signed by `tauri-action` in `.github/workflows/release.yml`. `scripts/linux/` no longer exists.

## WHERE TO LOOK
Gate matrix — which script proves what:

| Proof | Script | Notes |
|------|--------|-------|
| Plugin DLLs compile; MatchCore tests pass | `package.ps1` | dotnet build for TLI/PMC, `dotnet publish` for OMT, then `MatchCore.Tests` |
| Repo layout / pins / source invariants | `verify-workspace.ps1` | no args = workspace gate |
| Release shape, OpenRating gate | `package.ps1` | plugin-only bundle `LocalArena-plugin-<tag>-{windows,linux}.zip` (3 plugins + plus-only manifest), OpenRating gate, `-PluginBundleRoot` verification |
| Panel updater bundle + `latest.json` | `tauri-action` in `release.yml` | tauri-action builds/signs NSIS/AppImage/deb; tag runs create the release and merge the `latest.json` platform map |
| LA-only plugin bundle shape | `verify-workspace.ps1 -PluginBundleRoot <dir> [-Linux]` | three plugins + plus-only manifest; forbids upstream paths, `gameinfo.gi`, `backup/`, InventorySimulator. `package.ps1` runs it on the staged tree before zipping |
| Panel TS logic invariants | `test-sticker-editor.mjs`, `test-install-gate.mjs` | wired from `Panel/package.json`; also run by the release workflow |
| Update-payload authenticity | Tauri updater key + `release_source.rs` | Panel bundles are signed by `tauri-action` and verified by `tauri-plugin-updater`; plugin archives use the GitHub asset digest |
| Upstream pin | `dependencies.json` | single source; `upstream_package.rs` re-validates downloads |
| Cosmetics preset migration | `migrate-cosmetics-to-inventory.ps1` | manual only — Panel must not call it |
| Data regeneration | `generate-*.mjs` | sticker catalog works; placements generator BROKEN (see ANTI-PATTERNS) |

## CONVENTIONS
- Failure style: `verify-workspace.ps1` collects via `Add-Failure`, exits 1 at end; `package.ps1` throws on first failure.
- Version triplication — bump together: `package.ps1:5` `$ReleaseVersion`, `verify-workspace.ps1:4` `$ExpectedPackageVersion`, `release.yml` tag filter; format is SemVer `^\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?$`. The workflow does not rewrite versions; update the files before tagging.
- `package.ps1` dot-sources `.local-build.ps1` and resolves `dotnet` from `-DotNet`, that file, or PATH. `verify-workspace.ps1` also fails if `package.ps1` exposes the private tool-dir name — written as `"portable" + "-toolchain"`, so plain grep misses it.
- Generators pin upstream commits as literals (ByMykel `342d496`, SteamTracking `88d5684`, cosmetic API `c3953df`) and emit sha256 into `stickerCatalog.source.json`; the gate recomputes hashes + counts (35 placements / 81 charms / 35 CT + 44 T agents).

## ANTI-PATTERNS
- `package.ps1` builds and zips only the plugin payload; it never runs cargo or touches the Panel. Panel bundles belong to `tauri-action`.
- `generate-player-cosmetic-placements.mjs` re-run throws ENOENT — its inputs under `plugins/BotRandomizer/` are deleted (0 tracked files). Gate only asserts output counts + `charmAnchors`; don't regenerate without new inputs.
- Layout changes must update `verify-workspace.ps1` `$requiredSources` and the `-PluginBundleRoot` allowlist (`Get-PlusPayloadFiles`); the old `-PackageRoot` / `$requiredPackageFiles` / `$forbiddenInPackage` modes are gone.
- Never strip the OpenRating holdout assertion in `package.ps1` (release_gate.passed, dataset_sha256, holdout MAE/spearman bounds, 7 finite weights).
- Never pipe a gate through `tail` — it swallows exit codes.
- Dotnet drift: `package.ps1` runs `dotnet publish` for OMT itself, so the staged `plugins/OfflineMatchTelemetry/stage/...` deployment is refreshed by the same run.
