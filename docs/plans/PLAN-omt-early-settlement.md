# PLAN：OMT 结算提前到赢家面板（弹窗时机不变）+ 结束原因可视化

日期：2026-10-19　状态：**待实施**（用户已裁决三处：弹窗仍等地图结束、面板要显示、结束原因值名保留 `MAP_END`）

---

## 1. 目标与非目标

**目标**

1. OMT 的对局结算从"地图结束"提前到 **`EventCsWinPanelMatch`**（赢家面板）——数据库行在比赛判定那一刻就定稿（`status=completed`、比分、`rounds_played`）。
2. 新增一个结束原因值，让"**地图结束＝你离开服务器/换图**"与"比赛被判定完成"在数据上可区分。
3. Panel 显示比赛级结束原因（含中文文案）。

**非目标**

- 不改弹窗时机：结算提前后，Panel 的结算弹窗**仍等你退回主界面/换图**时才出现。
- 不改 PMC（上一轮已收敛为"结束只认引擎赢家面板"，提交 `7ded1cd`）。
- 不引入新的契约面（哨兵仍是同一个 `last-completed.json`）。

---

## 2. 现状与实测证据

### 2.1 结算触发点（OMT `OfflineMatchTelemetry.cs`）

| 触发点 | 现值 | 位置 |
|---|---|---|
| `Listeners.OnMapEnd` | `MAP_END` | ~:316 |
| `OnMapStart`（旧局仍活跃） | `MAP_CHANGED` | :132 |
| `OnTick` 轮询发现回合计数重置 | `ROUND_COUNTER_RESET` | :273 |
| `Unload` | `PLUGIN_UNLOAD` | :878 |
| 启动清理 `CleanOrphanedMatches()` | `INTERRUPTED`（行转 `abandoned`） | ~:832 |

OMT **没有**注册 `EventCsWinPanelMatch`（零命中）；哨兵 `last-completed.json` 在 `FinalizeMatch` 结尾写。

### 2.2 实测（本机，用户 `telemetry.db` + 游戏日志）

- 全库 **102 行**：101 行 `completed`/`MAP_END`，1 行 `abandoned`/`INTERRUPTED`（id 100：10-17 22:34 开局、无回合、10-18 18:27 启动清理时关闭；该时间戳与当天开游戏报错时刻一致）。
- 全部 **1783 条回合行 `source` 均为 `event`**（0 条 `poll`）→ 回合结束事件路径从未失效过，**最后一回合始终被记为 `event`**（各局 `rounds_played` 与最大 `round_number+1` 逐局一致，无漏记）。
- 一次真实时间线（match 101，日志原样）：

```
18:27:26.489 [OMT] Match 101 started on de_dust2 mode=competitive ruleset=round_based_unbalanced
18:49:01.862 [OMT] Round 14: match=101 source=event          ← 最后一回合（事件写入）
18:49:01.912 [PMC] Result written ... state="Finished", score=13-2   ← PMC（赢家面板 + 0.25s）
18:49:11.539 [OMT] Match 101 ended: de_dust2 reason=MAP_END  ← OMT 结算（地图结束）
```

**已用埋点实测（M1，2026-10-19 19:28，match 104 on de_nuke）——顺序已确证**：

```
19:28:10.284  DB  rounds round_number=17 captured_at   ← 最后一回合写入（写前拍的库时间）
19:28:10.294  [OMT] Round 17: match=104 source=event tick=95687
19:28:13.299  [OMT] WinPanel: match=104 round=17 tick=95881      ← +3.005 s / +194 ticks
19:28:13.580  [PMC] Result written ... state="Finished"          ← 面板 +281 ms（= 0.25 s 定时器 + 31 ms 工作）
19:28:19.043  [OMT] Match 104 ended: de_nuke reason=MAP_END      ← 面板 +5.74 s
```

- **回合结束（含写库）比赢家面板早约 3.0 秒**：墙钟差 3.005 s，tick 差 194（194/3.005 ≈ 64.6 tick/s，与 64 tick 服务器一致）。库内 `rounds_played=18`（回合号 0..17），赢家面板之后**没有任何回合行**。
- 这个 3 s 不是巧合而是玩法节奏：赢家面板在回合结束演出（胜负定格/击杀回放）**之后**才出。
- 附带确认：本次 PMC 的写入在面板后 **281 ms** 且日志行里已无 `score=` —— 正好是“赢家面板 + 0.25 s 定时器”路径，也侧面确认了 `7ded1cd`（去掉 PMC 自己数分）已部署生效。
- **更正记录**：此前一轮曾用“PMC 写入 − 0.25 s”反推面板时刻并据此**撤回**顺序结论——那个反推方法本身是错的（旧 PMC 的 `Finished` 可能来自规则路径，不经过定时器），但**结论“回合结束在前”反而是对的**。现在有直测证据。

→ 于是：**0.25 s 延迟不承担正确性**（最后一回合在面板前 3 s 就已落库）。**已按用户决定移除（2026-10-19）**：改为在处理器内同步结算，见 §3.1。

---

## 3. 设计

### 3.1 OMT：在赢家面板结算

