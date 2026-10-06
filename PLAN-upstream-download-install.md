# 计划：next 分支 — 移除上游 release 打包，改为 Panel 下载安装

> 状态：已按外部评审修订（未开始实施）
> 日期：2026-10-04 初稿；2026-10-05 评审修订
> 分支：`next`（基于 `f5f1f73`，= `update` 已推送状态）
> 审阅重点标注：**★关键设计** = 实施前必须确认的设计点；**⚠风险** = 可能出错的地方
>
> **2026-10-05 评审修订记录**（标记为〔评审〕处）：
> ① P1：§6.3 `payload_root()` 切换表补全（新增 3095/3097 restore、3122 inspect_quick、2991/3024
>    install/repair impl），"全部切换"表述改为逐行可核对的全量表；
> ② P2：决策 #4 改写 —— `backup/**gameinfo.gi` 包内保留、**排除出合并 manifest（不安装）**，
>    避免"装入即被 cleanup 删除 → 修复循环"；§6.2 步骤 5 改为前缀白名单 walk；
> ③ P3：决策 #5 升级 —— API 路径校验 `assets[].digest`（回退路径维持不校验）；R1 重写；
> ④ P4：更正 installer.rs 行号（59/73）、i18n 语言块数（28）、补 `pub(crate)` 可见性与
>    pwsh 路径环境说明。
>
> **2026-10-05 二轮遗留修订**：
> ⑤ 决策 #4 的启动路径引用更正 `lib.rs:1381` → `1213 launch_cs2` / `1135 set_mode`
>    （1381 属 `restore_demo_layout`，非启动路径）；§6.3 的 `ownership=="plus"` 过滤行号
>    `1317` → `1324`（1317 是 manifest 读取行）；
> ⑥ §6.2 新增 **merge 入口阻断式 payload 探测 + 平铺**（对齐 `package.ps1:141-147`），
>    防上游将来把内容包进一层顶层目录时静默生成 LA-only 合并 manifest；R5 与单测清单同步。

---

## 0. 背景与目标

### 0.1 现状（as-is）

今天的发布链路（`scripts/package.ps1`）：

1. CI 下载 **5 类钉版本资产**（全部走 `Get-VerifiedAsset`，SHA-256 校验）：
   - `CS2BotImprover.zip`（上游 v1.4.5，73MB，`.cache/package/CS2BotImprover.zip`）
   - `mmsource-2.0.0-git1406-windows.zip`（MetaMod）
   - `counterstrikesharp-with-runtime-windows-1.0.376.zip`（CSS + .NET runtime）
   - `RayTrace-CSS-API-v1.0.16.tar.gz` + `RayTrace-MM-v1.0.16-windows.tar.gz`
   - `BotHider-Windows-0.4.0.zip`
2. 解压上游 zip 作为 **payload 基底**（`package.ps1:125,150`），其中已含上游的
   `Panel v1.4.5.exe`、8 个预编译插件 DLL、17 个 gamemode cfg、4 个 botprofile.vpk、
   `backup/`、3× `gameinfo.gi`、CSS/metamod/BotHider 运行时。
3. 用 `Expand-TarGz` / `Expand-Archive` 的 metamod、CSS、RayTrace、BotHider 内容
   **覆盖**进 payload（`package.ps1:174-207`）。
4. 先跑 `build.ps1`（`package.ps1:102`）本地编译 9 个上游系插件，
   再 `Copy-Tree` **强删覆盖**上游 zip 内的同名预编译 DLL（`package.ps1:236-246`）。
   实测证据：`BotAI.dll` 上游 37,888 字节 `04881463…` vs 本地 39,424 字节 `9da33f8e…`
   （源码同为钉版子模块，编译环境不同 → 字节不同）。
5. 清理：删 `Panel*.exe`（151）、**递归**删所有 `gameinfo.gi`（156）、
   VPK 归一化 `ConvertTo-BotProfileOnlyVpk`（165-172）。
6. LA 独有覆盖：BotHider configs（210-212）、`my_bot_ffa/normal` cfg（213-214）、
   NadeSystem 手雷目录（218）、`overrides/scripts` 行为树（220）。
7. 生成 `plus-payload-manifest.json`（365-406）：path/size/sha256/component/ownership/restore_policy，
   top-level 仅 `addons|cfg|overrides`。
8. 打 3 个 zip：`LocalArena-vX-windows.zip`（完整包）、`LocalArena-panel-…`、`LocalArena-plugin-…`，
   外加签名的 `latest.json`（445-476）。

Panel 侧安装：`installer::plan/install/inspect/restore`（`installer.rs:867/957/638/1250`）
以 `payload_root()` 为源（`lib.rs:903` = exe 所在目录，或 `online_update::active_payload_root()`），
按 manifest 逐条 copy + backup + transaction。

### 0.2 目标（to-be）

- **LA 完整包只含自有内容**：`LocalArena.exe`、`WebView2Loader.dll`、README/README.zh-CN/LICENSE、
  4 个独占插件（PlayerKnifeCustomizer、PlusMatchCoordinator、TeamLineupInjector、OfflineMatchTelemetry）、
  LA 改过的 `cfg/my_bot_ffa_config.cfg` + `cfg/my_bot_normal_config.cfg`、`overrides/scripts/`、
  `plus-payload-manifest.json`。预计从 73MB+ 缩到几 MB。
- **安装/更新时由 Panel 从 ed0ard GitHub release 下载 `CS2BotImprover.zip`**（仅 HTTPS + GitHub API `digest` 校验，回退路径无校验），
  解压 → 过滤 → 叠加 LA payload → 生成**合并 manifest** → 走现有安装管线。
- 新增**安装管理界面**（Panel 内）：下载/缓存管理、手动指定本地 zip、已安装版本状态、页内触发安装/修复、
  可选启动随包而来的上游 Panel exe。
- RayTrace **全链路移除**（上游 v1.4.5 已不再分发 RayTrace 主体，仅剩 34 字节
  `addons/metamod/RayTrace.vdf` 残留，内容是 `github.com/ed0ard/CS2-Bot-Improver` 纯文本）。

---

## 1. 决策清单（全部已确认）

| # | 项 | 决定 |
|---|---|---|
| 1 | 剔除范围 | **A+C**：LA 包只有自有内容；metamod/CSS/BotHider/8 个上游系插件全部跟随上游；CI 不再下载钉版本依赖、拆掉 8 个插件本地构建 |
| 2 | RayTrace | **跟随上游移除**（package.ps1、verify-workspace、installer.rs 安装标记、mode_layout、install_checks、i18n） |
| 3 | botprofile.vpk | **直接采用上游原版**，不做 localization 剥离与加速度归一化；`ConvertTo-BotProfileOnlyVpk` 调用删除 |
| 4 | gameinfo.gi | **仅剔除根目录**那份（会覆盖游戏本体 Steam 校验文件的）；`backup/Online|WithBots/gameinfo.gi` **保留在合并 staging（包内），但排除出合并 manifest → 不安装进游戏目录**。〔2026-10-05 评审后修订，原表述"保留安装"作废〕依据：装入后会被 `mode_files.rs:144 cleanup_legacy_mode_backups` 在每次 `apply_launch_mode` 末尾删除，
而该函数经 `lib.rs:1213 launch_cs2`、`lib.rs:1135 set_mode` 等路径在启动/切模式时必经 → `inspect` 报 missing → 修复回装 = **永久修复循环**；且 `package.ps1:153-155` 注释明令不打包这两份备份，全仓 grep 无读取方。模式切换仍走现有 `rewrite_gameinfo`（以玩家当前游戏文件为基础插入/移除 metamod + botprofile.vpk 搜索路径），**无需改动** |
| 5 | sha256 校验 | **走 GitHub API 时校验 `assets[].digest`**〔2026-10-05 评审后升级，原"仅 HTTPS 不校验"作废〕：实测 v1.4.5 的 digest 与 `dependencies.json` 钉版 sha256 完全一致，校验零维护成本；403/429 回退 `latest/download` 重定向**无 digest** → 该路径维持仅 HTTPS；手动导入本地 zip 不校验 |
| 6 | 上游版本获取 | **自动解析 GitHub API**：`GET https://api.github.com/repos/ed0ard/CS2-Bot-Improver/releases/latest` |
| 7 | 更新流程 | **也改为"下载上游 + 叠加"**（plugin 组件更新 = 确保上游 zip 缓存 → 合并 → 安装） |
| 8 | 上游 `Panel v1.4.5.exe` | **移入 LocalArena.exe 所在目录**，安装管理页可选启动 |
| 9 | 仓库剔除 | 删除与上游重复的仓库文件 + 删除不再构建的子模块 + 删除 RoundDamageRecap 源码 + 删除 `plugins/disabled/` 树 + 删除 BotVision 子模块 |
| 10 | 安装管理界面 | 四个功能全选：上游包下载与缓存管理、手动指定本地 zip、显示已安装版本状态、页内触发安装/修复 |

