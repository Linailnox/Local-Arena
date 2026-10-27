import { useEffect, useMemo, useState } from "react";
import Modal from "../components/Modal";
import Toggle from "../components/Toggle";
import WearGauge from "../components/WearGauge";
import GLOVE_SKINS, { gloveModelName, type GloveSkin } from "../data/gloveSkins";
import { finishName, localizedSkinName } from "../data/skinLocalization";
import { api, type CosmeticsInventoryState, type InventoryItem } from "../lib/api";
import { glove, newItem, withGlove } from "../lib/cosmeticsInventory";
import { useT } from "../i18n";
import { useStore } from "../state/store";
import CosmeticsTeamSwitch, { useCosmeticsTeam } from "../components/CosmeticsTeamSwitch";
import { useSelectedPickerScroll } from "../lib/useSelectedPickerScroll";
import "./KnifePresetModal.css";

const DEFAULT_GLOVE: InventoryItem = newItem(5030, 10048, 0.01);
const rows = GLOVE_SKINS as GloveSkin[];

type Props = {
  open: boolean;
  csgoPath: string | null;
  state: CosmeticsInventoryState | null;
  onSaved: (state: CosmeticsInventoryState) => void;
  onError: (error: unknown) => void;
  onClose: () => void;
};

export default function GlovePresetModal({ open, csgoPath, state, onSaved, onError, onClose }: Props) {
  const t = useT();
  const { config: appConfig } = useStore();
  const language = appConfig?.language;
  const [draft, setDraft] = useState<InventoryItem>(DEFAULT_GLOVE);
  const [enabled, setEnabled] = useState(false);
  const [query, setQuery] = useState("");
  const [saving, setSaving] = useState(false);
  const [team, setTeam] = useCosmeticsTeam();
  const skinListRef = useSelectedPickerScroll(open, `${team}:${draft.def}:${draft.paint ?? 0}`);
  const modelName = (defindex: number) => gloveModelName(language, defindex);
  const skinName = (skin: GloveSkin) => finishName(localizedSkinName(language, skin.defindex, skin.paint, `${modelName(skin.defindex)} | ${skin.name}`));

  useEffect(() => {
    if (!open || !state) return;
    const saved = glove(state.inventory, team);
    setDraft(saved ?? DEFAULT_GLOVE);
    setEnabled(!!saved);
    setQuery("");
  }, [open, team, state]);

  const visible = useMemo(() => {
    const q = query.trim().toLocaleLowerCase();
    return rows.filter((row) => !q || `${modelName(row.defindex)} ${skinName(row)} ${row.paint}`.toLocaleLowerCase().includes(q));
  // Labels follow the selected Panel language.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language, query]);
  const selected = rows.find((row) => row.defindex === draft.def && row.paint === (draft.paint ?? 0));

  const save = async () => {
    if (!csgoPath || !state) return;
    setSaving(true);
    try {
      const inventory = withGlove(state.inventory, team, enabled ? { ...draft, def: draft.def } : null);
      const next = await api.saveCosmeticsInventory(csgoPath, state.steam_id, inventory, state.panel);
      onSaved(next);
      onClose();
    } catch (error) {
      onError(error);
    } finally {
      setSaving(false);
    }
  };

  return <Modal open={open} title={t("cosmetics.gloveTitle")} onClose={onClose} width={880} scrimClassName="picker-modal" footer={<div className="kp__footer-actions"><button className="kp__save" disabled={saving || (enabled && (draft.paint ?? 0) <= 0)} onClick={() => void save()}>{saving ? t("cosmetics.saving") : t("cosmetics.save")}</button></div>}>
    <div className="kp kp--split">
      <div className="kp__side">
        <div className="kp__preview">
          <span className="kp__team-fab"><CosmeticsTeamSwitch value={team} onChange={setTeam} ariaLabel={t("cosmetics.teamLoadout")} compact /></span>
          <img src={selected?.image} alt="" />
          <div><strong>{selected ? `${modelName(selected.defindex)} · ${skinName(selected)}` : t("cosmetics.chooseGlove")}</strong><span>{t("cosmetics.paintKit")} {draft.paint || "-"}</span></div>
        </div>
        <div className="kp__toggle-row"><span>{team === "ct" ? t("cosmetics.enableGloveCt") : t("cosmetics.enableGloveT")}</span><Toggle checked={enabled} onChange={setEnabled} /></div>
        <div className="kp__columns"><label className="kp__field"><span>{t("live.wear")}</span><input type="number" min={selected?.minWear ?? 0} max={selected?.maxWear ?? 1} step="0.000001" value={draft.wear ?? 0} onChange={(event) => setDraft((value) => ({ ...value, wear: Math.min(selected?.maxWear ?? 1, Math.max(selected?.minWear ?? 0, Number(event.target.value))) }))} /></label><label className="kp__field"><span>{t("live.seed")}</span><input type="number" min="0" max="1000" step="1" value={draft.seed ?? 0} placeholder={t("live.seedPlaceholder")} onChange={(event) => setDraft((value) => ({ ...value, seed: Math.min(1000, Math.max(0, Number(event.target.value))) }))} /></label></div>
        <WearGauge min={selected?.minWear ?? 0} max={selected?.maxWear ?? 1} value={draft.wear ?? 0} />
      </div>
      <div className="kp__main">
        <label className="kp__field"><span>{t("cosmetics.gloveStyle")}</span><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("cosmetics.searchGlove")} /></label>
        {visible.length ? <div className="kp__skins" ref={skinListRef}>{visible.map((skin) => {
          const isSelected = skin.defindex === draft.def && skin.paint === (draft.paint ?? 0);
          return (
            <button key={`${skin.defindex}-${skin.paint}`} className={isSelected ? "is-selected" : ""} onClick={() => setDraft((value) => ({ ...value, def: skin.defindex, paint: skin.paint, wear: Math.min(skin.maxWear, Math.max(skin.minWear, value.wear ?? 0)) }))}>
              <img src={skin.image} alt="" loading="lazy" />
              <span>{modelName(skin.defindex)} · {skinName(skin)} [{skin.paint}]</span>
            </button>
          );
        })}</div> : <div className="wp-modal__empty">{t("music.noResults")}</div>}
      </div>
    </div>
  </Modal>;
}
