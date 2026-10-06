# Upstream Policy

## Base

- Project: `ed0ard/CS2-Bot-Improver`
- Synced upstream source commit: `d9914454d8691b509bae4cd671a7a8bd9dcb174f` (upstream `main` = tag `v1.4.5`, 2026-10-02)
- Last synced upstream release: `d9914454d8691b509bae4cd671a7a8bd9dcb174f` (`v1.4.5`), the newest upstream tag
- Pinned Windows runtime asset: `CS2BotImprover.zip` (`v1.4.5`, see `scripts/dependencies.json`)
- Plus release line: `1.4.3.x` test line; the merged payload tracks the upstream `v1.4.5` enhanced-bot submodules
- `scripts/dependencies.json`: `upstream.baseCommit` stays on the pinned release commit, `upstream.sourceCommit`
  follows the upstream revision whose sources are merged. Packaging verification asserts every submodule gitlink and
  upstream-owned tree against `sourceCommit`, so bump it whenever a sync merge lands.

The repository stores source and configuration deltas. Upstream has published the `v1.4.5` release archive, and the
Windows package script obtains that official layout, then overlays the pinned submodule builds, BotHider v0.4.0 data,
pinned engine-compatible runtimes, and current Plus builds instead of committing generated or third-party binaries.

### 2026-09 v1.4.4 merge notes (historical)

- Adopted upstream `BotAI`, `BotState`, `BotControllerImpl`/`BotHiderImpl`, the `NadeSystem` partial-class rewrite,
  the steamid-keyed `bot_info.json` schema, `BotHider`/`gamedata.json`, and the new `BotVision`/`BotController`
  metamod payload (arrives from the pinned release archive).
- Kept Local Arena versions: `Panel`, `README*`, `Commands.txt`, `BotRandomizer` (feature-gated skins/agents/music
  pipeline), `BotAimImprover`/`BotBuy` custom builds, and the `NadeSystem` disconnected-pawn guard (now in
  `NadeSystemPlugin.Replay.cs`).
- Difficulty `overrides` DB mirrors are re-extracted from the normalized `v1.4.4` payload VPKs, so they carry the
  new v1.4.4 pros (e.g. `forsaken`) while keeping valid `LookAngle` values.
- Roster reconciliation: the pinned `v1.4.4` botprofile data no longer defines `Techno4K`; the Local Arena
  `The MongolZ` five-man lineups (`match_catalog.json`, `Commands.txt`, Panel command browser) now use `Senzu`,
  which exists in the shipped difficulty data, so packaging verification passes.

### 2026-09 upstream `main` merge notes (historical)

Merge base was the `v1.4.4` source commit, so the merge carried only upstream's 27 post-release commits. Adopted
upstream's `BotRandomizer` `1.3.2` revision rather than the older Local Arena copy:

- Team-intro publication: one frame after `round_prestart`, while `TeamIntroPeriod` is active, `ApplyIntroAgents` /
  `ApplyIntroAgentsForTeam` / `ApplyIntroAgent` publish each bot's rolled agent on the
  `team_intro_counterterrorist` / `team_intro_terrorist` preview entities (`m_agentItem`). Preview slots are matched
  to bots by `Xuid` first and the remaining `Xuid == 0` slots are then paired with the remaining bots by `Ordinal`;
  human-owned slots are never written to. The intermediate `TeamIntroPreview.cs` / `m_xuid` variant that upstream's
  1.3.2 commit introduced is not part of upstream's current revision (PR #141 removed it), so it is not adopted here.
- `AgentDefinition` (agent model plus economy definition index) replaces the plain model lists, and
  `BotCosmeticLoadout` carries `AgentDefIndex`, so the intro entity and the spawned pawn can never disagree.
- The economic-attribute writer signature follows upstream's refresh. If cosmetics ever stop applying after a game
  update, check the `[BotRandomizer] SetOrAddAttributeValueByName signature failed` log line before changing code.
- Local Arena feature gates are preserved and re-applied on top of the upstream revision: `LoadOptions`,
  `bot_randomizer_options.json` (`skins` / `profiles` / `agents` / `music`), `EnabledScope`, the
  `HasVisibleFeatures` early-outs, and the per-feature guards. Intro publication is gated on `agents`, so the
  option semantics are unchanged from the previous Local Arena build.
