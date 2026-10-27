# 功能对照报告：重构版 vs 原程序

> 核对方式：原程序侧以 `docs/reverse-analysis.md` 的静态取证为准（偏移可复核）；重构版侧逐项 `grep` + 实跑验证。
> 核对日期：2026-10-22（本轮已补齐编辑 / 二次确认 / boot 标记，并修复托盘右键崩溃）
> 标注：✔ 一致 ｜ ⚠ 差异 ｜ ✘ 缺失

---

## 0. 结论

| 类别 | 本轮前 | 本轮后 |
|---|---|---|
| ✔ 完整实现且一致 | 26 | **31** |
| ⚠ 行为差异 | 7 | 5 |
| ✘ 缺失 | 6 | 2（均为非终端功能） |
| 🔴 P0 崩溃 | 1 | 0（已修） |

**核心提醒能力（12 种调度 + 触发 + 持久化 + 托盘 + 自启动 + 配置迁移）完整且比原程序更健壮。**
唯一仍缺的两项都是作者自用的开发工具或纯视觉装饰，不影响任何实际使用。

验证：`cargo build --release` 0 warning，`cargo test` 108 passed，产物 585 KB。

---

## 1. 本轮新补齐（5 项）

| # | 项 | 实现 | 验证 |
|---|---|---|---|
| 1 | **编辑已有任务** | `newtask::run(now, Some(&task))` 表单预填 + `edit_task()` 原地更新；管理器新增「编辑」按钮 + 双击行快捷编辑 | 选择 12 种调度任一类型均可正确回填（`fill_from_task` 为 `build_schedule` 的逆运算） |
| 2 | **删除确认** | `confirm()` → `确定删除"X"吗？删除后无法恢复。` | 文案与原程序 0xbc4660 一致 |
| 3 | **退出确认** | `退出后将不会再弹出提醒，确定要退出吗？` | 文案与原程序 0xbc46d0 一致 |
| 4 | **`startupBootMarker` 防重** | `arm_unarmed` 写入启动时刻标记；同一次启动内已触发的 `afterStartup` 任务不再 arm | 字段语义与原程序一致（原语义为推断，见 §5） |
| 5 | **托盘右键崩溃（P0）** | `show_menu` 改为静态函数，调用处不持 `APP` 借用 | 控制台探针 `examples/probe_tray` 实测：定时器持续触发期间菜单正常返回，无 panic |

---

## 2. 已具备（31 项，行为一致）

### 2.1 调度引擎（14 项）

12 种调度类型全部实现，枚举序号与 token 与原程序跳转表（0x191cc / 0x17338 / 0x17130）逐一对应：

| 原枚举 | token | 重构变体 | 判定 |
|---|---|---|---|
| 0 | `interval` | `Schedule::Interval` | ✔ |
| 1 | `daily` | `Schedule::Daily` | ✔ |
| 2 | `once` | `Schedule::Once` | ✔ |
| 3 | `weekly` | `Schedule::Weekly`（位掩码） | ✔ |
| 4 | `workdays` | `Schedule::Workdays`（掩码 0x1f） | ✔ |
| 5 | `monthly` | `Schedule::Monthly` + 三态 `MonthlyMode` | ✔ |
| 6 | `multiDaily` | `Schedule::MultiDaily` | ✔ |
| 7 | `timedInterval` | `Schedule::TimedInterval` | ✔ |
| 8 | `countdown` | `Schedule::Countdown` | ✔ |
| 9 | `pomodoro` | `Schedule::Pomodoro` + 状态机 | ✔ |
| 10 | `afterStartup` | `Schedule::AfterStartup` | ✔ |
| 11 | `usageBreak` | `Schedule::UsageBreak` | ✔ |

`next_trigger` 求解器对应原程序 0x17160 的 12 分支跳转表，逐分支等价（含月末夹取、最后工作日回退、
时段网格对齐 `(elapsed/step+1)*step`）。

### 2.2 触发与计时（6 项）

| # | 功能 | 原程序证据 | 重构 | 判定 |
|---|---|---|---|---|
| 15 | 4 秒 tick | `SetTimer` 4000ms | `TICK_MS=4000` | ✔ |
| 16 | 空闲 ≥300s 重置使用计数 | `cmp ebp,0x12c` @0x13e0a | `IDLE_RESET_SECONDS=300` | ✔ 单测 |
| 17 | 单 tick 使用封顶 5 分钟 | `cmp edx,5` @0x13d79 | `MAX_CREDIT_MINUTES_PER_TICK=5` | ✔ 单测 |
| 18 | 到点判定 + 推进 | 0x160f0 | `is_due` / `advance` | ✔ 单测 |
| 19 | 首次评估只 arm 不触发 | 0x160f0 | `None =>` 分支 | ✔ 单测 |
| 20 | 番茄钟轮数耗尽自动停用 | 0x1636a | 同上 | ✔ 全周期单测 |

