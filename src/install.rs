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
    pub fn label(&self, t: &S) -> String {
        match self.kind {
            InstallKind::Store => format!("{}  {}", t.kind_store, self.version),
            InstallKind::Portable => format!("{}  {}", t.kind_portable, self.root.display()),
            InstallKind::Custom => format!("{}  {}", t.kind_custom, self.root.display()),
            InstallKind::App => self.root.display().to_string(),
        }
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
/// 拦截广告网络 / 多图表解锁 / 指标叠加解锁 / 禁用自动更新 各自独立；
/// 指标数量 / 布局保存 / 警报由服务端强制（实测拒绝码由服务端下发，
/// 客户端无法解除），只能静音其升级弹窗，故共用 `silence_paywalls` 一个开关。
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
    #[serde(default = "default_true")]
    pub silence_paywalls: bool,
    #[serde(default = "default_true")]
    pub no_auto_update: bool,
    /// 登录回调直达：把 --user-data-dir 写进 tradingview:// 注册
    #[serde(default = "default_true")]
    pub auth_route: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lang: Lang::Zh,
            custom_source: None,
            ads_block: true,
            charts_unlock: true,
            study_on_study: true,
            silence_paywalls: true,
            no_auto_update: true,
            auth_route: true,
        }
    }
}

impl Config {
    pub fn flags(&self) -> patch::Flags {
        patch::Flags {
            ads_block: self.ads_block,
            charts_unlock: self.charts_unlock,
            study_on_study: self.study_on_study,
            silence_paywalls: self.silence_paywalls,
            no_auto_update: self.no_auto_update,
            auth_route: self.auth_route,
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

pub fn save_config(cfg: &Config) {
    let _ = fs::create_dir_all(tvx_root());
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(config_path(), s);
    }
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

pub fn dest_for(src: &Install) -> PathBuf {
    managed_root().join(&src.version)
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

pub fn load_managed() -> Option<Managed> {
    let rd = fs::read_dir(managed_root()).ok()?;
    let mut found: Option<Managed> = None;
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
            found = Some(m);
        }
    }
    found
}

/// 复制 → 解包 asar → 按配置打补丁（macOS 再重签名）。失败即整体放弃。
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

    if dest.exists() {
        log(lang.b_cleanup(&dest.display().to_string()));
        fs::remove_dir_all(dest).with_context(|| lang.b_delete_failed(&dest.display().to_string()))?;
    }

    // macOS 必须保留 .app 外壳，否则 Info.plist / 签名校验会出问题
    #[cfg(target_os = "macos")]
    let app = dest.join("TradingView.app");
    #[cfg(not(target_os = "macos"))]
    let app = dest.to_path_buf();

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
    for (name, st) in patch::apply(&app_dir.join("index.js"), flags)? {
        // + 打上 / = 已打过 / - 撤回 / · 未启用
        let shown: &str = if lang == Lang::En {
            patch::name_en(name)
        } else {
            name
        };
        log(format!("  [{}] {shown}", st.mark()));
    }

    resign_if_needed(&app, lang, log)?;

    Ok(Managed {
        dir: dest.to_path_buf(),
        app,
        version: src.version.clone(),
    })
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
