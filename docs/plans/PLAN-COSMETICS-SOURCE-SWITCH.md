# 计划：饰品插件（InventorySimulator）直连/本地双模式 + Panel 下载更新服务

状态：**计划稿，实施中**。决策来源：用户 2026-10-27 口头拍板（见 §2）。事实基线取证于 2026-10-27，含本机实机验证。配套文档：`docs/PLAYER-COSMETICS.md`（现有饰品语义，PKC 部分已弃用）。交付物：`scripts/migrate-cosmetics-to-inventory.ps1`（单脚本，见 §5）。

---

## 1. 目标

1. Panel 提供一个开关：**官网直连**（默认）或**本地清单**。
2. 直连模式下，饰品完全由 `https://inventory.cstrike.app/` 决定，**Panel 不干预**（不生成、不读写 `inventories.json`）。
3. 本地模式下，Panel 读写 `inventories.json`，按 SteamID64 键控。
4. Panel 提供该插件的**下载、校验、安装、更新**服务；插件**不随包发布**。

**交付形态（用户 2026-10-27 追加）**：只提供一个脚本，语言由实施方选定 → **PowerShell 7**（`scripts/migrate-cosmetics-to-inventory.ps1`）。职责单一：**把 Panel 导出的 `cs2bip-cosmetics-preset.json` 迁移成 `inventories.json`**。理由：本仓流水线已经是 pwsh（`build.ps1`/`package.ps1`/`verify-workspace.ps1`），Windows 与 Linux 一致，无需引入第三种语言；且它只做纯数据转换，不进入 Panel 运行时。

**不在脚本职责内**：插件的下载/更新（用户已明确本次只要迁移脚本），以及 `autoexec.cfg` 写入。

---

## 2. 已拍板的决策

| # | 决策 | 值 |
| --- | --- | --- |
| D1 | SteamID64 来源 | **用户在 UI 里填写**（不读 `loginusers.vdf`） |
| D2 | 默认模式 | **直连**（`cosmetics_source = "remote"`） |
| D3 | 插件分发方式 | **不随包发布**，下载式安装 |
| D4 | 空 `inventories.json` 的行为 | 插件报错后**照常向远端拉取**（用户 2026-10-27 实机确认），因此空文件是"噪音 + 走远端"，不是致命错误 |
| D5 | 写 `autoexec.cfg` | **暂不实现**，只作为备选手段记录（见 §3.1） |
| D6 | `invsim_url` | **不写入**；插件默认值已是 `https://inventory.cstrike.app` |
| D7 | SteamID64 手填校验 | **不做**软校验/诊断，只做最小格式检查（17 位数字） |
| D8 | Souvenir | **不做**任何映射，等上游更新 `InventoryItem` 支持后再谈 |
| D9 | PKC 字段 | **全部弃用**；本地清单直接从 Panel 的饰品模型生成，不经过 PKC schema 5 |

D4 的意义：本地模式即使清单写坏，也不会让玩家彻底没饰品，只是退回官网数据。这让 P2 的实现可以更简单（不需要"宁可拒绝进入本地模式"这种硬失败），但**空文件仍然要避免**——它是日志噪音，也会让"本地模式"变成"看起来是本地、实际是远端"的假象。

---

## 3. 开关怎么实现（核心设计）

插件**没有**运行期布尔开关，行为由 `invsim_file` 指向的文件**是否存在**决定。这是整个计划的地基：

```
Inventories.Load(invsim_file)
  ├─ 文件不存在        → 静默 false，该 SteamID 走 HTTP 拉取
  ├─ 文件存在但非法/空  → 抛异常并记 EROR，随后仍走 HTTP（D4 实机确认）
  └─ 文件存在且合法     → 装入内存，该 SteamID 不再发 HTTP
```

因此开关的映射是：

