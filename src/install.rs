//! 定位已安装的 TradingView，并把“免弹窗副本”复制 + 解包 + 打补丁。
//!
//! 三平台的安装形态不一样，这里统一成一个 `Install`：
//! - Windows: MSIX 包目录（`...\WindowsApps\TradingView.Desktop_...`），里面有 `TradingView.exe`
//! - macOS:   `.app` 包，可执行在 `Contents/MacOS/TradingView`
//! - Linux:   普通目录，可执行在根下，资源在 `resources/`

use crate::i18n::{Lang, S};
use crate::{asar, patch};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- 数据模型

/// 安装来源的种类（界面标签随语言变化，所以存种类而不是成品字符串）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Windows Microsoft Store（MSIX）
    #[allow(dead_code)] // 只在 Windows 构建里构造
    Store,
    /// Windows 普通安装目录
    #[allow(dead_code)] // 只在 Windows 构建里构造
    Portable,
    /// macOS `.app`（Windows 构建里不会构造）
    #[allow(dead_code)]
    App,
    /// 用户手动指定
    Custom,
}

#[derive(Debug, Clone)]
pub struct Install {
    /// Windows/Linux 是安装根目录；macOS 是 `.app` 本身
    pub root: PathBuf,
    pub kind: InstallKind,
    pub version: String,
}

impl Install {
    /// 界面上显示的名字（随界面语言变化）
    ///
    /// 后面跟一个验证标记：补丁锚点只对白名单里的版本做过实测，其他版本必须
    /// 在界面上看得出来，否则用户会以为"能构建 = 能正常用"。
    pub fn label(&self, t: &S) -> String {
        let base = match self.kind {
            InstallKind::Store => format!("{}  {}", t.kind_store, self.version),
            InstallKind::Portable => format!("{}  {}", t.kind_portable, self.root.display()),
            InstallKind::Custom => format!("{}  {}", t.kind_custom, self.root.display()),
            InstallKind::App => self.root.display().to_string(),
        };
        let tag = if patch::is_verified(&self.version) {
            t.tag_verified
        } else {
            t.tag_unverified
        };
        format!("{base}  {tag}")
    }

    pub fn resources(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            self.root.join("Contents").join("Resources")
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.root.join("resources")
        }
    }

    pub fn asar(&self) -> PathBuf {
        self.resources().join("app.asar")
    }

    pub fn exe(&self) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        let candidates = vec![self.root.join("Contents").join("MacOS").join("TradingView")];
        #[cfg(target_os = "windows")]
        let candidates = vec![self.root.join("TradingView.exe")];
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        let candidates = ["tradingview", "TradingView", "TradingView.exe"]
            .iter()
            .map(|n| self.root.join(n))
            .collect::<Vec<_>>();

        candidates.into_iter().find(|p| p.is_file())
    }

    pub fn looks_valid(&self) -> bool {
        self.asar().is_file() && self.exe().is_some()
    }

    /// 从用户手选的目录/包构造一个来源。返回 None 表示这路径不像是 TradingView。
    pub fn from_path(p: &Path) -> Option<Install> {
        let is_bundle = p
            .file_name()
            .map(|n| n.to_string_lossy().ends_with(".app"))
            .unwrap_or(false);

        let root = if is_bundle {
            p.to_path_buf()
        } else if p.join("Contents").join("Resources").join("app.asar").is_file() {
            // 用户指到了 xxx.app/Contents
            p.parent()?.to_path_buf()
        } else {
            p.to_path_buf()
        };

        let inst = Install {
            version: "custom".into(),
            kind: InstallKind::Custom,
            root,
        };
        inst.looks_valid().then_some(inst)
    }
}

#[derive(Debug, Clone)]
pub struct Managed {
    /// 托管根目录
    pub dir: PathBuf,
    /// 真正的应用根（macOS 指向 `.app`，其他平台等于 dir）
    pub app: PathBuf,
    pub version: String,
}

