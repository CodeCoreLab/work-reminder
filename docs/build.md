# 构建与依赖说明

> 项目：work-reminder（工作提醒助手 · Rust 重构版）
> 版本：1.0.0 ｜ 许可：MIT（见 `LICENSE`）

---

## 1. 结论：依赖面

| 类别 | 数量 | 说明 |
|---|---|---|
| 第三方 crate | **2**（含传递依赖共 11 个） | `serde` + `serde_json`，仅用于 JSON 持久化 |
| 系统 DLL | **9**（实测） | 其中 6 个由本项目显式声明，3 个由 Rust std 引入（`msvcrt` `ntdll` `api-ms-win-core-synch-l1-2-0`），全部为 Windows 自带 |
| 外部运行时 | **0** | 无 Qt、无 .NET、无 VC++ 运行时、无 WebView |
| 静态资源 | **0** | 图标用系统 `IDI_APPLICATION`，提示音用系统声音别名 |

对比原程序：Qt 5 静态链接 + MSVC CRT 静态链接，产物 15.93 MB；本实现 427 KB，**体积降至 1/37**。

---

## 2. 环境要求

| 项 | 要求 | 本机实测值 |
|---|---|---|
| Rust | ≥ 1.70（本机 1.98.1） | ✔ |
| Host triple | `x86_64-pc-windows-gnu` | ✔ |
| 链接器 | GNU 工具链自带 `ld` | ✔ |
| MSVC / Visual Studio | **不需要** | — |
| Node.js / Python | **不需要** | — |

> **为什么是 `-gnu` 而不是 `-msvc`**：本机 `rustc` 默认 host 为 `x86_64-pc-windows-gnu`，且没有安装
> Visual Studio C++ 工具链。`-gnu` 目标自带 `ld.exe` 与预编译导入库，开箱可用。

---

## 3. 构建命令

### 3.1 标准构建

```bash
cd .
cargo build --release
```

产物：`target/release/work-reminder.exe`

### 3.2 本机必须附加的环境变量（重要）

本机 `dlltool.exe` 损坏（`CreateProcess` 失败），而 `serde_derive` 等过程宏 crate 需要它来
合成导入库。**必须**把 rustup 自带的 self-contained binutils 前置到 PATH：

```bash
export PATH="/c/Users/Administrator/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained:$PATH"
cargo build --release
```

> **这条限制只影响带过程宏的依赖编译，不影响运行产物。** 干净的 Windows + rustup 环境不需要它。
> 根因与规避过程见 `docs/refactor-plan.md §4.2`。

### 3.3 测试

```bash
cargo test --lib      # 67 项单元测试（reminder-core）
cargo test --release  # 同上，优化构建
```

### 3.4 诊断（非交付物）

```bash
cargo run --example diag
```

逐项打印 `RegisterClassExW` / `CreateWindowExW` / `SetTimer` / `Shell_NotifyIconW` /
`GetLastInputInfo` 的返回值与错误码，用于排查「GUI 起不来」这类环境问题。
在 Session 0（无交互桌面）或远程无桌面会话中，`Shell_NotifyIconW` 会失败，属环境限制。

---

## 4. 依赖清单

### 4.1 Cargo.toml

```toml
[dependencies]
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
```

传递依赖（`Cargo.lock` 锁定）：

```
serde, serde_core, serde_derive, serde_json
syn, quote, proc-macro2, unicode-ident   ← serde_derive 的构建期依赖
itoa, memchr, zmij                       ← serde_json 的运行时依赖
```

`zmij` 是 serde_json 使用的浮点格式化实现。构建期依赖不进最终二进制。

### 4.2 Win32 导入（手写 FFI）

`src/platform/ffi.rs` 用 `#[link(name = ...)]` + `extern "system"` 直接声明，共 **73 个函数**。
最终 PE 导入表实测 **155 个符号** —— 差额来自 Rust std 运行时（内存分配、栈展开、同步原语），
属正常且不可裁剪。下表只统计本项目显式声明的部分：

| DLL | 函数数 | 用途 |
|---|---|---|
| `kernel32` | 18 | 互斥体、线程、tick 计数、模块句柄、本地时间、错误码 |
| `user32` | 34 | 窗口类、消息循环、定时器、菜单、控件、空闲检测、GDI 调用转接 |
| `gdi32` | 6 | 卡片绘制（画刷、字体、DrawText、FillRect） |
| `shell32` | 2 | `Shell_NotifyIconW`（托盘）、`ShellExecuteW`（预留） |
| `advapi32` | 5 | 注册表 `Run` 键（自启动） |
| `winmm` | 1 | `PlaySoundW`（提示音） |