---

## 2. 关键事实核对（全部已实测，实施时可复验）

| 事实 | 结论 |
|---|---|
| `cfg/my_bot_ffa_config.cfg` vs 上游 | **不同**：LA 版多 `bh_namesource 1`（+注释），`bot_eco_limit 2800` vs 上游 `2800.000000` → **LA 保留覆盖** |
| `cfg/my_bot_normal_config.cfg` vs 上游 | **不同**：同上多 `bh_namesource 1` → **LA 保留覆盖** |
| `cfg/gamemode_*.cfg`（15 个）vs 上游 | 仅行尾差异（`retakecasual` 上游反而多一行 `sv_annotation_limits_max_rounds_per_half -1`，即上游更新）→ **仓库删除，包内交上游** |
| `cfg/bot_buy.cfg`、`my_bot_rush_config.cfg` | 与上游**完全相同** → 仓库删除 |
| `cfg/*_rules_unchanged.cfg`（3 个） | 上游无；无代码引用 → 保留（假设 #1） |
| `data/NadeSystem/grenades` 48 文件 vs 上游 `plugins/NadeSystem/grenades` | 47 个仅行尾差异 + `de_dust2_molotov.json` 仅末尾换行差异 = **实质全部相同** → 仓库删除，覆盖步骤删除 |
| `bot_info.json`/`gamedata.json`（BotHider 子模块 configs） vs 上游 zip | **逐字节相同** → 删覆盖拷贝步骤，**不改子模块内容**（改了会破坏上游钉版比对） |
| 4 个独占插件的编译依赖 | PlusMatchCoordinator、TeamLineupInjector 引用 `BotHider/csharp/BotHiderApi`（ProjectReference）+ `shared/MatchCore`；PlayerKnifeCustomizer、OfflineMatchTelemetry 无上游依赖 → **BotHider 子模块必须保留** |
| 上游 zip 根目录 | `Panel v1.4.5.exe`、`addons/`、`backup/`、`cfg/`、`gameinfo.gi`、`overrides/` |
| 上游 zip 内 4 个 LA 独占插件 | **完全不存在**（grep 无匹配）→ 叠加顺序天然无冲突 |
| 上游 zip `shared/` | 只有 `0Harmony`、`BotControllerApi`、`BotHiderApi`、`README.md`，**无 MatchCore** → MatchCore 必须随 PlusMatchCoordinator 的 build 输出（`bin/Release/net10.0/MatchCore.dll` 实测已在其中）进包 |
| manifest 生成范围 | 只 walk `addons|cfg|overrides`（`package.ps1:365`），所以根 `gameinfo.gi`/Panel exe/`backup/` 本来就不进 manifest —— 但 zip 供手动解压时根 gameinfo 会覆盖游戏文件，**解压步骤必须显式跳过**；合并 manifest 沿用同一前缀白名单（只 walk `addons|cfg|overrides`）→ `backup/**` 天然排除（决策 #4 修订） |
| `installer::install` | 只按 `manifest.entries` 拷贝（`installer.rs:1006` 循环）→ 上游文件必须进合并 manifest 才会被安装/备份/恢复 |

---

## 3. 阶段 1：打包脚本瘦身（`scripts/`）

### 3.1 `scripts/package.ps1`（488 行 → 预计 ~250 行）

**删除块（按行号，实施时以语义为准）：**

| 行 | 内容 |
|---|---|
| 29 | `. VpkTools.ps1`（VPK 归一化删掉后无引用；若 verify 仍用 `Get-VpkEntryPaths` 则保留 dot-source 到阶段 3 一并决定） |
| 106-111 | 5 路 `Get-VerifiedAsset`（upstream/metamod/CSS/rayTrace×2/BotHider） |
| 119-146 | 6 路 `Expand-*`、Unix 权限修复、`upstreamPayload` 探测 |
| 150 | `Copy-Tree $upstreamPayload.FullName $releaseRoot`（payload 基底） |
| 151 | `Panel*.exe` 清理（基底不再含上游 Panel） |
| 153-160 | **递归** gameinfo.gi 剔除（改为 Panel 解压时只跳根目录，见阶段 4） |
| 162-172 | VPK 归一化 `ConvertTo-BotProfileOnlyVpk` 循环（改为直接用上游原版） |
| 174-207 | metamod/CSS/RayTrace（css+native 两路）/BotHider 覆盖拷贝 + `BotHider.linux.vdf` 清理 |
| 210-212 | BotHider configs 覆盖（与上游 zip 逐字节相同） |
| 215-218 | NadeSystem 手雷目录覆盖（与上游实质相同，源目录也删） |
| 225-235 | `$upstreamPluginBuilds` 中 8 个上游系条目（BotAI、BotAimImprover、BotBuy、BotControllerImpl、BotRandomizer、NadeSystem、RoundDamageRecap）|
| 267-271 | `$botControllerApiBuild` 检查与拷贝（上游 zip `shared/BotControllerApi` 已有） |
| 273-274 | `$botImplBuild`/`$botApiBuild` 拷贝（上游 zip `plugins/BotHiderImpl`、`shared/BotHiderApi` 已有） |
| 312-326 | 0Harmony staging 与 nested shared 清理（`BotHiderImpl` 不再本地构建，上游 zip `shared/0Harmony` 已有） |
| 328-332 | `BotHider.dll` 原生 hash 断言（该文件不再由本包分发） |
| 381 | manifest component 分支的 `"addons/RayTrace/*" → "RayTrace"` |

**修改：**
- payload 构建从"空目录"开始：`$releaseRoot = stage/LocalArena-vX-windows`，随后仅 Copy-Tree：
  - 4 插件输出：`$pluginBuild`（PlayerKnifeCustomizer，行 222/272）、
    `$upstreamPluginBuilds` 缩减后的 PlusMatchCoordinator + TeamLineupInjector（236-246 循环保留）、
    `$telemetryStage`（247-266 保留）
  - `cfg/my_bot_ffa_config.cfg`、`cfg/my_bot_normal_config.cfg`（213-214 保留）
  - `overrides/scripts`（219-220 保留）
  - open-rating 模型校验与拷贝（275-310 保留 —— 这是 LA 自有模型文件）
  - Panel exe / WebView2Loader / README / README.zh-CN / LICENSE / preview 逻辑（334-361 全保留）
- `$plusOwned` 列表（370-376）：**删除** `BotHiderImpl`、`shared/BotHiderApi` 两行
  （不再是本包内容）；保留 PlayerKnifeCustomizer、PlusMatchCoordinator、TeamLineupInjector、
  OfflineMatchTelemetry、2 个 cfg。
- `$preserveConfig`（385-390）：删 `overrides/botprofile.vpk`（不再是包内条目 ——
  它由上游 zip 提供，合并 manifest 生成时再标 preserve-config，见阶段 4）；
  保留 PlayerKnifeCustomizer presets + 2 个 cfg。
