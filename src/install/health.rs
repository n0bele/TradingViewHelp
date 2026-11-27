//! 环境体检与自检报告：启动前把"会崩在启动阶段"的环境问题查出来。

use super::*;

/// 一项环境检查的结果。
///
/// 存在的意义：客户端在 `%APPDATA%` 缺失时会以
/// `ENOENT: no such file or directory, mkdir '…\AppData\Roaming\TradingView\logs'`
/// 崩在启动阶段（栈里只有 mkdirSync/initializeWrite，看不出是环境问题）。
/// 与其让用户对着一句英文崩溃框猜，不如启动前就查出来，用人话拦住。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvProblem {
    /// 稳定标识（日志/测试用，不翻译）
    pub key: &'static str,
    /// 面向用户的一句话（已带具体路径）
    pub detail: String,
}

impl EnvProblem {
    fn new(key: &'static str, detail: String) -> Self {
        Self { key, detail }
    }
}

/// 检查一个"必须存在且是目录"的环境变量；返回它的值（无论对错）供后续比对。
fn check_env_dir(
    out: &mut Vec<EnvProblem>,
    key: &'static str,
    var: &str,
) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(v) if v.is_empty() => {
            out.push(EnvProblem::new(key, format!("环境变量 {var} 为空")));
            None
        }
        Some(v) => {
            let p = PathBuf::from(&v);
            if !p.is_dir() {
                out.push(EnvProblem::new(
                    key,
                    format!("{var} 指向的目录不存在：{}", p.display()),
                ));
            }
            Some(p)
        }
        None => {
            out.push(EnvProblem::new(key, format!("缺少环境变量 {var}")));
            None
        }
    }
}

/// 客户端自己写日志的目录：`<appData>/TradingView/logs`。
///
/// 客户端建它的写法是 `exists(e) || createDir(e, {})`，而 `createDir` 就是
/// `mkdirSync(p, {})` —— **非递归**。所以只要 `…\AppData\Roaming\TradingView`
/// 这一层不存在，它在主进程启动阶段就会抛
/// `ENOENT: no such file or directory, mkdir '…\TradingView\logs'`，
/// 窗口还没出来就进 Critical Error（栈里只剩 mkdirSync / initializeWrite，
/// 看不出是环境问题）。
///
/// 这一层为什么会缺：商店（MSIX）版的 appData 被系统重定向到
/// `%LOCALAPPDATA%\Packages\<包名>\LocalCache\Roaming`，真正的
/// `%APPDATA%\TradingView` 从来没被创建过。托管副本是**解包运行**、不重定向，
/// 于是在"只装过商店版"的机器上第一次启动必崩。这不是用户环境的错，
/// 是两边的 appData 落点不同，替它把父目录补上就行。
pub fn client_logs_dir() -> Option<PathBuf> {
    // Electron 的 `app.getPath("appData")` 取的是 shell 已知文件夹，
    // **不看 `%APPDATA%` 环境变量**（实测：改环境变量后它照旧写原路径）。
    // `dirs::data_dir()` 在 Windows 上同样走 SHGetKnownFolderPath，两边一致。
    dirs::data_dir().map(|d| d.join("TradingView").join("logs"))
}

/// 幂等地把 `<app_data>/TradingView/logs` 建出来（递归，父层缺也一起补）。
pub fn ensure_client_logs_dir_at(app_data: &Path) -> std::io::Result<PathBuf> {
    let dir = app_data.join("TradingView").join("logs");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 同上，用系统解析出的 appData。
pub fn ensure_client_logs_dir() -> std::io::Result<PathBuf> {
    let base = dirs::data_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "无法确定 appData 目录")
    })?;
    ensure_client_logs_dir_at(&base)
}

