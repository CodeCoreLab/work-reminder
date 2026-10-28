# 变更日志 — 工作提醒助手（work-reminder）

本文件记录本项目的变更。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [未发布]

### 变更

- 移除代码与 Cargo 元数据中的公司邮箱，改用 GitHub noreply 地址
- README 中的测试数量表述与实际对齐（原「69 项」与「111 项」两处均不符，
  实际为 67 项）

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