- `Get-VerifiedAsset` 函数本体（35-60）：CI 不再下载任何资产 → 函数与 `$cache` 用途可整体删除
  （`.cache/package` 还用于 stage/extract，保留目录变量）。

**保留不动：** build 调用（94-104）、`Copy-Tree`/`Assert-ChildPath`、manifest JSON 序列化框架（391-406）、
`verify-workspace` 调用（408）、三 zip + latest.json + 签名 + SHA256SUMS（411-488）。

### 3.2 `scripts/build.ps1`

`$pluginProjects`（190-202）从 11 项删至 **3 项**：
保留 `PlayerKnifeCustomizer`、`TeamLineupInjector`、`PlusMatchCoordinator`；
删除 `BotAI`、`BotAimImprover`、`BotBuy`、`BotControllerImpl`、`BotRandomizer`、`NadeSystem`、
`RoundDamageRecap`、`BotHiderImpl`。
- `BotHiderApi` 无需显式列出（随两个插件的 ProjectReference 传递编译）。
- 保留：npm 三连（183-188）、OfflineMatchTelemetry publish（206-209）、
  MatchCore.Tests / PlayerKnifeCustomizer.Tests（210-217）、cargo test/build 与 Tauri 断言（219-270）。

### 3.3 `scripts/dependencies.json`

```jsonc
{
  "upstream": {
    "repository": "https://github.com/ed0ard/CS2-Bot-Improver",
    "baseCommit":  "d9914454d8691b509bae4cd671a7a8bd9dcb174f",
    "sourceCommit": "d9914454d8691b509bae4cd671a7a8bd9dcb174f",
    "release": "v1.4.5"
    // windowsAsset 整块删除（Panel 运行时走 GitHub API，不读此文件）
  }
  // metamod / counterStrikeSharp / rayTrace / botHider 四块全部删除
}
```
- 保留 `repository + sourceCommit`：`verify-workspace.ps1:61-72` 的钉版 commit 存在性检查
  与 BotHider gitlink 比对仍需要。
- ⚠风险：删除后 `$manifest.metamod.windowsLoaderSha256` 等引用点必须在阶段 3 同步清掉，否则 PS1 运行期报 null。

### 3.4 `scripts/VpkTools.ps1`

- `package.ps1:171` 的 `ConvertTo-BotProfileOnlyVpk` 调用已删；
- `verify-workspace.ps1:661` 的 `Get-VpkEntryPaths` + `666 Read-VpkEmbeddedEntry` 包内 VPK 断言在阶段 3 删除；
- 两处都删后**整文件删除**（约 300 行）。
- 保留对仓库 `overrides/{Low,Medium,High}/botprofile.db` 的 `Assert-BotProfileContent`
  （读的是 `.db` 文本，不依赖 VpkTools）—— 需实施时确认 `Assert-BotProfileContent` 的实现独立性
  （已确认：`verify-workspace.ps1:403-423` 纯正则，无 VpkTools 依赖）。

---

## 4. 阶段 2：仓库剔除

### 4.1 子模块删除（8 个）

```
git rm addons/counterstrikesharp/plugins/BotState            # ed0ard/CS2-Smarter-Bot
git rm addons/counterstrikesharp/plugins/BotAimImprover      # ed0ard/CS2-Bullseye-Bot
git rm addons/counterstrikesharp/plugins/BotAI               # ed0ard/CS2-BotAI
git rm addons/counterstrikesharp/plugins/BotRandomizer       # ed0ard/CS2-Bot-Randomizer
git rm addons/counterstrikesharp/plugins/NadeSystem          # ed0ard/CS2-Bot-NadeSystem
git rm addons/counterstrikesharp/plugins/BotBuy              # ed0ard/CS2-Bot-Buy
git rm addons/BotController                                  # XBribo/CS2-Bot-Controller
git rm addons/BotVision                                      # XBribo/CS2-Bot-Vision（用户确认删除）
```
- `.gitmodules` 同步清理 8 个 section，**保留 `addons/BotHider`**（XBribo/CS2-Bot-Hider）。
- 仓库当前 `.gitmodules` 共 9 个 section，删后剩 1 个。
- ⚠风险：`git submodule deinit` → `git rm` 顺序，避免 `.git/modules` 残留影响后续 CI（CI 全新 clone 无此问题）。

### 4.2 源码/数据删除

| 路径 | 原因 |
|---|---|
| `addons/counterstrikesharp/plugins/RoundDamageRecap/` | 上游衍生物，非子模块（普通 tracked 树），C 决策删除；上游 zip 有预编译版 |
| `addons/counterstrikesharp/plugins/disabled/` | ed0ard 的 Linux 变体源码树（BotAI_for_Linux 等），纯留存，用户确认删除 |
| `addons/counterstrikesharp/data/` | NadeSystem 手雷目录，48 文件与上游实质相同 |
| `cfg/gamemode_*.cfg`（15 个） | 与上游仅行尾差异（retakecasual 上游更新） |
| `cfg/bot_buy.cfg`、`cfg/my_bot_rush_config.cfg` | 与上游逐字节相同 |

**保留：** `cfg/my_bot_ffa_config.cfg`、`cfg/my_bot_normal_config.cfg`（LA 加了 `bh_namesource 1`）、
3 个 `*_rules_unchanged.cfg`、`overrides/scripts/`、`overrides/{Low,Medium,High}/botprofile.db`
（LA 难度调校，verify 白名单内）、`overrides/archived/`、4 个独占插件、MatchCore、BotHider 子模块。

**不改子模块内部：** `addons/BotHider/configs/…`（虽与上游 zip 相同，但改子模块 = 破坏钉版；
只删 `package.ps1:210-212` 的拷贝步骤）。

### 4.3 构建残留清理

- `git status` 检查 `addons/counterstrikesharp/plugins/*~ed0ard_main/`（若为 untracked 构建残留可直接 rm）。
- `addons/BotBuy/obj` 等 grep 命中过 ed0ard 字样的 obj 文件随子模块删除一并消失。

---

## 5. 阶段 3：`verify-workspace.ps1` 改造（736 行，接近重写）

### 5.1 上游钉版区（49-146）

| 段 | 处理 |
|---|---|
| `$base` 解析 + fetch（49-72） | **保留**（BotHider gitlink 比对仍用） |
| `$submodulePaths`（77-87，9 项） | 缩至 **1 项**：`addons/BotHider` |
| `$upstreamTrees`（101-105，3 项 RoundDamageRecap/disabled/data） | **整段删除**（树已不存在） |
| `$removedUpstreamPaths`（116-123） | 保留原有 6 项，**新增** 8 个已删子模块路径 + `addons/counterstrikesharp/data`、`addons/counterstrikesharp/plugins/disabled`，断言"必须无 gitlink/无 tracked 内容" |
| `$allowedOverrideChanges` + overrides 比对（131-145） | **保留**（`overrides/{L,M,H}/botprofile.db` 白名单继续有效） |

### 5.2 源码断言区

| 行 | 处理 |
|---|---|
| 148-151 BotAI.csproj `Tmp\ArchiveV02` 检查 | 删除（BotAI 已删） |
| 161-168 NadeSystem Less-mode 源码 guard | 删除（读的是已删子模块文件，会直接 throw） |
| 169-184 NadeSystem 48 手雷文件计数与 JSON 校验 | 删除（目录已删） |
| 186-198 PlusMatchCoordinator / MatchPanel 断言 | 保留 |
| 200-234 `$requiredSources` | 删 `BotRandomizer.cs`、`CosmeticModels.cs`、`charm_placements.json`、`cosmetic_catalog.json`、`BotControllerImplPlugin.cs`、`BotHiderImplPlugin.cs`（前 2 个在 BotRandomizer 子模块、后 2 个在已删子模块路径）；**注意** `BotHiderImplPlugin.cs` 在 BotHider 子模块内**仍存在** → 保留该项 |
| 236-280 PlayerCosmetics 断言 | 保留 |
| 282-308 `$jsonFiles` | 删 BotRandomizer 两项（jsonFiles 中 `charm_placements.json`、`cosmetic_catalog.json`）；BotHider configs 两项保留（子模块仍在） |
| 425-429 BotHider gamedata 断言 | 保留 |
| 430-434 `overrides/{L,M,H}/botprofile.db` 内容断言 | 保留（featured players + LookAngleMaxAccel 合法性） |

