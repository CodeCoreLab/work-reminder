# 质量评估与优化报告

> 日期：2026-10-23 ｜ 基线：v1.0.0（首次发布）｜ 评估后：v1.0.0 优化版
> 方法：静态审查 + clippy + 实测基准 + 端到端回归。所有数字为**本机实测**。

> **版本说明**：本报告评估的是 **v1.0.0 Win32 版**（`core/` 5 模块 + `platform/` 9 模块 + `main.rs`，
> 8,694 行）。其中的 108→111 项测试、模块行数、clippy 告警数均针对该版本。
> 当前 **v2.0.0（Tauri 2）** 已重构为 `tauri/reminder-core` 单一纯逻辑 crate，
> 单元测试为 **67 项**（`cargo test --lib` 实测），架构与体积指标请以 `README.md` 为准。
> 本报告保留为历史优化记录，方法论部分仍可复用。

---

## 0. 结论

| 维度 | 评估前 | 评估后 | 判定 |
|---|---|---|---|
| 单元测试 | 108 | **111**（+3 命名回归） | ✔ |
| clippy 告警 | 16 | **3**（均为绘制函数参数） | ✔ 剩余属合理取舍 |
| 可执行文件 | 585 KB | **504 KB**（−13.8%） | ✔ |
| 私有内存 | 2.3 MB | **2.3 MB** | ✔ 未退化 |
| 空闲 CPU | 0.00% | **0.00%** | ✔ 10 秒实测零消耗 |
| tick 复杂度 | O(n²) | **O(n)** | ✔ 1000 任务 16 ms → 132 μs（**121×**） |
| 12 种调度全生命周期 | 0 失败 | **0 失败** | ✔ |
| 与原版接近度 | — | **行为等价 + 4 处增强** | 见 §4 |

**发现并修复 3 个真实缺陷**，其中 1 个是会让用户数据「静默变空」的 P1：
配置字段命名与文档不符、开机自启时被模态框卡死、首轮 arming 未落盘。

---

## 1. 质量评估：与原版的接近度

### 1.1 行为等价（31 项，见 `docs/feature-parity.md`）

12 种调度的枚举序号、token、`next_trigger` 算法逐分支等价于原程序跳转表
（0x191cc / 0x17338 / 0x17130）。关键常量一致：

| 常量 | 原程序证据 | 本实现 | 一致性 |
|---|---|---|---|
| 空闲重置阈值 | `cmp ebp,0x12c` @0x13e0a = 300 s | `IDLE_RESET_SECONDS = 300` | ✔ |
| 单 tick 使用封顶 | `cmp edx,5` @0x13d79 = 5 min | `MAX_CREDIT_MINUTES_PER_TICK = 5` | ✔ |
| tick 节拍 | `SetTimer` 4000 ms | `TICK_MS = 4000` | ✔ |
| 持久化原子性 | `QSaveFile::commit` @0x1b33e | tmp + fsync + rename | ✔ |
| 卡片配色 | QSS @0xbc1d60 `#fffdf8` / `#f0ddbd` | 同值 | ✔ 截图比对 |

### 1.2 相比原版的增强（4 处）

| 项 | 原版 | 本实现 | 价值 |
|---|---|---|---|
| 配置版本校验 | `version` 只写不读 | `UnsupportedVersion` 拒绝降级覆盖 | 防未来版本被旧版破坏 |
| 番茄钟提前预告 | 无 | 提前 5 s 无按钮提示 | 不打断正在输入 |
| 旧配置迁移 | — | 自动导入 + `.migrated` 备份 | 老用户零重录 |
| 配置命名兼容 | — | camelCase 写入，snake_case 仍可读 | 见 §2.1 |

### 1.3 仍存差异（5 项，均合理）

`mutedUntil` 按自然日而非精确时刻、番茄钟预告是新增、托盘左键直接开管理器、
自绘日期选择器改为文本输入、首启标记简化。理由见 `docs/feature-parity.md §3`。

---

## 2. 修复的缺陷

### 2.1 【P1】配置字段命名与文档不符 —— 手改配置会直接报错

