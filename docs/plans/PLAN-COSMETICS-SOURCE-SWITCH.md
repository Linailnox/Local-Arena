# 计划：饰品插件（InventorySimulator）直连/本地双模式 + 直接编辑 inventories.json

状态：**已实现（2026-10-27）**。决策来源：用户 2026-10-27 拍板。本文档记录实现后的最终形态，与代码保持一致；未验证项见 §10。

相关提交：`f30bbbf`（迁移脚本）→ `26af8e2`（删除 PKC + 存储后端切换）→ `bcda7d9`（SteamID 设置）→ `d99207c`（前端 B2 改造）。

---

## 1. 目标

1. Panel 提供来源开关：**官网直连**（默认）或**本地清单**。
2. 直连模式下饰品完全由 `https://inventory.cstrike.app/` 决定，Panel 不写清单。
3. 本地模式下 Panel **直接编辑** `inventories.json`（不是导出、不是镜像）。
4. 插件由用户/脚本安装与升级，**不随包发布**。

---

## 2. 已拍板的决策

| # | 决策 | 值 | 状态 |
| --- | --- | --- | --- |
| D1 | SteamID64 来源 | 用户在 UI 里填写 | 已实现（来源卡片输入框 + 17 位校验） |
| D2 | 默认模式 | 直连（`cosmetics_source` 缺省 `remote`） | 已实现 |
| D3 | 插件分发 | 不随包发布 | 已实现（`package.ps1` 不含该插件） |
| D4 | 空 `inventories.json` | 插件报错后退回远端 | 实机确认；Panel 侧拒绝写空文件 |
| D5 | 写 `autoexec.cfg` | **不实现**，仅作备选记录（§3.1） | 未实现（有意） |
| D6 | `invsim_url` | **不写入** | 未实现（有意） |
| D7 | SteamID64 诊断 | 不做"是否等于当前账号"的软校验 | 已按此实现 |
| D8 | Souvenir | 丢弃，等上游支持 | 已实现（`_panel` 不含该字段） |
| D9 | PKC 及其字段 | 全部弃用 | 已实现（插件、测试、构建链删除） |
| D10 | 前端模型 | **直接用 inventories.json 形状**（B2） | 已实现 |
| D11 | Panel 专有字段 | 存进 `_panel` 命名空间 | 已实现（插件忽略未知字段） |

---

## 3. 开关怎么实现（已实现，2026-10-29 修订）

插件在 `Load()` 里就读取 `invsim_file`，而该调用会走到 `Utilities.GetPlayers()`——此时 CounterStrikeSharp 的 natives 尚未初始化，**默认路径上只要有一份合法清单，整个插件加载就会失败**（3.7.0 与 3.7.2 该段代码相同）。

因此清单不能放在默认路径。当前设计：

| 模式 | 磁盘状态 | 代码 |
| --- | --- | --- |
| 本地 | 清单在 `.csbip/cosmetics/inventories.json`；`cfg/autoexec.cfg` 的托管块把 `invsim_file` 指向它；默认路径清空 | `cosmetics_source::write_local` |
| 直连 | 删除托管块，默认路径同样清空 | `cosmetics_source::switch_to_remote` |

托管块追加在文件末尾，块外字节原样保留：

```
// local-arena-cosmetics:start
invsim_file "<绝对路径>/.csbip/cosmetics/inventories.json"
// local-arena-cosmetics:end
```

之所以可行：autoexec 在插件加载之后执行，此时 `invsim_file` 被赋值触发 `ValueChanged` -> `OnFileChanged`，走的是运行期路径，natives 已就绪。

危险路径（两种模式都必须清空，存在即改名 `.bak`）：

```
<csgo>/addons/counterstrikesharp/configs/plugins/InventorySimulator/inventories.json
<csgo>/inventories.json
```

`inspect()` 返回 `effective` / `selected` / `mismatch` / `autoexec_linked`：只有"清单合法 **且** 托管块指向它"才算本地模式生效。

### 3.1 已知限制