impl Managed {
    pub fn exe(&self) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        let c = self.app.join("Contents").join("MacOS").join("TradingView");
        #[cfg(target_os = "windows")]
        let c = self.app.join("TradingView.exe");
        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        let c = ["tradingview", "TradingView", "TradingView.exe"]
            .iter()
            .map(|n| self.app.join(n))
            .find(|p| p.is_file())
            .unwrap_or_else(|| self.app.join("TradingView"));

        c.is_file().then_some(c)
    }

    pub fn resources(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            self.app.join("Contents").join("Resources")
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.app.join("resources")
        }
    }

    pub fn index_js(&self) -> PathBuf {
        self.resources().join("app").join("index.js")
    }

    pub fn is_patched(&self, flags: &patch::Flags) -> bool {
        patch::is_patched(&self.index_js(), flags)
    }
}

// ---------------------------------------------------------------- 持久化配置

fn default_true() -> bool {
    true
}

/// tvx 的持久化配置。功能开关对应六条需求：
/// 拦截广告网络 / 多图表解锁 / 指标叠加解锁 / 绘图数量解锁 / 禁用自动更新 各自独立；
/// 指标数量 / 布局保存 / 警报由服务端强制（实测拒绝码由服务端下发，
/// 客户端无法解除），只能静音其升级弹窗，故共用 `silence_paywalls` 一个开关。
/// `alerts` 是本地指标报警，与服务端报警配额无关，单独一个开关。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub lang: Lang,
    #[serde(default)]
    pub custom_source: Option<String>,
    #[serde(default = "default_true")]
    pub ads_block: bool,
    #[serde(default = "default_true")]
    pub charts_unlock: bool,
    #[serde(default = "default_true")]
    pub study_on_study: bool,
    /// 单张图表的绘图数量上限（free 档 30，客户端 gate）
    #[serde(default = "default_true")]
    pub drawings_unlock: bool,
    #[serde(default = "default_true")]
    pub silence_paywalls: bool,
    #[serde(default = "default_true")]
    pub no_auto_update: bool,
    /// 断线自愈：连接被服务端关闭时自动点恢复 / 重载页面
    #[serde(default = "default_true")]
    pub auto_recover: bool,
    /// 本地指标报警：图表页里的报警设置页 + 指标条件判断，命中后系统通知 / 邮件
    #[serde(default = "default_true")]
    pub alerts: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lang: Lang::Zh,
            custom_source: None,
            ads_block: true,
            charts_unlock: true,
            study_on_study: true,
            drawings_unlock: true,
            silence_paywalls: true,
            no_auto_update: true,
            auto_recover: true,
            alerts: true,
        }
    }
}

impl Config {
    pub fn flags(&self) -> patch::Flags {
        patch::Flags {
            ads_block: self.ads_block,
            charts_unlock: self.charts_unlock,
            study_on_study: self.study_on_study,
            drawings_unlock: self.drawings_unlock,
            silence_paywalls: self.silence_paywalls,
            no_auto_update: self.no_auto_update,
            auto_recover: self.auto_recover,
            alerts: self.alerts,
        }
    }
}

fn config_path() -> PathBuf {
    tvx_root().join("config.json")
}

pub fn load_config() -> Config {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(cfg: &Config) -> std::io::Result<()> {
    let s = serde_json::to_string_pretty(cfg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_atomic(&config_path(), s.as_bytes())
}

// ---------------------------------------------------------------- 目录约定

pub fn tvx_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".tvx")
}

pub fn managed_root() -> PathBuf {
    tvx_root().join("apps")
}

// ---------------------------------------------------------------- 启动前环境体检

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
            let ok = patch::is_patched(&m.index_js(), &cfg.flags());
            s.push_str(&format!("补丁: {}\n", if ok { "完整" } else { "不完整（有开关对应的补丁缺失）" }));
        }
        None => s.push_str("副本: 未构建\n"),
    }
    s
}

