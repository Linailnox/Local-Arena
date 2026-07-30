import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Gem, ImageOff, RotateCcw, Save, Search, Trash2, UserRound } from "lucide-react";
import SubPage from "../components/SubPage";
import Toggle from "../components/Toggle";
import Segmented from "../components/Segmented";
import CosmeticsTeamSwitch, { useCosmeticsTeam } from "../components/CosmeticsTeamSwitch";
import { STICKERS, stickerName, type StickerCatalogEntry } from "../data/stickers";
import { WEAPON_ICONS } from "../data/weaponIcons";
import skinImages from "../data/skinImages.json";
import placementRows from "../data/cosmeticPlacements.json";
import charmRows from "../data/charmCatalog.json";
import agentRows from "../data/agentCatalog.json";
import { api, type CharmPreset, type KnifeCustomizerConfig, type StickerPreset } from "../lib/api";
import { useStore } from "../state/store";
import { useT } from "../i18n";
import {
  clampStickerValue,
  availableStickerSlotCount,
  defaultStickerPlacement,
  filterStickerCatalog,
  paginateStickerCatalog,
  removeSticker,
  replaceSticker,
  swapStickerSlots,
  updateGunPresetCharm,
  updateGunPresetStickers,
} from "../lib/stickerEditor";
import "./StickersPanel.css";

type EditorMode = "stickers" | "charms" | "agents";
type SkinImage = { weapon_defindex: number; paint: number | string; image: string };
type PreviewPosition = { schema: number; x: number; y: number };
type CharmPosition = { placementId: number; x: number; y: number };
type WeaponPlacement = {
  stickerSchemaCount: number;
  stickerPositions: PreviewPosition[];
  charmPositions: CharmPosition[];
};
type CharmCatalogEntry = { id: number; name: string; image: string };
type AgentCatalogEntry = { team: "ct" | "t"; model: string; name: string; image: string };

const PAGE_SIZE = 24;
const CHARM_PAGE_SIZE = 24;
const AGENT_PAGE_SIZE = 18;
const placements = placementRows as Record<string, WeaponPlacement>;
const charms = charmRows as CharmCatalogEntry[];
const agents = agentRows as AgentCatalogEntry[];
const imageMap = new Map((skinImages as SkinImage[]).map((row) => [`${row.weapon_defindex}:${Number(row.paint)}`, row.image]));
const stickerMap = new Map(STICKERS.map((entry) => [entry.id, entry]));

function equippedWeaponImage(weaponId: number, paint: number, fallback: string): string {
  return imageMap.get(`${weaponId}:${paint}`) ?? fallback;
}

type StickersPanelProps = {
  browserPreviewConfig?: KnifeCustomizerConfig;
};

