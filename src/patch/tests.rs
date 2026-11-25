//! 补丁引擎的回归测试：锚点自洽、旧副本按段迁移、开关可逆、失配降级。
//!
//! 这些用例直接对着 `super::` 的私有函数跑，故意不经过公开 API——
//! 出问题时它们指向的是"哪一段没对上"，而不是"整体失败"。

use super::*;

#[test]
fn patches_are_self_consistent() {
    for p in PATCHES {
        assert_ne!(p.old, p.new);
        assert!(!p.old.is_empty());
        // old 不得是 new 的子串：否则已打补丁的副本重跑 apply() 时，
        // contains(old) 仍为真，会把补丁再插一遍
        assert!(
            !p.new.contains(p.old),
            "补丁 {} 的 new 包含 old，可重入判定失效",
            p.name
        );
        // 反向同理：撤回要把 new 替换回 old，new 也不得是 old 的子串，
        // 否则 contains(new) 的撤回判定会在 pristine 状态误触发
        assert!(
            !p.old.contains(p.new),
            "补丁 {} 的 old 包含 new，撤回判定失效",
            p.name
        );
        // 补丁不得触碰权限判定
        for bad in ["isFeatureAllowed", "isPro", "getCurrentUser().plan"] {
            assert!(
                !p.new.contains(bad),
                "补丁 {} 触碰了权限判定 {}",
                p.name,
                bad
            );
        }
    }
}

#[test]
fn multi_chart_injection_matches_const() {
    // 注入脚本常量与补丁字面文本必须逐字一致，避免两处维护漂移
    let b = PATCHES
        .iter()
        .find(|p| p.group == Group::Charts)
        .expect("多图表补丁应在补丁集里");
    assert!(b.new.contains(MULTI_CHART_UNLOCK));
}

#[test]
fn study_on_study_injection_matches_const() {
    // 同上：指标叠加注入串常量与补丁字面文本逐字一致
    let s = PATCHES
        .iter()
        .find(|p| p.group == Group::StudyOnStudy)
        .expect("指标叠加补丁应在补丁集里");
    assert!(s.new.contains(STUDY_ON_STUDY_UNLOCK));
    // 注入串里不得出现双引号：它整体包在外层 executeJavaScript("...") 里
    assert!(!STUDY_ON_STUDY_UNLOCK.contains('"'));
}

#[test]
fn drawings_injection_matches_const() {
    // 绘图数量解锁脚本与补丁字面文本逐字一致；整串嵌在 executeJavaScript("…") 里，
    // 内部不得出现双引号（否则会提前闭合外层字符串）。
    let d = PATCHES
        .iter()
        .find(|p| p.group == Group::Drawings)
        .expect("绘图数量补丁应在补丁集里");
    assert!(d.new.contains(DRAWINGS_UNLOCK));
    assert!(!DRAWINGS_UNLOCK.contains('"'));
    // 锚点不得与其他补丁重复（重复会让先打上的那条吃掉另一条的锚点）
    for other in PATCHES.iter().filter(|p| p.name != d.name) {
        assert_ne!(other.old, d.old, "绘图数量锚点与 {} 重复", other.name);
    }
}

#[test]
fn disconnect_recovery_injection_matches_const() {
    // 断线自愈脚本与补丁字面文本必须逐字一致；且整串要嵌在双引号里，
    // 因此脚本内部不得出现双引号（否则会提前闭合 executeJavaScript("…")）。
    let p = PATCHES
        .iter()
        .find(|p| p.group == Group::Recover)
        .expect("断线自愈补丁应在补丁集里");
    assert!(p.new.contains(DISCONNECT_RECOVER));
    assert!(!DISCONNECT_RECOVER.contains('"'));
    // 弹窗消失后不应再被"藏起来"——旧行为已废弃
    assert!(!p.new.contains("display='none'"));
    // 界面语言跟随 TradingView 账号，不能只认中英文：抽查几条非中英词条
    for s in ["Verbindung wurde geschlossen", "接続を閉じました", "соединение закрыто", "연결이 종료되었습니다"] {
        assert!(DISCONNECT_RECOVER.contains(s), "断线文案缺少 {s}");
    }
    for s in ["Verbindung wiederherstellen", "再接続", "Переподключиться", "Reconectar"] {
        assert!(DISCONNECT_RECOVER.contains(s), "重连按钮文案缺少 {s}");
    }
    // 「获取更多连接 / Get more connections」是连接数付费墙的标题，不是断线文案：
    // 留着会把付费墙判成断线（找不到重连按钮 → 重载，最多 4 次）。已实测复现。
    assert!(
        !DISCONNECT_RECOVER.contains("获取更多连接"),
        "断线文案不能再吃付费墙标题"
    );
    assert!(
        !DISCONNECT_RECOVER.contains("Get more connections"),
        "断线文案不能再吃付费墙标题（en）"
    );
    // 但真正的断线句必须还在（否则自愈失效）
    assert!(DISCONNECT_RECOVER.contains("关闭了这个连接"));
    assert!(DISCONNECT_RECOVER.contains("We closed this connection"));
}