该方案依赖"autoexec 晚于 CSS 插件加载"这一时序。本机（listen server）成立；若某环境的 cfg 早于插件加载执行，仍会崩。彻底修复仍需上游把第 47 行的初次调用延后到 `Server.NextWorldUpdate` / `OnMapStart`，或给第 84 行加 `try/catch`。

---

## 4. 路径对照

| 用途 | 路径 |
| --- | --- |
| Panel 写入的清单 | `<csgo>/.csbip/cosmetics/inventories.json` |
| 托管块所在文件 | `<csgo>/cfg/autoexec.cfg`（仅 `local-arena-cosmetics` 块） |
| 必须保持为空的默认路径 | `<csgo>/addons/counterstrikesharp/configs/plugins/InventorySimulator/inventories.json`、`<csgo>/inventories.json` |

`Server.GameDirectory` 本机为 `.../Counter-Strike Global Offensive/game`（证据：PlusMatchCoordinator 实机日志）。

---

## 5. 分阶段落地（全部完成）

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| P0 现状收口 | 删除手工 `autoexec.cfg` 与空文件；直连可用 | 用户实机确认 |
| P1 迁移脚本 | `scripts/migrate-cosmetics-to-inventory.ps1`：旧导出包 → `inventories.json` | 合成样例 + 3 个负例通过 |
| P2 删除 PKC | 插件 1731 行 + 测试 278 行 + 4 份目录；`build.ps1`/`build.sh`/`package.ps1`/`verify-workspace.ps1`/`installer.rs` 同步清理 | `cargo` PASS（164 通过 / 7 基线）、`verify` PASS |
| P3 来源开关 | `cosmetics_source.rs`（路径判定、切换、`mismatch`）+ 7 个单测 + `CosmeticsSourceCard` UI | 单测通过；UI 显示 effective/selected |
| P4 直接编辑 | `cosmetics_inventory.rs` 双向映射 + `_panel` 侧车 + 前端 B2 模型 | 5 个单测；`bun run build` 0；`test:stickers` 0 |

---

## 6. 代码接口（当前实现）

### 6.1 脚本（独立，不依赖 Panel）

```
pwsh scripts/migrate-cosmetics-to-inventory.ps1 -Source <cs2bip-cosmetics-preset.json> \
     -SteamId <7656119...> [-Out inventories.json] [-DryRun]
```

把旧的 `cs2bip-cosmetics-preset` 导出包转换为 `inventories.json`：刀/手套/枪/贴纸/挂件/音乐盒；贴纸槽坐标取自 `cosmeticPlacements.json`；`souvenir` 丢弃并计数提示；`paint<=0` 跳过；拒绝写 0 字节文件。

### 6.2 Panel 命令（Rust）

| 命令 | 作用 |
| --- | --- |
| `get_cosmetics_source(csgo)` | 返回 `effective`/`selected`/`plugin_installed`/`files`/`mismatch` |
| `set_cosmetics_source(csgo, source)` | `remote` = 把清单改名 `.bak`；`local` = 要求已存在合法清单 |
| `set_cosmetics_steam_id(steam_id)` | 记录 17 位 SteamID64（空字符串 = 清除） |
| `get_cosmetics_inventory(csgo)` | 返回 `{ steam_id, inventory, panel, plugin_present, config_present, cs2_running }` |
| `save_cosmetics_inventory(csgo, steam_id, inventory, panel)` | 原子写入文档（`_panel` 由后端注入） |
| `export_cosmetics_inventory` / `import_cosmetics_inventory_file` | 导出/导入 `inventories.json` 本身 |

### 6.3 前端模型（B2）

```ts
InventoryItem   = { def?, paint?, seed?, wear?, stattrak?, nametag?, uid?, stickers?, keychains?, musicId? }
PlayerInventory = { knives?, gloves?, ctWeapons?, tWeapons?, agents?, musicKit? }
PanelCosmeticsSettings = { enabled, apply_*, stickers_enabled, charms_enabled, agents_enabled,
                           shared_weapon_links, default_knife }
```

队伍键：**CT = "3"，T = "2"**。辅助函数集中在 `Panel/src/lib/cosmeticsInventory.ts`。