- Deliberate deviation: `BotRandomizer.csproj` compiles against `CounterStrikeSharp.API` `1.0.373`, not upstream's
  `1.0.375`, because the packaged runtime is pinned to CounterStrikeSharp `v1.0.373` in `scripts/dependencies.json`.
- Roster: upstream's `JBOEN` (FaZe) and `max` (9z) botprofile entries are adopted in all three difficulty mirrors;
  `Commands.txt` already referenced both players.
- Files upstream still owns stay dropped in Local Arena: `.github/ISSUE_TEMPLATE/*`, `docs/README.ru.md`,
  `docs/README.zh-CN.md`, and upstream's `README.md` rewrite. Local Arena keeps its own `README.md` /
  `README.zh-CN.md`.
- Known data gap, inherited from upstream: seven `Commands.txt` players still have no botprofile entry anywhere
  (`HObbit`, `MATYS`, `S1ren`, `b1t`, `dav1deus`, `doc`, `flayy`). Upstream's own database lacks them too, so no
  profile values were invented; they fall back to the game's default profile. `verify-workspace.ps1` exempts exactly
  this list from the featured-player assertion through `$inheritedProfileGap`.

### 2026-10 upstream `v1.4.5` merge notes (historical)

> Superseded on 2026-10-06 by the download-and-merge rework (see `PLAN-upstream-download-install.md` and the
> sections below): eight of the nine submodules and the duplicated upstream trees were removed from this
> repository (only `addons/BotHider` remains), `package.ps1` no longer packages any upstream archive, and
> `verify-workspace.ps1` now asserts the LA-only package whitelist. The pin table below is retained as the
> historical record of the merge-time pointers.

Merge base was the previous sync commit (`abb2c0f`, post-`v1.4.4`); the merge adopts upstream `main` at `d991445`
(tag `v1.4.5`, 2026-10-02). Upstream converted its enhanced-bot sources into git submodules and Local Arena adopts
that structure verbatim. Every pointer must stay byte-identical to the pinned commit: `verify-workspace.ps1`
compares all nine gitlinks plus the upstream-owned trees (`RoundDamageRecap`, `plugins/disabled`,
`counterstrikesharp/data`) against `sourceCommit` and fails on any divergence.

| Submodule path | Upstream repository | Pinned commit |
|---|---|---|
| `addons/BotHider` | `XBribo/CS2-Bot-Hider` | `12069c25a756deedc72f8fc116ac3b18045d1223` |
| `addons/BotController` | `XBribo/CS2-Bot-Controller` | `451c7ba8ddb007eeb4d72edf291f6e0cbb051e56` |
| `addons/BotVision` | `XBribo/CS2-Bot-Vision` | `33ab01fc5ec4e27b6f07aff653d54638ad84ebcb` |
| `addons/counterstrikesharp/plugins/BotAI` | `ed0ard/CS2-BotAI` | `1bcd6dbe310fc059081baa9b56f6145b8a628967` |
| `addons/counterstrikesharp/plugins/BotAimImprover` | `ed0ard/CS2-Bullseye-Bot` | `c3d10f5f5e31302589032bb579dfc342c9ea5639` |
| `addons/counterstrikesharp/plugins/BotBuy` | `ed0ard/CS2-Bot-Buy` | `a4e8fded12dda5353e5c36eb2ff8cddd11d5193e` |
| `addons/counterstrikesharp/plugins/BotRandomizer` | `ed0ard/CS2-Bot-Randomizer` | `5b16e1447f4e5d3032a1625ac368c90f81b929ac` |
| `addons/counterstrikesharp/plugins/BotState` | `ed0ard/CS2-Smarter-Bot` | `30e791f2f615452b3403f352e731116632a71887` |
| `addons/counterstrikesharp/plugins/NadeSystem` | `ed0ard/CS2-Bot-NadeSystem` | `21788dc3fb61ca62db6a14b9657fd61cbbc888f1` |

Adopted with the merge:

