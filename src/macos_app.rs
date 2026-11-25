//! macOS 的多账号隔离：**每个账号一份 `.app` 副本**。
//!
//! 为什么不能像 Windows 那样只换 `--user-data-dir`：
//! - macOS 的 `tradingview://` 由 App bundle 的 `Info.plist` 声明，LaunchServices
//!   对同一 scheme 只有一个默认处理程序，且**不能携带命令行参数**；
//! - GUI App 接收 URL 走 Apple Event，不读 argv。
//!
//! 所以要让登录回调落回"正在登录的那个账号"，只能让每个账号在系统眼里是**独立的 App**：
//! 复制一份 `.app` → 改 `CFBundleIdentifier` / `CFBundleName` → 重新 ad-hoc 签名 →
//! 登录前把 `tradingview://` 的默认处理程序指向这份副本。
//!
//! 复制优先用 APFS `clonefile`（`cp -c`，几乎不额外占空间），不支持时退回 `ditto`。

use crate::install::Managed;
use crate::profiles::Profile;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 账号名 → bundle id 片段（只保留字母数字，其余转 '-'）
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let s = out.trim_matches('-').to_string();
    // 全中文等非 ASCII 名称会得到空串，退化成按字节的短哈希，保证唯一且合法
    if s.is_empty() {
        let mut h: u32 = 2166136261;
        for b in name.as_bytes() {
            h ^= *b as u32;
            h = h.wrapping_mul(16777619);
        }
        format!("u{h:08x}")
    } else {
        s
    }
}

/// 该账号副本的 bundle id
pub fn bundle_id(profile: &Profile) -> String {
    format!("com.tradingview.desktop.tvx.{}", slug(&profile.name))
}

/// 该账号副本的显示名（也决定 `~/Library/Application Support/<名字>` 的位置）
fn app_name(profile: &Profile) -> String {
    format!("TradingView-{}", slug(&profile.name))
}

/// 副本所在目录：`~/.tvx/instances/<账号>/TradingView.app`
///
/// 单独放 `instances/` 而不是 `apps/`：`apps/` 是托管副本的扫描根目录，
/// 把账号副本放进去会被 `load_managed()` 误认成另一个"版本"。
fn app_path(profile: &Profile) -> PathBuf {
    crate::install::tvx_root()
        .join("instances")
        .join(slug(&profile.name))
        .join("TradingView.app")
}

