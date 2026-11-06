# 变更日志 — 工作提醒助手（work-reminder）

本文件记录本项目的变更。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [未发布]

## [2.0.1] - 2026-11-06
### 变更

- 移除代码与 Cargo 元数据中的公司邮箱，改用 GitHub noreply 地址
- README 中的测试数量表述与实际对齐（原「69 项」与「111 项」两处均不符，

### 修复

- **配置组织目录统一**：Tauri 前端与 legacy 迁移模块此前使用不同的配置目录；新增唯一路径入口 `paths` 模块，两处统一走它
- **`days_between` 符号语义回归测试补齐**（实现此前已改对）
- `trace()` 只输出裸消息，无时间戳无级别，且单一 `tray.log` 无限增长 →
- `store::load` 失败只写进 UI 字段、不落盘，重启后线索丢失 → 错误写日志
- 定时器循环内 `advance()` 无错误处理，而 release profile 为 `panic=abort`，
- 番茄钟预备提醒的去重键是任务 id 而非 (id, 阶段)：某阶段的标记若未经
- `scheduler` 月度回退的注释与代码不一致（注释写 +1 月，代码 +1 天）→
- `legacy.rs` 仍被 `main.rs` 引用却无废弃标记 → 补 `#[deprecated]` 与迁移说明
## [2.0.0] - 2026-10-23

### 新增

- 架构重构：Win32 手绘前端 → Tauri 2 + HTML/CSS
- 提醒核心抽为独立 crate `reminder-core`（零平台依赖，可跨 host 单元测试）
- CLI 参数：`--probe` / `--params` / `--selftest`

### 移除

- 1.x 的 Win32 版本归档于 `_archive_v1_win32_20261023/`，仅作回退参考

## [1.0.0] - 2026-10-22

### 新增

- 首个发布版本（Win32 版）：12 种提醒方式、托盘常驻、开机自启
