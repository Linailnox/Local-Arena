import { useEffect, useState } from "react";
import { CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis, Bar, BarChart, Cell } from "recharts";
import { api } from "../lib/api";
import type { Cs2ssOverviewResponse, Cs2ssMatchSummary, Cs2ssPlayerDetailResponse } from "../data/cs2ssTypes";
import { cs2ssCalcRating, cs2ssCalcAdr, cs2ssCalcKast } from "../data/cs2ssRating";
import { cs2ssMapLabel } from "../data/cs2ssMaps";
import StatsMatchHistory from "./StatsMatchHistory";
import StatsMatchDetail from "./StatsMatchDetail";
import { useStore } from "../state/store";
import "./StatsPanel.css";

type SubView = "dashboard" | "history" | "matchDetail";
const HI = "#20b486", MID = "#e67e22", LO = "#e05d75";
function rcol(r: number) { return r >= 1.1 ? HI : r >= 0.9 ? MID : LO; }
function fmtD(iso: string) { try { const d = new Date(iso); return `${d.getMonth() + 1}/${d.getDate()} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`; } catch { return iso; } }

export default function StatsDashboard() {
  const { reportError, directory } = useStore();
  const csgo = directory?.valid ? directory.selected ?? "" : "";
  const [sub, setSub] = useState<SubView>("dashboard");
  const [selMatch, setSelMatch] = useState(0);
  const [data, setData] = useState<Cs2ssOverviewResponse | null>(null);
  const [matches, setMatches] = useState<Cs2ssMatchSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState("");
  const [mode, setMode] = useState<"competitive" | "deathmatch">("competitive");
  const [pid, setPid] = useState("");
  const [pd, setPd] = useState<Cs2ssPlayerDetailResponse | null>(null);
  const [cfgOpen, setCfgOpen] = useState(false);
  const [cfgInput, setCfgInput] = useState("");
  const [cfgSaving, setCfgSaving] = useState(false);

  useEffect(() => {
    if (!csgo) return;
    let c = false;
    (async () => {
      try {
        const cfgData = await api.getCs2ssConfig(csgo);
        if (c) return;
        if (!cfgData.steamId) { setCfgInput(""); setCfgOpen(true); setLoading(false); return; }
        setPid(cfgData.steamId);
        try {
          const [o, ms] = await Promise.all([api.getCs2ssOverview(csgo), api.listCs2ssMatches(csgo)]);
          if (c) return;
          setData(o); setMatches(ms);
        } catch {
          if (c) return;
          setData({ matchCount: 0, players: [] });
          setMatches([]);
        }
      } catch (e) {
        if (!c) setErr("无法连接数据。请确认 CS2SS 插件已安装。");
        reportError(e);
      } finally { if (!c) setLoading(false); }
    })();
    return () => { c = true; };
  }, [csgo, reportError]);

  useEffect(() => {
    if (!pid) return;
    api.getCs2ssPlayerDetail(csgo, pid).then(setPd).catch(() => {});
  }, [pid, csgo]);

  const saveCfg = async () => {
    if (!cfgInput.trim() || !csgo) return;
    setCfgSaving(true);
    try {
      await api.saveCs2ssConfig(csgo, { steamId: cfgInput.trim() });
      setPid(cfgInput.trim());
      setCfgOpen(false);
      try {
        const [o, ms] = await Promise.all([api.getCs2ssOverview(csgo), api.listCs2ssMatches(csgo)]);
        setData(o); setMatches(ms);
      } catch {
        setData({ matchCount: 0, players: [] });
        setMatches([]);
      }
    } catch (e) { reportError(e); }
    finally { setCfgSaving(false); }
  };

  if (cfgOpen) return (
    <div className="stats-panel" style={{ display: "flex", alignItems: "center", justifyContent: "center" }}>
      <div style={{ maxWidth: 440, width: "100%", padding: "32px 28px", borderRadius: 18, background: "var(--card)", border: "1px solid var(--line)", boxShadow: "var(--sh-pop)", textAlign: "center" }}>
        <div style={{ fontSize: 24, fontWeight: 800, color: "var(--c-accent)", marginBottom: 8 }}>⚙️ 首次配置</div>
        <p style={{ color: "var(--text-secondary)", fontSize: 13, lineHeight: 1.6, marginBottom: 20 }}>
          打开 Steam 个人资料页面，浏览器地址栏中
          <br />
          <code style={{ background: "var(--bg)", padding: "2px 6px", borderRadius: 4, fontSize: 12 }}>https://steamcommunity.com/profiles/<b>XXXXXX</b>/</code>
          <br />
          里的 <b>XXXXXX</b> 即为您的 Steam ID，将其粘贴到下方即可。
        </p>
        <input
          value={cfgInput}
          onChange={e => setCfgInput(e.target.value)}
          placeholder="7656119XXXXXXXXXX"
          autoFocus
          style={{
            width: "100%", padding: "10px 14px", border: "1px solid var(--line-strong)", borderRadius: 10,
            background: "var(--bg)", color: "var(--text-primary)", fontSize: 15, fontFamily: "monospace",
            textAlign: "center", outline: "none", marginBottom: 16,
          }}
          onKeyDown={e => { if (e.key === "Enter") saveCfg(); }}
        />
        <button
          disabled={!cfgInput.trim() || cfgSaving}
          onClick={saveCfg}
          style={{
            width: "100%", padding: "10px 0", borderRadius: 10, border: "none",
            background: "var(--c-accent)", color: "#fff", fontSize: 14, fontWeight: 700, cursor: "pointer",
            opacity: cfgInput.trim() ? 1 : 0.5,
          }}
        >
          {cfgSaving ? "保存中…" : "确定"}
        </button>
      </div>
    </div>
  );

  const comp = matches.filter(m => m.modeFamily === "competitive");
  const dms = matches.filter(m => m.modeFamily === "deathmatch");
  const po = data?.players.find(p => p.steamId === pid) ?? null;
  const tr = po?.totalRounds ?? 0;
  const tk = po?.kills ?? 0, td = po?.deaths ?? 0, ta = po?.assists ?? 0, tdm = po?.damage ?? 0, ths = po?.headshots ?? 0;
  const rating = tr > 0 ? cs2ssCalcRating(tk, td, ta, tdm, ths, tr, { kastRounds: po?.kastRounds, tradeKills: po?.tradeKills, multikill2: po?.multikill2, multikill3: po?.multikill3, multikill4: po?.multikill4, multikill5: po?.multikill5, clutchAttempts: po?.clutchAttempts, clutchesWon: po?.clutchesWon }) : 0;
  const adr = cs2ssCalcAdr(tdm, tr);
  const kast = cs2ssCalcKast(po?.kastRounds ?? 0, tr);

  const trend = (pd?.matches ?? []).filter(m => m.roundsPlayed > 0).slice(0, 20).reverse().map((m, i) => ({ i: i + 1, r: cs2ssCalcRating(m.totalKills, m.totalDeaths, m.totalAssists, m.totalDamage, m.totalHeadshotKills, m.roundsPlayed, { kastRounds: m.kastRounds, tradeKills: m.tradeKills, multikill2: m.multikill2, multikill3: m.multikill3, multikill4: m.multikill4, multikill5: m.multikill5, clutchAttempts: m.clutchAttempts, clutchesWon: m.clutchesWon }) }));
  const mpperf = (pd?.mapStats ?? []).filter(m => m.rounds > 0).map(m => ({ map: cs2ssMapLabel(m.map), r: cs2ssCalcRating(m.kills, m.deaths, m.assists, m.damage, m.headshots, m.rounds, { kastRounds: m.kastRounds, tradeKills: m.tradeKills, multikill2: m.multikill2, multikill3: m.multikill3, multikill4: m.multikill4, multikill5: m.multikill5, clutchAttempts: m.clutchAttempts, clutchesWon: m.clutchesWon }) })).sort((a, b) => b.r - a.r);

  const recent = (pd?.matches ?? []).slice(0, 10).map(m => {
    const r = m.roundsPlayed > 0 ? cs2ssCalcRating(m.totalKills, m.totalDeaths, m.totalAssists, m.totalDamage, m.totalHeadshotKills, m.roundsPlayed, { kastRounds: m.kastRounds, tradeKills: m.tradeKills, multikill2: m.multikill2, multikill3: m.multikill3, multikill4: m.multikill4, multikill5: m.multikill5, clutchAttempts: m.clutchAttempts, clutchesWon: m.clutchesWon }) : 0;
    const initTeam = m.initialTeam || m.team;
    const pw = initTeam === "CT" ? m.teamAScore : m.teamBScore;
    const ow = initTeam === "CT" ? m.teamBScore : m.teamAScore;
    return { ...m, r, pw, ow };
  });

  if (sub === "matchDetail" && selMatch > 0) return <StatsMatchDetail csgo={csgo} matchId={selMatch} onBack={() => setSub("dashboard")} />;
  if (sub === "history") return <StatsMatchHistory csgo={csgo} onOpenMatch={id => { setSelMatch(id); setSub("matchDetail"); }} onBack={() => setSub("dashboard")} />;
  if (loading) return <div className="stats-panel"><div className="stats-panel__loading">Loading…</div></div>;
  if (err && !data) return <div className="stats-panel"><div className="stats-panel__empty">{err}</div></div>;
  if (!data) return <div className="stats-panel"><div className="stats-panel__loading">Loading…</div></div>;

  if (data.matchCount === 0 && data.players.length === 0) return (
    <div className="stats-panel">
      <div className="stats-mode-switch">
        <button className="active">Competitive<small>0</small></button>
        <button className="">Deathmatch<small>0</small></button>
      </div>
      <div className="stats-panel-block">
        <div className="stats-panel__empty" style={{ padding: "60px 0", textAlign: "center" }}>
          暂无对局数据。安装 CS2SS 插件并完成至少一场比赛后，数据将在此展示。
        </div>
      </div>
    </div>
  );

  return (
    <div className="stats-panel">
      <div className="stats-mode-switch">
        <button className={mode === "competitive" ? "active" : ""} onClick={() => setMode("competitive")}>Competitive<small>{comp.length}</small></button>
        <button className={mode === "deathmatch" ? "active" : ""} onClick={() => setMode("deathmatch")}>Deathmatch<small>{dms.length}</small></button>
        <button style={{ marginLeft: 12, fontWeight: 600, color: "var(--c-accent)" }} onClick={() => setSub("history")}>All Matches →</button>
      </div>

      {mode === "competitive" && pid ? (<>
        <div className="stats-hero">
          <div><span className="stats-hero__eyebrow">PLAYER DOSSIER</span><h1>{po?.name ?? pid}</h1></div>
          <div className="stats-hero__rating"><small>OFFLINE RATING 2.0</small><strong style={{ color: rcol(rating) }}>{rating.toFixed(2)}</strong></div>
        </div>
        <div className="stats-cards">
          {[["Matches", po?.matches ?? 0], ["KAST", `${kast}%`, kast >= 75 ? HI : kast >= 65 ? MID : LO], ["ADR", adr.toFixed(1), adr >= 85 ? HI : adr >= 70 ? MID : LO], ["K/D", (tk / Math.max(1, td)).toFixed(2), (tk / Math.max(1, td)) >= 1.2 ? HI : (tk / Math.max(1, td)) >= 1 ? MID : LO], ["KDA", ((tk + ta) / Math.max(1, td)).toFixed(2)], ["KPR", (tr > 0 ? (tk / tr).toFixed(2) : "0.00")], ["HS%", `${(tk > 0 ? Math.round(ths / tk * 100) : 0)}%`], ["Clutches", `${po?.clutchesWon ?? 0}/${po?.clutchAttempts ?? 0}`]].map(([l, v, c]) => (
            <div className="stats-card" key={l}><span className="stats-card__label">{l}</span><span className="stats-card__value" style={c ? { color: c } as React.CSSProperties : undefined}>{v}</span></div>
          ))}
        </div>
        <div className="stats-impact">
          {[["Trade Kills", po?.tradeKills ?? 0], ["2K Rounds", po?.multikill2 ?? 0], ["3K Rounds", po?.multikill3 ?? 0], ["4K Rounds", po?.multikill4 ?? 0], ["Ace", po?.multikill5 ?? 0]].map(([l, v]) => (
            <div key={l}><span>{l}</span><b>{String(v)}</b></div>
          ))}
        </div>
        <div className="stats-charts">
          <div className="stats-panel-block">
            <div className="stats-panel-block__title"><div><span>TREND</span><h2>Rating Trend</h2></div></div>
            {trend.length > 0 ? <ResponsiveContainer width="100%" height={200}><LineChart data={trend}><CartesianGrid strokeDasharray="3 3" /><XAxis dataKey="i" tick={{ fontSize: 11 }} /><YAxis domain={[0, "auto"]} tick={{ fontSize: 11 }} /><Tooltip /><Line type="monotone" dataKey="r" stroke="#8e5cb8" strokeWidth={2.2} dot={{ r: 3, fill: "#8e5cb8" }} /></LineChart></ResponsiveContainer> : <p style={{ color: "var(--text-secondary)", textAlign: "center", padding: 32 }}>Not enough data</p>}
          </div>
          <div className="stats-panel-block">
            <div className="stats-panel-block__title"><div><span>MAPS</span><h2>Map Performance</h2></div></div>
            {mpperf.length > 0 ? <ResponsiveContainer width="100%" height={200}><BarChart data={mpperf} layout="vertical"><CartesianGrid strokeDasharray="3 3" /><XAxis type="number" domain={[0, "auto"]} tick={{ fontSize: 11 }} /><YAxis type="category" dataKey="map" tick={{ fontSize: 11 }} width={72} /><Tooltip /><Bar dataKey="r" radius={[0, 4, 4, 0]}>{mpperf.map((_entry, i) => <Cell key={i} fill={["#5d9cec","#3498db","#2ecc71","#f39c12","#9b59b6"][i % 5]} />)}</Bar></BarChart></ResponsiveContainer> : <p style={{ color: "var(--text-secondary)", textAlign: "center", padding: 32 }}>No map data</p>}
          </div>
        </div>
        <div className="stats-panel-block">
          <div className="stats-panel-block__title"><div><span>RECENT</span><h2>Recent Matches</h2></div></div>
          {recent.length > 0 ? (
            <table className="stats-table"><thead><tr><th>Map</th><th>Date</th><th>Score</th><th>K/D/A</th><th>ADR</th><th>Rating</th></tr></thead>
              <tbody>{recent.map(m => (
                <tr key={m.matchId} onClick={() => { setSelMatch(m.matchId); setSub("matchDetail"); }} style={{ cursor: "pointer" }}>
                  <td style={{ fontWeight: 600 }}>{cs2ssMapLabel(m.map)}</td><td style={{ color: "var(--text-secondary)", fontSize: 12 }}>{fmtD(m.startedAt)}</td>
                  <td><span style={{ color: "var(--st-green)", fontWeight: 600 }}>{m.pw}</span><span style={{ color: "var(--text-secondary)" }}> : </span><span style={{ color: "var(--st-red)", fontWeight: 600 }}>{m.ow}</span><span style={{ marginLeft: 8, fontWeight: 700, fontSize: 12, color: m.pw > m.ow ? "var(--st-green)" : "var(--st-red)" }}>{m.pw > m.ow ? "W" : "L"}</span></td>
                  <td>{m.totalKills}/{m.totalDeaths}/{m.totalAssists}</td><td>{cs2ssCalcAdr(m.totalDamage, m.roundsPlayed).toFixed(1)}</td>
                  <td><span style={{ fontWeight: 700, color: rcol(m.r) }}>{m.r.toFixed(2)}</span></td>
                </tr>
              ))}</tbody></table>
          ) : <div className="stats-panel__empty">No matches yet.</div>}
        </div>
      </>) : mode === "deathmatch" ? (
        <div className="stats-panel-block">
          <div className="stats-panel-block__title"><div><span>DM LOG</span><h2>Deathmatch Sessions</h2></div><p>{dms.length} sessions</p></div>
          {dms.length > 0 ? <table className="stats-table"><thead><tr><th>Map</th><th>Date</th><th>Mode</th><th>Duration</th><th style={{ textAlign: "right" }}>Rounds</th></tr></thead>
            <tbody>{dms.map(m => (
              <tr key={m.matchId} onClick={() => { setSelMatch(m.matchId); setSub("matchDetail"); }} style={{ cursor: "pointer" }}>
                <td style={{ fontWeight: 600 }}>{cs2ssMapLabel(m.map)} <span className="dm-tag">DM</span></td>
                <td style={{ color: "var(--text-secondary)", fontSize: 12 }}>{fmtD(m.startedAt)}</td>
                <td style={{ textTransform: "uppercase", fontSize: 11, color: "#df6b35", fontWeight: 700 }}>{m.ruleset}</td>
                <td>{Math.round(m.durationSeconds / 60)} min</td>
                <td style={{ textAlign: "right" }}>{m.roundsPlayed}</td>
              </tr>
            ))}</tbody></table> : <div className="stats-panel__empty">No DM sessions.</div>}
        </div>
      ) : null}
    </div>
  );
}