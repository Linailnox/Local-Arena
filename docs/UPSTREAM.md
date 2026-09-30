# Upstream Policy

## Base

- Project: `ed0ard/CS2-Bot-Improver`
- Synced upstream source commit: `abb2c0fc34e48183048134cfca0d2021db6b8d45` (upstream `main`, post-`v1.4.4`, 2026-09-25)
- Last synced upstream release: `7491e175f83e612dbb1c742c2241d454ed4c15ad` (`v1.4.4`), still the newest upstream tag
- Pinned Windows runtime asset: `CS2BotImprover.zip` (`v1.4.4`, see `scripts/dependencies.json`)
- Plus release line: `1.4.3.x` test line; the merged payload tracks upstream `main` BotRandomizer plus the `v1.4.4` enhanced-bot sources
- `scripts/dependencies.json`: `upstream.baseCommit` stays on the pinned release commit, `upstream.sourceCommit`
  follows the upstream revision whose sources are merged. Packaging verification diffs the upstream-owned modules
  against `sourceCommit`, so bump it whenever a sync merge lands.

The repository stores source and configuration deltas. Upstream has marked its Panel and source tree as 1.4.4 and has
published the `v1.4.4` release archive. The Windows package script obtains the official v1.4.4 layout, then overlays the
synced sources, BotHider v0.4.0 data, pinned engine-compatible runtimes, and current Plus builds instead of committing
generated or third-party binaries.

### 2026-09 v1.4.4 merge notes

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

### 2026-09 upstream `main` merge notes

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
  profile values were invented; they fall back to the game's default profile.

## Pinned Runtime Inputs

The machine-readable source of truth is `scripts/dependencies.json`.

- `CS2BotImprover.zip` supplies the official Windows runtime layout.
- MetaMod 2.0.0-git1406 supplies the engine 26 loader.
- CounterStrikeSharp v1.0.371 with its bundled .NET runtime replaces the stale v1.4.1 copy.
- RayTrace v1.0.16 supplies both the native module and CounterStrikeSharp API/implementation.
- `BotHider-windows-0.3.0.zip` supplies the native BotHider module.
- BotAI includes the tested Windows signature refresh from upstream PR #75 (`3db93ba`).
- BotAI, BotAimImprover, BotBuy, and NadeSystem are rebuilt from the pinned source tree so post-v1.4.1 fixes are not
  replaced by older release DLLs.
- BotAimImprover and NadeSystem receive `RayTraceApi.dll` from the verified v1.0.16 archive through an explicit
  MSBuild property; clean builds do not depend on an ignored `libs` file left on the developer machine.
- Plus-built `BotHiderImpl`, `BotHiderApi`, and `PlayerKnifeCustomizer` assemblies overlay their upstream locations.
- The Plus Panel replaces the upstream Panel executable while retaining the same standalone workflow.

Every downloaded archive and each critical runtime DLL is SHA-256 verified before packaging.

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

BotHider is maintained at `XBribo/CS2-Bot-Hider`. The package tracks v0.3.0, which supplies the current Windows
identity synchronization, team-join scope, entity-packing protection, and gamedata-driven
`CServerSideClient::SetName` target. Packaging verifies the official release archive and native DLL hashes without
binary patching.

`BotHiderImpl` supplements native name publication only for slots reported by BotHider as managed bots, through
`CBasePlayerController.m_iszPlayerName`, and forces the Plus `bot_info.json` name source before bots are created. It
does not write names for human-player slots. Steam IDs, avatars, cards,
crosshair codes, ping, scoreboard flair, bot disguise, respawn behavior, and every upstream enhanced-bot module stay
on their existing paths. The repository can verify this isolation and the package layout automatically, but the final
host-local scoreboard result still requires an in-game Enhanced Bots practice match.
