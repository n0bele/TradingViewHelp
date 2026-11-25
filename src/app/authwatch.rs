//! 登录页自动打开：从各账号日志里发现新的 `TVX_AUTH_URL`，只对"没处理过"的触发一次。
//!
//! 这里的每一道闸（按档案记录、冷却期、实例在跑就不开）都是踩过的死循环：
//! 登录页每加载一次就换一个 `key=`，按 URL 去重挡不住，会无限拉起客户端。

use crate::install;
use crate::profiles;
use eframe::egui;
use std::collections::BTreeMap;

/// 已处理过的登录页 URL，**按档案各记一条**（`~/.tvx/auth-seen.json`）。
///
/// 不能用"全局只存一个 URL"来去重：多个账号的登录 URL 里 `key=` 必然不同，
/// 单值基准会被另一个账号立刻顶掉 → 每轮扫描都判定"发现新 URL" → 反复打开
/// 登录页并叠加后台接管进程。这是真实踩过的死循环（一直闪命令行窗口）。
pub(crate) fn auth_seen_path() -> std::path::PathBuf {
    install::tvx_root().join("auth-seen.json")
}

/// 从各档案的最新登录 URL 里挑出"这个档案还没处理过"的一条，并把它记进 `seen`。
///
/// 关键是**按档案分别记**：多个账号的 URL 里 `key=` 不同，如果只维护一个全局值，
/// 两个账号会互相把对方顶成"新 URL"，于是每轮扫描都触发一次打开登录页 +
/// 启动接管线程——就是那个"命令行窗口一直闪"的死循环。
pub(crate) fn pick_unseen_url(
    seen: &mut std::collections::BTreeMap<String, String>,
    items: impl Iterator<Item = (String, String)>,
) -> Option<(String, String)> {
    let mut newest = None;
    for (name, url) in items {
        if seen.get(&name).map(|s| s.as_str()) == Some(url.as_str()) {
            continue;
        }
        seen.insert(name.clone(), url.clone());
        newest = Some((name, url));
    }
    newest
}

/// 自动打开登录页的冷却表（档案名 → unix 秒），`~/.tvx/auth-open.json`。
///
/// 为什么需要：登录页每加载一次都会生成新的 `key=`，客户端把它写进 `tvx-launch.log`
/// 的 `TVX_AUTH_URL`。只按"URL 变没变"去重的话，打开一次 → URL 变了 → 判定成新 URL
/// → 再拉起一个进程 → 又生成新的 key…… 这就是"客户端一直在不停重启"。
fn auth_open_path() -> std::path::PathBuf {
    install::tvx_root().join("auth-open.json")
}

/// 同一档案两次自动打开登录页的最小间隔（秒）。
pub(crate) const AUTH_OPEN_COOLDOWN_SECS: u64 = 180;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 去重表是否需要写回磁盘。
///
/// `pick_unseen_url` 命中"已记录的档案"时是覆盖旧值，`len` 不变——只按长度判断
/// 就会漏掉这种更新，下一轮又从磁盘读回旧值，判成新 URL，反复拉起客户端。
pub(crate) fn seen_needs_flush(picked_any: bool, len_before: usize, len_after: usize) -> bool {
    picked_any || len_before != len_after
}

/// 是否允许为这个档案自动打开登录页。
///
/// - 实例已经在跑：不允许（登录页由实例自己开，外部再拉进程会让 URL 被反复投递）
/// - 距离上次自动打开不足冷却期：不允许（`key=` 每次都会变，按 URL 去重挡不住）
pub(crate) fn auth_open_allowed(last_opened: Option<u64>, now: u64, running: bool) -> bool {
    if running {
        return false;
    }
    match last_opened {
        Some(t) => now.saturating_sub(t) >= AUTH_OPEN_COOLDOWN_SECS,
        None => true,
    }
}

/// 监听各账号日志里最新的 `TVX_AUTH_URL`，用**无痕窗口**打开（避开旧账号会话）。
/// 每个档案只对"自己没处理过"的 URL 触发一次。
pub(crate) fn open_auth_urls_if_new(ctx: Option<&egui::Context>) -> Option<String> {
    
    let seen_file = auth_seen_path();
    let mut seen: BTreeMap<String, String> = std::fs::read_to_string(&seen_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let before = seen.len();
    let newest = pick_unseen_url(
        &mut seen,
        profiles::list().into_iter().filter_map(|p| {
            let text = std::fs::read_to_string(p.dir.join("tvx-launch.log")).ok()?;
            let line = text.lines().rev().find(|l| l.contains("TVX_AUTH_URL "))?;
            let url = line.split("TVX_AUTH_URL ").nth(1)?.trim().to_string();
            url.starts_with("http").then_some((p.name, url))
        }),
    );
    // 只看长度是错的：`pick_unseen_url` 对**已存在的档案**是覆盖旧值，长度不变。
    // 于是"发现新 URL → 打开登录页 → 登录页生成新的 key= → 内存里更新了但没落盘
    // → 下一轮又从磁盘读回旧值 → 又判定新 URL"，客户端就会被无限拉起，
    // 表现出来就是"一直在重启"。
    if seen_needs_flush(newest.is_some(), before, seen.len()) {
        let _ = std::fs::create_dir_all(install::tvx_root());
        if let Ok(s) = serde_json::to_string_pretty(&seen) {
            let _ = std::fs::write(&seen_file, s);
        }
    }

    let (pname, url) = newest?;
    let url = url.as_str();

    // 闸一：档案已经在跑。登录页由那个实例自己开，从外面再拉一个进程只会让
    // 单实例锁把 URL 反复投递给同一个实例。
    let target = profiles::list().into_iter().find(|p| p.name == pname);
    let already_running = target
        .as_ref()
        .map(|p| profiles::running_pid(p).is_some())
        .unwrap_or(false);

    // 闸二：冷却期内不重复打开。key= 每次都会变，没有这道闸就是死循环。
    let mut opened: BTreeMap<String, u64> = std::fs::read_to_string(auth_open_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let now = unix_now();
    if !auth_open_allowed(opened.get(&pname).copied(), now, already_running) {
        return None;
    }
    opened.insert(pname.clone(), now);
    let _ = std::fs::create_dir_all(install::tvx_root());
    if let Ok(s) = serde_json::to_string(&opened) {
        let _ = std::fs::write(auth_open_path(), s);
    }

    // 先试"应用内标签页"：登录页在应用里渲染，cookie 直接落进该账号目录，
    // 不需要浏览器、也不需要 tradingview:// 回调（浏览器常把回调拦掉）。
    let opened_in_app = target
        .as_ref()
        .map(|p| profiles::open_in_app_tab(p, url).is_ok())
        .unwrap_or(false);

    // 同时盯住被商店版抢走的回调（token 卡在那个进程的命令行里）
    if let Some(c) = ctx {
        profiles::spawn_handoff_watcher(c.clone());
    }
    let lang = crate::install::load_config().lang;
    Some(if opened_in_app {
        lang.log_login_in_app()
    } else if crate::browser::open_private(url) {
        lang.log_login_private()
    } else {
        lang.log_login_default_browser()
    })
}

