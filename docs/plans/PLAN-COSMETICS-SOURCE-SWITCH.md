# 计划：饰品插件（InventorySimulator）直连/本地双模式 + Panel 下载更新服务

状态：**计划稿，未实施**。决策来源：用户 2026-10-27 口头拍板（见 §2）。事实基线取证于 2026-10-27，含本机实机验证。配套文档：`docs/PLAYER-COSMETICS.md`（现有饰品语义）。原可行性评估 `docs/COSMETICS-PROVIDER-EVALUATION.md` 的有用内容已并入 §11–§14，该文件已删除。

---

## 1. 目标

1. Panel 提供一个开关：**官网直连**（默认）或**本地清单**。
2. 直连模式下，饰品完全由 `https://inventory.cstrike.app/` 决定，**Panel 不干预**（不生成、不读写 `inventories.json`）。
3. 本地模式下，Panel 读写 `inventories.json`，按 SteamID64 键控。
4. Panel 提供该插件的**下载、校验、安装、更新**服务；插件**不随包发布**。

---

## 2. 已拍板的决策

| # | 决策 | 值 |
| --- | --- | --- |
| D1 | SteamID64 来源 | **用户在 UI 里填写**（不读 `loginusers.vdf`） |
| D2 | 默认模式 | **直连**（`cosmetics_source = "remote"`） |
| D3 | 插件分发方式 | **不随包发布**，下载式安装 |
| D4 | 空 `inventories.json` 的行为 | 插件报错后**照常向远端拉取**（用户 2026-10-27 实机确认），因此空文件是"噪音 + 走远端"，不是致命错误 |

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

**写 cfg 的硬性约束**：`autoexec.cfg` 是用户文件（本机现有内容就是一行手工 `invsim_file`），**必须用 managed block 写入**，块外字节保留。本仓已有先例 `// managed_team_lineup_start ... end`（`lib.rs:773`）：

```
// managed_cosmetics_start
invsim_file "<mode-dependent>"
invsim_url "https://inventory.cstrike.app"
// managed_cosmetics_end
```

同时写 `invsim_url` 是刻意的：插件默认值就是官网，但如果用户 cfg 里曾改过，Panel 必须能自证"我确实是直连"。

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

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| **P0 收口现状**（不改代码，**已完成 2026-10-27**） | 已删除 `cfg/autoexec.cfg`（内含手工 `invsim_file` 行），空文件已不再由启动链引用；直连路径已由用户实机确认可用 | 日志出现一次成功 equipped 拉取；游戏内看到官网配置的饰品 |
| **P1 配置层** | `AppConfig.cosmetics_source` + managed block 读写器 + 单测（块外保留、切换幂等、行尾风格保持） | `cargo test` 新用例通过；连续切换两次后 `autoexec.cfg` 字节稳定 |
| **P2 本地模式** | SteamID64 输入校验（17 位、以 7656119 开头）+ `inventories.json` 原子写入（temp → fsync → rename）+ schema 版本字段 + 从 PKC schema 5 映射 | 写入后插件加载无异常；切回直连后文件被改名且块内 `invsim_file` 指向不存在路径 |
| **P3 下载服务** | `scripts/dependencies.json` 新增 `inventorySimulator` 钉版 + `inventory_plugin.rs`（status/check/download/install）+ 设置页卡片 | 断网给出可读错误；sha256 不符拒绝安装；更新只覆盖 `plugins/InventorySimulator/` 与 `gamedata/inventory-simulator.json`，不碰 `configs/` |
| **P4 打包与门禁** | `verify-workspace.ps1` 白名单、`plus-payload-manifest.json`、`installer.rs` 的 `PLUS_MARKERS`、`install_checks.rs`、i18n 两文件流程 | `scripts/linux/build.sh` 五步 PASS；`package.ps1 -Linux` 出包且布局断言通过 |

P0 今天可做且零代码；P1–P4 每阶段独立提交。

---

## 6. 下载与更新服务设计

**复用现有上游下载器**，不新写 HTTP 客户端：`upstream_package.rs` 已有 `resolve_latest` / `download_by_tag` / `ensure_cached` / `import_local`，以及 `.part` 续传、`aria2c` 可选、`reqwest + rustls-tls-native-roots`（已适配本机 GitHub 代理环境）。

1. **钉版**：`scripts/dependencies.json` 增加
   ```json
   "inventorySimulator": {
     "repository": "https://github.com/ianlucas/cs2-css-inventory-simulator",
     "release": "3.7.0",
     "asset": "InventorySimulator-v3.7.0.zip",
     "sha256": "a19705a7825ca68379cd09e2b25afdc2364d6947db3f3121311954224f573933"
   }
   ```
