import type {
  CosmeticsInventoryState,
  CosmeticsTeam,
  InventoryItem,
  PanelCosmeticsSettings,
  PlayerInventory,
} from "./api";

export const TEAM_KEY: Record<CosmeticsTeam, string> = { ct: "3", t: "2" };

export function otherTeam(team: CosmeticsTeam): CosmeticsTeam {
  return team === "ct" ? "t" : "ct";
}

export function weaponsKey(team: CosmeticsTeam): "ctWeapons" | "tWeapons" {
  return team === "ct" ? "ctWeapons" : "tWeapons";
}

export function teamWeapons(inventory: PlayerInventory, team: CosmeticsTeam): Record<string, InventoryItem> {
  return inventory[weaponsKey(team)] ?? {};
}

export function withTeamWeapons(
  inventory: PlayerInventory,
  team: CosmeticsTeam,
  weapons: Record<string, InventoryItem>,
): PlayerInventory {
  const next: PlayerInventory = { ...inventory };
  const key = weaponsKey(team);
  if (Object.keys(weapons).length === 0) delete next[key];
  else next[key] = weapons;
  return next;
}

export function withWeapon(
  inventory: PlayerInventory,
  team: CosmeticsTeam,
  weaponId: number,
  item: InventoryItem | null,
): PlayerInventory {
  const weapons = { ...teamWeapons(inventory, team) };
  const key = String(weaponId);
  if (item) weapons[key] = item;
  else delete weapons[key];
  return withTeamWeapons(inventory, team, weapons);
}

export function knife(inventory: PlayerInventory, team: CosmeticsTeam): InventoryItem | undefined {
  return inventory.knives?.[TEAM_KEY[team]];
}

export function withKnife(
  inventory: PlayerInventory,
  team: CosmeticsTeam,
  item: InventoryItem | null,
): PlayerInventory {
  const knives = { ...(inventory.knives ?? {}) };
  const key = TEAM_KEY[team];
  if (item) knives[key] = item;
  else delete knives[key];
  const next: PlayerInventory = { ...inventory };
  if (Object.keys(knives).length === 0) delete next.knives;
  else next.knives = knives;
  return next;
}

export function glove(inventory: PlayerInventory, team: CosmeticsTeam): InventoryItem | undefined {
  return inventory.gloves?.[TEAM_KEY[team]];
}

export function withGlove(
  inventory: PlayerInventory,
  team: CosmeticsTeam,
  item: InventoryItem | null,
): PlayerInventory {
  const gloves = { ...(inventory.gloves ?? {}) };
  const key = TEAM_KEY[team];
  if (item) gloves[key] = item;
  else delete gloves[key];
  const next: PlayerInventory = { ...inventory };
  if (Object.keys(gloves).length === 0) delete next.gloves;
  else next.gloves = gloves;
  return next;
}

export function musicKitId(inventory: PlayerInventory): number {
  return inventory.musicKit?.musicId ?? 0;
}

export function withMusicKit(inventory: PlayerInventory, musicId: number): PlayerInventory {
  const next: PlayerInventory = { ...inventory };
  if (musicId > 0) next.musicKit = { ...(next.musicKit ?? {}), musicId };
  else delete next.musicKit;
  return next;
}

export function defaultKnifeId(panel: PanelCosmeticsSettings, team: CosmeticsTeam): number {
  return panel.default_knife?.[TEAM_KEY[team]] ?? 0;
}

export function withDefaultKnife(
  panel: PanelCosmeticsSettings,
  team: CosmeticsTeam,
  defindex: number,
): PanelCosmeticsSettings {
  const defaultKnife = { ...(panel.default_knife ?? {}) };
  const key = TEAM_KEY[team];
  if (defindex > 0) defaultKnife[key] = defindex;
  else delete defaultKnife[key];
  return { ...panel, default_knife: defaultKnife };
}

export function isLinked(panel: PanelCosmeticsSettings, weaponId: number): boolean {
  return panel.shared_weapon_links?.[String(weaponId)] ?? true;
}

export function withLink(
  panel: PanelCosmeticsSettings,
  weaponId: number,
  linked: boolean,
): PanelCosmeticsSettings {
  return {
    ...panel,
    shared_weapon_links: { ...(panel.shared_weapon_links ?? {}), [String(weaponId)]: linked },
  };
}

export function newItem(def: number, paint = 0, wear = 0.01): InventoryItem {
  return { def, paint, seed: 0, wear };
}

export function itemPaint(item: InventoryItem | undefined): number {
  return item?.paint ?? 0;
}

export function hasItem(item: InventoryItem | undefined): boolean {
  return !!item && itemPaint(item) > 0;
}

export function emptyState(): CosmeticsInventoryState {
  return {
    plugin_present: false,
    config_present: false,
    cs2_running: false,
    steam_id: "",
    inventory: {},
    panel: {
      enabled: false,
      apply_to_human_players: true,
      apply_on_pickup: true,
      stickers_enabled: false,
      charms_enabled: false,
      agents_enabled: false,
      shared_weapon_links: {},
      default_knife: {},
    },
  };
}

export function countWeapons(inventory: PlayerInventory, team: CosmeticsTeam): number {
  return Object.values(teamWeapons(inventory, team)).filter((item) => itemPaint(item) > 0).length;
}

export function configuredWeaponIds(inventory: PlayerInventory, team: CosmeticsTeam): Set<number> {
  return new Set(
    Object.entries(teamWeapons(inventory, team))
      .filter(([, item]) => (item.paint ?? 0) > 0)
      .map(([key]) => Number(key)),
  );
}