#[test]
fn promo_guard_injection_matches_const() {
    // 页内推广浮层脚本必须与常量逐字一致（常量只用于漂移检查），
    // 同样不得出现双引号：整串嵌在 executeJavaScript("…") 里。
    let p = PATCHES
        .iter()
        .find(|p| p.new.contains("__tvxGuard"))
        .expect("页内推广浮层补丁应在补丁集里");
    assert!(p.new.contains(PROMO_GUARD));
    assert!(!PROMO_GUARD.contains('"'));
    // v2 的核心：插入即判定（微任务里隐藏），不再等轮询
    assert!(PROMO_GUARD.contains("new MutationObserver"));
    assert!(PROMO_GUARD.contains("subtree:!0"));
    assert!(!PROMO_GUARD.contains(".click("), "v2 不再点按钮：点了没用的 × 是 v1 的漏点");
    // 识别不依赖任何一种语言的标题，也不依赖 TradingView 这个词
    assert!(PROMO_GUARD.contains("var TIER=/basic|essential|premium|ultimate|advanced/gi"));
    assert!(PROMO_GUARD.contains("if(k>=2&&(buy||br||lim))return 1"));
    // 摘节点时必须避开含 canvas 的容器（图表本体）
    assert!(PROMO_GUARD.contains("querySelector('canvas')"));
    assert!(PROMO_GUARD.contains("if(hasChart(e))return"));
    // 断线提示交给自愈，不能被当推广藏掉
    assert!(PROMO_GUARD.contains("恢复连接"));
    assert!(PROMO_GUARD.contains("if(DIS.test(t))return 0"));
    // 但 DIS 不能再认「获取更多连接」——那是连接数付费墙的标题（实测放过）
    assert!(!PROMO_GUARD.contains("获取更多连接"));
    assert!(!PROMO_GUARD.contains("Get more connections"));
    // v3：注入只填空位；「等 BrowserView 就绪」由恒开的 Core 补丁提供。
    // 两者绑在一起会让「关掉拦截广告弹窗」顺带关掉启动期重试（审计发现的耦合）。
    assert_eq!(p.old, SHELL_HOOK, "推广补丁只能填空位，不能自带壳");
    assert!(p.new.starts_with("t.handleDomReady(),void t.executeJavaScript(\""));
    assert!(p.new.ends_with("\").catch((()=>{}))"));
    let core = PATCHES
        .iter()
        .find(|q| q.group == Group::Core)
        .expect("Core 补丁必须在补丁集里");
    assert_eq!(core.old, DOM_READY_ORIG);
    assert_eq!(core.new, DOM_READY_SHELL);
    assert!(core.new.contains(SHELL_HOOK), "壳里必须留出注入空位");
    assert!(core.new.contains("(function r(){if(g!==t.__tvxDR)return;if(t.isBrowserViewValid())"));
    assert!(core.new.contains("browser view not ready, retrying"));
    // 重入判定与 is_patched 认的是这段特征（core_shell_present），
    // 壳若改了这段文本必须同步改判定，这里把两处钉在一起
    assert!(core.new.contains("t.__tvxDR=(t.__tvxDR||0)+1"));
    assert!(core_shell_present(core.new));
    assert!(!core_shell_present(core.old), "原始锚点里不应有壳特征");
    // 壳恒开：关掉全部功能开关也不能把重试带走
    let all_off = Flags {
        adblock: false,
        charts_unlock: false,
        study_on_study: false,
        drawings_unlock: false,
        no_auto_update: false,
        auto_recover: false,
        alerts: false,
    };
    assert!(Group::Core.enabled(&all_off), "Core 必须恒开");
}

/// 历史注入文本（只隐藏版 / 只存内存计数的 v1 / 上一版开窗过滤）：apply() 必须能就地
/// 升级成当前文本，而不是报"锚点失配"。这是升级路径的回归测试。
/// 断线自愈那两条必须是"完整锚点段"——apply() 拿 p.new 整段去替换它。
/// 若只存脚本本体，替换后新段会嵌在脚本位置，得到重复的 handleDomReady，
/// 主进程加载即语法错误（真实踩过：TradingView 弹出只有 "Error" 的窗口）。
/// 开窗过滤那条不用整段：它的位置在 `return;` 与 `const o` 之间，替换的是整条语句。
#[test]
fn legacy_entries_are_full_segments() {
    for (name, legacy) in LEGACY {
        let p = PATCHES
            .iter()
            .find(|p| p.name == *name)
            .unwrap_or_else(|| panic!("{name}：LEGACY 指向了不存在的补丁"));
        assert_ne!(*legacy, p.new, "{name}：历史文本不能等于当前文本");
        if p.group != Group::Recover {
            // 插入式补丁：旧文本 = 原锚点 + 插入的条件，所以锚点同时是首尾两端
            // 插入式补丁的历史文本 = 原锚点的前段 + 插入的条件 + 原锚点的后段，
            // 所以只校验首尾各 20 字符仍在（中间必然被插入了东西）。
            let head: String = p.old.chars().take(20).collect();
            let tail: String = p.old.chars().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect();
            assert!(legacy.starts_with(&head), "{name}：历史文本的首段必须来自原锚点");
            assert!(legacy.ends_with(&tail), "{name}：历史文本的尾段必须来自原锚点");
            continue;
        }
        assert!(
            legacy.starts_with(DOM_READY_HEAD),
            "{name}：必须以 handleDomReady 段开头"
        );
        // LEGACY 里的文本是历史版本写进副本的原样，段尾一律是 `this.url&&`
        // （URL 判断换写法是后来的事），所以这里只校验稳定的那半截。
        assert!(
            legacy.ends_with(RECOVER_URL_TAIL),
            "{name}：必须以原始锚点 this.url&& 结尾"
        );
        assert_eq!(
            legacy.matches("handleDomReady(){").count(),
            1,
            "{name}：只能含一处 handleDomReady"
        );
    }
}

