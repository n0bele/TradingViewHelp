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

    /// `resources/app.asar` 是否还残留。
    ///
    /// 残留时 Electron 优先加载 asar，`resources/app/index.js` 里的补丁**一条都不会执行**；
    /// 而 `is_patched` 读的正是那个 index.js，所以它照样返回 true——这就是"徽标正常、
    /// 功能全无、零报错"的成因。判「是否可用」必须同时看这一条。
    pub fn asar_residue(&self) -> bool {
        self.resources().join("app.asar").is_file()
    }

    /// 清掉残留的旧包。返回 `Ok(true)` 表示本次真的删掉了。
    ///
    /// 重打补丁时先走这一步：用户不必为了一个文件去重建 400MB 副本，
    /// 拨一下开关就能把病根去掉。删不掉通常是被运行中的实例占住或杀软在扫描，
    /// 这时把原因和处理方式一起交给调用方，别让用户只看到一句「失败」。
    pub fn clear_asar_residue(&self) -> Result<bool> {
        let path = self.resources().join("app.asar");
        if !path.is_file() {
            return Ok(false);
        }
        remove_file_retry(&path).or_else(|e| {
            // 重试仍失败：再确认一次，别把"其实已经没了"报成失败
            if path.is_file() {
                Err(e).with_context(|| format!("删除 {} 失败", path.display()))
            } else {
                Ok(())
            }
        })?;
        Ok(!path.is_file())
    }
}

// ---------------------------------------------------------------- 持久化配置

fn default_true() -> bool {
    true
}

/// tvx 的持久化配置。功能开关对应五条需求：
/// 拦截广告弹窗 / 多图表解锁 / 指标叠加解锁 / 绘图数量解锁 / 禁用自动更新 各自独立；
/// 「拦截广告弹窗」是一个功能：网络层拦广告域名 + 壳层不再开充值窗口 + 图表页内推广浮层
/// 就地隐藏，三者对用户是同一件事，共用一个开关（旧版的 ads_block / silence_paywalls
/// 已合并；JSON 键沿用 `ads_block`，老配置不用迁移）。
/// `alerts` 是本地指标报警，与服务端报警配额无关，单独一个开关。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub lang: Lang,
    #[serde(default)]
    pub custom_source: Option<String>,
    /// 拦截广告弹窗：广告域名 + 订阅/充值窗口 + 页内推广浮层
    #[serde(default = "default_true", rename = "ads_block")]
    pub adblock: bool,
    #[serde(default = "default_true")]
    pub charts_unlock: bool,
    #[serde(default = "default_true")]
    pub study_on_study: bool,
    /// 单张图表的绘图数量上限（free 档 30，客户端 gate）
    #[serde(default = "default_true")]
    pub drawings_unlock: bool,
    /// 回测报告的客户端判定项（扩展指标 / CSV·XLSX 导出），服务端算力项不在其中
    #[serde(default = "default_true")]
    pub backtest_unlock: bool,
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
            adblock: true,
            charts_unlock: true,
            study_on_study: true,
            drawings_unlock: true,
            backtest_unlock: true,
            no_auto_update: true,
            auto_recover: true,
            alerts: true,
        }
    }
}

impl Config {
    pub fn flags(&self) -> patch::Flags {
        patch::Flags {
            adblock: self.adblock,
            charts_unlock: self.charts_unlock,
            study_on_study: self.study_on_study,
            drawings_unlock: self.drawings_unlock,
            backtest_unlock: self.backtest_unlock,
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

mod build;
mod discover;
mod health;

pub use build::build;
pub use discover::discover;
pub use health::{doctor_report, environment_problems, EnvProblem};
// 清残留旧包与构建共用同一套"删不掉就重试"的文件操作
use build::remove_file_retry;

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

#[cfg(test)]
mod tests;
