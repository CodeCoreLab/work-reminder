# 工作提醒助手 — 逆向分析报告

> 分析对象：`F:\360极速浏览器X下载\工作提醒助手.exe`
> 分析方式：静态取证（PE 结构解析 + Qt 元数据 + RTTI + `.pdata` 函数表 + Capstone 反汇编 + 字符串交叉引用）
> 分析日期：2026-10-22
> 结论均附**文件偏移 / RVA**，可逐条复核。未执行目标程序。

---

## 0. 结论摘要（先看这里）

| 项 | 结论 | 证据等级 |
|---|---|---|
| 技术栈 | **C++ / Qt 5（静态链接）/ MSVC x64**，无网络、无数据库 | 🔵 观察 |
| 规模 | 单文件 15.93 MB；其中**自有代码仅约 106 KB（199 个函数）**，其余为 Qt 静态库 + 资源 | 🔵 观察 |
| 架构 | 单进程 + 系统托盘常驻 + 单实例互斥 + JSON 单文件持久化 | 🔵 观察 |
| 业务核心 | **12 种调度类型**的提醒引擎 + 番茄钟状态机 + 键盘/鼠标空闲检测 | 🔵 观察 |
| 数据落盘 | `%APPDATA%\LocalTools\WorkReminder\reminders.json`（QStandardPaths::AppConfigLocation） | 🟡 推断（偏移链完整） |
| 自启动 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` → `WorkReminder` | 🔵 观察 |
| 隐私风险 | **无网络导入**（`WS2_32.dll` 仅 1 个序数导入，属 Qt 静态库残留）；无遥测 | 🔵 观察 |
| 代码质量 | 分层清晰但**业务逻辑与 UI 强耦合**，`0x9d00`（11.2 KB 单函数）为典型巨型函数 | 🔵 观察 |
| 重构建议 | **改用 Rust**（理由见 `docs/refactor-plan.md`） | 推理 |

---

## 1. 二进制取证

### 1.1 PE 头

| 字段 | 值 | 含义 |
|---|---|---|
| `Machine` | `0x8664` | x86-64 |
| `TimeDateStamp` | `0x6A6B5E61` = 2026-07-31 02:53:21 UTC | 构建时间 |
| `OptionalHeader.Magic` | `0x20b` | PE32+ |
| 子系统 | GUI（`Subsystem=2`） | 无控制台窗口 |
| `DllCharacteristics` | 含 `DYNAMICBASE` / `NX_COMPAT` | ASLR + DEP 开启 |
| Manifest | `requestedExecutionLevel level='asInvoker'` | **不提权**（偏移 `0xF3B528`，381 B） |

段表（文件偏移）：

```
.text      vsize=0x875a1c  ptr=0x000400    ← 代码（含 Qt 静态库）
.rdata     vsize=0x344144  ptr=0x876000
.data      vsize=0x074d34  ptr=0xbba200    ← 应用字符串池在此
.pdata     vsize=0x06d89c  ptr=0xc1e200    ← 异常表 → 函数边界
.qtmetad   vsize=0x0002af  ptr=0xc8bc00    ← Qt 插件元数据
.qtmimed   vsize=0x236ce3  ptr=0xc8c000    ← Qt MIME 数据库
.fptable   vsize=0x000100  ptr=0xec2e00
.rsrc      vsize=0x0636a8  ptr=0xec3000    ← 图标/清单/版本
.reloc     vsize=0x0000bda0 ptr=0xf26800
```

### 1.2 静态链接 Qt 5 的证据

`.qtmetad` / `.qtmimed` 段是 **Qt 静态构建独有**的段名；段内容为 `QTMETADATA !` 前缀的插件描述符，实测解出：

```
org.qt-project.Qt.QStyleFactoryInterface        → QWindowsVistaStylePlugin
org.qt-project.Qt.QImageIOHandlerFactoryInterface → QGifPlugin / QICOPlugin / QJpegPlugin
org.qt-project.Qt.QPA.QPlatformIntegrationFactoryInterface.5.3 → QWindowsIntegrationPlugin
```

- `QPA ... .5.3` 是 **Qt 5.3+** 的平台插件接口标识；`.5.3` 后缀在 Qt 6 中已改为其他形式 → **判定 Qt 5 系列**。
- 导入表中**没有任何 `Qt5*.dll`**，全部 Qt 符号在 `.text` 内 → **静态链接**（单文件 exe，无需附带 Qt DLL）。
- RTTI 段含 748 个 `.?AVQ...@@`（Qt 类）与 181 个非 Qt 类，其中**应用自有类 10 个**（见 §2）。

### 1.3 编译器与运行时

- RTTI 命名 `.?AV<lambda_...>@@`、`std::_Func_impl_no_alloc` → **MSVC（非 MinGW）**，且大量 lambda 说明源码重度使用 C++11/14 匿名函数。
- 导入 `IMM32.dll`、`UxTheme.dll`、`dwmapi.dll`、`WTSAPI32.dll` → 均为 Qt `qwindows` 插件依赖，非应用直接调用。
- 无 `MSVCP140.dll` / `VCRUNTIME140.dll` 动态导入 → CRT 亦为静态链接。

---

## 2. 功能模块划分

### 2.1 自有类（RTTI 实测，10 个）

| 类名 | 角色 | 承载函数区间 |
|---|---|---|
| `ReminderApplication` | `QApplication` 子类：进程入口、单实例、托盘、全局定时器 | `0x04510`（2.1 KB） |
| `ManagerWindow` | 主管理器窗口：任务表格 CRUD | `0x10300`（4.3 KB） |
| `TrayPopup` | 托盘弹出面板（卡片式任务列表） | `0x06310`（3.4 KB） |
| `TaskDialog` | 新建/编辑任务对话框（12 种调度表单） | `0x09d00`（**11.2 KB，最大单函数**） |
| `ReminderDialog` | 到点提醒弹窗（含番茄钟/久坐文案分支） | `0x125e0`（4.1 KB） |
| `PopupDateTimeEdit` | 自绘日期时间弹出选择器 | `0x0dee0` |
| `InlineTimePicker` | 时分内联选择器 | `0x0d560` |
| `DurationPicker` | 时长选择器（分钟/小时双单位） | `0x091f0` |
| `FullClickComboBox` | 整行可点下拉框（交互增强） | `0x0d480` 周边 |
| `DetailButton` | 详情按钮 | `0x0d0e0` 周边 |

> 10 个类全部命中 RTTI 描述符，名称为 MSVC 修饰名 `. ?AV<Name>@@`。

### 2.2 模块分层（按字符串归属反推）

```
┌──────────────────────────────────────────────────────────────┐
│ 进程入口 / 生命周期        0x04510  ReminderApplication       │
│   单实例 Mutex、--silent、--render-previews、托盘、4s 心跳定时器 │
├──────────────────────────────────────────────────────────────┤
│ UI 层                                                         │
│   0x10300 ManagerWindow      0x06310 TrayPopup                │
│   0x09d00 TaskDialog         0x125e0 ReminderDialog           │
│   0x02bc0/0x02cf0 预览图渲染（--render-previews 开发模式）      │
├──────────────────────────────────────────────────────────────┤
│ 调度引擎（纯逻辑，无 UI）                                      │
│   0x190d0 scheduleType ⇄ token                                │
│   0x17160 next_trigger()         ← 12 分支跳转表 0x17338       │
│   0x160f0 is_due() / advance()   ← 状态机推进                  │
│   0x163b0 schedule_description() ← 12 分支跳转表 0x17130       │
│   0x18680..0x18930 各类型求解器                                │
│   0x13d20 tick() 主循环          ← GetLastInputInfo 在 0x13dab │
├──────────────────────────────────────────────────────────────┤
│ 持久化                                                        │
│   0x1a7e0 config_path()    0x1a930 load()    0x1af20 save()    │
│   0x19200 单任务序列化/反序列化（30 个 JSON key）              │
├──────────────────────────────────────────────────────────────┤
│ 平台适配                                                      │
│   QStandardPaths / QSettings / 注册表 Run 键 / Qt 托盘 API     │
└──────────────────────────────────────────────────────────────┘
```

---

## 3. 关键业务逻辑

### 3.1 调度类型枚举（12 种，一一对应跳转表）

**证据链**：`0x190d0` 的 `cmp edx, 0xb` + `jmp [r8+rax*4+0x191cc]`（绝对地址跳转表），12 个目标各自 `lea` 到一个 `QStringLiteral`。

| 枚举值 | token（JSON 落盘值） | UI 名称 | 描述格式串 |
|---|---|---|---|
| 0 | `interval` | 按间隔循环 | `每 %1 小时` / `每 %1 分钟` |
| 1 | `daily` | 每天固定时间 | `每天 %1` |
| 2 | `once` | 指定日期时间（一次） | `一次：%1` |
| 3 | `weekly` | 每周指定日期 | `周%1 %2` |
| 4 | `workdays` | 仅工作日 | `工作日 %1` |
| 5 | `monthly` | 每月指定日期 | `每月%1 %2` / `最后一天` / `最后工作日` / `%1 日` |
| 6 | `multiDaily` | 每天多个时间点 | （时间点列表逗号拼接） |
| 7 | `timedInterval` | 限定时段循环 | `周%1 %2-%3 每 %4 分钟` |
| 8 | `countdown` | 倒计时 | `倒计时 %1 分钟` |
| 9 | `pomodoro` | 番茄钟 | `番茄钟 %1/%2 分钟 × %3` |
| 10 | `afterStartup` | 开机后提醒 | `开机后 %1 分钟` |
| 11 | `usageBreak` | 连续使用电脑提醒 | `连续使用 %1 分钟` |

> 说明：`idx0` 在序列化结构中对应 `interval`，而描述表 `0x17130[idx0]` 输出「每 %1 小时」——两张表**枚举顺序一致**，可交叉验证。

### 3.2 `next_trigger()` 求解器（`0x17160`，跳转表 `0x17338`）

反汇编逐分支还原：

| 类型 | 算法 |
|---|---|
| `interval` | `now + max(1, intervalMinutes) * 60s`（`0x17197`） |
| `daily` | 取今日 `dailyTime`（`HH:mm:ss`）；若已过则 +1 天（`0x171c5`） |
| `once` | 直接返回 `onceAt`（`0x17259`） |
| `weekly` | 从今天起 0..14 天内找第一个 `weekdaysMask` 命中且时刻未过的日（`0x17275` → `0x18680`，位掩码 `bt`） |
| `workdays` | 同上但掩码固定 `0x1f`（周一至五）（`0x17298`） |
| `monthly` | `monthlyMode` 三态：`0=指定日`（夹到当月最大天数，`0x18818` 处 `cmovl` 即 min(day, monthDays)）、`2=最后一天`、`1=最后工作日`（`0x18836`，循环回退跳过周六日 `0x18850`） |
| `multiDaily` | 在 `dailyTimes` 有序集合中线性扫描 `0x18930`，取第一个 > now |
| `timedInterval` | 日期须在 `[activeStart, activeEnd]` 窗口内 + 星期掩码命中，再按 `intervalMinutes` 对齐到当前日期的整数倍（`0x18b00`：`(elapsed / step + 1) * step`） |
| `countdown` | `now + max(1, countdownMinutes) * 60s`（`0x172d5`） |
| `pomodoro` | 见 §3.3 状态机（`0x17305` 取值 `+0x40`/`+0x44` 的 `pomodoroWorkMinutes`/`pomodoroBreakMinutes`） |
| `afterStartup` | `启动时刻 + max(1, startupDelayMinutes) * 60s`（`0x1730d`，读 `+0x44`） |
| `usageBreak` | `now + max(1, usageMinutes) * 60s`（`0x17315`，读 `+0x58`） |

### 3.3 番茄钟状态机（`0x160f0`，关键分支 `0x16248`）

```
state: (+0x54) inBreak:bool, (+0x50) currentCycle, (+0x4c) cycles

