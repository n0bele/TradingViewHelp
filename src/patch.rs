//! 补丁集：1) 不弹订阅框；2) 拦截第三方广告/追踪域名；3) 解锁多图表布局与指标叠加的
//! 客户端 gate；4) 禁用启动时的自动更新检查。
//!
//! 指标数量 / 布局保存 / 警报数量等限制由服务端强制（实测：指标走服务端拒绝码
//! `study_limit_exceeded`，警报提交走 REST `createAlert`、拒绝码
//! `total_alerts_count_exceeded` 等），客户端无法解除拒绝——只能屏蔽其充值窗口。
//! 多图表布局的图表数量 gate 在客户端（实测 `_exposed_chartWidgetCollection`
//! 上的 `_checkProFeature`/`_planChartsLimit`，free 档实测 limit=1），可注入解除；
//! 指标叠加（STUDY_ON_STUDY）的子指标上限同样 gate 在客户端：CDP 实测
//! `getConfig('STUDY_ON_STUDY')` 返回渲染进程内嵌计划配置表的共享对象
//! （free 档 limit:800 / child_limit:1），执法方 `canCreateStudy` 经
//! `featureLimit` 活读取该表，改写共享表后 featureLimit 实测联动放行——可注入解除。
//! 自动更新只有启动链一处自动触发（`tp().checkForUpdates()` 唯一入口；
//! 断链后 `appInstaller` 恒空，`setupUpdatesMonitoring` 双重保险使更新定时器
//! 永不建立），`checkForUpdatesAndNotify` 全 bundle 仅定义未调用；
//! 设置面板的手动检查/下载路径不受影响。

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

/// 补丁锚点所针对、并做过端到端实测验证的 TradingView Desktop 版本。
/// 锚点是 index.js 里的字面代码片段（压缩标识符随版本漂移，效果等同"偏移不同"），
/// 其他版本不保证命中——apply() 在失配时显式失败并指出是哪条补丁没对上，
/// 绝不静默打一半；界面（app.rs）会把该版本标注在副本状态旁。
pub const VERIFIED_VERSION: &str = "3.5.0.8375";

/// 用户可配置的功能开关（持久化在 tvx 的 config.json）。
/// 五项覆盖六条需求：广告拦截 / 多图表解锁 / 指标叠加解锁 / 禁用自动更新各自独立；
/// 指标数量 / 布局保存 / 警报由服务端强制（客户端无法解除拒绝），共用「屏蔽充值窗口」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    pub ads_block: bool,
    pub charts_unlock: bool,
    pub study_on_study: bool,
    pub silence_paywalls: bool,
    pub no_auto_update: bool,
    pub auth_route: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            ads_block: true,
            charts_unlock: true,
            study_on_study: true,
            silence_paywalls: true,
            no_auto_update: true,
            auth_route: true,
        }
    }
}

/// 补丁所属的功能组，决定它受哪个开关控制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// 充值窗口屏蔽（涵盖指标数量、布局保存、警报的服务端配额弹窗）
    Silence,
    /// 广告/追踪域名网络拦截
    Ads,
    /// 多图表布局客户端 gate 解锁
    Charts,
    /// 指标叠加（STUDY_ON_STUDY）客户端 gate 解锁
    StudyOnStudy,
    /// 禁用启动时的自动更新检查
    NoAutoUpdate,
    /// 登录回调直达：每个实例把 `--user-data-dir` 写进 tradingview:// 注册，
    /// 授权回调才会回到"发起登录的那个实例"（否则 state 不匹配，新授权被丢弃）
    Auth,
}

impl Group {
    fn enabled(self, f: &Flags) -> bool {
        match self {
            Group::Silence => f.silence_paywalls,
            Group::Ads => f.ads_block,
            Group::Charts => f.charts_unlock,
            Group::StudyOnStudy => f.study_on_study,
            Group::NoAutoUpdate => f.no_auto_update,
            Group::Auth => f.auth_route,
        }
    }
}

pub struct Patch {
    pub name: &'static str,
    /// 英文名（日志在 English 界面下用）
    pub name_en: &'static str,
    pub group: Group,
    pub old: &'static str,
    pub new: &'static str,
}

