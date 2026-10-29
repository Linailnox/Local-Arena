# src/data KNOWLEDGE BASE

## OVERVIEW
Score 16 (>15): 28 files / ~7.1 MB of static game-data catalogs, all compiled
into the Vite bundle — zero runtime fetching, zero Tauri calls. No barrel file:
imports are exact-path (`../data/<name>`).

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Stats backend contract | `cs2ssTypes.ts` (329 LOC) | ~20 `Cs2ss*` interfaces mirroring Rust serde shapes; 7 importing views |
| Rating math | `cs2ssRating.ts` | `cs2ssCalcKd/Kda/Adr/Kast/HsPct/WinRate`, `cs2ssRatingBreakdown` |
| Map/reason/result labels | `cs2ssMaps.ts` (25 maps), `cs2ssReasons.ts` (`MATCH_DECIDED`, `MAP_END` → I18nKey), `cs2ssResult.ts` (W/L/D colors) | `cs2ssMapLabel` has 6 importers |
| Skin name localization | `skinLocalization.ts` | `localizedSkinName`/`finishName`/`itemName` over `skinNames.json` (28 locales) |
| Weapon/knife icons | `weaponIcons.ts`, `knifeIcons.ts` | wrap `skinImages.json` (404 KB) |
| Sticker catalog | `stickers.ts` → `stickerCatalog.json` | 10,555 entries |
| Music kits / gloves / charms / agents | `musicKits.ts`, `gloveSkins.ts`, `charmCatalog.json`, `agentCatalog.json` | 101 kits / 91 gloves / 81 charms / 79 agents (35 ct + 44 t) |
| Bot console script | `commands.ts` (119 LOC) | parses `commands.txt` (312 lines, Vite `?raw`) into `TEAMS`/`TEAM_META`; renaming a section header silently empties `TEAMS` — no test covers it |
| Avatars / team logos | `matchVisuals.ts` | FNV-1a hash assigns 6 avatars/team; changing list order reshuffles existing match views |
| Languages / credits | `languages.ts` (29 Steam codes), `devs.ts` (bilingual, inline in TS) | |

## GENERATED FILES (never hand-edit)
| File | Size | Generator (run from repo root) |
|------|------|-------------------------------|
| `stickerCatalog.json` | 4,201,790 B / 10,555 entries | `node scripts/generate-sticker-catalog.mjs` (network; also writes the two `.source`/ids outputs) |
| `stickerWeaponIds.json` | 345 B / 35 ids | `node scripts/generate-sticker-catalog.mjs` |
| `cosmeticPlacements.json` | 21,300 B / 35 weapons | `node scripts/generate-player-cosmetic-placements.mjs` (reads `agentCatalog.json`) |
| `stickerCatalog.source.json` | 883 B | provenance record: upstream commit + input/output sha256 |

`scripts/verify-workspace.ps1` recomputes JSON counts + sha256 against `stickerCatalog.source.json` and greps the generator's pinned commit constants — drift fails the gate.

## CONVENTIONS
- Companion pattern: heavy `<name>.json` + thin same-basename `<name>.ts` re-exporting a typed array via `as` cast (`musicKits`, `stickers`, `gloveSkins`); JSON without a wrapper is imported raw by panels.
- Bilingual field naming is inconsistent BY SOURCE: `name_en`/`name_zh_cn` (stickers) vs `name_en`/`name_zh` (musicKits); `weaponSkins.json.name` is Chinese-only (localization via `skinNames.json`); glove tri-language model names live in a TS table, not the JSON.
- Dual-consumed JSON: `stickerCatalog.json`, `stickerWeaponIds.json`, `agentCatalog.json` are `include_str!`-embedded in Rust (`src-tauri/src/lib.rs:527/538/662`) for preset-validation allowlists — a schema change breaks `cargo check`, not just tsc.
- Catalog images are remote CDN URLs (steamstatic, Nereziel/cs2-WeaponPaints raw), never vendored; only player avatars/team logos are local `/public` paths.

## ANTI-PATTERNS
- NEVER hand-edit the four generated files above — regenerate; `verify-workspace.ps1` fails on any drift.
- NEVER open `stickerCatalog.json` (4.0 MB) or `skinNames.json` (2.0 MB) whole — one minified line each; inspect with `jq`/`head -c`. Together they dominate the Vite bundle (~6.3 MB of ~7.1 MB).
- Hard count gates: 35 sticker weapons, 35 ct / 44 t agents, 81 charms (78 with images), 35 placement entries, 10,555 stickers — `test:stickers` and verify assert them.
- Image-host allowlist: every agent/charm image must start `https://community.akamai.steamstatic.com/economy/image/` (`scripts/test-sticker-editor.mjs`).
- `test-sticker-editor.mjs` and `scripts/migrate-cosmetics-to-inventory.ps1` read `agentCatalog.json` directly — this dir IS tested; do not assume only tsc covers it.
