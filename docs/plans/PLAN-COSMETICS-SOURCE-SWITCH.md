# 计划：饰品插件（InventorySimulator）直连/本地双模式 + Panel 下载更新服务

状态：**计划稿，未实施**。决策来源：用户 2026-10-27 口头拍板（见 §2）。事实基线取证于 2026-10-27，含本机实机验证。配套文档：`docs/COSMETICS-PROVIDER-EVALUATION.md`（可行性评估）、`docs/PLAYER-COSMETICS.md`（现有饰品语义）。

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