export default function StickersPanel({ browserPreviewConfig }: StickersPanelProps = {}) {
  const { csgoPath, config: appConfig, process, reportError } = useStore();
  const [config, setConfig] = useState<KnifeCustomizerConfig | null>(browserPreviewConfig ?? null);
  const [team, setTeam] = useCosmeticsTeam();
  const [mode, setMode] = useState<EditorMode>("stickers");
  const [weaponId, setWeaponId] = useState<number | null>(null);
  const [slot, setSlot] = useState(0);
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [saving, setSaving] = useState(false);
  const previewRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ x: number; y: number; offsetX: number; offsetY: number } | null>(null);
  const t = useT();
  const running = !!process?.running;

  useEffect(() => {
    if (browserPreviewConfig) {
      setConfig(browserPreviewConfig);
      return;
    }
    if (!csgoPath) return setConfig(null);
    void api.getKnifeCustomizer(csgoPath).then((state) => setConfig(state.config)).catch(reportError);
  }, [browserPreviewConfig, csgoPath, reportError]);

  const configuredWeapons = useMemo(() => {
    if (!config || mode === "agents") return [];
    const presets = config.loadouts[team].gun_presets;
    return WEAPON_ICONS.filter((weapon) => {
      const capability = placements[String(weapon.id)];
      return !!capability
        && (mode === "stickers" || capability.charmPositions.length > 0)
        && (weapon.availability === team || weapon.availability === "shared")
        && !!presets[String(weapon.id)];
    });
  }, [config, mode, team]);

  useEffect(() => {
    if (!configuredWeapons.some((weapon) => weapon.id === weaponId))
      setWeaponId(configuredWeapons[0]?.id ?? null);
  }, [configuredWeapons, weaponId]);

  useEffect(() => {
    setPage(0);
    setQuery("");
  }, [mode]);

  const weapon = configuredWeapons.find((entry) => entry.id === weaponId) ?? null;
  const capability = weapon ? placements[String(weapon.id)] : null;
  const preset = weapon && config ? config.loadouts[team].gun_presets[String(weapon.id)] : null;
  const stickers = [...(preset?.stickers ?? [])].sort((left, right) => left.slot - right.slot);
  const stickerSlotCount = availableStickerSlotCount(capability?.stickerSchemaCount ?? 0);
  const selectedSticker = stickers.find((entry) => entry.slot === slot) ?? null;
  const selectedCatalog = selectedSticker ? stickerMap.get(selectedSticker.id) : undefined;
  const selectedCharm = preset?.charm ?? null;
  const selectedCharmCatalog = selectedCharm ? charms.find((entry) => entry.id === selectedCharm.id) ?? null : null;
  const selectedAgentModel = config?.loadouts[team].agent_model ?? "";
  const selectedAgent = agents.find((entry) => entry.team === team && entry.model === selectedAgentModel) ?? null;
  const charmPosition = capability?.charmPositions.find((entry) => entry.placementId === selectedCharm?.placement_id) ?? null;
  const weaponImage = weapon && preset ? equippedWeaponImage(weapon.id, preset.paint, weapon.url) : "";
  const filteredStickers = useMemo(() => filterStickerCatalog(
    STICKERS,
    query,
    (entry) => stickerName(entry, appConfig?.language),
  ), [appConfig?.language, query]);
  const filteredCharms = useMemo(() => filterStickerCatalog(charms, query, (entry) => entry.name), [query]);
  const filteredAgents = useMemo(() => {
    const value = query.trim().toLocaleLowerCase();
    return agents.filter((entry) => entry.team === team &&
      (!value || `${entry.name} ${entry.model}`.toLocaleLowerCase().includes(value)));
  }, [query, team]);
  const catalogPage = mode === "stickers" ? paginateStickerCatalog(filteredStickers, page, PAGE_SIZE)
    : mode === "charms" ? paginateStickerCatalog(filteredCharms, page, CHARM_PAGE_SIZE)
      : paginateStickerCatalog(filteredAgents, page, AGENT_PAGE_SIZE);

  useEffect(() => setPage(0), [query]);

  useEffect(() => {
    if (mode !== "stickers" || stickerSlotCount === 0 || slot < stickerSlotCount) return;
    if (!stickers.some((entry) => entry.slot === slot)) setSlot(stickerSlotCount - 1);
  }, [mode, slot, stickerSlotCount, stickers]);

  const setPresetStickers = (next: StickerPreset[]) => {
    if (!config || !weapon || !preset) return;
    setConfig(updateGunPresetStickers(config, team, weapon.id, preset, next));
  };

  const setPresetCharm = (charm: CharmPreset | null) => {
    if (!config || !weapon || !preset) return;
    setConfig(updateGunPresetCharm(config, team, weapon.id, preset, charm));
  };

  const chooseSticker = (entry: StickerCatalogEntry) => {
    if (!capability || slot >= stickerSlotCount) return;
    const placement = defaultStickerPlacement(slot, capability.stickerSchemaCount);
    if (!placement) return;
    const next: StickerPreset = {
      slot,
      id: entry.id,
      ...placement,
      wear: 0,
      scale: 1,
      rotation: 0,
    };
    setPresetStickers(replaceSticker(stickers, next));
  };

  const chooseCharm = (entry: CharmCatalogEntry) => {
    const placement = charmPosition ?? capability?.charmPositions[0];
    if (!placement) return;
    setPresetCharm({ id: entry.id, placement_id: placement.placementId, seed: selectedCharm?.seed ?? 0 });
  };

  const chooseCharmPlacement = (placementId: number) => {
    if (selectedCharm) setPresetCharm({ ...selectedCharm, placement_id: placementId });
    else setPresetCharm({ id: charms[0]?.id ?? 0, placement_id: placementId, seed: 0 });
  };

  const chooseAgent = (entry: AgentCatalogEntry | null) => {
    if (!config) return;
    setConfig({
      ...config,
      loadouts: {
        ...config.loadouts,
        [team]: { ...config.loadouts[team], agent_model: entry?.model ?? "" },
      },
    });
  };

  const updateSelected = (patch: Partial<StickerPreset>) => {
    if (!selectedSticker) return;
    setPresetStickers(stickers.map((entry) => entry.slot === slot ? { ...entry, ...patch } : entry));
  };

  const swapSlot = (delta: -1 | 1) => {
    const target = slot + delta;
    if (target < 0 || target >= stickerSlotCount) return;
    const swapped = swapStickerSlots(stickers, slot, target).map((entry) => {
      if (!capability || entry.slot < capability.stickerSchemaCount || entry.custom_position) return entry;
      const placement = defaultStickerPlacement(entry.slot, capability.stickerSchemaCount);
      return placement ? { ...entry, ...placement } : entry;
    });
    setPresetStickers(swapped);
    setSlot(target);
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragRef.current || !selectedSticker?.custom_position || !previewRef.current) return;
    const rect = previewRef.current.getBoundingClientRect();
    updateSelected({
      offset_x: clampStickerValue(dragRef.current.offsetX + ((event.clientX - dragRef.current.x) / rect.width) * 4, -1, 1),
      offset_y: clampStickerValue(dragRef.current.offsetY + ((event.clientY - dragRef.current.y) / rect.height) * 4, -1, 1),
    });
  };

  const onPreviewKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (mode !== "stickers" || !selectedSticker) return;
    const offsets: Record<string, Partial<StickerPreset>> = {
      ArrowLeft: { offset_x: clampStickerValue(selectedSticker.offset_x - 0.01, -1, 1), custom_position: true },
      ArrowRight: { offset_x: clampStickerValue(selectedSticker.offset_x + 0.01, -1, 1), custom_position: true },
      ArrowUp: { offset_y: clampStickerValue(selectedSticker.offset_y - 0.01, -1, 1), custom_position: true },
      ArrowDown: { offset_y: clampStickerValue(selectedSticker.offset_y + 0.01, -1, 1), custom_position: true },
      q: { rotation: clampStickerValue(selectedSticker.rotation - 1, 0, 360) },
      e: { rotation: clampStickerValue(selectedSticker.rotation + 1, 0, 360) },
    };
    const patch = offsets[event.key] ?? offsets[event.key.toLocaleLowerCase()];
    if (!patch) return;
    event.preventDefault();
    updateSelected(patch);
  };

  const persist = async () => {
    if (browserPreviewConfig || !csgoPath || !config || running) return;
    setSaving(true);
    try {
      const state = await api.saveKnifeCustomizer(csgoPath, {
        ...config,
        enabled: true,
        stickers_enabled: true,
        charms_enabled: true,
        agents_enabled: true,
      });
      setConfig(state.config);
    } catch (error) { reportError(error); }
    finally { setSaving(false); }
  };

  return <SubPage
    title={t("stickers.title")}
    status={!csgoPath ? "off" : running ? "yellow" : "green"}
    right={<button className="stickers-save" disabled={!!browserPreviewConfig || !config || running || saving} onClick={() => void persist()}>
      <Save size={15} />{saving ? t("weapons.saving") : t("weapons.apply")}
    </button>}
  >
    <div className="stickers-page">
      <section className="stickers-weapons">
        <header><strong>{mode === "agents" ? t("stickers.agentTeam") : t("stickers.weapon")}</strong><CosmeticsTeamSwitch value={team} onChange={setTeam} ariaLabel={t("weapons.teamLoadout")} compact /></header>
        <div>
          {mode === "agents" ? <button className="agent-current is-active" onClick={() => {}}>
            {selectedAgent ? <CosmeticImage image={selectedAgent.image} kind="agent" lazy /> : <UserRound size={24} />}
            <span>{selectedAgent?.name ?? t("stickers.defaultAgent")}</span>
          </button> : configuredWeapons.map((entry) => {
            const entryPreset = config?.loadouts[team].gun_presets[String(entry.id)];
            const image = entryPreset ? equippedWeaponImage(entry.id, entryPreset.paint, entry.url) : entry.url;
            return <button key={entry.id} className={entry.id === weaponId ? "is-active" : ""} onClick={() => setWeaponId(entry.id)}>
              <img src={image} alt="" /><span>{entry.name}</span>
            </button>;
          })}
          {mode !== "agents" && !configuredWeapons.length && <p>{mode === "stickers" ? t("stickers.noWeapons") : t("stickers.noCharmWeapons")}</p>}
        </div>
      </section>

      <section className="stickers-editor">
        <div className="stickers-editor__toolbar">
          <Segmented
            options={[
              { value: "stickers", label: t("stickers.tab") },
              { value: "charms", label: t("stickers.charmsTab") },
              { value: "agents", label: t("stickers.agentsTab") },
            ]}
            value={mode}
            onChange={setMode}
            ariaLabel={t("stickers.mode")}
          />
        </div>
        <div
          className="stickers-preview"
          ref={previewRef}
          tabIndex={0}
          onKeyDown={onPreviewKeyDown}
          onPointerMove={onPointerMove}
          onPointerUp={() => { dragRef.current = null; }}
          onPointerLeave={() => { dragRef.current = null; }}
        >
          {weaponImage && <img className="stickers-preview__weapon" src={weaponImage} alt="" />}
          {mode === "agents" && <div className="stickers-preview__agent">
            {selectedAgent ? <CosmeticImage image={selectedAgent.image} kind="agent" /> : <UserRound size={76} />}
            <strong>{selectedAgent?.name ?? t("stickers.defaultAgent")}</strong>
            <small>{selectedAgent?.model ?? t("stickers.defaultAgentDetail")}</small>
          </div>}
          {mode === "stickers" && capability?.stickerPositions.map((position) => <button
            key={position.schema}
            className={`stickers-preview__hotspot ${selectedSticker?.schema === position.schema ? "is-active" : ""}`}
            style={{ left: `${position.x * 100}%`, top: `${position.y * 100}%` }}
            disabled={!selectedSticker}
            onClick={() => {
              const placement = capability && defaultStickerPlacement(slot, capability.stickerSchemaCount);
              updateSelected({
                schema: position.schema,
                offset_x: placement?.offset_x ?? 0,
                offset_y: placement?.offset_y ?? 0,
                custom_position: placement?.custom_position ?? false,
              });
            }}
            title={t("stickers.position", { n: position.schema + 1 })}
          ><span>{position.schema + 1}</span></button>)}
          {mode === "stickers" && stickers.map((entry) => {
            const catalog = stickerMap.get(entry.id);
            const anchor = capability?.stickerPositions.find((position) => position.schema === entry.schema);
            if (!anchor) return null;
            return <button
              key={entry.slot}
              className={`stickers-preview__sticker ${entry.slot === slot ? "is-active" : ""}`}
              style={{
                left: `${anchor.x * 100 + (entry.custom_position ? entry.offset_x * 18 : 0)}%`,
                top: `${anchor.y * 100 + (entry.custom_position ? entry.offset_y * 18 : 0)}%`,
                transform: `translate(-50%, -50%) rotate(${entry.rotation}deg) scale(${entry.scale})`,
                opacity: Math.max(.18, 1 - entry.wear * .82),
              }}
              onClick={() => setSlot(entry.slot)}
              onPointerDown={(event) => {
                setSlot(entry.slot);
                if (!entry.custom_position) return;
                dragRef.current = { x: event.clientX, y: event.clientY, offsetX: entry.offset_x, offsetY: entry.offset_y };
                event.currentTarget.setPointerCapture(event.pointerId);
              }}
            >{catalog ? <StickerImage entry={catalog} /> : <span>{entry.slot + 1}</span>}</button>;
          })}
          {mode === "charms" && capability?.charmPositions.map((position, index) => <button
            key={position.placementId}
            className={`stickers-preview__charm-hotspot ${selectedCharm?.placement_id === position.placementId ? "is-active" : ""}`}
            style={{ left: `${position.x * 100}%`, top: `${position.y * 100}%` }}
            onClick={() => chooseCharmPlacement(position.placementId)}
            title={t("stickers.charmPosition", { n: index + 1 })}
          >{selectedCharm?.placement_id === position.placementId && selectedCharmCatalog
              ? <CosmeticImage image={selectedCharmCatalog.image} kind="charm" />
              : <Gem size={15} />}</button>)}
        </div>

        {mode === "stickers" ? <StickerControls
          stickers={stickers}
          slot={slot}
          selectedSticker={selectedSticker}
          selectedCatalog={selectedCatalog}
          slotCount={stickerSlotCount}
          language={appConfig?.language}
          setSlot={setSlot}
          swapSlot={swapSlot}
          updateSelected={updateSelected}
          defaultPlacement={capability && defaultStickerPlacement(slot, capability.stickerSchemaCount)}
          remove={() => setPresetStickers(removeSticker(stickers, slot))}
        /> : mode === "charms"
          ? <CharmControls charm={selectedCharm} catalog={selectedCharmCatalog} remove={() => setPresetCharm(null)} update={setPresetCharm} />
          : <AgentControls agent={selectedAgent} clear={() => chooseAgent(null)} />}
      </section>

      <section className="sticker-catalog">
        <label><Search size={15} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={mode === "stickers" ? t("stickers.search") : mode === "charms" ? t("stickers.searchCharms") : t("stickers.searchAgents")} /></label>
        <div className={`sticker-catalog__grid ${mode === "charms" ? "is-charms" : mode === "agents" ? "is-agents" : ""}`}>
          {mode === "stickers" ? (catalogPage.entries as StickerCatalogEntry[]).map((entry) => <button key={entry.id} className={entry.id === selectedSticker?.id ? "is-active" : ""} disabled={slot >= stickerSlotCount} onClick={() => chooseSticker(entry)} title={`${stickerName(entry, appConfig?.language)} · ${entry.id}`}>
            <StickerImage entry={entry} lazy /><span>{stickerName(entry, appConfig?.language)}</span><small>#{entry.id}</small>
          </button>) : mode === "charms" ? (catalogPage.entries as CharmCatalogEntry[]).map((entry) => <button key={entry.id} className={entry.id === selectedCharm?.id ? "is-active" : ""} onClick={() => chooseCharm(entry)} title={entry.name}>
            <CosmeticImage image={entry.image} kind="charm" lazy /><span>{entry.name}</span><small>#{entry.id}</small>
          </button>) : (catalogPage.entries as AgentCatalogEntry[]).map((entry) => <button key={entry.model} className={entry.model === selectedAgentModel ? "is-active" : ""} onClick={() => chooseAgent(entry)} title={entry.model}>
            <CosmeticImage image={entry.image} kind="agent" lazy /><span>{entry.name}</span><small>{entry.model.split("\\").slice(-1)[0]?.replace(".vmdl", "")}</small>
          </button>)}
        </div>
        <footer><button disabled={catalogPage.page === 0} onClick={() => setPage((value) => value - 1)}><ArrowLeft size={15} /></button><span>{catalogPage.page + 1} / {catalogPage.pageCount}</span><button disabled={catalogPage.page + 1 >= catalogPage.pageCount} onClick={() => setPage((value) => value + 1)}><ArrowRight size={15} /></button></footer>
      </section>
    </div>
  </SubPage>;
}