**为什么不用 `windows` / `windows-sys` crate**：这两个 crate 的 build script 会调 `dlltool`
合成导入库，本机该路径不可用（见 §3.2）。GNU target 的 `lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained`
目录已带 `libuser32.a` / `libkernel32.a` 等预编译导入库，`#[link]` 可直接用，因此手写声明是
本机唯一可行路径。副作用是依赖面更小、可审计性更好。

---

## 5. 产物信息

| 项 | 值 |
|---|---|
| 文件名 | `work-reminder.exe` |
| 大小 | 427,520 字节（418 KB） |
| 子系统 | GUI（`windows_subsystem = "windows"`，无控制台窗口） |
| 架构 | x86-64 |
| 优化 | `opt-level="z"` + LTO + `codegen-units=1` + `panic="abort"` + `strip` |
| 签名 | 无（个人工具，未做代码签名） |
| 导入符号 | 155 个（实测），来自 9 个系统 DLL |

### 5.1 运行时行为

```
work-reminder.exe            正常启动，首次运行弹出引导提示
work-reminder.exe --silent   静默启动（开机自启动使用）
work-reminder.exe --help     参数说明
```

配置文件：`%APPDATA%\MaXin\WorkReminder\reminders.json`
注册表：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run\WorkReminder`

### 5.2 验证记录（本机实测）

| 验证项 | 方式 | 结果 |
|---|---|---|
| 编译通过 | `cargo build --release` | ✔ 42s，0 warning |
| 单元测试 | `cargo test --lib` | ✔ 67 passed / 0 failed |
| 进程常驻 | 启动后 `tasklist` | ✔ 驻留 12.2 MB，4s tick 正常运行 |
| 单实例互斥 | 同目录第二次启动 | ✔ 第二实例退出码 0 且立即返回，首实例不受影响 |
| Win32 调用链 | `cargo run --example diag` | ✔ 建窗/定时器/托盘/空闲检测全部返回成功 |
| 版权头 | 逐文件检查 | ✔ 19/19 个 `.rs` 全部含 SPDX 标注 |
| 原作者痕迹 | 全仓 grep | ✔ 源码零残留（仅分析报告保留 1 处作为证据引用） |

---

## 6. 目录结构

```
E:\xmtool\work-reminder\
├── Cargo.toml                 工程定义与依赖
├── Cargo.lock                 依赖锁定
├── LICENSE                    MIT 许可全文
├── README.md                  项目说明
├── src\
│   ├── main.rs                进程入口、消息循环、tick 编排
│   ├── lib.rs                 库根（供测试与二进制共用）
│   ├── core\                  纯逻辑层，零 Win32 依赖
│   │   ├── time.rs            日历/时间运算（无需 chrono）
│   │   ├── model.rs           12 种调度枚举 + Task 模型
│   │   ├── scheduler.rs       next_trigger 求解器 + 触发状态机
│   │   ├── engine.rs          tick 循环、使用时长累计
│   │   └── store.rs           JSON 持久化（原子写）
│   └── platform\              Win32 适配层
│       ├── ffi.rs             73 个手写 Win32 声明
│       ├── tray.rs            托盘图标 + 右键菜单
│       ├── single_instance.rs 命名互斥体
│       ├── autostart.rs       注册表 Run 键
│       ├── idle.rs            GetLastInputInfo 封装
│       ├── audio.rs           PlaySoundW 提示音
│       ├── window.rs          提醒卡片（GDI 自绘）
│       ├── manager.rs         任务列表窗口
│       └── newtask.rs         新建任务对话框
├── examples\diag.rs           启动链路诊断（非交付物）
├── docs\
│   ├── reverse-analysis.md    逆向分析报告
│   ├── refactor-plan.md       架构评估与重构方案
│   └── evidence\              原始证据表（字符串/函数/跳表/导入）
├── releases\                  发布制品（按版本分目录）
└── tools\                     构建辅助脚本（预留）
```

---

## 7. 故障排查

| 现象 | 原因 | 处理 |
|---|---|---|
| `error: error calling dlltool 'dlltool.exe': program not found` | 本机 `dlltool` 不在 PATH | 按 §3.2 前置 self-contained 目录 |
| `dlltool: CreateProcess` 失败 | 本机 dlltool 二进制损坏 | 同上（self-contained 版本可用） |
| 编译成功但双击无反应 | 已在运行（单实例生效） | 看托盘；或用 `--help` 确认能弹出提示框 |
| 托盘无图标 | 无交互桌面 / 资源管理器未运行 | 程序会弹「当前系统没有可用的系统托盘」并退出 |
| 中文显示为方块 | 系统缺 `Microsoft YaHei UI` | 卡片绘制会回退到系统默认字体，不影响功能 |