/// 按中文名查英文名（日志用）；查不到时退回占位。
pub fn name_en(name: &str) -> &'static str {
    PATCHES
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.name_en)
        .unwrap_or("patch")
}

/// 注入到图表页（dom-ready 后轮询执行）的多图表解锁脚本。
/// 拦截对象经运行时实测确认：window._exposed_chartWidgetCollection
/// 是壳层与页面既有共享入口（PlatformProxy 同样引用），
/// `_checkProFeature(e)` 原实现命中限制时弹 PAYWALL_MULTIPLE_CHARTS 并钳制布局，
/// 实例级改写为恒放行后，仅作用于本工具托管的页面。
/// 常量仅在测试里用于逐字漂移检查（正式文本内嵌在补丁 new 里）。
#[cfg(test)]
const MULTI_CHART_UNLOCK: &str = "(function(){try{var n=0;(function p(){var c=window._exposed_chartWidgetCollection;if(c&&'function'==typeof c._checkProFeature){c._checkProFeature=function(e){return e};c._planChartsLimit=function(){return 99}}else{n++<120&&setTimeout(p,1e3)}})()}catch(e){}})();";

/// 注入到图表页（dom-ready 后轮询执行）的指标叠加解锁脚本。
/// 实测链路（CDP、登录态 free 账号）：计划配置模块在 bundle 内嵌各计划的特征配额表
/// （free 档 `STUDY_ON_STUDY:{limit:800,child_limit:1}`），`getConfig(feature)` 每次返回
/// 该表的共享对象；执法方 `canCreateStudy`（子指标计数 >= child_limit 即拒）与注册到
/// 196712 的 `featureLimit` 闭包都在调用现场活读取 `getConfig`，因此改写共享表
/// `child_limit → 99` 后，`featureLimit` 实测联动返回 99，免费档实际不可能再触发拒绝；
/// `PAYWALL_STUDY_ON_STUDY` 弹窗分支在其失败下游，随之不可达（屏蔽补丁再兜一层）。
/// 配置模块的 webpack id 随版本变化，这里按「工厂源码含 child_limit + getConfig:」
/// 动态定位（实测全 bundle 仅该配置模块同时满足），不硬编码 id。探针验证整串可执行。
/// 常量仅在测试里用于逐字漂移检查（正式文本内嵌在补丁 new 里）。
#[cfg(test)]
const STUDY_ON_STUDY_UNLOCK: &str = "(function(){try{var n=0;(function p(){var ok=!1;try{var c=Object.keys(window).filter(function(k){return /webpackChunk/i.test(k)});if(c.length){var rq=null;window[c[0]].push([[Date.now()%1e6],{},function(r){rq=r}]);if(typeof rq=='function'){for(var ids=Object.keys(rq.m),i=0;i<ids.length;i++){var s=String(rq.m[ids[i]]);if(!(s.indexOf('child_limit')>=0&&s.indexOf('getConfig:')>=0))continue;var m=rq(ids[i]);if(!m||typeof m.getConfig!='function')continue;var f=m.getConfig('STUDY_ON_STUDY');if(f&&typeof f=='object'){f.child_limit=99;ok=!0;break}}}}}catch(e){}if(!(ok||n>120)){n++;setTimeout(p,1e3)}})()}catch(e){}})();";