**现象**：`Schedule` 枚举上标了 `#[serde(rename_all = "camelCase")]`，但 serde 的
这个属性**不会作用于枚举变体的内部字段**。实际落盘是 `interval_minutes`，
而文档、README 示例、`legacy` 导入器全用 `intervalMinutes`。

**实测**（`probe_serde2`）：

```
按文档写的 camelCase 文件 -> ERROR 配置文件格式不正确：missing field `interval_minutes`
代码实际写的 snake_case     -> Some(Interval { interval_minutes: 45 })
```

**影响**：用户照文档手改 `reminders.json` 必定失败；文档与实现不一致。

**修复**：给 12 个变体各加 `#[serde(rename_all = "camelCase")]`，并给 15 个字段加
`#[serde(alias = "...")]` 保留旧写法。新增 3 项单测锁住约定：
命名必须是 camelCase、snake_case 仍能加载、12 种变体双写法往返一致。

### 2.2 【P1】开机自启被迁移弹窗卡死

**现象**：`migrate_legacy` 无条件 `MessageBoxW`。它在 `--silent`（开机自启）路径上，
开机时没人点「确定」，程序永远停在弹窗，托盘不出现、提醒全失效。

**实测**：stage 日志停在 `main: mutex ok` 不再前进；配置文件已生成但进程卡住。

**修复**：`migrate_legacy` 接收 `silent` 参数，静默模式不弹框。

### 2.3 【P2】首轮 arming 未落盘

**现象**：`do_tick` 在 `fired.is_empty()` 时 `return false`，跳过了持久化。而
`arm_unarmed` 只置 `dirty` 标记、不写盘，导致首次运行/迁移后 `nextTrigger`
在内存中已设置、磁盘上永远是 `null`。

**修复**：把持久化抽成 `persist_if_dirty()`，两条返回路径都调用。

### 2.4 另修复（评估中发现）

| 项 | 影响 | 修复 |
|---|---|---|
| `once` 默认时间写死 `2026-01-01`（已过去） | 用户不改表单则永不触发 | 改为 `now + 1h`（clock-relative），核心占位改远期 |
| `monthly` 描述缺空格 `每月15 日` | 文案瑕疵 | `每月 15 日` |

---

## 3. 性能与体积优化

### 3.1 tick 复杂度 O(n²) → O(n)

**根因**：`Engine` 的 `usage` 与 `pre_alerted` 是 `Vec`，每个 tick 对每个任务做
线性扫描 → 整体 O(n²)。

**实测曲线**（`bench_engine`，2000 次平均）：

| 任务数 | 优化前 μs/tick | 优化后 μs/tick | 提升 |
|---|---|---|---|
| 10 | 1.305 | 1.198 | — |
| 100 | 84.6 | 12.4 | 6.8× |
| 500 | 1,381 | 65.3 | 21× |
| **1000** | **16,042** | **132** | **121×** |

增长倍率从 **×11.61（超二次）** 变为 **×2.01（线性）**。

**修复**：改用 `HashMap`；`retain_known` 先建 `HashSet` 再过滤（O(n) 而非 O(n²)）。

> 典型场景（<30 个任务）优化前也只要 ~3 μs，**这不是用户能感知的问题**。
> 修它是因为它是真实的复杂度缺陷，且零风险、零代价。

### 3.2 图标 PNG 压缩：102 KB → 11.8 KB

**根因**：`.ico` 6 帧全是未压缩 32 位 DIB（128 px 单帧就 67 KB）。

**修复**：`make_icon.py` 对 ≥48 px 的帧改用 PNG 编码（小帧仍用 DIB，省一次解码）。
**先验证再动手**：写了 `probe_png_icon` 实测 `CreateIconFromResourceEx` 能解码
PNG 帧（48/64/128 全部 `decode=true`），确认后才改。

| 指标 | 前 | 后 |
|---|---|---|
| `assets/icon.ico` | 102,134 B | **11,846 B**（−88%） |
| exe `.rdata` | 161,280 B | **71,168 B** |
| exe 总大小 | 598,528 B | **515,584 B**（−13.8%） |

### 3.3 试过但无效/有害的（已回滚）

