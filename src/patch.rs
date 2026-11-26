//! 补丁集：1) 不弹订阅框；2) 拦截第三方广告/追踪域名；3) 解锁多图表布局与指标叠加的
//! 客户端 gate；4) 禁用启动时的自动更新检查。
//!
//! 指标数量 / 布局保存 / 警报数量等限制由服务端强制（实测：指标走服务端拒绝码
//! `study_limit_exceeded`，警报提交走 REST `createAlert`、拒绝码
//! `total_alerts_count_exceeded` 等），客户端无法解除拒绝——只能屏蔽其充值窗口。
//! 「本地指标报警」(Group::Alerts) 是另一条路：不提交服务端报警，而是在图表页读
//! 图表自己算出的指标值做条件判断，命中后由主进程发本机通知/邮件；不占用也不绕过
//! 服务端报警配额，代价是软件关闭即停止监控。
//! 多图表布局的图表数量 gate 在客户端（实测 `_exposed_chartWidgetCollection`
//! 上的 `_checkProFeature`/`_planChartsLimit`，free 档实测 limit=1），可注入解除；
//! 单张图表的绘图数量（free 档 30）同样 gate 在客户端：图表页 bundle 里
//! `drawingsPerChartLimit()` 从计划配置表取 `DRAWINGS_ON_CHART.limit`，
//! 执法函数 `ensureDrawingsLimitAllowed(limitObj,delta,source)` 的第一条就是
//! `limitObj===null` 直接放行；把 `chartModel.drawingsLimit()` 打空后 CDP 实测
//! 连续建到 42 条不再弹 PAYWALL_DRAWINGS_LIMIT。**服务端是否接受 >30 条未验证**，
//! 见该补丁的注释；指标叠加（STUDY_ON_STUDY）的子指标上限同样 gate 在客户端：CDP 实测
//! `getConfig('STUDY_ON_STUDY')` 返回渲染进程内嵌计划配置表的共享对象
//! （free 档 limit:800 / child_limit:1），执法方 `canCreateStudy` 经
//! `featureLimit` 活读取该表，改写共享表后 featureLimit 实测联动放行——可注入解除。
//! 自动更新只有启动链一处自动触发（`op().checkForUpdates()` 唯一入口；
//! 断链后 `appInstaller` 恒空，`setupUpdatesMonitoring` 双重保险使更新定时器
//! 永不建立），`checkForUpdatesAndNotify` 全 bundle 仅定义未调用；
//! 设置面板的手动检查/下载路径不受影响。

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

mod data;

pub use data::*;

/// 用户可配置的功能开关（持久化在 tvx 的 config.json）。
/// 拦截广告弹窗 / 多图表解锁 / 指标叠加解锁 / 绘图数量解锁 / 禁用自动更新 / 断线自愈
/// 各自独立。拦截广告弹窗是**一个**功能：网络层拦广告域名、壳层不再开充值窗口、
/// 图表页内推广浮层就地隐藏——三者对用户是同一件事（窗口别弹出来），所以共用一个开关。
/// 登录回调直达不在其中：它是多账号能用起来的前提（见 `Group::Auth`），恒开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    pub adblock: bool,
    pub charts_unlock: bool,
    pub study_on_study: bool,
    /// 单张图表的绘图数量上限（free 档 30）：客户端 gate，注入后不限
    pub drawings_unlock: bool,
    /// 回测报告的客户端判定项：扩展指标 + CSV/XLSX 导出交易记录
    /// （高精度 K 线 / 每根历史 tick / 深回测由服务端算力决定，不在其中）
    pub backtest_unlock: bool,
    pub no_auto_update: bool,
    /// 断线自愈：检测到「连接已关闭 / 恢复连接」弹窗时自动点恢复、必要时重载页面
    pub auto_recover: bool,
    /// 本地指标报警：图表页注入报警引擎（设置页 + 指标条件求值），
    /// 命中后由主进程发系统通知 / 邮件。不涉及服务端报警配额。
    pub alerts: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            adblock: true,
            charts_unlock: true,
            study_on_study: true,
            drawings_unlock: true,
            backtest_unlock: true,
            no_auto_update: true,
            auto_recover: true,
            alerts: true,
        }
    }
}