### 5.3 包断言区（443-724，核心反转）

**新策略：白名单制 —— 包内只允许 LA 自有内容。**

- `$requiredPackageFiles`（445-494）缩减为：
  ```
  addons/counterstrikesharp/plugins/PlayerKnifeCustomizer/PlayerKnifeCustomizer.dll (+sticker_ids.json
    +sticker_weapon_ids.json +player_cosmetic_catalog.json +player_gun_presets.json +player_knife_presets.json)
  addons/counterstrikesharp/plugins/PlusMatchCoordinator/PlusMatchCoordinator.dll + MatchCore.dll
    + match_catalog.json + open-rating-3.0-proxy-v1.json + profiles/{Low,Medium,High}/botprofile.db
  addons/counterstrikesharp/plugins/TeamLineupInjector/TeamLineupInjector.dll
  addons/counterstrikesharp/plugins/OfflineMatchTelemetry/（8 文件 allowlist 不变，498-516 保留）
  cfg/my_bot_ffa_config.cfg + cfg/my_bot_normal_config.cfg
  overrides/scripts/**（至少断言非空）
  plus-payload-manifest.json + LocalArena.exe + README.md + README.zh-CN.md + LICENSE
  ```
- **新增反向断言**（防止上游内容回流进包）：
  ```powershell
  $forbiddenInPackage = @(
      "gameinfo.gi",                       # 根目录与 backup 内都不该有（新包不含上游内容）
      "addons/RayTrace", "addons/metamod", "addons/BotHider/bin", "addons/BotController/bin",
      "addons/BotVision/bin",
      "addons/counterstrikesharp/bin", "addons/counterstrikesharp/dotnet",
      "addons/counterstrikesharp/plugins/BotAI", "…/BotAimImprover", "…/BotBuy",
      "…/BotControllerImpl", "…/BotHiderImpl", "…/BotRandomizer", "…/BotState",
      "…/NadeSystem", "…/RoundDamageRecap", "…/RayTraceImpl",
      "addons/counterstrikesharp/shared/0Harmony", "…/shared/BotHiderApi", "…/shared/BotControllerApi",
      "overrides/botprofile.vpk",          # 不再由 LA 分发
      "cfg/gamemode_", "cfg/bot_buy.cfg", "cfg/my_bot_rush_config.cfg",
      "backup/"
  )
  # 目录型条目用 Test-Path，文件型用 Get-ChildItem -Filter 递归探测
  ```
- **删除**：
  - 手雷包内比对（517-536）
  - `nestedShared`/`linuxBotHiderVdf` 检查（551-558）—— 对应内容已不入包（559-562 gameinfo 检查保留但语义变为"绝对不含"）
  - 包内 VPK 四件套断言（650-675）
  - `BotHider.dll` hash（677-683）
  - `$pinnedRuntimeFiles` metamod/CSS/RayTrace 四项（685-699）
  - `$builtPlugins` 包内 DLL == 本地构建比对（701-723）→ 改为仅比对 4 个独占插件
    （PlusMatchCoordinator、TeamLineupInjector、PlayerKnifeCustomizer；OfflineMatchTelemetry 走 stage allowlist）
- manifest 断言（564-648）**保留**，但：
  - `$expectedPreserveConfigs`（611-617）删 `overrides/botprofile.vpk`（包内不再有）；
  - ownership 断言删 BotHiderImpl（若存在）—— 现有三项（PMC/TLI/OMT）保留，**新增 PlayerKnifeCustomizer**；
  - **新增**：断言 manifest 不含任何 `$forbiddenInPackage` 路径。

### 5.4 其他

- 开头 `. VpkTools.ps1`（11）：随 VpkTools 删除一并去掉。
- `443 if ($PackageRoot)` 结构保持，本地与 CI 同一入口。

---

## 6. 阶段 4：Panel Rust — 新模块 `upstream_package.rs`（核心）

### 6.1 职责与 API 设计

```rust
// Panel/src-tauri/src/upstream_package.rs（新文件，预计 500-700 行）

const RELEASES_LATEST: &str =
    "https://api.github.com/repos/ed0ard/CS2-Bot-Improver/releases/latest";
const LATEST_DOWNLOAD_FALLBACK: &str =
    "https://github.com/ed0ard/CS2-Bot-Improver/releases/latest/download/CS2BotImprover.zip";
const ASSET_NAME: &str = "CS2BotImprover.zip";
const MAX_ARCHIVE_BYTES: u64 = 400 * 1024 * 1024; // update_core 同款上限思路

/// 1) 解析最新 release
#[derive(Serialize, Deserialize)]
pub struct UpstreamRelease {
    pub tag: String,
    pub asset_url: String,
    pub digest: Option<String>, // 决策 #5：API 提供的 "sha256:…"，回退路径为 None
    pub from_api: bool,
}
pub fn resolve_latest() -> Result<UpstreamRelease>;
// 实现：GET RELEASES_LATEST（reqwest blocking，30s，复用 online_update::client 模式）
//   → 解析 tag_name + assets[].browser_download_url + assets[].digest（按 name == CS2BotImprover.zip 匹配）
//   → 403/429（匿名限流 60 次/时/IP）时回退 LATEST_DOWNLOAD_FALLBACK，
//     tag = "latest"，digest = None，from_api=false
//   → 复用 update_core::validate_https_github_url 校验 scheme/host

/// 2) 下载（带进度/取消/断点续传）
pub fn download_start(app: &AppHandle, tag: String, url: String) -> Result<()>;
// 落盘：update_root()/upstream/<tag>/CS2BotImprover.zip
// 临时：同目录 CS2BotImprover.zip.part；存在 .part 时带 Range: bytes=<size>- 续传
// 进度：复用 online_update::set_progress 风格，emit "upstream://progress"
//   { tag, downloaded_bytes, total_bytes, speed_bps, stage: "downloading"|"verifying"|"done" }
// 取消：复用 online_update::OperationGuard + cancel 标志
// 完整性（决策 #5）：大小检查（>0 且 <= MAX_ARCHIVE_BYTES）+ digest 存在时比对文件 sha256，
//   不匹配 → 删 .part 报错（重试可恢复）；digest 缺失（回退路径 / 手动导入）→ 仅 HTTPS 不校验

/// 3) 缓存管理
pub fn cache_info() -> Result<UpstreamCacheInfo>;   // [{tag, size, modified_at}], total_bytes
pub fn cache_clear(tag: Option<String>) -> Result<usize>;  // 返回释放字节数

/// 4) 手动导入本地 zip
pub fn import_local(app: &AppHandle, source: String) -> Result<UpstreamRelease>;
// 结构校验（无哈希）：zip 中存在 `addons/` + `cfg/` 前缀条目（对应 package.ps1:144 的 payload 探测逻辑）
// tag 探测：扫描根目录 `Panel v*.exe` 文件名解析版本，否则 "local"
// 拷入 update_root()/upstream/<tag>/CS2BotImprover.zip（同 tag 已存在则覆盖）

/// 5) 合并（★关键设计）
pub struct MergeInput {
    pub upstream_zip: PathBuf,   // 缓存或导入的 zip
    pub la_payload: PathBuf,     // 通常 = lib.rs::payload_root()（exe 旁 LA 包）
    pub staging_root: PathBuf,   // update_root()/merged/<tag>/
}
pub fn merge(input: &MergeInput) -> Result<PathBuf>; // 返回合并 payload 根
```

