import { cs2ssMapLabel } from "../data/cs2ssMaps";
import type { Cs2ssMatchDetailResponse } from "../data/cs2ssTypes";
import "./StatsPanel.css";

interface Props { data: Cs2ssMatchDetailResponse; steamId: string; onBack: () => void; }

export default function StatsDMDetail({ data, steamId, onBack }: Props) {
  const { match, matchPlayers: mps, deathmatchLives: dls } = data;
  const min = Math.max(1 / 60, match.durationSeconds / 60);
  const me = mps.find(p => p.steamId === steamId) ?? mps[0];
  const myLives = dls.filter(l => l.steamId === (me?.steamId ?? ""));
  const bestLives = [...myLives].sort((a, b) => b.kills - a.kills || b.damage - a.damage || b.durationSeconds - a.durationSeconds).slice(0, 8);
  const ruleset = match.ruleset === "ffa" ? "FREE FOR ALL" : match.ruleset === "team_dm" ? "TEAM DM" : "RESPAWN";

  const rows = mps.map(p => ({
    p, kd: p.totalDeaths > 0 ? Math.round(p.totalKills / p.totalDeaths * 100) / 100 : p.totalKills,
    hs: p.totalKills > 0 ? Math.round(p.totalHeadshotKills / p.totalKills * 100) : 0,
    kpm: p.totalKills / min, dpm: p.totalDamage / min
  })).sort((a, b) => b.p.score - a.p.score);

  const best = { score: Math.max(0, ...rows.map(r => r.p.score)), kills: Math.max(0, ...rows.map(r => r.p.totalKills)), deaths: Math.min(...rows.map(r => r.p.totalDeaths)), kd: Math.max(0, ...rows.map(r => r.kd)), kpm: Math.max(0, ...rows.map(r => r.kpm)), dpm: Math.max(0, ...rows.map(r => r.dpm)), hs: Math.max(0, ...rows.map(r => r.hs)), streak: Math.max(0, ...rows.map(r => r.p.dmMaxKillStreak)), longest: Math.max(0, ...rows.map(r => r.p.dmLongestLifeSeconds)) };

  return (
    <div className="stats-panel">
      <button className="stats-back" onClick={onBack}>← Back</button>

      <div className="stats-hero" style={{ background: "linear-gradient(125deg, #151923, #283448 58%, #df6b35)" }}>
        <div>
          <span className="stats-hero__eyebrow">DEATHMATCH · {ruleset}</span>
          <h1>{cs2ssMapLabel(match.map)}</h1>
          <p style={{ color: "rgba(255,255,255,.65)", fontSize: 13 }}>{Math.round(match.durationSeconds / 60)} min · game_type {match.gameType} / game_mode {match.gameMode}</p>
        </div>
        <div style={{ position: "relative", zIndex: 1 }}><span className="dm-tag" style={{ fontSize: 13, padding: "4px 14px" }}>DM</span></div>
      </div>

      <div style={{ padding: "12px 16px", borderRadius: 10, border: "1px solid rgba(223,107,53,0.2)", background: "rgba(223,107,53,0.04)", color: "#c14e21", fontSize: 12, fontWeight: 600, textAlign: "center" }}>
        ⚠️ 死斗数据展示功能尚未完善，部分统计可能不准确。
      </div>

      {me && (
        <div className="stats-snapshot">
          <div className="stats-snapshot__lead" style={{ background: "linear-gradient(135deg, #d65c2c, #f18a45)" }}>
            <span>YOUR SCORE</span>
            <strong style={{ color: "#fff", fontSize: 38 }}>{me.score}</strong>
            <small>pts</small>
          </div>
          {[["K/D", `${me.totalKills}/${me.totalDeaths}`], ["KPM", rows.find(r => r.p.steamId === me.steamId)?.kpm.toFixed(2) ?? "0"], ["DPM", String(Math.round(rows.find(r => r.p.steamId === me.steamId)?.dpm ?? 0))], ["Max Streak", String(me.dmMaxKillStreak)], ["Longest", `${Math.round(me.dmLongestLifeSeconds)}s`]].map(([l, v]) => (
            <div key={l}><span>{l}</span><b>{v}</b></div>
          ))}
        </div>
      )}

      {me && (
        <div className="stats-metrics">
          {[["Avg Life", `${(me.dmAliveSeconds / Math.max(1, me.dmSpawnCount)).toFixed(1)}s`], ["Kills/Life", (me.totalKills / Math.max(1, me.dmSpawnCount)).toFixed(2)], ["DMG/Life", String(Math.round(me.totalDamage / Math.max(1, me.dmSpawnCount)))], ["Alive %", `${((me.dmAliveSeconds / Math.max(1, match.durationSeconds)) * 100).toFixed(1)}%`]].map(([l, v]) => (
            <div className="stats-metric-card" key={l}><span>{l}</span><b>{v}</b></div>
          ))}
        </div>
      )}

      <div className="stats-panel-block" style={{ padding: 0 }}>
        <div style={{ padding: "20px 24px 0" }}><span style={{ color: "#df6b35", fontSize: 10, fontWeight: 900, letterSpacing: ".18em" }}>LEADERBOARD</span><h2 style={{ margin: "4px 0 16px", fontSize: 18, fontWeight: 700 }}>Session Ranking</h2></div>
        <div style={{ overflowX: "auto" }}>
          <table className="stats-table" style={{ minWidth: 860 }}>
            <thead><tr><th>#</th><th>Player</th><th style={{ textAlign: "right" }}>Score</th><th style={{ textAlign: "right" }}>K</th><th style={{ textAlign: "right" }}>D</th><th style={{ textAlign: "right" }}>K/D</th><th style={{ textAlign: "right" }}>KPM</th><th style={{ textAlign: "right" }}>DPM</th><th style={{ textAlign: "right" }}>HS%</th><th style={{ textAlign: "right" }}>Streak</th></tr></thead>
            <tbody>
              {rows.map(({ p, kd, hs, kpm, dpm }, i) => (
                <tr key={p.steamId} style={p.steamId === me?.steamId ? { background: "rgba(223,107,53,.07)" } : undefined}>
                  <td style={{ fontWeight: 700 }}>{i + 1}</td>
                  <td style={{ fontWeight: 600 }}>{p.name}{p.isBot && <small style={{ color: "var(--text-secondary)", marginLeft: 4 }}>BOT</small>}</td>
                  <td style={{ textAlign: "right", fontWeight: 900, color: "#c14e21" }}>{p.score}</td>
                  <td style={{ textAlign: "right", fontWeight: p.totalKills === best.kills ? 700 : 400 }}>{p.totalKills}</td>
                  <td style={{ textAlign: "right", fontWeight: p.totalDeaths === best.deaths ? 700 : 400 }}>{p.totalDeaths}</td>
                  <td style={{ textAlign: "right", fontWeight: kd === best.kd ? 700 : 400 }}>{kd.toFixed(2)}</td>
                  <td style={{ textAlign: "right" }}>{kpm.toFixed(2)}</td>
                  <td style={{ textAlign: "right" }}>{Math.round(dpm)}</td>
                  <td style={{ textAlign: "right" }}>{hs}%</td>
                  <td style={{ textAlign: "right", fontWeight: p.dmMaxKillStreak === best.streak ? 700 : 400 }}>{p.dmMaxKillStreak}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {me && (
        <div className="stats-charts">
          <div className="stats-panel-block">
            <div className="stats-panel-block__title"><div><span>BURST</span><h2>Burst Firepower</h2></div></div>
            {[["5 sec", me.dmBurst5s2, me.dmBurst5s3, me.dmBurst5s4], ["10 sec", me.dmBurst10s2, me.dmBurst10s3, me.dmBurst10s4]].map(([l, v2, v3, v4]) => (
              <div key={String(l)} style={{ display: "grid", gridTemplateColumns: "60px repeat(3, 1fr)", alignItems: "center", gap: 8, padding: "12px 0", borderBottom: "1px solid var(--border-color, var(--line))" }}>
                <small style={{ color: "#d85f2e", fontWeight: 900 }}>{l}</small>
                <span style={{ fontSize: 13 }}><b style={{ fontSize: 22 }}>{String(v2)}</b> 2K</span>
                <span style={{ fontSize: 13 }}><b style={{ fontSize: 22 }}>{String(v3)}</b> 3K</span>
                <span style={{ fontSize: 13 }}><b style={{ fontSize: 22 }}>{String(v4)}</b> 4K+</span>
              </div>
            ))}
          </div>

          <div className="stats-panel-block">
            <div className="stats-panel-block__title"><div><span>LIVES</span><h2>Best Lives</h2></div><p>{myLives.length} records</p></div>
            <div className="stats-life-list">
              {bestLives.map(l => (
                <div key={l.lifeId} className="stats-life-row">
                  <b>#{l.lifeIndex}</b><span>{l.kills}K</span><span>{l.damage} DMG</span><span>{l.durationSeconds.toFixed(1)}s</span><small>{l.endKind === "death" ? "Died" : "Survived"}</small>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}