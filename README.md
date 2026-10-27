# work-reminder · 工作提醒助手（Rust 重构版）

一个常驻系统托盘的轻量提醒工具，支持 **12 种提醒方式**，单文件 418 KB，零外部运行时依赖。

> 本仓库是对同名 Windows 工具的**净室重写**：只依据行为规格重新实现，未复制原程序的任何代码。
> 逆向分析与技术选型论证见 `docs/`。

---


## 版本

当前版本 **2.0.0（Tauri 2）**。1.x 的 Win32 手绘前端已移除，源码与产物归档在
`_archive_v1_win32_20261023/`（不含历史记录，仅作回退参考）。

| | 值 |
|---|---|
| UI 实现 | HTML/CSS（1149 行） |
| 产物 | `work-reminder.exe` 3.27 MB + `WebView2Loader.dll` 161 KB |
| 空闲内存 | ~40 MB |
| 构建 | 跳板机交叉编译（本机 mingw 缺 as.exe/cc1.exe） |

`tauri/reminder-core/` 是纯逻辑 crate：调度、持久化、时间运算，
零平台依赖，可独立单元测试。

### 构建与验证

```bash
# 交叉编译到 Windows x64 并自动拉回产物（含 WebView2Loader.dll）
python tools/build_tauri.py

# 只做类型检查（快）
python tools/build_tauri.py --check

# 打印某一提醒方式的参数表与默认值
dist/tauri/work-reminder.exe --params once

# 端到端：真实点击 UI 创建任务并校验落盘
python tools/ui_e2e.py
python tools/ui_e2e_once.py
```

### 已知约束

- **必须随包分发 `WebView2Loader.dll`**：mingw 交叉编译走动态导入，MSVC 才静态链接。
  缺它时进程在加载阶段即退出（`0xC0000135`），**Windows 不弹任何提示**。
- 32 位目标不支持（已实测工具链与 Rust target 均未安装，按用户决定放弃）。
- 跨机构建的网络取源规则见 `~/.workbuddy/ondemand/cross-build.md`。


## 功能

| 调度类型 | 说明 |
|---|---|
| 按间隔循环 | 每 N 分钟 / 小时 |
| 每天固定时间 | 每天 HH:mm |
| 指定日期时间（一次） | 单次绝对时刻 |
| 每周指定日期 | 周一至周日多选 + 时刻 |
| 仅工作日 | 周一至周五 + 时刻 |
| 每月指定日期 | 指定日（自动夹取月末）/ 最后一天 / 最后一个工作日 |
| 每天多个时间点 | `09:00, 12:00, 17:30` |
| 限定时段循环 | 指定星期 + 时段内按间隔重复 |
| 倒计时 | N 分钟后提醒一次 |
| 番茄钟 | 专注/休息交替，跑满 N 轮后自动结束 |
| 开机后提醒 | 启动 N 分钟后提醒 |
| 连续使用电脑提醒 | 连续用机 N 分钟后提醒（空闲 5 分钟自动归零） |

通用能力：任务暂停/启用、今日不再提醒、延后提醒、开机自启动、托盘常驻、提示音、卡片弹窗。

---

## 快速开始

```bash
# 构建（本机需前置 self-contained binutils，见 docs/build.md §3.2）
cargo build --release

# 运行
./target/release/work-reminder.exe
./target/release/work-reminder.exe --silent   # 静默启动（自启动用）
./target/release/work-reminder.exe --help
```

产物：`target/release/work-reminder.exe`（418 KB）

---

## 结构

```
tauri/
  src/main.rs        Tauri 后端：定时器、托盘、自启、IPC 命令（725 行）
  reminder-core/     纯逻辑 crate，零平台依赖
    time.rs          日历与时间运算（自实现，无 chrono）
    model.rs         12 种 Schedule 枚举 + Task
    scheduler.rs     next_trigger 求解 + 触发状态机
    engine.rs        tick 循环 + 使用时长累计
    store.rs         JSON 持久化（tmp + fsync + rename 原子写）
  ui/                仅负责渲染：无定时器、无存储、无调度
    index.html
    app.css          TickTick 风格设计系统（5 级字阶 / 4px 网格 / 单一主色）
    app.js           通过 IPC 取快照并渲染
  build.rs           Tauri 构建 + 为 mingw 目标分发 WebView2Loader.dll

tools/
  build_tauri.py     交叉编译并在本机校验产物
  fetch_to_host.py   本机下载资源后上传跳板机
  shot_tauri.py      WebView2 窗口截图（PrintWindow + PW_RENDERFULLCONTENT）
  ui_e2e.py          端到端：真实点击 UI 创建任务并校验落盘
  ui_probe.py        逐像素驱动界面
```


## 数据位置

| 项 | 位置 |
|---|---|
| 配置 | `%APPDATA%\MaXin\WorkReminder\reminders.json` |
| 自启动 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\WorkReminder` |

配置格式：

```json
{
  "version": 1,
  "tasks": [
    {
      "id": "t1761118080000-0",
      "title": "喝水",
      "message": "去接杯水吧，顺便起来活动一下～",
      "scheduleType": "interval",
      "intervalMinutes": 60,
      "nextTrigger": { "date": {"year":2026,"month":10,"day":22}, "time": {"hour":16,"minute":0,"second":0} },
      "mutedUntil": null,
      "enabled": true,
      "pomodoro": { "currentCycle": 0, "inBreak": false },
      "startupBootMarker": null
    }
  ]
}
```

---

## 依赖

- 第三方 crate：**2**（`serde`、`serde_json`，仅用于 JSON）
- 系统 DLL：7 个，全部 Windows 自带
- 外部运行时：**无**（无 Qt / .NET / VC++ / WebView）

详见 `docs/build.md`。

---

## 测试

```bash
cargo test --lib   # 67 项，覆盖闰年、月末、周末、跨年、番茄钟全周期、空闲重置等边界
```

---

## 文档

| 文件 | 内容 |
|---|---|
| `docs/reverse-analysis.md` | 逆向分析：功能模块、关键业务逻辑、外部依赖、整体架构（含证据偏移） |
| `docs/refactor-plan.md` | 架构评估、Rust/Go/C++ 选型论证、迁移映射表 |
| `docs/build.md` | 构建、依赖、产物、故障排查 |
| `docs/evidence/` | 原始证据表（277 条字符串 / 199 个函数 / 3 张跳转表 / 479 条导入） |

---

## 许可

MIT © 2026 Ma Xin。全文见 `LICENSE`。

## 功能对照

与原程序逐项对照的结果见 `docs/feature-parity.md`：31 项功能一致，5 项行为差异（多为有意简化），
2 项未实现（均为作者自用的开发工具或纯装饰，不影响使用）。

## 质量与性能

实测数据（本机）：504 KB 单文件 · 私有内存 2.3 MB · 空闲 CPU 0.00% · 67 项单元测试。
评估结论与优化记录见 `docs/quality-review.md`。
