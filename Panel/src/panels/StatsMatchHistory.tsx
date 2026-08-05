import { useState, useEffect, useMemo } from "react";
import { api } from "../lib/api";
import type { Cs2ssMatchWithStats } from "../data/cs2ssTypes";
import { cs2ssCalcRating, cs2ssCalcAdr } from "../data/cs2ssRating";
import { cs2ssMapLabel } from "../data/cs2ssMaps";
import { useStore } from "../state/store";
import { useT } from "../i18n";
import "./StatsPanel.css";

interface Props { csgo: string; onOpenMatch?: (id: number) => void; onBack?: () => void; }

function fmtDT(iso: string) { try { const d = new Date(iso); return `${d.getMonth() + 1}/${d.getDate()} ${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`; } catch { return iso; } }
function rcol(r: number) { return r >= 1.1 ? "#20b486" : r >= 0.9 ? "#e67e22" : "#e05d75"; }

export default function StatsMatchHistory({ csgo, onOpenMatch, onBack }: Props) {
  const { reportError } = useStore();
  const t = useT();
  const [matches, setMatches] = useState<Cs2ssMatchWithStats[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState("");
  const [mapF, setMapF] = useState("");
  const [modeF, setModeF] = useState("all");
  const [dateF, setDateF] = useState("");
  const [dateT, setDateT] = useState("");

  useEffect(() => {
    api.listCs2ssMatchesWithStats(csgo).then(ms => { setMatches(ms ?? []); setLoading(false); }).catch(e => { setErr(String(e)); setLoading(false); reportError(e); });
  }, [csgo, reportError]);

  const maps = useMemo(() => [...new Set(matches.map(m => m.map))].sort(), [matches]);
  const filtered = useMemo(() => {
    let r = matches;
    if (mapF) r = r.filter(m => m.map === mapF);
    if (modeF !== "all") r = r.filter(m => m.modeFamily === modeF);
    if (dateF) { const t = new Date(dateF).getTime(); r = r.filter(m => new Date(m.startedAt).getTime() >= t); }
    if (dateT) { const t = new Date(dateT + "T23:59:59").getTime(); r = r.filter(m => new Date(m.startedAt).getTime() <= t); }
    return r;
  }, [matches, mapF, modeF, dateF, dateT]);

  if (loading) return <div className="stats-panel"><div className="stats-panel__loading">{t("stats.loading")}</div></div>;
  if (err) return <div className="stats-panel"><div className="stats-panel__error">{err}</div></div>;

  return (
    <div className="stats-panel">
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 22 }}>
        {onBack && <button className="stats-back" onClick={onBack}>← {t("stats.back")}</button>}
        <span style={{ fontSize: 13, color: "var(--text-secondary)", fontWeight: 600 }}>{t("stats.matchesShown", { shown: filtered.length, total: matches.length })}</span>
      </div>

      <div className="stats-filters">
        <label><span>{t("stats.map")}</span><select value={mapF} onChange={e => setMapF(e.target.value)}><option value="">{t("stats.all")}</option>{maps.map(m => <option key={m} value={m}>{cs2ssMapLabel(m)}</option>)}</select></label>
        <label><span>{t("stats.mode")}</span><select value={modeF} onChange={e => setModeF(e.target.value)}><option value="all">{t("stats.all")}</option><option value="competitive">{t("stats.competitive")}</option><option value="deathmatch">{t("stats.deathmatch")}</option></select></label>
        <label><span>{t("stats.from")}</span><input type="date" value={dateF} onChange={e => setDateF(e.target.value)} /></label>
        <label><span>{t("stats.to")}</span><input type="date" value={dateT} onChange={e => setDateT(e.target.value)} /></label>
        <button onClick={() => { setMapF(""); setModeF("all"); setDateF(""); setDateT(""); }}>{t("stats.reset")}</button>
      </div>

      <div className="stats-panel-block" style={{ padding: 0 }}>
        <table className="stats-table">
          <thead><tr><th>{t("stats.map")}</th><th>{t("stats.date")}</th><th>{t("stats.score")}</th><th>{t("stats.rounds")}</th><th style={{ textAlign: "right" }}>K/D/A</th><th style={{ textAlign: "right" }}>ADR</th><th style={{ textAlign: "right" }}>{t("stats.rating")}</th></tr></thead>
          <tbody>
            {filtered.length === 0 ? (
              <tr><td colSpan={7} style={{ textAlign: "center", padding: 40, color: "var(--text-secondary)" }}>{t("stats.noFilteredMatches")}</td></tr>
            ) : filtered.map(m => {
              const dm = m.modeFamily === "deathmatch";
              const it = m.playerInitialTeam || m.playerTeam;
              const pw = it === "CT" ? m.teamAScore : m.teamBScore;
              const ow = it === "CT" ? m.teamBScore : m.teamAScore;
              const won = !dm && pw > ow; const lost = !dm && pw < ow;

              // Competitive stats
              const rating = !dm && m.roundsPlayed > 0
                ? cs2ssCalcRating(m.playerKills, m.playerDeaths, m.playerAssists, m.playerDamage, m.playerHeadshots, m.roundsPlayed, {
                    kastRounds: m.playerKastRounds, tradeKills: m.playerTradeKills,
                    multikill2: m.playerMk2, multikill3: m.playerMk3, multikill4: m.playerMk4, multikill5: m.playerMk5,
                    clutchAttempts: m.playerClutchAttempts, clutchesWon: m.playerClutchesWon,
                  })
                : 0;
              const adr = !dm ? cs2ssCalcAdr(m.playerDamage, m.roundsPlayed) : 0;

              // DM stats
              const durMin = Math.max(1, m.durationSeconds / 60);
              const dpm = dm ? Math.round(m.playerDamage / durMin) : 0;
              const kpm = dm ? Math.round(m.playerKills / durMin * 100) / 100 : 0;

              return (
                <tr key={m.matchId} onClick={() => onOpenMatch?.(m.matchId)} style={{ cursor: "pointer" }}>
                  <td style={{ fontWeight: 600 }}>{cs2ssMapLabel(m.map)}{dm && <span className="dm-tag">DM</span>}</td>
                  <td style={{ color: "var(--text-secondary)", fontSize: 12, whiteSpace: "nowrap" }}>{fmtDT(m.startedAt)}</td>
                  <td>
                    {dm ? (
                      <span style={{ color: "#df6b35", fontWeight: 700 }}>{t("stats.minutesShort", { count: Math.round(m.durationSeconds / 60) })}</span>
                    ) : (
                      <><span style={{ color: "var(--st-green)", fontWeight: 600 }}>{pw}</span><span style={{ color: "var(--text-secondary)" }}> : </span><span style={{ color: "var(--st-red)", fontWeight: 600 }}>{ow}</span>
                        <span style={{ marginLeft: 8, fontWeight: 700, fontSize: 12, color: won ? "var(--st-green)" : lost ? "var(--st-red)" : "var(--text-secondary)" }}>{won ? "W" : lost ? "L" : "D"}</span></>
                    )}
                  </td>
                  <td>{dm ? t("stats.minutesShort", { count: Math.round(m.durationSeconds / 60) }) : t("stats.roundsShort", { count: m.roundsPlayed })}</td>
                  <td style={{ textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                    {dm ? `${m.playerKills}/${m.playerDeaths}/${m.playerAssists}` : `${m.playerKills}/${m.playerDeaths}/${m.playerAssists}`}
                  </td>
                  <td style={{ textAlign: "right", fontVariantNumeric: "tabular-nums", color: "var(--text-secondary)" }}>
                    {dm ? `${dpm} DPM` : adr.toFixed(1)}
                  </td>
                  <td style={{ textAlign: "right", fontWeight: 700, fontVariantNumeric: "tabular-nums", color: dm ? "#df6b35" : rcol(rating) }}>
                    {dm ? `${kpm} KPM` : rating.toFixed(2)}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
