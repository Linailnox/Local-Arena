# shared (MatchCore + MatchCore.Tests)

## OVERVIEW
MatchCore: net8.0 classlib, 8 .cs / 553 LOC, zero package deps — roster selection and managed paths shared with PlusMatchCoordinator. MatchCore.Tests: console runner.

## WHERE TO LOOK
| Symbol | Role |
|--------|------|
| Contracts.cs | shared enums + data contracts (MatchSide, MatchDifficulty, MatchOpponentKind, MatchState, DemoState, PlayerKind, TeamSide) |
| MatchRules | MR12 + overtime + halftime side-swap logic — consumed ONLY by MatchCore.Tests; production MR12 lives in PlusMatchCoordinator convars |
| RosterSelector / RosterReconciler | unique-pick team selection / reconciliation |
| BotIdentityCatalog.Load | frozen roster catalog to BotIdentity (duplicates: first wins) |
| PlusManagedPaths, ManagedMatchRuntimeStore | csgo-root/runtime path resolution, purchase gating |
| BotAimPolicy, BotBuyPolicy | difficulty behavior knobs (all bot-behavior enums live in BotBehaviorPolicy.cs) |
| MatchJson | shared JsonSerializerOptions |

## CONVENTIONS
- Tests: OutputType=Exe, top-level statements, hand-rolled static Check(bool, msg) throwing InvalidOperationException; the runner ends by printing "MatchCore tests passed" (~30 checks). NOT dotnet test — there is no xunit anywhere.
- One domain per file (MatchRules.cs, RosterSelector.cs, ...) — add new public logic as its own file rather than growing Contracts.cs.
- Test temp files key on Environment.ProcessId and are deleted in finally — no timing waits, no nondeterminism.
- open-rating-3.0-proxy-v1.json lives HERE; the copy in PlusMatchCoordinator output is a csproj <None Link> artifact — edit only this one. No C# code reads it (rating math removed 2026-10-15; per-match stats come from the cs2ss telemetry DB); the model file and its packaging/install-check gate stay — deleting it is a packaging change, not a code change.
- Zero NuGet dependencies by design — keep it dependency-free so it builds under the net8 host constraint.

## ANTI-PATTERNS
- BotIdentityCatalog.cs:48 — duplicate catalog entries must never abort the match coordinator (first entry wins; see also plugins/AGENTS.md).

## NOTES
- Consumed via ProjectReference by PlusMatchCoordinator and the tests; OfflineMatchTelemetry and TeamLineupInjector do not reference it.
- MatchCore.Tests runs inside the dotnet gate step of scripts/linux/build.sh and scripts/build.ps1 — a failing Check() blocks the whole gate.
- net8.0 target is deliberate (CSSharp host runtime); do not bump it to match the net10.0 plugins.
- ManagedMatchRuntimeStore.Write is how PMC persists `<csgo>/.csbip/match-active.json` and `.csbip/aim-runtime.json` — Panel reads the same files.