新增 `EventCsWinPanelMatch` 处理器，**同步结算（无定时器、无延迟）**：

```
EventCsWinPanelMatch
  → FinalizeMatch("MATCH_DECIDED")   // 写库：completed + 比分/rounds_played 定稿
  → ResetMatchState()            // 必须，见下
```

- **为什么不要延迟**：§2.2 实测最后一回合比面板早约 **3.0 s**，结算那一刻数据已就绪；同步结算额外赢两点：
  - **`MATCH_DECIDED` 是确定性的**：定时器版带 `STOP_ON_MAPCHANGE`，若地图在这 0.25 s 内被换掉，回调作废、原因会退化成 `MAP_END`/`MAP_CHANGED`；同步版不会。
  - **少一套机制**：不需要 `Modules.Timers`、不需要定时器标志，处理器形态与 `OnRoundEnd`/`OnMapEnd` 同构。
  - **玩家快照更准**：`FinalizeMatch` 首行 `CachePlayers(CapturePlayers())` 读的是**当时还在服务器上的玩家实体**；面板那一刻玩家齐全，延迟 0.25 s 也无害，而旧的地图结束路径下玩家可能已开始卸载。
- **已知残余风险（唯一一条）**：若某类结束方式下引擎在最后一个 `round_end` **之前**就发赢家面板，结算时比分字段 `_ctScore/_tScore/_teamAScore/_teamBScore` 仍停在前一回合，**最后一条回合行也不会被写入**（`OnRoundEnd` 的 `if (_matchActive)` 已因重置而跳过）。注意 `rounds_played` 有部分兜底：`CompletedRounds()` 取 `Math.Max(_lastWrittenRound + 1, Math.Max(_lastObservedRounds, GetTotalRoundsPlayed()))`，所以回合**数**可能仍对，但**比分与回合行**会缺。已测的常规结束（回合演出后出面板）有 **3 s** 余量；投降/特殊模式的结束未测量。检测方式：比最后一条 `Round N` 与 `WinPanel … round=N` 的先后，并核对库内回合行数/比分是否等于该局实际；一旦出现，恢复 0.25 s 延迟即可。
  - **用户裁决（2026-10-19）：不预先加保险**（当时实测回合结束总是在赢家面板前 3 s）——不加“库存回合行数 vs 游戏回合数”的一致性校验；真出问题再处理。
- **为什么必须 `ResetMatchState()`**：不重置则 `_matchActive` 仍为真；玩家**留在服务器**、游戏回到选边界面时，`OnTick` 的"回合计数重置"分支会再次结算同一行，把 `end_reason` 覆盖成 `ROUND_COUNTER_RESET`（`MATCH_DECIDED` 白写）。重置是安全的：`DetectMode()` 在 `BeginMatch()` 内，下次开局自动重新识别模式。
- 不需要 `Modules.Timers`（同步结算）。
- 幂等：`FinalizeMatch` 首行 `if (!_matchActive) return;` 已保证。
- 附带收益：正常打完的局不再停留在 `in_progress`，启动清理（`INTERRUPTED`）只会命中真正异常退出的局。

### 3.2 end_reason 词表

| 值 | 含义 | 触发点 |
|---|---|---|
| **`MATCH_DECIDED`**（新增） | 比赛被引擎判定完成（赢家面板出现） | `EventCsWinPanelMatch`（同步） |
| `MAP_END` | **地图结束——你离开服务器/换图（此时比赛尚未被判定）** | `OnMapEnd` |
| `MAP_CHANGED` | 新地图开始时发现旧局仍活跃 | `OnMapStart` |
| `ROUND_COUNTER_RESET` | 回合计数被重置（回到选边界面） | `OnTick` |
| `PLUGIN_UNLOAD` | 插件卸载（游戏退出/重载） | `Unload` |
| `INTERRUPTED` | 上次异常退出留下的未结算行 | 启动清理 |

**为什么叫 `MATCH_DECIDED` 而不是 `WIN_PANEL`**：这个字段回答的是“**这一行为什么关闭**”，不是“哪个 UI 元素出现了”。`WIN_PANEL` 是引擎面板的名字（`cs_win_panel_match`），面板一改名或改成别的展示形式，存储值就失真；而且直接查库的人得先回文档才能解释它。`MATCH_DECIDED` 自解释，并与 `INTERRUPTED`（没打完就被打断）天然对照。代价为零：该值尚未写入任何一行数据，Panel 对未知值本来就原样显示（前向兼容），以后再改也不会断旧数据。备选：想要更口语就用 `MATCH_COMPLETE`；想坚持“按触发事件命名”的旧风格则保留 `WIN_PANEL`。

**已裁决（2026-10-19）：保留 `MAP_END` 这个值名**（加了 `MATCH_DECIDED` 之后它的语义正好收窄成“你离开服务器/换图”；库里已有 101 行旧值，改名会让词表分叉）。不再考虑改名 `LEFT_SERVER`。

### 3.3 弹窗时机不变：把"写数据"与"发信号"拆开（方案 B1）