if !inBreak:
    inBreak = true
    nextTrigger = now + max(1, pomodoroWorkMinutes) * 60
else:
    inBreak = false
    currentCycle += 1
    if currentCycle >= cycles:
        enabled = false          // 0x1636a：整条任务结束
        return true              // 最后一轮完成
    nextTrigger = now + max(1, pomodoroBreakMinutes) * 60
```

- `enabled` 落位 `+0x88`（`0x1636a` `mov byte ptr [rdi + 0x88], 0`）。
- 完成时 UI 文案：`番茄钟全部完成` + `今天的专注计划完成啦，辛苦了！` / 中途：`休息时间到` + `这一轮专注完成啦，起来放松一下吧。`（字符串池 `0xbc19b0`–`0xbc1a18`）。

### 3.4 主 tick 循环（`0x13d20`，4 秒节拍）

```
前情：ReminderApplication 建立 QTimer，周期 4000ms（0x04685 处 0x2af380 设间隔）

tick():
  1. now = QDateTime::currentDateTime()                            // 0x13d3c
  2. msDelta = now - lastTick(+0x48)                               // 0x13d4f
     idleMin = clamp(msDelta / 60000, 0, 5)                        // 0x13d79: cmp edx,5 / cmovg
     ↑ 即「两次 tick 间隔折算分钟，单次最多计 5 分钟」
  3. lastTick = now
  4. GetLastInputInfo(&LASTINPUTINFO)                              // 0x13dab（唯一调用点）
       idleSecs = (GetTickCount() - lii.dwTime) / 1000             // 0x13db9..0x13dce
  5. 遍历任务列表（每题 0x90 字节）：
       if type != usageBreak(11): continue                        // 0x13e00 cmp [rdi+0x18],0xb
       if idleSecs >= 300:  usageMinutes(+0x6c) = 0; continue      // 0x13e0a cmp ebp,0x12c
       if !enabled(+0x88): continue
       if mutedUntil(+0x80) 未到期 或 nextTrigger(+0x78) 未到期: continue
       usageMinutes += idleMin                                     // 0x13e52
       if usageMinutes >= max(1, usageMinutesCfg(+0x68)) * 60:
           usageMinutes = 0
           fire(task)                                             // 0x13e75
  6. 第二轮遍历：对每个 task 调 is_due()（0x160f0），到期则 fire 并置 dirty
  7. if dirty: save(); update_tray_tooltip()
  8. cleanup