function StickerControls({ stickers, slot, selectedSticker, selectedCatalog, slotCount, language, setSlot, swapSlot, updateSelected, defaultPlacement, remove }: {
  stickers: StickerPreset[];
  slot: number;
  selectedSticker: StickerPreset | null;
  selectedCatalog: StickerCatalogEntry | undefined;
  slotCount: number;
  language: string | null | undefined;
  setSlot: (slot: number) => void;
  swapSlot: (delta: -1 | 1) => void;
  updateSelected: (patch: Partial<StickerPreset>) => void;
  defaultPlacement: Pick<StickerPreset, "schema" | "offset_x" | "offset_y" | "custom_position"> | null;
  remove: () => void;
}) {
  const t = useT();
  const slotIndexes = Array.from(new Set([
    ...Array.from({ length: slotCount }, (_, index) => index),
    ...stickers.map((entry) => entry.slot).filter((index) => index >= slotCount && index < 5),
  ])).sort((left, right) => left - right);
  return <>
    <div className="sticker-slots" style={{ gridTemplateColumns: `repeat(${Math.max(1, slotIndexes.length)}, minmax(0, 1fr))` }}>
      {slotIndexes.map((index) => {
        const entry = stickers.find((item) => item.slot === index);
        const catalog = entry ? stickerMap.get(entry.id) : null;
        return <button key={index} className={`${slot === index ? "is-active" : ""} ${index >= slotCount ? "is-unavailable" : ""}`} onClick={() => setSlot(index)}>
          <b>{index + 1}</b>{catalog ? <StickerImage entry={catalog} /> : <span>+</span>}
        </button>;
      })}
    </div>
    <div className="sticker-controls">
      <header>
        <span><small>{t("stickers.slot", { n: slot + 1 })}</small><strong>{selectedCatalog ? stickerName(selectedCatalog, language) : t("stickers.empty")}</strong></span>
        <div>
          <button disabled={!selectedSticker || slot === 0 || slot >= slotCount} onClick={() => swapSlot(-1)} title={t("stickers.moveLeft")}><ArrowLeft size={15} /></button>
          <button disabled={!selectedSticker || slot + 1 >= slotCount} onClick={() => swapSlot(1)} title={t("stickers.moveRight")}><ArrowRight size={15} /></button>
          <button disabled={!selectedSticker} onClick={remove} title={t("stickers.remove")}><Trash2 size={15} /></button>
        </div>
      </header>
      {selectedSticker && <>
        <Control label={t("stickers.wear")} value={selectedSticker.wear} min={0} max={1} step={0.01} onChange={(wear) => updateSelected({ wear })} />
        <Control label={t("stickers.scale")} value={selectedSticker.scale} min={0.1} max={2} step={0.01} onChange={(scale) => updateSelected({ scale })} />
        <Control label={t("stickers.rotation")} value={selectedSticker.rotation} min={0} max={360} step={1} onChange={(rotation) => updateSelected({ rotation })} />
        <label className="sticker-custom"><span>{t("stickers.customPosition")}</span><Toggle checked={selectedSticker.custom_position} disabled={!!defaultPlacement?.custom_position} onChange={(custom_position) => updateSelected({ custom_position })} /></label>
        <details className="sticker-advanced">
          <summary>{t("stickers.advanced")}</summary>
          <Control label="X" value={selectedSticker.offset_x} min={-1} max={1} step={0.01} disabled={!selectedSticker.custom_position} onChange={(offset_x) => updateSelected({ offset_x })} />
          <Control label="Y" value={selectedSticker.offset_y} min={-1} max={1} step={0.01} disabled={!selectedSticker.custom_position} onChange={(offset_y) => updateSelected({ offset_y })} />
        </details>
        <button className="sticker-reset" onClick={() => updateSelected({ wear: 0, scale: 1, rotation: 0, ...(defaultPlacement ?? { offset_x: 0, offset_y: 0, custom_position: false }) })}><RotateCcw size={14} />{t("stickers.reset")}</button>
      </>}
    </div>
  </>;
}

