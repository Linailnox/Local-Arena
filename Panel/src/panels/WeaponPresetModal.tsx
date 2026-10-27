import { useEffect, useMemo, useState } from "react";
import Modal from "../components/Modal";
import Toggle from "../components/Toggle";
import WearGauge from "../components/WearGauge";
import { useT } from "../i18n";
import { api, type CosmeticsInventoryState, type CosmeticsTeam, type InventoryItem } from "../lib/api";
import { isLinked, newItem, teamWeapons, withLink, withWeapon } from "../lib/cosmeticsInventory";
import type { WeaponIcon } from "../data/weaponIcons";
import catalogRows from "../data/weaponSkins.json";
import imageRows from "../data/skinImages.json";
import { localizedSkinName } from "../data/skinLocalization";
import { useStore } from "../state/store";
import { useSelectedPickerScroll } from "../lib/useSelectedPickerScroll";
import "./KnifePresetModal.css";
import "./WeaponPresetsPanel.css";

type CatalogSkin = { weapon_defindex: number; paint: number; name: string; min_wear: number; max_wear: number; stattrak: boolean; souvenir: boolean };
type ImageSkin = { weapon_defindex: number; paint: number | string; image: string };
type Skin = CatalogSkin & { image?: string };
const images = new Map((imageRows as ImageSkin[]).map((row) => [`${row.weapon_defindex}:${Number(row.paint)}`, row.image]));

type Props = {
  weapon: WeaponIcon | null;
  team: CosmeticsTeam;
  csgoPath: string | null;
  state: CosmeticsInventoryState | null;
  onSaved: (state: CosmeticsInventoryState) => void;
  onError: (error: unknown) => void;
  onClose: () => void;
};

function preserveDecorations(base: InventoryItem, existing: InventoryItem | undefined): InventoryItem {
  return {
    ...base,
    stickers: existing?.stickers?.map((sticker) => ({ ...sticker })),
    keychains: existing?.keychains?.map((keychain) => ({ ...keychain })),
  };
}

