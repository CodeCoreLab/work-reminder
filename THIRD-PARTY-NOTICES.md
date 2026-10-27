# 第三方组件与许可声明 — work-reminder（工作提醒助手）

本文件列出本项目使用的第三方组件及其许可协议。

**本仓库不包含任何第三方二进制文件**——所有依赖一律通过语言原生的包管理机制
（`PackageReference` / `go.mod` / `Cargo.toml`）在构建时还原。
克隆本仓库后执行对应的构建命令，即可获得完整的依赖树。

## work-reminder（工作提醒助手）

技术栈：Rust + Tauri 2

| 组件 | 版本 | 许可 | 用途 |
|---|---|---|---|
| [serde](https://github.com/serde-rs/serde) | 1.x | MIT / Apache-2.0 | 序列化框架 |
| [serde_json](https://github.com/serde-rs/json) | 1.x | MIT / Apache-2.0 | JSON 编解码 |
| [tauri](https://github.com/tauri-apps/tauri) | 2.x | MIT / Apache-2.0 | 桌面应用框架（托盘、窗口） |
| [tauri-build](https://github.com/tauri-apps/tauri) | 2.x | MIT / Apache-2.0 | 构建期脚本（前端资源嵌入） |
| [tauri-plugin-autostart](https://github.com/tauri-apps/plugins-workspace) | 2.x | MIT / Apache-2.0 | 开机自启动 |
| [tauri-plugin-single-instance](https://github.com/tauri-apps/plugins-workspace) | 2.x | MIT / Apache-2.0 | 单实例运行 |

**Tauri 生态的传递依赖**包含 `wry`（MIT/Apache-2.0）、`tao`（MIT/Apache-2.0）、
`webkit2gtk` 绑定（LGPL-2.1 / MPL-2.0）、`serde` 系列等数十项，
均为宽松许可。完整清单见 `tauri/Cargo.lock`。

### 运行时组件

| 组件 | 许可 | 说明 |
|---|---|---|
| [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) | 运行时 EULA | Windows 上的 WebView 承载；Edge/Chrome 已内置，无需单独安装 |
| WebKitGTK | LGPL-2.1 / MPL-2.0 | Linux 平台的 WebView 承载（动态链接） |

> Windows 产物需随包分发 `WebView2Loader.dll`（MIT，Microsoft 提供），
> 该文件由构建脚本自动拉取，不入库。

---
