//! 发现系统里装着的 TradingView（注册表 / 应用目录 / 常见便携路径）。

use super::*;

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

