# Contributing to Local Arena

## Scope

Local Arena is independently maintained. Selected AGPL-3.0 components are derived from
`ed0ard/CS2-Bot-Improver` and may receive audited compatibility updates from that source. Keep changes focused on
Local Arena or compatibility with a current CS2 release, and preserve source attribution when adapting upstream code.

## Development Rules

1. Do not commit `bin`, `obj`, `node_modules`, `dist`, `target`, downloaded release archives, or third-party native
   DLLs.
2. Add every visible Panel string to `Panel/src/i18n/keys.ts`. Add locale overrides in
   `Panel/src/i18n/dictionary.ts`; never hardcode a new UI sentence in a component.
3. Keep saved cosmetic data language-independent. Persist numeric item definition indexes, paint kits, seeds, wear,
   and flags, not translated display names.
4. Keep normal matchmaking isolated from Metamod and player cosmetics. Changes to mode switching require Rust tests.
5. Preserve the package's upstream-compatible `game/csgo` copy layout.

## Verification

```powershell
bun install --frozen-lockfile --cwd Panel
bun run --cwd Panel build
cd Panel; bun tauri build --bundles nsis   # Windows host; CI uses tauri-action
D:\dotnet-sdk-10\dotnet.exe build addons\counterstrikesharp\plugins\BotHiderImpl\BotHiderImpl.csproj -c Release
.\scripts\package.ps1 -DotNet D:\dotnet-sdk-10\dotnet.exe
.\scripts\verify-workspace.ps1
```

The Panel bundle is produced by the Tauri CLI locally or by `tauri-action` in CI; `scripts/` no longer builds it.
`package.ps1` only builds the Local Arena plugins with dotnet and writes the plugin ZIP. Caches live under the
repository's ignored `.cache` directory.

Before publishing, run `scripts/package.ps1`, inspect the plugin ZIP file list, verify `SHA256SUMS.txt`, and smoke-test both
Enhanced Bots and Online Mode on a disposable CS2 installation copy.