现状：哨兵在 `FinalizeMatch` 结尾写 → 结算提前，哨兵提前，Panel 提前弹（＝被否决的 A 方案）。

**B1（选定）**

1. `FinalizeMatch` 只写数据库，**不再写哨兵**。
2. 新增 `FlushCompletionMarker()`：在**本局生命周期真正关闭**的四处调用——`OnMapEnd`、`OnMapStart`（旧局仍活跃时）、`ROUND_COUNTER_RESET` 分支、`Unload`。
3. **暂存**：因 `ResetMatchState()` 会清空 `_currentMatchId`/`_matchMap`/`_matchStartedAt`，需在结算时把 `match_id / map / started_at / ended_at / reason` 抄进待发字段，到上述四个时刻再写文件、写后清空。
4. Panel 侧**零改动**：弹窗时机与现状逐字一致（退回主界面/换图时才弹）；打赢后留在服务器不回主界面仍不弹。

**B3（备选，未选）**：赢家面板写哨兵（带 `reason`），Panel 收到只取数据不弹；地图结束时再写第二个标记（如 `last-closed.json`），Panel 收到才弹。代价：Rust watcher 新契约 + 契约测试（哨兵形状是字节钉住的）。

### 3.4 Panel 显示比赛级结束原因

- **数据面无需改动**：`cs2ss_bridge` 已取回 match 级 `end_reason`（`Panel/src/data/cs2ssTypes.ts` 的 match 类型），只是**没有任何界面渲染**（现仅渲染每回合原因 `cs2ssRoundEndReasonLabel`）。
- 新增映射（与现有 round 版放同一处）：

| 值 | 中文文案 |
|---|---|
| `MATCH_DECIDED` | 比赛已判定（引擎判定完成） |
| `MAP_END` | 地图结束（离开了服务器/换图） |
| `MAP_CHANGED` | 换图前收尾 |
| `ROUND_COUNTER_RESET` | 回合计数重置（回到选边） |
| `PLUGIN_UNLOAD` | 插件卸载（游戏退出/重载） |
| `INTERRUPTED` | 未结算被打断（上次异常退出） |
| 其它 | 原样显示（前向兼容） |

- 渲染位置：`Panel/src/panels/StatsMatchDetail.tsx` 现有"结束原因"区域旁加一行比赛级原因。
- i18n 两文件流程：`Panel/src/i18n/keys.ts`（EN，源）＋ `Panel/src/i18n/dictionary.ts`（简中），其余语言回落英文。

---

## 4. 验证计划

1. 门禁：`scripts/linux/build.sh --only dotnet`（0 警告 0 错误 + MatchCore/PKC 测试）；`npm run build`（tsc 权威）。
2. 实机三场景：
   - ① 打完 13-2 → 库里应出现 `completed`/`MATCH_DECIDED`，且 `rounds_played`/比分**此刻**即正确；**退回主界面后** Panel 才弹结算。
   - ② 中途退出/换图 → 应为 `MAP_END`/`MAP_CHANGED`，哨兵照写、照弹。
   - ③ 留在服务器不回主界面 → 不弹（与现状一致）。
3. 顺序确认：**已完成（M1 埋点，match 104）**——回合结束比赢家面板早约 3.0 s（+194 ticks），见 §2.2。
4. 面板文案：统计页比赛详情能看到中文原因；未映射值原样显示。
5. 部署：OMT DLL → 游戏目录；`pwsh scripts/package.ps1 -Linux -ReleaseVersion 1.4.3.3 -Explode` 刷新 bundle。

---

## 5. 风险与已知不确定

| 项 | 说明 |
|---|---|
| 事件顺序（已实测） | 回合结束比赢家面板早约 3.0 s（match 104 埋点，+194 ticks）；残余风险与检测方式见 §3.1 |
| 死亡竞赛 | DM 结束若也触发赢家面板，会多一行 `MATCH_DECIDED`（其 `rounds_played` 仍按既有规则记 0）——可接受 |
| 同图重开产生 0 回合行 | 既有行为（如 id 25/35）：结算后回到选边会 `BeginMatch()` 建新行；本次改动不加重也不修复 |
| 玩家直接杀进程 | 抽屉里的哨兵不会贴出 → Panel 不弹（与现状一致）；但账本已在赢家面板结清，故下次启动不会被标 `INTERRUPTED` |

---

## 6. 落地清单（实施时按此顺序）

1. `addons/counterstrikesharp/plugins/OfflineMatchTelemetry/OfflineMatchTelemetry.cs`
   - 注册 `EventCsWinPanelMatch` + 处理器（**同步**：`FinalizeMatch("MATCH_DECIDED")` → `ResetMatchState()`，不加定时器）
   - 哨兵写入拆出 `FlushCompletionMarker()` + 待发字段；四处关闭点调用
2. `Panel/src/data/cs2ssRating.ts`（或现有 round 版所在文件）新增 match 级原因映射
3. `Panel/src/panels/StatsMatchDetail.tsx` 渲染一行
4. `Panel/src/i18n/keys.ts` + `dictionary.ts` 新增文案键
5. 门禁 + 部署 + 重打包（§4）
