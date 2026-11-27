# scripts/ KNOWLEDGE BASE

**Earned:** score 17 (reference centrality: 34 filename refs repo-wide, 46 pipeline symbols; 13 files + own configs) — distinct domain: the repo's entire build/package pipeline lives only here.

## OVERVIEW
PowerShell plugin build/package pipeline + Node generators + Panel TS tests; the Panel itself is built and signed by `tauri-action` in `.github/workflows/release.yml`. `scripts/linux/` no longer exists, and there is no workspace verification script.

## WHERE TO LOOK
Pipeline map — which script does what:

| Proof | Script | Notes |
|------|--------|-------|
| Plugin DLLs compile; MatchCore tests pass | `package.ps1` | dotnet build for TLI/PMC, `dotnet publish` for OMT, then `MatchCore.Tests` |
| Release shape, OpenRating gate | `package.ps1` | plugin-only bundle `LocalArena-plugin-<tag>-{windows,linux}.zip` (3 plugins + plus-only manifest), OpenRating gate, zip written to `artifacts/` |
| Panel updater bundle + `latest.json` | `tauri-action` in `release.yml` | tauri-action builds/signs NSIS/AppImage/deb; tag runs create the release and merge the `latest.json` platform map |
| Panel TS logic invariants | `test-sticker-editor.mjs`, `test-install-gate.mjs` | wired from `Panel/package.json`; also run by the release workflow |
| Update-payload authenticity | Tauri updater key + `release_source.rs` | Panel bundles are signed by `tauri-action` and verified by `tauri-plugin-updater`; plugin archives use the GitHub asset digest |
| Cosmetics preset migration | `migrate-cosmetics-to-inventory.ps1` | manual only — Panel must not call it |
| Data regeneration | `generate-*.mjs` | sticker catalog works; placements generator BROKEN (see ANTI-PATTERNS) |

## CONVENTIONS
- `package.ps1` throws on the first failure; it has no workspace-gate step anymore.
- Version duplication — bump together: `package.ps1:5` `$ReleaseVersion` and the `release.yml` tag filter; format is SemVer `^\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?$`. The workflow does not rewrite versions; update the files before tagging.
- `package.ps1` uses the system `dotnet` from PATH and fails clearly when it is missing; there is no local build config file or `-DotNet` override.
- Generators pin upstream commits as literals (ByMykel `342d496`, SteamTracking `88d5684`, cosmetic API `c3953df`) and emit sha256 into `stickerCatalog.source.json`; no script gate verifies them anymore.

## ANTI-PATTERNS
- `package.ps1` builds and zips only the plugin payload; it never runs cargo or touches the Panel. Panel bundles belong to `tauri-action`.
- `generate-player-cosmetic-placements.mjs` re-run throws ENOENT — its inputs under `plugins/BotRandomizer/` are deleted (0 tracked files). Don't regenerate without new inputs.
- The plugin bundle must stay LA-only (three plugins + plus manifest); no upstream runtime, `gameinfo.gi`, `backup/`, or Panel executable.
- Never strip the OpenRating holdout assertion in `package.ps1` (release_gate.passed, dataset_sha256, holdout MAE/spearman bounds, 7 finite weights).
- Never pipe a build through `tail` — it swallows exit codes.
- Dotnet drift: `package.ps1` runs `dotnet publish` for OMT itself, so the staged `plugins/OfflineMatchTelemetry/stage/...` deployment is refreshed by the same run.
