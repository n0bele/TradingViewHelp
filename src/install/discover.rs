//! 发现系统里装着的 TradingView（注册表 / 应用目录 / 常见便携路径）。

use super::*;
use std::collections::HashMap;

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

    sort_installs(&mut out);
    dedupe_roots(out)
}

/// 排序：按版本号**数字**比较。
///
/// 字符串比较会把 `3.10.1.2000` 排在 `3.5.1.8500` 前面（`'1' < '5'`），列表顺序看着就像
/// 选错了源。占位版本（`portable`/`custom`）是 `[0]`，自然排在最前，不会被误认为"最新"。
pub(crate) fn sort_installs(out: &mut [Install]) {
    out.sort_by(|a, b| {
        ver_key(&a.version)
            .cmp(&ver_key(&b.version))
            .then_with(|| a.version.cmp(&b.version))
            .then_with(|| a.root.cmp(&b.root))
    });
}

/// 同一个目录可能被好几条线索分别认领：反安装登记里 `InstallLocation` 与 `DisplayIcon`
/// 各给一次、HKLM 的 32/64 位视图各给一次、候选列表与登记里的路径大小写/尾随分隔符也不一样。
/// 不去重的话界面会列出两条一模一样的源，用户根本分不清该选哪条。
///
/// 按**规范化路径**去重（去尾随分隔符、Windows 下忽略大小写）；同一条保留版本号更明确的
/// 那份（`portable` 让位给 `3.5.1.8500`），都明确时保留先出现的。
pub(crate) fn dedupe_roots(list: Vec<Install>) -> Vec<Install> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<Install> = Vec::with_capacity(list.len());
    for inst in list {
        let key = root_key(&inst.root);
        match seen.get(&key).copied() {
            Some(i) => {
                let better = match (version_known(&inst.version), version_known(&out[i].version)) {
                    (true, false) => true,
                    (false, true) => false,
                    // 版本都明确（或都不明确）时，用户显式指定的那条说话
                    _ => kind_rank(inst.kind) > kind_rank(out[i].kind),
                };
                if better {
                    out[i] = inst;
                }
            }
            None => {
                seen.insert(key, out.len());
                out.push(inst);
            }
        }
    }
    out
}

/// 版本号是不是"真的"（不是 `portable` / `custom` / `app` 这类占位值）
fn version_known(v: &str) -> bool {
    ver_key(v).iter().any(|&n| n > 0)
}

/// 同一目录被多条线索认领时的取舍顺序：用户显式指定 > 商店 > 目录里翻出来的
fn kind_rank(k: InstallKind) -> u8 {
    match k {
        InstallKind::Custom => 3,
        InstallKind::Store => 2,
        InstallKind::Portable => 1,
        InstallKind::App => 1,
    }
}

/// 去重用的路径键：`C:\TV\` 与 `c:\tv` 视为同一个目录
fn root_key(p: &Path) -> String {
    let s = p.to_string_lossy();
    let trimmed = s.trim_end_matches(['\\', '/']);
    #[cfg(windows)]
    {
        trimmed.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        trimmed.to_string()
    }
}

#[cfg(windows)]
fn discover_windows(out: &mut Vec<Install>) {
    use winreg::enums::*;
    use winreg::RegKey;

    // MSIX 的安装位置只能从注册表拿：WindowsApps 根目录不允许枚举
    let key = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    if let Ok(pkgs) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(key, KEY_READ) {
        for name in pkgs.enum_keys().flatten() {
            if !is_tradingview_package(&name) {
                continue;
            }
            let Ok(sub) = pkgs.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            let Ok(root) = sub.get_value::<String, _>("PackageRootFolder") else {
                continue;
            };
            let name_version = name.split('_').nth(1).unwrap_or("unknown").to_string();
            let inst = Install {
                kind: InstallKind::Store,
                version: name_version.clone(),
                root: PathBuf::from(root),
            };
            if !inst.looks_valid() {
                continue;
            }
            // 包名里的版本是备选：极少数重打包的包名不带版本段，exe 里一定有
            let version = inst.exe_version().unwrap_or(name_version);
            out.push(Install { version, ..inst });
        }
    }

    let mut candidates = Vec::new();
    if let Some(d) = dirs::data_local_dir() {
        candidates.push(d.join("Programs").join("TradingView"));
        candidates.push(d.join("Programs").join("TradingView Desktop"));
        candidates.push(d.join("TradingView"));
    }
    if let Some(d) = dirs::data_dir() {
        candidates.push(d.join("TradingView"));
    }
    // 官方安装器允许用户自选目录，这时只能靠这几个默认位置 + 反安装登记兜底
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(d) = std::env::var_os(var) {
            let base = PathBuf::from(d);
            candidates.push(base.join("TradingView"));
            candidates.push(base.join("TradingView Desktop"));
        }
    }
    candidates.extend(registry_install_locations());

    for c in candidates {
        if let Some((root, version)) = resolve_portable_root(&c) {
            out.push(Install {
                kind: InstallKind::Portable,
                version,
                root,
            });
        }
    }
}

