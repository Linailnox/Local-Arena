# Panel 原生 UI 重构选型（探讨记录）

状态：**技术选型探讨，未立项、未实施**。记录于 2026-10-23，用于后续决策参考。

## 1. 背景与规模基线

目标是让 Panel 不再依赖类浏览器的 UI 前端（WebView），后端可保留也可重写。当前规模（本仓实测）：

| 指标 | 数值 |
| --- | --- |
| 前端 `.tsx` 文件 | 52 |
| 前端 TS/TSX 行数 | ≈ 12,100 |
| CSS 文件 / 行数 | 34 / ≈ 6,800 |
| i18n | `dictionary.ts` 2,249 行，约 29 个语言块（EN 为完备基准） |
| 后端 Rust | ≈ 16,260 行 |
| Tauri 命令 | 81 个 `#[tauri::command]`（经 `lib/api.ts` 单一边界） |
| 图表 / 图标 | recharts 用于 3 处；lucide 图标散在 22 个文件 |

界面形态：**表单 + 数据表格 + 分步向导 + 少量图表**，不是富媒体/动画驱动。这个判断决定了框架取舍（见 §4）。

## 2. 候选方案对照

| 方案 | 语言 | 强项 | 主要代价 |
| --- | --- | --- | --- |
| **Avalonia 11** | C# / .NET 10 | DataGrid/虚拟化/样式系统/XAML 最贴近现有"卡片+表格+向导"结构；图表有 LiveCharts2、OxyPlot、ScottPlot；透明无边框窗口；Windows + Linux(X11/Wayland) 成熟；`.NET 10` 已在用（MatchCore 测试依赖 net8 运行时） | 16k 行 Rust 要重写；81 个命令边界重新画 |
| **iced** | Rust | 后端零改动、无 IPC/序列化层、单一 cargo 构建（bun/tsc/vite 门禁整体消失）；wgpu 渲染，Win/Linux 一致 | 无 CSS：虚拟化表格、图表、主题 token、毛玻璃全部自建；前期主要在造轮子 |
| egui | Rust | 上手最快，内部工具够用 | 即时模式不适合大量文本 + 精细布局 + 无障碍；做不出玻璃质感 |
| Qt 6 + QML | C++ | 最成熟（表格/图表/输入法/无障碍/模糊齐备） | C++ 重写 + 交叉编译到 Windows 麻烦 + LGPL 动态链接约束；投入最大 |
| Slint | `.slint` DSL + Rust/C++ | 声明式、性能好、包体小 | 许可证需逐条核对（GPLv3/商业）；生态与图表弱 |
| Dioxus / Freya / GPUI / Floem | Rust | 名义上"Rust 版 React" | Dioxus desktop 默认仍走 WebView（等于没换）；Freya 实验级；GPUI 是 Zed 内部 API、无稳定文档；Floem 更小。均不建议上生产 |

## 3. 结论

- **可以重写后端时：Avalonia（C# / .NET 10）为首选。**
- **必须保留 Rust 后端时：iced 为唯一体面选择**（"Rust 后端 + Avalonia 前端"只能走 named pipe/本地 socket，等于把 81 个命令的通道手写一遍，长期是双语言双运行时，不推荐）。
- egui 仅适合内部工具；Qt 只在"要长期商业产品、预算充足"时考虑。

Avalonia 的隐性收益：`addons/` 四个插件已是 C#/.NET 10，Panel 也换成 C# 后全仓收敛为 **dotnet + cargo(可选) + pwsh**，`bun/tsc/vite/bun.lock` 整套门禁消失（5 步 → 3 步）；i18n 可由现有 `dictionary.ts` 生成 `.resx` 或继续读 JSON。
iced 的隐性收益：前后端同语言，`cargo check` 即全量类型检查；代价是 Avalonia 白送的 DataGrid/虚拟化/图表/样式继承全部自建。

**判断规则**：界面是"表单 + 表格 + 向导 + 少量图表"而非动画驱动 —— 这是 XAML 系主场、即时模式短板。所以只要后端重写不是红线，选 Avalonia；后端不可动，选 iced。

## 4. 无论选哪个都要重做的部分（按难度）

1. **统计家族虚拟化表格**：`StatsMatchDetail`、`StatsPlayerDetail`、`StatsMatchHistory` —— WebView 里是 DOM + CSS overflow，原生要自己做行回收。
2. **图表**：recharts 的 3 处（趋势折线、地图横向柱、回合曲线）。Avalonia 有现成库；iced 要接 plotters 或自绘。
3. **玻璃/模糊 + 主题 token**：`tokens.css` / `glass.css` 要翻译成各框架样式系统；Windows 走 DWM、Linux 无统一模糊方案，需降级为半透明。
4. **29 语言 i18n**：数据可复用（JSON/字典），机制需重做（Avalonia `resx`/`DynamicResource`；iced 用静态表 + 手动重绘）。
5. **无边框自定义标题栏 + 拖拽区**：需重新处理命中测试与 `-webkit-app-region` 的等价物。
6. **图标**：22 个文件在用 lucide，需换图标库或内联 SVG path。

## 5. 动工前的验证 spike（约 3 天）

- 选最难看的一屏（`StatsMatchDetail` 或 `SettingsInstallationPage`）在候选框架各写一遍，看**表格 + 长文本 + 图片**手感。
- 透明/无边框窗 + 拖拽 + 玻璃，在 Windows 与 Linux 各验一次（WebKitGTK 上踩过的坑，原生不会自动消失）。
- 一屏图表的等价物，确认图表库与主框架版本兼容（这类库跟主框架版本耦合经常出问题）。

## 6. 反方意见（务必保留）

如果真实痛点是 Linux 上 WebKitGTK 的毛病（devtools、Wayland/X11、`custom-protocol`），那重写是拿 6–9 个月换一个"Linux 桌面本来就被列为可选"的收益，不划算 —— 更便宜的做法是维持现状、只针对具体病症单点修。只有当痛点是 WebView 的启动时间、内存占用、质感配不上产品定位时，这次重写才值得。
