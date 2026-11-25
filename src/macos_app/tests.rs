//! macos_app 模块的回归测试。

use super::*;


fn p(name: &str) -> Profile {
    Profile {
        name: name.to_string(),
        dir: PathBuf::from("/tmp/x"),
    }
}

#[test]
fn slug_is_filesystem_and_bundle_safe() {
    assert!(slug("主号").starts_with('u')); // 全非 ASCII：退化成短哈希
    assert_eq!(slug("Main 2"), "main-2");
    assert_eq!(slug("A/B:C"), "a-b-c");
}

#[test]
fn bundle_ids_are_distinct_and_stable() {
    let a = bundle_id(&p("main"));
    let b = bundle_id(&p("alt"));
    assert_ne!(a, b);
    assert!(a.starts_with("com.tradingview.desktop.tvx."));
    assert_eq!(a, bundle_id(&p("main")));
}

#[test]
fn app_name_matches_bundle_dir() {
    assert_eq!(app_name(&p("main")), "TradingView-main");
    assert!(app_path(&p("main"))
        .to_string_lossy()
        .contains("instances/main/TradingView.app"));
}
