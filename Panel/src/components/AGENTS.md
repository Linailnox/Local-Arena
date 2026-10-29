# AGENTS.md — src/components (shared UI primitives)

Score 12 — shared primitive layer consumed by App + every panel; distinct domain
(build-once, compose-everywhere). 19 `.tsx` + 17 colocated `.css` (`Card.tsx` and `icons.tsx` have no css).

## OVERVIEW
Build-once shared primitives; every panel composes these instead of importing UI libraries.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Toasts | `Toast.tsx` | `ToastProvider` mounted in `main.tsx`; named `useToast()` hook |
| Icons | `icons.tsx` | 11 named icons (Gear, Min, Close, ChevronRight/Down/Up, Back, Copy, External, Refresh, Alert) — single module, changes ripple widely |
| Modals | `Modal.tsx` (fan-in 12), `ErrorModal.tsx` | `ErrorModal` hardcodes `KNOWN_CATS` + `GUIDE_ANCHORS` (guide-issue-1..5) — must track Rust error codes |
| Status | `StatusDot.tsx` (`type Status`), `StatusBar.tsx`, `Toggle.tsx` (`type Tone`) | |
| Layout | `Section`, `Card`, `Collapsible`, `Segmented` (`type SegOption<T>`), `Dropdown`, `SubPage`, `TitleBar` | `TitleBar` carries the `data-tauri-drag-region` frameless drag |
| Cosmetics | `CosmeticsTeamSwitch.tsx` (+ `useCosmeticsTeam`), `CosmeticsPresetActions.tsx`, `CosmeticsSourceCard.tsx` | team switch used by all 4 preset modals + StickersPanel |
| InventorySimulator plugin card | `InventoryPluginCard.tsx` | download/update card, rendered by `settings/InstallationPage.tsx` |
| Skin float | `WearGauge.tsx` | wear ranges for weapon preset modals |

## CONVENTIONS
- COMPOSE EXISTING PRIMITIVES — never add UI/component npm packages and never inline ad-hoc SVGs; extend `icons.tsx` instead.
- Primitives are intentionally tiny (Toggle 35, WearGauge 32 LOC); variants via small union types (`Status`, `Tone`), no props-styling system.
- One default-export component per file + same-named `.css`; named exports only for hooks/types.
- No business logic here — data/logic belongs in `lib/` or `panels/`.
- `Segmented`/`Dropdown` power settings-page option rows; `SubPage` frames the settings subpages.

## ANTI-PATTERNS
- Dropdown popovers must never be clipped by card overflow (`Dropdown.css` hard rule).
- `ErrorModal`'s `KNOWN_CATS`/`GUIDE_ANCHORS` drifting from Rust `error.rs` codes = wrong help links; update together.
- Don't grow `StatusBar`/`ErrorModal` into page logic — they are shells fed by the store.