### 6.2 `merge()` 详细步骤

1. `clear_directory(staging_root)`（复用 `online_update::clear_directory`；
   ⚠`clear_directory` 与 `update_root()` 当前均为**私有 `fn`**（`online_update.rs:481`/`115`），
   需改为 `pub(crate)` 才能跨模块复用）。
2. 解压上游 zip → `staging_root`：
   - 复用 `update_core::extract_zip_safely`（已有 zip-slip 防护，`installer::safe_relative` 同源）。
   - **payload 探测（对齐 `package.ps1:141-147`，前移到 merge 入口，阻断式）**：在 staging 内定位
     同时含 `addons/` 与 `cfg/` 的目录（staging 根自身优先，再逐层递归，取第一个命中），
     后续步骤 4/5 的叠加与白名单 walk **一律相对该 `payload_dir` 执行**；找不到 →
     `AppError::payload("Could not locate the upstream game/csgo payload")` 直接失败。
     ⚠防御点：上游将来若把内容包进一层顶层目录，缺此步会按 staging 根走白名单 →
     **静默生成 LA-only 合并 manifest**（步骤 5 的非白名单 WARN 只记日志不阻断，故必须在此阻断）。
   - **平铺**：若探测到的 `payload_dir != staging_root` → 将其内容 `fs::rename` 上移到 staging 根
     （目标已存在同名条目则报错，不做静默合并），此后 `payload_dir == staging_root`，
     步骤 4/5/6 全部按 staging 根进行，逻辑与常规路径完全一致。
3. **过滤上游内容**（决策 #4、#8）：
   ```rust
   // a) 跳过根 gameinfo.gi（只根目录；backup/Online|WithBots/gameinfo.gi 保留落 staging，
   //    但按决策 #4 不进合并 manifest、不安装进游戏目录）
   if rel == "gameinfo.gi" { skip }
   // b) Panel v*.exe 不写入 staging，而是移动到 current_exe().parent()
   //    命名保留原名（如 "Panel v1.4.5.exe"），并记录到 state 供管理页启动
   if rel.starts_with("Panel ") && rel.ends_with(".exe") && !rel.contains('/') { relocate }
   // c) addons/metamod/RayTrace.vdf：照常安装（假设 #3，上游自己发的 34 字节残留）
   ```
   实现方式：解压到 staging 后用后置移动更简单（解压全程落盘 → `fs::rename` 根 gameinfo 删除、
   Panel exe `fs::rename` 出去）。⚠风险：`fs::rename` 跨盘失败 → 降级 copy+remove。
4. **叠加 LA payload**：walk `la_payload` 的 `addons|cfg|overrides`，`copy_file` 覆盖 staging 同名文件
   （LA 独占插件/2 cfg/overrides-scripts 与上游无同名冲突；若上游未来出现同名，LA 后写 = LA 赢，
   这正是"叠加"语义）。
   - **不**把 LA 包内的 `plus-payload-manifest.json`、`LocalArena.exe` 等非 payload 文件带入。
