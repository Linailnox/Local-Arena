import { useEffect, useState, useMemo } from "react";
import { CartesianGrid, Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { api } from "../lib/api";
import type { Cs2ssMatchDetailResponse, Cs2ssRoundPlayer } from "../data/cs2ssTypes";
import { cs2ssCalcRating, cs2ssCalcAdr, cs2ssCalcKast, cs2ssCalcHsPct, cs2ssRatingBreakdown } from "../data/cs2ssRating";
import { cs2ssMapLabel } from "../data/cs2ssMaps";
import { cs2ssRoundEndReasonLabel } from "../data/cs2ssReasons";
import { useStore } from "../state/store";
import { useT } from "../i18n";
import StatsDMDetail from "./StatsDMDetail";
import "./StatsPanel.css";

const CH = ["#7c5cff", "#20b486", "#ff9f43", "#e05d75", "#3f8efc", "#00a8a8", "#bf6bdb", "#d87b35"];

interface Props { csgo: string; matchId: number; onBack: () => void; }

function rcol(r: number) { return r >= 1.1 ? "#20b486" : r >= 0.9 ? "#e67e22" : "#e05d75"; }
function fmtT(iso: string) { try { const d = new Date(iso); return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`; } catch { return iso; } }

function badges(p: Cs2ssRoundPlayer, t: ReturnType<typeof useT>) {
  const bs: { l: string; t: string }[] = [];
  if (p.multikill >= 3) bs.push({ l: p.multikill >= 5 ? "ACE" : `${p.multikill}K`, t: "kill" });
  if (p.tradeKills > 0) bs.push({ l: t("stats.trade", { count: p.tradeKills }), t: "trade" });
  if (p.traded) bs.push({ l: t("stats.traded"), t: "support" });
  if (p.clutchAttempt) bs.push({ l: t(p.clutchWon ? "stats.clutchWon" : "stats.clutchLost", { size: p.clutchSize }), t: p.clutchWon ? "clutchWin" : "clutch" });
  return bs;
}

export default function StatsMatchDetail({ csgo, matchId, onBack }: Props) {
  const { reportError } = useStore();
  const t = useT();
  const [data, setData] = useState<Cs2ssMatchDetailResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState("");
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      setLoading(true);
      api.getCs2ssMatchDetail(csgo, matchId).then(d => {
        if (cancelled || !d) return;
        setData(d);
        const visiblePlayers = d.matchPlayers.filter(mp => !mp.isBot || mp.totalKills > 0 || mp.totalDeaths > 0 || mp.totalDamage > 0);
        if (visiblePlayers.length > 0) {
          const self = visiblePlayers.find(mp => !mp.isBot) ?? visiblePlayers[0];
          const initTeamOf = (sid: string) => d.roundPlayers.find(rp => rp.steamId === sid)?.team ?? d.matchPlayers.find(p => p.steamId === sid)?.team;
          const selfInit = initTeamOf(self.steamId);
          const ns = new Set<string>(); ns.add(self.steamId);
          const topEnemy = visiblePlayers.filter(p => initTeamOf(p.steamId) !== selfInit).sort((a, b) => (b.totalKills + b.totalAssists) - (a.totalKills + a.totalAssists))[0];
          if (topEnemy) ns.add(topEnemy.steamId);
          setSel(ns);
        }
        setLoading(false);
      }).catch(e => { if (!cancelled) { setErr(String(e)); setLoading(false); reportError(e); } });
    };
    load();
    return () => { cancelled = true; };
  }, [csgo, matchId, reportError]);

  const c = useMemo(() => {
    if (!data) return null;
    const { match, matchPlayers: mpsRaw, roundPlayers: rps, rounds: rs } = data;

    const mps = mpsRaw.filter(mp => !mp.isBot || mp.totalKills > 0 || mp.totalDeaths > 0 || mp.totalDamage > 0);
    if (mps.length === 0) return { empty: true, match, status: match.status } as const;

    const s = mps.find(p => !p.isBot) ?? mps[0];
    const myTeam = rps.find(rp => rp.steamId === s.steamId)?.team ?? s.team;
    const hf = match.teamAScore + match.teamBScore === match.roundsPlayed;
    const pw = hf ? (myTeam === "CT" ? match.teamAScore : match.teamBScore) : rs.filter(r => rps.find(rp => rp.roundNumber === r.roundNumber && rp.steamId === s.steamId)?.team === r.winnerTeam).length;
    const ow = hf ? (myTeam === "CT" ? match.teamBScore : match.teamAScore) : match.roundsPlayed - pw;

    const rows = mps.map(mp => {
      const r = cs2ssCalcRating(mp.totalKills, mp.totalDeaths, mp.totalAssists, mp.totalDamage, mp.totalHeadshotKills, match.roundsPlayed, { kastRounds: mp.kastRounds, tradeKills: mp.tradeKills, multikill2: mp.multikill2, multikill3: mp.multikill3, multikill4: mp.multikill4, multikill5: mp.multikill5, clutchAttempts: mp.clutchAttempts, clutchesWon: mp.clutchesWon });
      const mpInitTeam = rps.find(rp => rp.steamId === mp.steamId)?.team ?? mp.team;
      return { mp, side: mpInitTeam === myTeam ? "mine" : "enemy", r, adr: cs2ssCalcAdr(mp.totalDamage, match.roundsPlayed), kast: cs2ssCalcKast(mp.kastRounds, match.roundsPlayed) };
    }).sort((a, b) => b.r - a.r);

    const hl = rps.filter(p => badges(p, t).length > 0).sort((a, b) => a.roundNumber - b.roundNumber);

    const tl = rs.map(r => {
      const pr = rps.find(rp => rp.roundNumber === r.roundNumber && rp.steamId === s.steamId);
      return { r, pr };
    });

    return { match, s, pw, ow, rows, hl, tl, myTeam };
  }, [data, t]);

  if (loading) return <div className="stats-panel"><div className="stats-panel__loading">{t("stats.loading")}</div></div>;
  if (err) return <div className="stats-panel"><div className="stats-panel__error">{err}</div></div>;
  if (!c) return <div className="stats-panel"><div className="stats-panel__empty">{t("stats.noData")}</div></div>;
  if ("empty" in c) return (
    <div className="stats-panel">
      <button className="stats-back" onClick={onBack}>← {t("stats.back")}</button>
      <div className="stats-panel__empty" style={{ marginTop: 24, textAlign: "center" }}>
        <h2 style={{ color: "var(--text-secondary)" }}>{t("stats.matchNumber", { id: c.match.matchId })}</h2>
        <p style={{ color: "var(--text-tertiary)", marginTop: 8 }}>
          {c.status === "in_progress" ? t("stats.inProgressNoPlayers") :
           c.status === "abandoned" ? t("stats.abandonedNoPlayers") :
           t("stats.noPlayers")}
        </p>
      </div>
    </div>
  );

  const { match, s, pw, ow, rows, hl, tl } = c;
  if (match.modeFamily === "deathmatch") return <StatsDMDetail data={data!} steamId={s.steamId} onBack={onBack} />;

  const myRows = rows.filter(r => r.side === "mine");
  const en = rows.filter(r => r.side === "enemy");
  const sArr = [...sel].slice(0, 8);
  const myR = rows.find(r => r.mp.steamId === s.steamId)?.r ?? 0;

  const dmgData = tl.map(({ r }) => {
    const row: Record<string, any> = { round: `R${r.roundNumber + 1}` };
    sArr.forEach(sid => { const rp = data!.roundPlayers.find(p => p.roundNumber === r.roundNumber && p.steamId === sid); row[sid] = rp?.damage ?? 0; });
    return row;
  });

  const toggle = (sid: string) => setSel(s => { const n = new Set(s); if (n.has(sid)) n.delete(sid); else n.add(sid); return n; });

  const renderTeam = (label: string, players: typeof myRows, score: number) => (
    <div style={{ marginBottom: 24 }}>
      <div className="stats-team-block__head">{label} <span>{score}</span></div>
      <div style={{ border: "1px solid var(--line)", borderRadius: 11, overflow: "hidden" }}>
        <div style={{ display: "grid", gridTemplateColumns: "minmax(100px, 1.5fr) repeat(8, 1fr)", alignItems: "center", borderBottom: "1px solid var(--line)", cursor: "default", background: "rgba(0,0,0,0.02)" }}>
          <div style={{ fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 10px" }}>{t("stats.player")}</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>K-D</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>ADR</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>KAST</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>{t("stats.hsPct")}</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>{t("stats.tradeKills")}</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>{t("stats.multikills")}</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>{t("stats.clutches")}</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 6px" }}>{t("stats.rating")}</div>
        </div>
        {players.map(({ mp, r, adr, kast }) => {
          const iss = mp.steamId === s.steamId;
          const dots = mp.multikill2 + mp.multikill3 + mp.multikill4 + mp.multikill5;
          const hsPct = cs2ssCalcHsPct(mp.totalHeadshotKills, mp.totalKills);
          const breakdown = cs2ssRatingBreakdown(mp.totalKills, mp.totalDeaths, mp.totalAssists, mp.totalDamage, match.roundsPlayed, { kastRounds: mp.kastRounds, tradeKills: mp.tradeKills, multikill2: mp.multikill2, multikill3: mp.multikill3, multikill4: mp.multikill4, multikill5: mp.multikill5, clutchAttempts: mp.clutchAttempts, clutchesWon: mp.clutchesWon });
          const peak = Math.max(0.01, ...breakdown.map(b => Math.abs(b.value)));
          const isOpen = expanded === mp.steamId;
          return (
            <div key={mp.steamId}>
            <div style={{ display: "grid", gridTemplateColumns: "minmax(100px, 1.5fr) repeat(8, 1fr)", alignItems: "center", borderBottom: "1px solid var(--line)", cursor: "pointer", background: iss ? "rgba(124,92,255,.055)" : undefined, fontSize: 13 }} onClick={() => { toggle(mp.steamId); setExpanded(isOpen ? null : mp.steamId); }}>
              <div style={{ padding: "7px 10px", fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                <span className={`stats-team-row__dot${sel.has(mp.steamId) ? " sel" : ""}`} style={{ marginRight: 6 }} />{mp.name}
              </div>
              <div style={{ textAlign: "center", fontWeight: 700, padding: "7px 4px" }}>{mp.totalKills}<span style={{ color: "var(--text-secondary)", fontWeight: 400 }}>/{mp.totalDeaths}</span></div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: "var(--text-secondary)" }}>{adr.toFixed(0)}</div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: kast >= 75 ? "#20b486" : "var(--text-secondary)" }}>{kast.toFixed(0)}%</div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: hsPct >= 40 ? "#20b486" : "var(--text-secondary)" }}>{hsPct}%</div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: "var(--text-secondary)" }}>{mp.tradeKills || "—"}</div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: "var(--text-secondary)" }}>{dots || "—"}</div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: "var(--text-secondary)" }}>{mp.clutchesWon}/{mp.clutchAttempts}</div>
              <div style={{ textAlign: "center", fontWeight: 800, padding: "7px 6px", color: rcol(r) }}>{r.toFixed(2)}</div>
            </div>
            {isOpen && (
              <div className="mr-detail" onClick={e => e.stopPropagation()} style={{ borderBottom: "1px solid var(--line)", padding: "10px 12px" }}>
                <div className="mr-detail__bars">
                  <small className="mr-detail__bars-title">{t("match.breakdown")}</small>
                  {breakdown.map(row => {
                    const tier = row.value >= 0.1 ? "is-high" : row.value <= -0.02 ? "is-low" : "is-mid";
                    return (
                      <span className="mr-bar" key={row.label}>
                        <small>{row.label}</small>
                        <span className="mr-bar__track">
                          <i className={tier} style={{ width: `${Math.max(4, (Math.abs(row.value) / peak) * 100)}%` }} />
                        </span>
                        <b className={tier}>{row.value >= 0 ? "+" : ""}{row.value.toFixed(2)}</b>
                      </span>
                    );
                  })}
                </div>
              </div>
            )}
            </div>
          );
        })}
      </div>
    </div>
  );

  return (
    <div className="stats-panel">
      <button className="stats-back" onClick={onBack}>← {t("stats.back")}</button>

      <div className="stats-hero">
        <div>
          <span className="stats-hero__eyebrow">{t("stats.matchNumber", { id: match.matchId })} · {fmtT(match.startedAt)}</span>
          <h1>{cs2ssMapLabel(match.map)}</h1>
          <p style={{ color: "rgba(255,255,255,.68)", fontSize: 13 }}>{t("stats.roundsShort", { count: match.roundsPlayed })} · {t("stats.minutesShort", { count: Math.round(match.durationSeconds / 60) })}</p>
        </div>
        <div className="stats-hero__rating">
          <small>{t("match.finished")}</small>
          <strong style={{ color: "#fff" }}>{pw}:{ow}</strong>
        </div>
      </div>

      <div className="stats-snapshot">
        <div className="stats-snapshot__lead">
          <span>{t("stats.yourContribution")}</span>
          <strong style={{ color: "#fff" }}>{myR.toFixed(2)}</strong>
          <small>Rating 2.0</small>
        </div>
        {[
          ["K/D/A", `${s.totalKills}/${s.totalDeaths}/${s.totalAssists}`],
          ["ADR", cs2ssCalcAdr(s.totalDamage, match.roundsPlayed).toFixed(1)],
          ["KAST", `${cs2ssCalcKast(s.kastRounds, match.roundsPlayed).toFixed(1)}%`],
          [t("stats.tradeKills"), String(s.tradeKills)],
          [t("stats.clutches"), `${s.clutchesWon}/${s.clutchAttempts}`],
        ].map(([l, v]: string[]) => (
          <div key={l}><span>{l}</span><b>{v}</b></div>
        ))}
      </div>

      <div>
        <div className="stats-panel-block__title" style={{ marginBottom: 12 }}>
          <div><span>{t("stats.scoreboard")}</span><h2>{t("stats.playerPerformance")}</h2></div>
          <p>{t("stats.toggleDamage")}</p>
        </div>
        {renderTeam(t("stats.ourTeam"), myRows, pw)}
        {renderTeam(t("stats.enemyTeam"), en, ow)}
      </div>

      <div className="stats-charts">
        <div className="stats-panel-block">
          <div className="stats-panel-block__title"><div><span>{t("stats.highlights")}</span><h2>{t("stats.matchHighlights")}</h2></div></div>
          <div className="stats-highlights">
            {hl.length > 0 ? hl.map((p, i) => (
              <div className="stats-highlight" key={`${p.roundPlayerId}-${i}`}>
                <span className="stats-highlight__r">R{p.roundNumber + 1}</span>
                <div><span className="stats-highlight__name">{p.name}</span> <span className="stats-highlight__team">{p.team}</span></div>
                <div className="stats-badges">{badges(p, t).map(b => <span key={b.l} className={`stats-badge ${b.t}`}>{b.l}</span>)}</div>
              </div>
            )) : <p style={{ color: "var(--text-secondary)", textAlign: "center" }}>{t("stats.noHighlights")}</p>}
          </div>
        </div>

        <div className="stats-panel-block">
          <div className="stats-panel-block__title"><div><span>{t("stats.damageFlow")}</span><h2>{t("stats.roundDamage")}</h2></div></div>
          <ResponsiveContainer width="100%" height={240}>
            <LineChart data={dmgData}>
              <CartesianGrid strokeDasharray="3 3" />
              <XAxis dataKey="round" tick={{ fontSize: 11 }} />
              <YAxis tick={{ fontSize: 11 }} />
              <Tooltip />
              <Legend wrapperStyle={{ fontSize: 11 }} />
              {sArr.map((sid, i) => { const mp = data!.matchPlayers.find(p => p.steamId === sid); return <Line key={sid} type="monotone" dataKey={sid} name={mp?.name ?? sid} stroke={CH[i % CH.length]} strokeWidth={2.2} dot={false} />; })}
            </LineChart>
          </ResponsiveContainer>
        </div>
      </div>

      <div className="stats-panel-block">
        <div className="stats-panel-block__title"><div><span>{t("stats.roundLog")}</span><h2>{t("stats.roundTimeline")}</h2></div></div>
        <div className="stats-round-grid">
          {tl.map(({ r, pr }) => (
            <div key={r.roundId} className="stats-round-card">
              <div className="stats-round-card__top"><b>R{r.roundNumber + 1}</b><span>{r.teamAScore}:{r.teamBScore}</span></div>
              <div className="stats-round-card__result"><span className={r.winnerTeam === "CT" ? "ct" : "t"}>{r.winnerTeam}</span></div>
              <p>{cs2ssRoundEndReasonLabel(r.endReason)}</p>
              <div className="stats-round-card__stats"><span>{pr?.kills ?? 0}K</span><span>{pr?.damage ?? 0} DMG</span><span>{pr?.survived ? t("stats.survived") : `${pr?.deaths ?? 0}D`}</span></div>
              {pr && badges(pr, t).length > 0 && <div className="stats-badges">{badges(pr, t).map(b => <span key={b.l} className={`stats-badge ${b.t}`}>{b.l}</span>)}</div>}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