fn plist_set(plist: &Path, key: &str, value: &str) -> bool {
    // 先试 Set（键已存在），失败再 Add
    let set = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Set :{key} {value}")])
        .arg(plist)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if set {
        return true;
    }
    Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Add :{key} string {value}")])
        .arg(plist)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 读 Info.plist 里的某个键（用于判断副本是否已就绪）
fn plist_get(plist: &Path, key: &str) -> Option<String> {
    let out = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", &format!("Print :{key}")])
        .arg(plist)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// ad-hoc 重签：改过 Info.plist 后原签名必然失效，否则 Gatekeeper 会拦
fn resign(app: &Path) -> Result<()> {
    let _ = Command::new("xattr").args(["-cr"]).arg(app).status();
    let st = Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(app)
        .status()
        .with_context(|| format!("codesign {}", app.display()))?;
    if !st.success() {
        bail!("ad-hoc 重签失败：{}", app.display());
    }
    Ok(())
}

/// 确保该账号有自己的一份 `.app` 副本；返回 `.app` 路径。
pub fn ensure_profile_app(managed: &Managed, profile: &Profile) -> Result<PathBuf> {
    let app = app_path(profile);
    let plist = app.join("Contents").join("Info.plist");
    let exe = app.join("Contents").join("MacOS").join("TradingView");
    let want_bid = bundle_id(profile);

    // 已经就绪（副本在、bundle id 已是本账号的）就直接用
    if exe.is_file() && plist_get(&plist, "CFBundleIdentifier").as_deref() == Some(want_bid.as_str())
    {
        return Ok(app);
    }

    // 从托管副本重新复制
    if let Some(parent) = app.parent() {
        fs::create_dir_all(parent)?;
    }
    if app.exists() {
        fs::remove_dir_all(&app).ok();
    }
    let src = managed.app.clone();
    // APFS clonefile 最快、几乎不占空间；失败退回 ditto
    let cloned = Command::new("cp")
        .arg("-c")
        .arg("-R")
        .arg(&src)
        .arg(&app)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !cloned || !exe.is_file() {
        if app.exists() {
            fs::remove_dir_all(&app).ok();
        }
        let st = Command::new("ditto").arg(&src).arg(&app).status()?;
        if !st.success() || !exe.is_file() {
            bail!("复制 .app 失败：{} → {}", src.display(), app.display());
        }
    }

    if !plist.is_file() {
        bail!("副本缺少 Info.plist：{}", plist.display());
    }
    // 写入 .app 图标（Finder / Dock 显示用）
    let res_dir = app.join("Contents").join("Resources");
    let _ = fs::create_dir_all(&res_dir);
    let _ = fs::write(res_dir.join("AppIcon.icns"), include_bytes!("../assets/icon.icns"));
    let name = app_name(profile);
    for (k, v) in [
        ("CFBundleIdentifier", want_bid.as_str()),
        ("CFBundleName", name.as_str()),
        ("CFBundleDisplayName", name.as_str()),
        ("CFBundleIconFile", "AppIcon"),
    ] {
        if !plist_set(&plist, k, v) {
            bail!("改写 Info.plist 失败：{k}");
        }
    }

    resign(&app)?;
    Ok(app)
}

/// 把 `~/Library/Application Support/<副本名>` 做成指向账号目录的符号链接。
///
/// macOS 上**不能**给启动加 `--user-data-dir`：系统为 URL 回调拉起的进程不带参数，
/// 会用副本的默认 userData；一旦两边 userData 不同，单实例锁也不共享，
/// 回调就会落进另一个实例。让默认 userData 就是我们账号目录（符号链接），
/// 两边路径一致 → 同一个实例 → 授权回调送达。
pub fn ensure_userdata_link(profile: &Profile) -> Result<()> {
    let Some(base) = dirs::data_dir() else {
        bail!("找不到 Application Support 目录");
    };
    let link = base.join(app_name(profile));
    let target = &profile.dir;

    if let Ok(meta) = fs::symlink_metadata(&link) {
        if meta.file_type().is_symlink() {
            if fs::read_link(&link).map(|p| p == *target).unwrap_or(false) {
                return Ok(()); // 已指向正确位置
            }
            fs::remove_file(&link).ok();
        } else {
            // 真实目录：可能是旧版留下的数据，保守起见不动它
            return Ok(());
        }
    }

    fs::create_dir_all(target)?;
    std::os::unix::fs::symlink(target, &link)
        .with_context(|| format!("创建符号链接 {}", link.display()))?;
    Ok(())
}

/// 把 `tradingview://` 的默认处理程序指向该账号的副本。
///
/// 走 CoreServices 的 `LSSetDefaultHandlerForURLScheme`（JXA 调用）。
/// 这一步需要 macOS 允许修改默认处理程序；失败时返回错误，由上层记录，
/// 不至于让"启动账号"整体失败（登录仍可在窗口内完成）。
pub fn set_url_handler(profile: &Profile) -> Result<()> {
    let bid = bundle_id(profile);
    let script = format!(
        "ObjC.import('CoreServices'); $.LSSetDefaultHandlerForURLScheme('tradingview','{}');",
        bid
    );
    let out = Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &script])
        .output()
        .context("调用 osascript 失败")?;
    if !out.status.success() {
        bail!(
            "设置 tradingview:// 处理程序失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// 删除账号时一并清掉它的副本
pub fn remove_profile_app(profile: &Profile) {
    if let Some(dir) = app_path(profile).parent() {
        let _ = fs::remove_dir_all(dir);
    }
    // 顺手清掉符号链接（只删链接，不动账号目录本身）
    if let Some(base) = dirs::data_dir() {
        let link = base.join(app_name(profile));
        if fs::symlink_metadata(&link)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&link);
        }
    }
}

#[cfg(test)]
mod tests;
