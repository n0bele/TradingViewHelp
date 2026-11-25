//! 运行态：PID 文件、存活判定、启动/停止、连接数上限。

use super::*;

/// 同时运行的实例上限：每个实例是一整棵 Electron 进程树（渲染 + GPU + 网络），
/// 8 个覆盖常见多账号场景；再多先撞内存，而不是撞服务端。
pub const MAX_RUNNING: usize = 8;

/// 当前在跑的实例数（按 PID 文件 + 进程映像校验，不依赖界面缓存）
pub fn running_count() -> usize {
    list()
        .into_iter()
        .filter(|p| running_pid(p).is_some())
        .count()
}

/// 启动结果：PID 之外还要带回"不致命但该让用户知道"的提示。
pub struct Launched {
    pub pid: u32,
    /// 同一账号在别的档案里也在跑：**不阻止**（客户端本身允许多开，
    /// 每个档案一个 --user-data-dir 就是独立实例），只提示可能撞服务端并发限制。
    pub warning: Option<String>,
}

pub fn launch(managed: &Managed, profile: &Profile, lang: Lang) -> Result<Launched> {
    if let Some(pid) = running_pid(profile) {
        return Ok(Launched { pid, warning: None });
    }

    // 启动前先体检。客户端的日志目录固定建在 `%APPDATA%\TradingView\logs`，
    // 而 `%APPDATA%` / `%USERPROFILE%` 指向不存在的目录时，它会以
    // `ENOENT: mkdir '…\logging'` 崩在启动阶段（栈里只有 mkdirSync/initializeWrite）。
    // 那种崩溃框看不出原因，所以在这里拦住并说人话。
    let problems = crate::install::environment_problems();
    if !problems.is_empty() {
        let detail = problems
            .iter()
            .map(|p| p.detail.as_str())
            .collect::<Vec<_>>()
            .join("；");
        crate::install::log_error("启动前环境检查", &detail);
        bail!("{detail}");
    }

    // 同账号多开：老版本在这里直接拒绝，理由是"服务端按账号限制并发连接"。
    // 但这属于拿不准的推断——客户端本身不拦（每个 --user-data-dir 一个实例），
    // 真正会撞限制的是同一账号的多个窗口抢行情连接。所以改成"允许 + 提示"：
    // 用户要一间屋子开两个窗口看不同品种是合理需求，出问题自己关一个即可。
    let uid = account_id(profile).or_else(|| {
        let d = detect_uid_from_storage(profile)?;
        let _ = set_account_id(profile, &d);
        Some(d)
    });
    let warning = uid.as_ref().and_then(|uid| {
        same_account_others(profile, uid)
            .into_iter()
            .find(|(_, running)| *running)
            .map(|(name, _)| lang.warn_same_account(&profile.name, &name, uid))
    });
    // macOS：每个账号一份独立的 .app 副本（独立 bundle id → 独立 userData 与 scheme 归属）
    #[cfg(target_os = "macos")]
    let exe = {
        let app = crate::macos_app::ensure_profile_app(managed, profile)?;
        crate::macos_app::ensure_userdata_link(profile)?;
        app.join("Contents").join("MacOS").join("TradingView")
    };
    #[cfg(not(target_os = "macos"))]
    let exe = managed
        .exe()
        .ok_or_else(|| anyhow::anyhow!("托管副本里找不到可执行文件，请先重新生成"))?;

    // 启动前把环境准备好：账号的 user-data 目录必须存在，
    // 否则 Electron 会退回默认配置目录，多开就串了。
    fs::create_dir_all(&profile.dir)
        .with_context(|| format!("创建账号目录 {}", profile.dir.display()))?;

    let mut cmd = Command::new(&exe);
    // macOS 走"副本独立 userData + 符号链接指回账号目录"，不能传 --user-data-dir，
    // 否则系统拉起的 URL 回调进程与运行实例不是同一个 userData、拿不到单实例锁。
    #[cfg(not(target_os = "macos"))]
    cmd.arg(format!("--user-data-dir={}", profile.dir.display()));
    // 不要把宿主进程的 Electron/Node 变量带进去
    for k in [
        "ELECTRON_RUN_AS_NODE",
        "ELECTRON_NO_ATTACH_CONSOLE",
        "ELECTRON_ENABLE_LOGGING",
        "NODE_OPTIONS",
    ] {
        cmd.env_remove(k);
    }

    // 实例输出落到账号目录，登录/深链出问题时能直接看日志
    let log_path = profile.dir.join("tvx-launch.log");
    if let Ok(f) = fs::File::create(&log_path) {
        cmd.stdout(std::process::Stdio::from(f.try_clone()?));
        cmd.stderr(std::process::Stdio::from(f));
    }

    let child: Child = cmd
        .spawn()
        .with_context(|| format!("启动 {} 失败", exe.display()))?;
    let pid = child.id();
    // 记录创建时间，后面用它区分"PID 被复用"
    #[cfg(windows)]
    let stamp = format!("{pid}:{}", win_proc::creation_time(pid).unwrap_or(0));
    #[cfg(not(windows))]
    let stamp = pid.to_string();
    // PID 文件是"停止/查重/托盘状态"的唯一依据，写不上等于实例失控：
    // 进程已经起来了，不能假装启动成功，至少告诉用户 PID 和手动处理方式。
    if let Err(e) = crate::install::write_atomic(&pid_file(&profile.dir), stamp.as_bytes()) {
        crate::install::log_error("写 PID 文件", &e);
        return Err(anyhow::anyhow!(
            "实例已启动（PID {pid}），但 PID 记录写入失败：{e}。请在任务管理器里结束该进程后重试"
        ));
    }

    // 记录登录目标：应用启动后自己会改写协议命令，扫描线程会按它纠回来
    write_login_target(profile);

    // 让浏览器的登录回调落回这个账号
    #[cfg(target_os = "macos")]
    {
        // macOS：把 tradingview:// 指向本账号的 .app 副本（失败不阻断启动）
        let _ = crate::macos_app::set_url_handler(profile);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = crate::protocol::set_target(&exe, Some(&profile.dir));

    Ok(Launched { pid, warning })
}

pub fn stop(pid: u32) -> Result<()> {
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .context("taskkill 失败")?;
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .context("kill 失败")?;
    }
    // 没有实例在跑时还原协议命令，不留某个账号的目录
    if profiles_with_running_instances() == 0 {
        clear_login_target();
        if let Some(m) = crate::install::load_managed() {
            if let Some(exe) = m.exe() {
                let _ = crate::protocol::set_target(&exe, None);
            }
        }
    }
    Ok(())
}