| 模式 | `invsim_file` 指向 | `inventories.json` |
| --- | --- | --- |
| 直连（默认） | 一个**不存在**的路径（如 `.../.csbip/no-local-inventory.json`） | 不创建；已存在则改名为 `.bak` |
| 本地 | `inventories.json`（默认值，落点见 §4） | Panel 原子写入合法 JSON |

**必须三层一致**，否则会出现"面板显示直连、实际读本地"这类最难查的问题：

1. `AppConfig.cosmetics_source: Option<String>`（`lib.rs:104` 结构体内新增，`#[serde(default)]`，缺省 `"remote"`）；
2. `game/csgo/cfg/autoexec.cfg` 里的 `invsim_file` 行；
3. 磁盘上的 `inventories.json` 是否存在且合法。

**写 cfg 的硬性约束**：**本期不实现**。`autoexec.cfg` 写入仅作为备选手段保留（见 §3.1），因为默认查找路径（§4）已能表达两种模式，无需改 cfg。

```
// managed_cosmetics_start
invsim_file "<mode-dependent>"
// managed_cosmetics_end
```

### 3.1 备选：写 cfg（暂不实现）

若将来必须把清单放在非默认路径（例如多实例共存、或用户已有自建 API 需要改 `invsim_url`），才需要写 `autoexec.cfg`。届时必须用 managed block，块外字节保留；本仓已有先例 `// managed_team_lineup_start ... end`（`lib.rs:773`）。

**不写 `invsim_url`**：插件默认值就是 `https://inventory.cstrike.app`（`ConVars.Url`），写它只会在用户已自定义时产生覆盖风险，而没有任何收益。

---

## 4. `inventories.json` 的绝对路径（已实机确认）

`Server.GameDirectory` 在本机是 `.../Counter-Strike Global Offensive/game`（叶子 `game`；证据：PlusMatchCoordinator 实机日志 `game directory=.../Counter-Strike Global Offensive/game`）。

上游 `Inventories.Load()` 的候选顺序代入本机：

| 优先级 | 绝对路径 |
| --- | --- |
| 1（默认值首选） | `<game>/csgo/addons/counterstrikesharp/configs/plugins/InventorySimulator/inventories.json` |
| 2（次选） | `<game>/csgo/inventories.json` |
| 3（兜底） | `invsim_file` 写成绝对路径时的任意位置 |

其中 `<game>` = `/mnt/game/SteamLibrary/steamapps/common/Counter-Strike Global Offensive/game`（Windows 上为 `...\Counter-Strike Global Offensive\game`）。

**结论**：保持默认 `invsim_file "inventories.json"` 时，Panel 只需写到候选 1 的路径，不必写绝对路径。§3 表格里"直连模式指向不存在路径"要写成绝对路径（因为候选 1 目录可能不存在，绝对路径更可控）。

---

## 5. 分阶段落地

**交付形态：单一脚本 `scripts/migrate-cosmetics-to-inventory.ps1`（PowerShell 7）。** 不进入 Panel 运行时，不改 `AppConfig`、不改 `generate_handler!`、不改前端。

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| **P0 收口现状**（已完成 2026-10-27） | `cfg/autoexec.cfg` 已删除；直连路径由用户实机确认可用 | 游戏内看到官网配置的饰品 |
| **P1 迁移脚本**（已完成） | `scripts/migrate-cosmetics-to-inventory.ps1`：读 Panel 导出包 → 映射为 `inventories.json`，含贴纸/挂件坐标解析、souvenir 丢弃提示、空 paint 跳过 | 干跑与真实写出均验证；`keychains` 为数组、`uid` 连续唯一（实测） |
| **P2 落地验证**（待办） | 用真实导出文件跑一次，把结果放进插件默认路径，进游戏确认刀/手套/枪/贴纸/挂件生效 | 游戏内可见 + 日志无加载错误 |
| **P3 后续**（非本期） | 若需把迁移搬进 Panel（按钮/命令），再按 §9 的"后续"表改动 | — |

