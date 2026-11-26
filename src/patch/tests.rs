//! 补丁引擎的回归测试：锚点自洽、旧副本按段迁移、开关可逆、失配降级。
//!
//! 这些用例直接对着 `super::` 的私有函数跑，故意不经过公开 API——
//! 出问题时它们指向的是"哪一段没对上"，而不是"整体失败"。

use super::*;

/// 2026-11-26 下线的「页内推广浮层」注入段的历史形态，只用于迁移回归：
/// 它曾被填进 Core 壳的空位，形态是「handleDomReady + executeJavaScript(脚本) + catch」。
/// 现在这一段必须能被整段认出来并收回到壳，否则老副本升级后旧脚本还在页面上跑。
const LEGACY_PROMO_SEG: &str = "t.handleDomReady(),void t.executeJavaScript(\"(function(){window.__tvxPromo=1})();\").catch((()=>{}))";

/// 原样 index.js：把每条补丁的 `old` 依序拼起来。apply() 按组应用组内全部补丁，
/// 所以每个 old 都必须恰好出现一次。
fn pristine_source() -> String {
    PATCHES.iter().map(|p| p.old).collect::<Vec<_>>().join("\n")
}

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

/// Core 壳的形状与重入判定：壳恒开、留出注入空位、靠代次令牌去重。
///
/// 2026-11-26 起页内推广浮层的注入脚本整段下线（靠文案/面积猜 DOM 去隐藏，
/// 误伤远大于收益），这里只钉壳本身，不再钉注入文本。
#[test]
fn core_shell_shape() {
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
        backtest_unlock: false,
        no_auto_update: false,
        auto_recover: false,
        alerts: false,
    };
    assert!(Group::Core.enabled(&all_off), "Core 必须恒开");
}

