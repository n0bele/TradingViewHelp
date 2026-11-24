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
//! 指标叠加（STUDY_ON_STUDY）的子指标上限同样 gate 在客户端：CDP 实测
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

/// 补丁锚点所针对、并做过端到端实测验证的 TradingView Desktop 版本。
/// 锚点是 index.js 里的字面代码片段（压缩标识符随版本漂移，效果等同"偏移不同"），
/// 其他版本不保证命中——apply() 在失配时把该条标成未生效并指出是哪条没对上，
/// 绝不静默打一半；界面（app.rs）会把该版本标注在副本状态旁。
///
/// **锚点只跟这一个版本对齐**。3.5.1.8500 相对 3.5.0.8375 只是重排了压缩标识符
/// （`vL` / `Le.Qh` / `$C.xE` / `Zu` / `op`）并把 `handleDomReady` 里的 URL 判断
/// 写成 `""!==this.url`，结构性代码没动。维护两张版本表没有收益：托管副本是按
/// 版本目录隔离的自包含拷贝，旧副本建好就不用再重打，跟着新版本走即可。
///
/// **这就是"版本锁"的唯一事实来源**：托管副本 `~/.tvx/apps/<version>/` 是按目录名
/// 存在的，一旦在这个版本上构建成功，它是一份自包含的拷贝——客户端以后自动升级到
/// 新版也不会改动这份副本，所以副本本身就是锁。
///
/// 换版本时必须真的拿该版本的 `resources/app/index.js` 跑通全部锚点
/// （`apply()` 全绿、并实机登录一次），不能凭"看起来差不多"改数字。
pub const VERIFIED_VERSION: &str = "3.5.1.8500";

/// 已验证版本白名单（只有当前对齐的这一版）。
pub const VERIFIED_VERSIONS: &[&str] = &[VERIFIED_VERSION];

/// 该客户端版本是否属于已验证版本（决定默认选源、是否弹风险确认、是否标"未验证"）。
pub fn is_verified(version: &str) -> bool {
    VERIFIED_VERSIONS.iter().any(|v| *v == version)
}

/// 用户可配置的功能开关（持久化在 tvx 的 config.json）。
/// 广告拦截 / 多图表解锁 / 指标叠加解锁 / 禁用自动更新 / 断线自愈各自独立；
/// 指标数量 / 布局保存 / 警报由服务端强制（客户端无法解除拒绝），共用「屏蔽充值窗口」。
/// 登录回调直达不在其中：它是多账号能用起来的前提（见 `Group::Auth`），恒开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    pub ads_block: bool,
    pub charts_unlock: bool,
    pub study_on_study: bool,
    pub silence_paywalls: bool,
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
            ads_block: true,
            charts_unlock: true,
            study_on_study: true,
            silence_paywalls: true,
            no_auto_update: true,
            auto_recover: true,
            alerts: true,
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
    /// 授权回调才会回到"发起登录的那个实例"（否则 state 不匹配，新授权被丢弃）。
    /// 恒开、不提供开关：多账号在 Windows 上必须靠它才能登录成功，
    /// 而 macOS 走的是「一个账号一个独立 App」的等价机制，不需要用户选择。
    Auth,
    /// 断线自愈（服务端踢连接后自动恢复，不改配额）
    Recover,
    /// 本地指标报警（页内引擎 + 设置页 + 主进程通知）
    Alerts,
}