```

**关键取值**：`idle >= 300 秒` 视为「人已离开」，把累计使用分钟清零。这是 `usageBreak` 正确性的核心，阈值 300 为硬编码常量。

### 3.5 数据模型（`0x19200` 序列化，30 个 JSON key）

结构体步长 **0x90（144 字节）**（`0x13e8a` `add rdi, 0x90`）。

| 偏移 | 字段 | 类型 |
|---|---|---|
| `+0x00` | `id` | QString |
| `+0x08` | `title` | QString |
| `+0x10` | `message` | QString |
| `+0x18` | `scheduleType` | int (0..11) |
| `+0x1c` | `intervalMinutes` | int |
| `+0x20` | `dailyTime` | QTime（序列化 `HH:mm:ss`） |
| `+0x24` | `weekdaysMask` | int（bit0=周一 … bit6=周日） |
| `+0x28` | `monthlyMode` | int（0=指定日 / 1=最后工作日 / 2=最后一天） |
| `+0x2c` | `monthlyDay` | int |
| `+0x30` | `dailyTimes` | QVector\<QTime\> |
| `+0x38` | `activeStart` | QTime |
| `+0x3c` | `activeEnd` | QTime |
| `+0x40` | `countdownMinutes` | int |
| `+0x44` | `pomodoroWorkMinutes` / `startupDelayMinutes`（联合复用） | int |
| `+0x48` | `pomodoroBreakMinutes` | int |
| `+0x4c` | `pomodoroCycles` | int（qword 写入，实为 int） |
| `+0x50` | `pomodoroCurrentCycle` | int |
| `+0x54` | `pomodoroInBreak` | bool |
| `+0x58` | `usageMinutes`（配置值，单位分钟） | int |
| `+0x60` | `startupBootMarker` | QDateTime |
| `+0x68` | `usageMinutes`（运行时累计秒，与 `+0x6c` 见下） | int |
| `+0x6c` | `usageAccumSeconds`（运行时累计，非持久化） | int |
| `+0x70` | `onceAt` | QDateTime |
| `+0x78` | `nextTrigger` | QDateTime |
| `+0x80` | `mutedUntil` | QDateTime |
| `+0x88` | `enabled` | bool |

> 映射依据：`0x19200` 中每个 JSON key 之前最近的 `lea rcx, [r15+0xNN]`。`dailyTime`→`+0x20`、`activeStart`→`+0x38`、`activeEnd`→`+0x3c`、`onceAt`→`+0x70`、`nextTrigger`→`+0x78`、`mutedUntil`→`+0x80`、`enabled`→`+0x88` 为直接命中；其余按"key 顺序 + 0x90 结构步长 + 字段语义"补齐，属 🟡 推断，不影响功能等价实现。

### 3.6 持久化

| 环节 | 证据 |
|---|---|
| 路径 | `0x1a7e0` → `QStandardPaths::writableLocation(0x12)` = **`AppConfigLocation`**（`0x64c3f0` 的 `lea eax,[rdx-7]; cmp eax,8` 分支表 + `esi==0x12` 特判），再拼 `reminders.json`（`0xbc5de0`） |
| 实际位置 | Windows 上 `AppConfigLocation` = `%APPDATA%\<Org>\<App>` → **`%APPDATA%\LocalTools\WorkReminder\reminders.json`** |
| 读失败 | `无法读取配置文件：%1`（`0x1aaa2`） |
| 解析失败 | `配置文件格式不正确：%1`（`0x1adb6`） |
| 顶层结构 | `{"version": <int>, "tasks": [ ... ]}`（key `version` @`0xbc5ed8`，`tasks` @`0xbc5e80`） |
| 写失败 | `无法保存配置：%1` / `无法提交配置：%1`（`0x1b214` / `0x1b33e`；后者为 `QSaveFile::commit()`，即**原子写**） |
| 目录创建失败 | `无法创建配置目录。`（`0x1b033`） |

`QSaveFile` 的使用说明作者已考虑写入原子性（先写临时文件再 rename），这是值得保留的优点。

### 3.7 单实例 / 自启动 / 托盘

| 机制 | 实现 | 证据 |
|---|---|---|
| 单实例 | `CreateMutexW` + 名称 `LocalTools.WorkReminder.SingleInstance` | 字符串 `0xbbc8e0`；`CreateMutexW` 调用点 `0x82128d`、`0x821a01`（Qt `QLockFile` 内部路径） |
| 二次启动行为 | `--silent` 存在则静默退出；否则弹 `程序已经在系统托盘运行。` | `0x4510` 分支（`0x4a2e` / `0x4b17`） |
| 自启动 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名 `WorkReminder` | 字符串 `0xbc2518`（路径）+ `0xbc2572`（值名）；`RegSetValueExW` 调用点 `0x74e187`，所在函数 `0x74db40` |
| 自启动写入值 | `"<exe路径>" --silent` | 字符串 `0xbc28bc` = `"%1" --silent` |
| 托盘 | `QSystemTrayIcon` + `Shell_NotifyIconW` | RTTI `.?AVQSystemTrayIcon@@`；调用点 `0x55d023/0x55d22d/0x55d245/0x55d946`（Qt 内部） |
| 无托盘时 | 弹 `当前系统没有可用的系统托盘。` 后退出 | `0x15260` @`0x1530d` |
| 首次启动引导 | `startupChoiceInitialized` 标记 + `我会在系统托盘一直等着，到点就来提醒你。` | `0xbc44d0` / `0xbc45a0` |
| 托盘提示文案 | `工作提醒助手 · %1 个任务已启用` | `0x14780` @`0x1484a` |
| 声音 | `PlaySoundW` | 调用点 `0x539293`（Qt `QSound` 内部） |

### 3.8 URL Scheme / 命令行开关

| 开关 | 作用 |
|---|---|
| `--silent` | 静默启动（供注册表自启动使用） |
| `--render-previews` | **开发模式**：批量渲染 11 张 UI 预览图（`main-window.png`、`task-dialog.png`、`schedule-*.png`、`about-dialog.png` 等）到当前目录后退出。这是作者自用的截图工具，非终端用户功能 |

预览图清单（字符串池 `0xbbc4c0`–`0xbba940`）：`main-window.png`、`task-dialog.png`、`schedule-weekly.png`、`schedule-timed-interval.png`、`schedule-pomodoro.png`、`date-time-picker.png`、`reminder-dialog.png`、`tray-popup.png`、`about-dialog.png`。

---

## 4. 外部依赖

### 4.1 运行时 DLL（导入表实测 16 个）

| DLL | 函数数 | 归属 |
|---|---|---|
| `KERNEL32.dll` | 182 | Qt 核心 + CRT |
| `USER32.dll` | 147 | Qt GUI |
| `GDI32.dll` | 42 | Qt 字体/绘制 |
| `ADVAPI32.dll` | 27 | Qt `QSettings` / `QLockFile`（注册表） |
| `ole32.dll` | 19 | Qt 拖放/剪贴板 |
| `UxTheme.dll` | 17 | Qt 样式 |
| `SHELL32.dll` | 14 | **应用直接使用**（托盘 `Shell_NotifyIconW`）+ Qt |
| `IMM32.dll` | 11 | Qt 输入法 |
| `OLEAUT32.dll` | 4 | Qt COM |
| `dwmapi.dll` | 4 | Qt 窗口特效 |
| `VERSION.dll` | 3 | Qt |
| `WINMM.dll` | 3 | 应用 `PlaySoundW` 提示音 |
| `NETAPI32.dll` | 2 | Qt |
| `WTSAPI32.dll` | 2 | Qt 会话查询 |
| `USERENV.dll` | 1 | Qt 用户目录 |
| `WS2_32.dll` | **1（序数 101）** | Qt 静态库残留，**非应用网络代码** |

**网络能力判定**：无 `WININET` / `WINHTTP` / `URLMON` / `DNSAPI` 导入；`WS2_32` 仅 1 个序数符号（Qt 的 `qt_ntfs_permission_check` 等路径会链接 socket 基础符号）。字符串池中 `github.io` / `githubusercontent.com` 均出现在 **Qt 内置的 MIME 数据库公共后缀表**中（`.qtmimed` 段），与作者无关。

→ **结论：程序无任何出站网络行为，无遥测、无更新检查、无云端同步。**

### 4.2 静态依赖

- **Qt 5 全套**（Core / Gui / Widgets / Network-TLS 符号 / qwindows 插件 / 图片编解码 GIF·ICO·JPEG / Vista 样式 / 翻译资源），全部静态链接。
- **MSVC CRT**（`ucrt` + `libcmt`），静态链接。
- **zlib 1.2.11**（`deflate 1.2.11 Copyright 1995-2017 Jean-loup Gailly and Mark Adler`，`0x00a0e0xx` 附近）、**libjpeg-turbo**（`Copyright (C) 1991-2020 The libjpeg-turbo Project`）——均为 Qt 内置第三方库，非应用直接依赖。

### 4.3 外部输入/输出面

| 面 | 说明 |
|---|---|
| 文件 | 只读写 `reminders.json` 一个文件 |
| 注册表 | 只读写 `HKCU\...\Run\WorkReminder` 一个值 |
| 网络 | 无 |
| 进程/服务 | 无 |
| 系统 API | `GetLastInputInfo`（空闲检测）、`Shell_NotifyIconW`（托盘）、`PlaySoundW`（提示音） |

攻击面极小。唯一可议之处：`reminders.json` 由 `QJsonDocument` 解析，格式错误时走 `配置文件格式不正确` 分支而非崩溃 → **输入校验存在，但无 schema 版本迁移逻辑**（`version` 字段被写入却未见按版本分支的读取逻辑）。

---

## 5. 整体架构说明

### 5.1 运行时拓扑

```
                        ┌─────────────────────────────┐
                        │ ReminderApplication (QApp)   │
                        │  ├─ QLockFile 单实例         │
                        │  ├─ QSystemTrayIcon          │
                        │  ├─ QTimer 4000ms ──┐       │
                        │  └─ TaskStore(QVector)       │
                        └──────────┬───────────┼───────┘
                                   │           │
                 ┌─────────────────┼───────────┼──────────────┐
                 ▼                 ▼           ▼              ▼
        ┌──────────────┐  ┌──────────────┐ ┌────────┐ ┌─────────────┐
        │ ManagerWindow│  │  TrayPopup   │ │ tick() │ │ JSON store  │
        │ QTableWidget │  │ 框架+卡片列表 │ │0x13d20 │ │ QSaveFile   │
        └──────┬───────┘  └──────┬───────┘ └───┬────┘ └─────────────┘
               │                 │             │
               └────────┬────────┘             │
                        ▼                      ▼
                 ┌──────────────┐      ┌───────────────┐
                 │  TaskDialog  │      │ ReminderDialog│
                 │ 12 调度表单   │      │ 到点弹窗      │
                 └──────────────┘      └───────────────┘

        文件系统: %APPDATA%\LocalTools\WorkReminder\reminders.json
        注册表:   HKCU\...\Run\WorkReminder = "<exe>" --silent