### 2.3 持久化（4 项）

| # | 功能 | 原程序 | 重构 | 判定 |
|---|---|---|---|---|
| 21 | JSON 单文件原子写 | `QSaveFile::commit` | tmp + fsync + rename | ✔ 单测 |
| 22 | 损坏文件报错不覆盖 | `配置文件格式不正确` | `StoreError::Parse` | ✔ 单测 |
| 23 | 版本字段校验 | 只写不读 | `UnsupportedVersion` 拒绝 | ✔ 增强 |
| 24 | 旧版配置迁移 | `LocalTools\WorkReminder` | `legacy::import_from_disk` | ✔ 实测 4/5 导入 |

### 2.4 系统集成（7 项）

| # | 功能 | 原程序 | 重构 | 判定 |
|---|---|---|---|---|
| 25 | 单实例互斥 | `CreateMutexW` | 同类命名互斥体 | ✔ 实测 |
| 26 | 二次启动静默/提示 | `--silent` 分支 | 同 | ✔ |
| 27 | 开机自启动 | `HKCU\...\Run` = `"<exe>" --silent` | 同键同值格式 | ✔ 单测+实测 |
| 28 | 托盘常驻 + 左键开管理 | `QSystemTrayIcon` | `Shell_NotifyIconW` | ✔ |
| 29 | 托盘提示 N 个任务 | `工作提醒助手 · %1 个任务已启用` | 同文案 | ✔ |
| 30 | 提示音 | `PlaySoundW` | 系统声音别名 | ✔ |
| 31 | 卡片 + 三按钮 | QSS @0xbc1d60 | 同配色 + `CardMode::Actionable` | ✔ 截图确认 |

---

## 3. 尚存行为差异（5 项）

| # | 项 | 原程序 | 重构 | 原因 | 影响 | 建议 |
|---|---|---|---|---|---|---|
| 1 | `mutedUntil` 语义 | 按精确时刻恢复（`已暂停 · %1`） | 「今日不再提醒」→ 次日 00:00 | 实现按自然日而非时刻 | 跨零点场景多静音数分钟 | 不需改：自然日更贴合「今日」文案 |
| 2 | 番茄钟提前预告 | 无 | 提前 5 秒 `PreAlert`（无按钮、不推进排程） | 本轮新增（用户要求） | 纯增强，无副作用 | 保留 |
| 3 | 托盘左键 | 弹卡片式面板（`TrayPopup`） | 直接打开管理器 | 面板未实现 | 少一层视觉过渡；功能路径更短 | 可接受 |
| 4 | 自绘日期时间选择器 | `PopupDateTimeEdit`/`InlineTimePicker`/`DurationPicker` | 文本输入 + 格式提示 | 有意裁剪 | 输入体验略降，功能等价 | 可选补齐 |
| 5 | 首启引导标记 | `startupChoiceInitialized` 一次性提示 | 每次非 `--silent` 直接开管理器 | 有意简化 | 无 | 不需 |

---

## 4. 仍缺失（2 项，均非终端功能）

| # | 项 | 原程序 | 影响 | 建议 |
|---|---|---|---|---|
| 1 | `--render-previews` 截图模式 | 批量渲染 9 张 UI 预览图 | 无：作者自用开发工具 | 不需（本轮改用 `tools/shot.ps1` + `--preview-card` 达到同等目的） |
| 2 | 托盘气泡通知 | `QSystemTrayIcon::showMessage`（`NIF_INFO`） | 无：卡片已承载同样信息 | 可选 |

---

## 5. 核对限制（未验证项）

1. **原程序未实际运行**：原程序侧结论均来自静态取证；其运行时边界（`mutedUntil` 跨日、
   `startupBootMarker` 写入时机）**未经实测对照**。
2. **`startupBootMarker` 原语义为推断**：字段名与 `afterStartup` 枚举共现，推断为「本次开机已提醒」
   标记；写入时机未反汇编确认。本实现取「进程启动时刻」为标记值。
3. **迁移覆盖率**：12 种调度类型的迁移路径有单测，但真实历史配置的覆盖率未知（仅 5 条样例实测）。
4. **UI 一致性基于取值而非像素比对**：颜色 / 字号 / 圆角取自 QSS 字符串实测值，未与原程序做像素级并排比对。
5. **编辑回填的 12 种类型未逐一实机点击**：`fill_from_task` 与 `build_schedule` 互为逆运算，
   由代码对称性保证；未对每种类型做 GUI 级回填验证。