`_panel` 侧车承载上游模型没有的 Panel 字段：`enabled`、`apply_*`、`stickers_enabled`、`charms_enabled`、`agents_enabled`、`shared_weapon_links`、`default_knife`。插件忽略未知成员，因此可以安全共存。

---

## 7. 兼容矩阵

已实现（脚本/后端，可实测）：

| 场景 | 行为 |
| --- | --- |
| 切到直连但存在清单 | 两条路径上的文件都改名 `.bak` |
| 切到本地但没有合法清单 | 拒绝并提示先运行迁移脚本 |
| 写入空文档 | 拒绝（插件会因此报错） |
| 文档缺该 SteamID | 拒绝并报错 |
| `uid` | 从 900000 起连续唯一 |
| `paint<=0` | 不写入 |
| `souvenir_enabled` | 丢弃（D8） |
| 探员 | 不写入 `agents`（目录无 defindex） |

运行期（仍需实机验证）：

| 场景 | 期望 |
| --- | --- |
| 直连 + 断网 | 加载无异常；无饰品 |
| 本地 + 清单合法但 SteamID 未命中 | 插件自行走 HTTP |
| 在线模式 | 沿用 `enforce_mode_cosmetics` 门控 |

---

## 8. 风险

1. **上游契约漂移**：`EquippedV5Response` 字段变化会破坏映射 → 升级插件后重跑一次导出/导入对比。
2. **`Server.GameDirectory` 语义**：本机是 `.../game`；Windows/Proton 可能不同 → 路径经 `gameinfo.gi` 探测，不硬编码。
3. **SteamID64 手填错误**：不做账号一致性诊断（D7）；写错的表现是"本地模式不生效"。
4. **直连依赖官网可用性**：离线即无饰品；`invsim_public_api_*` 默认开启会在断网时刷错误日志。
5. **`-insecure` 边界**：文档不得暗示"直连官网 = 安全对局"。
6. **签名面**：上游 gamedata 26 条记录，跨 `server` 与 `engine2`；游戏更新可能失效，但可热修 gamedata。
7. **单维护者 + 无测试**：上游无测试工程 → 依赖固定版本与安装后自检。
8. **CSS 版本差**：本仓 1.0.371 vs 上游编译 1.0.375。

---

## 9. 与既有实现的衔接点（已完成）

| 位置 | 改动 |
| --- | --- |
| `addons/.../PlayerKnifeCustomizer/` + `.Tests/` | 删除 |
| `scripts/build.ps1`、`linux/build.sh` | 不再编译 PKC 与其测试 |
| `scripts/package.ps1` | 不打包 PKC；不再保护 `player_*_presets.json` |
| `scripts/verify-workspace.ps1` | 移除 34 处 PKC 断言；保留 Panel 目录计数校验 |
| `Panel/src-tauri/src/installer.rs` | `PLUS_MARKERS`、`SUITE_OWNED_*`、镜像与安装事务测试改指 InventorySimulator |
| `Panel/src-tauri/src/app_storage.rs` | `mirror_cosmetics` 镜像 `inventories.json` |
| `Panel/src-tauri/src/lib.rs` | 新增 6 个命令；`read/save_knife_config` 走文档 + 映射；删除 4 个 PKC sanitize 函数 |
| `Panel/src/lib/api.ts`、`lib/cosmeticsInventory.ts`、`lib/stickerEditor.ts` | 新模型与辅助层 |
| `Panel/src/panels/*`、`components/CosmeticsSourceCard.tsx`、`components/CosmeticsPresetActions.tsx` | 改用文档模型；导出/导入语义改为 `inventories.json` |

---

## 10. 未验证项（显式标注）

- **实机闭环**：Panel 写出的 `inventories.json` 在真实 CS2 中是否按预期生效（需要客户端）。
- 本地模式下"清单合法但 SteamID 未命中"的实际回退表现（代码上走 HTTP）。
- Windows/Proton 下 `Server.GameDirectory` 的叶子是否为 `game`（Linux 已确认）。
- 上游新版本是否仍兼容当前 `EquippedV5Response` 字段。

---

## 11. 上游事实基线（InventorySimulator v3.7.0）