pub const PATCHES: &[Patch] = &[
    Patch {
        name: "订阅遮罩窗口不再创建",
        name_en: "Paywall overlay window not created",
        group: Group::Silence,
        old: "async function nk(e,t){const n=(0,Pe.Qh)().getLastFocusedWindow();",
        new: "async function nk(e,t){return;const n=(0,Pe.Qh)().getLastFocusedWindow();",
    },
    Patch {
        name: "showPaywall 变空操作",
        name_en: "showPaywall becomes a no-op",
        group: Group::Silence,
        old: "showPaywall(e,t){return nk(e,t)}",
        new: "showPaywall(e,t){return Promise.resolve()}",
    },
    Patch {
        name: "网页内充值窗口不再注入",
        name_en: "In-page billing popup not injected",
        group: Group::Silence,
        old: "async requestProFeature(){const e=(0,Pe.Qh)().getLastFocusedWindow();",
        new: "async requestProFeature(){return;const e=(0,Pe.Qh)().getLastFocusedWindow();",
    },
    Patch {
        name: "页面请求开窗时拦截充值/推广链接",
        name_en: "Block promo/pricing windows requested by the page",
        group: Group::Silence,
        // 页面（web app）可以通过 IPC OPEN_NEW_WINDOW 让壳开任意 URL 的新窗口，
        // 这条路径之前没有任何限制——定时弹出的"充值窗口"就是从这里来的。
        // 只拦 URL 命中充值/推广关键字的请求，正常业务开窗（K 线弹窗、分享等）不受影响。
        old: "null==n||!1===(0,M.HU)(n))return;const o={url:n};",
        new: "null==n||!1===(0,M.HU)(n))return;if(/(pricing|subscribe|subscription|billing|upgrade|go-?pro|promo|trial)/i.test(String(n)))return;const o={url:n};",
    },
    Patch {
        name: "广告/追踪域名网络拦截",
        name_en: "Ad/tracker domain blocking",
        group: Group::Ads,
        // 挂在既有 initSession 的 webRequest 配置处（onBeforeSendHeaders 之后，
        // will-download 注册之前）。实测当前图表页无广告 DOM，此为预防性网络层拦截，
        // 命中第三方广告网络（Google Ads 系/Taboola/Outbrain 等常见买量网络）时直接取消。
        old: "r({cancel:!1,requestHeaders:e.requestHeaders})})),e.on(\"will-download\",$u)",
        new: "r({cancel:!1,requestHeaders:e.requestHeaders})})),e.webRequest.onBeforeRequest(((e,r)=>{r({cancel:/(doubleclick\\.net|googlesyndication\\.com|googleadservices\\.com|googletagservices\\.com|adservice\\.google\\.|taboola\\.com|outbrain\\.com|amazon-adsystem\\.com|adsrvr\\.org|rubiconproject\\.com)/.test(e.url)})})),e.on(\"will-download\",$u)",
    },
    Patch {
        name: "多图表布局解锁注入（客户端 gate）",
        name_en: "Multi-chart unlock injection (client gate)",
        group: Group::Charts,
        // 挂在 dom-ready 非本地文件处理处：紧随 app-region CSS 注入之后、executeTasks 之前，
        // 轮询等待页面创建图表集合实例后做实例级改写（每次 dom-ready 重新注入）。
        // old/new 尾部都带 executeTasks()，保证 old 不是 new 的子串（可重入判定依赖这点）。
        old: "(0,M.uY)(this.browserView.webContents.insertCSS(\"html{ -webkit-app-region: none !important;}\")),this.executeTasks()",
        new: "(0,M.uY)(this.browserView.webContents.insertCSS(\"html{ -webkit-app-region: none !important;}\")),this.executeJavaScript(\"(function(){try{var n=0;(function p(){var c=window._exposed_chartWidgetCollection;if(c&&'function'==typeof c._checkProFeature){c._checkProFeature=function(e){return e};c._planChartsLimit=function(){return 99}}else{n++<120&&setTimeout(p,1e3)}})()}catch(e){}})();\").catch((()=>{})),this.executeTasks()",
    },
    Patch {
        name: "指标叠加解锁注入（客户端 gate）",
        name_en: "Study-on-study unlock injection (client gate)",
        group: Group::StudyOnStudy,
        // 挂在 dom-ready 非本地文件处理语句链的最前面（旧体首语句之前前插一条
        // executeJavaScript），与「多图表」补丁同通道、不同锚点，互不依赖、可独立开关。
        // 注入脚本按源码特征动态定位计划配置模块并改写共享表 child_limit → 99
        //（整串已在登录态真实页面实弹验证：getConfig/featureLimit 同步联动返回 99）。
        old: "handleDomReadyForNonLocalFiles(){(0,u.Ow)().debug(",
        new: "handleDomReadyForNonLocalFiles(){this.executeJavaScript(\"(function(){try{var n=0;(function p(){var ok=!1;try{var c=Object.keys(window).filter(function(k){return /webpackChunk/i.test(k)});if(c.length){var rq=null;window[c[0]].push([[Date.now()%1e6],{},function(r){rq=r}]);if(typeof rq=='function'){for(var ids=Object.keys(rq.m),i=0;i<ids.length;i++){var s=String(rq.m[ids[i]]);if(!(s.indexOf('child_limit')>=0&&s.indexOf('getConfig:')>=0))continue;var m=rq(ids[i]);if(!m||typeof m.getConfig!='function')continue;var f=m.getConfig('STUDY_ON_STUDY');if(f&&typeof f=='object'){f.child_limit=99;ok=!0;break}}}}}catch(e){}if(!(ok||n>120)){n++;setTimeout(p,1e3)}})()}catch(e){}})();\").catch((()=>{})),(0,u.Ow)().debug(",
    },
    Patch {
        name: "登录回调直达发起实例",
        name_en: "Route auth callback back to the initiating instance",
        group: Group::Auth,
        // 授权回调靠 tradingview:// 拉起进程。系统注册里没有 --user-data-dir 时，
        // 回调会落到"默认配置目录"的实例；那个实例没有本次登录的 state（随机 key），
        // acceptRedirectUrl 会直接丢弃，新授权就白拿了。
        // 这里让每个实例启动时把自己的 --user-data-dir 一并注册进去。
        old: "i.app.isDefaultProtocolClient(\"tradingview\")||i.app.setAsDefaultProtocolClient(\"tradingview\")",
        new: "i.app.setAsDefaultProtocolClient(\"tradingview\",process.execPath,process.argv.filter(e=>e.startsWith(\"--user-data-dir=\")))",
    },
    Patch {
        name: "记录浏览器授权 URL（诊断）",
        name_en: "Log the browser auth URL (diagnostic)",
        group: Group::Auth,
        // 登录走浏览器授权时，把生成的 URL 写进实例日志，
        // 便于确认浏览器到底打开了什么、有没有走完回调。
        old: "async signInWithBrowser(){const e=this.createAuthenticationUrl();this.authStateSubject.next({stage:CE.xE.WaitingToken})",
        new: "async signInWithBrowser(){const e=this.createAuthenticationUrl();console.log(\"TVX_AUTH_URL \"+e);this.authStateSubject.next({stage:CE.xE.WaitingToken})",
    },
    Patch {
        name: "登录改由助手打开无痕窗口",
        name_en: "Let TVHelper open the login page in a private window",
        group: Group::Auth,
        // 默认浏览器里通常已经登录了别的账号，授权只会拿回旧账号。
        // 这里不再让应用调系统默认浏览器：URL 会打进实例日志，助手读到后
        // 用无痕窗口打开（干净会话），用户直接登新账号即可。
        old: ":i.shell.openExternal(e))}catch(e){",
        new: ":0)}catch(e){",
    },
    Patch {
        name: "隐藏站点「连接已关闭」弹窗",
        name_en: "Hide the site's 'connection closed' dialog",
        group: Group::Silence,
        // 该弹窗由网页下发（本地包里没有这段文案），触发原因是服务器判定同账号并发超限。
        // 这里在页面里挂一个观察器，把这类弹窗直接藏掉，避免打断看盘。
        old: "handleDomReady(){this.isBrowserViewValid()&&(this.url&&",
        new: "handleDomReady(){this.isBrowserViewValid()&&(this.executeJavaScript(\"(function(){try{if(window.__tvxNoDisconnect)return;window.__tvxNoDisconnect=1;var K=/恢复连接|获取更多连接|关闭了这个连接|We closed this connection|Restore connection|Get more connections/i;function s(){var n=document.querySelectorAll('[role=dialog],[data-name*=dialog],[class*=dialog]');for(var i=0;i<n.length;i++){var e=n[i],t=e.textContent||'';if(t.length<600&&K.test(t)){e.style.display='none';e.setAttribute('data-tvx-hidden','1')}}}setInterval(s,800)}catch(e){}})();\").catch((()=>{})),this.url&&",
    },
    Patch {
        name: "放宽离线判定超时",
        name_en: "Relax the offline-detection timeout",
        group: Group::Silence,
        // 应用用一个 1 秒超时的 ping（data.tradingview.com/ping）判定在线/离线。
        // 走代理时一次握手就可能超过 1 秒，于是被误判成"离线"（并触发重连弹窗）。
        // 这里把默认超时从 1s 放宽到 10s。
        old: "he=1e3,",
        new: "he=1e4,",
    },
    Patch {
        name: "启动自动更新检查禁用",
        name_en: "Startup auto-update check disabled",
        group: Group::NoAutoUpdate,
        // 启动链里唯一一次自动检查（逗号表达式链的末位语句，结果值被丢弃）。
        // 插入恒真短路后 checkForUpdates 在任何平台都不会执行：appInstaller 永不填充，
        // setupUpdatesMonitoring 双重保险（if(!this.appInstaller)return）使更新定时器
        // 永不建立；设置面板手动检查/下载路径不受影响。Linux 官方原本就跳过自动更新。
        old: "(0,M.j9)()||tp().checkForUpdates()",
        new: "(0,M.j9)()||!0||tp().checkForUpdates()",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 本次打上
    Applied,
    /// 之前已打过，未动
    AlreadyPatched,
    /// 开关已关闭，把之前打的补丁撤回
    Reverted,
    /// 开关关闭且文件里本来就没有，无需动
    Untouched,
}

impl Status {
    pub fn mark(self) -> &'static str {
        match self {
            Status::Applied => "+",
            Status::AlreadyPatched => "=",
            Status::Reverted => "-",
            Status::Untouched => "·",
        }
    }
}