impl Group {
    fn enabled(self, f: &Flags) -> bool {
        match self {
            Group::Silence => f.silence_paywalls,
            Group::Ads => f.ads_block,
            Group::Charts => f.charts_unlock,
            Group::StudyOnStudy => f.study_on_study,
            Group::NoAutoUpdate => f.no_auto_update,
            Group::Auth => true,
            Group::Recover => f.auto_recover,
            Group::Alerts => f.alerts,
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

/// 注入到图表页的断线自愈脚本（单引号书写：整串要嵌在 `executeJavaScript("…")` 里）。
/// 逻辑：发现「连接已关闭 / 恢复连接」类弹窗 → 先点「恢复连接 / Reconnect」；
/// 4 秒后弹窗还在就重载页面（会话与布局都在服务端，重载会重新拉历史数据）；
/// 之后按 15s / 60s / 5min 再试，最多 4 次后停手——配额真的不够时保留弹窗，
/// 至少让人看见原因，而不是静默停更。
/// 连续 60 秒没有弹窗就把计数清零：否则一次用满次数后，这个标签页在关掉之前
/// 再断线也不会自愈（sessionStorage 跨重载保留）。
///
/// **文案匹配必须是多语言的**：图表页的语言跟随 TradingView 账号设置，不是助手的
/// `lang`。这里覆盖 zh_CN / zh_TW / en / de / ru / ja / ko / es / fr / pt / tr /
/// vi / th / id / pl / it / ar。界面文案是这套匹配唯一的依赖，新增语言时只改
/// `K`（弹窗文案）与 `R`（重连按钮）两条正则；未能匹配只是不自愈，不会误点。
/// 常量仅在测试里用于逐字漂移检查（正式文本内嵌在补丁 new 里）。
/// 当前版本：重试次数写在 sessionStorage 里，跨页面重载仍然有效——
/// 否则 reload 后计数归零，服务端一直拒绝时会变成每 4 秒重载一次的循环。
#[cfg(test)]
const DISCONNECT_RECOVER: &str = "(function(){try{if(window.__tvxRecover)return;window.__tvxRecover=1;var K=/恢复连接|获取更多连接|关闭了这个连接|连接已关闭|连接已断开|關閉了這個連線|連線已中斷|We closed this connection|Connection closed|Get more connections|Verbindung wurde geschlossen|Verbindung getrennt|соединение закрыто|соединение было закрыто|この接続を閉じました|接続が切断されました|연결이 종료되었습니다|연결이 끊어졌습니다|conexión cerrada|conexión se ha cerrado|connexion fermée|connexion a été fermée|conexão fechada|conexão foi encerrada|bağlantı kapatıldı|bağlantı kesildi|kết nối đã bị đóng|kết nối đã ngắt|การเชื่อมต่อถูกปิด|koneksi ditutup|koneksi terputus|połączenie zostało zamknięte|connessione chiusa|connessione è stata chiusa|تم إغلاق الاتصال/i;var R=/恢复连接|重新连接|恢復連線|重新連線|Restore connection|Reconnect|Verbindung wiederherstellen|Erneut verbinden|Neu verbinden|Восстановить соединение|Переподключиться|接続を復元|再接続|연결 복원|다시 연결|Restaurar conexión|Reconectar|Rétablir la connexion|Reconnecter|Restaurar conexão|Reconectar|Bağlantıyı geri yükle|Yeniden bağlan|Khôi phục kết nối|Kết nối lại|เชื่อมต่อใหม่|กู้คืนการเชื่อมต่อ|Pulihkan koneksi|Hubungkan ulang|Przywróć połączenie|Połącz ponownie|Ripristina connessione|Riconnetti|إعادة الاتصال/i;var waits=[0,15000,60000,300000];function st(){try{return JSON.parse(sessionStorage.getItem('tvxRecover')||'{}')}catch(e){return {}}}function save(o){try{sessionStorage.setItem('tvxRecover',JSON.stringify(o))}catch(e){}}function dlg(){var q=document.querySelectorAll('[role=dialog],[data-name*=dialog],[class*=dialog]');for(var i=0;i<q.length;i++){var e=q[i],t=e.textContent||'';if(t.length<900&&K.test(t))return e}return null}function btn(d){var b=d.querySelectorAll('button,[role=button],a');for(var i=0;i<b.length;i++){var t=(b[i].textContent||'').trim();if(t&&t.length<40&&R.test(t))return b[i]}return null}var ok=0;function tick(){var d=dlg();if(!d){if(++ok>=75){var h=st();if(h.n||h.t)save({n:0,t:0});ok=0}return}ok=0;var s=st();if((s.n||0)>=4)return;d.setAttribute('data-tvx-recovering','1');if(!d.__tvxClicked){var b=btn(d);if(b){try{b.click()}catch(e){}d.__tvxClicked=1}}setTimeout(function(){if(!dlg())return;var o=st(),n=o.n||0;if(n>=4)return;var now=Date.now();if(now-(o.t||0)<(waits[n]||0))return;save({n:n+1,t:now});try{location.reload()}catch(e){}},4000)}setInterval(tick,800)}catch(e){}})();";

/// 注入到图表页的「页内推广浮层拦截器」（v2：插入即拦，不再轮询）。
///
/// 背景：充值/升级提示不是壳层弹窗——TradingView 在图表页里自己渲染 React 浮层，
/// 壳层那四条补丁（nk / showPaywall / requestProFeature / OPEN_NEW_WINDOW 过滤）
/// 都碰不到它。
///
/// **v1 为什么没拦住（实测复盘，勿重犯）**：
///   1. 识别门槛要求 textContent 里出现 `TradingView`。但「更多指标，更多的交易机会」
///      这张指标数量弹窗的正文里根本没有这个词——TradingView 只印在宣传图（位图）上，
///      于是直接被 `!T.test(t)` 判成"不是推广"而跳过。**文案不是可靠指纹。**
///   2. 找到"长得像关闭"的按钮就只点它、然后 `return`：按钮无效（装饰用 ×、
///      或点了不销毁浮层）时浮层原样留着，等于什么都没做。
///   3. 1.2 秒轮询：即使判定正确，也至少已经画出来 1 秒以上，用户看着它闪一下。
///   4. 只查 `[role=dialog]/[class*=modal]…` 这批选择器：根节点换成别的类名、
///      落在 shadow root 里、或整卡是个跨域 iframe 时，压根看不见。
///
/// **v2 的做法（提早到"插入那一刻"）**：
///   - `MutationObserver` 监听 document 的 childList+subtree：节点一插进 DOM 就判定，
///     回调本身是微任务，在浏览器绘制之前执行 → 判定命中时浮层**一帧都不会画出来**。
///     不再"点关闭"，直接 `display:none !important`（不依赖浮层有没有关闭按钮）。
///   - 识别不再要求任何语言的标题译文，只看**不翻译的结构与品牌词**，任一条命中即拦：
///       a) 出现 ≥2 个套餐名（Basic/Essential/Premium/Ultimate/Advanced，各语言都不译）
///          且带购买/品牌/限制语义 → 截图 1 这类"指标数量"卡就落在这条；
///       b) 品牌词 + 购买语义 + 大尺寸媒体（video / 大图 / 大 iframe）→ 截图 2 这类推广卡；
///       c) 购买语义 + 限制语义（limit/上限/最大…）+ 媒体或套餐名。
///   - 跨域 iframe 兜底：任何覆盖 ≥35% 视口的 iframe 视为浮层（图表页没有这么大的正常 frame）。
///   - shadow root：新节点带 shadowRoot 时把观察器挂进去，不靠 document 查询。
///   - 1 秒全量扫描只作最后兜底（应对 React 复用节点、框架内部重排）。
///
/// **四道保险，绝不误伤**：
///   1. 含 `<canvas>` 的子树一律跳过（图表本体）；`body`/`html`/`#root` 永不处理；
///   2. 文案含「恢复连接/连接已关闭」跳过（交给断线自愈，那里要的是"恢复"不是"藏"）；
///   3. 每批 DOM 变更最多判 80 个节点，避免图表高频重绘时拖慢页面；
///   4. 只在"有按钮/链接/媒体"的子树里判定，纯文本节点与图表图元直接跳过。
///
/// 注意：这只是"不让它显示"。指标数量是服务端拒绝（study_limit_exceeded），
/// 客户端没有可解除的 gate——想真正多开指标只能升级方案，这里不改变任何配额。
/// 常量仅在测试里用于逐字漂移检查（正式文本内嵌在补丁 new 里）。
#[cfg(test)]
const PROMO_GUARD: &str = "(function(){try{if(window.__tvxGuard)return;window.__tvxGuard=1;var ATTR='data-tvx-hidden';var BUY=/upgrade|pricing|subscribe|subscription|plans?|升级|升級|订阅|訂閱|方案|套餐|abonn|подписк|тариф|プラン|요금|업그레이드/i;var LIM=/limit|maximum|reached|exceed|no more|上限|最大|最多|已达|已達|限制|превыш|достиг/i;var TIER=/basic|essential|premium|ultimate|advanced/gi;var BR=/tradingview/i;var DIS=/恢复连接|获取更多连接|关闭了这个连接|Restore connection|We closed this connection|Connection closed/i;function vw(){return Math.max(document.documentElement.clientWidth||0,window.innerWidth||0)}function vh(){return Math.max(document.documentElement.clientHeight||0,window.innerHeight||0)}function tiers(s){var m=s.match(TIER),o={},c=0,i,k;if(!m)return 0;for(i=0;i<m.length;i++){k=m[i].toLowerCase();if(!o[k]){o[k]=1;c++}}return c}function hasChart(e){try{return !!(e&&e.querySelector&&e.querySelector('canvas'))}catch(x){return !1}}function big(e){try{if(!e||!e.querySelectorAll)return 0;var q=e.querySelectorAll('video,img,iframe'),n=q.length>20?20:q.length,i,area=vw()*vh()*0.05,r;for(i=0;i<n;i++){r=q[i].getBoundingClientRect();if(r.width*r.height>=area)return 1}return 0}catch(x){return 0}}function hide(e){try{if(!e||e.nodeType!==1||e.__tvxH)return;if(e===document.body||e===document.documentElement||e.id==='root')return;if(hasChart(e))return;e.__tvxH=1;e.style.setProperty('display','none','important');e.setAttribute(ATTR,'1');window.__tvxHid=(window.__tvxHid||0)+1;try{console.log('[tvx] hid paywall overlay',e.tagName,(e.getAttribute('data-name')||e.className||'').toString().slice(0,80))}catch(x){}try{if(document.body)document.body.style.removeProperty('overflow');document.documentElement.style.removeProperty('overflow')}catch(x){}}catch(x){}}function climb(e){var c=e,b=null;while(c&&c.parentNode&&c.parentNode!==document.documentElement){if(c===document.body||c.id==='root'||hasChart(c))break;var r=c.getBoundingClientRect();if(r.width>=vw()*0.8&&r.height>=vh()*0.8)b=c;c=c.parentElement}return b||e}function promo(e){try{if(!e||e.nodeType!==1)return 0;if(e===document.body||e===document.documentElement||e.id==='root')return 0;if(hasChart(e))return 0;if(!e.querySelector)return 0;if(!e.querySelector('button,[role=button],a[href],video,img,iframe'))return 0;var t=e.textContent||'';if(!t||t.length>8000)return 0;if(DIS.test(t))return 0;var k=tiers(t),buy=BUY.test(t),lim=LIM.test(t),br=BR.test(t),m=big(e);if(k>=2&&(buy||br||lim))return 1;if(br&&buy&&m)return 1;if(buy&&lim&&(m||k>=1))return 1;return 0}catch(x){return 0}}function bigFrame(e){try{if(!e||e.tagName!=='IFRAME')return 0;var r=e.getBoundingClientRect();return r.width>=vw()*0.35&&r.height>=vh()*0.35?1:0}catch(x){return 0}}var O={childList:!0,subtree:!0},mo=null,budget=0;function watch(r){try{mo.observe(r,O)}catch(x){}}function scan(n){try{if(!n||n.nodeType!==1)return;if(bigFrame(n)){hide(n);return}if(n.shadowRoot)watch(n.shadowRoot);if(budget>80)return;if(!n.querySelector)return;if(!n.querySelector('button,[role=button],a[href],video,img,iframe'))return;budget++;if(promo(n))hide(climb(n))}catch(x){}}function sweep(){try{var q=document.querySelectorAll('iframe,[role=dialog],dialog,[class*=overlay],[class*=modal],[class*=popup],[class*=paywall],[data-name*=paywall],[data-name*=dialog]'),i,e;for(i=0;i<q.length;i++){e=q[i];if(e.__tvxH)continue;if(bigFrame(e)){hide(e);continue}if(promo(e))hide(climb(e))}}catch(x){}}try{mo=new MutationObserver(function(ms){budget=0;var i,j,a;for(i=0;i<ms.length;i++){a=ms[i].addedNodes;for(j=0;j<a.length;j++)scan(a[j])}});mo.observe(document,O)}catch(x){}setTimeout(sweep,0);setTimeout(sweep,1500);setInterval(sweep,1e3)}catch(e){}})();";

/// 上一版（计数只存内存，重载即清零）。已打过的副本要先迁移到当前版本。
/// handleDomReady 段的开头（补丁 new 与迁移文本都必须以它起头）
const DOM_READY_HEAD: &str =
    "handleDomReady(){this.isBrowserViewValid()&&(this.executeJavaScript(\"";

/// `handleDomReady` 段的两块稳定边界文本：注入脚本之后固定是 `").catch((()=>{}))`，
/// 再接 URL 判断；判断各版本写法不同（`this.url&&` / `""!==this.url&&`），但都以
/// `this.url&&` 收尾。所以段尾只要认这两块，不必按版本枚举。
const RECOVER_SCRIPT_TAIL: &str = "\").catch((()=>{})),";
const RECOVER_URL_TAIL: &str = "this.url&&";

/// 「页内推广浮层」注入段的边界：`.on("dom-ready",…executeJavaScript("` 到 `").catch((()=>{}))}))`。
/// 脚本本体每升级一次（v1 轮询点关闭 → v2 插入即拦）文本就全变，靠 old 精确匹配必然失配，
/// 所以和断线段一样按边界整段替换。
const PROMO_SEG_HEAD: &str = ".on(\"dom-ready\",(()=>{this.handleDomReady(),this.isBrowserViewValid()&&this.executeJavaScript(\"";
const PROMO_SEG_TAIL: &str = "\").catch((()=>{}))}))";

const LEGACY_RECOVER_V1: &str = r#"handleDomReady(){this.isBrowserViewValid()&&(this.executeJavaScript("(function(){try{if(window.__tvxRecover)return;window.__tvxRecover=1;var K=/恢复连接|获取更多连接|关闭了这个连接|We closed this connection|Restore connection|Get more connections/i;var n=0,last=0,waits=[15000,60000,300000];function dlg(){var q=document.querySelectorAll('[role=dialog],[data-name*=dialog],[class*=dialog]');for(var i=0;i<q.length;i++){var e=q[i],t=e.textContent||'';if(t.length<600&&K.test(t))return e}return null}function btn(d){var b=d.querySelectorAll('button,[role=button],a');for(var i=0;i<b.length;i++){var t=(b[i].textContent||'').trim();if(/恢复连接|重新连接|Restore connection|Reconnect/i.test(t))return b[i]}return null}function tick(){var d=dlg();if(!d)return;d.setAttribute('data-tvx-recovering','1');var b=btn(d);if(b){try{b.click()}catch(e){}}setTimeout(function(){if(!dlg())return;if(n>=3)return;var now=Date.now();if(now-last<(waits[n]||3e5))return;last=now;n++;try{location.reload()}catch(e){}},4000)}setInterval(tick,800)}catch(e){}})();").catch((()=>{})),this.url&&"#;

/// 旧版本注入过的「只隐藏不恢复」脚本。已装好的副本里是这段文本，
/// apply() 会先把它升级成新脚本（或按开关撤回成原始锚点），
/// 否则新版 old 锚点对不上，会被判成"锚点失配"而整体失败。
const LEGACY_HIDE_DISCONNECT: &str = r#"handleDomReady(){this.isBrowserViewValid()&&(this.executeJavaScript("(function(){try{if(window.__tvxNoDisconnect)return;window.__tvxNoDisconnect=1;var K=/恢复连接|获取更多连接|关闭了这个连接|We closed this connection|Restore connection|Get more connections/i;function s(){var n=document.querySelectorAll('[role=dialog],[data-name*=dialog],[class*=dialog]');for(var i=0;i<n.length;i++){var e=n[i],t=e.textContent||'';if(t.length<600&&K.test(t)){e.style.display='none';e.setAttribute('data-tvx-hidden','1')}}}setInterval(s,800)}catch(e){}})();").catch((()=>{})),this.url&&"#;

/// 需要就地迁移的历史注入文本：(所属补丁名, 旧文本)
const LEGACY: &[(&str, &str)] = &[
    ("断线自动恢复", LEGACY_HIDE_DISCONNECT),
    ("断线自动恢复", LEGACY_RECOVER_V1),
];

pub const PATCHES: &[Patch] = &[
    Patch {
        name: "订阅遮罩窗口不再创建",
        name_en: "Paywall overlay window not created",
        group: Group::Silence,
        old: "async function vL(e,t){const n=(0,Le.Qh)().getLastFocusedWindow();",
        new: "async function vL(e,t){return;const n=(0,Le.Qh)().getLastFocusedWindow();",
    },
    Patch {
        name: "showPaywall 变空操作",
        name_en: "showPaywall becomes a no-op",
        group: Group::Silence,
        old: "showPaywall(e,t){return vL(e,t)}",
        new: "showPaywall(e,t){return Promise.resolve()}",
    },
    Patch {
        name: "网页内充值窗口不再注入",
        name_en: "In-page billing popup not injected",
        group: Group::Silence,
        old: "async requestProFeature(){const e=(0,Le.Qh)().getLastFocusedWindow();",
        new: "async requestProFeature(){return;const e=(0,Le.Qh)().getLastFocusedWindow();",
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
        name: "页内推广浮层插入即拦截",
        name_en: "Intercept in-page upgrade popups before they paint",
        group: Group::Silence,
        // 前三条只管壳层（独立窗口 / IPC）。TradingView 把升级提示改成了图表页自己渲染的
        // React 浮层，壳层补丁看不到它。v1 用"1.2 秒轮询 + 文案匹配 + 点关闭"来兜，
        // 实战漏了两张（文案里没有 TradingView 的那张指标数量卡、以及点了没用的 ×）：
        // 判定发生在浮层已经画出来之后，本质上仍是"事后清理"。
        // v2 改成 MutationObserver：节点插入 DOM 的微任务里就判定并隐藏，命中即不上屏；
        // 识别只看不翻译的结构与品牌词，不再要求某种语言的标题文案。
        old: ".on(\"dom-ready\",(()=>{this.handleDomReady()}))",
        new: ".on(\"dom-ready\",(()=>{this.handleDomReady(),this.isBrowserViewValid()&&this.executeJavaScript(\"(function(){try{if(window.__tvxGuard)return;window.__tvxGuard=1;var ATTR='data-tvx-hidden';var BUY=/upgrade|pricing|subscribe|subscription|plans?|升级|升級|订阅|訂閱|方案|套餐|abonn|подписк|тариф|プラン|요금|업그레이드/i;var LIM=/limit|maximum|reached|exceed|no more|上限|最大|最多|已达|已達|限制|превыш|достиг/i;var TIER=/basic|essential|premium|ultimate|advanced/gi;var BR=/tradingview/i;var DIS=/恢复连接|获取更多连接|关闭了这个连接|Restore connection|We closed this connection|Connection closed/i;function vw(){return Math.max(document.documentElement.clientWidth||0,window.innerWidth||0)}function vh(){return Math.max(document.documentElement.clientHeight||0,window.innerHeight||0)}function tiers(s){var m=s.match(TIER),o={},c=0,i,k;if(!m)return 0;for(i=0;i<m.length;i++){k=m[i].toLowerCase();if(!o[k]){o[k]=1;c++}}return c}function hasChart(e){try{return !!(e&&e.querySelector&&e.querySelector('canvas'))}catch(x){return !1}}function big(e){try{if(!e||!e.querySelectorAll)return 0;var q=e.querySelectorAll('video,img,iframe'),n=q.length>20?20:q.length,i,area=vw()*vh()*0.05,r;for(i=0;i<n;i++){r=q[i].getBoundingClientRect();if(r.width*r.height>=area)return 1}return 0}catch(x){return 0}}function hide(e){try{if(!e||e.nodeType!==1||e.__tvxH)return;if(e===document.body||e===document.documentElement||e.id==='root')return;if(hasChart(e))return;e.__tvxH=1;e.style.setProperty('display','none','important');e.setAttribute(ATTR,'1');window.__tvxHid=(window.__tvxHid||0)+1;try{console.log('[tvx] hid paywall overlay',e.tagName,(e.getAttribute('data-name')||e.className||'').toString().slice(0,80))}catch(x){}try{if(document.body)document.body.style.removeProperty('overflow');document.documentElement.style.removeProperty('overflow')}catch(x){}}catch(x){}}function climb(e){var c=e,b=null;while(c&&c.parentNode&&c.parentNode!==document.documentElement){if(c===document.body||c.id==='root'||hasChart(c))break;var r=c.getBoundingClientRect();if(r.width>=vw()*0.8&&r.height>=vh()*0.8)b=c;c=c.parentElement}return b||e}function promo(e){try{if(!e||e.nodeType!==1)return 0;if(e===document.body||e===document.documentElement||e.id==='root')return 0;if(hasChart(e))return 0;if(!e.querySelector)return 0;if(!e.querySelector('button,[role=button],a[href],video,img,iframe'))return 0;var t=e.textContent||'';if(!t||t.length>8000)return 0;if(DIS.test(t))return 0;var k=tiers(t),buy=BUY.test(t),lim=LIM.test(t),br=BR.test(t),m=big(e);if(k>=2&&(buy||br||lim))return 1;if(br&&buy&&m)return 1;if(buy&&lim&&(m||k>=1))return 1;return 0}catch(x){return 0}}function bigFrame(e){try{if(!e||e.tagName!=='IFRAME')return 0;var r=e.getBoundingClientRect();return r.width>=vw()*0.35&&r.height>=vh()*0.35?1:0}catch(x){return 0}}var O={childList:!0,subtree:!0},mo=null,budget=0;function watch(r){try{mo.observe(r,O)}catch(x){}}function scan(n){try{if(!n||n.nodeType!==1)return;if(bigFrame(n)){hide(n);return}if(n.shadowRoot)watch(n.shadowRoot);if(budget>80)return;if(!n.querySelector)return;if(!n.querySelector('button,[role=button],a[href],video,img,iframe'))return;budget++;if(promo(n))hide(climb(n))}catch(x){}}function sweep(){try{var q=document.querySelectorAll('iframe,[role=dialog],dialog,[class*=overlay],[class*=modal],[class*=popup],[class*=paywall],[data-name*=paywall],[data-name*=dialog]'),i,e;for(i=0;i<q.length;i++){e=q[i];if(e.__tvxH)continue;if(bigFrame(e)){hide(e);continue}if(promo(e))hide(climb(e))}}catch(x){}}try{mo=new MutationObserver(function(ms){budget=0;var i,j,a;for(i=0;i<ms.length;i++){a=ms[i].addedNodes;for(j=0;j<a.length;j++)scan(a[j])}});mo.observe(document,O)}catch(x){}setTimeout(sweep,0);setTimeout(sweep,1500);setInterval(sweep,1e3)}catch(e){}})();\").catch((()=>{}))}))",
    },
    Patch {
        name: "广告/追踪域名网络拦截",
        name_en: "Ad/tracker domain blocking",
        group: Group::Ads,
        // 挂在既有 initSession 的 webRequest 配置处（onBeforeSendHeaders 之后，
        // will-download 注册之前）。实测当前图表页无广告 DOM，此为预防性网络层拦截，
        // 命中第三方广告网络（Google Ads 系/Taboola/Outbrain 等常见买量网络）时直接取消。
        old: "r({cancel:!1,requestHeaders:e.requestHeaders})})),e.on(\"will-download\",Zu)",
        new: "r({cancel:!1,requestHeaders:e.requestHeaders})})),e.webRequest.onBeforeRequest(((e,r)=>{r({cancel:/(doubleclick\\.net|googlesyndication\\.com|googleadservices\\.com|googletagservices\\.com|adservice\\.google\\.|taboola\\.com|outbrain\\.com|amazon-adsystem\\.com|adsrvr\\.org|rubiconproject\\.com)/.test(e.url)})})),e.on(\"will-download\",Zu)",
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
        old: "async signInWithBrowser(){const e=this.createAuthenticationUrl();this.authStateSubject.next({stage:$C.xE.WaitingToken})",
        new: "async signInWithBrowser(){const e=this.createAuthenticationUrl();console.log(\"TVX_AUTH_URL \"+e);this.authStateSubject.next({stage:$C.xE.WaitingToken})",
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
        name: "断线自动恢复",
        name_en: "Auto-recover from disconnect dialogs",
        group: Group::Recover,
        // 该弹窗由网页下发（本地包里没有这段文案），触发原因是服务器判定同账号并发超限。
        // 原来的做法是把弹窗直接藏掉，结果是窗口静默停更、用户完全看不出原因。
        // 现在改成自愈：先尝试点「恢复连接 / Reconnect」，4 秒仍未恢复就重载页面；
        // 之后按 15s/60s/5min 再试，最多 4 次，之后停手并让弹窗留着（把原因还给人）。
        // 连续 60 秒无弹窗则清零计数，避免用满次数后整个标签页不再自愈。
        // 这不改变服务端并发配额——配额在服务端强制，客户端没有可改的 gate。
        old: "handleDomReady(){this.isBrowserViewValid()&&(\"\"!==this.url&&",
        new: "handleDomReady(){this.isBrowserViewValid()&&(this.executeJavaScript(\"(function(){try{if(window.__tvxRecover)return;window.__tvxRecover=1;var K=/恢复连接|获取更多连接|关闭了这个连接|连接已关闭|连接已断开|關閉了這個連線|連線已中斷|We closed this connection|Connection closed|Get more connections|Verbindung wurde geschlossen|Verbindung getrennt|соединение закрыто|соединение было закрыто|この接続を閉じました|接続が切断されました|연결이 종료되었습니다|연결이 끊어졌습니다|conexión cerrada|conexión se ha cerrado|connexion fermée|connexion a été fermée|conexão fechada|conexão foi encerrada|bağlantı kapatıldı|bağlantı kesildi|kết nối đã bị đóng|kết nối đã ngắt|การเชื่อมต่อถูกปิด|koneksi ditutup|koneksi terputus|połączenie zostało zamknięte|connessione chiusa|connessione è stata chiusa|تم إغلاق الاتصال/i;var R=/恢复连接|重新连接|恢復連線|重新連線|Restore connection|Reconnect|Verbindung wiederherstellen|Erneut verbinden|Neu verbinden|Восстановить соединение|Переподключиться|接続を復元|再接続|연결 복원|다시 연결|Restaurar conexión|Reconectar|Rétablir la connexion|Reconnecter|Restaurar conexão|Reconectar|Bağlantıyı geri yükle|Yeniden bağlan|Khôi phục kết nối|Kết nối lại|เชื่อมต่อใหม่|กู้คืนการเชื่อมต่อ|Pulihkan koneksi|Hubungkan ulang|Przywróć połączenie|Połącz ponownie|Ripristina connessione|Riconnetti|إعادة الاتصال/i;var waits=[0,15000,60000,300000];function st(){try{return JSON.parse(sessionStorage.getItem('tvxRecover')||'{}')}catch(e){return {}}}function save(o){try{sessionStorage.setItem('tvxRecover',JSON.stringify(o))}catch(e){}}function dlg(){var q=document.querySelectorAll('[role=dialog],[data-name*=dialog],[class*=dialog]');for(var i=0;i<q.length;i++){var e=q[i],t=e.textContent||'';if(t.length<900&&K.test(t))return e}return null}function btn(d){var b=d.querySelectorAll('button,[role=button],a');for(var i=0;i<b.length;i++){var t=(b[i].textContent||'').trim();if(t&&t.length<40&&R.test(t))return b[i]}return null}var ok=0;function tick(){var d=dlg();if(!d){if(++ok>=75){var h=st();if(h.n||h.t)save({n:0,t:0});ok=0}return}ok=0;var s=st();if((s.n||0)>=4)return;d.setAttribute('data-tvx-recovering','1');if(!d.__tvxClicked){var b=btn(d);if(b){try{b.click()}catch(e){}d.__tvxClicked=1}}setTimeout(function(){if(!dlg())return;var o=st(),n=o.n||0;if(n>=4)return;var now=Date.now();if(now-(o.t||0)<(waits[n]||0))return;save({n:n+1,t:now});try{location.reload()}catch(e){}},4000)}setInterval(tick,800)}catch(e){}})();\").catch((()=>{})),\"\"!==this.url&&",
    },
    Patch {
        name: "放宽离线判定超时",
        name_en: "Relax the offline-detection timeout",
        group: Group::Silence,
        // 应用用一个 1 秒超时的 ping（data.tradingview.com/ping）判定在线/离线。
        // 走代理时一次握手就可能超过 1 秒，于是被误判成"离线"（并触发重连弹窗）。
        // 这里把默认超时从 1s 放宽到 10s。
        // 锚点带上 ping 域名：单看 `he=1e3,` 只有 7 个字符，压缩代码里任何一处
        // 1000ms 常量都可能撞车（"期望 1 处"会因多处命中而误判失配）。
        old: "\"https://data.tradingview.com/ping\",he=1e3,",
        new: "\"https://data.tradingview.com/ping\",he=1e4,",
    },
    Patch {
        name: "启动自动更新检查禁用",
        name_en: "Startup auto-update check disabled",
        group: Group::NoAutoUpdate,
        // 启动链里唯一一次自动检查（逗号表达式链的末位语句，结果值被丢弃）。
        // 插入恒真短路后 checkForUpdates 在任何平台都不会执行：appInstaller 永不填充，
        // setupUpdatesMonitoring 双重保险（if(!this.appInstaller)return）使更新定时器
        // 永不建立；设置面板手动检查/下载路径不受影响。Linux 官方原本就跳过自动更新。
        old: "(0,M.j9)()||op().checkForUpdates()",
        new: "(0,M.j9)()||!0||op().checkForUpdates()",
    },
    Patch {
        name: "本地指标报警引擎",
        name_en: "Local indicator alerts",
        group: Group::Alerts,
        // 报警引擎（图表页脚本）、设置页、主进程通知/邮件模块都放在 app 目录里，
        // 由 install_assets() 写入（见 alerts-engine.js / alerts-main.js）。
        // index.js 只负责把主进程模块挂上。锚点是 bundle 末尾的 sourceMappingURL 注释：
        // 用 `//#` 注释本身做 old、替换成 require（注释丢弃），这样 new 不含 old，
        // 重复 apply 不会叠加，撤回时注释原样还原。
        old: "//# sourceMappingURL=index.js.map",
        new: ";(function(){try{require(\"./tvx-alerts-main.js\")}catch(e){console.error(\"[tvx-alerts]\",e)}})();\n/* sourceMappingURL removed by tvhelper (local alerts module appended) */",
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

/// 定位「页内推广浮层」注入段的字节区间（PROMO_SEG_HEAD … PROMO_SEG_TAIL）。
fn promo_segment_range(src: &str) -> Option<(usize, usize)> {
    let start = src.find(PROMO_SEG_HEAD)?;
    let body = start + PROMO_SEG_HEAD.len();
    let tail = src[body..].find(PROMO_SEG_TAIL)?;
    Some((start, body + tail + PROMO_SEG_TAIL.len()))
}

/// 按开关对解包后的 index.js 应用/撤回补丁。任一处匹配数不为 1 就整体失败，
/// 避免"打了一半"的半残状态。
///
/// 语义：
/// - 开关开 + 未打过 → 打上（Applied）
/// - 开关开 + 已打过 → 不动（AlreadyPatched）
/// - 开关关 + 已打过 → 撤回 new→old（Reverted）
/// - 开关关 + 未打过  → 不动（Untouched）
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

    // 0c) 「页内推广浮层」同样按段迁移：v1（文案门 + 点关闭 + 1.2 秒轮询）与 v2
    //     （MutationObserver 插入即拦）文本完全不同，靠 old 精确匹配会失配，
    //     还会把已装好的副本判成"锚点未命中"。这里整段换成当前版本。
    if let Some(p) = PATCHES.iter().find(|p| p.new.contains("__tvxGuard")) {
        if let Some((s, e)) = promo_segment_range(&src) {
            let stale = {
                let seg = &src[s..e];
                seg != p.old && seg != p.new
            };
            if stale {
                let target = if p.group.enabled(flags) { p.new } else { p.old };
                src.replace_range(s..e, target);
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
const ALERTS_ENGINE_JS: &str = include_str!("alerts-engine.js");

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
pub fn install_assets(app_dir: &Path, flags: &Flags) -> Result<()> {
    let engine = app_dir.join("tvx-alerts-engine.js");
    let main = app_dir.join("tvx-alerts-main.js");
    let preload = app_dir.join("preload").join("platform-preload.js");

    if flags.alerts {
        fs::write(&engine, ALERTS_ENGINE_JS)
            .with_context(|| format!("写入 {}", engine.display()))?;
        fs::write(&main, ALERTS_MAIN_JS).with_context(|| format!("写入 {}", main.display()))?;

        if preload.is_file() {
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
        return Ok(());
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
    Ok(())
}

/// 检查 index.js 是否处于"当前开关组合所要求"的已打补丁状态：
/// 开着的补丁必须都在，关着的补丁必须都不在。
pub fn is_patched(index_js: &Path, flags: &Flags) -> bool {
    match fs::read_to_string(index_js) {
        Ok(s) => PATCHES.iter().all(|p| {
            // 精确锚点之外，再看通配锚点：上游只重排压缩标识符时，new/old 字面都不在
            // 文件里，但补丁其实打得是对的，不能误报"补丁异常"。
            let wild = WILD_ANCHORS.iter().find(|w| w.patch == p.name);
            let has_new = s.contains(p.new) || wild.is_some_and(|w| wild_find_all(&s, w.to).len() == 1);
            let has_old = s.contains(p.old) || wild.is_some_and(|w| wild_find_all(&s, w.from).len() == 1);
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
    }

    /// 历史注入文本（只隐藏版 / 只存内存计数的 v1）：apply() 必须能就地升级成当前脚本，
    /// 而不是报"锚点失配"。这是升级路径的回归测试。
    /// LEGACY 里的文本必须是"完整锚点段"——apply() 拿 p.new 整段去替换它。
    /// 若只存脚本本体，替换后新段会嵌在脚本位置，得到重复的 handleDomReady，
    /// 主进程加载即语法错误（真实踩过：TradingView 弹出只有 "Error" 的窗口）。
    #[test]
    fn legacy_entries_are_full_segments() {
        for (name, legacy) in LEGACY {
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

    /// v1（文案门 + 点关闭 + 轮询）→ v2（插入即拦）：已装好的副本里是 v1 文本，
    /// apply() 必须整段换掉，而不是报"锚点失配"——否则用户升级助手后还是要手工删副本。
    #[test]
    fn stale_promo_segment_is_migrated_whole() {
        let dir = std::env::temp_dir().join("tvx-patch-stale-promo-test");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("index.js");

        let p = PATCHES
            .iter()
            .find(|q| q.new.contains("__tvxGuard"))
            .expect("页内推广浮层补丁应在补丁集里");
        // 模拟 v1：同一个注入位置，脚本本体是旧版标记
        let stale = p.new.replace("window.__tvxGuard=1", "window.__tvxPromo=1");
        assert_ne!(stale, p.new, "模拟的历史脚本不能与当前脚本相同");

        let src: String = PATCHES
            .iter()
            .map(|q| {
                if q.new.contains("__tvxGuard") {
                    stale.as_str()
                } else {
                    q.new
                }
            })
            .collect();
        fs::write(&f, &src).unwrap();

        apply(&f, &Flags::default()).unwrap();
        let after = fs::read_to_string(&f).unwrap();
        assert!(!after.contains("__tvxPromo"), "v1 脚本应被整段换掉");
        assert!(after.contains("__tvxGuard"), "应迁移成 v2");
        assert_eq!(after.matches(PROMO_SEG_HEAD).count(), 1, "注入段必须只有一处");
        assert_eq!(after.matches(PROMO_SEG_TAIL).count(), 1, "注入段尾巴必须只有一处");
        assert!(is_patched(&f, &Flags::default()));

        // 关掉「屏蔽充值弹窗」：整段回退成原始锚点
        let off = Flags {
            silence_paywalls: false,
            ..Flags::default()
        };
        apply(&f, &off).unwrap();
        let after = fs::read_to_string(&f).unwrap();
        assert!(!after.contains("__tvxGuard"), "关掉后不应留注入");
        assert!(after.contains(p.old), "关掉后应回到原始 dom-ready 锚点");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_injections_are_migrated_in_place() {
        let dir = std::env::temp_dir().join("tvx-patch-legacy-test");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("index.js");

        for (_, legacy) in LEGACY {
            let mut src = String::new();
            for p in PATCHES {
                if p.group == Group::Recover {
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
            .items
            .iter()
            .all(|(_, st)| *st == Status::Applied || *st == Status::AlreadyPatched));
        assert!(read().contains(ads.new));
        assert!(read().contains(charts.new));
        assert!(read().contains(silence.new));
        assert!(read().contains(study.new));
        assert!(read().contains(noauto.new));

        // 2) 全关：可开关的组全部撤回。Auth 组恒开（多账号登录的前提），
        //    所以期望结果是「pristine + Auth 补丁」，而不是逐字 pristine。
        let all_off = Flags {
            ads_block: false,
            charts_unlock: false,
            study_on_study: false,
            silence_paywalls: false,
            no_auto_update: false,
            auto_recover: false,
            alerts: false,
        };
        // 只有 Auth 组生效时的目标文本
        let keep_auth: String = PATCHES
            .iter()
            .map(|p| if p.group == Group::Auth { p.new } else { p.old })
            .collect();
        write_pristine(&read()); // 当前是已打状态，先按现状
        let report = apply(&f, &all_off).unwrap();
        assert!(report.items.iter().all(|(_, st)| matches!(
            st,
            Status::Reverted | Status::Untouched | Status::AlreadyPatched
        )));
        assert_eq!(read(), keep_auth, "撤回后应只剩恒开的 Auth 补丁");
        for p in PATCHES.iter().filter(|p| p.group == Group::Auth) {
            assert!(read().contains(p.new), "{} 应被恒开保留", p.name);
        }
        assert!(is_patched(&f, &all_off), "全关状态下 is_patched 应为真");

        // 3) 部分开关：只开图表解锁 —— 其余组未启用，charts 打上
        write_pristine(&read()); // 已回到 keep_auth
        let only_charts = Flags {
            ads_block: false,
            charts_unlock: true,
            study_on_study: false,
            silence_paywalls: false,
            no_auto_update: false,
            auto_recover: false,
            alerts: false,
        };
        apply(&f, &only_charts).unwrap();
        let now = read();
        assert!(now.contains(charts.new));
        for p in PATCHES
            .iter()
            .filter(|p| p.group != Group::Charts && p.group != Group::Auth)
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

        // 用所有补丁的 old 拼出一份"原始" index.js
        let pristine: String = PATCHES.iter().map(|p| p.old).collect::<Vec<_>>().join("\n");
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
            ads_block: false,
            charts_unlock: false,
            study_on_study: false,
            silence_paywalls: false,
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
}