/// 原子写：先写同目录临时文件再 rename。
/// 直接 `fs::write` 覆盖时，断电/崩溃会留下半截 JSON —— 配置或 PID 直接损坏，
/// 而这两类文件恰好决定"开关是否生效"和"实例是否在跑"。
/// 目标被短暂占用（杀软扫描、Windows 索引）时重试几次。
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    let mut last = None;
    for attempt in 0..3u64 {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(80 * (attempt + 1)));
                }
            }
        }
    }
    let _ = fs::remove_file(&tmp);
    Err(last.expect("至少失败过一次"))
}

/// GUI 没有控制台：影响功能但不该弹窗的写入/清理失败，记一行到 ~/.tvx/error.log。
/// 不记就没有任何线索——用户只会看到"开关下次启动又回去了"。
pub fn log_error(context: &str, err: &dyn std::fmt::Display) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{ts}] {context}: {err}\n");
    let path = tvx_root().join("error.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn dest_for(src: &Install) -> PathBuf {
    managed_root().join(&src.version)
}

// ---------------------------------------------------------------- 活跃版本

/// 当前锁定的托管副本版本（`~/.tvx/active-app`，一行版本号）。
///
/// 单独一个文件而不是塞进 `config.json`：界面每次拨开关都会用内存里的 cfg
/// 整体覆盖 config.json，混在一起容易把刚写入的版本冲掉。
fn active_app_path() -> PathBuf {
    tvx_root().join("active-app")
}

pub fn active_app() -> Option<String> {
    let s = fs::read_to_string(active_app_path()).ok()?;
    let v = s.trim().to_string();
    (!v.is_empty()).then_some(v)
}

pub fn set_active_app(version: &str) -> std::io::Result<()> {
    write_atomic(&active_app_path(), format!("{version}\n").as_bytes())
}

/// 版本号按**数字段**比较：3.10.1 > 3.9.7（按字符串比会反过来）。
/// 非数字版本（custom / portable / app）解析成 [0]，永远排在最前，不会被当成"最新"。
pub fn ver_key(v: &str) -> Vec<u64> {
    v.split('.')
        .map(|s| s.trim().parse::<u64>().unwrap_or(0))
        .collect()
}

/// 在已发现的安装里找比当前副本更新的版本（客户端升级后副本仍停在旧版）。
/// 纯计算，不碰磁盘——界面每帧调用也安全。
pub fn newest_than<'a>(installs: &'a [Install], current: &str) -> Option<&'a Install> {
    let cur = ver_key(current);
    installs
        .iter()
        .filter(|i| ver_key(&i.version) > cur)
        .max_by(|a, b| ver_key(&a.version).cmp(&ver_key(&b.version)))
}

/// 默认选源：**已验证版本优先**，都未验证时退回版本号最大的那份。
///
/// 客户端升级后旧安装目录常常还留在盘上，用户机器上因此可能同时有两三个版本。
/// 「最新」正是锚点没验证过的那份，不该当默认；只有白名单里那版才默认选中。
pub fn preferred_index(installs: &[Install]) -> Option<usize> {
    if let Some(i) = installs.iter().position(|i| patch::is_verified(&i.version)) {
        return Some(i);
    }
    installs
        .iter()
        .enumerate()
        .max_by(|a, b| ver_key(&a.1.version).cmp(&ver_key(&b.1.version)))
        .map(|(i, _)| i)
}

// ---------------------------------------------------------------- 发现安装

pub fn discover() -> Vec<Install> {
    let mut out: Vec<Install> = Vec::new();

    #[cfg(windows)]
    discover_windows(&mut out);
    #[cfg(target_os = "macos")]
    discover_macos(&mut out);
    #[cfg(all(not(windows), not(target_os = "macos")))]
    discover_linux(&mut out);

    // 手动指定的来源永远排在最后，方便用户认领
    if let Some(p) = load_config().custom_source {
        if let Some(inst) = Install::from_path(Path::new(&p)) {
            out.retain(|i| i.root != inst.root);
            out.push(inst);
        }
    }

    out.sort_by(|a, b| a.version.cmp(&b.version));
    out.dedup_by(|a, b| a.root == b.root);
    out
}

