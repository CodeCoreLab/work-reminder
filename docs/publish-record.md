# 发布记录

## 2026-10-22 首次上线

| 项 | 值 |
|---|---|
| URL | https://example-site.com/work-reminder/ |
| 目标机 | example-test-host（**不是**跳板机 example-jump-host） |
| 目录 | `/opt/xmtooldev/www/work-reminder/`（index.html 55.7 KB + assets 252 KB） |
| 安装包 | `assets/work-reminder-v1.0.0.zip` · 257,904 字节 · zip 头 `PK` |
| 属主权限 | `root:root`，目录 755 / 文件 644 |
| 入口卡片 | 已插入 `/opt/xmtooldev/www/index.html`，`grep -c 'href="/work-reminder/"'` = 1 |
| 入口页备份 | `www/index.html.bak_cards_20261022_1939`（md5 `17b4e4f2…`，与原件一致） |
| healthcheck | exit 0，含 `tool:work-reminder -> 200` |
| 解耦红线 | vhost `grep -c aireport` = 0；页面 `grep -c aireport` = 0 |
| 回滚点 | 首次发布，无旧快照；回滚即 `deploy.sh work-reminder --rollback`（无旧版时不适用） |

### 验收（真浏览器，非服务端 curl）

| 项 | 结果 |
|---|---|
| 页面 HTTPS 200 + 标题正确 | ✓ |
| 12 个调度按钮齐全、每个时间线非空 | ✓ 12/12 |
| 番茄钟计时真实递减（24:58 → 24:57） | ✓ |
| 安装包真实 GET → 200 且字节数一致 | ✓ 257,904 |
| 入口页卡片存在且点击可进入工具页 | ✓ |
| 滚动全页后无元素停在 opacity:0 | ✓ 14/14 可见 |
| 390px 无横向溢出 | ✓ 0px |
| console / pageerror | ✓ 0 条 |

### 迭代修复记录（同一轮内）

| # | 问题 | 根因 | 修复 |
|---|---|---|---|
| 1 | 首屏后 13/14 个 `.reveal` 停在 opacity:0 | 默认 CSS 即隐藏，依赖 IO 触发 | 改渐进增强：默认可见，`html.js` 才允许隐藏；另加打印/降级兜底 |
| 2 | 星期条对 `interval`/`daily` 等全亮 7 天 | 这些类型没有星期掩码 | `mask()` 返回 `null` 时隐藏星期条并显示语义说明 |
| 3 | 下载区上下留白过大 | `section.band` 自带 padding 与内联 padding 叠加 | 改为独立 section，去掉 `band` 类 |
| 4 | 导航「下载」按钮紧贴容器边缘像被裁 | 右边缘恰好等于容器右边缘 | 容器与 nav 各加 2/8px 右内边距 |
| 5 | 手机端 12 项竖排占满整屏 | 单列布局 | ≤640px 改两列（`grid-template-columns:1fr 1fr`） |
| 6 | 两列规则不生效 | `.kind-list{display:flex}` 写在媒体查询**之后**，同优先级后者胜 | 媒体查询移到样式表末尾（媒体查询不增加特异性，顺序决定） |
| 7 | 番茄进度环一开始就满环 | 用 `remaining` 计算 offset，满环语义反直觉 | 改为按已消耗比例增长，空环起步 |

### 注意

- 覆盖 `site-pub/index.html` 会**丢掉** `prepare_publish_dir.py` 注入的「工具列表」返回链接。
  改页面后必须重新跑准备脚本，不能直接复制。
- vhost 配置实际路径是 `/opt/xmtooldev/conf/xmtooldev.conf`（不是 skill 文档里写的 `conf.d/`）。