/// 按开关对解包后的 index.js 应用/撤回补丁。任一处匹配数不为 1 就整体失败，
/// 避免"打了一半"的半残状态。
///
/// 语义：
/// - 开关开 + 未打过 → 打上（Applied）
/// - 开关开 + 已打过 → 不动（AlreadyPatched）
/// - 开关关 + 已打过 → 撤回 new→old（Reverted）
/// - 开关关 + 未打过  → 不动（Untouched）
pub fn apply(index_js: &Path, flags: &Flags) -> Result<Vec<(&'static str, Status)>> {
    let bytes = fs::read(index_js).with_context(|| format!("读取 {}", index_js.display()))?;
    let mut src = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("index.js 不是合法 UTF-8，拒绝改写"))?;

    let mut report = Vec::new();
    let mut failures = Vec::new();

    for p in PATCHES {
        if !p.group.enabled(flags) {
            if src.contains(p.new) {
                match src.matches(p.new).count() {
                    1 => {
                        src = src.replacen(p.new, p.old, 1);
                        report.push((p.name, Status::Reverted));
                    }
                    n => failures.push(format!(
                        "{}：撤回时期望 new 1 处匹配，实际 {n} 处",
                        p.name
                    )),
                }
            } else {
                report.push((p.name, Status::Untouched));
            }
            continue;
        }

        if src.contains(p.new) && !src.contains(p.old) {
            report.push((p.name, Status::AlreadyPatched));
            continue;
        }
        match src.matches(p.old).count() {
            1 => {
                src = src.replacen(p.old, p.new, 1);
                report.push((p.name, Status::Applied));
            }
            n => failures.push(format!("{}：期望 1 处匹配，实际 {n} 处", p.name)),
        }
    }

    if !failures.is_empty() {
        bail!(
            "补丁未应用（{} 处锚点未命中）。补丁锚点按版本 {} 的 index.js 提取，\
             其他版本的字面结构可能已变化——请更新 TVHelper 的补丁集或手动核对：\n  {}",
            failures.len(),
            VERIFIED_VERSION,
            failures.join("\n  ")
        );
    }

    fs::write(index_js, src.as_bytes())
        .with_context(|| format!("写回 {}", index_js.display()))?;
    Ok(report)
}

