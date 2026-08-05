import { useEffect, useState } from "react";
import { RadarChart, PolarGrid, PolarAngleAxis, PolarRadiusAxis, Radar, ResponsiveContainer, BarChart, Bar, Cell, XAxis, YAxis, CartesianGrid, Tooltip } from "recharts";
import { api } from "../lib/api";
import { cs2ssCalcRating, cs2ssCalcAdr, cs2ssCalcKd, cs2ssCalcHsPct, cs2ssCalcWinRate } from "../data/cs2ssRating";
import { cs2ssMapLabel } from "../data/cs2ssMaps";
import type { Cs2ssPlayerDetailResponse } from "../data/cs2ssTypes";
import { useT } from "../i18n";
import "./StatsPanel.css";

interface Props { csgo: string; steamId: string; selfSteamId?: string; onBack: () => void; }

export default function StatsPlayerDetail({ csgo, steamId, selfSteamId, onBack }: Props) {
  const t = useT();
  const [d, setD] = useState<Cs2ssPlayerDetailResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState("");

  useEffect(() => {
    api.getCs2ssPlayerDetail(csgo, steamId).then(r => { setD(r); setLoading(false); }).catch(e => { setErr(String(e)); setLoading(false); });
  }, [csgo, steamId]);

  if (loading) return <div className="stats-panel"><div className="stats-panel__loading">{t("stats.loading")}</div></div>;
  if (err || !d) return <div className="stats-panel"><div className="stats-panel__error">{err || t("stats.noData")}</div></div>;

  const { total, matches: pms, mapStats, name } = d;
  let wins = 0;
  for (const m of pms) if ((m.team === "CT" && m.ctScore > m.tScore) || (m.team === "T" && m.tScore > m.ctScore)) wins++;
  const avgR = pms.reduce((s, m) => s + cs2ssCalcRating(m.totalKills, m.totalDeaths, m.totalAssists, m.totalDamage, m.totalHeadshotKills, m.roundsPlayed, { kastRounds: m.kastRounds, tradeKills: m.tradeKills, multikill2: m.multikill2, multikill3: m.multikill3, multikill4: m.multikill4, multikill5: m.multikill5, clutchAttempts: m.clutchAttempts, clutchesWon: m.clutchesWon }), 0) / Math.max(1, pms.length);
  const wr = cs2ssCalcWinRate(wins, pms.length);
  const kd = cs2ssCalcKd(total.kills, total.deaths);
  const rcol = (r: number) => r >= 1.1 ? "#20b486" : r >= 0.9 ? "#e67e22" : "#e05d75";
  const fmtD = (iso: string) => { try { const dt = new Date(iso); return `${dt.getMonth() + 1}/${dt.getDate()}`; } catch { return iso; } };

  const radar = [
    { s: t("stats.rating"), v: Math.max(0, avgR) / 2 }, { s: "K/D", v: Math.min(kd / 3, 1) },
    { s: "ADR", v: Math.min(cs2ssCalcAdr(total.damage, total.rounds) / 150, 1) },
    { s: "KPR", v: Math.min(total.kills / Math.max(1, total.rounds), 1) },
    { s: "HS%", v: cs2ssCalcHsPct(total.headshots, total.kills) / 100 }, { s: t("stats.winPercent"), v: wr / 100 },
  ];

  const bar = [...pms].reverse().slice(-20).map((m, i) => ({ i: i + 1, r: cs2ssCalcRating(m.totalKills, m.totalDeaths, m.totalAssists, m.totalDamage, m.totalHeadshotKills, m.roundsPlayed, { kastRounds: m.kastRounds, tradeKills: m.tradeKills, multikill2: m.multikill2, multikill3: m.multikill3, multikill4: m.multikill4, multikill5: m.multikill5, clutchAttempts: m.clutchAttempts, clutchesWon: m.clutchesWon }) }));

  return (
    <div className="stats-panel">
      <button className="stats-back" onClick={onBack}>← {t("stats.back")}</button>

      <div className="stats-hero">
        <div>
          <span className="stats-hero__eyebrow">{t("stats.playerDossier")}</span>
          <h1>{name}{steamId === selfSteamId ? <span style={{ color: "#a99aff", fontSize: 14, marginLeft: 10, fontWeight: 400 }}>({t("stats.you")})</span> : ""}</h1>
          <span style={{ display: "block", color: "rgba(255,255,255,.55)", fontSize: 11, fontFamily: "monospace", marginTop: 4 }}>{steamId}</span>
        </div>
        <div className="stats-hero__rating">
          <small>{t("stats.averageRating")}</small>
          <strong style={{ color: rcol(avgR) }}>{avgR.toFixed(2)}</strong>
        </div>
      </div>

      <p style={{ textAlign: "center", color: "var(--text-secondary)", fontSize: 13, fontWeight: 600, margin: "-12px 0 0" }}>{pms.length} matches · {wins}W {pms.length - wins}L · {wr}%</p>

      <div className="stats-charts">
        <div className="stats-panel-block">
          <div className="stats-panel-block__title"><div><span>{t("stats.careerOverview")}</span><h2>{t("stats.performanceRadar")}</h2></div></div>
          <ResponsiveContainer width="100%" height={220}><RadarChart data={radar}><PolarGrid stroke="var(--border-color, #e2e4e9)" /><PolarAngleAxis dataKey="s" tick={{ fill: "var(--text-secondary)", fontSize: 11 }} /><PolarRadiusAxis angle={90} domain={[0, 1]} tick={false} /><Radar name={name} dataKey="v" stroke="#8e5cb8" fill="#8e5cb8" fillOpacity={0.2} /></RadarChart></ResponsiveContainer>
        </div>
        <div className="stats-panel-block">
          <div className="stats-panel-block__title"><div><span>{t("stats.trend")}</span><h2>{t("stats.ratingTrend")}</h2></div></div>
          <ResponsiveContainer width="100%" height={220}><BarChart data={bar}><CartesianGrid strokeDasharray="3 3" /><XAxis dataKey="i" tick={{ fontSize: 11 }} /><YAxis domain={[0, "auto"]} tick={{ fontSize: 11 }} /><Tooltip /><Bar dataKey="r" radius={[4, 4, 0, 0]}>{bar.map((d, i) => <Cell key={i} fill={rcol(d.r)} />)}</Bar></BarChart></ResponsiveContainer>
        </div>
      </div>

      <div className="stats-panel-block" style={{ padding: 0 }}>
        <div style={{ padding: "20px 24px 0" }}><span style={{ fontSize: 10, fontWeight: 900, letterSpacing: ".18em", color: "var(--c-accent)" }}>{t("stats.maps")}</span><h2 style={{ margin: "4px 0 16px", fontSize: 18, fontWeight: 700 }}>{t("stats.mapPerformance")}</h2></div>
        <table className="stats-table"><thead><tr><th>{t("stats.map")}</th><th style={{ textAlign: "right" }}>{t("stats.matches")}</th><th style={{ textAlign: "right" }}>K/D</th><th style={{ textAlign: "right" }}>ADR</th><th style={{ textAlign: "right" }}>{t("stats.rating")}</th></tr></thead>
          <tbody>{mapStats.map(ms => { const r = cs2ssCalcRating(ms.kills, ms.deaths, ms.assists, ms.damage, ms.headshots, ms.rounds, { kastRounds: ms.kastRounds, tradeKills: ms.tradeKills, multikill2: ms.multikill2, multikill3: ms.multikill3, multikill4: ms.multikill4, multikill5: ms.multikill5, clutchAttempts: ms.clutchAttempts, clutchesWon: ms.clutchesWon }); return (<tr key={ms.map}><td style={{ fontWeight: 600 }}>{cs2ssMapLabel(ms.map)}</td><td style={{ textAlign: "right" }}>{ms.matches}</td><td style={{ textAlign: "right" }}>{cs2ssCalcKd(ms.kills, ms.deaths).toFixed(2)}</td><td style={{ textAlign: "right" }}>{cs2ssCalcAdr(ms.damage, ms.rounds).toFixed(1)}</td><td style={{ textAlign: "right", fontWeight: 700, color: rcol(r) }}>{r.toFixed(2)}</td></tr>); })}</tbody></table>
      </div>

      <div className="stats-panel-block" style={{ padding: 0 }}>
        <div style={{ padding: "20px 24px 0" }}><span style={{ fontSize: 10, fontWeight: 900, letterSpacing: ".18em", color: "var(--c-accent)" }}>{t("stats.recent")}</span><h2 style={{ margin: "4px 0 16px", fontSize: 18, fontWeight: 700 }}>{t("stats.recentMatches")}</h2></div>
        <table className="stats-table"><thead><tr><th>{t("stats.map")}</th><th>{t("stats.date")}</th><th>{t("stats.score")}</th><th style={{ textAlign: "right" }}>K/D/A</th><th style={{ textAlign: "right" }}>ADR</th><th style={{ textAlign: "right" }}>HS%</th><th style={{ textAlign: "right" }}>{t("stats.rating")}</th></tr></thead>
          <tbody>{[...pms].reverse().map(m => { const r = cs2ssCalcRating(m.totalKills, m.totalDeaths, m.totalAssists, m.totalDamage, m.totalHeadshotKills, m.roundsPlayed, { kastRounds: m.kastRounds, tradeKills: m.tradeKills, multikill2: m.multikill2, multikill3: m.multikill3, multikill4: m.multikill4, multikill5: m.multikill5, clutchAttempts: m.clutchAttempts, clutchesWon: m.clutchesWon }); const pw = m.team === "CT" ? m.ctScore : m.tScore; const ow = m.team === "CT" ? m.tScore : m.ctScore; const won = pw > ow; return (<tr key={m.matchId}><td style={{ fontWeight: 600 }}>{cs2ssMapLabel(m.map)}</td><td style={{ color: "var(--text-secondary)", fontSize: 12 }}>{fmtD(m.startedAt)}</td><td><span style={{ color: "var(--st-green)", fontWeight: 600 }}>{pw}</span><span style={{ color: "var(--text-secondary)" }}> : </span><span style={{ color: "var(--st-red)", fontWeight: 600 }}>{ow}</span><span style={{ marginLeft: 8, fontWeight: 700, fontSize: 11, color: won ? "var(--st-green)" : "var(--st-red)" }}>{won ? "W" : "L"}</span></td><td style={{ textAlign: "right" }}>{m.totalKills}/{m.totalDeaths}/{m.totalAssists}</td><td style={{ textAlign: "right" }}>{cs2ssCalcAdr(m.totalDamage, m.roundsPlayed).toFixed(1)}</td><td style={{ textAlign: "right" }}>{cs2ssCalcHsPct(m.totalHeadshotKills, m.totalKills)}%</td><td style={{ textAlign: "right", fontWeight: 700, color: rcol(r) }}>{r.toFixed(2)}</td></tr>); })}</tbody></table>
      </div>
    </div>
  );
}