/// 补丁所属的功能组，决定它受哪个开关控制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// 拦截广告弹窗（合并后的单一功能）：
    /// - 网络层拦掉广告域名；
    /// - 壳层不再创建订阅遮罩窗口、`showPaywall` 空操作、开窗拦充值/推广链接；
    /// - 图表页内的推广浮层插入即隐藏；
    /// 指标数量 / 布局保存 / 警报这类服务端强制项（客户端解不掉）也归这里——
    /// 用户看到的就是「少一个要充值的窗口」，不该拆成两个开关。
    /// 另外「放宽离线判定超时」（1s→10s）也留在这组：它防的是代理环境被误判离线
    /// 而弹出重连框，和"别弹窗"是同一件事，合并前就跟着充值屏蔽开关走。
    AdBlock,
    /// 多图表布局客户端 gate 解锁
    Charts,
    /// 指标叠加（STUDY_ON_STUDY）客户端 gate 解锁
    StudyOnStudy,
    /// 单张图表的绘图数量（DRAWINGS_ON_CHART）客户端 gate 解锁
    Drawings,
    /// 回测报告的**客户端**判定项解锁：扩展指标、交易记录 CSV/XLSX 导出。
    /// 高精度历史 K 线 / 每根历史 tick / 深回测由服务端算力决定（实测打开后
    /// 报告变空 + 升级提示），不放进来。
    Backtest,
    /// 禁用启动时的自动更新检查
    NoAutoUpdate,
    /// 登录回调直达：每个实例把 `--user-data-dir` 写进 tradingview:// 注册，
    /// 授权回调才会回到"发起登录的那个实例"（否则 state 不匹配，新授权被丢弃）。
    /// 恒开、不提供开关：多账号在 Windows 上必须靠它才能登录成功，
    /// 而 macOS 走的是「一个账号一个独立 App」的等价机制，不需要用户选择。
    Auth,
    /// 断线自愈（服务端踢连接后自动恢复，不改配额）
    Recover,
    /// 本地指标报警（页内引擎 + 设置页 + 主进程通知）
    Alerts,
    /// 注入前提：`dom-ready` 等到 BrowserView 就绪再调 `handleDomReady()`。
    /// 恒开、不提供开关——其余所有页内注入（自愈/多图表/指标/绘图/推广）都排在他后面，
    /// 若做成可关，用户关掉某一个功能就会把「启动期重试」一起关掉，那些功能重新出现
    /// 启动后几十秒的空窗（v0.1.6 初版真的踩到了：壳挂在「拦截广告弹窗」的推广补丁里）。
    Core,
}

