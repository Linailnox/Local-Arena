import { useEffect, useMemo, useState } from "react";
import Modal from "../components/Modal";
import Toggle from "../components/Toggle";
import WearGauge from "../components/WearGauge";
import { api, type CosmeticsInventoryState, type InventoryItem } from "../lib/api";
import { defaultKnifeId, knife, newItem, withDefaultKnife, withKnife } from "../lib/cosmeticsInventory";
import { KNIFE_ICONS, type KnifeIcon } from "../data/knifeIcons";
import imageRows from "../data/skinImages.json";
import catalogRows from "../data/weaponSkins.json";
import { finishName, itemName, localizedSkinName } from "../data/skinLocalization";
import { useT, type I18nKey } from "../i18n";
import { useStore } from "../state/store";
import CosmeticsTeamSwitch, { useCosmeticsTeam } from "../components/CosmeticsTeamSwitch";
import { useSelectedPickerScroll } from "../lib/useSelectedPickerScroll";
import "./KnifePresetModal.css";

type SkinImage = { weapon_defindex: number; paint: number; image: string };
type CatalogSkin = { weapon_defindex: number; paint: number; name: string; min_wear: number; max_wear: number; stattrak: boolean };

const PHASE_NAMES: Record<number, I18nKey | string> = {
  415: "phase.ruby", 416: "phase.sapphire", 417: "phase.blackPearl",
  418: "P1", 419: "P2", 420: "P3", 421: "P4", 568: "phase.emerald",
  569: "P1", 570: "P2", 571: "P3", 572: "P4", 617: "phase.blackPearl",
  618: "P2", 619: "phase.sapphire", 852: "P1", 853: "P2", 854: "P3",
  855: "P4", 856: "phase.ruby", 857: "phase.sapphire", 858: "phase.blackPearl",
};

type Props = {
  open: boolean;
  csgoPath: string | null;
  state: CosmeticsInventoryState | null;
  onSaved: (state: CosmeticsInventoryState) => void;
  onError: (error: unknown) => void;
  onClose: () => void;
};

