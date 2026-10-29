# AGENTS.md — src/panels (feature views)

Score 12 — every user-facing view; distinct domain from shared primitives.
42 tracked files (29 `.tsx` + 13 `.css`), all mounted from `src/App.tsx`
(`type View = "main" | "stickers" | DashboardTarget`); no router.

## OVERVIEW
Every user-facing feature view; navigation is App.tsx state, not a router.

## WHERE TO LOOK
| User flow | Location | Notes |
|-----------|----------|-------|
| Home tiles → navigate | `OverviewDashboard.tsx` | exports `DashboardTarget` — the app's de-facto route enum |
| Bot presets (aim/nades/difficulty) | `PresetsPanel.tsx` | renders `BotItemsPanel` inline; nade `value: "less"` is gate-pinned |
| Cosmetics hub | `WeaponPresetsPanel.tsx` | owns the 4 preset modals + `CosmeticsPresetActions` + `CosmeticsSourceCard` |
| Skin / knife / glove / music-kit pickers | `WeaponPresetModal.tsx`, `KnifePresetModal.tsx`, `GlovePresetModal.tsx`, `MusicKitPresetModal.tsx` | write the InventorySimulator doc via `lib/cosmeticsInventory`, persist `api.saveCosmeticsInventory` |
| Sticker / charm workshop | `StickersPanel.tsx` | over `data/cosmeticPlacements.json` + `data/charmCatalog.json` |
| Match setup / live / results | `MatchPanel.tsx` | results branch renders `StatsMatchDetail`; reachable only from a real finished match — cannot be rendered offline |
| Stats | `StatsDashboard.tsx` → `StatsMatchHistory` / `StatsMatchDetail` / `StatsPlayerDetail` / `StatsDMDetail` | sub-view switch `dashboard`/`history`/`matchDetail` |
| Guide walkthrough | `GuideView.tsx` | 16 screenshots, anchor-scrollable |
| Console command viewer | `CommandsPanel.tsx` | read-only, over `data/commands.ts` + `data/commands.txt` |
| Settings | `settings/SettingsView.tsx` + `*Page.tsx` | `InstallationPage.tsx` (529 LOC, 12 api calls) = install gate + upstream download + `OnlineUpdateSection` + `InventoryPluginCard`; `FirstRunLanguages.tsx` is mounted directly by `App.tsx` |

## CONVENTIONS
- Adding a view = panel file + `DashboardTarget` member + `VIEWS` entry (`App.tsx:39`) + App branch; panels receive callbacks only (`onNavigate`, `onBack`, `anchor`, ...).
- `SettingsView` is a two-level state machine (`root` | SettingsEntry | AboutTarget); a new subpage must join `TITLE_KEYS`/`DESC_KEYS`/`ICONS`/`STATUS` and the render switch.
- Dynamic i18n keys use the accepted `as never` escape hatch — only `MatchPanel.tsx:279` (`match.${value}`) and `StatsMatchDetail.tsx:316` (`match.demoState.${state}`); don't "fix" without extending `I18nKey`.
- UI prefs persist in localStorage with the `cs2bi.` prefix (`cs2bi.matchDifficulty`, `cs2bi.matchDemoV2`).
- Silent startup catches are deliberate — errors surface via the store error modal, not per-panel alerts.
- CSS pairing: `StatsPanel.css` shared by the 5 stats files; `KnifePresetModal.css` by the 4 preset modals; `settings/settings.css` (1930 LOC) by 6 settings pages + `FirstRunLanguages`; one-to-one elsewhere.
- Panels with zero `api.` calls (PresetsPanel, SettingsView, stats leaves, about pages) read `useStore()` only.

## ANTI-PATTERNS
- `pwsh scripts/verify-workspace.ps1` fails if: MatchPanel loses the GOTV opt-in (`cs2bi.matchDemoV2` check + `match.demoWarning`); any of the 5 cosmetics sources (`KnifePresetModal`/`GlovePresetModal`/`WeaponPresetModal`/`MusicKitPresetModal`/`StickersPanel` + its css) is deleted/renamed; PresetsPanel drops `value: "less"`.
- `TeamsSection.tsx` is an orphan — no importer anywhere in `src/`; don't extend it.
- `settings.css` (1930 LOC, 6 pages) and `StatsPanel.css` (857, 5 panels) have no automated gate — a rule added for one page silently restyles the others.
- Large catalogs import eagerly (`weaponSkins.json` 323 KB + `skinImages.json` 414 KB in the preset modals; no lazy `import()` anywhere) — import changes move main-bundle size.
