# 开发说明

给改代码的人看的。日常使用看 [README](README.md)。

## 构建

需要 Rust stable（Windows 另需 MSVC 工具链）。

```bash
cargo build --release        # 产物 target/release/tvhelper(.exe)
cargo test --release         # 51 条；2 条 ignored 会真起客户端
```

release 版是 `windows_subsystem = "windows"`，双击没有控制台。要排查就先跑无头命令，结论会打出来并落盘：

| 命令 | 作用 |
|---|---|
| `tvhelper _version` | 版本 + 自身路径 + 文件大小，并写 `~/.tvx/version.txt` |
| `tvhelper _doctor` | 环境自检与副本补丁清单，报告写 `~/.tvx/doctor.txt` |
| `tvhelper _reapply` | 按当前开关重打托管副本的补丁（不开界面） |
| `tvhelper _build` | 从系统安装构建 / 重建托管副本 |

## 代码结构

```
src/
  main.rs         入口：解析命令行 → 无头命令 → 否则开界面
  cli.rs          子命令定义与参数解析（纯函数，有测试）
  app.rs          界面主体：App 状态、后台结果消费、update() 骨架
  app/state.rs    Shared / 盘点线程 / 一次性任务
  app/authwatch.rs 发现新登录页 → 决定要不要自动打开（三道防死循环闸）
  app/actions.rs  Act 的执行（启动、停止、重建、开关重打…）
  app/headless.rs _reapply / _build / 打开目录
  app/theme.rs    字体栈、样式、状态色
  app/view.rs     一帧的只读快照 + 日志区；面板在 view/head.rs、view/accounts.rs
  install.rs      配置、托管副本、路径与原子写
  install/health.rs   环境体检 + 自检报告
  install/discover.rs 发现已安装的客户端
  install/build.rs    复制 → 解包 asar → 打补丁 → macOS 重签名
  patch.rs        补丁引擎：匹配、通配锚点、按段迁移、资源落盘
  patch/data.rs   版本白名单 + 注入脚本 + 补丁表（换版本只改这里）
  profiles.rs     多账号档案 CRUD
  profiles/account.rs 账号身份识别（JWT / 本地存储）
  profiles/auth.rs    深链回调、应用内登录页、回调接管
  profiles/runtime.rs PID 文件、存活判定、启动停止
  i18n.rs         文案结构 + 按语言取词的助手方法；zh.rs / en.rs 是文案表
  alerts/0*.js    页内报警引擎（build.rs 按序拼成一份再嵌入）
  alerts-main.js  主进程侧：IPC、配置持久化、系统通知、SMTP
```

两条不变量：

- **补丁数据与匹配算法分离**。换客户端版本只改 `patch/data.rs`，`patch.rs` 里的通配锚点与迁移逻辑不动。
- **界面不直接做系统调用**。面板只读快照、只产出 `Act`；副作用集中在 `app/actions.rs` 与后台线程。

## 改补丁时的注意事项

- 锚点是字面代码串，必须**恰好命中 1 次**；0 次会走通配锚点，仍失配就整条 `Failed` 并在界面点名。
- `alerts/0*.js` 七段共享同一个 IIFE 作用域，拼接顺序由 `build.rs` 的 `ALERTS_PARTS` 决定。
- 副本里残留 `app.asar` 时补丁全部不生效（Electron 优先加载 asar），判定可用必须是 `is_patched && !asar_residue()`。
- 验证落点别只看单测：副本里 `handleDomReady` 与 `.on("dom-ready",…)` 注入段各 1 处，主进程 `index.js` 过 `node --check`。

## 发版

`Cargo.toml` 的 `version` 是唯一版本源；`release.yml` 由 tag `v*` 触发，并校验 tag 与 `Cargo.toml` 一致。

```bash
# 改版本 → 不带 --locked 编一次（更新 Cargo.lock）→ 再跑 locked 测试
cargo test --release --locked
git commit -am "x.y.z: …" && git push origin main
git tag -a vX.Y.Z -m "…" && git push origin vX.Y.Z
```