export default function KnifePresetModal({ open, csgoPath, state, onSaved, onError, onClose }: Props) {
  const t = useT();
  const { config: appConfig } = useStore();
  const language = appConfig?.language;
  const [team, setTeam] = useCosmeticsTeam();
  const [knifeId, setKnifeId] = useState<number>(KNIFE_ICONS[0]?.id ?? 515);
  const [draft, setDraft] = useState<InventoryItem>(newItem(515));
  const [query, setQuery] = useState("");
  const [saving, setSaving] = useState(false);
  const [useAsDefault, setUseAsDefault] = useState(false);
  const skinListRef = useSelectedPickerScroll(open, `${team}:${draft.paint ?? 0}`);

  useEffect(() => {
    if (!open || !state) return;
    const saved = knife(state.inventory, team);
    const fallback = defaultKnifeId(state.panel, team) || KNIFE_ICONS[0]?.id || 515;
    const id = saved?.def ?? fallback;
    setKnifeId(id);
    setDraft(saved ?? newItem(id, 0, 0.01));
    setUseAsDefault(defaultKnifeId(state.panel, team) === id);
    setQuery("");
  }, [open, team, state]);

  const allSkins = useMemo(() => !knifeId ? [] : (imageRows as SkinImage[])
    .filter((row) => row.weapon_defindex === knifeId && row.paint > 0), [knifeId]);
  const catalog = useMemo(() => new Map((catalogRows as CatalogSkin[])
    .map((row) => [`${row.weapon_defindex}:${row.paint}`, row])), []);
  const label = (row: SkinImage) => {
    const full = localizedSkinName(language, row.weapon_defindex, row.paint);
    const phase = PHASE_NAMES[row.paint];
    const phaseLabel = typeof phase === "string" && phase.startsWith("phase.")
      ? t(phase as I18nKey)
      : phase;
    return `${finishName(full)}${phaseLabel ? ` · ${phaseLabel}` : ""} [${row.paint}]`;
  };
  const skins = useMemo(() => {
    const q = query.trim().toLocaleLowerCase();
    return allSkins.filter((row) => !q || label(row).toLocaleLowerCase().includes(q));
  // Labels follow the selected Panel language.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [allSkins, language, query]);
  const selectedSkin = allSkins.find((row) => row.paint === draft.paint);
  const selectedCatalog = catalog.get(`${knifeId}:${draft.paint ?? 0}`);

  const chooseKnife = (entry: KnifeIcon) => {
    setKnifeId(entry.id);
    setDraft((value) => ({ ...value, def: entry.id, paint: 0 }));
  };

  const choose = (skin: SkinImage) => {
    const details = catalog.get(`${skin.weapon_defindex}:${skin.paint}`);
    setDraft((value) => ({
      ...value,
      paint: skin.paint,
      wear: Math.min(details?.max_wear ?? 1, Math.max(details?.min_wear ?? 0, value.wear ?? 0)),
    }));
  };

  const save = async () => {
    if (!csgoPath || !state) return;
    setSaving(true);
    try {
      const active: InventoryItem = { ...draft, def: knifeId };
      let inventory = withKnife(state.inventory, team, active);
      let panel = state.panel;
      if (useAsDefault) panel = withDefaultKnife(panel, team, knifeId);
      else if (defaultKnifeId(panel, team) === knifeId) panel = withDefaultKnife(panel, team, 0);
      const next = await api.saveCosmeticsInventory(csgoPath, state.steam_id, inventory, panel);
      onSaved(next);
      onClose();
    } catch (error) {
      onError(error);
    } finally {
      setSaving(false);
    }
  };

  const selectedKnife = KNIFE_ICONS.find((entry) => entry.id === knifeId);
  const knifeName = itemName(localizedSkinName(language, knifeId, 0, `Knife ${knifeId}`));

  return <Modal open={open} title={`${selectedKnife ? knifeName : t("cosmetics.knifeTitle")} · ${t("cosmetics.knifeTitle")}`} onClose={onClose} width={880} scrimClassName="picker-modal" footer={
    <div className="kp__footer-actions"><button className="kp__save" disabled={saving || (draft.paint ?? 0) <= 0} onClick={() => void save()}>
      {saving ? t("cosmetics.saving") : t("cosmetics.save")}
    </button></div>
  }>
    <div className="kp kp--split">
      <div className="kp__side">
        <div className="kp__preview">
          <span className="kp__team-fab"><CosmeticsTeamSwitch value={team} onChange={setTeam} ariaLabel={t("cosmetics.teamLoadout")} compact /></span>
          <img src={selectedSkin?.image || selectedKnife?.url} alt="" />
          <div><strong>{selectedSkin ? label(selectedSkin) : t("cosmetics.chooseSkin")}</strong><span>{t("cosmetics.paintKit")} {draft.paint || "-"}</span></div>
        </div>
        <div className="kp__knife-row">{KNIFE_ICONS.map((entry) => (
          <button
            key={entry.id}
            className={`kp__knife-pick ${entry.id === knifeId ? "is-selected" : ""}`}
            onClick={() => chooseKnife(entry)}
            title={itemName(localizedSkinName(language, entry.id, 0, `Knife ${entry.id}`))}
          >
            <img src={entry.url} alt="" loading="lazy" />
          </button>
        ))}</div>
        <div className="kp__columns"><label className="kp__field"><span>{t("live.wear")}</span><input type="number" min={selectedCatalog?.min_wear ?? 0} max={selectedCatalog?.max_wear ?? 1} step="0.000001" value={draft.wear ?? 0} onChange={(event) => setDraft((value) => ({ ...value, wear: Math.min(selectedCatalog?.max_wear ?? 1, Math.max(selectedCatalog?.min_wear ?? 0, Number(event.target.value))) }))} /></label><label className="kp__field"><span>{t("live.seed")}</span><input type="number" min="0" max="1000" step="1" value={draft.seed ?? 0} placeholder={t("live.seedPlaceholder")} onChange={(event) => setDraft((value) => ({ ...value, seed: Math.min(1000, Math.max(0, Number(event.target.value))) }))} /></label></div>
        <WearGauge min={selectedCatalog?.min_wear ?? 0} max={selectedCatalog?.max_wear ?? 1} value={draft.wear ?? 0} />
        <label className="kp__field"><span>{t("live.nameTag")}</span><input maxLength={20} value={draft.nametag ?? ""} onChange={(event) => setDraft((value) => ({ ...value, nametag: event.target.value }))} placeholder={t("cosmetics.namePlaceholder")} /></label>
        <div className="kp__toggle-row"><span>{team === "ct" ? t("cosmetics.defaultKnifeCt") : t("cosmetics.defaultKnifeT")}</span><Toggle checked={useAsDefault} onChange={setUseAsDefault} /></div>
        <div className="kp__toggle-row"><span>{t("live.stattrak")}</span><Toggle checked={draft.stattrak !== undefined} disabled={!selectedCatalog?.stattrak} onChange={(enabled) => setDraft((value) => ({ ...value, stattrak: enabled ? value.stattrak ?? 0 : undefined }))} /></div>
        {draft.stattrak !== undefined && <label className="kp__field"><span>{t("cosmetics.initialCount")}</span><input type="number" min="0" step="1" value={draft.stattrak} onChange={(event) => setDraft((value) => ({ ...value, stattrak: Math.max(0, Number(event.target.value)) }))} /></label>}
      </div>
      <div className="kp__main">
        <label className="kp__field"><span>{t("weapons.paint")}</span><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("cosmetics.searchSkin")} /></label>
        {skins.length ? <div className="kp__skins" ref={skinListRef}>{skins.map((skin) => {
          const details = catalog.get(`${skin.weapon_defindex}:${skin.paint}`);
          return (
            <button key={skin.paint} className={skin.paint === draft.paint ? "is-selected" : ""} onClick={() => choose(skin)}>
              {details?.stattrak && <span className="kp__badges" aria-hidden="true"><i className="is-st">ST</i></span>}
              <img src={skin.image} alt="" loading="lazy" />
              <span>{label(skin)}</span>
            </button>
          );
        })}</div> : <div className="wp-modal__empty">{t("weapons.noSkins")}</div>}
      </div>
    </div>
  </Modal>;
}
