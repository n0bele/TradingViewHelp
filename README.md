# TVHelper · TradingView 助手

<img src="assets/icon.png" width="88" alt="TVHelper icon" />

## 功能

本地运行的 TradingView 桌面版多账号管理器：每个账号一份独立配置目录，多个账号可同时在线、互不串号；补丁只作用于本地副本，不改动已安装的 TradingView。

界面上的功能开关（拨动即自动重打补丁）：

- 拦截广告/追踪网络
- 多图表布局解锁
- 指标叠加解锁
- 屏蔽充值窗口
- 断线自动恢复
- 禁用自动更新
- 本地指标报警

## 本地指标报警

免费版只有价格报警、没有「指标报警」。这条功能在本地补上**指标条件监控**，不涉及服务端配额：

- 入口：右侧绘图工具栏里、原生「警报」按钮正下方有一个蓝色铃铛；右下角另有「报警 N」悬浮按钮；快捷键 `Alt+A` 也可以打开。
- 报警绑定**当前图表**：品种、周期、该图上真实存在的指标（含参数，如 `RSI(14, close)`）。
- 设置页按 TradingView 原生报警的四段组织：
  - **条件**：先选指标，再选**这个指标自己声明的条件名**，与原生「创建警报」对话框的列表一致：
    - 脚本指标（Pine）的条件来自 `metaInfo.plots` 里 `type=alertcondition` 的条目，名字取 `metaInfo.styles[plotId].title`，例如 `Machine Learning Adaptive SuperTrend` 的 `Bullish Trend Shift` / `Bearish Trend Shift` / `High Volatility` / `Medium Volatility` / `Low Volatility`。
    - 免费账号的客户端 `metaInfo` 往往**不带** `alertcondition`（技术报警属于服务端付费能力）。这时按指标名在公开脚本库里找到同名脚本、读它的源码，把 `alertcondition(...)` 的条件名原样取出来（结果缓存在本地，不用每次开面板都请求）。条件名与原生对话框一致，等于绕开了元数据缺失。
    - 每个条件会自动落到客户端能读到的同源信号上：同名（或同名去 Shift/Signal 后缀）的图形列 → 「信号出现」；状态列 → 「状态切换」；`na` 交替的上下轨 → 「出现」；名字对不上时按方向词（Bullish/Bearish ↔ up/down）配对上下轨。设置页会写明每个条件对应的是哪条输出。
    - 确实还原不出的条件（如这段脚本的波动率聚类切换，脚本内部算了但没画成可读列）会灰显并标注「客户端不可读」，不会假装能报警。
    - 内置指标（RSI、Volume、MACD…）没有 `alertcondition` 元数据，退回按输出列给条件：数值列 → 上穿 / 下穿 / 大于 / 小于 / 区间 / 出现 / 消失 + 阈值；状态列（colorer）→ 只选「变成哪个值」。
  - **周期和触发**：不单独设置，只标注「与图表一致」「每根 K 线最多一次」。
  - **消息**：标题与内容都可改，支持占位符 `{{ticker}}` `{{symbol}}` `{{exchange}}` `{{interval}}` `{{open}}` `{{high}}` `{{low}}` `{{close}}` `{{volume}}` `{{time}}` `{{study}}` `{{plot}}` `{{condition}}` `{{threshold}}` `{{value}}`（点标签即可插入）。
  - **通知**：系统通知与邮件可分别勾选；勾邮件后可直接填这一条报警的发件人/收件人（留空则用「通知设置」里的全局默认值）。
- 邮件走自填 SMTP（465 SSL 或 587 STARTTLS，账号 + 授权码），通知设置里可单独测试系统通知与邮件。
- 设置存在本机账号配置目录（`tvx-alerts.json`），重启后自动恢复；**图表没开、品种或周期不符、指标被删时只显示「未就绪」，不会硬触发**。
- 边界：软件关闭就不监控；行情延迟随账号数据权限（免费版部分市场延迟 15 分钟）；服务端真正的报警（邮件/短信/webhook、关机也运行）仍然只有订阅才有。

## 构建

需要 Rust stable。

```bash
cargo build --release
# 产物：target/release/tvhelper(.exe)，Windows 另需 MSVC 工具链
```

## 使用

1. 先安装 TradingView 桌面版。
2. 运行程序，「添加账号」→ 填名字 → 「启动」。
3. 在弹出的 TradingView 窗口里登录；切换账号直接启动另一个即可。

## 免责声明

- 非官方工具，与 TradingView, Inc. 无关联；TradingView 名称与商标归其权利人所有。
- 仅在本机运行，不修改原始安装、不上传账号数据；不绕过服务端配额或订阅机制。
- 使用前请遵守 TradingView 服务条款，风险自负。