export default function WeaponPresetModal({ weapon, team, csgoPath, state, onSaved, onError, onClose }: Props) {
  const t = useT();
  const { config: appConfig } = useStore();
  const [draft, setDraft] = useState<InventoryItem>(newItem(0));
  const [query, setQuery] = useState("");
  const [saving, setSaving] = useState(false);
  const [linkSides, setLinkSides] = useState(true);
  const skinListRef = useSelectedPickerScroll(!!weapon, `${team}:${draft.paint ?? 0}`);
  const allSkins = useMemo<Skin[]>(() => !weapon ? [] : (catalogRows as CatalogSkin[]).filter((row) => row.weapon_defindex === weapon.id && row.paint > 0).map((row) => ({ ...row, image: images.get(`${row.weapon_defindex}:${row.paint}`) })), [weapon]);
  const visible = useMemo(() => {
    const q = query.trim().toLocaleLowerCase();
    return allSkins.filter((skin) => !q || `${localizedSkinName(appConfig?.language, skin.weapon_defindex, skin.paint, skin.name)} ${skin.paint}`.toLocaleLowerCase().includes(q));
  }, [allSkins, appConfig?.language, query]);
  const selected = allSkins.find((skin) => skin.paint === (draft.paint ?? 0));

  useEffect(() => {
    if (!weapon || !state) return;
    const existing = teamWeapons(state.inventory, team)[String(weapon.id)];
    const first = allSkins[0];
    setDraft(existing ?? newItem(weapon.id, first?.paint ?? 0, first?.min_wear ?? 0.01));
    setLinkSides(isLinked(state.panel, weapon.id));
    setQuery("");
  }, [weapon, team, state, allSkins]);

  const choose = (skin: Skin) => setDraft((value) => ({
    ...value,
    paint: skin.paint,
    wear: Math.min(skin.max_wear, Math.max(skin.min_wear, value.wear ?? 0)),
  }));

  const persist = async (remove = false) => {
    if (!weapon || !csgoPath || !state) return;
    setSaving(true);
    try {
      const key = String(weapon.id);
      const shared = weapon.availability === "shared";
      const otherTeam: CosmeticsTeam = team === "ct" ? "t" : "ct";
      const wasLinked = isLinked(state.panel, weapon.id);
      let inventory = state.inventory;
      let panel = state.panel;

      if (remove) {
        inventory = withWeapon(inventory, team, weapon.id, null);
        if (shared && linkSides) inventory = withWeapon(inventory, otherTeam, weapon.id, null);
        else if (shared && wasLinked && !linkSides) {
          const other = teamWeapons(inventory, otherTeam)[key];
          if (other) inventory = withWeapon(inventory, otherTeam, weapon.id, preserveDecorations(other, other));
        }
      } else {
        const active = { ...draft, def: weapon.id };
        inventory = withWeapon(inventory, team, weapon.id, active);
        if (shared && (linkSides || wasLinked)) {
          const other = teamWeapons(inventory, otherTeam)[key];
          inventory = withWeapon(inventory, otherTeam, weapon.id, preserveDecorations(active, other));
        }
      }
      if (shared) panel = withLink(panel, weapon.id, linkSides);

      const next = await api.saveCosmeticsInventory(csgoPath, state.steam_id, inventory, panel);
      onSaved(next);
      onClose();
    } catch (error) { onError(error); } finally { setSaving(false); }
  };

  const existing = weapon && state ? teamWeapons(state.inventory, team)[String(weapon.id)] : undefined;
  const stattrakOn = draft.stattrak !== undefined;
  return <Modal open={!!weapon} title={weapon ? `${weapon.name} · ${t("weapons.title")}` : t("weapons.title")} onClose={onClose} width={880} scrimClassName="picker-modal" footer={<div className="kp__footer-actions"><button className="wp-modal__remove" disabled={saving || !existing} onClick={() => void persist(true)}>{t("weapons.remove")}</button><button className="kp__save" disabled={saving || !selected} onClick={() => void persist()}>{saving ? t("weapons.saving") : t("weapons.apply")}</button></div>}>
    <div className="kp kp--split">
      <div className="kp__side">
        <div className="kp__preview"><img src={selected?.image || weapon?.url} alt="" /><div><i className={`wp-modal__team wp-modal__team--${team}`}>{team.toUpperCase()}</i><strong>{selected ? localizedSkinName(appConfig?.language, selected.weapon_defindex, selected.paint, selected.name) : t("weapons.noSkins")}</strong><span>{t("cosmetics.paintKit")} {draft.paint || "-"}</span></div></div>
        <div className="kp__columns"><label className="kp__field"><span>{t("live.wear")}</span><input type="number" min={selected?.min_wear ?? 0} max={selected?.max_wear ?? 1} step="0.000001" value={draft.wear ?? 0} onChange={(event) => setDraft((value) => ({ ...value, wear: Math.min(selected?.max_wear ?? 1, Math.max(selected?.min_wear ?? 0, Number(event.target.value))) }))} /></label><label className="kp__field"><span>{t("live.seed")}</span><input type="number" min="0" max="1000" step="1" value={draft.seed ?? 0} placeholder={t("live.seedPlaceholder")} onChange={(event) => setDraft((value) => ({ ...value, seed: Math.min(1000, Math.max(0, Number(event.target.value))) }))} /></label></div>
        <WearGauge min={selected?.min_wear ?? 0} max={selected?.max_wear ?? 1} value={draft.wear ?? 0} />
        <label className="kp__field"><span>{t("live.nameTag")}</span><input maxLength={20} value={draft.nametag ?? ""} placeholder={t("cosmetics.namePlaceholder")} onChange={(event) => setDraft((value) => ({ ...value, nametag: event.target.value }))} /></label>
        <div className="kp__toggle-row"><span>{t("live.stattrak")}</span><Toggle checked={stattrakOn} disabled={!selected?.stattrak} onChange={(enabled) => setDraft((value) => ({ ...value, stattrak: enabled ? value.stattrak ?? 0 : undefined }))} /></div>
        {stattrakOn && <label className="kp__field"><span>{t("live.count")}</span><input type="number" min="0" step="1" value={draft.stattrak ?? 0} onChange={(event) => setDraft((value) => ({ ...value, stattrak: Math.max(0, Number(event.target.value)) }))} /></label>}
        {weapon?.availability === "shared" && <div className="kp__toggle-row"><span>{t("weapons.sameSkinBoth")}</span><Toggle checked={linkSides} onChange={setLinkSides} /></div>}
      </div>
      <div className="kp__main">
        <label className="kp__field"><span>{t("weapons.paint")}</span><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("weapons.search")} /></label>
        {visible.length ? <div className="kp__skins" ref={skinListRef}>{visible.map((skin) => (
          <button key={skin.paint} className={skin.paint === draft.paint ? "is-selected" : ""} onClick={() => choose(skin)}>
            {skin.stattrak && (
              <span className="kp__badges" aria-hidden="true">
                <i className="is-st">ST</i>
              </span>
            )}
            <img src={skin.image || weapon?.url} alt="" loading="lazy" />
            <span>{localizedSkinName(appConfig?.language, skin.weapon_defindex, skin.paint, skin.name)} [{skin.paint}]</span>
          </button>
        ))}</div> : <div className="wp-modal__empty">{t("weapons.noSkins")}</div>}
      </div>
    </div>
  </Modal>;
}