/// 包名是不是 TradingView（大小写不敏感，只认 `_` 前的 identity 段）。
///
/// 不能写死 `TradingView.Desktop_`：商店里出现过不同 identity / 不同发布者哈希
/// 的包（重打包、测试通道），名字对不上就把整个安装漏掉，用户只能看到"没有副本"。
/// 真正的把关交给 `looks_valid()`（exe + resources\app.asar），不会误收。
pub(crate) fn is_tradingview_package(name: &str) -> bool {
    let ident = name.split('_').next().unwrap_or("").trim();
    ident.to_ascii_lowercase().starts_with("tradingview")
}

/// 从一个候选目录解析出真正的应用根。
///
/// 两种形态都要认：
/// - 目录本身就是根（`TradingView.exe` + `resources\app.asar`）；
/// - Squirrel 式安装：`<目录>\app-3.5.1.8500\` 才是本体，取版本最高的那个。
pub(crate) fn resolve_portable_root(dir: &Path) -> Option<(PathBuf, String)> {
    let direct = Install {
        root: dir.to_path_buf(),
        kind: InstallKind::Portable,
        version: "portable".into(),
    };
    if direct.looks_valid() {
        // 目录本身是应用根时没有任何版本线索，只能问 exe
        let version = direct.exe_version().unwrap_or_else(|| "portable".into());
        return Some((dir.to_path_buf(), version));
    }

    let mut best: Option<(PathBuf, String)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(ver) = name.strip_prefix("app-") else {
            continue;
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let inst = Install {
            root: path.clone(),
            kind: InstallKind::Portable,
            version: ver.to_string(),
        };
        if !inst.looks_valid() {
            continue;
        }
        // 目录名 `app-<ver>` 已经带版本，exe 版本资源一致时无差别；
        // 被改名/搬过的目录只有 exe 靠得住，所以以它为准
        let version = inst.exe_version().unwrap_or_else(|| ver.to_string());
        let newer = best
            .as_ref()
            .map(|(_, v)| ver_key(&version) > ver_key(v))
            .unwrap_or(true);
        if newer {
            best = Some((path, version));
        }
    }
    best
}

/// 反安装登记里的安装位置（HKCU/HKLM，32/64 位视图都看）。
///
/// 官方 `.exe` 安装器允许把程序装到任意目录，硬编码候选目录必然漏；这里读系统
/// 自己记着的 `InstallLocation` / `DisplayIcon`（`"D:\TV\TradingView.exe",0`）。
/// 读不到就返回空，不影响其它来源。
#[cfg(windows)]
fn registry_install_locations() -> Vec<PathBuf> {
    use winreg::enums::*;
    use winreg::RegKey;

    let mut out = Vec::new();
    const SUB: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    for predef in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_READ | KEY_WOW64_64KEY, KEY_READ | KEY_WOW64_32KEY] {
            let Ok(root) = RegKey::predef(predef).open_subkey_with_flags(SUB, view) else {
                continue;
            };
            for key in root.enum_keys().flatten() {
                let Ok(sub) = root.open_subkey_with_flags(&key, KEY_READ) else {
                    continue;
                };
                let name = sub
                    .get_value::<String, _>("DisplayName")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !name.contains("tradingview") {
                    continue;
                }
                if let Ok(loc) = sub.get_value::<String, _>("InstallLocation") {
                    if !loc.trim().is_empty() {
                        out.push(PathBuf::from(loc.trim().trim_matches('"')));
                    }
                }
                if let Ok(icon) = sub.get_value::<String, _>("DisplayIcon") {
                    // "C:\path\TradingView.exe,0" → 取父目录
                    let p = icon.split(',').next().unwrap_or("").trim().trim_matches('"');
                    if let Some(parent) = Path::new(p).parent() {
                        if !p.is_empty() {
                            out.push(parent.to_path_buf());
                        }
                    }
                }
            }
        }
    }
    out
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

