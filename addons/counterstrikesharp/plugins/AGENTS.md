# CSSharp Plugins (OfflineMatchTelemetry / PlusMatchCoordinator / TeamLineupInjector)

## OVERVIEW
Earned file: score 16 — three plugin domains sharing hosting conventions and verify-gated invariants. `PlayerKnifeCustomizer*/` and `RoundDamageRecap/` on disk are untracked/ignored build leftovers, not source.

## WHERE TO LOOK
| Task | Location |
|------|----------|
| Round lifecycle, roster settlement, SQLite telemetry | OfflineMatchTelemetry/OfflineMatchTelemetry.cs (1615 LOC) |
| Match orchestration, halftime side handling, roster catalog | PlusMatchCoordinator/PlusMatchCoordinator.cs (787 LOC) |
| Bot kick/lineup latch | TeamLineupInjector/TeamLineupInjector.cs (374 LOC) |
| Bot capability API consumed by PMC + TLI | ../../BotHider/csharp/BotHiderApi/IBotHiderApi.cs (submodule) |
| Frozen HLTV roster catalog (Panel contract) | PlusMatchCoordinator/match_catalog.json |

## CONVENTIONS
- One file per plugin: entire plugin in a single <Name>.cs, file-scoped namespace, no folder structure inside plugin projects.
- Mixed TFMs deliberately: OfflineMatchTelemetry net8.0 (CSSharp host + SQLite constraint), PlusMatchCoordinator/TeamLineupInjector net10.0.
- CounterStrikeSharp.API referenced with Private=false + ExcludeAssets=runtime, CopyLocalLockFileAssemblies=false — the game host provides runtime assemblies; never ship them in plugin output.
- BotHiderApi ProjectReference is also Private=false (assembly comes from the game-loaded BotHider plugin).
- Data files reach build output via csproj <None Link> into ../../shared/MatchCore or ../../../overrides/{Low,Medium,High}/botprofile.db — source of truth lives outside the plugin dir; PMC's open-rating json and profiles/ are linked build artifacts, not sources.
- OfflineMatchTelemetry + TeamLineupInjector define StageForGameDeploy (AfterTargets=Publish) copying output to stage/game/csgo/addons/counterstrikesharp/plugins/<Name>/.
- csproj paths are Windows-style (..\..\shared\...); Linux builds go through scripts/linux/build.sh.

## ANTI-PATTERNS (verify-gated regressions — do not reintroduce)
- OfflineMatchTelemetry.cs:326 — never pre-arm _round to the next round number.
- OfflineMatchTelemetry.cs:599 WriteRoundSummary — rounds with unsettled roster must be recorded, never silently dropped; once _matchSettled (set at :545, the winner panel) nothing may be appended to the ledger.
- OfflineMatchTelemetry.cs:1251 FixedTeamForSide — a decided round winner must never map to null.
- TeamLineupInjector.cs:312 — never issue mp_restartgame from cleanup; a disabled lineup must not reset a live match.
- MatchCore/BotIdentityCatalog.cs:48 — duplicate catalog entries: first entry wins, never abort the coordinator.
- scripts/verify-workspace.ps1 asserts these mechanically (plus halftime-side and lineup-latch invariants); failures block the build gate.

## NOTES
- match_catalog.json (schema_version 1, frozen HLTV top-20) is parsed by Panel Rust: Panel/src-tauri/src/match_system.rs CATALOG_RELATIVE; schema changes also touch installer/verify scripts.
- OfflineMatchTelemetry settles at the winner panel (_matchSettled, :545), not at session close; the completion marker `<csgo>/.csbip/cs2ss/last-completed.json` is written atomically (tmp file + File.Move, :585). The Panel watches it (match_system.rs COMPLETION_MARKER, event `cs2ss-match-completed`) and opens the results view only then — reading the telemetry DB earlier returns the previous match on the same map. Changing the marker shape means changing Rust `CompletedMatchMarker` and the Rust test that pins the wire JSON.
- OfflineMatchTelemetry carries Microsoft.Data.Sqlite; the Linux game package ships its libe_sqlite3.so (scripts/package.ps1 -Linux).
- CSSharp loads plugins by reflection — no code imports plugin projects; only MatchCore and BotHiderApi are ProjectReferenced.
- PlusMatchCoordinator encodes MR12 itself via convars (:267 `mp_maxrounds 24`/`mp_halftime 1`); `MatchCore.MatchRules` is consumed only by MatchCore.Tests, not the production rules source.
- ModuleVersion: OfflineMatchTelemetry 0.8.2, PlusMatchCoordinator 1.4.3.3 (tracks the release version), TeamLineupInjector 1.0.0. Handlers register imperatively in Load; only TLI carries `[MinimumApiVersion(301)]`.

## COMMANDS
```bash
scripts/linux/build.sh --only dotnet   # Linux gate: build all projects + run MatchCore.Tests
pwsh scripts/build.ps1                 # Windows equivalent
```