impl Group {
    fn enabled(self, f: &Flags) -> bool {
        match self {
            Group::AdBlock => f.adblock,
            Group::Charts => f.charts_unlock,
            Group::StudyOnStudy => f.study_on_study,
            Group::Drawings => f.drawings_unlock,
            Group::Backtest => f.backtest_unlock,
            Group::NoAutoUpdate => f.no_auto_update,
            Group::Auth => true,
            Group::Recover => f.auto_recover,
            Group::Alerts => f.alerts,
            Group::Core => true,
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
    /// 锚点没命中（上游版本漂移）：本条跳过，其余补丁继续
    Failed,
}

impl Status {
    pub fn mark(self) -> &'static str {
        match self {
            Status::Applied => "+",
            Status::AlreadyPatched => "=",
            Status::Reverted => "-",
            Status::Untouched => "·",
            Status::Failed => "!",
        }
    }
}

/// apply() 的结果：逐条状态 + 未命中的补丁名。
/// 单条锚点失配不再整体失败——上游大版本改一次压缩标识符，不至于把全部开关拖死。
#[derive(Debug, Default)]
pub struct ApplyReport {
    pub items: Vec<(&'static str, Status)>,
    /// 没打上的补丁名（锚点未命中或匹配数不为 1）
    pub failed: Vec<String>,
    /// 历史注入脚本升级失配等软告警，不影响最终状态
    pub warnings: Vec<String>,
}

impl ApplyReport {
    fn push(&mut self, item: (&'static str, Status)) {
        self.items.push(item);
    }
}

/// 压缩标识符（webpack 重打包后 `M.j9`、`vL` 这类名字会重排）的兜底锚点。
/// `{}` 表示"一个标识符"，按出现顺序回填到替换文本里；`from` 是未打补丁的形态，
/// `to` 是打完的形态。字面锚点优先，只有精确匹配 0 处时才启用——这样换版本时
/// 只改 `old`/`new` 一处，兜底表不用跟着改。
struct WildAnchor {
    patch: &'static str,
    from: &'static str,
    to: &'static str,
}

const WILD_ANCHORS: &[WildAnchor] = &[
    WildAnchor {
        patch: "订阅遮罩窗口不再创建",
        from: "async function {}(e,t){const n=(0,{}.{})().getLastFocusedWindow();",
        to: "async function {}(e,t){return;const n=(0,{}.{})().getLastFocusedWindow();",
    },
    WildAnchor {
        patch: "showPaywall 变空操作",
        from: "showPaywall(e,t){return {}(e,t)}",
        to: "showPaywall(e,t){return Promise.resolve()}",
    },
    WildAnchor {
        patch: "网页内充值窗口不再注入",
        from: "async requestProFeature(){const e=(0,{}.{})().getLastFocusedWindow();",
        to: "async requestProFeature(){return;const e=(0,{}.{})().getLastFocusedWindow();",
    },
    WildAnchor {
        // 启动链里唯一一次自动检查；取用函数名（`tp`→`op`）也算标识符，一并通配。
        patch: "启动自动更新检查禁用",
        from: "(0,{}.{})()||{}().checkForUpdates()",
        to: "(0,{}.{})()||!0||{}().checkForUpdates()",
    },
];

/// 用 `{}` 通配模式在 `src` 里找全部匹配，返回 (区间, 捕获的标识符)。
/// 只认 ASCII 标识符字符（`[A-Za-z0-9_$]`），不做任何正则语法解释。
fn wild_find_all<'a>(src: &'a str, pat: &str) -> Vec<(std::ops::Range<usize>, Vec<&'a str>)> {
    let parts: Vec<&str> = pat.split("{}").collect();
    let mut out = Vec::new();
    if parts.len() == 1 {
        // 没有通配符，退化成普通查找
        let mut from = 0;
        while let Some(rel) = src[from..].find(parts[0]) {
            let s = from + rel;
            out.push((s..s + parts[0].len(), Vec::new()));
            from = s + 1;
        }
        return out;
    }
    let mut search = 0;
    while let Some(rel) = src[search..].find(parts[0]) {
        let start = search + rel;
        let mut pos = start + parts[0].len();
        let mut caps = Vec::new();
        let mut ok = true;
        for part in parts.iter().skip(1) {
            let id_start = pos;
            while pos < src.len() {
                let c = src[pos..].chars().next().unwrap();
                if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                    pos += c.len_utf8();
                } else {
                    break;
                }
            }
            if pos == id_start {
                ok = false;
                break;
            }
            caps.push(&src[id_start..pos]);
            if !src[pos..].starts_with(part) {
                ok = false;
                break;
            }
            pos += part.len();
        }
        if ok {
            out.push((start..pos, caps));
        }
        search = start + 1;
    }
    out
}

/// 把 `to` / `from` 模板里的 `{}` 按捕获顺序填回。
fn wild_expand(pat: &str, caps: &[&str]) -> String {
    let mut out = String::new();
    for (i, part) in pat.split("{}").enumerate() {
        if i > 0 {
            out.push_str(caps.get(i - 1).copied().unwrap_or(""));
        }
        out.push_str(part);
    }
    out
}

/// 定位「断线自动恢复」注入段的字节区间：从 `DOM_READY_HEAD` 到 `this.url&&` 的末尾。
/// 段的边界是稳定的（脚本本体怎么改都不影响），所以历史脚本可以按段整体替换，
/// 不必为每一版都写一条 LEGACY 全文；URL 判断换写法（`this.url` → `""!==this.url`）也不影响。
fn recover_segment_range(src: &str) -> Option<(usize, usize)> {
    let start = src.find(DOM_READY_HEAD)?;
    let body = start + DOM_READY_HEAD.len();
    let close = body + src[body..].find(RECOVER_SCRIPT_TAIL)?;
    let after = close + RECOVER_SCRIPT_TAIL.len();
    let end = after + src[after..].find(RECOVER_URL_TAIL)? + RECOVER_URL_TAIL.len();
    Some((start, end))
}

/// 定位 `.on("dom-ready",…)` 回调段的字节区间（含重试壳与可能的注入脚本）。
///
/// 起点只认「我们改过」的形态：v3 初版（壳与注入耦合，壳以 `var t=this` 开头）或 v2。
/// 最初的 `.on("dom-ready",(()=>{this.handleDomReady()}))` 不在此列——它还没有壳，
/// 交给 Core 补丁的常规 old→new 打上。
///
/// 段尾不按字面抄：从 `.on(` 起按括号配平走到深度归零（跳过字符串字面量），
/// 壳里的重试分支与注入脚本写多长都不影响。配平后还要看紧邻的字符是否像
/// 语句边界（`;` `,` `)` 或换行）：正则字面量里的括号如果把它带偏，这里会拒绝迁移，
/// 宁可不迁也不能切错半个表达式——切错会让主进程加载即语法错误。
fn promo_segment_range(src: &str) -> Option<(usize, usize)> {
    let open = ".on(\"dom-ready\",(()=>{";
    let (start, body) = {
        let mut from = 0usize;
        loop {
            let s = from + src[from..].find(open)?;
            let rest = &src[s..];
            if let Some(h) = PROMO_SEG_HEADS.iter().find(|h| rest.starts_with(**h))
            {
                break (s, s + h.len());
            }
            from = s + open.len();
        }
    };
    // 段内必须真的调了 handleDomReady，否则不是这个回调
    if !src[body..].contains("handleDomReady()") {
        return None;
    }
    // 括号配平（跳过字符串字面量：注入脚本里全是引号里的括号）
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut esc = false;
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i];
        if let Some(q) = quote {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == q {
                quote = None;
            }
        } else {
            match c {
                b'"' | b'\'' | b'`' => quote = Some(c),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        // 语句边界校验：配平若被正则字面量带偏，这里会落到表达式中间。
                        // `.` 是合法的：真实代码里 `.on("dom-ready",…)` 后面直接链式
                        // `.on("did-navigate-in-page",…)`（2026-11-26 实测，段尾下一字节
                        // 就是 `.`）。早先不认 `.`，老副本升级注入会被判"锚点未命中"，
                        // 只能整份重建副本才打得进去。
                        let ok = match bytes.get(i + 1) {
                            None => true,
                            Some(b';' | b',' | b')' | b'.' | b'\n' | b'\r' | b' ' | b'\t') => true,
                            _ => false,
                        };
                        return if ok { Some((start, i + 1)) } else { None };
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// 按开关对解包后的 index.js 应用/撤回补丁。任一处匹配数不为 1 就整体失败，
/// 避免"打了一半"的半残状态。
///
/// 语义：
/// - 开关开 + 未打过 → 打上（Applied）
/// - 开关开 + 已打过 → 不动（AlreadyPatched）
/// - 开关关 + 已打过 → 撤回 new→old（Reverted）
/// - 开关关 + 未打过  → 不动（Untouched）
/// Core 壳的识别标记（代次令牌的赋值语句，壳独有）。
///
/// 壳的 `new` 里留了注入空位（`SHELL_HOOK`），推广补丁打上后会把它替换成注入脚本——
/// 于是"整段 `contains(core.new)`"在打完补丁的文件里必然为假。重入判定与 `is_patched`
/// 都只能认这个标记，否则每次 _reapply 都会把 Core 误报成"锚点未命中"。
fn core_shell_present(s: &str) -> bool {
    s.contains("t.__tvxDR=(t.__tvxDR||0)+1")
}

pub fn apply(index_js: &Path, flags: &Flags) -> Result<ApplyReport> {
    let bytes = fs::read(index_js).with_context(|| format!("读取 {}", index_js.display()))?;
    let mut src = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("index.js 不是合法 UTF-8，拒绝改写"))?;

    let mut report = ApplyReport::default();
    // 历史注入脚本升级失配只算告警：后面的常规流程仍会给这条补丁一个准确状态，
    // 不能因为一段旧文本没对上就整批失败。
    let mut warnings: Vec<String> = Vec::new();

    // 0) 历史注入文本先就地升级：已装好的副本里还是旧脚本时，新版锚点是对不上的。
    //    按当前开关决定升级成新脚本（开）还是撤回成原始锚点（关）。
    for (patch_name, legacy) in LEGACY {
        let Some(p) = PATCHES.iter().find(|p| p.name == *patch_name) else {
            continue;
        };
        if !src.contains(legacy) {
            continue;
        }
        match src.matches(legacy).count() {
            1 => {
                let target = if p.group.enabled(flags) { p.new } else { p.old };
                src = src.replacen(legacy, target, 1);
            }
            n => warnings.push(format!(
                "{}：旧版注入脚本期望 1 处匹配，实际 {n} 处",
                p.name
            )),
        }
    }

    // 0b) 「断线自动恢复」按段迁移：脚本本身会随版本演进（内存计数 → 会话存储 → 现在的
    //     首试不等 + 健康清零），每一版的注入文本都不一样，靠 old 精确匹配必然失配。
    //     段的边界（DOM_READY_HEAD … RECOVER_URL_TAIL）是稳定的，这里整段换成当前版本；
    //     已是原始锚点或已是当前脚本时不动，交给下面的常规流程给出准确状态。
    if let Some(p) = PATCHES.iter().find(|p| p.group == Group::Recover) {
        match src.matches(DOM_READY_HEAD).count() {
            0 | 1 => {
                if let Some((s, e)) = recover_segment_range(&src) {
                    let seg = &src[s..e];
                    if seg != p.old && seg != p.new {
                        let target = if p.group.enabled(flags) { p.new } else { p.old };
                        src.replace_range(s..e, target);
                    }
                }
            }
            n => warnings.push(format!(
                "{}：handleDomReady 段期望 1 处，实际 {n} 处",
                p.name
            )),
        }
    }

    // 0c) `.on("dom-ready",…)` 回调段按段迁移：v2（注入与 handleDomReady 耦合）与
    //     v3 初版（壳与注入耦合）文本完全不同，靠 old 精确匹配会失配，
    //     还会把已装好的副本判成"锚点未命中"。
    //     现行形态只有「恒开的 Core 壳」：页内推广浮层的注入脚本 2026-11-26 已整段下线
    //     （靠文案/面积猜 DOM 去隐藏，误伤远大于收益），老副本里残留的那段必须收回到壳，
    //     否则用户升级了 tvhelper，旧脚本还在页面上跑。
    if let Some(core) = PATCHES.iter().find(|p| p.group == Group::Core) {
        if let Some((s, e)) = promo_segment_range(&src) {
            let seg = &src[s..e];
            if seg != core.new {
                src.replace_range(s..e, core.new);
            }
        }
    }

    for p in PATCHES {
        if !p.group.enabled(flags) {
            if src.contains(p.new) {
                match src.matches(p.new).count() {
                    1 => {
                        src = src.replacen(p.new, p.old, 1);
                        report.push((p.name, Status::Reverted));
                    }
                    n => {
                        report.push((p.name, Status::Failed));
                        report.failed.push(format!(
                            "{}：撤回时期望 new 1 处匹配，实际 {n} 处",
                            p.name
                        ));
                    }
                }
            } else if let Some(w) = WILD_ANCHORS.iter().find(|w| w.patch == p.name) {
                // 之前可能是用通配锚点打的（压缩标识符已经变了），精确 new 对不上；
                // 但文件里仍是"已打补丁"的形态，按 to 模式定位后回填 from。
                match wild_find_all(&src, w.to).as_slice() {
                    [(range, caps)] => {
                        let back = wild_expand(w.from, caps);
                        src.replace_range(range.clone(), &back);
                        report.push((p.name, Status::Reverted));
                    }
                    [] => report.push((p.name, Status::Untouched)),
                    many => {
                        report.push((p.name, Status::Failed));
                        report.failed.push(format!(
                            "{}：撤回时通配锚点命中 {} 处",
                            p.name,
                            many.len()
                        ));
                    }
                }
            } else {
                report.push((p.name, Status::Untouched));
            }
            continue;
        }

        // Core 壳的空位可能已被注入填掉（见 core_shell_present），先认壳特征
        if p.group == Group::Core && core_shell_present(&src) {
            report.push((p.name, Status::AlreadyPatched));
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
            0 => {
                // 字面锚点没命中：多半只是压缩标识符重排（`M.j9` → `M.xQ` 这种），
                // 用 `{}` 通配再定位一次。
                let Some(w) = WILD_ANCHORS.iter().find(|w| w.patch == p.name) else {
                    report.push((p.name, Status::Failed));
                    report.failed.push(format!(
                        "{}：锚点未命中（上游代码结构已变，需要更新补丁集）",
                        p.name
                    ));
                    continue;
                };
                match wild_find_all(&src, w.from).as_slice() {
                    [(range, caps)] => {
                        let to = wild_expand(w.to, caps);
                        src.replace_range(range.clone(), &to);
                        report.push((p.name, Status::Applied));
                    }
                    [] => {
                        // 也可能已经是通配锚点打过的形态
                        if wild_find_all(&src, w.to).len() == 1 {
                            report.push((p.name, Status::AlreadyPatched));
                        } else {
                            report.push((p.name, Status::Failed));
                            report.failed.push(format!(
                                "{}：字面与通配锚点都未命中（上游代码结构可能已变）",
                                p.name
                            ));
                        }
                    }
                    many => {
                        report.push((p.name, Status::Failed));
                        report.failed.push(format!(
                            "{}：通配锚点命中 {} 处，无法确定替换位置",
                            p.name,
                            many.len()
                        ));
                    }
                }
            }
            n => {
                report.push((p.name, Status::Failed));
                report.failed
                    .push(format!("{}：期望 1 处匹配，实际 {n} 处", p.name));
            }
        }
    }

    // 成功一条就落一条：单条失配只影响该功能，不再把整批补丁和副本一起拖死。
    // 调用方据 ApplyReport::failed 提示"哪几项没生效、建议重建副本"。
    fs::write(index_js, src.as_bytes())
        .with_context(|| format!("写回 {}", index_js.display()))?;
    report.warnings = warnings;
    Ok(report)
}

/// 图表页报警引擎（设置页 + 条件求值）：作为真实 JS 文件随程序分发，
/// 编译期嵌入，运行时写到 app 目录，由主进程读出来 executeJavaScript。
///
/// 源码按功能分成 `src/alerts/0*.js` 七段（图表访问 / 脚本兜底 / 条件求值 / 规则 / 主循环 /
/// 设置页 / 启动），由 `build.rs` 按文件名顺序拼成一份再嵌进来——它们共享同一个 IIFE
/// 作用域，必须是一段脚本，拆文件只是为了能读。
const ALERTS_ENGINE_JS: &str = include_str!(concat!(env!("OUT_DIR"), "/alerts-engine.js"));

/// 主进程侧报警模块：IPC、配置持久化、系统通知、SMTP。
const ALERTS_MAIN_JS: &str = include_str!("alerts-main.js");

const PRELOAD_BRIDGE_MARK: &str = "/*tvx-alerts-bridge*/";

/// 追加到 platform-preload.js 末尾的桥：把主进程的报警 IPC 暴露给图表页。
/// preload 里 `process.contextIsolated` 为真（实测），所以必须走 contextBridge。
const PRELOAD_BRIDGE: &str = "\n;(function(){/*tvx-alerts-bridge*/try{var e=require(\"electron\");if(!e||!e.contextBridge||!e.ipcRenderer)return;e.contextBridge.exposeInMainWorld(\"__TVX_ALERTS__\",{get:function(){return e.ipcRenderer.invoke(\"tvx-alerts:get\")},save:function(c){return e.ipcRenderer.invoke(\"tvx-alerts:save\",c)},state:function(s){return e.ipcRenderer.invoke(\"tvx-alerts:state\",s)},fire:function(p){return e.ipcRenderer.invoke(\"tvx-alerts:fire\",p)},test:function(k){return e.ipcRenderer.invoke(\"tvx-alerts:test\",k)},dump:function(d){return e.ipcRenderer.invoke(\"tvx-alerts:dump\",d)}})}catch(err){console.error(\"[tvx-alerts-bridge]\",err)}})();\n";

/// 去掉 preload 里的桥（整行），并把因此留下的多余空行收掉，保证撤回后与原文一致。
fn strip_bridge(src: &str) -> String {
    let mut out = String::new();
    for line in src.split_inclusive('\n') {
        if !line.contains(PRELOAD_BRIDGE_MARK) {
            out.push_str(line);
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// 把报警功能所需的文件落到托管副本里（开关打开时写入，关闭时清掉）。
///
/// 这三个文件与 index.js 里那条 `require("./tvx-alerts-main.js")` 一起构成完整功能：
/// - tvx-alerts-engine.js：图表页脚本（设置页 UI + 指标条件求值）
/// - tvx-alerts-main.js：主进程模块（IPC / 持久化 / 通知 / 邮件）
/// - preload/platform-preload.js 末尾的桥（幂等追加，可按标记整段撤回）
///
/// 关闭开关时全部移除，副本回到"没有报警功能"的干净状态。
///
/// 返回 warnings：不致命但会让功能静默失效的情况（例如 preload 不在预期路径，
/// 桥写不进去 → 引擎第一行就退出，用户只看到"按钮不见了"）。
pub fn install_assets(app_dir: &Path, flags: &Flags) -> Result<Vec<String>> {
    let mut warnings: Vec<String> = Vec::new();
    let engine = app_dir.join("tvx-alerts-engine.js");
    let main = app_dir.join("tvx-alerts-main.js");
    let preload = app_dir.join("preload").join("platform-preload.js");

    if flags.alerts {
        fs::write(&engine, ALERTS_ENGINE_JS)
            .with_context(|| format!("写入 {}", engine.display()))?;
        fs::write(&main, ALERTS_MAIN_JS).with_context(|| format!("写入 {}", main.display()))?;

        if !preload.is_file() {
            // 桥是报警功能的前提：没有它，图表页脚本第一行就 return。
            // 以前这里静默跳过，副本看起来"构建成功"，实际报警全无。
            warnings.push(format!(
                "preload 路径不存在（{}），报警桥写不进去——报警功能将不可用。\
                 处理：点「生成 / 更新副本」重建；仍不行说明该版本目录结构不同，请反馈",
                preload.display()
            ));
        } else {
            let src = fs::read_to_string(&preload)
                .with_context(|| format!("读取 {}", preload.display()))?;
            // 桥可能来自旧版本（比如还没暴露 dump 通道）。按标记整行换掉，再追加当前桥，
            // 这样升级时不会把旧桥留在页面里。
            let out = if !src.contains(PRELOAD_BRIDGE_MARK) || !src.contains("tvx-alerts:dump") {
                let mut cleaned = strip_bridge(&src);
                cleaned.push_str(PRELOAD_BRIDGE);
                Some(cleaned)
            } else {
                None
            };
            if let Some(out) = out {
                fs::write(&preload, out)
                    .with_context(|| format!("写入 {}", preload.display()))?;
            }
        }
        return Ok(warnings);
    }

    let _ = fs::remove_file(&engine);
    let _ = fs::remove_file(&main);
    if preload.is_file() {
        let src = fs::read_to_string(&preload)
            .with_context(|| format!("读取 {}", preload.display()))?;
        if src.contains(PRELOAD_BRIDGE_MARK) {
            let out = strip_bridge(&src);
            fs::write(&preload, out)
                .with_context(|| format!("写入 {}", preload.display()))?;
        }
    }
    Ok(warnings)
}

/// 单条补丁是否处于「当前开关组合所要求」的状态。
fn patch_ok(s: &str, p: &Patch, flags: &Flags) -> bool {
    // 精确锚点之外，再看通配锚点：上游只重排压缩标识符时，new/old 字面都不在
    // 文件里，但补丁其实打得是对的，不能误报"补丁异常"。
    let wild = WILD_ANCHORS.iter().find(|w| w.patch == p.name);
    let has_new = if p.group == Group::Core {
        // 壳里的空位可能已被注入填掉，整段比对不可靠（见 core_shell_present）
        core_shell_present(s)
    } else {
        s.contains(p.new) || wild.is_some_and(|w| wild_find_all(s, w.to).len() == 1)
    };
    let has_old = s.contains(p.old) || wild.is_some_and(|w| wild_find_all(s, w.from).len() == 1);
    if p.group.enabled(flags) {
        has_new && !has_old
    } else {
        !has_new
    }
}

/// 检查 index.js 是否处于"当前开关组合所要求"的已打补丁状态：
/// 开着的补丁必须都在，关着的补丁必须都不在。
pub fn is_patched(index_js: &Path, flags: &Flags) -> bool {
    match fs::read_to_string(index_js) {
        Ok(s) => PATCHES.iter().all(|p| patch_ok(&s, p, flags)),
        Err(_) => false,
    }
}

/// 哪几条没通过。自检报告只说「补丁不完整」用户无从下手，得点名到具体功能。
pub fn failing_patches(index_js: &Path, flags: &Flags) -> Vec<String> {
    match fs::read_to_string(index_js) {
        Ok(s) => PATCHES
            .iter()
            .filter(|p| !patch_ok(&s, p, flags))
            .map(|p| p.name.to_string())
            .collect(),
        Err(_) => vec![format!("(读不到 {})", index_js.display())],
    }
}

#[cfg(test)]

#[cfg(test)]
mod tests;