脚本不依赖 Panel，因此不触发 `cargo test` / `bun run build` 的变化。

---

## 6. 迁移脚本接口

```
pwsh scripts/migrate-cosmetics-to-inventory.ps1 -Source <cs2bip-cosmetics-preset.json> \
     -SteamId <7656119...> [-Out inventories.json] [-CsgoRoot path] \
     [-AgentDefindex N] [-UidBase 900000] [-DryRun]
```

行为要点：

- **只做数据转换**：不联网、不装插件、不改 cfg。`-Out` 缺省时落到插件默认路径 `<csgo>/addons/counterstrikesharp/configs/plugins/InventorySimulator/inventories.json`。
- **校验输入**：必须是 `kind = cs2bip-cosmetics-preset` 且 `schema_version ∈ {1,2}` 的 Panel 导出包；`-SteamId` 强制 17 位数字。
- **映射**：`knife_presets[def] → knives[team]`、`glove → gloves[team]`、`gun_presets[def] → ctWeapons/tWeapons`、`stickers[] → stickers[]`、`charm{id,placement_id} → keychains[{x,y,z}]`、`music_kit_id → musicKit`。队伍键：CT=3、T=2。
- **贴纸坐标**：Panel 存的是相对偏移；当偏移为 `(0,0)` 且目录里有该槽位坐标时，用 `cosmeticPlacements.json` 的 `stickerPositions[schema]` 填充；自定义偏移则原样保留。
- **挂件坐标**：按 `charm.placement_id` 在 `cosmeticPlacements.json[defindex].charmPositions` 里查 `x/y`，`z` 固定 0。
- **明确丢弃**：`souvenir_enabled`（上游无字段，会在报告里计数提示）；`paint <= 0` 的预设（无皮肤可应用）；刀不输出挂件（`cosmeticPlacements.json` 只覆盖 defindex 1–64）。
- **探员**：Panel 目录只存模型路径，没有探员 defindex → 默认跳过并提示；需要时用 `-AgentDefindex` 显式指定。
- **原子写入**：temp → rename，拒绝写出 0 字节文件（空文件会让插件在加载期抛异常）。
- **`-DryRun`**：只打印映射报告，不写文件。

---

## 7. 兼容矩阵（验收必须覆盖）

脚本侧（已实现，可实测）：

| 场景 | 脚本行为 |
| --- | --- |
| 输入不是 Panel 导出包 | 报错并回显实际 `kind`（实测） |
| `-SteamId` 不是 17 位 | 参数校验直接拒绝（实测） |
| `paint <= 0` 的预设 | 跳过并计数提示（实测） |
| `souvenir_enabled` | 丢弃并计数提示（实测：2 条） |
| 贴纸偏移为 (0,0) | 用 `cosmeticPlacements.json` 的槽位坐标填充（实测：0.25/0.48） |
| 贴纸自定义偏移 | 原样保留（实测：0.4/0.6） |
| 挂件 `placement_id` | 解析成 `x/y/z` 并输出为**数组**（实测：`[{...}]`） |
| `uid` 分配 | 从 `-UidBase` 起连续且唯一（实测：900000…900004） |
| 探员缺 defindex | 跳过并提示可传 `-AgentDefindex`（实测） |
| `-DryRun` | 只打印映射报告，不写盘（实测） |

运行期（需实机验证，脚本不负责）：

| 场景 | 期望 |
| --- | --- |
| 直连 + 断网 | 加载无异常；拉取失败有日志；无饰品 |
| 本地 + 清单合法但 SteamID 未命中 | 插件自行走 HTTP（代码逻辑，实机未验） |
| 在线模式（Online） | 沿用现有 `enforce_mode_cosmetics` 门控，不因本计划放宽 |

---

## 8. 风险

