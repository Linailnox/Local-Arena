# src/i18n KNOWLEDGE BASE

## OVERVIEW
Score 5 raw, kept as a distinct domain: the whole locale system lives in these
3 files (3,363 LOC, all TypeScript). No subdirectories.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Add/rename a string | `keys.ts` (1,072 LOC) | `EN` const: 1,065 flat dot-namespaced keys; `export type I18nKey = keyof typeof EN` at keys.ts:1072 |
| Translate existing keys | `dictionary.ts` (2,260 LOC / 250 KB) | `DICTS`: 28 blocks — `english` (aliases `EN` itself) + 27 partial overrides (schinese:7 … latam:2224) |
| Runtime lookup | `index.ts` (31 LOC) | the only sanctioned import surface: `translate`, `useT`, `removeTerminalFullStop`, types `TParams`/`I18nKey` |
| Find a key's usage | `I18nKey` references | 56 references outside this dir; `useT()` has 40 call sites across panels/components |

## CONVENTIONS
- All 31 importing modules go through `../i18n` (`index.ts`) — never import `keys`/`dictionary` directly.
- Locale identifiers are Steam codes (`schinese`, `tchinese`, `koreana`, `brazilian`, `latam`), never BCP-47; the selectable list lives in `src/data/languages.ts` (`LANGUAGES`, 29 entries) — adding a locale touches both files.
- `DICTS.english` is the `EN` object itself, not a copy — editing `EN` IS editing the English locale.
- Fallback chain: `DICTS[lang][key] ?? EN[key] ?? key` → `{param}` substitution (single-brace, first occurrence per param) → `removeTerminalFullStop` (strips trailing `.。．`, keeps `...`).
- `useT()` binds `useStore()`'s language (`config?.language ?? "english"`); outside React, `translate()` — the `main.tsx` startup-failure path reads `localStorage["cs2bi.language"]` directly.
- Key families: `nav.` `match.` `stats.` `pre.` `settings.` `guide.` `stickers.` `experimental.` `err.` `errcat.` `st.` `cmd.` `upstream.` `startup.`; `errcat.<category>` mirrors backend `AppError.category` one key per Rust error category.
- `keys.ts` (72.5 KB) and `dictionary.ts` (250 KB) are the two largest frontend source files.
- No i18n-specific script or test exists — tsc (via the frontend build gate) is the only automated check on key correctness; nothing asserts locale coverage.

## ANTI-PATTERNS
- NEVER reformat `dictionary.ts` to one key per line — dense multi-declaration lines (up to 322 chars) are intentional; match the neighbouring line density when inserting.
- NEVER inline user-facing strings in components — add a key in `keys.ts`.
- Do not duplicate EN text into every locale or delete a fallback — locales are partial overrides by design; untranslated keys render English.
- Do not translate internal game command values (kept in English on purpose).
- Removing a key from `EN` type-errors every leftover locale entry — clean `dictionary.ts` in the same change.
- `dictionary.ts` (250 KB, dense lines) breaks some grep/GUI tools ("unreadable/large") — search it from the CLI.
