import type { AppConfig, CosmeticsTeam, InventoryItem, InventoryKeychain, InventorySticker, PlayerInventory } from "./api";

function teamWeaponsKey(team: CosmeticsTeam): "ctWeapons" | "tWeapons" {
  return team === "ct" ? "ctWeapons" : "tWeapons";
}

export const STICKER_SLOT_COUNT = 5;
export const STICKER_RELEASE_ENABLED = true;

const STICKER_OVERFLOW_OFFSETS = [
  { x: 0.45, y: 0.45 },
  { x: -0.45, y: 0.45 },
  { x: 0.45, y: -0.45 },
  { x: -0.45, y: -0.45 },
] as const;

export function stickerFeatureEnabled(config: AppConfig | null | undefined): boolean {
  return STICKER_RELEASE_ENABLED
    && !!config?.experimental_features_enabled
    && !!config?.experimental_stickers_enabled;
}

export function clampStickerValue(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Number.isFinite(value) ? value : min));
}

export function normalizeStickerSlots(stickers: InventorySticker[]): InventorySticker[] {
  return [...stickers].sort((left, right) => left.slot - right.slot);
}

export function availableStickerSlotCount(schemaCount: number): number {
  if (!Number.isFinite(schemaCount) || Math.trunc(schemaCount) <= 0) return 0;
  return STICKER_SLOT_COUNT;
}

export function defaultStickerPlacement(
  slot: number,
  schemaCount: number,
): Pick<InventorySticker, "schema" | "x" | "y"> | null {
  const normalizedSlot = Math.trunc(slot);
  const normalizedSchemaCount = Math.trunc(schemaCount);
  if (!Number.isFinite(slot) || !Number.isFinite(schemaCount)
    || normalizedSlot < 0 || normalizedSlot >= STICKER_SLOT_COUNT || normalizedSchemaCount <= 0)
    return null;

  if (normalizedSlot < normalizedSchemaCount) {
    return { schema: normalizedSlot, x: 0, y: 0 };
  }

  const offset = STICKER_OVERFLOW_OFFSETS[
    Math.min(STICKER_OVERFLOW_OFFSETS.length - 1, normalizedSlot - normalizedSchemaCount)
  ];
  return { schema: normalizedSchemaCount - 1, x: offset.x, y: offset.y };
}

export function replaceSticker(stickers: InventorySticker[], sticker: InventorySticker): InventorySticker[] {
  return normalizeStickerSlots([...stickers.filter((entry) => entry.slot !== sticker.slot), sticker]);
}

export function removeSticker(stickers: InventorySticker[], slot: number): InventorySticker[] {
  return normalizeStickerSlots(stickers.filter((entry) => entry.slot !== slot));
}

export function swapStickerSlots(stickers: InventorySticker[], slot: number, target: number): InventorySticker[] {
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
  inventory: PlayerInventory,
  team: CosmeticsTeam,
  weaponId: number,
  item: InventoryItem,
  stickers: InventorySticker[],
): PlayerInventory {
  const next: InventoryItem = { ...item };
  if (stickers.length === 0) delete next.stickers;
  else next.stickers = normalizeStickerSlots(stickers);
  return withWeapon(inventory, team, weaponId, next);
}

export function updateGunPresetCharm(
  inventory: PlayerInventory,
  team: CosmeticsTeam,
  weaponId: number,
  item: InventoryItem,
  keychain: InventoryKeychain | null,
): PlayerInventory {
  const next: InventoryItem = { ...item };
  if (keychain) next.keychains = [keychain];
  else delete next.keychains;
  return withWeapon(inventory, team, weaponId, next);
}

function withWeapon(
  inventory: PlayerInventory,
  team: CosmeticsTeam,
  weaponId: number,
  item: InventoryItem,
): PlayerInventory {
  const key = teamWeaponsKey(team);
  const weapons = { ...(inventory[key] ?? {}), [String(weaponId)]: item };
  return { ...inventory, [key]: weapons };
}
