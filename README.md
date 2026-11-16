# TVHelper · TradingView 助手

<img src="assets/icon.png" width="88" alt="TVHelper icon" />

## 功能

本地运行的 TradingView 桌面版多账号管理器：每个账号一份独立配置目录，多个账号可同时在线、互不串号；补丁只作用于本地副本，不改动已安装的 TradingView。

界面上的功能开关（拨动即自动重打补丁）：

- 拦截广告/追踪网络
- 多图表布局解锁
- 指标叠加解锁
- 屏蔽充值窗口
- 禁用自动更新

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