5. **生成合并 manifest**（★关键设计）：
   ```rust
   // 输入 A：读 la_payload/plus-payload-manifest.json —— LA 条目原样继承
   //   （path/size/sha256/component/ownership=plus/restore_policy 全不动）
   // 输入 B：walk staging 的 `addons|cfg|overrides` 子树中"不等于 LA 条目"的所有文件 → 上游条目：
   //   ⚠白名单前缀是刻意的：`backup/**` 不在其中 → 天然排除出 manifest（决策 #4 修订，
   //     避免"装入即被 cleanup 删除 → inspect 报 missing → 修复循环"）；
   //     backup/ 属已知顶层条目 → INFO 级跳过；其余未知顶层/非白名单条目打 WARN 后跳过
   //     （防上游改结构静默丢文件，见 R5；探测+平铺已在步骤 2 阻断嵌套包裹的情况）
   //   ownership   = "shared"
   //   component   = 移植 package.ps1:377-384 同规则：
   //                 addons/counterstrikesharp/plugins/<Name>/* → <Name>
   //                 addons/BotHider/* → "BotHider"
   //                 cfg/* → "configuration"; overrides/* → "overrides"; 其余 → "runtime"
   //                 （删除 RayTrace 分支）
   //   restore_policy：
   //     preserve-config = overrides/botprofile.vpk（与今天 manifest 行为对齐）
   //     其余 = "restore"
   //   sha256/size = 合并时对 staging 文件现算（serde + sha2 crate 已在依赖里）
   // 写出 staging/plus-payload-manifest.json，schema_version=1，
   //   package_version = LA manifest 的 package_version（保持 verify_payload 版本比对语义）
   ```
6. 返回 staging 根 → 交给 `installer::plan/install/verify/restore`。

### 6.3 安装管线接线（★关键设计 #2）

**问题**：`lib.rs::payload_root()`（903）当前 = exe 目录或 active payload；安装/检查/恢复全走它。
新模型下 exe 旁的 LA 包**不含上游文件**，直接用它跑 `installer::install` 会漏装一切上游内容。

**方案**：引入"合并 payload 就绪"前置状态，所有以游戏目录为目标的操作统一先合并：

```rust
// lib.rs 新增
fn merged_payload_root(app: &AppHandle, csgo: &str) -> Result<PathBuf> {
    // 1) 读 state/upstream.json（已安装记录，见 6.4）+ 缓存的上游 zip
    // 2) 若 merged/<tag> 存在且 LA payload 的 mtime/hash 未变 → 复用（打 mtime 标记避免每次重合并）
    // 3) 否则调 upstream_package::merge()（后台线程，emit 进度）
}
```

受影响的调用点（下表已按 `grep -n "payload_root()" Panel/src-tauri/src/lib.rs` **全量**核对，
除"保持"项外一律从 `payload_root()` 切换为 `merged_payload_root`；`payload_root()` 保留给
"读 LA 包自身 manifest/版本"的场景）：

| 位置 | 现状 | 改为 |
|---|---|---|
| `lib.rs:954` + `validate_files_at`（948 起） | `payload_root()` | merged |
| `lib.rs:1248/1278` `collect_install_checks`（match 流程） | `payload_root()` | merged |
| `lib.rs:1723` `run_install_checks` 命令 | `payload_root()` | merged |
| `lib.rs:2927` `export_diagnostics` | `payload_root()` | merged（诊断应报告合并视图） |
| `lib.rs:2970/2981` `inspect_installation`/`get_install_plan` | `payload_root()` | merged（inspect 需 merged 才能判断 PAYLOAD_* 项） |
| `lib.rs:2987 install_payload`、**`2991` impl 内 payload** | 传 `&payload` | merged |
| `lib.rs:3020 repair_payload`、**`3024` impl 内 payload** | 传 `&payload` | merged |
| **`lib.rs:3095 restore_pristine` / `3097 restore`** | `payload_root()` | **merged〔评审补漏〕** |
| **`lib.rs:3122 inspect_quick`** | `payload_root()` | **merged〔评审补漏〕** |
| `lib.rs:1232` `match_system::load_catalog`（1250 取 payload） | `payload_root()` | **保持**（match_catalog 是 LA 自有，exe 旁就有） |
| `lib.rs:3149 install_plugin_update_impl` | `prepare_plugin` → active payload | **重写**：见 6.5 |
| `online_update::activate_payload/active_payload_root` | 指向 payloads/ 活动 payload | 合并缓存语义替换（见 6.5） |

> **restore 两处的补漏说明**（2026-10-05 评审）：`restore` 有 record 时读 state record 不读 payload
> manifest；无 record 分支只过滤 `ownership=="plus"` 条目（`installer.rs:1324`，两 manifest 中相同）。
> `restore_pristine` 用 `manifest ∪ record`（1436-1446），record 在时兜得住。真正缺口是 **record
> 丢失/损坏时**：`SUITE_OWNED_ROOTS` 覆盖不到 `cfg/gamemode_*` 等上游 cfg → pristine 清理漏删。
> 故这两处仍须切 merged（防御性完整），并列入 R3 核对项与 9.2 场景 6。

**前置门禁**：上游 zip 未就绪（未下载 + 无本地 zip）时，`get_install_plan` 返回明确错误
`AppError::payload("Upstream package is not available; download it in Installation Management")`，
前端据此把用户引导到安装管理页（决策 #10 的"页内触发安装/修复"依赖此状态机）。

### 6.4 已安装状态记录（决策 #10 的版本展示）

```jsonc
// state_root()/upstream-install.json（原子写，复用 atomic_fs::write_replace）
{
  "schema_version": 1,
  "upstream_tag": "v1.4.5",         // 或 "latest"（回退解析）/ "local"（手动导入）
  "upstream_source": "api" | "fallback" | "manual",
  "upstream_installed_at": 1759...,
  "la_version": "1.4.3.3",           // LA manifest package_version
  "la_installed_at": 1759...,
  "target": "F:/Steam/.../csgo",
  "upstream_panel_exe": "F:/.../LocalArena/Panel v1.4.5.exe"  // 决策 #8，可为 null
}
```
- 写入时机：`installer::install` 成功返回后。
- `inspect` 读不到该文件 → "未安装/未知"，管理页显示初始态。

### 6.5 更新流程改造（决策 #7：更新也改为上游 + 叠加）

`online_update.rs::prepare_plugin`（500-527）重写为：

```rust
pub fn prepare_plugin(app: &AppHandle) -> Result<(String, PathBuf)> {
    let (component, archive) = download_component(app, "plugin")?;   // LA plugin zip 照旧（签名+sha256 不变）
    let plugin_root = /* 解压 find_payload_root + verify_payload（现有逻辑）*/;
    let upstream = upstream_package::ensure_cached(app)?;  // 读缓存；tag != resolve_latest()?.tag 则下载
    let merged = upstream_package::merge(MergeInput {
        upstream_zip: upstream.zip,
        la_payload: plugin_root,          // ★ 叠加源从 exe 旁换成刚下载的 plugin zip
        staging_root: update_root()/"merged"/upstream.tag,
    })?;
    installer::verify_payload(&merged)?;   // 合并后自校验
    Ok((component.version, merged))
}
```
- `activate_payload`（529-549）：路径断言 `starts_with(payloads)` 改为 `starts_with(update_root()/"merged")`；
  或干脆删除 `active-payload.json` 指针机制（合并根由 `upstream_package` 自管理）。
- `install_plugin_update_impl`（lib.rs:3149-3192）主体不变（install + 写 state + 日志）。
- `latest.json` 的 panel/plugin 组件 schema、签名（`sign-update.py`、`UPDATE_PUBLIC_KEY`）、
  sha256 全部**不动** —— 上游 zip 是唯一例外（走 API digest，无钉版 sha256）。
- `install_all_updates`（3241）同样走新链路。
- ⚠风险：`prepare_plugin` 现被 `install_all_updates` 在 worker 里调用（`worker_app`），
  确保 `upstream_package` 的进度 emit 在无 window 时安全（`app.emit` 对无 window 句柄的处理需验证，
  online_update 已有同款模式可照抄）。

### 6.6 RayTrace 全链路移除（决策 #2）

| 文件 | 行 | 动作 |
|---|---|---|
| `installer.rs` | 36 `"addons/metamod/RayTrace.vdf"`（UPSTREAM_MARKERS） | **删除**（上游已移除依赖；marker 用于识别旧上游安装，其余 7 项足够） |
| `installer.rs` | 42 `"addons/RayTrace"`、**59** `"addons/counterstrikesharp/plugins/RayTraceImpl"`（SUITE_OWNED） | **保留**（假设 #2：旧安装残留文件仍需识别为可清理）〔行号 2026-10-05 评审更正〕 |
| `installer.rs` | **73** `"addons/metamod/RayTrace.vdf"`（SUITE_OWNED_FILES） | **保留**（同上）〔行号评审更正〕 |
| `mode_layout.rs` | 17 `RayTraceImpl.dll`、24 `addons/metamod/RayTrace.vdf` | **删除**（预览布局不再管理） |
| `install_checks.rs` | 214 `("RAYTRACE_X64", …)` 元组 | **删除**（TARGET_/PAYLOAD_ 成对消失） |
| `lib.rs` | 1779 `"TARGET_RAYTRACE_X64"`（`ensure_match_components_pass` 必过前缀） | **删除** |
| `Panel/src/lib/installCheckLocalization.ts` | 37 `RAYTRACE_X64: "RayTrace"` | 删除键（确认类型定义是否要求全键） |
| i18n 字典 | 各语言 `install.check.*RAYTRACE*` 相关键 | 删除（实施时 grep 全量） |
| ⚠联动 | `install_checks.rs` 的 component_check 循环（212-230）生成 `PAYLOAD_{code}` 查 `payload_root.join(rel)` | merged 切换后该循环天然工作（merged 含上游文件） |

**注意**：删除 `RAYTRACE_X64` 后，旧安装的报告里不再出现该检查项 —— 属预期（跟随上游）。

### 6.7 Tauri 命令新增（`lib.rs` + `generate_handler` 注册）

```rust
// resolve/download/cache/merge 状态机
fn upstream_release_info(app) -> Result<UpstreamReleaseInfo>;      // 缓存 6h，含 latest tag + 本地缓存状态
async fn upstream_download(app, tag: String) -> Result<()>;        // spawn_blocking + OperationGuard
fn upstream_cancel();                                              // 转发 online_update::cancel 或独立标志
fn upstream_cache_info() -> Result<UpstreamCacheInfo>;
fn upstream_cache_clear(tag: Option<String>) -> Result<usize>;
async fn upstream_import_local(app, source: String) -> Result<UpstreamRelease>;
fn upstream_installed_state() -> Result<Option<UpstreamInstalledState>>;   // 6.4 的 JSON
fn launch_upstream_panel(path: String) -> Result<()>;               // 决策 #8：启动前断言
                                                                     // canonical path 位于 current_exe parent
                                                                     // 且文件名匹配 "Panel v*.exe"
```
事件：`upstream://progress`（下载进度）、`upstream://merged`（合并完成）。
沿用 `online_update::OperationGuard`（单操作互斥）避免与组件更新并发。

---

## 7. 阶段 5：前端 — 新"安装管理"页

### 7.1 入口与路由

- 位置：设置 → 安装（`SettingsView.tsx` 的 `Page` union + `TITLE_KEYS` + 渲染分支，
  参照现有 `installation`/`updates` 页模式）→ 新增 `"installManagement"` 条目。
  - 方案 A（推荐）：**扩展现有 `InstallationPage.tsx`** 为两段式（上段=上游包管理，下段=现有安装检查），
    避免设置页再加卡片。
  - 方案 B：独立 `InstallManagementPage.tsx` + 设置页新卡片。
  - 实施时选 A（少一层导航），若现有 InstallationPage 代码过大则选 B。
- 侧边栏 `App.tsx` 不加新一级入口（决策说的是"管理界面"，放在安装域内足够）。

### 7.2 页面区块（决策 #10 四功能）