1. **上游契约漂移**：`invsim_file` 语义、候选路径、`EquippedV5Response` 字段都可能在 v4 变化 → 迁移脚本的输出需在升级插件后重新验证一次（字段集固定，升级后跑一遍 dry-run 即可对比）。
2. **`Server.GameDirectory` 语义**：本机是 `.../game`，Windows/Proton 可能不同 → 脚本用"探测 + 多候选 + `gameinfo.gi` 确认"，不硬编码。
3. **SteamID64 手填错误**：**不做诊断**（D7）。脚本只做最小格式检查与"清单里是否真的有这个键"；写错的表现是"本地模式不生效"，由 `status` 的 mode 段落可看出。
4. **直连仍依赖官网可用性**：`Api.cs` 有 50 容量/3.6 秒补给的令牌桶，离线时 GET 会重试 3 次（30 秒超时）→ 断网即无饰品，不做重试放大。
5. **`-insecure` 边界**：文档不得暗示"直连官网 = 安全对局"。
6. **纪念品（Souvenir）**：**不处理**（D8），等上游 `InventoryItem` 支持后再谈。
7. **签名面**：上游 gamedata 26 条记录（约 20 个函数签名 + 6 个结构体偏移），跨 `server` 与 `engine2`；好处是走 CSS 标准 gamedata、可热修，且 v3.7.0 与 10-15 更新同日发布。
8. **单维护者 + 无测试**：上游无测试工程 → 靠"钉版 + `assets[].digest` 校验 + 安装标记"兜底。
9. **CSS 版本差**：本仓 1.0.371 vs 上游 1.0.375 → 脚本不自动升级 CSS；若加载报版本错，需手工升级 CSS 后重试。
10. **许可**：MIT 与本仓 AGPLv3 兼容，分发需保留上游版权声明（`License.txt`）。

---

## 9. 与既有实现的衔接点

**本期（脚本）**

| 位置 | 改动 |
| --- | --- |
| `scripts/migrate-cosmetics-to-inventory.ps1` | 新增（已交付） |
| `scripts/AGENTS.md` | 补一行说明（待办） |

**后续若要把脚本能力搬进 Panel（非本期）**

| 位置 | 改动 |
| --- | --- |
| `Panel/src-tauri/src/lib.rs:104` `AppConfig` | 新增 `cosmetics_source` |
| `Panel/src-tauri/src/lib.rs` `generate_handler!` | 注册对应命令 |
| `Panel/src/lib/api.ts` | 对应 invoke wrapper |
| `Panel/src-tauri/src/installer.rs:22-26` `PLUS_MARKERS` | PKC 标记改为 InventorySimulator 标记 |
| `Panel/src-tauri/src/installer.rs` `SUITE_OWNED_FILES` | 加入 `configs/plugins/InventorySimulator/*.json` |
| `scripts/verify-workspace.ps1`（addons 白名单，:97 起） | 加入新插件目录与 gamedata 文件 |
| `Panel/src-tauri/src/install_checks.rs:255` | 健康检查增加插件存在性 |
| `Panel/src/i18n/keys.ts` + `dictionary.ts` | 新模式/新错误码 |

---

## 10. 未验证项（显式标注）

- **已撤回的错误结论**：之前本文件写过"`autoexec.cfg` 删除后 `inventories.json` 仍被加载"。复核后不成立：插件只从 §4 的两个默认路径取文件，而 `/mnt/game/inventories.json` 不在其中；日志中该路径的报错是 `invsim_file` 指向绝对路径时产生的（该行随后已随 `autoexec.cfg` 删除）。用户实测结果（删除后走网站数据）与代码一致。
- 本地模式写入后，插件在"清单合法但 SteamID 未命中"时的回退行为（代码上会走 HTTP，实机未验）。
- Windows/Proton 下 `Server.GameDirectory` 的叶子是否为 `game`（本机 Linux 已确认）。
- `update` 拉到的新版本是否仍兼容当前 `EquippedV5Response` 字段（只能升级后实机确认）。
---

## 11. 上游事实基线（InventorySimulator v3.7.0）