#[cfg(windows)]
fn discover_windows(out: &mut Vec<Install>) {
    use winreg::enums::*;
    use winreg::RegKey;

    // MSIX 的安装位置只能从注册表拿：WindowsApps 根目录不允许枚举
    let key = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    if let Ok(pkgs) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(key, KEY_READ) {
        for name in pkgs.enum_keys().flatten() {
            if !name.starts_with("TradingView.Desktop_") {
                continue;
            }
            let Ok(sub) = pkgs.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            let Ok(root) = sub.get_value::<String, _>("PackageRootFolder") else {
                continue;
            };
            let version = name.split('_').nth(1).unwrap_or("unknown").to_string();
            let inst = Install {
                kind: InstallKind::Store,
                version,
                root: PathBuf::from(root),
            };
            if inst.looks_valid() {
                out.push(inst);
            }
        }
    }

    let mut candidates = Vec::new();
    if let Some(d) = dirs::data_local_dir() {
        candidates.push(d.join("Programs").join("TradingView"));
        candidates.push(d.join("TradingView"));
    }
    if let Some(d) = dirs::data_dir() {
        candidates.push(d.join("TradingView"));
    }
    for c in candidates {
        let inst = Install {
            version: "portable".into(),
            kind: InstallKind::Portable,
            root: c,
        };
        if inst.looks_valid() {
            out.push(inst);
        }
    }
}

#[cfg(target_os = "macos")]
fn discover_macos(out: &mut Vec<Install>) {
    let mut candidates = vec![PathBuf::from("/Applications/TradingView.app")];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Applications").join("TradingView.app"));
    }
    for c in candidates {
        let inst = Install {
            version: "app".into(),
            kind: InstallKind::App,
            root: c,
        };
        if inst.looks_valid() {
            out.push(inst);
        }
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn discover_linux(out: &mut Vec<Install>) {
    let mut candidates = vec![
        PathBuf::from("/opt/TradingView"),
        PathBuf::from("/usr/lib/tradingview"),
        PathBuf::from("/usr/share/tradingview"),
        PathBuf::from("/opt/tradingview"),
    ];
    if let Some(home) = dirs::home_dir() {
        for sub in [
            ".local/share/TradingView",
            ".local/share/tradingview",
            "Applications/TradingView",
            ".local/opt/TradingView",
        ] {
            candidates.push(home.join(sub));
        }
    }
    for c in candidates {
        let inst = Install {
            version: "app".into(),
            kind: InstallKind::App,
            root: c,
        };
        if inst.looks_valid() {
            out.push(inst);
        }
    }
}

// ---------------------------------------------------------------- 托管构建

/// 找到"当前应使用"的托管副本。
///
/// 历史上这里是"遍历到最后一个是哪个就是哪个"，`apps/` 下同时存在两个版本目录时
/// 返回结果取决于目录枚举顺序——可能启动到旧版本、甚至是没打补丁的那份。
/// 现在改成确定性选择：锁定版本优先，否则取版本号最大的一份并锁定它。
pub fn load_managed() -> Option<Managed> {
    let rd = fs::read_dir(managed_root()).ok()?;
    let mut found: Vec<Managed> = Vec::new();
    for entry in rd.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Some(name) = dir.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        #[cfg(target_os = "macos")]
        let app = dir.join("TradingView.app");
        #[cfg(not(target_os = "macos"))]
        let app = dir.clone();

        let m = Managed {
            dir: dir.clone(),
            app,
            version: name,
        };
        if m.exe().is_some() {
            found.push(m);
        }
    }
    if found.is_empty() {
        return None;
    }

    // 1) 上次实际使用的版本优先（多版本共存时行为可预期）
    if let Some(v) = active_app() {
        if let Some(m) = found.iter().find(|m| m.version == v) {
            return Some(m.clone());
        }
    }
    // 2) 没有锁定记录、或锁定的目录已被删：取版本号最大的一份，并锁定
    found.sort_by(|a, b| ver_key(&a.version).cmp(&ver_key(&b.version)));
    let best = found.pop()?;
    if let Err(e) = set_active_app(&best.version) {
        log_error("记录活跃副本版本 active-app", &e);
    }
    Some(best)
}

