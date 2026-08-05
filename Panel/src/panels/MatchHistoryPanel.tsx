import { useCallback, useEffect, useMemo, useState } from "react";
import { Clock3, Film, Trash2 } from "lucide-react";
import Modal from "../components/Modal";
import { api, type MatchResult, type MatchSession, type MatchHistoryStats } from "../lib/api";
import { useT, type I18nKey } from "../i18n";
import { useStore } from "../state/store";
import MatchResultView from "./MatchResultView";
import { MAP_IMAGES, MAP_LABELS } from "../data/maps";
import { CartesianGrid, Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis, Bar, BarChart, Cell } from "recharts";
import "./MatchPanel.css";

type SessionStatus = "finished" | "interrupted" | "active";
const SESSION_STATUS_KEYS: Record<SessionStatus, I18nKey> = {
  finished: "match.finished",
  interrupted: "match.interrupted",
  active: "match.active",
};
const MC = ["#5d9cec","#e74c3c","#2ecc71","#f39c12","#9b59b6","#1abc9c","#3498db","#e67e22","#95a5a6","#34495e"];

function statusOf(s: MatchSession): SessionStatus {
  if (s.state === "interrupted") return "interrupted";
  return s.state === "finished" ? "finished" : "active";
}

export default function MatchHistoryPanel() {
  const t = useT();
  const { directory, reportError } = useStore();
  const csgo = directory?.valid ? directory.selected : null;
  const [history, setHistory] = useState<MatchSession[]>([]);
  const [result, setResult] = useState<MatchResult | null>(null);
  const [del, setDel] = useState<MatchSession | null>(null);
  const [mhs, setMhs] = useState<MatchHistoryStats | null>(null);
  const [tMaps, setTMaps] = useState<Set<string>>(new Set());

  const refresh = useCallback(async () => {
    if (!csgo) return setHistory([]);
    try {
      const [h, s] = await Promise.all([api.listMatchHistory(csgo), api.getMatchHistoryStats(csgo).catch(() => null)]);
      setHistory(h ?? []); setMhs(s);
      if (s?.perMap) setTMaps(new Set(s.perMap.map(m => m.map)));
    } catch (error) { reportError(error); }
  }, [csgo, reportError]);

  useEffect(() => { void refresh(); }, [refresh]);

  const stats = useMemo(() => {
    const f = history.filter(s => s.state === "finished");
    if (!f.length) return null;
    const avgPlayerScore = f.reduce((a, s) => a + s.player_score, 0) / f.length;
    const avgOpponentScore = f.reduce((a, s) => a + s.opponent_score, 0) / f.length;
    const mc = new Map<string, number>(); f.forEach(s => mc.set(s.map_id, (mc.get(s.map_id) ?? 0) + 1));
    const fav = [...mc.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
    return { total: history.length, completed: f.length, avgPlayerScore, avgOpponentScore, fav };
  }, [history]);

  const barData = useMemo(() => mhs?.perMap?.map(m => ({ map: MAP_LABELS[m.map] ?? m.map, r: +m.avgRating.toFixed(2), n: m.matches })) ?? [], [mhs]);
  const trendData = useMemo(() => {
    if (!mhs?.ratingTrend) return [];
    const maxN = Math.max(...mhs.perMap.map(m => m.matches));
    const maps = mhs.perMap.map(m => m.map);
    return Array.from({ length: maxN }, (_, i) => {
      const row: Record<string, any> = { i: i + 1 }; maps.forEach(m => { row[m] = null; }); return row;
    });
  }, [mhs]);

  const trendLines = useMemo(() => {
    if (!mhs?.ratingTrend || !trendData.length) return [];
    return mhs.perMap.filter(m => tMaps.has(m.map)).map((m, idx) => {
      const data = trendData.map(r => ({ ...r }));
      mhs.ratingTrend.filter(p => p.map === m.map).forEach((pt, i) => { data[i][m.map] = pt.rating; });
      return { map: m.map, data, color: MC[idx % MC.length] };
    });
  }, [mhs, trendData, tMaps]);

  const toggle = (m: string) => setTMaps(p => { const n = new Set(p); if (n.has(m)) n.delete(m); else n.add(m); return n; });

  const openResult = async (s: MatchSession) => {
    if (!csgo || !s.result_path || !["finished", "interrupted"].includes(s.state)) return;
    try { setResult(await api.getMatchResult(csgo, s.session_id)); } catch (error) { reportError(error); }
  };

  if (result) return <MatchResultView result={result} onClose={() => setResult(null)} t={t} csgo={csgo} />;

  return <div className="match-page match-history-page">
    <header className="workspace__head match-page__head">
      <div className="match-page__title"><span className="workspace__eyebrow">LOCAL ARENA MATCH</span><h1>{t("match.history")}</h1><p>{t("match.historySubtitle")}</p></div>
      <span className="match-map-count">{history.length}</span>
    </header>

    {stats && (
      <section className="mh-stats glass">
        <div className="mh-stat"><small>{t("mh.totalMatches")}</small><strong>{stats.total}</strong></div>
        <div className="mh-stat"><small>{t("mh.completedMatches")}</small><strong>{stats.completed}</strong></div>
        <div className="mh-stat"><small>{t("mh.avgScore")}</small><strong>{stats.avgPlayerScore.toFixed(1)} : {stats.avgOpponentScore.toFixed(1)}</strong></div>
        <div className="mh-stat"><small>Rating</small><strong>{mhs?.avgRating.toFixed(2) ?? "--"}</strong></div>
        <div className="mh-stat"><small>ADR</small><strong>{mhs?.avgAdr.toFixed(1) ?? "--"}</strong></div>
        <div className="mh-stat"><small>{t("mh.favMap")}</small><strong>{stats.fav ? MAP_LABELS[stats.fav] ?? stats.fav : "--"}</strong></div>
      </section>
    )}

    {barData.length > 0 && (
      <section className="glass" style={{ borderRadius: 14, padding: "16px 20px 12px" }}>
        <small style={{ color: "var(--text-tertiary)", fontSize: 9, fontWeight: 650, letterSpacing: ".05em", textTransform: "uppercase", display: "block", marginBottom: 8 }}>Rating by Map</small>
        <ResponsiveContainer width="100%" height={Math.max(80, barData.length * 38)}>
          <BarChart data={barData} layout="vertical" margin={{ top: 0, right: 40, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--line)" />
            <XAxis type="number" domain={[0, "auto"]} tick={{ fontSize: 11, fill: "var(--text-tertiary)" }} />
            <YAxis type="category" dataKey="map" tick={{ fontSize: 11, fill: "var(--text-secondary)", fontWeight: 600 }} width={72} />
            <Tooltip contentStyle={{ background: "var(--card)", border: "1px solid var(--line-strong)", borderRadius: 8, fontSize: 12 }} formatter={((_v: number, _n: string, props: any) => [`${(props.payload?.r ?? 0).toFixed(2)}`, `${props.payload?.n ?? 0} matches`]) as any} labelFormatter={((l: any) => `${l}`) as any} />
            <Bar dataKey="r" radius={[0, 4, 4, 0]} maxBarSize={26} isAnimationActive={false} label={{ position: "right", fontSize: 11, fill: "var(--text-secondary)", fontWeight: 600, formatter: ((v: any) => (typeof v === "number" ? v.toFixed(2) : "")) as any }}>
              {barData.map((_, i) => <Cell key={i} fill="var(--c-accent)" />)}
            </Bar>
          </BarChart>
        </ResponsiveContainer>
        <div style={{ display: "flex", gap: 16, flexWrap: "wrap", marginTop: 6 }}>
          {barData.map(d => <span key={d.map} style={{ fontSize: 11, color: "var(--text-tertiary)", fontWeight: 600 }}>{d.map} <b style={{ color: "var(--text-secondary)" }}>{d.n}</b></span>)}
        </div>
      </section>
    )}

    {trendLines.length > 0 && (
      <section className="glass" style={{ borderRadius: 14, padding: "16px 20px 12px" }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 4, flexWrap: "wrap", gap: 6 }}>
          <small style={{ color: "var(--text-tertiary)", fontSize: 9, fontWeight: 650, letterSpacing: ".05em", textTransform: "uppercase" }}>Rating Trend</small>
          <div style={{ display: "flex", gap: 10, flexWrap: "wrap" }}>
            {mhs!.perMap.map((m, i) => {
              const on = tMaps.has(m.map);
              return (
                <label key={m.map} style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 11, fontWeight: 600, color: on ? "var(--text-primary)" : "var(--text-tertiary)", cursor: "pointer" }}>
                  <span style={{ display: "inline-block", width: 10, height: 10, borderRadius: "50%", background: on ? MC[i % MC.length] : "var(--line-strong)", flexShrink: 0 }} />
                  <input type="checkbox" checked={on} onChange={() => toggle(m.map)} style={{ display: "none" }} />
                  {MAP_LABELS[m.map] ?? m.map}
                </label>
              );
            })}
          </div>
        </div>
        <ResponsiveContainer width="100%" height={220}>
          <LineChart margin={{ top: 8, right: 10, bottom: 0, left: 0 }}>
            <CartesianGrid strokeDasharray="3 3" stroke="var(--line)" />
            <XAxis dataKey="i" type="number" domain={[1, "dataMax"]} tickCount={Math.min(10, trendData.length)} tick={{ fontSize: 11, fill: "var(--text-tertiary)" }} />
            <YAxis domain={[0, "auto"]} tick={{ fontSize: 11, fill: "var(--text-tertiary)" }} />
            <Tooltip contentStyle={{ background: "var(--card)", border: "1px solid var(--line-strong)", borderRadius: 8, fontSize: 12 }} formatter={((v: number, n: string) => [v.toFixed(2), MAP_LABELS[n] ?? n]) as any} labelFormatter={((i: any) => `Match #${i}`) as any} />
            <Legend wrapperStyle={{ fontSize: 11, paddingTop: 6 }} />
            {trendLines.map(({ map, data, color }) => (
              <Line key={map} data={data} type="monotone" dataKey={map} name={map} stroke={color} strokeWidth={2.2} dot={false} activeDot={{ r: 4, fill: color }} connectNulls={false} />
            ))}
          </LineChart>
        </ResponsiveContainer>
      </section>
    )}

    {history.length === 0 ? (
      <div className="match-empty"><Clock3 size={19} /><span>{t("match.emptyHistory")}</span></div>
    ) : (
      <div className="mh-cards">
        {history.map((session, index) => {
          const status = statusOf(session);
          return (
            <article className={`mh-card is-${status}`} key={session.session_id} style={{ animationDelay: `${Math.min(index, 10) * 45}ms` }} onClick={() => void openResult(session)}>
              <span className="mh-card__map" aria-hidden="true">{MAP_IMAGES[session.map_id] && <img src={MAP_IMAGES[session.map_id]} alt="" />}<i>{MAP_LABELS[session.map_id] ?? session.map_id}</i></span>
              <span className="mh-card__main">
                <span className="mh-card__scoreline"><em className={`mh-badge is-${status}`}>{t(SESSION_STATUS_KEYS[status])}</em><strong>{session.player_score} : {session.opponent_score}</strong></span>
                <span className="mh-card__meta"><b>{session.opponent_name}</b><span>{new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(new Date(session.created_at_unix * 1000))}</span><span className={`mh-card__demo is-${session.demo.state}`}><Film size={12} />{t(`match.demoState.${session.demo.state}`)}</span></span>
              </span>
              <span className="mh-card__actions"><button className="match-history-delete" onClick={e => { e.stopPropagation(); setDel(session); }} aria-label={t("match.delete")} title={t("match.delete")}><Trash2 size={15} /></button></span>
            </article>
          );
        })}
      </div>
    )}

    <Modal open={!!del} title={t("match.delete")} onClose={() => setDel(null)} footer={<><button className="match-dialog-cancel" onClick={() => setDel(null)}>{t("common.cancel")}</button><button className="match-dialog-delete" onClick={async () => { if (!csgo || !del) return; try { await api.deleteMatch(csgo, del.session_id, true); setDel(null); await refresh(); } catch (error) { reportError(error); } }}>{t("match.delete")}</button></>}><p className="match-dialog-copy">{t("match.confirmDelete")}</p></Modal>
  </div>;
}