| 维度 | 事实 |
| --- | --- |
| 许可 / 活跃度 | MIT（`License.txt`，"(c) 2024 - present Ian Lucas"）；C#；130 stars / 28 forks / 0 open issues；`pushed_at` 2026-10-15T17:01:20Z；v3.7.0 与 CS2 10-15 更新同日发布 |
| 发布资产 | `InventorySimulator-v3.7.0.zip`，83,624 B，`sha256:a19705a7825ca68379cd09e2b25afdc2364d6947db3f3121311954224f573933` |
| 目标框架 | `net10.0`；`CounterStrikeSharp.API 1.0.375`（本仓现用 1.0.371） |
| 包内布局 | `addons/counterstrikesharp/plugins/InventorySimulator/{InventorySimulator.dll,.deps.json,.pdb,lang/{en,zh-Hans,pt-BR}.json}` + `addons/counterstrikesharp/gamedata/inventory-simulator.json` |
| 应用机制 | **不逐武器写属性**：注入式 inventory —— Hook `Natives.CCSPlayerInventory::GetItemInLoadout`（Post）+ 改 `CCSPlayerController.InventoryServices`，配合 `VirtualFunctions.GiveNamedItemFunc`（Pre）与 `OnEntityCreated/Spawned/Deleted` 处理实体侧，最后 `SendInventoryUpdateEvent()` 推送 |
| 触发点 | `player_connect`、`player_connect_full`、`player_spawn`（Post）、`player_death`（StatTrak）、`round_mvp`（音乐盒 StatTrak）、`player_disconnect` |
| 本地清单 | `{"<steamid64>": EquippedV5Response}`，查找路径见 §4 |
| 远程清单 | `GET {invsim_url}/api/equipped/v5/{steamid}.json`，默认 `https://inventory.cstrike.app`，30 s 超时 × 3 次重试 |
| 额外能力 | 涂鸦喷漆（`!spray`）、挂件、宠物、武器购买菜单 loadout、`!ws` 热刷新、Steam 登录（默认关闭） |
| 测试 | **无任何测试工程**；质量依赖作者与 `.github/workflows/ci.yml` 构建 |

---

## 12. 饰品字段（本仓模型 → `EquippedV5Response`）

本地模式生成 `inventories.json` 时按下表转换。**不经过 PKC schema**（D9：PKC 及其字段已弃用），直接从 Panel 现有饰品模型生成。

| 本仓模型（Panel `src/data` + 预设） | 上游字段 | 结论 |
| --- | --- | --- |
| 刀：`defindex` + `paint` + `seed` + `wear` + `name_tag` + `stattrak_*` + `stickers[]` | `knives: {team: InventoryItem}`（`def/paint/seed/wear/nametag/stattrak/stickers[]`） | 可直接映射 |
| 手套：`defindex` + `paint` + `seed` + `wear` | `gloves: {team: InventoryItem}` | 可直接映射 |
| 枪械：`weapon_defindex` + `paint` + `seed` + `wear` + StatTrak | `ctWeapons / tWeapons: {defindex: InventoryItem}` | 可直接映射 |
| 贴纸：`id` + `schema` + `offset x/y` + `wear` + `rotation` | `stickers[]: {def, schema, slot, x, y, wear, rotation}` | 一一对应 |
| 挂件：`placement_id`（原生 XYZ 在只读目录） | `keychains[]: {def, seed, slot, sticker, x, y, z}` | 需先把 placement 解析成 XYZ |
| 音乐盒：`def_index` | `musicKit: {musicId}` | 可直接映射 |
| 探员：模型路径 | `agents: {team: InventoryItem{def}}` | **语义不同**，需 model-path → defindex 映射 |
| （无对应） | `souvenir` | 上游无此字段，**不做**（D8） |

上游 `InventoryItem` 额外提供 `style`（Doppler 相位）、`tint`、`charges`、`upgradeLevel`、`uid`、`hash`、`petId` —— 对后续 UI 是纯增量。