/// 复制 → 解包 asar → 按配置打补丁（macOS 再重签名）。
///
/// 先在旁边的暂存目录里完整构建，成功后才替换正式目录：复制几百 MB 的中途失败、
/// 或补丁写回失败，都只丢暂存目录，用户已有的副本原封不动。
pub fn build(
    src: &Install,
    dest: &Path,
    flags: &patch::Flags,
    lang: Lang,
    log: &mut dyn FnMut(String),
) -> Result<Managed> {
    if !src.looks_valid() {
        bail!(lang.b_incomplete(&src.root.display().to_string()));
    }

    // 未在白名单里的版本：先明说"锚点不是按它提的"，再让它尽力打。
    // 打不上的条目会在 build_into 里逐条标记失败，不会静默当成功。
    if !patch::is_verified(&src.version) {
        log(lang.b_unverified(&src.version, patch::VERIFIED_VERSION));
    }

    let staging = staging_dir(dest);
    // 上次中断留下的暂存目录先清掉（可能被占用；删不掉就让下面的构建自己报错）
    let _ = remove_dir_retry(&staging);

    if let Err(e) = build_into(src, &staging, flags, lang, log) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    if dest.exists() {
        log(lang.b_cleanup(&dest.display().to_string()));
        // 旧副本可能正被运行中的实例占着（Windows 会锁 exe），重试几次再放弃
        remove_dir_retry(dest)
            .with_context(|| lang.b_delete_failed(&dest.display().to_string()))?;
    }
    fs::rename(&staging, dest).with_context(|| {
        format!(
            "把构建好的副本从 {} 移到 {} 失败",
            staging.display(),
            dest.display()
        )
    })?;

    Ok(Managed {
        dir: dest.to_path_buf(),
        app: app_dir_in(dest),
        version: src.version.clone(),
    })
}

/// 托管副本里的可执行根：macOS 是 .app 外壳，其他平台就是目录本身。
fn app_dir_in(dest: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        dest.join("TradingView.app")
    }
    #[cfg(not(target_os = "macos"))]
    {
        dest.to_path_buf()
    }
}

/// 与 dest 同级的隐藏暂存目录（同一分区，最后的 rename 才是原子的）。
fn staging_dir(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "copy".into());
    dest.with_file_name(format!(".{name}.building"))
}

/// 删除目录；Windows 上遇到实例占用 / 杀软扫描时重试几次，而不是一次失败就放弃。
fn remove_dir_retry(dir: &Path) -> std::io::Result<()> {
    use std::time::Duration;
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..4u64 {
        match fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if !dir.exists() {
                    return Ok(());
                }
                last = Some(e);
                if attempt < 3 {
                    std::thread::sleep(Duration::from_millis(250 * (attempt + 1)));
                }
            }
        }
    }
    Err(last.expect("至少失败过一次"))
}

