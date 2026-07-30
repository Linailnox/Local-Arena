import type { AppConfig, CharmPreset, CosmeticsTeam, KnifeCustomizerConfig, KnifePreset, StickerPreset } from "./api";

export const STICKER_SLOT_COUNT = 5;
export const STICKER_RELEASE_ENABLED = true;

const STICKER_OVERFLOW_OFFSETS = [
  { offset_x: 0.45, offset_y: 0.45 },
  { offset_x: -0.45, offset_y: 0.45 },
  { offset_x: 0.45, offset_y: -0.45 },
  { offset_x: -0.45, offset_y: -0.45 },
] as const;

export function stickerFeatureEnabled(config: AppConfig | null | undefined): boolean {
  return STICKER_RELEASE_ENABLED
    && !!config?.experimental_features_enabled
    && !!config?.experimental_stickers_enabled;
}

export function clampStickerValue(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Number.isFinite(value) ? value : min));
}

export function normalizeStickerSlots(stickers: StickerPreset[]): StickerPreset[] {
  return [...stickers].sort((left, right) => left.slot - right.slot);
}

export function withPreservedGunPresetDecorations(
  base: KnifePreset,
  existing: KnifePreset | undefined,
): KnifePreset {
  return {
    ...base,
    stickers: (existing?.stickers ?? []).map((sticker) => ({ ...sticker })),
    charm: existing?.charm ? { ...existing.charm } : null,
  };
}

export function availableStickerSlotCount(schemaCount: number): number {
  if (!Number.isFinite(schemaCount) || Math.trunc(schemaCount) <= 0) return 0;
  return STICKER_SLOT_COUNT;
}

export function defaultStickerPlacement(
  slot: number,
  schemaCount: number,
): Pick<StickerPreset, "schema" | "offset_x" | "offset_y" | "custom_position"> | null {
  const normalizedSlot = Math.trunc(slot);
  const normalizedSchemaCount = Math.trunc(schemaCount);
  if (!Number.isFinite(slot) || !Number.isFinite(schemaCount)
    || normalizedSlot < 0 || normalizedSlot >= STICKER_SLOT_COUNT || normalizedSchemaCount <= 0)
    return null;

  if (normalizedSlot < normalizedSchemaCount) {
    return { schema: normalizedSlot, offset_x: 0, offset_y: 0, custom_position: false };
  }

  const offset = STICKER_OVERFLOW_OFFSETS[
    Math.min(STICKER_OVERFLOW_OFFSETS.length - 1, normalizedSlot - normalizedSchemaCount)
  ];
  return {
    schema: normalizedSchemaCount - 1,
    offset_x: offset.offset_x,
    offset_y: offset.offset_y,
    custom_position: true,
  };
}

export function replaceSticker(stickers: StickerPreset[], sticker: StickerPreset): StickerPreset[] {
  return normalizeStickerSlots([...stickers.filter((entry) => entry.slot !== sticker.slot), sticker]);
}

export function removeSticker(stickers: StickerPreset[], slot: number): StickerPreset[] {
  return normalizeStickerSlots(stickers.filter((entry) => entry.slot !== slot));
}

export function swapStickerSlots(stickers: StickerPreset[], slot: number, target: number): StickerPreset[] {
  if (slot < 0 || slot >= STICKER_SLOT_COUNT || target < 0 || target >= STICKER_SLOT_COUNT)
    return normalizeStickerSlots(stickers);
  return normalizeStickerSlots(stickers.map((entry) => entry.slot === slot
    ? { ...entry, slot: target }
    : entry.slot === target ? { ...entry, slot } : entry));
}

export function filterStickerCatalog<T extends { id: number }>(
  entries: T[],
  query: string,
  displayName: (entry: T) => string,
): T[] {
  const value = query.trim().toLocaleLowerCase();
  if (!value) return entries;
  return entries.filter((entry) => `${entry.id} ${displayName(entry)}`.toLocaleLowerCase().includes(value));
}

export function paginateStickerCatalog<T>(entries: T[], requestedPage: number, pageSize: number) {
  const pageCount = Math.max(1, Math.ceil(entries.length / pageSize));
  const page = Math.min(pageCount - 1, Math.max(0, requestedPage));
  return { page, pageCount, entries: entries.slice(page * pageSize, (page + 1) * pageSize) };
}

export function updateGunPresetStickers(
  config: KnifeCustomizerConfig,
  team: CosmeticsTeam,
  weaponId: number,
  preset: KnifePreset,
  stickers: StickerPreset[],
): KnifeCustomizerConfig {
  return updateGunPresetDecorations(config, team, weaponId, preset, {
    stickers: normalizeStickerSlots(stickers),
  });
}

export function updateGunPresetCharm(
  config: KnifeCustomizerConfig,
  team: CosmeticsTeam,
  weaponId: number,
  preset: KnifePreset,
  charm: CharmPreset | null,
): KnifeCustomizerConfig {
  return updateGunPresetDecorations(config, team, weaponId, preset, { charm });
}

function updateGunPresetDecorations(
  config: KnifeCustomizerConfig,
  team: CosmeticsTeam,
  weaponId: number,
  preset: KnifePreset,
  patch: Pick<KnifePreset, "stickers"> | Pick<KnifePreset, "charm">,
): KnifeCustomizerConfig {
  const key = String(weaponId);
  const nextPreset: KnifePreset = { ...preset, ...patch };
  return {
    ...config,
    loadouts: {
      ...config.loadouts,
      [team]: {
        ...config.loadouts[team],
        gun_presets: { ...config.loadouts[team].gun_presets, [key]: nextPreset },
      },
    },
  };
}
