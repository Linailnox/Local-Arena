import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Gem, ImageOff, RotateCcw, Save, Search, Trash2 } from "lucide-react";
import SubPage from "../components/SubPage";
import Toggle from "../components/Toggle";
import Segmented from "../components/Segmented";
import CosmeticsTeamSwitch, { useCosmeticsTeam } from "../components/CosmeticsTeamSwitch";
import { STICKERS, stickerName, type StickerCatalogEntry } from "../data/stickers";
import { WEAPON_ICONS } from "../data/weaponIcons";
import skinImages from "../data/skinImages.json";
import placementRows from "../data/cosmeticPlacements.json";
import charmRows from "../data/charmCatalog.json";
import { api, type CosmeticsInventoryState, type InventoryItem, type InventoryKeychain, type InventorySticker } from "../lib/api";
import { configuredWeaponIds, teamWeapons } from "../lib/cosmeticsInventory";
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

type EditorMode = "stickers" | "charms";
type SkinImage = { weapon_defindex: number; paint: number | string; image: string };
type PreviewPosition = { schema: number; x: number; y: number };
type CharmPosition = { placementId: number; x: number; y: number };
type WeaponPlacement = {
  stickerSchemaCount: number;
  stickerPositions: PreviewPosition[];
  charmPositions: CharmPosition[];
};
type CharmCatalogEntry = { id: number; name: string; image: string };

const PAGE_SIZE = 24;
const CHARM_PAGE_SIZE = 24;
const placements = placementRows as Record<string, WeaponPlacement>;
const charms = charmRows as CharmCatalogEntry[];
const imageMap = new Map((skinImages as SkinImage[]).map((row) => [`${row.weapon_defindex}:${Number(row.paint)}`, row.image]));
const stickerMap = new Map(STICKERS.map((entry) => [entry.id, entry]));

function equippedWeaponImage(weaponId: number, paint: number, fallback: string): string {
  return imageMap.get(`${weaponId}:${paint}`) ?? fallback;
}

function hasCustomPosition(sticker: InventorySticker): boolean {
  return sticker.x !== 0 || sticker.y !== 0;
}

type StickersPanelProps = {
  browserPreviewState?: CosmeticsInventoryState;
};