/// 启动前的环境自查。返回空 = 正常；非空 = 一条条人话，直接可以给用户看。
pub fn environment_problems() -> Vec<EnvProblem> {
    let mut out = Vec::new();

    // 1) 用户目录与 AppData。Windows 上客户端固定用 `%APPDATA%\TradingView\logs`，
    //    父目录不存在时 mkdir 报 ENOENT（不是权限问题，EACCES 才是）；
    //    macOS/Linux 走 `$HOME`，没有这两个变量是正常现象，不能当错误报。
    #[cfg(windows)]
    let profile = check_env_dir(&mut out, "no-userprofile", "USERPROFILE");
    #[cfg(windows)]
    let _appdata = check_env_dir(&mut out, "no-appdata", "APPDATA");
    #[cfg(not(windows))]
    let profile = check_env_dir(&mut out, "no-home-var", "HOME");

    #[cfg(windows)]
    if let Some(p) = &profile {
        let roaming = p.join("AppData").join("Roaming");
        if p.is_dir() && !roaming.is_dir() {
            out.push(EnvProblem::new(
                "broken-profile",
                format!(
                    "用户配置不完整：{} 里没有 AppData\\Roaming（客户端会在建 logs 目录时崩溃）",
                    p.display()
                ),
            ));
        }
    }

    // 2) ~ 的解析结果和 USERPROFILE 不一致：账号会被建到另一个地方，
    //    表现是"重启后账号不见了"或"两个界面看到的账号不一样"。
    match (dirs::home_dir(), &profile) {
        (Some(h), Some(u)) if h != *u => out.push(EnvProblem::new(
            "home-mismatch",
            format!(
                "用户目录解析不一致：home_dir={} 但 USERPROFILE={}",
                h.display(),
                u.display()
            ),
        )),
        (None, _) => out.push(EnvProblem::new(
            "no-home",
            "无法确定用户目录（home_dir 为空），配置会落到当前工作目录".to_string(),
        )),
        _ => {}
    }

    // 3) 客户端自己的日志目录。缺了它客户端会在启动阶段 ENOENT 崩掉，
    //    所以这里不只是"检查"——直接替它建出来（幂等，已存在时什么也不做）。
    //    这是"只装过商店版的机器"上客户端起不来的真正原因，见 client_logs_dir()。
    match ensure_client_logs_dir() {
        Ok(_) => {}
        Err(e) => out.push(EnvProblem::new(
            "client-logs",
            format!(
                "客户端日志目录建不了：{}（{e}）——客户端启动会以 ENOENT 崩溃",
                client_logs_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(未知)".into())
            ),
        )),
    }

    // 4) 配置目录必须可建可写：PID/登录去重/开关都落在里面，写不上等于功能全废。
    let root = tvx_root();
    if let Err(e) = fs::create_dir_all(&root) {
        out.push(EnvProblem::new(
            "root-unwritable",
            format!("配置目录无法创建：{}（{e}）", root.display()),
        ));
    } else {
        let probe = root.join(".write-probe");
        match fs::write(&probe, b"1") {
            Ok(_) => {
                let _ = fs::remove_file(&probe);
            }
            Err(e) => out.push(EnvProblem::new(
                "root-unwritable",
                format!("配置目录不可写：{}（{e}）", root.display()),
            )),
        }
    }

    out
}