```tsx
// 上游包
<section> 最新版本: {tag}（来源: GitHub API / 回退 / 本地导入）
  [检查更新] [下载] —— 下载中: 进度条 {downloaded}/{total} · {speed} MB/s · [取消]（断点续传提示）
  缓存: 73.2 MB（v1.4.5）  [重新下载] [清除缓存]
  [手动选择本地 ZIP…]   ← tauri dialog（现有 DirectoryPage 有目录选择可参考；文件选择需新 dialog 调用）
</section>

// 已安装状态（决策 #10）
<section> 游戏目录: {csgo}
  上游: v1.4.5 · 2026-10-04 · 来源 api   LA: 1.4.3.3 · 2026-10-04
  对比: 已是最新 / 可更新到 v1.4.6
  [启动上游 Panel v1.4.5.exe]（决策 #8；文件不存在时隐藏）
</section>

// 操作
<section> [安装到所选目录] [修复] —— 前置：上游就绪门禁
  状态机: upstream-ready? → get_install_plan → install_payload（全部复用现有 api.ts 封装）
</section>
```

### 7.3 `api.ts` 封装

在 `Panel/src/lib/api.ts`（`invoke<T>` helper，12 行）旁新增：
```ts
upstreamReleaseInfo: () => invoke<UpstreamReleaseInfo>("upstream_release_info"),
upstreamDownload: (tag: string) => invoke<void>("upstream_download", { tag }),
upstreamCancel: () => invoke<void>("upstream_cancel"),
upstreamCacheInfo: () => invoke<UpstreamCacheInfo>("upstream_cache_info"),
upstreamCacheClear: (tag?: string) => invoke<number>("upstream_cache_clear", { tag }),
upstreamImportLocal: (source: string) => invoke<UpstreamRelease>("upstream_import_local", { source }),
upstreamInstalledState: () => invoke<UpstreamInstalledState | null>("upstream_installed_state"),
launchUpstreamPanel: (path: string) => invoke<void>("launch_upstream_panel", { path }),
```
类型定义放 `api.ts` 现有类型区（与 `OnlineUpdateSnapshot` 等并列）。

### 7.4 i18n

- 新键前缀 `upstream.*`（约 35-45 个键）：`upstream.title`、`upstream.latest`、`upstream.download`、
  `upstream.downloading`、`upstream.resume`、`upstream.cancel`、`upstream.cache`、`upstream.clear`、
  `upstream.manual`、`upstream.installed`、`upstream.notInstalled`、`upstream.launchPanel`、
  `upstream.installAction`、`upstream.repairAction`、`upstream.needDownload` 等。
- 字典（`dictionary.ts`，**28 种语言块**——grep 实测 `schinese`…`latam` 计 28，另有 `english: EN` 引用块；
  原写 26 为笔误〔2026-10-05 评审更正〕；`schinese`/`tchinese`/`english` 全量，其余语言按现有惯例
  部分翻译 + fallback 到英文 —— 实施时先确认 `i18n/index.ts` 的 fallback 行为再定翻译覆盖范围）。
- 删除 `RAYTRACE_X64` 相关键（若存在于各语言块）。
- **新增向导文案**：`guide.install.*` 或 `installation.upstreamStep.*`
  （GuideView.tsx `INSTALL_STEPS` 数组 60-63 行，四步 → 需要插入"下载上游组件"步或在第 3 步
  "Review the installation plan" 文案中说明）。

### 7.5 门禁联动

- `installGate.ts`（`test-install-gate.mjs` 断言的纯函数）：新增
  `upstreamNotReady(disabled)` 参与 `installAttemptDisabled`，或新增独立导出
  `installBlockedByUpstream(ready: boolean)`。⚠若改 `installAttemptDisabled` 签名，
  `scripts/test-install-gate.mjs` 的 4 个断言需同步补例（`npm run test:install-gate` 在 build.ps1:187 必跑）。

---

## 8. 阶段 6：文档

### 8.1 README.md / README.zh-CN.md

- `## Four-Step First Installation`（80-129）：第 3/4 步之间补充"下载上游组件"（73MB，需联网，
  可手动指定本地 zip）；说明 LA 包本身只有几 MB。
- `## Updating an Existing Installation`（131-149）：补"组件更新会重新拉取上游包（缓存命中则跳过）"。
- `## Installation, Updates, and Recovery`（240-277）：Online updates 小节补充上游包缓存管理说明。
- **署名/致谢/AGPL 相关章节一律不动**（AGPL-3.0 合规要求，且用户未选"删除文档与署名链接"）。
- `package.ps1:346-353` preview 分支的 README 替换字符串（"The current main branch targets **1.4.3.3**"）
  若动 README 正文需核对仍能命中。

### 8.2 `docs/UPSTREAM.md`

- `## Pinned Runtime Inputs`（126-144）整段重写：
  - `CS2BotImprover.zip` 改为"运行时由 Panel 从 GitHub latest release 下载（仅 HTTPS，API 路径校验 digest）"；
  - 删除 MetaMod/CSS/RayTrace/BotHider 钉版本 bullet；
  - 删除"release DLLs are never copied from an older archive"整段（8 插件不再本地构建）；
  - 保留 BotHider 子模块钉版说明。
- `## Synchronizing`（147+）：第 6 步"create a disposable package"保留；新增一节
  记录本次架构变更（LA 包 = 自有内容 + Panel 运行时合并上游）。

### 8.3 `docs/` 其余

- `Local-Arena-v1.4.3.*` 历史发布说明**不改**（历史记录）。

---

## 9. 阶段 7：验证清单（每阶段完成后跑对应项）

### 9.1 命令级

```bash
# pwsh 路径按环境而定：本会话约定用 /tmp/opencode/pwsh/pwsh（不手动安装 PowerShell）；
# 其他环境用 PATH 中的 pwsh，CI 用 pwsh/powershell。
# 阶段 1-2 后
/tmp/opencode/pwsh/pwsh -NoProfile -Command \
  "foreach ($f in 'scripts/package.ps1','scripts/build.ps1','scripts/verify-workspace.ps1') { \
     $t = Get-Content $f -Raw; [scriptblock]::Create($t) | Out-Null; Write-Host \"OK $f\" }"
dotnet build -c Release   # 3 个插件 + MatchCore.Tests + PlayerKnifeCustomizer.Tests
dotnet run --project addons/counterstrikesharp/shared/MatchCore.Tests -c Release

# 阶段 3 后（本地无 GITHUB_ACTIONS 时需 .local-build.ps1 存在）
/tmp/opencode/pwsh/pwsh -NoProfile -File scripts/verify-workspace.ps1   # 期望 exit 0

# 阶段 4-5 后（Linux cargo 注意：需临时 icons/icon.png，历史已知问题）
cargo check --manifest-path Panel/src-tauri/Cargo.toml
cargo test  --manifest-path Panel/src-tauri/Cargo.toml
# 新模块单测清单：
#   merge_manifest_*：LA 条目继承 / 上游条目推导 component/ownership/policy / sha256 现算
#   merge_filter_*：根 gameinfo.gi 跳过、backup/ gameinfo 保留在 staging 但**不进 manifest**、
#                   Panel exe 移出、RayTrace.vdf 保留、非白名单顶层条目被跳过并记日志
#   payload_probe_*：上游整体包一层顶层目录 → 探测+平铺后仍生成含上游条目的 manifest；
#                    完全无 addons+cfg → 报错阻断（不产出 LA-only 静默合并）
#   zip_slip_*：恶意 zip 条目被拒
#   resolve_*：API 正常 / 403 回退 / 资产缺失报错
#   digest_*（决策 #5）：digest 匹配通过 / 不匹配报错并清理 .part / 回退路径无 digest 跳过校验
#   version_parse：从 "Panel v1.4.5.exe" 解析 tag、无 exe → "local"
cd Panel && npm test 2>/dev/null; npm run build
npm run test:stickers && npm run test:install-gate   # build.ps1:186-187 必跑项
```

### 9.2 手动流程（合并后必须在 Windows 实机过一遍）

