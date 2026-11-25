//! 界面侧纯逻辑的回归测试（去重表、构建门禁、来源命名）。

use super::authwatch::*;
use super::*;
use std::collections::BTreeMap;



// ---------------------------------------------------------------- 版本门禁
// 这组用例覆盖"白名单 + 显式确认"的实际判定，不依赖 UI 渲染。

/// 去重表落盘判定：命中已存在的档案时 `len` 不变，但值确实变了，必须写回。
/// 漏掉这一步 = 每轮都从磁盘读回旧 URL → 反复拉起客户端（"一直在重启"）。
#[test]
fn seen_map_flushes_when_an_existing_entry_changes() {
    // 新档案：长度变了
    assert!(seen_needs_flush(true, 3, 4));
    // 覆盖已有档案：长度没变，但 pick 返回了 Some → 必须落盘
    assert!(seen_needs_flush(true, 4, 4));
    // 什么都没发现 → 不写盘
    assert!(!seen_needs_flush(false, 4, 4));
}

/// 自动打开登录页的两道闸。缺了它们就是"客户端不停重启"：
/// 打开一次会生成新的 `key=`，按 URL 去重的旧逻辑每次都判定成新 URL → 再拉进程。
#[test]
fn auth_page_is_not_reopened_in_a_loop() {
    let now = 1_000_000;
    // 从没开过、也没在跑 → 允许
    assert!(auth_open_allowed(None, now, false));
    // 实例在跑 → 一律不再从外面拉进程
    assert!(!auth_open_allowed(None, now, true));
    assert!(!auth_open_allowed(Some(now - 1), now, true));
    // 刚开过（冷却期内）→ 不重复开
    assert!(!auth_open_allowed(Some(now - 1), now, false));
    assert!(!auth_open_allowed(Some(now - AUTH_OPEN_COOLDOWN_SECS + 1), now, false));
    // 过了冷却期 → 允许重试
    assert!(auth_open_allowed(Some(now - AUTH_OPEN_COOLDOWN_SECS), now, false));
}

#[test]
fn verified_source_needs_no_confirmation() {
    // 没副本、选中的是白名单版本 → 不弹确认，直接能构建
    let t = unverified_target(false, Some(patch::VERIFIED_VERSION), None);
    assert_eq!(t, None);
    assert!(build_allowed(t.as_deref(), false));
}

#[test]
fn unverified_source_needs_confirmation() {
    let v = "3.6.0.9000";
    let t = unverified_target(false, Some(v), None);
    assert_eq!(t.as_deref(), Some(v));
    assert!(!build_allowed(t.as_deref(), false), "没勾确认不能构建");
    assert!(build_allowed(t.as_deref(), true), "勾了确认才放行");
    // 非版本号的来源（portable / custom）同样要确认
    assert_eq!(
        unverified_target(false, Some("custom"), None).as_deref(),
        Some("custom")
    );
}

/// 回归：勾了确认之后，警告条不能消失。
/// `unverified_target` 只回答"源有没有验证过"，与用户是否勾选无关——
/// 早先把两者混在一起，勾选后整条提示消失，用户既看不到状态也无法反悔。
#[test]
fn warning_stays_after_confirm() {
    let v = "3.6.0.9000";
    assert_eq!(
        unverified_target(false, Some(v), None).as_deref(),
        Some(v),
        "确认与否不该影响提示条的显示条件"
    );
}

#[test]
fn selection_ignored_once_copy_exists() {
    // 有副本时，选中的安装不参与构建 → 选到未验证版本也不该弹确认
    assert_eq!(unverified_target(true, Some("3.6.0.9000"), None), None);
}

#[test]
fn rebuild_target_decides_not_selection() {
    // 有副本 + 发现未验证新版：提示并拦
    let t = unverified_target(true, Some(patch::VERIFIED_VERSION), Some(("3.6.0.9000", false)));
    assert_eq!(t.as_deref(), Some("3.6.0.9000"));
    assert!(!build_allowed(t.as_deref(), false));

    // 有副本 + 新版本身在白名单：不提示（白名单扩张后要能用）
    assert_eq!(
        unverified_target(true, None, Some((patch::VERIFIED_VERSION, true))),
        None
    );
    // 有副本 + 没有新版可重建：不提示
    assert_eq!(unverified_target(true, None, None), None);
}

#[test]
fn no_install_falls_through_to_later_error() {
    // 一个安装都没发现：门禁不该拦，交给后面的 log_no_source 报错
    let t = unverified_target(false, None, None);
    assert_eq!(t, None);
    assert!(build_allowed(t.as_deref(), false));
}

/// 回归：两个账号各有一条登录 URL 时，**不能每轮都判定"发现新 URL"**。
/// 旧实现只维护一个全局 last，两个账号会互相顶掉 → 每 2 秒触发一次，
/// 反复打开登录页并叠加接管线程（用户看到命令行窗口一直在闪）。
#[test]
fn two_accounts_do_not_retrigger_every_scan() {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let a = "https://auth.tradingview.com/connect/?key=AAA".to_string();
    let b = "https://auth.tradingview.com/connect/?key=BBB".to_string();

    let items = || {
        vec![
            ("colonychain".to_string(), a.clone()),
            ("dog000".to_string(), b.clone()),
        ]
        .into_iter()
    };

    // 第一轮：确实有新 URL，触发一次
    assert!(pick_unseen_url(&mut seen, items()).is_some());
    assert_eq!(seen.len(), 2, "两个档案都要被记住，否则会互相顶掉");

    // 第二轮、第三轮：同样的 URL，不能再触发
    assert_eq!(pick_unseen_url(&mut seen, items()), None);
    assert_eq!(pick_unseen_url(&mut seen, items()), None);

    // 某个账号重新登录（URL 变化）→ 只触发那一条
    let c = "https://auth.tradingview.com/connect/?key=CCC".to_string();
    let got = pick_unseen_url(
        &mut seen,
        vec![
            ("colonychain".to_string(), a.clone()),
            ("dog000".to_string(), c.clone()),
        ]
        .into_iter(),
    );
    assert_eq!(got, Some(("dog000".to_string(), c)));
    // 再变回旧 URL 仍算"这个档案又变了一次"（只看自己的上一条，日志只追加，
    // 真实场景不会来回跳）——重点是 colonychain 那条不会被顺带重触发。
    let back = pick_unseen_url(&mut seen, items());
    assert_eq!(back.as_ref().map(|(n, _)| n.as_str()), Some("dog000"));
}