/// 任意一版历史注入段（v1 内存计数 / v2 会话存储 / 以后的新版）都必须能按段边界
/// 整段换成当前脚本，而不是报"锚点失配"。这里用一个"既不是原始锚点、也不等于
/// 当前脚本"的段来模拟历史版本——这正是已装好的副本真实所处的状态。
#[test]
fn stale_recover_segment_is_migrated_whole() {
    let dir = std::env::temp_dir().join("tvx-patch-stale-segment-test");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("index.js");

    let p = PATCHES
        .iter()
        .find(|p| p.group == Group::Recover)
        .expect("断线自愈补丁应在补丁集里");
    // 模拟旧版：等待序列不同（v2 就是这样）
    let stale = p.new.replace("var waits=[0,15000,60000,300000]", "var waits=[15000,60000,300000]");
    assert_ne!(stale, p.new, "模拟的历史脚本不能与当前脚本相同");

    let src: String = PATCHES
        .iter()
        .map(|q| if q.group == Group::Recover { stale.as_str() } else { q.new })
        .collect();
    fs::write(&f, &src).unwrap();

    // 开：整段换成当前脚本，且文件里只能剩一个 handleDomReady 段
    apply(&f, &Flags::default()).unwrap();
    let after = fs::read_to_string(&f).unwrap();
    assert!(!after.contains(&stale), "历史脚本应被整段换掉");
    assert!(after.contains(p.new), "应迁移成当前脚本");
    assert_eq!(
        after.matches(DOM_READY_HEAD).count(),
        1,
        "迁移后 handleDomReady 段必须只有一处"
    );
    assert_eq!(after.matches("this.url&&").count(), 1, "尾部锚点必须只有一处");
    assert!(is_patched(&f, &Flags::default()));

    // 幂等：再跑一次不应改动文件
    let before = fs::read_to_string(&f).unwrap();
    let report = apply(&f, &Flags::default()).unwrap();
    assert!(report.items.iter().all(|(_, st)| matches!(
        st,
        Status::AlreadyPatched | Status::Untouched
    )));
    assert_eq!(fs::read_to_string(&f).unwrap(), before);

    let _ = fs::remove_dir_all(&dir);
}