export default function StickersPanel({ browserPreviewState }: StickersPanelProps = {}) {
  const { csgoPath, config: appConfig, process, reportError } = useStore();
  const [state, setState] = useState<CosmeticsInventoryState | null>(browserPreviewState ?? null);
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
    if (browserPreviewState) {
      setState(browserPreviewState);
      return;
    }
    if (!csgoPath) return setState(null);
    void api.getCosmeticsInventory(csgoPath).then(setState).catch(reportError);
  }, [browserPreviewState, csgoPath, reportError]);

  const configuredWeapons = useMemo(() => {
    if (!state) return [];
    const configured = configuredWeaponIds(state.inventory, team);
    return WEAPON_ICONS.filter((weapon) => {
      const capability = placements[String(weapon.id)];
      return !!capability
        && (mode === "stickers" || capability.charmPositions.length > 0)
        && (weapon.availability === team || weapon.availability === "shared")
        && configured.has(weapon.id);
    });
  }, [state, mode, team]);

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
  const item: InventoryItem | null = weapon && state ? teamWeapons(state.inventory, team)[String(weapon.id)] ?? null : null;
  const stickers = [...(item?.stickers ?? [])].sort((left, right) => left.slot - right.slot);
  const stickerSlotCount = availableStickerSlotCount(capability?.stickerSchemaCount ?? 0);
  const selectedSticker = stickers.find((entry) => entry.slot === slot) ?? null;
  const selectedCatalog = selectedSticker ? stickerMap.get(selectedSticker.def) : undefined;
  const selectedKeychain: InventoryKeychain | null = item?.keychains?.[0] ?? null;
  const selectedCharmCatalog = selectedKeychain ? charms.find((entry) => entry.id === selectedKeychain.def) ?? null : null;
  const charmPosition = capability?.charmPositions.find((entry) =>
    selectedKeychain && Math.abs(entry.x - selectedKeychain.x) < 1e-3 && Math.abs(entry.y - selectedKeychain.y) < 1e-3) ?? null;
  const weaponImage = weapon && item ? equippedWeaponImage(weapon.id, item.paint ?? 0, weapon.url) : "";
  const filteredStickers = useMemo(() => filterStickerCatalog(
    STICKERS,
    query,
    (entry) => stickerName(entry, appConfig?.language),
  ), [appConfig?.language, query]);
  const filteredCharms = useMemo(() => filterStickerCatalog(charms, query, (entry) => entry.name), [query]);
  const catalogPage = mode === "stickers"
    ? paginateStickerCatalog(filteredStickers, page, PAGE_SIZE)
    : paginateStickerCatalog(filteredCharms, page, CHARM_PAGE_SIZE);

  useEffect(() => setPage(0), [query]);

  useEffect(() => {
    if (mode !== "stickers" || stickerSlotCount === 0 || slot < stickerSlotCount) return;
    if (!stickers.some((entry) => entry.slot === slot)) setSlot(stickerSlotCount - 1);
  }, [mode, slot, stickerSlotCount, stickers]);

  const setPresetStickers = (next: InventorySticker[]) => {
    if (!state || !weapon || !item) return;
    setState({ ...state, inventory: updateGunPresetStickers(state.inventory, team, weapon.id, item, next) });
  };

  const setPresetKeychain = (keychain: InventoryKeychain | null) => {
    if (!state || !weapon || !item) return;
    setState({ ...state, inventory: updateGunPresetCharm(state.inventory, team, weapon.id, item, keychain) });
  };

  const chooseSticker = (entry: StickerCatalogEntry) => {
    if (!capability || slot >= stickerSlotCount) return;
    const placement = defaultStickerPlacement(slot, capability.stickerSchemaCount);
    if (!placement) return;
    setPresetStickers(replaceSticker(stickers, {
      slot,
      def: entry.id,
      schema: placement.schema,
      x: placement.x,
      y: placement.y,
      wear: 0,
      rotation: 0,
    }));
  };

  const chooseCharm = (entry: CharmCatalogEntry) => {
    const placement = charmPosition ?? capability?.charmPositions[0];
    if (!placement) return;
    setPresetKeychain({ def: entry.id, slot: 0, seed: selectedKeychain?.seed ?? 0, x: placement.x, y: placement.y, z: 0 });
  };

  const chooseCharmPlacement = (position: CharmPosition) => {
    setPresetKeychain({
      def: selectedKeychain?.def ?? charms[0]?.id ?? 0,
      slot: 0,
      seed: selectedKeychain?.seed ?? 0,
      x: position.x,
      y: position.y,
      z: 0,
    });
  };

  const updateSelected = (patch: Partial<InventorySticker>) => {
    if (!selectedSticker) return;
    setPresetStickers(stickers.map((entry) => entry.slot === slot ? { ...entry, ...patch } : entry));
  };

  const swapSlot = (delta: -1 | 1) => {
    const target = slot + delta;
    if (target < 0 || target >= stickerSlotCount) return;
    const swapped = swapStickerSlots(stickers, slot, target).map((entry) => {
      if (!capability || entry.slot < capability.stickerSchemaCount || hasCustomPosition(entry)) return entry;
      const placement = defaultStickerPlacement(entry.slot, capability.stickerSchemaCount);
      return placement ? { ...entry, ...placement } : entry;
    });
    setPresetStickers(swapped);
    setSlot(target);
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragRef.current || !selectedSticker || !hasCustomPosition(selectedSticker) || !previewRef.current) return;
    const rect = previewRef.current.getBoundingClientRect();
    updateSelected({
      x: clampStickerValue(dragRef.current.offsetX + ((event.clientX - dragRef.current.x) / rect.width) * 4, -1, 1),
      y: clampStickerValue(dragRef.current.offsetY + ((event.clientY - dragRef.current.y) / rect.height) * 4, -1, 1),
    });
  };

  const onPreviewKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (mode !== "stickers" || !selectedSticker) return;
    const offsets: Record<string, Partial<InventorySticker>> = {
      ArrowLeft: { x: clampStickerValue(selectedSticker.x - 0.01, -1, 1) },
      ArrowRight: { x: clampStickerValue(selectedSticker.x + 0.01, -1, 1) },
      ArrowUp: { y: clampStickerValue(selectedSticker.y - 0.01, -1, 1) },
      ArrowDown: { y: clampStickerValue(selectedSticker.y + 0.01, -1, 1) },
      q: { rotation: clampStickerValue(selectedSticker.rotation - 1, 0, 360) },
      e: { rotation: clampStickerValue(selectedSticker.rotation + 1, 0, 360) },
    };
    const patch = offsets[event.key] ?? offsets[event.key.toLocaleLowerCase()];
    if (!patch) return;
    event.preventDefault();
    updateSelected(patch);
  };

  const persist = async () => {
    if (browserPreviewState || !csgoPath || !state || running) return;
    setSaving(true);
    try {
      const panel = { ...state.panel, enabled: true, stickers_enabled: true, charms_enabled: true };
      const next = await api.saveCosmeticsInventory(csgoPath, state.steam_id, state.inventory, panel);
      setState(next);
    } catch (error) { reportError(error); }
    finally { setSaving(false); }
  };

  return <SubPage
    title={t("stickers.title")}
    status={!csgoPath ? "off" : running ? "yellow" : "green"}
    right={<button className="stickers-save" disabled={!!browserPreviewState || !state || running || saving} onClick={() => void persist()}>
      <Save size={15} />{saving ? t("weapons.saving") : t("weapons.apply")}
    </button>}
  >
    <div className="stickers-page">
      <section className="stickers-weapons">
        <header><strong>{t("stickers.weapon")}</strong><CosmeticsTeamSwitch value={team} onChange={setTeam} ariaLabel={t("weapons.teamLoadout")} compact /></header>
        <div>
          {configuredWeapons.map((entry) => {
            const entryItem = state ? teamWeapons(state.inventory, team)[String(entry.id)] : undefined;
            const image = entryItem ? equippedWeaponImage(entry.id, entryItem.paint ?? 0, entry.url) : entry.url;
            return <button key={entry.id} className={entry.id === weaponId ? "is-active" : ""} onClick={() => setWeaponId(entry.id)}>
              <img src={image} alt="" /><span>{entry.name}</span>
            </button>;
          })}
          {!configuredWeapons.length && <p>{mode === "stickers" ? t("stickers.noWeapons") : t("stickers.noCharmWeapons")}</p>}
        </div>
      </section>

      <section className="stickers-editor">
        <div className="stickers-editor__toolbar">
          <Segmented
            options={[
              { value: "stickers", label: t("stickers.tab") },
              { value: "charms", label: t("stickers.charmsTab") },
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
          {mode === "stickers" && capability?.stickerPositions.map((position) => <button
            key={position.schema}
            className={`stickers-preview__hotspot ${selectedSticker?.schema === position.schema ? "is-active" : ""}`}
            style={{ left: `${position.x * 100}%`, top: `${position.y * 100}%` }}
            disabled={!selectedSticker}
            onClick={() => {
              const placement = capability && defaultStickerPlacement(slot, capability.stickerSchemaCount);
              updateSelected({ schema: position.schema, x: placement?.x ?? 0, y: placement?.y ?? 0 });
            }}
            title={t("stickers.position", { n: position.schema + 1 })}
          ><span>{position.schema + 1}</span></button>)}
          {mode === "stickers" && stickers.map((entry) => {
            const catalog = stickerMap.get(entry.def);
            const anchor = capability?.stickerPositions.find((position) => position.schema === entry.schema);
            if (!anchor) return null;
            const custom = hasCustomPosition(entry);
            return <button
              key={entry.slot}
              className={`stickers-preview__sticker ${entry.slot === slot ? "is-active" : ""}`}
              style={{
                left: `${anchor.x * 100 + (custom ? entry.x * 18 : 0)}%`,
                top: `${anchor.y * 100 + (custom ? entry.y * 18 : 0)}%`,
                transform: `translate(-50%, -50%) rotate(${entry.rotation}deg)`,
                opacity: Math.max(.18, 1 - entry.wear * .82),
              }}
              onClick={() => setSlot(entry.slot)}
              onPointerDown={(event) => {
                setSlot(entry.slot);
                if (!custom) return;
                dragRef.current = { x: event.clientX, y: event.clientY, offsetX: entry.x, offsetY: entry.y };
                event.currentTarget.setPointerCapture(event.pointerId);
              }}
            >{catalog ? <StickerImage entry={catalog} /> : <span>{entry.slot + 1}</span>}</button>;
          })}
          {mode === "charms" && capability?.charmPositions.map((position, index) => <button
            key={position.placementId}
            className={`stickers-preview__charm-hotspot ${charmPosition?.placementId === position.placementId ? "is-active" : ""}`}
            style={{ left: `${position.x * 100}%`, top: `${position.y * 100}%` }}
            onClick={() => chooseCharmPlacement(position)}
            title={t("stickers.charmPosition", { n: index + 1 })}
          >{charmPosition?.placementId === position.placementId && selectedCharmCatalog
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
        /> : <CharmControls
          charm={selectedKeychain}
          catalog={selectedCharmCatalog}
          remove={() => setPresetKeychain(null)}
          update={(seed) => selectedKeychain && setPresetKeychain({ ...selectedKeychain, seed })}
        />}
      </section>

      <section className="sticker-catalog">
        <label><Search size={15} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={mode === "stickers" ? t("stickers.search") : t("stickers.searchCharms")} /></label>
        <div className={`sticker-catalog__grid ${mode === "charms" ? "is-charms" : ""}`}>
          {mode === "stickers" ? (catalogPage.entries as StickerCatalogEntry[]).map((entry) => <button key={entry.id} className={entry.id === selectedSticker?.def ? "is-active" : ""} disabled={slot >= stickerSlotCount} onClick={() => chooseSticker(entry)} title={`${stickerName(entry, appConfig?.language)} · ${entry.id}`}>
            <StickerImage entry={entry} lazy /><span>{stickerName(entry, appConfig?.language)}</span><small>#{entry.id}</small>
          </button>) : (catalogPage.entries as CharmCatalogEntry[]).map((entry) => <button key={entry.id} className={entry.id === selectedKeychain?.def ? "is-active" : ""} onClick={() => chooseCharm(entry)} title={entry.name}>
            <CosmeticImage image={entry.image} kind="charm" lazy /><span>{entry.name}</span><small>#{entry.id}</small>
          </button>)}
        </div>
        <footer><button disabled={catalogPage.page === 0} onClick={() => setPage((value) => value - 1)}><ArrowLeft size={15} /></button><span>{catalogPage.page + 1} / {catalogPage.pageCount}</span><button disabled={catalogPage.page + 1 >= catalogPage.pageCount} onClick={() => setPage((value) => value + 1)}><ArrowRight size={15} /></button></footer>
      </section>
    </div>
  </SubPage>;
}

function StickerControls({ stickers, slot, selectedSticker, selectedCatalog, slotCount, language, setSlot, swapSlot, updateSelected, defaultPlacement, remove }: {
  stickers: InventorySticker[];
  slot: number;
  selectedSticker: InventorySticker | null;
  selectedCatalog: StickerCatalogEntry | undefined;
  slotCount: number;
  language: string | null | undefined;
  setSlot: (slot: number) => void;
  swapSlot: (delta: -1 | 1) => void;
  updateSelected: (patch: Partial<InventorySticker>) => void;
  defaultPlacement: Pick<InventorySticker, "schema" | "x" | "y"> | null;
  remove: () => void;
}) {
  const t = useT();
  const slotIndexes = Array.from(new Set([
    ...Array.from({ length: slotCount }, (_, index) => index),
    ...stickers.map((entry) => entry.slot).filter((index) => index >= slotCount && index < 5),
  ])).sort((left, right) => left - right);
  const custom = !!selectedSticker && hasCustomPosition(selectedSticker);
  return <>
    <div className="sticker-slots" style={{ gridTemplateColumns: `repeat(${Math.max(1, slotIndexes.length)}, minmax(0, 1fr))` }}>
      {slotIndexes.map((index) => {
        const entry = stickers.find((item) => item.slot === index);
        const catalog = entry ? stickerMap.get(entry.def) : null;
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
        <Control label={t("stickers.rotation")} value={selectedSticker.rotation} min={0} max={360} step={1} onChange={(rotation) => updateSelected({ rotation })} />
        <label className="sticker-custom"><span>{t("stickers.customPosition")}</span><Toggle checked={custom} onChange={(enabled) => updateSelected(enabled ? { x: 0.05, y: 0.05 } : { x: defaultPlacement?.x ?? 0, y: defaultPlacement?.y ?? 0 })} /></label>
        <details className="sticker-advanced">
          <summary>{t("stickers.advanced")}</summary>
          <Control label="X" value={selectedSticker.x} min={-1} max={1} step={0.01} disabled={!custom} onChange={(x) => updateSelected({ x })} />
          <Control label="Y" value={selectedSticker.y} min={-1} max={1} step={0.01} disabled={!custom} onChange={(y) => updateSelected({ y })} />
        </details>
        <button className="sticker-reset" onClick={() => updateSelected({ wear: 0, rotation: 0, x: defaultPlacement?.x ?? 0, y: defaultPlacement?.y ?? 0 })}><RotateCcw size={14} />{t("stickers.reset")}</button>
      </>}
    </div>
  </>;
}

function CharmControls({ charm, catalog, remove, update }: { charm: InventoryKeychain | null; catalog: CharmCatalogEntry | null; remove: () => void; update: (seed: number) => void }) {
  const t = useT();
  return <div className="sticker-controls charm-controls">
    <header>
      <span className="cosmetic-selection">{charm && catalog && <CosmeticImage image={catalog.image} kind="charm" />}
        <span><small>{t("stickers.charm")}</small><strong>{catalog?.name ?? (charm ? `Charm #${charm.def}` : t("stickers.noCharm"))}</strong></span>
      </span>
      <div><button disabled={!charm} onClick={remove} title={t("stickers.removeCharm")}><Trash2 size={15} /></button></div>
    </header>
    {charm && <Control label={t("stickers.seed")} value={charm.seed} min={0} max={2147483647} step={1} onChange={(seed) => update(Math.trunc(seed))} />}
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

function CosmeticImage({ image, kind, lazy = false }: { image: string; kind: "charm"; lazy?: boolean }) {
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [image]);
  if (failed || !image) return <span className={`cosmetic-image-fallback is-${kind}`} aria-hidden="true"><Gem size={20} /></span>;
  return <img className={`cosmetic-image is-${kind}`} src={image} alt="" loading={lazy ? "lazy" : undefined} draggable={false} onError={() => setFailed(true)} />;
}