/// 一行行拼好的环境报告（`_doctor` 与界面「自检」共用）。
pub fn doctor_report() -> String {
    let mut s = String::new();
    s.push_str("TVHelper 环境自检\n");
    s.push_str(&format!("平台: {}\n", std::env::consts::OS));
    #[cfg(windows)]
    let vars = ["USERPROFILE", "APPDATA", "LOCALAPPDATA"];
    #[cfg(not(windows))]
    let vars = ["HOME", "XDG_CONFIG_HOME"];
    for v in vars {
        let val = std::env::var(v).unwrap_or_else(|_| "(未设置)".into());
        let ok = std::env::var_os(v)
            .map(|p| PathBuf::from(p).is_dir())
            .unwrap_or(false);
        s.push_str(&format!("{v}: {val} [{}]\n", if ok { "存在" } else { "不存在" }));
    }
    let home = dirs::home_dir();
    s.push_str(&format!(
        "home_dir: {}\n",
        home.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "(空)".into())
    ));
    s.push_str(&format!("配置目录: {}\n", tvx_root().display()));
    s.push_str(&format!("托管副本: {}\n", managed_root().display()));

    let probs = environment_problems();
    if probs.is_empty() {
        s.push_str("环境检查: 通过\n");
    } else {
        s.push_str(&format!("环境检查: {} 项异常\n", probs.len()));
        for p in &probs {
            s.push_str(&format!("  ! [{}] {}\n", p.key, p.detail));
        }
    }

    // 上面 environment_problems() 会顺手把客户端的日志目录建出来，这里报最终状态
    let logs = client_logs_dir();
    s.push_str(&format!(
        "客户端日志目录: {} [{}]\n",
        logs.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(未知)".into()),
        match &logs {
            Some(p) if p.is_dir() => "已就绪",
            _ => "建不了",
        }
    ));

    let list = crate::profiles::list();
    s.push_str(&format!("账号: {} 个\n", list.len()));
    for p in &list {
        s.push_str(&format!(
            "  - {} -> {} [{}]\n",
            p.name,
            p.dir.display(),
            if p.dir.is_dir() { "存在" } else { "不存在" }
        ));
    }
    match load_managed() {
        Some(m) => {
            s.push_str(&format!("副本版本: {}\n", m.version));
            s.push_str(&format!(
                "副本 exe: {}\n",
                m.exe().map(|p| p.display().to_string()).unwrap_or_else(|| "(缺失)".into())
            ));
            let cfg = load_config();
            let failed = patch::failing_patches(&m.index_js(), &cfg.flags());
            s.push_str(&format!(
                "补丁: {} / {}（{}）\n",
                patch::PATCHES.len() - failed.len(),
                patch::PATCHES.len(),
                if m.asar_residue() {
                    "文件里齐了，但因旧包残留不会被执行"
                } else {
                    "已生效"
                }
            ));
            if !failed.is_empty() {
                s.push_str(&format!("  未生效: {}\n", failed.join("、")));
                s.push_str(
                    "  说明: 单条锚点没对上只影响该功能，其它补丁照常工作；\n\
                     \x20       上游代码结构变了才会这样，重建副本通常也修不了，请把这份报告反馈\n",
                );
            }

            // 页内推广守卫写在 preload 里：它比页面脚本早，负责关掉那个
            // "登录后 10 分钟自动弹、没有关闭按钮"的桌面端升级墙的定时器。
            // 主进程补丁全绿也不代表它在——preload 是另一个文件，漏了同样要单独说。
            if cfg.flags().adblock {
                let preload = m
                    .index_js()
                    .parent()
                    .map(|p| p.join("preload").join("platform-preload.js"));
                let ok = preload
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .map(|t| t.contains(patch::PROMO_GUARD_MARK))
                    .unwrap_or(false);
                s.push_str(&format!(
                    "页内推广守卫: {}\n",
                    if ok {
                        "已就位"
                    } else {
                        "缺失 —— 自动升级墙仍会弹；点「重建副本」修复"
                    }
                ));
            }
            if m.asar_residue() {
                s.push_str(&format!(
                    "⚠ 旧包残留: {}\n",
                    m.resources().join("app.asar").display()
                ));
                s.push_str(
                    "  后果: 客户端优先加载 app.asar，app/index.js 里的补丁一条都不会执行\n\
                     \x20       （界面可能仍显示“补丁完整”，但所有功能都不生效）\n\
                     怎么做:\n\
                     \x20 1. 关掉所有客户端窗口（或在任务管理器结束 TradingView.exe）\n\
                     \x20 2. 打开助手，拨动任意一个开关 —— 会自动重打补丁并删掉这个文件\n\
                     \x20 3. 重新启动客户端\n\
                     \x20 若仍删不掉：把该目录加入杀软白名单，或手动删除上面列出的文件后重试\n",
                );
            }
        }
        None => s.push_str("副本: 未构建\n  怎么做: 点「添加」创建账号，首次会自动生成副本\n"),
    }
    s
}

