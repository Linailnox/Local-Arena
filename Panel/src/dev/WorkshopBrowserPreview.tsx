import type { AppConfig, CosmeticsInventoryState, InventoryItem, PlayerInventory } from "../lib/api";
import StickersPanel from "../panels/StickersPanel";
import { AppStatePreviewProvider } from "../state/store";
import { WEAPON_ICONS } from "../data/weaponIcons";
import skinImages from "../data/skinImages.json";
import placementRows from "../data/cosmeticPlacements.json";
import charmRows from "../data/charmCatalog.json";
import { STICKERS } from "../data/stickers";
import { defaultStickerPlacement } from "../lib/stickerEditor";
import { TEAM_KEY } from "../lib/cosmeticsInventory";
import "./WorkshopBrowserPreview.css";

type SkinImage = { weapon_defindex: number; paint: number | string; legacy_model: boolean };
type WeaponPlacement = {
  stickerSchemaCount: number;
  stickerPositions: { schema: number }[];
  charmPositions: { placementId: number; x: number; y: number }[];
};

const placements = placementRows as Record<string, WeaponPlacement>;
const skins = skinImages as SkinImage[];
const previewCharms = charmRows as { id: number }[];

function previewItem(weaponId: number, team: "ct" | "t"): InventoryItem {
  const paintedSkins = skins.filter((entry) => entry.weapon_defindex === weaponId && Number(entry.paint) > 0);
  const paint = Number(paintedSkins.find((entry) => entry.legacy_model)?.paint ?? paintedSkins[0]?.paint ?? 0);
  const capability = placements[String(weaponId)];
  const stickerCatalog = STICKERS.filter((entry) => entry.id > 0);
  const stickerStart = (weaponId * 5 + (team === "ct" ? 0 : 5)) % stickerCatalog.length;
  const charm = previewCharms[(weaponId + (team === "ct" ? 0 : 1)) % previewCharms.length];
  const charmPosition = capability?.charmPositions[0];
  const stickers = capability ? Array.from({ length: 5 }, (_, slot) => {
    const placement = defaultStickerPlacement(slot, capability.stickerSchemaCount);
    const sticker = stickerCatalog[(stickerStart + slot) % stickerCatalog.length];
    return placement ? {
      slot,
      def: sticker.id,
      schema: placement.schema,
      x: placement.x,
      y: placement.y,
      wear: 0,
      rotation: 0,
    } : null;
  }).filter((entry): entry is NonNullable<typeof entry> => !!entry) : [];

  return {
    def: weaponId,
    paint,
    seed: 0,
    wear: 0.08,
    stickers,
    keychains: charm && charmPosition
      ? [{ def: charm.id, slot: 0, seed: 0, x: charmPosition.x, y: charmPosition.y, z: 0 }]
      : undefined,
  };
}

function previewInventory(team: "ct" | "t"): PlayerInventory {
  const weapons = Object.fromEntries(WEAPON_ICONS
    .filter((weapon) => (weapon.availability === team || weapon.availability === "shared") && placements[String(weapon.id)])
    .map((weapon) => [String(weapon.id), previewItem(weapon.id, team)]));
  return team === "ct" ? { ctWeapons: weapons } : { tWeapons: weapons };
}

const previewState: CosmeticsInventoryState = {
  plugin_present: true,
  config_present: true,
  cs2_running: false,
  steam_id: "76561199145436841",
  inventory: {
    ...previewInventory("ct"),
    ...previewInventory("t"),
    knives: { [TEAM_KEY.ct]: { def: 515, paint: 12, seed: 0, wear: 0.01 } },
    gloves: { [TEAM_KEY.ct]: { def: 5030, paint: 10048, seed: 0, wear: 0.01 } },
    musicKit: { musicId: 1 },
  },
  panel: {
    enabled: true,
    apply_to_human_players: true,
    apply_on_pickup: true,
    stickers_enabled: true,
    charms_enabled: true,
    agents_enabled: false,
    shared_weapon_links: {},
    default_knife: { [TEAM_KEY.ct]: 515 },
  },
};

const previewAppConfig = {
  language: "schinese",
  experimental_features_enabled: true,
  experimental_stickers_enabled: true,
} as AppConfig;

export default function WorkshopBrowserPreview() {
  return <AppStatePreviewProvider value={{
    config: previewAppConfig,
    csgoPath: "browser-preview",
    process: { running: false, pid: null, executable: null, path_accessible: true, matches_selected: true },
    reportError: (error) => console.error("[workshop-preview]", error),
  }}>
    <main className="workshop-browser-preview">
      <StickersPanel browserPreviewState={previewState} />
    </main>
  </AppStatePreviewProvider>;
}