- Upstream's new content: the five `cfg` rush files, the nine updated `gamemode_*.cfg` files, the four
  behavior-tree overrides (`overrides/scripts/{Low,Medium,High}/bt_config.kv3`, `overrides/scripts/bt_default.kv3`),
  the `README` 1.4.5 hunks, and upstream's revised `RoundDamageRecap` (net8.0, CSS 1.0.367).
- The submodule structure: the former in-repo copies of `BotHiderImpl`, `BotControllerImpl`,
  `shared/BotControllerApi`, `shared/BotHiderApi`, `addons/metamod/*.vdf`, and `addons/BotHider/*.json` are gone.
  Committed BotHider data now lives at `addons/BotHider/configs/addons/BotHider/`.
- CounterStrikeSharp moves from `v1.0.373` to `v1.0.376`: the runtime upstream's `v1.4.5` payload ships (verified
  byte-identical core DLL) and the maximum compile-time pin among the submodules (`BotRandomizer` 1.0.376). The
  pinned `windowsCoreSha256` follows.

Local modifications dismantled by the submodule conversion (functional losses versus the previous Local Arena build):

- `BotBuy`: the PLUS purchase gate (`ManagedMatchRuntimeStore.IsPurchasingAllowed`) and purchase telemetry hooks.
- `RoundDamageRecap`: the PLUS statistics yield (`PlusManagedPaths.ActiveMatchPath` ownership).
- `BotAimImprover`: the managed schema-targeting build; upstream's signature-hook `2.1.3` ships instead.
- `NadeSystem`: the disconnected-pawn guard in `NadeSystemPlugin.Replay.cs`.
- `BotHiderImpl`: the forced `EnsureBotInfoNameSource()` bootstrap; upstream ships the `bh_namesource` console
  command instead, and both Local Arena `my_bot_*` configs still set `bh_namesource 1`, so the name chain now
  survives through configuration rather than a forced initialization.
- `BotRandomizer`: the `bot_randomizer_options.json` feature gates (`LoadOptions` / `EnabledScope` /
  `HasVisibleFeatures`); the options file is neither read nor shipped any more.
- `map_whitelist.json` (no consumer remained) and `plugins/disabled/CS2_ExecAfter` (deleted upstream).
- `bot_info.json` no longer carries the local `vsm` / `2035477657` identity entry.

Tooling adaptations:

- `verify-workspace.ps1`: the module diff is replaced by gitlink/upstream-tree/removed-path assertions; upstream
  content checks and the dropped-mod gates are gone; the repo grenade catalog is read from
  `addons/counterstrikesharp/data/NadeSystem/grenades`; the built-plugin list follows the measured TFMs
  (`RoundDamageRecap` net8.0, `PlusMatchCoordinator` and `TeamLineupInjector` net10.0, `BotControllerImpl` built
  from `addons/BotController/csharp/BotControllerImpl`); the documented profile gap is exempted through
  `$inheritedProfileGap`.
- `package.ps1`: the payload base bumps to the `v1.4.5` zip; BotHider data is sourced from the submodule `configs`
  tree; build outputs come from the submodule `csharp` trees; explicit copies carry the NadeSystem grenade catalog
  (`data/NadeSystem/grenades` → payload `plugins/NadeSystem/grenades`) and the behavior-tree overrides, both of
  which are absent or stale in the release zip; `bot_randomizer_options.json` staging is removed.
- `build.ps1`: impl project paths point into the submodules, and the unused `RayTraceApiPath` MSBuild plumbing was
  removed together with the reverted `BotAimImprover`/`NadeSystem` csproj edits.
- `installer.rs`: `disabled/CS2_ExecAfter` is dropped from `SUITE_OWNED_ROOTS`.
- `.github/workflows/build.yml` and `release.yml` check out with `submodules: recursive`.

## Pinned Runtime Inputs

The machine-readable source of truth is `scripts/dependencies.json`.