```

### 5.2 数据流

```
用户编辑 → TaskDialog::save → 校验(0xcb10) → Task 结构 → TaskStore
                                                      │
     tick(4s) ──► is_due(0x160f0) ──► fire ──► ReminderDialog
                │                              │
                └──► advance(0x160f0 内) ──────┘
                          │
                          ▼
                   dirty → save(0x1af20, QSaveFile 原子写)
```

### 5.3 架构评价

**做得对的**

1. 分三层：UI / 调度 / 持久化，边界基本清晰（`0x17160`、`0x160f0`、`0x1a930` 均无 UI 依赖）。
2. 12 种调度用枚举 + 跳转表分发，新增类型只需加一个分支 + 一个格式串。
3. 持久化用 `QSaveFile` 保证原子性，损坏时降级为错误提示而非崩溃。
4. 空输入、非法时间格式、结束早于开始等均有前置校验（`0xcb10` 里 7 条校验分支）。
5. 静默启动 + 单实例 + 托盘常驻，符合「后台提醒器」的正确产品形态。

**做得差的**

1. **`TaskDialog` 单函数 11.2 KB**（`0x9d00`）——12 种表单的构造、显隐、取值、校验全塞在一个函数里，无子函数拆分。这是**最大重构收益点**。
2. **业务状态直接存 UI 控件**：校验函数 `0xcb10` 大量 `mov rcx, qword ptr [rsi + 0x38]`（直接读控件指针取文本），说明表单状态没有独立 ViewModel。
3. **`+0x44` 字段被 `pomodoroWorkMinutes` 与 `startupDelayMinutes` 复用**——类型联合靠"当前 scheduleType"隐式决定，容易出错。
4. **无数据迁移**：`version` 写了不读，未来改 schema 会直接判定「格式不正确」。
5. **无单元测试痕迹**：无 test 框架符号，无 assert 字符串。
6. 全局单一定时器 4 秒节拍驱动全部逻辑，`once`/`countdown` 精度受节拍限制（最大 4 秒延迟）。
7. 状态字段散落 30 个（含运行时非持久化的 `+0x6c`），`0x90` 结构手写偏移，无编译期检查。

### 5.4 个人信息与作者痕迹（重构需清除项）

| 位置 | 内容 | 偏移 |
|---|---|---|
| 关于对话框 HTML | `<b>作者</b><br>Thebzk@52pojie<br><a href="https://www.52pojie.cn/">www.52pojie.cn</a>` | `0xbbb408` |
| 关联文案 | `关于工作提醒助手` | `0xbbd128` |
| 组织名 | `LocalTools`（`QSettings` / `QStandardPaths` 组织名） | `0xbba980` |
| 应用名 | `WorkReminder` | `0xbba9b0` |
| 互斥体 | `LocalTools.WorkReminder.SingleInstance` | `0xbbc8e0` |
| 注册表值名 | `WorkReminder` | `0xbc2572` |

> 除上述外，PE 资源中 `VERSION` 资源**无 `CompanyName` / `LegalCopyright` / `Author` 字段**（`res_3_4` 解析出的版本块仅有 `VS_VERSION_INFO` 骨架），即作者未在文件属性里留名——留名只在"关于"对话框里。
> 图标资源 10 个尺寸（16→256 px），无签名。文件**未做数字签名**（无 `SECURITY` 数据目录）。

---

## 6. 证据索引（可复核）

| 文件 | 内容 |
|---|---|
| `docs/evidence/strings.tsv` | 277 条 `QStringLiteral` 全表（偏移 / 文本 / 引用点偏移 / 所属函数） |
| `docs/evidence/functions.tsv` | 自有函数 199 条（起始 RVA / 长度 / 关联字符串） |
| `docs/evidence/jumptables.tsv` | 3 张 12 路跳转表解码结果 |
| `docs/evidence/imports.tsv` | 479 条导入符号（DLL / 函数名） |

---

## 7. 分析限制（必读）

1. **未执行程序**：全部结论来自静态取证。运行时行为（例如 `usageBreak` 在真实睡眠/唤醒场景的表现）未经实测验证。
2. **字段偏移表为半推断**：`+0x70`/`+0x78`/`+0x80`/`+0x88` 直接命中证据；`+0x24`/`+0x28`/`+0x2c`/`+0x30`/`+0x44`/`+0x48`/`+0x4c`/`+0x50`/`+0x54`/`+0x58`/`+0x60`/`+0x68` 由 `0x160f0`、`0x17160`、`0x13d20`、`0x18b00` 的读写偏移交叉推断，语义准确但字段名可能与源码原名不完全一致。
3. **Qt 版本判定为 Qt 5 系列**：依据 `.qtmetad` 的 `QPA...5.3` 接口后缀 + 无 `Qt6` 专有符号。具体小版本（5.12 / 5.15）无法从静态证据唯一确定。
4. **未还原 C++ 源码**：本报告给出的是**行为规格**，重构采用净室重写（clean-room），不复制原二进制中的任何代码，仅依据公开可观测的行为与数据结构重新实现。
5. **`--render-previews` 的实际输出分辨率未验证**（需运行程序，本次未执行）。
