# src/lib KNOWLEDGE BASE

## OVERVIEW
Score 9 + distinct domain: the Tauri invoke boundary plus the only unit-tested
frontend logic. 11 files, 1,827 LOC; `api.ts` (901) is imported by 41 modules.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Add a backend call | `api.ts` | single invoke boundary — one `invoke<...>` wrapper method per Rust command (92 call sites, 88 `export type` DTOs); grep-verified: zero `invoke(` calls exist outside this file |
| Cosmetics inventory editing | `cosmeticsInventory.ts` (170) | `TEAM_KEY {ct:"3", t:"2"}` + typed getters/setters over the inventories document; fan-in 7 |
| Dialogs/clipboard/shell/events | `platform.ts` | Tauri plugin wrappers (`openDialog`, `writeClipboard`, `openExternalUrl`, `listenAppEvent`) — no direct `window.*` browser calls |
| Appearance engine | `appearance.ts` (301) | writes ~40 CSS custom properties per apply, injects custom-font `<style id="local-arena-custom-font">`, data-URL validator cache (`DATA_URL_CACHE_LIMIT` 6) |
| Team palettes | `teamThemes.ts` | `TEAM_THEMES`, `applyTeamTheme` (sets `--c-ink` etc. per side) |
| Sticker editor model | `stickerEditor.ts` | `STICKER_SLOT_COUNT=5`, `paginate`; exercised by `test:stickers` |
| Install gating | `installGate.ts` | process state is advisory; `upstreamReady` defaults `true` |
| Install-check texts | `installCheckLocalization.ts` | maps `install_checks.rs` result codes → i18n keys |
| Global hotkeys | `keycapture.ts` | `captureKeyName` |
| Picker scroll sync | `useSelectedPickerScroll.ts` | used by all four cosmetic preset modals (glove/weapon/knife/music-kit) |
| Version | `version.ts` | `APP_SEMVER`/`APP_DISPLAY_VERSION` sourced from `Panel/package.json` (`1.4.3+3` → displays `1.4.3.3`) |

## CONVENTIONS
- `toAppError`/`isAppError` normalize every failure to stable machine-readable codes (TS mirrors Rust `error.rs`); codes are never localized prose — `errcat.*` i18n keys resolve them.
- DTO edits are three-way: `api.ts` type ↔ Rust serde struct ↔ consuming panel; `tsc` catches only the TS side.
- `appearance.ts` is the frontend half of `src-tauri/src/appearance.rs` (its 4 commands) — change both sides together.
- `cosmeticsInventory.ts` keeps Panel-only fields (`enabled`, `shared_weapon_links`, `default_knife`, ...) in a `_panel` sidecar object the plugin ignores.

## ANTI-PATTERNS
- NEVER call Tauri `invoke` from a component/panel — extend `api.ts` instead.
- Do not hand-roll error prose from raw invoke failures — route through `toAppError` or `errcat.*` keys stay unresolved.
- Do not mutate the `api` object outside `dev/SettingsPreview.tsx` (its module-scope monkey-patching is safe only because `main.tsx` dynamic-imports it under `?settings-preview=1` in DEV).