2. **命令面**（新增 `inventory_plugin.rs`，注册进 `generate_handler!`，并在 `Panel/src/lib/api.ts` 加对应 wrapper）：
   - `inventory_plugin_status()` → 已装版本、可用更新、安装位置、当前模式
   - `inventory_plugin_check(force)` → 查 release（GitHub API `assets[].digest` 直接比对 sha256，与现有上游校验同路）
   - `inventory_plugin_download(tag)` → 下到 `.cache/inventory-plugin/`，校验后才落地
   - `inventory_plugin_install(tag)` → 解压到 payload，**白名单只覆盖** `addons/counterstrikesharp/plugins/InventorySimulator/**` 与 `addons/counterstrikesharp/gamedata/inventory-simulator.json`
3. **不碰 `configs/`**：convar 配置仅在文件不存在时创建一次（含 `invsim_public_api_* = false`，避免断网刷错误日志）；存在则永不覆盖。
4. **版本边界**：只允许"已校验过的版本"；上游发新版且 sha256 未钉入时，UI 显示"未验证版本"，不自动升级。

---

## 7. 兼容矩阵（验收必须覆盖）

| 场景 | 期望 |
| --- | --- |
| 插件未安装 | 开关置灰 + "先安装饰品插件"；不报错 |
| 已装 + 直连 + 断网 | 加载无异常；拉取失败有日志；UI 说明"离线时无饰品" |
| 已装 + 本地 + 清单缺失 | Panel 立即生成合法清单（不写空文件） |
| 已装 + 本地 + 清单写坏 | 依 D4：插件报错后退回远端；Panel 应检测到非法 JSON 并提示 |
| 已装 + 直连 + 残留 `inventories.json` | 依 D4 实机观察：残留文件（含合法内容）在直连模式下仍会被 `Inventories.Load()` 读入，从而**劫持直连**。因此直连模式必须同时改名该文件，不能只改 `invsim_file` |
| 用户手工改过 `autoexec.cfg` | managed block 外逐字节保留（单测） |
| 上游删除旧资产 | 下载失败给出可读错误，不静默降级 |
| 在线模式（Online） | 沿用现有 `enforce_mode_cosmetics` 门控，不因本计划放宽 |

---

## 8. 风险

1. **上游契约漂移**：`invsim_file` 语义、候选路径、`EquippedV5Response` 字段都可能在 v4 变化 → 钉版 + 版本白名单 + 升级前 diff 检查。
2. **`Server.GameDirectory` 语义**：本机是 `.../game`，但不能假定所有用户一致（Windows/Proton 可能不同）→ 落盘路径用"探测 + 三候选"策略，与本仓 `PlusManagedPaths.CandidateCsgoRoots` 同思路，而不是硬编码。
3. **SteamID64 手填错误**：17 位、以 `7656119` 开头、校验位无法验证 → 写入后做"是否等于当前登录账号"的软校验（可用 Steam Web API 或本地 `loginusers.vdf` 只读比对，仅提示不阻断）。
4. **直连模式仍依赖官网可用性**：`Api.cs` 有 50 容量/3.6 秒补给的本地令牌桶，且离线时 GET 会重试 3 次（30 秒超时）→ UI 需明确"断网 = 无饰品"，不做自动重试放大。
5. **`-insecure` 边界**：文案不得暗示"直连官网 = 安全对局"。
6. **功能回退：纪念品（Souvenir）**：上游 `InventoryItem` 无该字段，PKC 的 `souvenir_enabled` 无处落地 → 要么放弃，要么给上游提 PR/本地补丁（后者与 D3 的"不 fork"取向冲突）。
7. **探员语义差异**：PKC 存模型路径白名单，上游要探员 defindex → P2 需 model-path → defindex 映射表，并定义"目录里没有该探员"的降级行为。
8. **签名面更大**：上游 gamedata 有 26 条记录（约 20 个函数签名 + 6 个结构体偏移），跨 `server` 与 `engine2` 两个库，比 PKC 的单一 `SetAttrByName` 宽得多；好处是走 CSS 标准 gamedata 机制、可热修，且 v3.7.0 与 10-15 更新同日发布，维护响应有证据。
9. **单维护者 + 无测试**：上游无任何测试工程，质量靠作者与 CI 构建 → 依赖"钉版 sha256 + 安装后自检"兜底，不直接把上游 zip 透传给用户。
10. **CSS 版本差**：本仓 1.0.371 vs 上游编译用 1.0.375 → P3 需在安装前校验 CSS 版本，不匹配时给可读提示而不是运行期崩。
11. **许可**：MIT 与本仓 AGPLv3 兼容，分发需保留上游版权声明与许可文本（`License.txt`）。

---

## 9. 与既有实现的衔接点（实施时逐条核对）