| 维度 | 事实 |
| --- | --- |
| 许可 / 活跃度 | MIT；C#；130 stars / 28 forks / 0 open issues；`pushed_at` 2026-10-15T17:01:20Z |
| 发布资产 | `InventorySimulator-v3.7.0.zip`，83,624 B，`sha256:a19705a7825ca68379cd09e2b25afdc2364d6947db3f3121311954224f573933` |
| 目标框架 | `net10.0`；`CounterStrikeSharp.API 1.0.375`（本仓 1.0.371） |
| 包内布局 | `addons/counterstrikesharp/plugins/InventorySimulator/{dll,deps.json,pdb,lang/*}` + `gamedata/inventory-simulator.json` |
| 应用机制 | 注入式 inventory：Hook `CCSPlayerInventory::GetItemInLoadout` + 改 `InventoryServices`，配合 `GiveNamedItem` 前置与实体监听器 |
| 本地清单 | `{"<steamid64>": EquippedV5Response}`，查找路径见 §4 |
| 远程清单 | `GET {invsim_url}/api/equipped/v5/{steamid}.json`，默认 `https://inventory.cstrike.app`，30 s × 3 次重试 |
| 测试 | 无；质量依赖作者与 CI 构建 |

---

## 12. 饰品字段映射（已实现于 `cosmetics_inventory.rs`）

| Panel 模型 | 上游字段 |
| --- | --- |
| 刀 `knives[team]` | `InventoryItem{def,paint,seed,wear,nametag,stattrak,stickers,keychains}` |
| 手套 `gloves[team]` | 同上（存在即启用） |
| 枪 `ctWeapons/tWeapons[defindex]` | 同上 |
| 贴纸 | `stickers[]{def,slot,schema,x,y,wear,rotation}` |
| 挂件 | `keychains[]{def,seed,slot,x,y,z}`（`placement_id` 经 `cosmeticPlacements.json` 解析） |
| 音乐盒 | `musicKit{musicId}` |
| Panel 专有 | `_panel{enabled,apply_*,stickers_enabled,charms_enabled,agents_enabled,shared_weapon_links,default_knife}` |

---

## 13. 能力对账

| 能力 | 旧 PKC（已删除） | InventorySimulator | 备注 |
| --- | --- | --- | --- |
| 刀 / 手套 / 枪皮 / 磨损 / 种子 | 有 | 有 | 每队一把刀 |
| 贴纸（槽/坐标/磨损/旋转） | 有 | 有 | 无 `scale` |
| 挂件（XYZ / seed） | 有 | 有 | 需解析 placement |
| 名称标签 / StatTrak | 有 | 有 | |
| 纪念品 | 有 | **无** | D8 丢弃 |
| 探员 | 有（模型路径） | 有（defindex） | 本仓目录无 defindex → 不写 |
| 音乐盒 | 有 | 有 | |
| 涂鸦 / 宠物 / 购买菜单 loadout | 无 | 有 | 上游增量 |
| 真人/机器人隔离 | 真人专属 | 真人专属 | `BotRandomizer` 不受影响 |

---

## 14. 取证索引

**上游**（`ianlucas/cs2-css-inventory-simulator@main`，v3.7.0）：`InventorySimulator.cs`、`GameEvents.cs`、`Services/Api.cs`、`Services/ConVars.cs`、`Services/Inventories.cs`、`Models/*`、`Extensions/CCSPlayerControllerExtensions.cs`、`gamedata/inventory-simulator.json`、`License.txt`、`Readme.txt`。

**本仓**：`docs/PLAYER-COSMETICS.md`、`Panel/src-tauri/src/{cosmetics_source.rs,cosmetics_inventory.rs,installer.rs,app_storage.rs}`、`Panel/src/lib/{api.ts,cosmeticsInventory.ts,stickerEditor.ts}`、`scripts/migrate-cosmetics-to-inventory.ps1`、`scripts/test-sticker-editor.mjs`。

**实机**：`PlusMatchCoordinator` 日志 `game directory=.../Counter-Strike Global Offensive/game`（§4 依据）；`InventorySimulator` 日志 2026-10-27 的 `/mnt/game/inventories.json` 报错（空文件，D4 依据）。
