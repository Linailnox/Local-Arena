import { useEffect, useState, useMemo } from "react";
import { CartesianGrid, Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { api } from "../lib/api";
import type { Cs2ssMatchDetailResponse, Cs2ssRoundPlayer } from "../data/cs2ssTypes";
import { cs2ssCalcRating, cs2ssCalcAdr, cs2ssCalcKast } from "../data/cs2ssRating";
import { cs2ssMapLabel } from "../data/cs2ssMaps";
import { cs2ssRoundEndReasonLabel } from "../data/cs2ssReasons";
import { useStore } from "../state/store";
import StatsDMDetail from "./StatsDMDetail";
import "./StatsPanel.css";

const CH = ["#7c5cff", "#20b486", "#ff9f43", "#e05d75", "#3f8efc", "#00a8a8", "#bf6bdb", "#d87b35"];

interface Props { csgo: string; matchId: number; onBack: () => void; }

function rcol(r: number) { return r >= 1.1 ? "#20b486" : r >= 0.9 ? "#e67e22" : "#e05d75"; }
function fmtT(iso: string) { try { const d = new Date(iso); return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`; } catch { return iso; } }

function badges(p: Cs2ssRoundPlayer) {
  const bs: { l: string; t: string }[] = [];
  if (p.multikill >= 3) bs.push({ l: p.multikill >= 5 ? "ACE" : `${p.multikill}K`, t: "kill" });
  if (p.tradeKills > 0) bs.push({ l: `Trade x${p.tradeKills}`, t: "trade" });
  if (p.traded) bs.push({ l: "Traded", t: "support" });
  if (p.clutchAttempt) bs.push({ l: `1v${p.clutchSize} ${p.clutchWon ? "Won" : "Lost"}`, t: p.clutchWon ? "clutchWin" : "clutch" });
  return bs;
}

export default function StatsMatchDetail({ csgo, matchId, onBack }: Props) {
  const { reportError } = useStore();
  const [data, setData] = useState<Cs2ssMatchDetailResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState("");
  const [sel, setSel] = useState<Set<string>>(new Set());

  useEffect(() => {
    api.getCs2ssMatchDetail(csgo, matchId).then(d => {
      if (d) { setData(d); const self = d.matchPlayers.find(mp => !mp.isBot) ?? d.matchPlayers[0]; const initTeamOf = (sid: string) => d.roundPlayers.find(rp => rp.steamId === sid)?.team ?? d.matchPlayers.find(p => p.steamId === sid)?.team; const selfInit = initTeamOf(self.steamId); const ns = new Set<string>(); ns.add(self.steamId); const topEnemy = d.matchPlayers.filter(p => initTeamOf(p.steamId) !== selfInit).sort((a, b) => (b.totalKills + b.totalAssists) - (a.totalKills + a.totalAssists))[0]; if (topEnemy) ns.add(topEnemy.steamId); setSel(ns); }
      setLoading(false);
    }).catch(e => { setErr(String(e)); setLoading(false); reportError(e); });
  }, [matchId, reportError]);

  const c = useMemo(() => {
    if (!data) return null;
    const { match, matchPlayers: mps, roundPlayers: rps, rounds: rs } = data;
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

    const hl = rps.filter(p => badges(p).length > 0).sort((a, b) => a.roundNumber - b.roundNumber);

    let rp = 0, ro = 0;
    const tl = rs.map(r => {
      const pr = rps.find(rp => rp.roundNumber === r.roundNumber && rp.steamId === s.steamId);
      const w = pr?.team === r.winnerTeam;
      if (hf) { rp = myTeam === "CT" ? r.teamAScore : r.teamBScore; ro = myTeam === "CT" ? r.teamBScore : r.teamAScore; }
      else { if (w) rp++; else ro++; }
      return { r, pr, w, ps: rp, os: ro };
    });

    return { match, s, pw, ow, rows, hl, tl, myTeam };
  }, [data]);

  if (loading) return <div className="stats-panel"><div className="stats-panel__loading">Loading…</div></div>;
  if (err) return <div className="stats-panel"><div className="stats-panel__error">{err}</div></div>;
  if (!c) return <div className="stats-panel"><div className="stats-panel__empty">No data</div></div>;

  const { match, s, pw, ow, rows, hl, tl } = c;
  if (match.modeFamily === "deathmatch") return <StatsDMDetail data={data!} steamId={s.steamId} onBack={onBack} />;

  const won = pw > ow;
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
        <div style={{ display: "grid", gridTemplateColumns: "minmax(100px, 1.5fr) repeat(7, 1fr)", alignItems: "center", borderBottom: "1px solid var(--line)", cursor: "default", background: "rgba(0,0,0,0.02)" }}>
          <div style={{ fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 10px" }}>玩家</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>K-D</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>ADR</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>KAST</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>补枪</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>多杀</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 4px" }}>残局</div>
          <div style={{ textAlign: "center", fontWeight: 600, fontSize: 10, color: "var(--text-tertiary)", textTransform: "uppercase", letterSpacing: ".05em", padding: "6px 6px" }}>Rating</div>
        </div>
        {players.map(({ mp, r, adr, kast }) => {
          const iss = mp.steamId === s.steamId;
          const dots = mp.multikill2 + mp.multikill3 + mp.multikill4 + mp.multikill5;
          return (
            <div key={mp.steamId} style={{ display: "grid", gridTemplateColumns: "minmax(100px, 1.5fr) repeat(7, 1fr)", alignItems: "center", borderBottom: "1px solid var(--line)", cursor: "pointer", background: iss ? "rgba(124,92,255,.055)" : undefined, fontSize: 13 }} onClick={() => toggle(mp.steamId)}>
              <div style={{ padding: "7px 10px", fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                <span className={`stats-team-row__dot${sel.has(mp.steamId) ? " sel" : ""}`} style={{ marginRight: 6 }} />{mp.name}
              </div>
              <div style={{ textAlign: "center", fontWeight: 700, padding: "7px 4px" }}>{mp.totalKills}<span style={{ color: "var(--text-secondary)", fontWeight: 400 }}>/{mp.totalDeaths}</span></div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: "var(--text-secondary)" }}>{adr.toFixed(0)}</div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: kast >= 75 ? "#20b486" : "var(--text-secondary)" }}>{kast.toFixed(0)}%</div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: "var(--text-secondary)" }}>{mp.tradeKills || "—"}</div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: "var(--text-secondary)" }}>{dots || "—"}</div>
              <div style={{ textAlign: "center", padding: "7px 4px", color: "var(--text-secondary)" }}>{mp.clutchesWon}/{mp.clutchAttempts}</div>
              <div style={{ textAlign: "center", fontWeight: 800, padding: "7px 6px", color: rcol(r) }}>{r.toFixed(2)}</div>
            </div>
          );
        })}
      </div>
    </div>
  );

  return (
    <div className="stats-panel">
      <button className="stats-back" onClick={onBack}>← Back</button>

      <div className="stats-hero" style={{ background: won ? "linear-gradient(125deg, #102a25, #175b4c 58%, #20a27e)" : "linear-gradient(125deg, #2b1820, #6e2938 58%, #b7495e)" }}>
        <div>
          <span className="stats-hero__eyebrow">MATCH #{match.matchId} · {fmtT(match.startedAt)}</span>
          <h1>{cs2ssMapLabel(match.map)}</h1>
          <p style={{ color: "rgba(255,255,255,.68)", fontSize: 13 }}>{match.roundsPlayed} 回合 · {Math.round(match.durationSeconds / 60)} 分钟</p>
        </div>
        <div className="stats-hero__rating">
          <small>{won ? "VICTORY" : "DEFEAT"}</small>
          <strong style={{ color: won ? "#20b486" : "#e05d75" }}>{pw}:{ow}</strong>
        </div>
      </div>

      <div className="stats-snapshot">
        <div className="stats-snapshot__lead">
          <span>你的贡献</span>
          <strong style={{ color: "#fff" }}>{myR.toFixed(2)}</strong>
          <small>Rating 2.0</small>
        </div>
        {[
          ["K/D/A", `${s.totalKills}/${s.totalDeaths}/${s.totalAssists}`],
          ["ADR", cs2ssCalcAdr(s.totalDamage, match.roundsPlayed).toFixed(1)],
          ["KAST", `${cs2ssCalcKast(s.kastRounds, match.roundsPlayed).toFixed(1)}%`],
          ["补枪", String(s.tradeKills)],
          ["残局", `${s.clutchesWon}/${s.clutchAttempts}`],
        ].map(([l, v]: string[]) => (
          <div key={l}><span>{l}</span><b>{v}</b></div>
        ))}
      </div>

      <div>
        <div className="stats-panel-block__title" style={{ marginBottom: 12 }}>
          <div><span>计分板</span><h2>玩家表现</h2></div>
          <p>点击可切换伤害曲线</p>
        </div>
        {renderTeam("我方", myRows, pw)}
        {renderTeam("敌方", en, ow)}
      </div>

      <div className="stats-charts">
        <div className="stats-panel-block">
          <div className="stats-panel-block__title"><div><span>高光时刻</span><h2>本场高光</h2></div></div>
          <div className="stats-highlights">
            {hl.length > 0 ? hl.map((p, i) => (
              <div className="stats-highlight" key={`${p.roundPlayerId}-${i}`}>
                <span className="stats-highlight__r">R{p.roundNumber + 1}</span>
                <div><span className="stats-highlight__name">{p.name}</span> <span className="stats-highlight__team">{p.team}</span></div>
                <div className="stats-badges">{badges(p).map(b => <span key={b.l} className={`stats-badge ${b.t}`}>{b.l}</span>)}</div>
              </div>
            )) : <p style={{ color: "var(--text-secondary)", textAlign: "center" }}>无高光事件</p>}
          </div>
        </div>

        <div className="stats-panel-block">
          <div className="stats-panel-block__title"><div><span>伤害流动</span><h2>逐回合伤害</h2></div></div>
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
        <div className="stats-panel-block__title"><div><span>回合日志</span><h2>回合时间线</h2></div></div>
        <div className="stats-round-grid">
          {tl.map(({ r, pr, w, ps, os }) => (
            <div key={r.roundId} className={`stats-round-card ${w ? "w" : "l"}`}>
              <div className="stats-round-card__top"><b>R{r.roundNumber + 1}</b><span>{ps}:{os}</span></div>
              <div className="stats-round-card__result">{w ? "WON" : "LOST"} <span className={pr?.team === "CT" ? "ct" : "t"} style={{ float: "right" }}>{pr?.team}</span></div>
              <p>{cs2ssRoundEndReasonLabel(r.endReason)}</p>
              <div className="stats-round-card__stats"><span>{pr?.kills ?? 0}K</span><span>{pr?.damage ?? 0} DMG</span><span>{pr?.survived ? "SURVIVED" : `${pr?.deaths ?? 0}D`}</span></div>
              {pr && badges(pr).length > 0 && <div className="stats-badges">{badges(pr).map(b => <span key={b.l} className={`stats-badge ${b.t}`}>{b.l}</span>)}</div>}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}