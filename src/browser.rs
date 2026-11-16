//! 打开"干净的"登录窗口：
//!
//! TradingView 的登录只能走浏览器授权，而浏览器里往往已经登录了别的账号——
//! 用户不希望为了切账号先手动退出。所以这里把授权地址开在**无痕窗口**里：
//! 无痕会话不带任何已有 cookie，等于一块干净的白板。
//!
//! 平台差异：Windows 优先 Edge（`--inprivate`）再试 Chrome（`--incognito`）；
//! macOS 用 `open -na` 指定浏览器；Linux 用 `--incognito`。
//! 找不到任何浏览器时退回系统默认打开（用户仍可手动切账号）。

#[cfg(windows)]
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// 打开登录页；返回 true 表示成功用上了无痕模式。
pub fn open_private(url: &str) -> bool {
    #[cfg(windows)]
    {
        for (exe, flag) in [
            (r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", "--inprivate"),
            (r"C:\Program Files\Microsoft\Edge\Application\msedge.exe", "--inprivate"),
            (r"C:\Program Files\Google\Chrome\Application\chrome.exe", "--incognito"),
            (r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe", "--incognito"),
        ] {
            if PathBuf::from(exe).is_file() {
                let ok = Command::new(exe)
                    .arg(flag)
                    .arg(url)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .is_ok();
                if ok {
                    return true;
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        for app in ["Google Chrome", "Microsoft Edge", "Brave Browser"] {
            let ok = Command::new("open")
                .args(["-na", app, "--args", "--incognito", url])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok();
            if ok {
                return true;
            }
        }
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        for exe in ["google-chrome", "chromium", "microsoft-edge"] {
            let ok = Command::new(exe)
                .arg("--incognito")
                .arg(url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok();
            if ok {
                return true;
            }
        }
    }

    // 兜底：系统默认浏览器
    #[cfg(windows)]
    {
        let _ = Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(url).spawn();
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
    false
}