fn build_into(
    src: &Install,
    dest: &Path,
    flags: &patch::Flags,
    lang: Lang,
    log: &mut dyn FnMut(String),
) -> Result<()> {
    // macOS 必须保留 .app 外壳，否则 Info.plist / 签名校验会出问题
    let app = app_dir_in(dest);

    log(lang.b_copying(
        &src.root.display().to_string(),
        &app.display().to_string(),
    ));
    let mut n = 0usize;
    copy_tree(&src.root, &app, &mut n, lang, log)?;
    log(lang.b_copied(n));

    let resources = {
        #[cfg(target_os = "macos")]
        {
            app.join("Contents").join("Resources")
        }
        #[cfg(not(target_os = "macos"))]
        {
            app.join("resources")
        }
    };
    let asar_path = resources.join("app.asar");
    let app_dir = resources.join("app");

    log(lang.b_unpack());
    let st = asar::extract(&asar_path, &app_dir)?;
    log(lang.b_unpacked(st.files, st.skipped_links));

    // 改成目录加载后，asar 与 unpacked 必须移除，否则会被优先读取
    fs::remove_file(&asar_path).ok();
    let unpacked = PathBuf::from(format!("{}.unpacked", asar_path.to_string_lossy()));
    if unpacked.is_dir() {
        fs::remove_dir_all(&unpacked).ok();
    }

    log(lang.b_patching());
    // 补丁逐条应用：单条锚点失配只跳过该功能，不再让整次构建失败
    let applied = patch::apply(&app_dir.join("index.js"), flags)?;
    for &(name, st) in &applied.items {
        // + 打上 / = 已打过 / - 撤回 / · 未启用 / ! 未命中
        let shown: &str = if lang == Lang::En {
            patch::name_en(name)
        } else {
            name
        };
        log(format!("  [{}] {shown}", st.mark()));
    }
    for w in &applied.warnings {
        log(format!("  ! {w}"));
    }
    if !applied.failed.is_empty() {
        log(lang.log_patch_partial(&applied.failed));
    }
    // 报警功能是「文件 + 一处 require」，不只改 index.js，所以补丁之后还要落资源
    patch::install_assets(&app_dir, flags)?;

    resign_if_needed(&app, lang, log)?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn resign_if_needed(app: &Path, lang: Lang, log: &mut dyn FnMut(String)) -> Result<()> {
    use std::process::Command;
    // 改过 app.asar 之后原签名必然失效，必须 ad-hoc 重签，否则 Gatekeeper 会拦
    let _ = Command::new("xattr").arg("-cr").arg(app).status();
    match Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(app)
        .status()
    {
        Ok(s) if s.success() => {
            log(lang.b_resigned());
            Ok(())
        }
        _ => {
            log(lang.b_resign_warn());
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn resign_if_needed(_app: &Path, _lang: Lang, _log: &mut dyn FnMut(String)) -> Result<()> {
    Ok(())
}

fn copy_tree(
    from: &Path,
    to: &Path,
    n: &mut usize,
    lang: Lang,
    log: &mut dyn FnMut(String),
) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from).with_context(|| format!("枚举 {}", from.display()))? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_tree(&src, &dst, n, lang, log)?;
        } else if ft.is_file() {
            fs::copy(&src, &dst).with_context(|| format!("复制 {}", src.display()))?;
            *n += 1;
            if *n % 40 == 0 {
                log(lang.b_progress(*n));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 客户端的 `createDir(e, {})` 是非递归 mkdir，父层 `TradingView` 不存在时
    /// 它必崩（`ENOENT: mkdir '…\TradingView\logs'`）。所以助手必须能把整条
    /// 路径一次建出来，而不是只建最后一级。
    #[test]
    fn client_logs_dir_is_created_with_missing_parent() {
        let base = std::env::temp_dir().join(format!("tvx-logs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        // 模拟"只装过商店版"的机器：appData 在，TradingView 这一层不在
        assert!(!base.join("TradingView").exists());

        let dir = ensure_client_logs_dir_at(&base).expect("应当能建出来");
        assert!(dir.is_dir(), "{} 没建出来", dir.display());
        assert!(dir.ends_with("TradingView/logs") || dir.ends_with("TradingView\\logs"));

        // 幂等：再调一次不报错
        assert!(ensure_client_logs_dir_at(&base).is_ok());
        let _ = fs::remove_dir_all(&base);
    }

    /// 版本号必须按数字段比较：字符串比较会把 "3.10.1" 排在 "3.9.7" 前面，
    /// 于是客户端升级后反而锁定到旧副本。
    #[test]
    fn version_compare_is_numeric() {
        assert!(ver_key("3.10.1") > ver_key("3.9.7"));
        assert!(ver_key("3.5.0.8375") > ver_key("3.5.0.999"));
        assert!(ver_key("4.0.0") > ver_key("3.99.99"));
        // 非数字版本（custom / portable / app）永远不被当成"最新"
        assert!(ver_key("3.5.0.8375") > ver_key("custom"));
        assert!(ver_key("0.0.1") > ver_key("portable"));
    }

    /// 客户端升级后要能挑出"比当前副本新"的最高版本；没有更高版本时返回 None
    #[test]
    fn newest_than_picks_highest_newer_install() {
        let mk = |v: &str| Install {
            version: v.into(),
            kind: InstallKind::Custom,
            root: PathBuf::from("C:/nope").join(v),
        };
        let all = vec![
            mk("3.5.0.8375"),
            mk("3.6.0.9000"),
            mk("3.10.0.1"),
            mk("custom"),
        ];
        assert_eq!(
            newest_than(&all, "3.5.0.8375").map(|i| i.version.as_str()),
            Some("3.10.0.1")
        );
        // 已经是最新：不再提示
        assert!(newest_than(&all, "3.10.0.1").is_none());
        // 只有非数字版本时不误报
        assert!(newest_than(&[mk("custom"), mk("portable")], "3.5.0.8375").is_none());
    }

    /// 默认选源必须落在白名单版本上，而不是"版本号最大"的那份。
    /// 这就是"版本锁"的入口：客户端升级后盘上留着 3.6.x，默认不能选它。
    #[test]
    fn preferred_index_prefers_verified_version() {
        let mk = |v: &str| Install {
            version: v.into(),
            kind: InstallKind::Custom,
            root: PathBuf::from("C:/nope").join(v),
        };
        let all = vec![
            mk("3.6.0.9000"),
            mk(patch::VERIFIED_VERSION),
            mk("custom"),
        ];
        assert_eq!(preferred_index(&all), Some(1), "有已验证版本时选它");

        // 都不在白名单：退回版本号最大的那份（保持原有行为，仍可构建但会弹风险确认）
        let newer_only = vec![mk("3.6.0.9000"), mk("3.5.0.8375.1")];
        assert_eq!(preferred_index(&newer_only), Some(0));

        assert_eq!(preferred_index(&[]), None);
    }

    #[test]
    fn verified_whitelist_is_consistent() {
        // 白名单里至少要包含 "当前用于提取锚点" 的那个版本，否则界面提示会自相矛盾
        assert!(patch::is_verified(patch::VERIFIED_VERSION));
        assert!(!patch::is_verified("3.6.0.9000"));
        // 非版本号（portable / custom / app）一律未验证
        assert!(!patch::is_verified("portable"));
        assert!(!patch::is_verified(""));
    }

    #[test]
    fn build_end_to_end() {
        let installs = discover();
        let Some(src) = installs.first() else {
            eprintln!("跳过：系统里没有安装 TradingView");
            return;
        };
        eprintln!("源：{} ({})", src.root.display(), src.version);

        let dest = std::env::temp_dir().join("tvx-build-test");
        let mut log = |m: String| eprintln!("  {m}");
        let m = build(src, &dest, &patch::Flags::default(), Lang::Zh, &mut log).expect("构建失败");

        assert!(m.exe().is_some(), "副本里没有可执行文件");
        assert!(m.is_patched(&patch::Flags::default()), "补丁没生效");
        assert!(m.index_js().is_file(), "index.js 不在预期位置");
        assert!(
            !m.resources().join("app.asar").exists(),
            "asar 应该已被目录取代"
        );
        let _ = fs::remove_dir_all(&dest);
    }
}
