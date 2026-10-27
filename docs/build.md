# 构建与依赖说明

> 项目：work-reminder（工作提醒助手 · Tauri 2 版）
> 版本：2.0.0 ｜ 许可：MIT（见 `LICENSE`）

---

## 1. 结论：依赖面

| 类别 | 数量 | 说明 |
|---|---|---|
| 直接 crate | **5** | `tauri` / `tauri-plugin-autostart` / `tauri-plugin-single-instance` / `serde` / `serde_json` |
| 传递依赖 | **461**（`Cargo.lock` 实测） | Tauri 生态 + WebView2 绑定 + Tokio 运行时 |
| 系统 DLL | **1**（需随包分发） | `WebView2Loader.dll`，仅 mingw 目标需要 |
| 外部运行时 | **0** | 无 Qt、无 .NET、无 VC++ 运行时；WebView2 为 Windows 10/11 系统自带 |
| 静态资源 | **1** | `tauri/icons/` 下的应用图标，编译期嵌入 |

> `Cargo.lock` 提交入库，构建可复现。直接依赖只有 5 个，传递依赖多来自 Tauri 自身的
> 依赖树（Tokio / wry / tao / webview2-com），属框架固有成本。

---

## 2. 环境要求

| 项 | 要求 | 说明 |
|---|---|---|
| Rust | ≥ 1.77 | `tauri` 2.x 的 MSRV |
| Windows 目标 | `x86_64-pc-windows-msvc` 或 `x86_64-pc-windows-gnu` | 32 位不支持 |
| MSVC 工具链 | 用 MSVC 目标则需要 | 用 GNU 目标则不需要 Visual Studio |
| Linux 构建主机 | 交叉编译时需要 | 需装 `mingw-w64` 与 `rustup` |
| Node.js / Python | **不需要** | `Python` 仅用于 `tools/` 下的构建与验证脚本 |

> **为什么本项目用远程 Linux 交叉编译**：构建机的 Rust 工具链自带的 mingw 缺少
> `as.exe` / `cc1.exe`，`dlltool` 无法合成导入库，任何需要导入库的 crate 都会链接失败。
> Linux 主机装齐 mingw-w64 后即可交叉编译到 Windows。
> **如果你本机的 mingw 完整，直接本地构建即可**，无需远程主机。

---

## 3. 构建命令

### 3.1 本地构建（推荐，前提是本机 mingw 完整）

```bash
cd tauri
cargo build --release
```

产物：`tauri/target/release/work-reminder.exe`

### 3.2 交叉编译（远程 Linux 构建主机）

```bash
# 需要设置环境变量：WM_BUILD_HOST / WM_BUILD_USER / WM_BUILD_KEY / WM_BUILD_REMOTE
python tools/build_tauri.py

# 只做类型检查（快，用于快速反馈）
python tools/build_tauri.py --check
```

脚本流程：上传 `tauri/` 源码 → 清理远端陈旧 `Cargo.lock` → `cargo build --release
--target x86_64-pc-windows-gnu` → 拉回 `work-reminder.exe` 与 `WebView2Loader.dll`
→ 校验 SHA-256 一致 → 落盘到 `dist/tauri/`。

关键实现细节：

- **传输先写 `.part` 再原子改名**：连接中断不会留下看起来像有效产物的截断二进制。
- **传输后比对远端与本地 SHA-256**：不一致直接失败，不产出「疑似成功」的产物。
- **远端会先删 `Cargo.lock`**：上一次上传留下的锁文件可能钉住旧依赖图。

### 3.3 测试

```bash
cd tauri/reminder-core
cargo test --lib      # 67 项单元测试
cargo test --release  # 同上，优化构建
```

> 测试必须**在 `tauri/reminder-core/` 目录下执行**。在 `tauri/` 根目录执行
> `cargo test --lib` 会报 `no library targets found in package 'work-reminder'`——
> 主包是二进制 crate，唯一的 library target 属于 workspace 成员 `reminder-core`。

### 3.4 验证脚本

```bash
# 纯逻辑验证：直接调用二进制的 --params 探针，不依赖 UI 像素坐标
python tools/verify_logic.py

# 全链路功能验收：渲染 → 表单 → IPC → 落盘
python tools/final_verify.py

# WebView2 窗口截图（PrintWindow + PW_RENDERFULLCONTENT）
python tools/shot_tauri.py
```

---

## 4. 依赖清单

### 4.1 直接依赖（`tauri/Cargo.toml`）