- The upstream runtime is no longer pinned into the package. The Panel resolves the latest
  `ed0ard/CS2-Bot-Improver` release through the GitHub API at install time and verifies `CS2BotImprover.zip`
  against the API-provided SHA-256 `digest` before merging it; the rate-limit fallback redirect and manually
  imported local ZIPs are HTTPS-only without a digest (accepted residual risk, plan decision #5).
- MetaMod, CounterStrikeSharp, RayTrace, and the BotHider Windows archive are no longer downloaded or
  verified by the build pipeline; they all ship inside the upstream release zip that the Panel downloads.
- The Local Arena package contains only Local Arena's own content: the four exclusive plugins,
  `cfg/my_bot_ffa_config.cfg`, `cfg/my_bot_normal_config.cfg`, `overrides/scripts/`, the payload manifest,
  the Panel executable, and the docs. `verify-workspace.ps1` enforces this whitelist and rejects upstream
  files in the package.
- `PlusMatchCoordinator`, `TeamLineupInjector`, `PlayerKnifeCustomizer`, `MatchCore`, and
  `OfflineMatchTelemetry` still build from source and overlay the merged payload at install time; the
  `addons/BotHider` submodule stays pinned at the commit recorded in `scripts/dependencies.json` and supplies
  `BotHiderApi` for the two dependent plugins.
- The Panel merges the downloaded upstream zip with its own payload into a staging directory and installs
  from the generated merged manifest, so upstream files are installed, backed up, and restored like any
  other payload files.
- The upstream `Panel v*.exe` shipped inside the release zip is relocated next to `LocalArena.exe` and can be
  launched optionally from the Installation management page.
- Local Arena's own panel/plugin update artifacts remain signed and SHA-256 verified through `latest.json`;
  only the upstream zip follows the digest/HTTPS policy above.

## 2026-10 Download-and-Merge Architecture

`PLAN-upstream-download-install.md` in the repository root records the full design; the essentials:

- CI no longer downloads any pinned runtime asset; `package.ps1` stages Local Arena-owned files only, and the
  release zip shrank from 73 MB to a few megabytes.
- `Panel/src-tauri/src/upstream_package.rs` resolves, downloads (resumable, digest-verified when available),
  caches, and merges the upstream release; `merged_payload_root` feeds install, inspect, repair, and restore.
- Eight upstream submodules, `RoundDamageRecap`, `plugins/disabled`, `counterstrikesharp/data`, and the
  duplicated `cfg` files were removed from the repository; RayTrace support was removed end to end.
- Settings → Installation gained an upstream package management section (download, cache, offline ZIP import,
  installed-state display, page-triggered install and repair).

## Synchronizing

1. Fetch `upstream/main` and inspect release notes, issues, and relevant PRs.
2. Rebase or merge in an isolated branch.
3. Preserve Plus-only modules and Panel routes.
4. Reconcile I18N by keeping the entire new upstream key/dictionary set, then reapply Plus keys and translations.
5. Refresh catalogs only from a traceable source and verify locale and entry counts.
6. Build all three targets and create a disposable package before updating the pinned manifest.

## Third-Party Data

Weapon images and localized skin names are derived from `Nereziel/cs2-WeaponPaints`. Indonesian currently uses the
English fallback because that source does not provide an Indonesian skin-name table. This fallback affects display
only; item application uses numeric catalog identifiers.

BotHider is maintained at `XBribo/CS2-Bot-Hider`. The package tracks `v0.4.0`, which supplies the current Windows
identity synchronization, team-join scope, entity-packing protection, and gamedata-driven
`CServerSideClient::SetName` target. Packaging verifies the official release archive and native DLL hashes without
binary patching.

`BotHiderImpl` is built from the pinned `CS2-Bot-Hider` submodule and supplements native name publication only for
slots reported by BotHider as managed bots, through `CBasePlayerController.m_iszPlayerName`. The upstream
`bh_namesource` console command selects the `bot_info.json` display-name source, and both Local Arena `my_bot_*`
configs set `bh_namesource 1`. It does not write names for human-player slots. Steam IDs, avatars, cards,
crosshair codes, ping, scoreboard flair, bot disguise, respawn behavior, and every upstream enhanced-bot module stay
on their existing paths. The repository can verify this isolation and the package layout automatically, but the final
host-local scoreboard result still requires an in-game Enhanced Bots practice match.