/// 检查 index.js 是否处于"当前开关组合所要求"的已打补丁状态：
/// 开着的补丁必须都在，关着的补丁必须都不在。
pub fn is_patched(index_js: &Path, flags: &Flags) -> bool {
    match fs::read_to_string(index_js) {
        Ok(s) => PATCHES.iter().all(|p| {
            let has_new = s.contains(p.new);
            let has_old = s.contains(p.old);
            if p.group.enabled(flags) {
                has_new && !has_old
            } else {
                !has_new
            }
        }),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
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
            .find(|p| p.name.contains("多图表"))
            .expect("多图表补丁应在补丁集里");
        assert!(b.new.contains(MULTI_CHART_UNLOCK));
    }

    #[test]
    fn study_on_study_injection_matches_const() {
        // 同上：指标叠加注入串常量与补丁字面文本逐字一致
        let s = PATCHES
            .iter()
            .find(|p| p.name.contains("指标叠加"))
            .expect("指标叠加补丁应在补丁集里");
        assert!(s.new.contains(STUDY_ON_STUDY_UNLOCK));
        // 注入串里不得出现双引号：它整体包在外层 executeJavaScript("...") 里
        assert!(!STUDY_ON_STUDY_UNLOCK.contains('"'));
    }

    /// 用一段含真实 old 锚点的临时文件，验证开关矩阵下的
    /// 打上 / 撤回 / 幂等行为。每个组抽一个代表补丁（取其 old/new 对）。
    #[test]
    fn apply_respects_flags_and_is_reversible() {
        let dir = std::env::temp_dir().join("tvx-patch-flag-test");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("index.js");

        // pristine = 全部补丁 old 依序拼接（apply 按组应用组内全部补丁，
        // 所以每个 old 都必须恰好出现一次；old 之间互不包含已由
        // patches_are_self_consistent 覆盖）
        let pristine: String = PATCHES.iter().map(|p| p.old).collect();

        // 五个组各取代表补丁，便于后半段断言
        let silence = PATCHES.iter().find(|p| p.group == Group::Silence).unwrap();
        let ads = PATCHES.iter().find(|p| p.group == Group::Ads).unwrap();
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
            .iter()
            .all(|(_, st)| *st == Status::Applied || *st == Status::AlreadyPatched));
        assert!(read().contains(ads.new));
        assert!(read().contains(charts.new));
        assert!(read().contains(silence.new));
        assert!(read().contains(study.new));
        assert!(read().contains(noauto.new));

        // 2) 全关：把刚打的全部撤回，文件应回到逐字 pristine
        let all_off = Flags {
            ads_block: false,
            charts_unlock: false,
            study_on_study: false,
            silence_paywalls: false,
            no_auto_update: false,
            auth_route: false,
        };
        write_pristine(&read()); // 当前是已打状态，先按现状
        let report = apply(&f, &all_off).unwrap();
        assert!(report
            .iter()
            .all(|(_, st)| *st == Status::Reverted || *st == Status::Untouched));
        assert_eq!(read(), pristine, "撤回后应与 pristine 逐字一致");
        assert!(is_patched(&f, &all_off), "全关状态下 is_patched 应为真");

        // 3) 部分开关：只开图表解锁 —— 其余四组未启用，charts 打上
        write_pristine(&read()); // 已回到 pristine
        let only_charts = Flags {
            ads_block: false,
            charts_unlock: true,
            study_on_study: false,
            silence_paywalls: false,
            no_auto_update: false,
            auth_route: false,
        };
        apply(&f, &only_charts).unwrap();
        let now = read();
        assert!(now.contains(charts.new));
        for p in PATCHES.iter().filter(|p| p.group != Group::Charts) {
            assert!(!now.contains(p.new), "{} 不应被打上", p.name);
        }
        assert!(is_patched(&f, &only_charts));

        // 4) 幂等：同配置重跑应全部 AlreadyPatched/Untouched 且文件不变
        let before = read();
        let report = apply(&f, &only_charts).unwrap();
        assert!(report.iter().all(|(_, st)| matches!(
            st,
            Status::AlreadyPatched | Status::Untouched
        )));
        assert_eq!(read(), before);

        let _ = fs::remove_dir_all(&dir);
    }
}