```toml
[dependencies]
reminder_core = { path = "reminder-core", package = "work-reminder-core" }
tauri = { version = "2", features = ["tray-icon", "image-png"] }
tauri-plugin-autostart = "2"
tauri-plugin-single-instance = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

关键版本（`Cargo.lock` 锁定）：

| crate | 版本 |
|---|---|
| `tauri` | 2.12.5 |
| `tauri-build` | 2.7.4 |
| `tauri-plugin-autostart` | 2.7.0 |
| `tauri-plugin-single-instance` | 2.5.4 |
| `tauri-runtime-wry` | 2.12.3 |
| `wry` | 0.57.0 |
| `webview2-com` | 0.39.1 |
| `tao` | 0.37.1 |

### 4.2 两个插件的作用

| 插件 | 解决的问题 |
|---|---|
| `tauri-plugin-single-instance` | 第二次启动时聚焦已有窗口，而不是起一个并发的调度器写同一个 JSON 文件 |
| `tauri-plugin-autostart` | 开机自启。Windows 上写 `HKCU\...\CurrentVersion\Run` 注册表项 |

> **单实例插件必须最先注册**：检查要在任何窗口、托盘、定时器创建之前完成，
> 否则第二次启动会先创建定时器再发现已有实例。

### 4.3 权限清单（`tauri/capabilities/default.json`）

UI 需要的权限被显式收敛到最小集：

```
core:default
core:window:allow-close / allow-hide / allow-show / allow-set-focus
core:event:default
autostart:default / allow-enable / allow-disable / allow-is-enabled
```

`tauri.conf.json` 另配了 CSP：`default-src 'self'`，
`script-src 'self'`（无 `unsafe-inline`），`img-src 'self' data:`。

---

## 5. 产物信息

| 项 | 值 |
|---|---|
| 文件名 | `work-reminder.exe` |
| 大小 | 3,230,208 字节（3.08 MiB） |
| 配套 DLL | `WebView2Loader.dll` 160,880 字节（157 KiB） |
| 架构 | x86-64 |
| 优化 | `opt-level="z"` + LTO + `codegen-units=1` + `panic="abort"` + `strip` |
| 签名 | 无（个人工具，未做代码签名） |
| 落盘位置 | `dist/tauri/`（`dist/` 不入库，见根 `.gitignore`） |

### 5.1 运行时行为

```
work-reminder.exe                 正常启动（托盘常驻 + 主窗口）
work-reminder.exe --probe         启动后立即退出，用于探测进程能否正常加载
work-reminder.exe --params once   打印 once 类型的参数表与默认值
```

`--selftest=<name>` 仅在 debug 构建生效（release 构建会忽略它），
用于驱动一次脚本化 UI 交互做自动化验收。

配置文件：`%APPDATA%\MaXin\WorkReminder\reminders.json`
自启动：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下的注册表项

### 5.2 验证记录（本机实测）

| 验证项 | 方式 | 结果 |
|---|---|---|
| 单元测试 | `cd tauri/reminder-core && cargo test --lib` | ✔ 67 passed / 0 failed |
| 交叉编译 | `python tools/build_tauri.py` | ✔ 产物生成，SHA-256 校验一致 |
| 纯逻辑验证 | `python tools/verify_logic.py` | ✔ 12 种调度参数表全部符合预期 |
| 全链路验收 | `python tools/final_verify.py` | ✔ 渲染 → 表单 → IPC → 落盘全通 |
| clippy | `cargo clippy --release` | ✔ 0 告警 |

---

## 6. 目录结构

```
work-reminder/
├── LICENSE                    MIT 许可全文
├── README.md                  项目说明
├── assets/                    站点与图标素材
├── dist/tauri/                构建产物（不入库）
├── docs/                      文档与证据
├── tools/                     构建与验证脚本
├── _archive_v1_win32_20261023/  v1.0.0 Win32 版归档（回退参考，不参与构建）
└── tauri/
    ├── Cargo.toml             workspace 定义：主包 + reminder-core
    ├── Cargo.lock             依赖锁定（461 项）
    ├── build.rs               Tauri 构建 + mingw 目标分发 WebView2Loader.dll
    ├── tauri.conf.json        窗口 / 安全策略 / 标识
    ├── capabilities/default.json  权限清单
    ├── icons/                 应用图标
    ├── src/main.rs            Tauri 后端：定时器、托盘、自启、IPC（798 行）
    ├── reminder-core/         纯逻辑 crate，零平台依赖
    │   ├── Cargo.toml
    │   ├── lib.rs             模块入口
    │   ├── time.rs            日历与时间运算（自实现，无 chrono）
    │   ├── model.rs           12 种 Schedule 枚举 + Task
    │   ├── scheduler.rs       next_trigger 求解 + 触发状态机
    │   ├── engine.rs          tick 循环 + 使用时长累计
    │   ├── store.rs           JSON 持久化（tmp + fsync + rename 原子写）
    │   └── legacy.rs          v1 配置文件的导入兼容
    └── ui/                    仅负责渲染
        ├── index.html         104 行
        ├── app.css            608 行
        └── app.js             527 行
```

---

## 7. 故障排查

| 现象 | 原因 | 处理 |
|---|---|---|
| 双击 exe 无反应、无任何窗口 | 缺 `WebView2Loader.dll` | 把 DLL 放到 exe 同目录 |
| `0xC0000135` 退出且 Windows 不报错 | 同上（STATUS_DLL_NOT_FOUND 静默退出） | 同上 |
| `cargo test --lib` 报 `no library targets found` | 在 `tauri/` 根目录执行了 | 改到 `tauri/reminder-core/` 目录 |
| `error: error calling dlltool 'dlltool.exe'` | 本机 mingw 缺 `as.exe`/`cc1.exe` | 用 §3.2 交叉编译，或安装完整 mingw-w64 |
| 交叉编译报 crates.io 拉取失败 | 构建主机访问 crates.io 受限 | 在构建主机配 `/root/.cargo/config.toml` 指向可用镜像 |
| 双击后程序在跑但找不到窗口 | 单实例插件生效，第二次启动只聚焦已有实例 | 看系统托盘 |
| 托盘无图标 | 无交互桌面 / 资源管理器未运行 | 在有交互桌面的会话中运行 |
