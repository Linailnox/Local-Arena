# AGENTS.md — Panel (Tauri desktop app)

## OVERVIEW
Score 17 — subtree root. Two halves: React 18 + TypeScript + Vite frontend (`src/`),
Rust backend (`src-tauri/`, crate `cs2-bot-improver-plus-panel`). Frameless window
(custom TitleBar), NSIS-bundled Windows app.

## WHERE TO LOOK
| Task | Location |
|------|----------|
| Add a view | `src/panels/`, then mount in `src/App.tsx` (imports every panel) |
| Add/rename a Rust command | `src-tauri/src/lib.rs` command fn + `generate_handler!`, then `src/lib/api.ts` |
| Frontend↔backend contract | `src/lib/api.ts` — the only `invoke` boundary |
| Global UI state | `src/state/store.tsx` (`useStore`, `AppStateProvider`; `AppStatePreviewProvider` renders panels without Tauri) |
| Theme state | `src/state/appearance.tsx` (`useAppearance`) |
| New user-facing string | `src/i18n/` — key in `keys.ts` EN map + `dictionary.ts` locale |
| UI primitive | `src/components/` |
| Game data catalogs | `src/data/` |
| CSS tokens / glass | `src/styles/tokens.css` + `glass.css` (`global.css` @imports both); `fonts.css` is imported by nothing — the `modern` font preset silently falls back to Segoe UI |
| Dev-only preview harness | `src/dev/` — `SettingsPreview.tsx`, `WorkshopBrowserPreview.tsx`; DEV-only dynamic imports gated by `?settings-preview=1` / `?workshop-preview=1` |
| Sticker / cosmetics helpers | `src/lib/stickerEditor.ts`, `src/lib/cosmeticsInventory.ts` (`TEAM_KEY` ct="3"/t="2") |

## CONVENTIONS
- Relative imports only; no path aliases.
- Plain colocated CSS: same-named `.css` next to each `.tsx`; no Tailwind, no CSS modules.
- Entry chain: `index.html` → `src/main.tsx` (ToastProvider → AppStateProvider → AppearanceProvider → `App.tsx`); `main.tsx` also owns window fit/zoom (1300×800 canvas, zoom 0.7–1.0) and global input policy (contextmenu blocked).
- `state/store.tsx` polls one `refreshAll(true)` snapshot every 2 s (paused when hidden) and mirrors `cs2bi.*` localStorage keys to the backend every 1 s on change; global error/unhandledrejection handlers route every failure into `ErrorModal`.
- `bun run build` (`tsc && vite build`) is the type-check authority — TS LSP is unreliable in this repo's environments.
- No `bun test` script. Frontend gates are only `test:stickers` and `test:install-gate`.
- Dependencies come from bun: `Panel/bun.lock` is the only lockfile — do not reintroduce `package-lock.json` (build.ps1/CI install with `bun install --frozen-lockfile`).

## COMMANDS
```bash
cd Panel
bun run dev                # vite dev server (enables src/dev previews)
bun run build              # tsc + vite — authoritative typecheck
bun run test:stickers      # node ../scripts/test-sticker-editor.mjs
bun run test:install-gate  # node ../scripts/test-install-gate.mjs
```

## ANTI-PATTERNS
- Frontend is never the authority on process/install state — Rust re-checks every write; `lib/installGate.ts` keeps `upstreamReady` defaulting to `true`.
- Don't delete the `--c-blue/--c-green/--c-orange/...` legacy aliases in `tokens.css` — they are live migration aliases, not dead code.
- Scrolling pages must keep children `flex: 0 0 auto` (`global.css` rule) or content clips with no scrollbar.
- `dev/SettingsPreview.tsx` monkey-patches `api` at import time — safe only as a DEV-only dynamic import, never eagerly.

## NOTES
- `src-tauri/` has its own AGENTS.md (cargo gates, merge/restore invariants, Linux build traps).
- Panel bun + dotnet steps are driven by the repo gate suite `scripts/linux/build.sh` (`--only <step>`) / `scripts/build.ps1`.
- Build outputs, never edit: `dist/`, `node_modules/`, `src-tauri/target/`, `src-tauri/gen/`.
