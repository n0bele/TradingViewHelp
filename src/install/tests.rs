//! install 模块的回归测试。

use super::health::ensure_client_logs_dir_at;
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

/// 旧包残留会让补丁"写了却永不执行"，而界面读的是 app/index.js，徽标照样显示完好。
/// 这里验证两件事：能识别残留、能真的删掉（用户不必为一个文件重建整个副本）。
#[test]
fn asar_residue_is_detected_and_cleared() {
    let dir = std::env::temp_dir().join(format!("tvx-residue-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let app = dir.join("app");
    fs::create_dir_all(app.join("resources").join("app")).unwrap();
    #[cfg(target_os = "macos")]
    let app = {
        let a = dir.join("TradingView.app");
        fs::create_dir_all(a.join("Contents").join("Resources")).unwrap();
        a
    };

    let m = Managed {
        dir: dir.clone(),
        app,
        version: "9.9.9".into(),
    };
    assert!(!m.asar_residue(), "还没放旧包，不该报残留");

    let asar = m.resources().join("app.asar");
    fs::write(&asar, b"not really an archive").unwrap();
    assert!(m.asar_residue(), "放了旧包必须报出来");

    assert_eq!(m.clear_asar_residue().unwrap(), true, "应当真的删掉");
    assert!(!asar.exists());
    assert!(!m.asar_residue());
    // 再调一次是幂等的 No-op，不报错
    assert_eq!(m.clear_asar_residue().unwrap(), false);

    let _ = fs::remove_dir_all(&dir);
}

/// 用户上报：构建副本时报
/// `复制 C:\Program Files\WindowsApps\TradingView Desktop_3.5.1.8500.164_x64__… 失败:
///  无法加密指定的文件。(os error 6000)`。
///
/// 6000 = `ERROR_ENCRYPTION_FAILED`：`fs::copy` 在 Windows 上走 `CopyFileExW`，
/// 会把源文件的 EFS「加密」属性一起搬到目标；源文件带加密属性、目标位置又加不了密时，
/// 这一步就必然失败。我们只要内容，属性不要——所以构建和 asar 解包都必须走
/// `copy_file`（失败后退回「流式读写」）。
#[test]
fn copy_never_inherits_encryption_attribute() {
    // 内容一模一样，长度对得上
    let base = std::env::temp_dir().join(format!("tvx-copy-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let src = base.join("a.bin");
    let dst = base.join("b.bin");
    fs::write(&src, b"tradingview").unwrap();
    assert_eq!(copy_file(&src, &dst).unwrap(), 11);
    assert_eq!(fs::read(&dst).unwrap(), b"tradingview");

    // 源文件不存在：返回原始错误，而不是被"兜底"掩盖成别的
    let err = copy_file(&base.join("missing.bin"), &dst).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);

    // 兜底路径本身：跨过 1 MB 缓冲区边界，内容必须逐字节一致
    // （这条路只在原生复制失败时才走，平时不会被构建覆盖到）
    let big = base.join("big.bin");
    let pattern: Vec<u8> = (0..3 * 1024 * 1024 + 7).map(|i| (i % 251) as u8).collect();
    fs::write(&big, &pattern).unwrap();
    let big_dst = base.join("big-out.bin");
    assert_eq!(stream_copy(&big, &big_dst).unwrap(), pattern.len() as u64);
    assert_eq!(fs::read(&big_dst).unwrap(), pattern);

    // 真正的门禁：这两处不能再用 fs::copy（属性继承 → 用户再次撞 6000）
    for (name, text) in [
        ("build.rs", include_str!("build.rs")),
        ("asar.rs", include_str!("../asar.rs")),
    ] {
        assert!(
            !text.contains("fs::copy("),
            "{name} 又用回 fs::copy 了：它会继承 EFS 加密属性，目标加不了密就报 os error 6000"
        );
    }
    let _ = fs::remove_dir_all(&base);
}

/// 商店包名不能写死 `TradingView.Desktop_`：别的 identity（测试通道、重打包、
/// 以后改名）对不上就会被整条跳过，用户只看到"没有副本"。真伪由 looks_valid 把关。
#[test]
fn package_identity_is_matched_loosely() {
    use super::discover::is_tradingview_package;

    assert!(is_tradingview_package(
        "TradingView.Desktop_3.5.1.8500_x64__n534cwy3pjxzj"
    ));
    assert!(is_tradingview_package(
        "TradingView.TradingViewDesktop_3.5.2.0_x64__abc123"
    ));
    assert!(is_tradingview_package("tradingview.desktop_3.5.1.8500_x64__abc"));
    assert!(!is_tradingview_package(
        "Microsoft.WindowsTerminal_1.21_x64__8wekyb3d8bbwe"
    ));
    assert!(!is_tradingview_package("NotTradingView.Foo_1.0_x64__abc"));
}

/// 官方 `.exe` 安装器允许自选目录，Squirrel 式布局里本体在 `app-<版本>\`：
/// 只认"目录即根"会直接把这类安装判成不存在。
#[test]
fn portable_root_resolves_squirrel_app_dir() {
    use super::discover::resolve_portable_root;

    let base = std::env::temp_dir().join(format!("tvx-disc-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);

    // 形态一：目录本身就是应用根
    let direct = base.join("direct");
    fs::create_dir_all(direct.join("resources")).unwrap();
    fs::write(direct.join("TradingView.exe"), b"x").unwrap();
    fs::write(direct.join("resources").join("app.asar"), b"x").unwrap();
    assert_eq!(resolve_portable_root(&direct).unwrap().1, "portable");

    // 形态二：Squirrel，取版本号最高的 app-*（按数字比，不是字符串）
    for v in ["3.9.7.1000", "3.10.1.2000"] {
        let d = base.join("sq").join(format!("app-{v}"));
        fs::create_dir_all(d.join("resources")).unwrap();
        fs::write(d.join("TradingView.exe"), b"x").unwrap();
        fs::write(d.join("resources").join("app.asar"), b"x").unwrap();
    }
    let (root, ver) = resolve_portable_root(&base.join("sq")).unwrap();
    assert_eq!(ver, "3.10.1.2000", "应当按数字段比较版本");
    assert!(root.ends_with("app-3.10.1.2000"));

    // 形态三：不像 TradingView 的目录
    fs::create_dir_all(base.join("empty")).unwrap();
    assert!(resolve_portable_root(&base.join("empty")).is_none());

    let _ = fs::remove_dir_all(&base);
}

/// 便携安装 / 用户手选的目录没有版本线索，唯一可靠的版本来源是 exe 的版本资源。
/// 没有它，这类目录的版本是占位值 `[0]`，选源时会输给更旧的带版本目录。
#[test]
fn exe_version_is_read_from_file_version_resource() {
    // 真实副本：目录名就是版本号，两者必须对得上（没有副本就跳过，别在 CI 上假红）
    match load_managed() {
        Some(m) => {
            let inst = Install {
                root: m.dir.clone(),
                kind: InstallKind::Portable,
                version: "portable".into(),
            };
            let v = inst.exe_version().expect("副本的 TradingView.exe 应当带版本资源");
            assert_eq!(v, m.version, "exe 版本资源与副本目录名必须一致");
            assert!(patch::is_verified(&v) || !patch::is_verified(&m.version));
        }
        None => println!("跳过真实副本比对：本机没有托管副本"),
    }

    // 不是 PE 的文件必须返回 None，不能编出 0.0.0.0 去参与版本比较
    let base = std::env::temp_dir().join(format!("tvx-ver-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let fake = base.join("fake.exe");
    fs::write(&fake, b"not a pe").unwrap();
    let inst = Install {
        root: base.clone(),
        kind: InstallKind::Custom,
        version: "custom".into(),
    };
    assert_eq!(inst.exe_version(), None);
    let _ = fs::remove_dir_all(&base);
}

/// 选源的最后一关：同一目录被多条线索认领时不能出现两条，版本必须按数字排。
/// 反安装登记里 `InstallLocation` 与 `DisplayIcon` 会各给一次、32/64 位视图再各给一次，
/// 路径的大小写和尾随反斜杠还不一样 —— 不规范化就会列出两条"看起来不同"的同一个源。
#[test]
fn discover_dedupes_one_entry_per_root_and_sorts_numerically() {
    use super::discover::{dedupe_roots, sort_installs};

    let mk = |root: &str, ver: &str, kind: InstallKind| Install {
        root: PathBuf::from(root),
        kind,
        version: ver.into(),
    };
    let mut list = vec![
        // 同一个目录，一条来自候选列表（真版本），一条来自登记（尾随反斜杠 + 小写 + 占位版本）
        mk(
            r"C:\Users\t\AppData\Local\Programs\TradingView",
            "3.5.1.8500",
            InstallKind::Portable,
        ),
        mk(
            r"c:\users\t\appdata\local\programs\tradingview\",
            "portable",
            InstallKind::Portable,
        ),
        mk(r"D:\TV\app-3.10.1.2000", "3.10.1.2000", InstallKind::Portable),
        mk(r"D:\TV\app-3.9.7.1000", "3.9.7.1000", InstallKind::Portable),
    ];
    sort_installs(&mut list);
    let out = dedupe_roots(list);

    assert_eq!(out.len(), 3, "同一个目录只该留一条，实际：{out:#?}");
    let versions: Vec<&str> = out.iter().map(|i| i.version.as_str()).collect();
    assert_eq!(
        versions,
        vec!["3.5.1.8500", "3.9.7.1000", "3.10.1.2000"],
        "必须按数字段排序（3.10 在 3.9/3.5 之后），不能按字符串"
    );
    assert!(
        out[0].root.ends_with("TradingView"),
        "去重后要保留带真版本的那条"
    );
}

/// 同一目录既是"目录里翻出来的"又是"用户显式指定的"时，用户指定的那条说话
/// （否则界面会把用户手选的源显示成 `本地安装 <路径>`，看起来像没生效）。
#[test]
fn dedupe_prefers_the_source_user_picked() {
    use super::discover::dedupe_roots;

    let root = PathBuf::from(r"D:\TV");
    let out = dedupe_roots(vec![
        Install {
            root: root.clone(),
            kind: InstallKind::Portable,
            version: "3.5.1.8500".into(),
        },
        Install {
            root,
            kind: InstallKind::Custom,
            version: "3.5.1.8500".into(),
        },
    ]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, InstallKind::Custom);
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

/// 6000 的**可控复现**：EFS 加密的源文件 → 加不了密的目标。
///
/// 默认不跑，因为需要两个本机前提：能加密的源卷、一个加不了密的目标
/// （WSL 的 9p 重定向器 `\\wsl.localhost\...`、SMB 共享、exFAT 盘都算）。
///
/// ```text
/// set TVX_NON_EFS_DEST=\\wsl.localhost\docker-desktop\tmp
/// cargo test --release efs_source_to_non_efs -- --ignored --nocapture
/// ```
///
/// 2026-11-27 实测（1 MiB EFS 源，目标 `\\wsl.localhost\docker-desktop\tmp`）：
/// - `fs::copy` → `raw_os_error() == Some(6000)`，就是用户报的「无法加密指定的文件」；
/// - `copy_file` → `Ok(1048576)`，读回来逐字节一致。
///
/// 这条用例有意**先要求旧写法失败**：只有真复现出 6000，后面的"新写法成功"才有意义，
/// 否则说明源文件没被加密，测试直接说明并跳过，不会给一个假绿。
#[test]
#[ignore = "需要 EFS 环境 + 一个加不了密的目标；用 TVX_NON_EFS_DEST 指定"]
fn efs_source_to_non_efs_destination_needs_the_flag() {
    let Ok(dest_root) = std::env::var("TVX_NON_EFS_DEST") else {
        println!("跳过：未设置 TVX_NON_EFS_DEST（例如 \\\\wsl.localhost\\docker-desktop\\tmp）");
        return;
    };
    let dest_root = PathBuf::from(dest_root);
    let base = std::env::temp_dir().join(format!("tvx-efs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let src = base.join("encrypted.bin");
    let bytes: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();
    fs::write(&src, &bytes).unwrap();

    // 加不上密就直接说清楚并跳过，别把"环境不满足"报成"修复失效"
    let out = std::process::Command::new("cipher")
        .arg("/e")
        .arg("/a")
        .arg(&src)
        .output()
        .expect("cipher 启动失败");
    let cipher_says = String::from_utf8_lossy(&out.stdout).replace(['\r', '\n'], " ");

    let legacy = dest_root.join(format!("tvx-legacy-{}.bin", std::process::id()));
    let ours = dest_root.join(format!("tvx-ours-{}.bin", std::process::id()));
    let _ = fs::remove_file(&legacy);
    let _ = fs::remove_file(&ours);

    // 先要求**旧写法真的失败**：只有复现出 6000，"新写法成功"才证明得了什么
    let legacy_err = fs::copy(&src, &legacy).err().and_then(|e| e.raw_os_error());
    if legacy_err != Some(6000) {
        println!(
            "跳过：没复现出 6000（旧写法错误={legacy_err:?}，cipher 输出={})",
            cipher_says.trim()
        );
        let _ = fs::remove_dir_all(&base);
        return;
    }

    // 同一个源、同一个目标：换我们的复制必须成功且内容一致
    let n = copy_file(&src, &ours).expect("带 0x8 的复制不该再被加密属性挡住");
    assert_eq!(n, bytes.len() as u64);
    assert_eq!(fs::read(&ours).unwrap(), bytes, "内容必须逐字节一致");

    let _ = fs::remove_file(&legacy);
    let _ = fs::remove_file(&ours);
    let _ = fs::remove_dir_all(&base);
}