> 脚本本期只负责“写入/切换文件”，**不生成字段内容**；上表是将来由 Panel 生成清单时的映射依据。

---

## 13. 能力对账

| 能力 | 旧 PKC（已弃用） | InventorySimulator | 备注 |
| --- | --- | --- | --- |
| 刀 / 手套 / 枪皮 / 磨损 / 种子 | 有 | 有 | |
| 贴纸（槽/偏移/磨损/旋转） | 有 | 有 | 契约相同 |
| 挂件（含 XYZ / seed） | 有 | 有 | 需先解析 placement |
| 名称标签 | 有 | 有 | |
| StatTrak | 有（写初值/计数） | 有（原生计数 + 可选上报） | 上报默认开启，见 §8.4 |
| 纪念品（Souvenir） | 有 | **无** | 不做（D8） |
| 探员 | 有（模型路径白名单） | 有（探员定义） | 语义不同 |
| 音乐盒 | 有（MVP 时应用） | 有（原生 music kit） | 上游更原生 |
| 武器购买菜单 loadout | 无 | 有（`invsim_loadout_enabled`） | 上游增量 |
| 涂鸦喷漆 / 宠物 | 无 | 有 | 上游增量 |
| 真人/机器人隔离 | 真人专属 | 真人专属（所有 handler 均 `!IsBot`） | `BotRandomizer` 不受影响 |
| 在线模式门控 | Panel 关闭并复原 | 同样需 Panel 管 | 不因替换而放宽 |

---

## 14. 取证索引

**上游（`ianlucas/cs2-css-inventory-simulator@main`，v3.7.0）**

- 元数据：`GET /repos/...` → MIT、C#、130 stars、`pushed_at 2026-10-15T17:01:20Z`
- 已读源码：`InventorySimulator.cs`（钩子注册）、`InventorySimulator.GameEvents.cs`、`Services/Api.cs`（`FetchEquippedAsync`、`CanSendPublicApiRequest`、令牌桶）、`Services/ConVars.cs`（全部 convar 与默认值）、`Services/Inventories.cs`（加载与候选路径）、`Models/CCSPlayerControllerState.cs`（`Inventory = Inventories.Get(steamId)` 初值）、`Models/EquippedV5Response.cs`、`Models/InventoryItem.cs`、`Models/StickerItem.cs`、`Models/KeychainItem.cs`、`Extensions/CCSPlayerControllerExtensions.cs`（`HandleConnect`/`RefreshInventory`/`FetchInventory`）、`InventorySimulator.csproj`、`License.txt`、`Readme.txt`、`gamedata/inventory-simulator.json`（26 条记录）
- 实机产物：`InventorySimulator-v3.7.0.zip` 解包布局已核对（见 §11）

**本仓**

- `docs/PLAYER-COSMETICS.md`（饰品语义参考；其中 PKC 部分已弃用，见 D9）、`docs/Local-Arena-v1.4.3.2-richer-cosmetics-compatibility.md`（Skin Forge 结论）
- `addons/counterstrikesharp/plugins/PlayerKnifeCustomizer/AGENTS.md`（历史实现，已弃用）
- `Panel/src/data/*`（饰品目录：`skinNames.json`、`stickerCatalog.json`、`gloveSkins.json`、`musicKits.json`、`charmCatalog.json`、`agentCatalog.json`、`cosmeticPlacements.json`）
- 实机：`PlusMatchCoordinator` 日志中的 `game directory=.../Counter-Strike Global Offensive/game`（§4 路径结论的依据）
- 脚本实测（2026-10-27）：用合成导出包（schema 2，含 CT 刀/手套/AK + 两张贴纸 + 一个挂件 + T 刀 + 音乐盒）跑通；输出经 `ConvertFrom-Json` 回读校验（`keychains` 为数组、`uid` 连续唯一）；两个负例（错误 kind、SteamID 非 17 位）均拒绝