| # | 场景 | 期望 |
|---|---|---|
| 1 | 干净 CS2 + 全新安装（API 下载） | 下载进度/取消/断点续传可用；digest 校验通过（决策 #5）；合并后游戏目录 = 今天包安装结果，**除**：无 RayTrace 主体、根 gameinfo.gi 未被覆盖、`csgo/backup/` 无 gameinfo（决策 #4）、VPK 为上游原版、多出上游 `Panel v1.4.5.exe` 在 LA 目录 |
| 2 | 断网 + 手动指定本地 zip | 结构校验通过 → 完整安装；tag 显示 "local" |
| 3 | LA plugin 组件更新（上游版本不变） | 缓存命中不重下 73MB；合并 + 安装成功 |
| 4 | 上游发新版（latest 变化） | `upstream_release_info` 显示可更新；更新时自动下载新上游 zip |
| 5 | 模式切换（Normal/Enhanced/Preview） | `rewrite_gameinfo` 以当前游戏文件重写，搜索路径正确 |
| 6 | 修复 / restore / restore_pristine | 合并 manifest 下 backup/restore 完整（上游文件也能被恢复） |
| 7 | 安装检查（`run_install_checks`） | TARGET_/PAYLOAD_ 项全绿；RAYTRACE 项消失；上游未就绪时给出引导文案 |
| 8 | 启动上游 Panel | 只能启动 LA 目录下 `Panel v*.exe`，路径越权被拒 |
| 9 | 旧安装（含 RayTrace 残留）升级 | SUITE_OWNED 仍识别 RayTrace 残留可清理；报告不再列 RAYTRACE_X64 检查 |

### 9.3 CI

- push 到 `update`/`build` 触发 `build.yml`（`ca7de23` 后的配置）：build → package → verify-workspace。
- 关注点：CI 不再下载 5 路资产（提速 + 无外网依赖风险）；`f5f1f73` 的 BotController 输出路径修复
  在 BotController 子模块删除后相关代码一并消失（package.ps1 229 行条目、verify 705 行条目）。

---

## 10. 提交切分（Conventional Commits + gitmoji，本地不推送）

| # | 提交 | 内容 | 验证门槛 |
|---|---|---|---|
| 1 | `🔥 refactor(pipeline): 移除上游 release 内容打包与钉版本依赖下载` | 阶段 1：package.ps1 / build.ps1 / dependencies.json /（VpkTools 视引用情况） | PS1 语法 + dotnet build + npm build |
| 2 | `🔥 refactor(repo): 删除不再构建的上游子模块与重复文件` | 阶段 2：8 子模块 + RoundDamageRecap + disabled + data + cfg | git status 干净 + dotnet build |
| 3 | `✅ test(verify): 重写 verify-workspace 为 LA 独有内容断言` | 阶段 3 | verify-workspace exit 0 |
| 4 | `🔥 refactor(panel): 移除 RayTrace 残留` | 阶段 4.6 全部条目（独立成提交便于回溯） | cargo check + cargo test |
| 5 | `✨ feat(panel): 上游 release 解析/下载/缓存与合并安装管线` | 阶段 4.1-4.5、4.7、6.5 | cargo test 新模块单测全绿 |
| 6 | `✨ feat(ui): 安装管理页面` | 阶段 5 全部 + i18n + 向导文案 | npm build + test:install-gate |
| 7 | `📝 docs: 更新安装说明与 UPSTREAM.md` | 阶段 6 | 无（文档） |

依赖关系：2 → 1 → 3 可并行度高；4 可在 5 前后独立；6 依赖 5 的命令注册；7 最后。

---

## 11. 默认假设（审阅时确认或推翻）

1. **`cfg/my_bot_rush_config_rules_unchanged.cfg` 保留**——不在确认的删除清单里
   （其主文件 `my_bot_rush_config.cfg` 因与上游相同会删），无引用、无害。
2. **RayTrace 在 `SUITE_OWNED` 的归属条目保留**（`installer.rs:42,59,73`）——只删安装标记
   （`UPSTREAM_MARKERS:36`）、`mode_layout`、`install_checks`；旧安装残留的 RayTrace 文件
   仍被识别为可清理对象，否则会被误报为"第三方未知文件"。
3. **上游 zip 里 34 字节 `addons/metamod/RayTrace.vdf` 残留照常安装**——上游自己就发这个文件
   （内容只是 URL，metamod 忽略），"跟随上游"即原样装入。
4. **本次只在本地提交、不推送**，除非明确要求推送。
5. **安装管理页放设置 → 安装域内（扩展 InstallationPage），不加一级侧边栏入口。**
6. **合并 staging 缓存策略**：`update_root()/merged/<tag>/`，LA payload 变化时重合并
   （以 LA 包 manifest 文件 mtime+sha256 为失效键），避免每次安装都重解压 73MB。

---

## 12. 风险登记簿

| # | 风险 | 影响 | 缓解 |
|---|---|---|---|
| R1 | 上游 zip 完整性：API 路径靠 `assets[].digest` 校验，**回退重定向与手动导入路径无校验** | 回退路径下载被投毒可写入游戏目录 | API 路径 digest 校验（决策 #5 升级）+ 仅 HTTPS + `validate_https_github_url` + 大小上限 + zip-slip 防护；残余风险 = "API 响应本身被投毒"（与 URL 同源）+ 回退路径，已与用户确认接受 |
| R2 | GitHub API 匿名限流 60 次/时/IP | 解析失败（回退路径无 digest 校验） | 403/429 回退 `releases/latest/download` 重定向；6h 本地缓存（`CACHE_SECONDS` 同款） |
| R3 | `payload_root()` 切换到 merged 漏改调用点 | 漏装/检查失真 | `grep -n "payload_root()" Panel/src-tauri/src/lib.rs` 逐行对照 6.3 全量表（含评审补入的 3095/3097、3122、2991/3024）；9.2 场景 6/7 实测 |
| R4 | restore 语义：合并 manifest 的上游条目 backup 数据量大（73MB 全量备份） | state 目录膨胀 | 上游条目默认 `restore`；可考虑上游条目改 `replace`（不备份、卸载即删）——**实施时与用户确认此点** |
| R5 | 上游 zip 结构变化（新增顶层目录/改名/整体包一层） | 合并失败或静默丢内容 | **merge 入口阻断式 payload 探测**（6.2 步骤 2，对齐 `package.ps1:141-147`：找含 addons+cfg 的目录，找不到即报错）+ 平铺；`import_local` 沿用同一探测；步骤 5 未知顶层条目 WARN 兜底 |
| R6 | `Panel v1.4.5.exe` 文件名随上游版本变化 | 管理页启动入口失效 | glob 匹配 `Panel v*.exe` + state 记录绝对路径 |
| R7 | CI 构建不再编译 8 个插件 → verify 的"包内 DLL==本地构建"断言失效 | 漏检 | 白名单反转（5.3）替代逐 DLL 比对 |
| R8 | i18n 28 语言块新增键 | 翻译不全 | 确认 fallback 链；en/schinese/tchinese 全量，其余按现有稀疏惯例 |
| R9 | `test-install-gate.mjs` 改签名破坏必跑测试 | CI 红 | 改动同提交内更新断言（7.5） |
| R10 | 合并操作与组件更新并发 | 文件锁冲突 | 共用 `online_update::OperationGuard` |

---

## 13. 术语对照

| 术语 | 含义 |
|---|---|
| 上游 / upstream | ed0ard/CS2-Bot-Improver（v1.4.5 = `d991445`） |
| LA / Plus | Local-Arena 本仓库与本产品 |
| 完整包 / full zip | `LocalArena-vX-windows.zip`（改造后只含 LA 自有内容） |
| plugin zip | `LocalArena-plugin-vX-windows.zip`（更新通道用，仍是 manifest 驱动的 payload） |
| 合并 payload / merged | 上游 zip 解压 + LA payload 叠加 + 合并 manifest 生成后的安装源 |
| 叠加 | 后写覆盖：LA 文件覆盖上游同名文件（现状 236-246 的镜像，方向与今天一致） |