function CharmControls({ charm, catalog, remove, update }: { charm: CharmPreset | null; catalog: CharmCatalogEntry | null; remove: () => void; update: (charm: CharmPreset) => void }) {
  const t = useT();
  return <div className="sticker-controls charm-controls">
    <header>
      <span className="cosmetic-selection">{charm && catalog && <CosmeticImage image={catalog.image} kind="charm" />}
        <span><small>{t("stickers.charm")}</small><strong>{catalog?.name ?? (charm ? `Charm #${charm.id}` : t("stickers.noCharm"))}</strong></span>
      </span>
      <div><button disabled={!charm} onClick={remove} title={t("stickers.removeCharm")}><Trash2 size={15} /></button></div>
    </header>
    {charm && <Control label={t("stickers.seed")} value={charm.seed} min={0} max={2147483647} step={1} onChange={(seed) => update({ ...charm, seed: Math.trunc(seed) })} />}
  </div>;
}

function AgentControls({ agent, clear }: { agent: AgentCatalogEntry | null; clear: () => void }) {
  const t = useT();
  return <div className="sticker-controls agent-controls">
    <header>
      <span><small>{t("stickers.selectedAgent")}</small><strong>{agent?.name ?? t("stickers.defaultAgent")}</strong></span>
      <div><button disabled={!agent} onClick={clear} title={t("stickers.clearAgent")}><Trash2 size={15} /></button></div>
    </header>
  </div>;
}

