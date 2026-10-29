# scripts/ KNOWLEDGE BASE

**Earned:** score 17 (reference centrality: 34 filename refs repo-wide, 46 pipeline symbols; 13 files + own configs) — distinct domain: the repo's entire build/package/verify pipeline lives only here.

## OVERVIEW
PowerShell-first build/package/verify pipeline, Node generators + Panel TS tests, Python update signer; `linux/` wraps the same ps1 gates in bash for POSIX hosts.

## WHERE TO LOOK
Gate matrix — which script proves what:

| Proof | Script | Notes |
|------|--------|-------|
| Code builds; Panel exe is production-compiled | `build.ps1` | greps tauri output for `cargo:dev=false` / absence of `cargo:rustc-cfg=dev` (:254-274) |
| Repo layout / pins / source invariants | `verify-workspace.ps1` | 638 LOC; no args = workspace gate |
| Released payload LA-only + manifest-complete | `verify-workspace.ps1 -PackageRoot <dir> -ExpectedPackageVersion X` | package mode |
| Release shape, OpenRating gate, signing, checksums | `package.ps1` | `latest.json`, SHA256SUMS, PREVIEW-NOTICE |
| Five local gates on Linux | `linux/build.sh` | step keys `pwsh\|bun\|dotnet\|cargo\|verify`; `--only <step>` |
| Cross-compile prerequisites | `linux/release.sh --check` | `--version X` cross-builds via package.ps1; env `LLVM_BIN PWSH XWIN_CACHE` |
| Panel TS logic invariants | `test-sticker-editor.mjs`, `test-install-gate.mjs` | wired from `Panel/package.json`; run from Panel/ |
| Update-payload authenticity | `sign-update.py` + `update-public-key.txt` | Ed25519; runs only when `CSBIP_UPDATE_SIGNING_KEY` set |
| Upstream pin | `dependencies.json` | single source; `upstream_package.rs` re-validates downloads |
| Cosmetics preset migration | `migrate-cosmetics-to-inventory.ps1` | manual only — Panel must not call it |
| Data regeneration | `generate-*.mjs` | sticker catalog works; placements generator BROKEN (see ANTI-PATTERNS) |

## CONVENTIONS
- Failure style: `verify-workspace.ps1` collects via `Add-Failure`, exits 1 at end (one run reports all violations); `build.ps1`/`package.ps1` throw on first failure.
- Version triplication — bump together: `package.ps1:10` `$ReleaseVersion`, `verify-workspace.ps1:4` `$ExpectedPackageVersion`, `release.yml:6` tag filter (all `1.4.3.3`); format regex `^\d+\.\d+\.\d+\.\d+(?:-Preview\.\d+)?$`.
- `build.ps1` dot-sources `.local-build.ps1`; `verify-workspace.ps1:56-59` also fails if `build.ps1` exposes the private tool-dir name — that literal is written as `"portable" + "-toolchain"`, so plain grep misses it.
- Pinned-asset download (`Get-PinnedAsset`/`Get-VerifiedAsset`): fetch to `*.download` → sha256 vs `dependencies.json` → rename; cache hits re-hashed, refreshed on mismatch.
- `linux/build.sh` does more than locate tools: synthesizes temp `icon.png`, writes the `.local-build.ps1` stub, owns the 7-name `KNOWN_WINDOWS_ONLY` cargo-test whitelist (heredoc literal, :24) — Windows-semantic failures outside the list fail the gate.
- Frontend gate order differs by host on purpose: `build.ps1` = install → test:stickers → test:install-gate → build; `build.sh run_bun` = build → tests. Don't "harmonize".
- Generators pin upstream commits as literals (ByMykel `342d496`, SteamTracking `88d5684`, cosmetic API `c3953df`) and emit sha256 into `stickerCatalog.source.json`; the gate recomputes hashes + counts (35 placements / 81 charms / 35 CT + 44 T agents).

## ANTI-PATTERNS
- `Assert-ChildPath` guards ONLY `$stage`/`$extract` under `.cache/package` (:88-89); `package.ps1` also writes `-OutputDirectory` (default `artifacts/`) and copies into `Panel/src-tauri/target/release`. Never claim it "mutates nothing outside `.cache/package`".
- `generate-player-cosmetic-placements.mjs` re-run throws ENOENT — its inputs under `plugins/BotRandomizer/` are deleted (0 tracked files). Gate only asserts output counts + `charmAnchors`; don't regenerate without new inputs.
- Layout/package change must update all three `verify-workspace.ps1` allowlists: `$requiredSources` (166-189), `$requiredPackageFiles` (326-391), `$forbiddenInPackage` (396-438).
- Never strip the OpenRating holdout assertion in `package.ps1` (release_gate.passed, dataset_sha256, holdout MAE/spearman bounds, 7 finite weights).
- `-Explode` requires `-Linux` and a previously built native Panel binary.
- Never pipe a gate through `tail` — it swallows exit codes (`run_cargo` tees to a log and greps instead).
- `linux/*.sh` run `set -uo pipefail` WITHOUT `-e` by design: failures accumulate via `step()` for a full PASS/FAIL summary. Don't "fix" to `set -e`.
- Dotnet drift: `build.sh run_dotnet` builds OMT, but `package.ps1:149-203` consumes `plugins/OfflineMatchTelemetry/stage/...` which only `dotnet publish` (StageForGameDeploy) produces — the gate suite alone doesn't refresh that stage.