/// v3 初版（壳与注入耦合在同一个 `.on("dom-ready")` 回调里）→ 现状（恒开壳 + 可开关注入）：
/// 已装好的副本里是"整段耦合"的文本，apply() 必须整段重置成壳再按开关补注入，
/// 而不是报"锚点失配"——否则用户升级助手后还是要手工删副本。
#[test]
fn stale_promo_segment_is_migrated_whole() {
    let dir = std::env::temp_dir().join("tvx-patch-stale-promo-test");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("index.js");

    let p = PATCHES
        .iter()
        .find(|q| q.new.contains("__tvxGuard"))
        .expect("页内推广浮层补丁应在补丁集里");
    // 模拟 v3 初版：壳 + 旧脚本（脚本标记换成 __tvxPromo）耦合在同一个回调里
    let stale_guard = p.new.replace("window.__tvxGuard=1", "window.__tvxPromo=1");
    let stale = DOM_READY_SHELL.replacen(SHELL_HOOK, &stale_guard, 1);
    assert_ne!(stale, DOM_READY_SHELL, "模拟的历史段不能与当前壳相同");

    // 旧副本的形态：Core 的位置放耦合段，推广补丁的小段不存在
    let src: String = PATCHES
        .iter()
        .map(|q| {
            if q.name == p.name {
                ""
            } else if q.group == Group::Core {
                stale.as_str()
            } else {
                q.new
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&f, &src).unwrap();

    let report = apply(&f, &Flags::default()).unwrap();
    assert!(report.failed.is_empty(), "不应有补丁失配：{:?}", report.failed);
    let after = fs::read_to_string(&f).unwrap();
    assert!(!after.contains("__tvxPromo"), "旧脚本应被整段换掉");
    assert!(after.contains("__tvxGuard"), "应迁移成当前注入");
    // 注入打上后壳里的空位被填掉，整段文本不再是 DOM_READY_SHELL，认壳特征
    assert!(core_shell_present(&after), "壳必须存在");
    assert_eq!(
        after.matches("t.__tvxDR=(t.__tvxDR||0)+1").count(),
        1,
        "壳必须只有一处"
    );
    // 尾巴标记 `"` + `.catch` 在别的注入补丁里也有（多图表/指标/绘图/断线），
    // 这里只认壳独有的重试语句，确保没有被复制成两份。
    assert_eq!(after.matches("browser view not ready, retrying").count(), 1, "重试日志必须只有一处");
    assert!(is_patched(&f, &Flags::default()));

    // 关掉「拦截广告弹窗」：注入回退成空位，但**壳必须留着**——
    // 否则断线自愈/绘图等注入会跟着退回"启动后几十秒空窗"的老毛病（审计发现的耦合）。
    let off = Flags {
        adblock: false,
        ..Flags::default()
    };
    apply(&f, &off).unwrap();
    let after = fs::read_to_string(&f).unwrap();
    assert!(!after.contains("__tvxGuard"), "关掉后不应留注入");
    assert!(after.contains(DOM_READY_SHELL), "关掉推广后重试壳必须保留");
    assert!(after.contains(SHELL_HOOK), "空位必须回填");
    assert!(is_patched(&f, &off));

    let _ = fs::remove_dir_all(&dir);
}

/// 审计回归：关掉「拦截广告弹窗」（合并后的单一开关）不能把 dom-ready 重试壳一起关掉。
///
/// v0.1.6 初版把轮询壳写在「页内推广浮层」这条补丁里（当时的充值屏蔽组），
/// 于是用户只要关掉这一个开关，断线自愈 / 绘图 / 多图表的注入就退回
/// "首次 dom-ready 视图未就绪 → 整段短路且不重试"的老毛病。
/// 现在壳是恒开的 Core 补丁，这条用例把关系钉死。
#[test]
fn adblock_off_keeps_dom_ready_shell() {
    let dir = std::env::temp_dir().join("tvx-patch-shell-independence");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("index.js");

    let promo_name = PATCHES
        .iter()
        .find(|p| p.new.contains("__tvxGuard"))
        .map(|p| p.name)
        .expect("页内推广浮层补丁应在补丁集里");
    let pristine: String = PATCHES
        .iter()
        .filter(|p| p.name != promo_name)
        .map(|p| p.old)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&f, &pristine).unwrap();

    let off = Flags {
        adblock: false,
        ..Flags::default()
    };
    apply(&f, &off).unwrap();
    let after = fs::read_to_string(&f).unwrap();

    assert!(core_shell_present(&after), "关掉推广后重试壳必须还在");
    assert!(after.contains(SHELL_HOOK), "壳里应留空位");
    assert!(!after.contains("__tvxGuard"), "推广脚本不应被注入");
    // 其余注入照常（它们都排在壳后面，靠壳等视图就绪）
    for mark in ["__tvxRecover", "__tvxDraw", "_exposed_chartWidgetCollection"] {
        assert!(after.contains(mark), "关掉推广不应影响 {mark} 注入");
    }
    assert!(is_patched(&f, &off), "is_patched 应认为当前状态正确");

    // 再打开：注入补上，壳仍只有一处
    apply(&f, &Flags::default()).unwrap();
    let back = fs::read_to_string(&f).unwrap();
    assert!(back.contains("__tvxGuard"));
    assert_eq!(back.matches("t.__tvxDR=(t.__tvxDR||0)+1").count(), 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn legacy_injections_are_migrated_in_place() {
    let dir = std::env::temp_dir().join("tvx-patch-legacy-test");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("index.js");

    for (name, legacy) in LEGACY {
        let mut src = String::new();
        for p in PATCHES {
            if p.name == *name {
                src.push_str(legacy);
            } else {
                src.push_str(p.new);
            }
        }
        fs::write(&f, &src).unwrap();

        // 开：历史脚本 → 当前脚本
        apply(&f, &Flags::default()).unwrap();
        let after = fs::read_to_string(&f).unwrap();
        assert!(!after.contains(legacy), "历史脚本应被替换掉");
        assert!(after.contains(DISCONNECT_RECOVER), "应升级成当前脚本");
        // 迁移必须整段替换：只换脚本本体会把新段嵌进脚本位置，
        // 生成两段 handleDomReady → 主进程语法错误、直接弹 Error 窗。
        assert_eq!(
            after.matches(DOM_READY_HEAD).count(),
            1,
            "迁移后 handleDomReady 段必须只有一处"
        );
        assert_eq!(after.matches("this.url&&").count(), 1, "尾部锚点必须只有一处");
        assert!(is_patched(&f, &Flags::default()));
    }

    // 关：当前脚本 → 原始锚点
    let recover = PATCHES
        .iter()
        .find(|p| p.group == Group::Recover)
        .expect("断线自愈补丁应在补丁集里");
    let off = Flags {
        auto_recover: false,
        ..Flags::default()
    };
    apply(&f, &off).unwrap();
    let after = fs::read_to_string(&f).unwrap();
    assert!(after.contains(recover.old), "关掉后应回到原始断点锚点");
    assert!(!after.contains(DISCONNECT_RECOVER));

    let _ = fs::remove_dir_all(&dir);
}

/// v2（`handleDomReady()` 与注入直连）→ 现状（恒开壳 + 注入填空位）：已装好的副本里是 v2 文本，
/// 段首是 `.on("dom-ready",(()=>{this.handleDomReady(),this.isBrowserViewValid()&&…`——
/// 迁移必须认出这个旧起点并整段换成壳，否则用户升级后又要手工删副本。
#[test]
fn v2_promo_segment_is_migrated_to_shell() {
    let dir = std::env::temp_dir().join("tvx-patch-promo-v2-test");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("index.js");

    let p = PATCHES
        .iter()
        .find(|q| q.new.contains("__tvxGuard"))
        .expect("页内推广浮层补丁应在补丁集里");
    let guard = p
        .new
        .strip_prefix("t.handleDomReady(),void t.executeJavaScript(\"")
        .and_then(|r| r.strip_suffix("\").catch((()=>{}))"))
        .expect("当前补丁文本必须符合「空位 + 脚本 + 收尾」的结构");
    // 显式拼接，避免 format! 的 `}}` 转义把收尾括号算错一位
    let mut v2 = String::new();
    v2.push_str(PROMO_SEG_HEAD_V2);
    v2.push_str(guard);
    v2.push_str(PROMO_SEG_END_MARK);
    v2.push_str("}))");

    // 旧副本形态：Core 的位置放 v2 段，推广补丁的小段不存在
    let src: String = PATCHES
        .iter()
        .map(|q| {
            if q.name == p.name {
                ""
            } else if q.group == Group::Core {
                v2.as_str()
            } else {
                q.new
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&f, &src).unwrap();

    let report = apply(&f, &Flags::default()).unwrap();
    assert!(report.failed.is_empty(), "不应有补丁失配：{:?}", report.failed);
    let after = fs::read_to_string(&f).unwrap();
    assert!(!after.contains(PROMO_SEG_HEAD_V2), "v2 段首应被换掉");
    assert!(core_shell_present(&after), "应迁移成壳（带就绪重试）");
    assert!(after.contains("__tvxGuard"), "注入应按开关补上");
    assert_eq!(after.matches("browser view not ready, retrying").count(), 1);
    assert!(is_patched(&f, &Flags::default()));

    let _ = fs::remove_dir_all(&dir);
}

/// 上一版「页面开窗过滤」（正则里没有 paywall / pro-plans）必须能就地迁移：
/// 副本里是"条件已插在 `return;` 与 `const o` 之间"的旧形态，原锚点已被拆开，
/// 靠精确匹配会判成锚点未命中，从而整批失败。
#[test]
fn former_open_window_filter_is_migrated() {
    let dir = std::env::temp_dir().join("tvx-patch-openwin-test");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("index.js");

    let p = PATCHES
        .iter()
        .find(|q| q.new.contains("const o={url:n};") && q.new.contains("paywall"))
        .expect("开窗过滤补丁应在补丁集里");
    let src: String = PATCHES
        .iter()
        .map(|q| {
            if q.name == p.name {
                LEGACY_OPEN_WINDOW_V1
            } else {
                q.new
            }
        })
        .collect();
    fs::write(&f, &src).unwrap();

    let report = apply(&f, &Flags::default()).unwrap();
    assert!(report.failed.is_empty(), "不应有补丁失配：{:?}", report.failed);
    let after = fs::read_to_string(&f).unwrap();
    assert!(!after.contains(LEGACY_OPEN_WINDOW_V1), "旧文本应被替换");
    assert!(after.contains(p.new), "应迁移成当前文本");
    assert_eq!(after.matches(p.new).count(), 1);

    // 幂等
    apply(&f, &Flags::default()).unwrap();
    assert_eq!(fs::read_to_string(&f).unwrap(), after);

    let _ = fs::remove_dir_all(&dir);
}

/// 用一段含真实 old 锚点的临时文件，验证开关矩阵下的
/// 打上 / 撤回 / 幂等行为。合并后的「拦截广告弹窗」组含多条补丁，
/// 这里整组参与断言：一个开关必须同时管住网络层与壳层。
#[test]
fn apply_respects_flags_and_is_reversible() {
    let dir = std::env::temp_dir().join("tvx-patch-flag-test");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("index.js");

    // pristine = 全部补丁 old 依序拼接（apply 按组应用组内全部补丁，
    // 所以每个 old 都必须恰好出现一次；old 之间互不包含已由
    // patches_are_self_consistent 覆盖）
    // 推广补丁的 old/new 只是壳内的一小段（空位 ↔ 注入），不是独立语句：
    // 它在文件里的形态由 Core 的 old（原始调用）或 new（壳）决定，拼接时跳过。
    let promo_name = PATCHES
        .iter()
        .find(|p| p.new.contains("__tvxGuard"))
        .map(|p| p.name)
        .expect("页内推广浮层补丁应在补丁集里");
    let pristine: String = PATCHES
        .iter()
        .filter(|p| p.name != promo_name)
        .map(|p| p.old)
        .collect();

    // 各功能组各取代表补丁，便于后半段断言
    let adblock: Vec<&Patch> = PATCHES
        .iter()
        .filter(|p| p.group == Group::AdBlock)
        .collect();
    assert!(
        adblock.len() >= 2,
        "拦截广告弹窗应同时包含网络层与壳层补丁，实际 {} 条",
        adblock.len()
    );
    let charts = PATCHES.iter().find(|p| p.group == Group::Charts).unwrap();
    let study = PATCHES
        .iter()
        .find(|p| p.group == Group::StudyOnStudy)
        .unwrap();
    let noauto = PATCHES
        .iter()
        .find(|p| p.group == Group::NoAutoUpdate)
        .unwrap();

    let write_pristine = |contents: &str| fs::write(&f, contents).unwrap();
    let read = || fs::read_to_string(&f).unwrap();

    // 1) 全开：全部打上，且是合法 UTF-8 侧无损（长度按替换对变化）
    write_pristine(&pristine);
    let report = apply(&f, &Flags::default()).unwrap();
    assert!(report
        .items
        .iter()
        .all(|(_, st)| *st == Status::Applied || *st == Status::AlreadyPatched));
    for p in &adblock {
        assert!(read().contains(p.new), "{} 应被打上", p.name);
    }
    assert!(read().contains(charts.new));
    assert!(read().contains(study.new));
    assert!(read().contains(noauto.new));

    // 2) 全关：可开关的组全部撤回。Auth 组恒开（多账号登录的前提），
    //    所以期望结果是「pristine + Auth 补丁」，而不是逐字 pristine。
    let all_off = Flags {
        adblock: false,
        charts_unlock: false,
        study_on_study: false,
        drawings_unlock: false,
        no_auto_update: false,
        auto_recover: false,
        alerts: false,
    };
    // 只有 Auth 组生效时的目标文本
    let keep_auth: String = PATCHES
        .iter()
        .filter(|p| p.name != promo_name)
        .map(|p| {
            if p.group == Group::Auth || p.group == Group::Core {
                p.new
            } else {
                p.old
            }
        })
        .collect();
    write_pristine(&read()); // 当前是已打状态，先按现状
    let report = apply(&f, &all_off).unwrap();
    assert!(report.items.iter().all(|(_, st)| matches!(
        st,
        Status::Reverted | Status::Untouched | Status::AlreadyPatched
    )));
    assert_eq!(read(), keep_auth, "撤回后应只剩恒开的 Auth + Core 补丁");
    for p in PATCHES
        .iter()
        .filter(|p| p.group == Group::Auth || p.group == Group::Core)
    {
        assert!(read().contains(p.new), "{} 应被恒开保留", p.name);
    }
    assert!(is_patched(&f, &all_off), "全关状态下 is_patched 应为真");

    // 3) 部分开关：只开图表解锁 —— 其余组未启用，charts 打上
    write_pristine(&read()); // 已回到 keep_auth
    let only_charts = Flags {
        adblock: false,
        charts_unlock: true,
        study_on_study: false,
        drawings_unlock: false,
        no_auto_update: false,
        auto_recover: false,
        alerts: false,
    };
    apply(&f, &only_charts).unwrap();
    let now = read();
    assert!(now.contains(charts.new));
    for p in PATCHES
        .iter()
        .filter(|p| {
            p.group != Group::Charts && p.group != Group::Auth && p.group != Group::Core
        })
    {
        assert!(!now.contains(p.new), "{} 不应被打上", p.name);
    }
    assert!(is_patched(&f, &only_charts));

    // 4) 幂等：同配置重跑应全部 AlreadyPatched/Untouched 且文件不变
    let before = read();
    let report = apply(&f, &only_charts).unwrap();
    assert!(report.items.iter().all(|(_, st)| matches!(
        st,
        Status::AlreadyPatched | Status::Untouched
    )));
    assert_eq!(read(), before);

    let _ = fs::remove_dir_all(&dir);
}

/// 报警功能的资源文件：开关打开时落盘、关闭时清干净、preload 桥幂等可逆。
#[test]
fn alerts_assets_are_written_and_removed() {
    let dir = std::env::temp_dir().join("tvx-alerts-assets-test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("preload")).unwrap();
    let preload = dir.join("preload").join("platform-preload.js");
    fs::write(&preload, "console.log('x');\n").unwrap();

    let on = Flags::default();
    install_assets(&dir, &on).unwrap();
    assert!(dir.join("tvx-alerts-engine.js").is_file(), "引擎脚本应落盘");
    assert!(dir.join("tvx-alerts-main.js").is_file(), "主进程模块应落盘");
    let p = fs::read_to_string(&preload).unwrap();
    assert_eq!(p.matches(PRELOAD_BRIDGE_MARK).count(), 1, "桥只应追加一次");
    assert!(p.contains("__TVX_ALERTS__"));

    // 幂等：重跑不应再追加一份桥
    install_assets(&dir, &on).unwrap();
    let p2 = fs::read_to_string(&preload).unwrap();
    assert_eq!(p2.matches(PRELOAD_BRIDGE_MARK).count(), 1);
    assert_eq!(p2, p, "重复执行不应改动 preload");

    let off = Flags {
        alerts: false,
        ..Flags::default()
    };
    install_assets(&dir, &off).unwrap();
    assert!(!dir.join("tvx-alerts-engine.js").exists(), "关闭后引擎脚本应删除");
    assert!(!dir.join("tvx-alerts-main.js").exists(), "关闭后主进程模块应删除");
    let p3 = fs::read_to_string(&preload).unwrap();
    assert!(!p3.contains(PRELOAD_BRIDGE_MARK), "关闭后桥应撤回");
    assert_eq!(p3, "console.log('x');\n", "撤回后应逐字还原");

    // 再打开：能从关闭态恢复
    install_assets(&dir, &on).unwrap();
    assert!(fs::read_to_string(&preload)
        .unwrap()
        .contains(PRELOAD_BRIDGE_MARK));

    let _ = fs::remove_dir_all(&dir);
}

/// 两个 JS 资源的基本形态：引擎走 executeJavaScript（必须自执行），
/// 主进程模块必须注册 IPC 处理器。
#[test]
fn alerts_bundles_look_valid() {
    assert!(ALERTS_ENGINE_JS.contains("__TVX_ALERTS__"), "引擎要认 preload 桥");
    assert!(ALERTS_ENGINE_JS.contains("getAllStudies"), "引擎按图表指标列表配置");
    assert!(ALERTS_ENGINE_JS.contains("__tvxAlerts"), "引擎必须防重复注入");
    assert!(ALERTS_MAIN_JS.contains("ipcMain.handle"), "主进程模块要注册 IPC");
    assert!(ALERTS_MAIN_JS.contains("tvx-alerts:fire"), "触发通道名要固定");
    assert!(ALERTS_MAIN_JS.contains("executeJavaScript"), "主进程负责注入引擎");
    assert!(!ALERTS_MAIN_JS.contains("process.exit"), "主进程模块不得退出进程");

    // 报警设置的四段结构要与需求对齐（条件 / 周期和触发 / 消息 / 通知）
    for s in ["条件", "周期和触发", "消息", "通知"] {
        assert!(ALERTS_ENGINE_JS.contains(s), "设置页缺少分组：{s}");
    }
    // 条件必须与原生对话框一致：脚本 alertcondition 的名字在 metainfo.styles[plotId].title
    // （如 "Bullish Trend Shift"），数量与顺序来自 metainfo.plots 里 type=alertcondition 的条目。
    // 实测：alertcondition 不占数据列，靠同名/同方向输出列还原信号；colorer 占数据列但
    // 不在 valuesProvider 里；na 交替列用「出现/消失」表达。
    assert!(ALERTS_ENGINE_JS.contains("_metaInfo"), "条件要读指标的元数据");
    assert!(ALERTS_ENGINE_JS.contains("COL_TYPES"), "要按 plot 类型把不占数据列的条目分开");
    assert!(ALERTS_ENGINE_JS.contains("alertcondition"), "要识别脚本声明的 alertcondition");
    assert!(ALERTS_ENGINE_JS.contains("buildCond"), "alertcondition 要落到可读信号上");
    assert!(ALERTS_ENGINE_JS.contains("styles[cd.id]"), "条件名取自 styles[plotId].title");
    assert!(ALERTS_ENGINE_JS.contains("nameScore"), "条件与输出列要能按名字对账");
    assert!(ALERTS_ENGINE_JS.contains("isNaAlternating"), "上下轨这类 na 交替列要能当信号");
    assert!(ALERTS_ENGINE_JS.contains("evalChangedTo"), "colorer 状态列按「变为」求值");
    assert!(ALERTS_ENGINE_JS.contains("disappear"), "na 交替列要能用出现/消失表达");
    assert!(ALERTS_ENGINE_JS.contains("serverConds"), "要告诉用户脚本还有几个服务端条件");
    // 免费账号的客户端 metainfo 常常不带 alertcondition（技术报警是付费能力）。
    // 这时按**这个实例自己的脚本 id**读源码，把 alertcondition 的名字原样取出来，
    // 保证条件下拉仍是脚本真正声明的那几条，而不是一堆无关输出列。
    assert!(ALERTS_ENGINE_JS.contains("parsePineConds"), "要能从脚本源码解析条件名");
    assert!(ALERTS_ENGINE_JS.contains("pine-facade"), "要能取到脚本源码");
    assert!(ALERTS_ENGINE_JS.contains("needPine"), "元数据缺条件名时要走源码兜底");
    // 关键：一切条件名必须来自「图表上这个指标实例」。
    // 改名副本（如 "…SuperTrend [MergeAlerts]"）与公开版条件并不相同，按名字搜索会拉错脚本，
    // 所以：① 多来源读该实例的元数据（alertConditions 与 plots 可能只挂在其中一处）；
    //      ② 只按该实例的脚本 id 读源码；③ 明确禁止按名字去公开库猜。
    assert!(ALERTS_ENGINE_JS.contains("fetchSourceConds"), "要能按脚本 id 读它自己的源码");
    assert!(ALERTS_ENGINE_JS.contains("scriptId"), "要从元数据里取脚本 id");
    assert!(ALERTS_ENGINE_JS.contains("metaSources"), "要读图上这个实例自己的元数据");
    assert!(ALERTS_ENGINE_JS.contains("alertConditions"), "原生对话框的条件名就在 alertConditions 里");
    assert!(!ALERTS_ENGINE_JS.contains("pubscripts-suggest-json"),
        "条件名只能来自图上这个实例，不能按名字去公开脚本库猜");
    assert!(ALERTS_ENGINE_JS.contains("plotConds"), "读不到条件名时要用输出列兜底，不能空");
    assert!(ALERTS_ENGINE_JS.contains("输出列·过渡"), "读取期间要标明当前是过渡列表");
    // 报警列表照原生「警报」列表做：一行一条、细节展开；指标参数只显示可读项，
    // 绝不把 pineId / pineFeatures / in_0 / base64 长文本铺到列表里
    assert!(ALERTS_ENGINE_JS.contains("tvxa-arow"), "列表要用原生那种紧凑行");
    assert!(ALERTS_ENGINE_JS.contains("fmtThreshold"), "阈值显示要去掉多余的零并加千分位");
    assert!(ALERTS_ENGINE_JS.contains("in_\\d"), "指标参数要过滤 Pine 内部输入项");
    assert!(ALERTS_ENGINE_JS.contains("tvx-pine-conds"), "条件名要缓存，别每次开面板都请求");
    assert!(ALERTS_MAIN_JS.contains("tvx-alerts:dump"), "自检转储通道");
    // 周期/触发不单独设置，只标注与图表一致
    assert!(ALERTS_ENGINE_JS.contains("与图表一致"));
    assert!(!ALERTS_ENGINE_JS.contains("冷却（分钟"), "触发频次不再暴露给用户");
    // 占位符渲染与 OHLC 数据源
    assert!(ALERTS_ENGINE_JS.contains("renderTemplate"));
    assert!(ALERTS_ENGINE_JS.contains("mainSeries"));
    assert!(ALERTS_ENGINE_JS.contains("\\{\\{\\s*([a-zA-Z_][a-zA-Z0-9_]*)\\s*\\}\\}"));
    // 入口：右侧工具栏 + 品牌色悬浮胶囊
    assert!(ALERTS_ENGINE_JS.contains("right-toolbar"));
    assert!(ALERTS_ENGINE_JS.contains("tvxa-toolbar-btn"));
    // 邮件可按报警覆盖收发件人
    assert!(ALERTS_MAIN_JS.contains("emailFrom") && ALERTS_MAIN_JS.contains("emailTo"));
    assert!(ALERTS_MAIN_JS.contains("if (p.body) return p.body"), "邮件正文优先用模板渲染结果");
}

#[test]
fn wild_anchor_tolerates_renamed_identifiers() {
    // 压缩标识符随 webpack 重打包重排：通配锚点要能重新定位并保留新名字
    let src = "async function xQ(e,t){const n=(0,Ab.cD)().getLastFocusedWindow();";
    let pat = "async function {}(e,t){const n=(0,{}.{})().getLastFocusedWindow();";
    let hits = wild_find_all(src, pat);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].1, vec!["xQ", "Ab", "cD"]);
    let to = wild_expand(
        "async function {}(e,t){return;const n=(0,{}.{})().getLastFocusedWindow();",
        &hits[0].1,
    );
    assert_eq!(
        to,
        "async function xQ(e,t){return;const n=(0,Ab.cD)().getLastFocusedWindow();"
    );
    // 通配只吃标识符字符，不能跨过标点乱匹配
    assert!(wild_find_all("async function f(){const n=0}", pat).is_empty());
}

#[test]
fn apply_degrades_per_patch_and_survives_renames() {
    let dir = std::env::temp_dir().join("tvx-patch-degrade-test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let idx = dir.join("index.js");

    // 用所有补丁的 old 拼出一份"原始" index.js。
    // 推广补丁的 old 只是壳内空位、不是独立语句，跳过（由 Core 的 old 提供落点）。
    let promo_name = PATCHES
        .iter()
        .find(|p| p.new.contains("__tvxGuard"))
        .map(|p| p.name)
        .expect("页内推广浮层补丁应在补丁集里");
    let pristine: String = PATCHES
        .iter()
        .filter(|p| p.name != promo_name)
        .map(|p| p.old)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&idx, &pristine).unwrap();

    let on = Flags::default();
    let first = apply(&idx, &on).unwrap();
    assert!(first.failed.is_empty(), "全开时不应有失败：{:?}", first.failed);
    assert!(first.items.iter().all(|(_, s)| *s == Status::Applied));

    // 幂等：再跑一次全部是"已打过"
    let second = apply(&idx, &on).unwrap();
    assert!(second
        .items
        .iter()
        .all(|(_, s)| *s == Status::AlreadyPatched));

    // 撤回：开关全关后逐条还原（Auth 恒开，跳过它）
    let off = Flags {
        adblock: false,
        charts_unlock: false,
        study_on_study: false,
        drawings_unlock: false,
        no_auto_update: false,
        auto_recover: false,
        alerts: false,
    };
    let third = apply(&idx, &off).unwrap();
    assert!(third.failed.is_empty(), "撤回不应失败：{:?}", third.failed);

    // 模拟上游重打包：压缩标识符全改名，字面锚点失配，通配锚点应接住
    let drifted = pristine
        .replace("vL(e,t)", "xQ(e,t)")
        .replace("(0,Le.Qh)()", "(0,Zz.Yy)()")
        .replace("(0,M.j9)()", "(0,Wq.r8)()");
    assert!(drifted.contains("xQ(e,t)"));
    fs::write(&idx, &drifted).unwrap();
    let fourth = apply(&idx, &on).unwrap();
    assert!(
        fourth.failed.is_empty(),
        "只改压缩标识符时不应报失配：{:?}",
        fourth.failed
    );
    let text = fs::read_to_string(&idx).unwrap();
    assert!(text.contains("async function xQ(e,t){return;"));
    assert!(text.contains("showPaywall(e,t){return Promise.resolve()}"));
    assert!(text.contains("(0,Wq.r8)()||!0||op().checkForUpdates()"));

    // 真·结构变化（锚点彻底不存在）只让该条失败，其余照打
    let broken = pristine.replace("showPaywall(e,t){return vL(e,t)}", "showPaywallV2(){}");
    fs::write(&idx, &broken).unwrap();
    let fifth = apply(&idx, &on).unwrap();
    assert!(fifth.failed.iter().any(|f| f.contains("showPaywall")));
    assert!(!fifth.failed.iter().any(|f| f.contains("自动更新")));
    let text = fs::read_to_string(&idx).unwrap();
    assert!(text.contains("(0,M.j9)()||!0||op().checkForUpdates()"));

    let _ = fs::remove_dir_all(&dir);
}

/// 自检报告不能只说「补丁不完整」——用户拿着这句话无从下手。
/// 这里确认它能把没生效的补丁点名到具体功能。
#[test]
fn failing_patches_names_what_did_not_apply() {
    let dir = std::env::temp_dir().join("tvx-patch-failing-test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let idx = dir.join("index.js");

    let promo_name = PATCHES
        .iter()
        .find(|p| p.new.contains("__tvxGuard"))
        .map(|p| p.name)
        .expect("页内推广浮层补丁应在补丁集里");
    let pristine: String = PATCHES
        .iter()
        .filter(|p| p.name != promo_name)
        .map(|p| p.old)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&idx, &pristine).unwrap();

    let on = Flags::default();
    let before = failing_patches(&idx, &on);
    assert!(
        !before.is_empty(),
        "原始文件一条都没打，必须点名而不是只回一个 false"
    );
    // 点名要认得出来是哪条功能
    assert!(
        before.iter().any(|n| n.contains("多图表") || n.contains("绘图")),
        "点名应落到具体功能：{before:?}"
    );

    apply(&idx, &on).unwrap();
    assert!(
        failing_patches(&idx, &on).is_empty(),
        "全部打上之后不该再点名：{:?}",
        failing_patches(&idx, &on)
    );

    // 关掉某个开关后，该组补丁"必须在文件里不存在"才算通过，这里确认不会误报
    let off = Flags {
        adblock: false,
        ..Flags::default()
    };
    let _ = apply(&idx, &off).unwrap();
    assert!(failing_patches(&idx, &off).is_empty(), "关掉开关后不该误报");

    let _ = fs::remove_dir_all(&dir);
}