| 位置 | 改动 |
| --- | --- |
| `Panel/src-tauri/src/lib.rs:104` `AppConfig` | 新增 `cosmetics_source` |
| `Panel/src-tauri/src/lib.rs` `generate_handler!`（约 :4508） | 注册 4 个新命令 |
| `Panel/src/lib/api.ts` | 对应 4 个 invoke wrapper |
| `Panel/src-tauri/src/installer.rs:22-26` `PLUS_MARKERS` | 把 PKC 标记换成 InventorySimulator 标记（PKC 已从实机移除） |
| `Panel/src-tauri/src/installer.rs` `SUITE_OWNED_FILES` | 加入 `configs/plugins/InventorySimulator/*.json`（用户配置保护） |
| `scripts/dependencies.json` | 新增 `inventorySimulator` 钉版 |
| `scripts/verify-workspace.ps1`（addons 白名单，:97 起） | 加入新插件目录与 gamedata 文件 |
| `scripts/package.ps1`（payload manifest walk） | 确认不把插件打进包（D3） |
| `Panel/src-tauri/src/install_checks.rs:255` | 安装健康检查增加插件存在性 |
| `Panel/src/i18n/keys.ts` + `dictionary.ts` | 新模式/新错误码（两文件流程） |

---

## 10. 未验证项（显式标注）

- **直连模式下残留 `inventories.json` 的优先级**：2026-10-27 实机观察到 `invsim_file` 已随 `autoexec.cfg` 删除、但 `inventories.json` 内容仍被加载——说明默认值 `inventories.json` 仍在生效，且文件存在时优先于远端。这直接支持 §3 的"直连模式必须改名该文件"设计；但"合法清单残留时是否仍静默劫持"尚无逐帧证据，实施 P1 时需复验。

- 直连模式在**真实对局**中的稳定性（用户已确认能拉到数据，但长对局/换图/重连的表现未记录）。
- 本地模式写入后，插件在"清单合法但 SteamID 未命中"时的回退行为（代码上会走 HTTP，实机未验）。
- Windows/Proton 下 `Server.GameDirectory` 的叶子是否为 `game`（本机 Linux 已确认）。

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

## 12. 契约映射（PKC schema 5 → `EquippedV5Response`）

本地模式（P2）生成 `inventories.json` 时按下表转换；直连模式不涉及。

| PKC 字段 | 上游字段 | 结论 |
| --- | --- | --- |
| `loadouts.<ct\|t>.knife_presets[defindex]`（paint/seed/wear/name_tag/stattrak_*/stickers[]） | `knives: {team: InventoryItem}`（`def/paint/seed/wear/nametag/stattrak/stickers[]`） | 可直接映射 |
| `loadouts.<ct\|t>.glove` | `gloves: {team: InventoryItem}` | 可直接映射 |
| 枪械预设 + `shared_weapon_links` | `ctWeapons / tWeapons: {defindex: InventoryItem}` | 可直接映射（Panel 先解析 shared links） |
| 贴纸（`id/schema/offset x/offset y/wear/rotation`） | `stickers[]: {def, schema, slot, x, y, wear, rotation}` | 一一对应 |
| 挂件（Panel 只存 `placement_id`） | `keychains[]: {def, seed, slot, sticker, x, y, z}` | 兼容：Panel 先把 placement 解析成 XYZ |
| `music_kit_id` | `musicKit: {musicId}` | 可直接映射 |
| `loadouts.<ct\|t>.agent_model`（模型路径） | `agents: {team: InventoryItem{def}}` | **语义不同**，需 model-path → defindex 映射 |
| `souvenir_enabled` | 无字段 | **能力缺失** |
| `apply_to_human_players` | 无（按 SteamID） | 由 D1 已决定为单机语义 |

上游 `InventoryItem` 额外提供 `style`（Doppler 相位）、`tint`、`charges`、`upgradeLevel`、`uid`、`hash`、`petId` —— 对 Panel UI 是纯增量。

---

## 13. 能力对账

| 能力 | PKC | InventorySimulator | 备注 |
| --- | --- | --- | --- |
| 刀 / 手套 / 枪皮 / 磨损 / 种子 | 有 | 有 | |
| 贴纸（槽/偏移/磨损/旋转） | 有 | 有 | 契约相同 |
| 挂件（含 XYZ / seed） | 有 | 有 | Panel 需先解析 placement |
| 名称标签 | 有 | 有 | |
| StatTrak | 有（写初值/计数） | 有（原生计数 + 可选上报） | 上报默认开启，见 §8.3 |
| 纪念品（Souvenir） | 有 | **无** | 功能回退 |
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

- `docs/PLAYER-COSMETICS.md`（现有饰品语义）、`docs/Local-Arena-v1.4.3.2-richer-cosmetics-compatibility.md`（Skin Forge 结论）
- `addons/counterstrikesharp/plugins/PlayerKnifeCustomizer/AGENTS.md` 与其 `player_knife_presets.json`
- `Panel/src-tauri/src/{app_storage.rs,installer.rs}`、`Panel/src/data/*`
- 实机：`PlusMatchCoordinator` 日志中的 `game directory=.../Counter-Strike Global Offensive/game`（§4 路径结论的依据）