/// 拦截只允许发生在「壳层补丁 + 网络层 + 能力表」，不允许再有
/// 「按文案/面积隐藏已渲染 DOM」的注入——那正是误伤用户手动打开界面的根源。
#[test]
fn no_dom_hiding_injection_remains() {
    for p in PATCHES {
        let body = format!("{}{}", p.old, p.new);
        for banned in [
            "display','none'",
            "MutationObserver",
            "data-tvx-hidden",
            "__tvxGuard",
        ] {
            assert!(
                !body.contains(banned),
                "补丁「{}」里仍有 DOM 隐藏逻辑（{banned}）",
                p.name
            );
        }
    }
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

    // 模拟 v3 初版：壳 + 旧注入段耦合在同一个回调里
    let stale = DOM_READY_SHELL.replacen(SHELL_HOOK, LEGACY_PROMO_SEG, 1);
    assert_ne!(stale, DOM_READY_SHELL, "模拟的历史段不能与当前壳相同");

    // 旧副本的形态：Core 的位置放耦合段
    let src: String = PATCHES
        .iter()
        .map(|q| {
            if q.group == Group::Core {
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
    assert!(after.contains(SHELL_HOOK), "注入段应收回到壳的空位");
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

/// 真实 index.js 里 `.on("dom-ready",…)` 后面直接链式挂 `.on("did-navigate-in-page",…)`，
/// 段尾下一个字节是 `.` 而不是换行。边界校验不认 `.` 时整段迁移被拒，老副本升级注入
/// 会一直报"锚点未命中"，只能整份重建副本（2026-11-26 在真机副本上撞到）。
#[test]
fn promo_segment_tail_followed_by_chained_on_is_migrated() {
    let dir = std::env::temp_dir().join("tvx-patch-promo-chained-test");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("index.js");

    let stale = DOM_READY_SHELL.replacen(SHELL_HOOK, LEGACY_PROMO_SEG, 1);
    // 关键：段尾不换行，直接接下一个链式 `.on(…)`
    let mut src = format!("{}.on(\"did-navigate-in-page\",(()=>{{}}))", stale);
    for q in PATCHES {
        if q.group != Group::Core {
            src.push('\n');
            src.push_str(q.new);
        }
    }
    fs::write(&f, &src).unwrap();

    let report = apply(&f, &Flags::default()).unwrap();
    assert!(report.failed.is_empty(), "链式段尾也应能迁移：{:?}", report.failed);
    let after = fs::read_to_string(&f).unwrap();
    assert!(!after.contains("__tvxPromo"), "旧脚本应被整段换掉");
    assert!(after.contains(SHELL_HOOK), "注入段应收回到壳的空位");
    assert!(after.contains(".on(\"did-navigate-in-page\""), "链式尾巴不能被切掉");

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

    fs::write(&f, pristine_source()).unwrap();

    let off = Flags {
        adblock: false,
        ..Flags::default()
    };
    apply(&f, &off).unwrap();
    let after = fs::read_to_string(&f).unwrap();

    assert!(core_shell_present(&after), "关掉推广后重试壳必须还在");
    assert!(after.contains(SHELL_HOOK), "壳里应留空位");
    // 页内推广注入 2026-11-26 已下线：任何开关下都不应再有它
    assert!(!after.contains("__tvxPromo"), "不应残留推广注入");
    // 其余注入照常（它们都排在壳后面，靠壳等视图就绪）
    for mark in ["__tvxRecover", "__tvxDraw", "_exposed_chartWidgetCollection"] {
        assert!(after.contains(mark), "关掉推广不应影响 {mark} 注入");
    }
    assert!(is_patched(&f, &off), "is_patched 应认为当前状态正确");

    // 再打开：壳仍只有一处，且不会冒出注入脚本
    apply(&f, &Flags::default()).unwrap();
    let back = fs::read_to_string(&f).unwrap();
    assert!(!back.contains("__tvxPromo"));
    assert!(back.contains(SHELL_HOOK), "空位必须一直保留");
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

    // 显式拼接，避免 format! 的 `}}` 转义把收尾括号算错一位
    let mut v2 = String::new();
    v2.push_str(PROMO_SEG_HEAD_V2);
    v2.push_str(LEGACY_PROMO_SEG);
    v2.push_str(PROMO_SEG_END_MARK);
    v2.push_str("}))");

    // 旧副本形态：Core 的位置放 v2 段
    let src: String = PATCHES
        .iter()
        .map(|q| {
            if q.group == Group::Core {
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
    assert!(!after.contains("__tvxPromo"), "旧注入段必须被丢掉");
    assert!(after.contains(SHELL_HOOK), "壳里应留空位");
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
    let pristine: String = pristine_source();

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
        backtest_unlock: false,
        no_auto_update: false,
        auto_recover: false,
        alerts: false,
    };
    // 只有 Auth 组生效时的目标文本
    let keep_auth: String = PATCHES
        .iter()
        .map(|p| {
            if p.group == Group::Auth || p.group == Group::Core {
                p.new
            } else {
                p.old
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
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
        backtest_unlock: false,
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
    fs::write(&idx, pristine_source()).unwrap();

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
        backtest_unlock: false,
        no_auto_update: false,
        auto_recover: false,
        alerts: false,
    };
    let third = apply(&idx, &off).unwrap();
    assert!(third.failed.is_empty(), "撤回不应失败：{:?}", third.failed);

    // 模拟上游重打包：压缩标识符全改名，字面锚点失配，通配锚点应接住
    let pristine = pristine_source();
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

    fs::write(&idx, pristine_source()).unwrap();

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

/// 回测增强：注入串与补丁字面文本逐字一致；补的是**付费档同形的空对象**，
/// 且不含 `DEEP_HISTORY_BACKTEST`——深回测走独立 WebSocket、由服务端签发数据，
/// 客户端把 flag 打开只会得到「按钮能点、点了报错」，比不解锁更糟（实测见补丁注释）。
#[test]
fn backtest_injection_matches_const() {
    let p = PATCHES
        .iter()
        .find(|q| q.group == Group::Backtest)
        .expect("回测增强补丁应在补丁集里");
    assert!(p.new.contains(BACKTEST_UNLOCK));
    assert!(!BACKTEST_UNLOCK.contains('"'), "整串嵌在 executeJavaScript(\"…\") 里");
    // 只有 `BACKTESTING_EXTENDED_METRICS` 真正对用户可见（14 个报告字段从锁变成数值）；
    // 导出那两个 key 判定能翻但界面没按钮，2026-11-26 起不再补。
    assert!(BACKTEST_UNLOCK.contains("BACKTESTING_EXTENDED_METRICS"));
    for k in [
        "BACKTESTING_EXPORT_CSV",
        "BACKTESTING_EXPORT_XLSX",
        "BAR_DETALIZATION",
        "SCRIPT_EXECUTION_ON_HISTORY_BARS",
        "DEEP_HISTORY_BACKTEST",
    ] {
        assert!(
            !BACKTEST_UNLOCK.contains(k),
            "{k} 没有用户可见效果或由服务端强制，不能由客户端打开"
        );
    }
    // 值必须是空对象：这是付费档在套餐表里的原样，不是"随手一个真值"
    assert!(BACKTEST_UNLOCK.contains("if(!f[FE[j]])f[FE[j]]={}"));
    // free 档没有 extends，getProductFeaturesConfig 返回的就是表本身
    assert!(BACKTEST_UNLOCK.contains("getProductFeaturesConfig('free')"));
    // 锚点不得与其他补丁重复（重复会让先打上的那条吃掉另一条的锚点）
    for other in PATCHES.iter().filter(|q| q.name != p.name) {
        assert_ne!(other.old, p.old, "回测增强锚点与 {} 重复", other.name);
    }
}

/// 开关可逆：打上后同链上的其余注入都还在，关掉后回到精确原文、不残留。
#[test]
fn backtest_patch_applies_and_reverts() {
    let dir = std::env::temp_dir().join("tvx-patch-backtest-test");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join("index.js");
    // 合成"未打补丁"的 index.js：取每条补丁的 old。
    // 推广浮层那条的锚点是 Core 壳里的空位（SHELL_HOOK），原文里本来没有，
    // 它由 Core 打上后才出现——不能一起拼进来，否则会变成两处。
    let src: String = PATCHES
        .iter()
        .filter(|q| q.old != SHELL_HOOK)
        .map(|q| q.old)
        .collect();
    assert_eq!(src.matches("this.setupHeapLog(),").count(), 1);
    fs::write(&f, &src).unwrap();

    let on = Flags::default();
    let report = apply(&f, &on).unwrap();
    assert!(report.failed.is_empty(), "不应有补丁失配：{:?}", report.failed);
    assert!(is_patched(&f, &on));
    let after = fs::read_to_string(&f).unwrap();
    // 脚本只落一份：__tvxBT 恰好三处（守卫、置 1、置 2）
    assert_eq!(after.matches("__tvxBT").count(), 3, "注入脚本只应有一份");
    // 同链上其余注入不受影响（页内推广注入已下线，这里换断线自愈当哨兵）
    for marker in ["__tvxDraw", "__tvxRecover"] {
        assert!(after.contains(marker), "{marker} 被挤掉了");
    }

    let off = Flags {
        backtest_unlock: false,
        ..Flags::default()
    };
    apply(&f, &off).unwrap();
    assert!(is_patched(&f, &off));
    let reverted = fs::read_to_string(&f).unwrap();
    assert!(!reverted.contains("__tvxBT"), "关掉后不应残留注入");
    assert!(reverted.contains("this.jsExecutionTimeouts.push(n,o),"), "锚点应原样还原");

    let _ = fs::remove_dir_all(&dir);
}

/// 从 `new` 里切出正则原文：补丁把判定写成内联正则，注释写在别处，
/// 只有取原文跑一遍才能证明"没误拦"是真的，而不是注释里写的。
fn slice_between(hay: &str, open: &str, close: &str) -> String {
    let start = hay
        .find(open)
        .unwrap_or_else(|| panic!("找不到起始标记 {open}"))
        + open.len();
    let len = hay[start..]
        .find(close)
        .unwrap_or_else(|| panic!("找不到结束标记 {close}"));
    hay[start..start + len].to_string()
}

/// 误拦回归（用户对上一版的判词是"方法不科学、误拦截太多"）。
///
/// 现在只剩两条正则判定：页面请求开窗时的 URL 过滤、网络层的请求域名过滤。
/// 这里把两条正则**原文**从补丁表里取出来，交给同一个 JS 引擎跑真实语料：
/// 正常业务流量必须全部放行，付费墙/广告必须全部拦下。
/// 语料改坏、正则改宽，这条用例都会红。
#[test]
fn adblock_layers_only_block_what_they_claim() {
    let promo = PATCHES
        .iter()
        .find(|p| p.name.contains("充值/推广链接"))
        .expect("开窗过滤补丁不见了");
    let promo_src = slice_between(promo.new, "if(/", "/i.test(");
    let ads = PATCHES
        .iter()
        .find(|p| p.new.contains("onBeforeRequest"))
        .expect("广告域名补丁不见了");
    let ads_src = slice_between(ads.new, "cancel:/", "/.test(");

    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("跳过误拦回归：本机没有 node");
        return;
    }

    let dir = std::env::temp_dir().join("tvx-adblock-corpus");
    fs::create_dir_all(&dir).unwrap();
    let js = dir.join("corpus.js");
    fs::write(&js, ADBLOCK_CORPUS_JS).unwrap();

    let out = std::process::Command::new("node")
        .arg(&js)
        .arg(&promo_src)
        .arg(&ads_src)
        .output()
        .expect("node 启动失败");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "误拦/漏拦：\n{stdout}\n{stderr}"
    );
    assert!(stdout.contains("ok"), "语料没跑完：{stdout}{stderr}");
    let _ = fs::remove_dir_all(&dir);
}

/// 语料：左边是"必须放行"的正常流量，右边是"必须拦住"的付费墙/广告。
const ADBLOCK_CORPUS_JS: &str = r#"
const [, , pSrc, aSrc] = process.argv;
// 用 eval 拼回**字面量**而不是 new RegExp：new RegExp 里 `/` 不需要转义，
// 于是「把 `/` 写进 /…/ 却没转义」这种致命错误在测试里永远绿（实测踩过：2026-11-27
// 副本 index.js 出现 SyntaxError: Invalid regular expression flags，客户端起不来，
// 而这条用例照样通过）。eval 走的是解析器，多一个裸 / 直接抛 SyntaxError。
const P = eval('/' + pSrc + '/i');
const A = eval('/' + aSrc + '/');

// 页面请求开窗：真实会出现的正常窗口（分享、社区、帮助、行情页、登录、OAuth 回跳）
const winPass = [
  'https://cn.tradingview.com/chart/GOLD/',
  'https://www.tradingview.com/chart/?symbol=NASDAQ%3AAAPL&interval=60',
  'https://www.tradingview.com/x/AbCdEf12/',
  'https://www.tradingview.com/u/someuser/',
  'https://www.tradingview.com/ideas/',
  'https://www.tradingview.com/scripts/editors-picks/',
  'https://www.tradingview.com/markets/stocks-usa/market-movers-gainers/',
  'https://www.tradingview.com/economic-calendar/',
  'https://www.tradingview.com/news/',
  'https://www.tradingview.com/brokers/',
  'https://www.tradingview.com/symbols/NASDAQ-AAPL/',
  'https://www.tradingview.com/pine-script-docs/',
  'https://www.tradingview.com/streams/',
  'https://www.tradingview.com/screener/',
  'https://www.tradingview.com/widget/',
  'https://www.tradingview.com/accounts/signin/',
  'https://www.tradingview.com/support/solutions/43000594236/',
  'https://www.tradingview.com/terms-of-use/',
  'https://www.tradingview.com/privacy-policy/',
  'https://cn.tradingview.com/hotlist/',
  'https://cn.tradingview.com/chart/AAPL/',
  'https://www.tradingview.com/desktop/',
  'file:///C:/Users/x/.tvx/apps/3.5.1.8500/resources/app/app/window/index.html'
];

// 该拦的：壳里两个付费墙常量 + 常见充值入口
const winBlock = [
  'https://www.tradingview.com/desktop/paywall/',
  'https://www.tradingview.com/desktop/paywall/upgrade/',
  'https://www.tradingview.com/pro-plans/features/',
  'https://www.tradingview.com/pricing/',
  'https://www.tradingview.com/accounts/subscribe/',
  'https://www.tradingview.com/billing/',
  'https://www.tradingview.com/promo/black-friday/',
  'https://www.tradingview.com/go-pro/?source=desktop'
];

// 正常网络请求：TradingView 自己的域名一个都不能被域名表命中
const netPass = [
  'https://data.tradingview.com/symbol',
  'https://pushstream.tradingview.com/message-pipe-ws/public',
  'https://news-mediator.tradingview.com/news-flow/v2/news',
  'https://s3-symbol-logo.tradingview.com/apple.svg',
  'https://static.tradingview.com/static/bundles/abc.js',
  'https://widgets.tradingview.com/widget-watchlist/',
  'https://www.tradingview.com/chart/',
  'https://accounts.tradingview.com/accounts/signin/',
  'https://www.google-analytics.com/g/collect',
  'https://fonts.gstatic.com/s/roboto/v30/abc.woff2',
  'https://www.googletagmanager.com/gtag/js',
  // 同一个域名下的正常接口不能跟着被拦（offers 是按路径精确命中的）
  'https://cn.tradingview.com/api/v1/symbols_list/colored/?source=web-tvd',
  'https://cn.tradingview.com/api/v1/study-templates'
];

// 广告/追踪网络：命中即取消
const netBlock = [
  'https://securepubads.g.doubleclick.net/tag/js/gpt.js',
  'https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js',
  'https://www.googleadservices.com/pagead/conversion.js',
  'https://www.googletagservices.com/tag/js/gpt.js',
  'https://adservice.google.com/adsid/integrator.js',
  'https://cdn.taboola.com/libtrc/tradingview/loader.js',
  'https://widgets.outbrain.com/outbrain.js',
  'https://s.amazon-adsystem.com/iu3?d=forester-id',
  'https://match.adsrvr.org/track/rid',
  'https://ads.rubiconproject.com/ad/1234',
  // 网页自己的推广活动数据：实测屏蔽后整屏 Black Friday 推广不再渲染
  'https://cn.tradingview.com/api/v1/offers/',
  'https://www.tradingview.com/api/v1/offers/?locale=zh'
];

const bad = [];
winPass.forEach((u) => { if (P.test(u)) bad.push('误拦开窗  ' + u); });
winBlock.forEach((u) => { if (!P.test(u)) bad.push('漏拦开窗  ' + u); });
netPass.forEach((u) => { if (A.test(u)) bad.push('误拦请求  ' + u); });
netBlock.forEach((u) => { if (!A.test(u)) bad.push('漏拦请求  ' + u); });
if (bad.length) { console.log(bad.join('\n')); process.exit(1); }
console.log('ok ' + (winPass.length + winBlock.length + netPass.length + netBlock.length) + ' 条语料');
"#;


/// 注入重试的门禁。实测事故（2026-11-26）：注入前永久占位、只在失败时释放，
/// 于是 executeJavaScript 卡住时所有重试都被 has() 挡回 —— 图表页开着两分钟没有注入，
/// 日志里连一条失败都没有。现在必须做到：先探页面状态、放弃时留日志、不留永久占位。
#[test]
fn alert_injection_retries_until_engine_is_present() {
    assert!(ALERTS_MAIN_JS.contains("const PROBE_JS"), "注入前应先探页面里的引擎对象");
    assert!(ALERTS_MAIN_JS.contains("inject present"), "引擎已在页面上时应封口");
    assert!(ALERTS_MAIN_JS.contains("inject give-up"), "放弃重试必须留日志");
    assert!(!ALERTS_MAIN_JS.contains("INJECTED"), "不能回到永久占位的老写法");
}


/// 落盘前的语法门禁本身要能判对：坏正则必须判 Err，好代码必须判 Ok。
/// 事故背景（2026-11-27）：补丁把 `/` 写进正则字面量却没转义，改完的 index.js
/// 解析失败 → 客户端一启动就弹 Error 退出，而 `_doctor` 仍报 17/17（它只看字符串在不在）。
#[test]
fn syntax_gate_rejects_unparsable_patch_output() {
    let dir = std::env::temp_dir().join("tvx-syntax-gate");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let fake = dir.join("index.js");

    let broken = "var re=/(a/b)/.test('x');";
    assert!(
        crate::patch::check_js_syntax(&fake, broken).is_err(),
        "坏正则必须被判为不可解析"
    );

    let good = "var re=/(a\\/b)/.test('x');";
    match crate::patch::check_js_syntax(&fake, good) {
        Ok(_) => {}
        Err(e) => panic!("正常代码被误判：{e}"),
    }
    assert!(!dir.join(".tvx-syntax-check.js").exists(), "语法检查的临时文件没清掉");
    let _ = fs::remove_dir_all(&dir);
}
