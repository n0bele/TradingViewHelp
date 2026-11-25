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