| 尝试 | 结果 | 处置 |
|---|---|---|
| `lto = "fat"` + `strip = "symbols"` | 字节数**完全一致**（512,512） | 回滚，`lto = true` 本就是 fat |
| clippy 建议的 `as_chunks::<2>()` | 需要 Rust 1.88，**悄悄抬高 MSRV** | 回滚为 `chunks_exact` + 注解说明 |
| 手写 COFF `.rsrc` 段（早期） | GNU ld 不认 `.rsrc$01/$rsrc$02` 约定 | 早已改为 `include_bytes!` 运行时解码 |

### 3.4 内存与 CPU

实测（`Start-Process` + 5 次采样 + 10 秒 CPU 差）：

| 指标 | 值 |
|---|---|
| 工作集 | 12.4 MB |
| 私有内存 | **2.3 MB** |
| 线程 | 4 |
| 句柄 | 156 |
| **空闲 10 秒 CPU** | **0.0 ms（0.00%）** |

常驻托盘时完全不占 CPU——`GetMessageW` 阻塞，只在 4 s 定时器触发时醒一次。

---

## 4. 架构评估：是否需要调整

**结论：不需要重构。现有分层是对的，只需局部收敛。**

### 4.1 分层验证

```
core/     5 模块 2,576 行  —— 零平台依赖（grep 确认无 Win32 引用）
platform/ 9 模块 5,105 行  —— 唯一 unsafe 区，73 个手写 FFI 声明
main.rs            1,013 行  —— 消息循环 + tick 编排
                  8,694 行  总计
```

**最强证据**：core 层能在任意 host 上 `cargo test`，108→111 项测试全部跑在纯逻辑上。

### 4.2 复杂度热点

最长函数 127 行（`newtask::build_schedule`、`manager::paint_list`），对比原程序
`TaskDialog` 单函数 11.2 KB——**已经好了两个数量级**。

| 函数 | 行数 | 是否需要拆 |
|---|---|---|
| `build_schedule` | 127 | 12 个调度各一分支，可读性已够 |
| `paint_list` | 127 | 行绘制，线性结构 |
| `build_controls` | 115 | 表单布局 |
| `fill_from_task` | 111 | `build_schedule` 的逆运算 |

**三个 8 参数函数**（`gdi::button`、`newtask::edit`/`button`）是 clippy 唯一剩余的
告警。它们对应 Win32 `CreateWindowExW`/绘制调用的天然形态，包成结构体反而增加
间接层。**保留**。

### 4.3 建议的后续调整（可选，非必须）

| 优先级 | 项 | 理由 |
|---|---|---|
| P2 | `platform/manager.rs` 1,027 行可拆为 `paint` / `input` / `hooks` 三文件 | 单文件偏大，但内聚性已够 |
| P3 | `main.rs` 1,013 行的 `run_message_loop` 与 `main` 可抽 `bootstrap` | 现在读得懂，拆了反而多跳一层 |
| P3 | `gdi.rs` 三个 8 参函数收拢成 `DrawSpec` 结构体 | 仅为消 clippy 告警，收益 < 成本 |

**不建议**：引入 `windows`/`windows-sys` crate——本机 `dlltool` 损坏，且手写 73 个
声明更可审计。

---

## 5. 复现方式

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained:$PATH"