function Control({ label, value, min, max, step, disabled, onChange }: {
  label: string; value: number; min: number; max: number; step: number; disabled?: boolean; onChange: (value: number) => void;
}) {
  return <label className="sticker-control"><span>{label}</span><input type="range" value={value} min={min} max={max} step={step} disabled={disabled} onChange={(event) => onChange(clampStickerValue(Number(event.target.value), min, max))} /><input type="number" value={value} min={min} max={max} step={step} disabled={disabled} onChange={(event) => onChange(clampStickerValue(Number(event.target.value), min, max))} /></label>;
}

function StickerImage({ entry, lazy = false }: { entry: StickerCatalogEntry; lazy?: boolean }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [entry.image]);
  return failed || !entry.image
    ? <span className="sticker-image-fallback" aria-hidden="true"><ImageOff size={16} /></span>
    : <img src={entry.image} alt="" loading={lazy ? "lazy" : undefined} draggable={false} onError={() => setFailed(true)} />;
}

function CosmeticImage({ image, kind, lazy = false }: { image: string; kind: "charm" | "agent"; lazy?: boolean }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [image]);
  if (failed || !image) return <span className={`cosmetic-image-fallback is-${kind}`} aria-hidden="true">
    {kind === "charm" ? <Gem size={20} /> : <UserRound size={20} />}
  </span>;
  return <img className={`cosmetic-image is-${kind}`} src={image} alt="" loading={lazy ? "lazy" : undefined} draggable={false} onError={() => setFailed(true)} />;
}
