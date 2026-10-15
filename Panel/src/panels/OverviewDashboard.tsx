import { useEffect, useState } from "react";
import {
  ArrowRight,
  BarChart3,
  BookOpenText,
  Command,
  Crosshair,
  Download,
  Play,
  Settings2,
  SlidersHorizontal,
  Swords,
  type LucideIcon,
} from "lucide-react";
import StatusBar from "../components/StatusBar";
import ModeCard from "./ModeCard";
import DifficultyCard from "./DifficultyCard";
import { api, type OnlineUpdateSnapshot } from "../lib/api";
import type { Cs2ssMatchWithStats } from "../data/cs2ssTypes";
import { cs2ssMapLabel } from "../data/cs2ssMaps";
import { useStore } from "../state/store";
import { MAP_IMAGES } from "../data/maps";
import { useT, type I18nKey } from "../i18n";

export type DashboardTarget =
  | "match" | "stats" | "settings" | "presets" | "commands" | "weaponPresets" | "guide";

type Tile = { view: DashboardTarget; key: I18nKey; icon: LucideIcon };

const TILES: Tile[] = [
  { view: "match", key: "match.title", icon: Swords },
  { view: "weaponPresets", key: "weapons.title", icon: Crosshair },
  { view: "presets", key: "pre.title", icon: SlidersHorizontal },
  { view: "stats", key: "stats.globalHistory", icon: BarChart3 },
  { view: "commands", key: "cmd.title", icon: Command },
  { view: "guide", key: "nav.guide", icon: BookOpenText },
  { view: "settings", key: "set.title", icon: Settings2 },
];

export default function OverviewDashboard({ onNavigate }: { onNavigate: (view: DashboardTarget) => void }) {
  const t = useT();
  const { directory } = useStore();
  const csgo = directory?.valid ? directory.selected : null;
  const [updates, setUpdates] = useState<OnlineUpdateSnapshot | null>(null);
  const [matches, setMatches] = useState<Cs2ssMatchWithStats[] | null>(null);

  useEffect(() => {
    void api.getUpdateSnapshot().then(setUpdates).catch(() => {});
  }, []);

  useEffect(() => {
    if (!csgo) {
      setMatches(null);
      return;
    }
    void api.listCs2ssMatchesWithStats(csgo).then(setMatches).catch(() => setMatches([]));
  }, [csgo]);

  const updateAvailable = !!updates && (updates.panel.update_available || updates.plugin.update_available);
  const latest = matches?.[0] ?? null;

  return (
    <div className="dashboard">
      <header className="workspace__head">
        <span className="workspace__eyebrow">Local Arena</span>
        <h1>{t("nav.overview")}</h1>
      </header>
      <StatusBar onOpenSettings={() => onNavigate("settings")} />

      {updateAvailable && (
        <button className="update-banner" onClick={() => onNavigate("settings")}>
          <span className="update-banner__icon" aria-hidden="true"><Download size={17} /></span>
          <strong>{t("update.available")}</strong>
          <small>{updates.release_version ? `v${updates.release_version}` : ""}</small>
          <span className="update-banner__action">
            {t("set.updates")}
            <ArrowRight size={14} />
          </span>
        </button>
      )}

      <div className="dashboard__controls">
        <ModeCard />
        <DifficultyCard />
      </div>

      {latest ? (
        <button className="recent-match glass" onClick={() => onNavigate("stats")}>
          <span className="recent-match__map" aria-hidden="true">
            {MAP_IMAGES[latest.map] && <img src={MAP_IMAGES[latest.map]} alt="" />}
            <i>{cs2ssMapLabel(latest.map)}</i>
          </span>
          <span className="recent-match__body">
            <small>{t("match.recentMatches")}</small>
            <strong>
              {latest.teamAScore} : {latest.teamBScore}
              <em> · {t("stats.matchNumber", { id: latest.matchId })}</em>
            </strong>
            <span>{new Date(latest.startedAt).toLocaleString()}</span>
          </span>
          <span className={`recent-match__state is-${latest.result === "W" ? "finished" : "interrupted"}`}>
            {latest.result ?? t("match.finished")}
          </span>
        </button>
      ) : (
        <button className="recent-match recent-match--empty" onClick={() => onNavigate("match")}>
          <span className="recent-match__empty-icon" aria-hidden="true"><Play size={17} /></span>
          <span className="recent-match__body">
            <small>{t("match.recentMatches")}</small>
            <strong>{t("overview.startFirstMatch")}</strong>
            <span>{t("stats.noMatches")}</span>
          </span>
          <ArrowRight size={16} className="recent-match__chev" />
        </button>
      )}

      <div className="quick-grid" role="navigation" aria-label={t("overview.quickActions")}>
        {TILES.map(({ view, key, icon: Icon }) => (
          <button key={view} className="quick-tile" onClick={() => onNavigate(view)}>
            <Icon size={18} strokeWidth={1.9} aria-hidden="true" />
            <span>{t(key)}</span>
          </button>
        ))}
      </div>

      <button className="tutorial-card glass" onClick={() => onNavigate("guide")}>
        <span className="tutorial-card__icon" aria-hidden="true">
          <BookOpenText size={22} strokeWidth={1.8} />
        </span>
        <span className="tutorial-card__body">
          <small>{t("overview.guideEyebrow")}</small>
          <strong>{t("overview.guideTitle")}</strong>
          <span>{t("overview.guideDesc")}</span>
        </span>
        <span className="tutorial-card__action">
          {t("overview.guideAction")}
          <ArrowRight size={16} strokeWidth={1.9} />
        </span>
      </button>
    </div>
  );
}
