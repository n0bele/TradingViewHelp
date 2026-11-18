//! 深链回调的目标路由：让 `tradingview://` 授权回调落回"正在登录的那个账号"。
//!
//! 背景：TradingView 的登录是「浏览器授权 → 系统用协议拉起 TradingView.exe <url>」。
//! 系统注册的命令里如果没有 `--user-data-dir`，回调进程会用默认配置目录，
//! 授权码就落到默认 profile，账号窗口收不到 —— 表现是"第二个账号登不进去"。
//!
//! 各平台能力不一样，这里如实分开实现：
//! - **Windows**：改 `HKCU\Software\Classes\tradingview\shell\open\command`，
//!   可以带任意参数（`--user-data-dir`），能精确路由到账号。✅
//! - **Linux**：写 `~/.local/share/applications/` 下的 .desktop，`Exec` 里带上
//!   `--user-data-dir`，并用 `xdg-mime default` 指过去。✅
//! - **macOS**：URL scheme 绑在 App bundle 的 Info.plist 上，且系统对同一 scheme
//!   只有一个默认处理程序，不能用命令行参数区分账号。这里走**路由 App**：
//!   生成一个极小的 `TVXRouter.app` 注册 `tradingview` scheme，它只做一件事——
//!   把收到的 URL 交给 `tvx _deeplink`，由 tvx 按"当前登录目标账号"转发给对应实例。

use std::path::Path;

// ---------------------------------------------------------------- Windows

/// 协议命令统一指向 tvx 自己：系统把 `tradingview://` 交给 `tvx _auth <url>`，
/// 再由 tvx 广播给所有在跑的实例——哪个实例在等登录，哪个就接住。
#[cfg(windows)]
pub fn set_target(_tv_exe: &Path, _profile_dir: Option<&Path>) -> anyhow::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let cmd = command_line();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(r"Software\Classes\tradingview\shell\open\command")?;
    key.set_value("", &cmd)?;
    Ok(())
}

/// `"<tvx.exe>" _auth "%1"`
#[cfg(windows)]
fn command_line() -> String {
    let me = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("tvx.exe"));
    format!("\"{}\" _auth \"%1\"", me.display())
}

// ---------------------------------------------------------------- Linux

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn set_target(_tv_exe: &Path, _profile_dir: Option<&Path>) -> anyhow::Result<()> {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    // ~/.local/share/applications/tvhelper-tradingview.desktop
    let Some(data) = dirs::data_dir() else {
        return Ok(());
    };
    let apps = data.join("applications");
    fs::create_dir_all(&apps)?;
    let entry: PathBuf = apps.join("tvhelper-tradingview.desktop");

    // 图标：写到 ~/.local/share/icons/tvhelper.png，desktop 用绝对路径引用
    let icons = data.join("icons");
    let _ = fs::create_dir_all(&icons);
    let icon_path = icons.join("tvhelper.png");
    let _ = fs::write(&icon_path, include_bytes!("../assets/icon.png"));

    let me = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("tvhelper"));
    let exec = format!("\"{}\" _auth %u", me.display());
    let body = format!(
        "[Desktop Entry]\nType=Application\nName=TradingView Helper\nExec={exec}\nIcon={}\nNoDisplay=true\nMimeType=x-scheme-handler/tradingview;\nTerminal=false\n",
        icon_path.display()
    );
    fs::write(&entry, body)?;

    // 让 xdg 默认用这个 entry 处理 tradingview://
    let _ = Command::new("xdg-mime")
        .args(["default", "tvhelper-tradingview.desktop", "x-scheme-handler/tradingview"])
        .status();
    Ok(())
}

// ---------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
pub fn set_target(_exe: &Path, _profile_dir: Option<&Path>) -> anyhow::Result<()> {
    // macOS 的 URL scheme 绑在 App bundle 的 Info.plist 上，LaunchServices 对同一个
    // scheme 只认一个默认处理程序，且 GUI App 收到的是 Apple Event 而不是 argv。
    // 所以这里保持 no-op：按账号路由由 macos_app::ensure_profile_app 完成——
    // 每个账号一份改过 CFBundleIdentifier 的 .app 副本，登录前把默认 handler 指过去。
    // 不要在这里全局改 handler，那会把其他账号正在等的回调抢走。
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    fn windows_roundtrip() {
        use super::*;
        use std::path::Path;
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let path = r"Software\Classes\tradingview\shell\open\command";
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let original: Option<String> = hkcu
            .open_subkey(path)
            .ok()
            .and_then(|k| k.get_value("").ok());

        set_target(Path::new("tvx.exe"), None).expect("写入失败");
        let with: String = hkcu.open_subkey(path).unwrap().get_value("").unwrap();
        assert!(with.contains("_auth"), "应指向 tvx _auth：{with}");
        assert!(with.ends_with("\"%1\""), "应带 %1 占位：{with}");

        if let Some(o) = original {
            let (k, _) = hkcu.create_subkey(path).unwrap();
            k.set_value("", &o).unwrap();
        }
    }

    #[test]
    #[cfg(windows)]
    fn windows_command_line_shape() {
        use super::*;
        let cmd = command_line();
        assert!(cmd.starts_with('"'), "路径要加引号：{cmd}");
        assert!(cmd.contains("_auth"), "必须走 tvx _auth：{cmd}");
        assert!(cmd.ends_with("\"%1\""), "必须以 %1 结尾：{cmd}");
    }
}