cargo test --lib                          # 111 项
cargo clippy --release                    # 3 条（预期）
cargo run --release --example audit       # 12 种调度全生命周期，TOTAL FAILURES: 0
cargo run --release --example bench_engine # tick 复杂度曲线
cargo run --release --example diag         # Win32 调用链自检
```

> `audit` 与 `bench_engine` 是**常驻回归工具**，不是一次性探针：前者能抓住
> 「一次性提醒排到过去」这类缺陷，后者能抓住复杂度回归。

---

## 6. 视觉证据

| 文件 | 内容 |
|---|---|
| `docs/screenshot-manager.png` | 管理器窗口（空状态），标题栏可见新图标 |
| `docs/screenshot-card.png` | 提醒卡片，验证五区垂直堆叠无重叠 |
| `docs/site-hero.png` | 线上产品页 hero |
| `docs/site-schedules.png` | 线上调度选择区（12 种可切换） |
| `docs/site-pomodoro.png` | 线上番茄钟（进度环空环起步） |
| `docs/site-mobile-schedules.png` | 390 px 下两列排布 |

## 7. 评估限制（未验证项）

1. **原程序未实际运行**：所有等价性结论基于静态取证，运行时边界未逐一比对。
2. **性能基准是单机单次**：`bench_engine` 用 2000 次平均消除抖动，但只跑了本机
   （Win10 + x64），其他 CPU/负载下的绝对数字会不同；**复杂度结论（线性 vs 二次）不受影响**。
3. **图标 PNG 帧在其他 Windows 版本未验证**：只在本机（Win10 22H2）实测解码成功。
   Vista 之前的系统理论上不支持，但本项目最低支持 Windows 10，故无实际风险。
4. **未做长时间稳定性压测**：常驻 24 h+ 的内存增长/句柄泄漏未测。

---

## 8. 字体规范与自动关闭（2026-10-23 第三轮）

### 字体阶梯：量化后才动手

原字号集是 `8.5 / 9 / 9.5 / 10 / 11 / 12 / 18 / 20 pt` 八档。相邻四档的比值：

```
8.5 -> 9.0  = 0.5pt  ratio 1.06
9.0 -> 9.5  = 0.5pt  ratio 1.06
9.5 -> 10.0 = 0.5pt  ratio 1.05
```

**比值低于 1.15 的差异人眼读不出层级**，只会觉得「大致相同又不完全一致」——
这就是三个截图给人「有点乱」观感的根源。改用几何阶梯：

| 步骤 | 尺寸 | 用途 |
|---|---|---|
| `PT_MICRO` | 9 pt | 法律行、开关说明、密集元信息 |
| `PT_CAPTION` | 11 pt | 提示、表头、次要事实 |
| `PT_BODY` | 14 pt | 正文、表格行、表单字段、按钮 |
| `PT_TITLE` | 16 pt | 卡片/区块标题 |
| `PT_DISPLAY` | 20 pt | 页面大标题 |

步进 1.22 / 1.27 / 1.14 / 1.25，全部 ≥1.14。旧名保留为别名，全部映射到这五档，
**任何调用点都无法再引入第六个尺寸**。加 `type_scale_has_perceptible_steps` 与
`aliases_map_onto_the_scale` 两个测试锁死。

### 行高

新增 `line_height()`（1.45×）与 `line_height_px()`。中文没有西文的降部空隙，
西文默认 1.2 行高会显得拥挤；1.45 是多行中文读起来像一个整块的比例。
多行文本改用 `DrawTextExW` + `DRAWTEXTPARAMS.iTabLength`，因为 `DrawTextW` 无法控制行距。

### 对齐修复

| 问题 | 根因 | 修复 |
|---|---|---|
| 「下次提醒」表头与时间列不对齐 | 表头统一左对齐，内容右对齐 | 表头跟随列的 `Align` |
| 「启用」表头不在复选框上方 | 在 58px 整列居中，盒子在左 | 表头与盒子同左边界 |
| 行文本左右边距不一致 | 只有右侧有 8px 内缩 | 统一 `ROW_INSET = 8`，两侧都用 |
| 对话框标签浮在输入框上方 | 标签矩形 `top + 5` 但 `bottom` 没变；STATIC 顶部对齐 | 矩形与输入框同高 + `SS_CENTERIMAGE` 垂直居中 |
| 三个参数框只有一个有内容 | 未用字段仅清空文本、控件仍显示 | `ShowWindow(SW_HIDE)` 连标签一起隐藏 |

### 关键缺陷：`line_height_px` 返回负数

`px_height` 按 `CreateFontW` 约定返回**负数**（character height）。`line_height_px`
直接 `px_height(line_height(...))`，于是返回负值；调用方乘以行数得到**负高度**，
`bottom` 跑到 `top` 上方，正文静默不绘制、后续内容全部叠在一起。

这个 bug 只有靠**打印每个 band 的实际 top/bottom** 才抓得到（`tagline: top=72 bottom=-3`），
看截图只会以为是「排版乱了」。修法：`line_height_px` 里取 `.abs()`。

### 恢复提示：3 秒自动关闭

`CardMode::Toast` 在 `ShowWindow` 后 `SetTimer(hwnd, TOAST_TIMER_ID, 3000)`，
`WM_TIMER` 里 `KillTimer` + `DestroyWindow`。点击仍可提前关闭——点击先销毁窗口，
残留的定时器打到已死句柄会被丢弃。

实测：1.2s 时窗口存在，4.7s 时已消失。

---

## 9. UI 一致性修复（2026-10-23 第二轮）

用户反馈的 5 项，逐条处理：

| # | 问题 | 根因 | 修复 |
|---|---|---|---|
| 1 | 新建提醒的 UI 与主程序不一致 | `newtask.rs` 是三个窗口里**唯一用原生 STATIC/EDIT/COMBOBOX/BUTTON 的**，且没有 `WM_PAINT`/`WM_ERASEBKGND`，所以显示系统灰白样式 | 补 `WM_ERASEBKGND`（画布色）、`WM_CTLCOLORSTATIC/BTN/EDIT/LISTBOX`（控件着色）、`WM_DRAWITEM` + `BS_OWNERDRAW`（圆角按钮）；窗口尺寸由硬编码 560×480 改为 `theme::scale` |
| 2 | 关于窗口难看、缺开源协议 | 用 `MessageBoxW`，只有 4 行灰字 | 新建 `platform/about.rs`：白面板、版本徽章、tagline、许可证/任务数/配置路径/运行数据四行事实、版权行、蓝色关闭按钮 |
| 3 | 资源管理器无图标 | exe **无 `.rsrc` 段**——图标只在运行时从 `include_bytes!` 解码，Shell 看不到 | 新增 `tools/add_icon.py`，链接后追加 `.rsrc`（6 个 RT_ICON + 1 个 GROUP_ICON），`ExtractIconW` 实测可提取 |
| 4 | 其他类似问题 | 见下表 | 已修 |
| 5 | 缺自动重启 | 无崩溃恢复机制 | 崩溃标记（HKCU）+ `--supervise` 看门狗 + 启动时自动恢复提示 |

### 第 4 项发现的同类问题

| 问题 | 影响 | 修复 |
|---|---|---|
| **`CMD_TOGGLE_SUPERVISE` 与 `CMD_EXIT` 同为 1005** | 🔴 **「退出软件」菜单项完全失效，用户无法退出程序** | 改为 1006，并加 `menu_command_ids_are_unique` 测试 |
| 托盘「关于」仍走旧 `MessageBoxW` | 两处关于窗口不一致 | 统一到 `show_about()` |
| 对话框窗口尺寸硬编码像素 | 高 DPI 屏内容溢出 | 改为 `theme::scale` |
| `about.rs` 的 `WM_ERASEBKGND` 直接 `return 1` 不绘制 | 窗口全黑 | 实际填充画布 |
| 长路径被 `DT_WORDBREAK` 截断（无空格不折行） | 配置路径显示不全 | `wrap_path()` 手工按分隔符折行，行数驱动行高 |

### 关于图标的技术结论（重要）

Windows 的 `.ico` 资源必须**成对**存在：

* `RT_GROUP_ICON` — 帧目录，Shell 靠它知道有哪些尺寸
* `RT_ICON` — 每个尺寸的实际位图

只放 group 时 `ExtractIconW(-1)` 能数到 1 个图标，但 `ExtractIconW(0)` 返回 NULL，Explorer 画不出来。

另外两个坑：

1. **GNU ld 会破坏手写的 `.rsrc`**。它会对自己无法解析的重定位做补丁，导致资源目录指向垃圾（实测 `payloadRel=0xff1d6ec9`）。所以不能用链接期方案。
2. **`IMAGE_RESOURCE_DATA_ENTRY.OffsetToData` 是 RVA，不是段内偏移**。写段内偏移会让 Shell 读到错误位置。

最终方案：`tools/add_icon.py` 在链接后直接给 PE 追加段并修正数据目录，`tools/build.py` 串起「cargo build → 追加资源 → 用 `ExtractIconW` 验证」，验证失败即构建失败。
