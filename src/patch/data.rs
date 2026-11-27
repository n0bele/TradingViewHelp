//! 补丁数据：版本白名单、注入脚本原文、补丁表。
//!
//! 这里只有"事实"（哪个版本、哪段代码、锚点长什么样），不含任何匹配算法；
//! 匹配与改写逻辑在 `patch` 模块本体（`apply` / `is_patched` / 通配锚点）。
//! 换客户端版本时，通常只需要改这个文件。

use super::{Group, Patch};

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
pub const MULTI_CHART_UNLOCK: &str = "(function(){try{var n=0;(function p(){var c=window._exposed_chartWidgetCollection;if(c&&'function'==typeof c._checkProFeature){c._checkProFeature=function(e){return e};c._planChartsLimit=function(){return 99}}else{n++<120&&setTimeout(p,1e3)}})()}catch(e){}})();";

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
pub const STUDY_ON_STUDY_UNLOCK: &str = "(function(){try{var n=0;(function p(){var ok=!1;try{var c=Object.keys(window).filter(function(k){return /webpackChunk/i.test(k)});if(c.length){var rq=null;window[c[0]].push([[Date.now()%1e6],{},function(r){rq=r}]);if(typeof rq=='function'){for(var ids=Object.keys(rq.m),i=0;i<ids.length;i++){var s=String(rq.m[ids[i]]);if(!(s.indexOf('child_limit')>=0&&s.indexOf('getConfig:')>=0))continue;var m=rq(ids[i]);if(!m||typeof m.getConfig!='function')continue;var f=m.getConfig('STUDY_ON_STUDY');if(f&&typeof f=='object'){f.child_limit=99;ok=!0;break}}}}}catch(e){}if(!(ok||n>120)){n++;setTimeout(p,1e3)}})()}catch(e){}})();";

/// 注入到图表页（dom-ready 后轮询执行）的绘图数量上限解锁脚本。
///
/// 实测链路（CDP、登录态 free 账号、把 `Debugger.searchInContent` 打到
/// `static.tradingview.com/static/bundles/2958.*.js`）：
///   - 上限来源：`drawingsPerChartLimit()` 读计划配置表的
///     `DRAWINGS_ON_CHART.limit`（free 档 30），`>= UNLIMITED` 或 `undefined` 视为不限；
///   - 执法方：`ensureDrawingsLimitAllowed(limitObj, delta, source)` 的第一条就是
///     `limitObj===null || delta===0` 直接放行，否则才比 `counter.count()+delta <= limit`，
///     超了就 `trackEvent("Gopro Features","Drawings Limit")` + `openPaywall(PAYWALL_DRAWINGS_LIMIT)`；
///   - 调用点覆盖手绘 / 复制粘贴 / 克隆 / 图片拖入，全部走同一个执法函数。
///
/// 所以把「那个 limit 对象」变成 null 就等于全路径放行——比改计划表更稳（不动共享配置，
/// 不依赖 webpack 模块 id）。实测：改写后带 actionSource 连续建线，30 → 42 条不弹付费墙。
///
/// **只改 `chartModel` 自身的 `drawingsLimit`**：它和 `_checkProFeature` 一样是页面里
/// 每个图表实例共用的方法，实例级 + 原型级各打一次，后者保证同页后续新建的图表也生效。
/// 轮询到图表实例出现为止（最多约 2 分钟），拿不到就静默放弃——宁可没解锁，也不能报错。
///
/// **端到端实测（2026-11-24，登录态 free 账号 + 副本实例 + CDP）**：
///   - 注入生效：`window.__tvxDraw===1`、`chartModel.drawingsLimit()===null`；
///   - 上限确实被拆：同一张图表连续建到 **40 条**趋势线（14 → 54 个 dataSource），
///     全程没有 `PAYWALL_DRAWINGS_LIMIT` 弹窗（DOM 里无 paywall 节点）；
///   - **服务端存档也放行**：超额绘图触发 `PUT charts-storage/layout/<id>/sources`
///     返回 200、请求体里 39 条 `LineToolTrendLine`，整页重载后 39 条原样回来
///     （53 = 14 基础源 + 39 绘图）——服务端没有第二道 30 的闸门；
///   - 收尾用 `model.removeSource()` 逐条撤回并重载核对，布局回到 14 个基础源，
///     确认超额的增减都不影响原有内容。
///
/// 唯一注意点：这里的注入依赖「主进程能执行 executeJavaScript」，而 Node 侧
/// `NODE_OPTIONS` 里若带 `--require` 垫片（打包应用不支持多数 NODE_OPTION），
/// 注入会整体静默不执行——助手启动实例时本来就 `env_remove("NODE_OPTIONS")`，
/// 手工起实例排查时要照做，否则会误判成"补丁没生效"。
///
/// 常量仅在测试里用于逐字漂移检查（正式文本内嵌在补丁 new 里）。
#[cfg(test)]
pub const DRAWINGS_UNLOCK: &str = "(function(){try{if(window.__tvxDraw)return;window.__tvxDraw=1;var n=0,NO=function(){return null};function ok(){try{var c=window._exposed_chartWidgetCollection,aw=c&&c.activeChartWidget,w=aw&&typeof aw.value=='function'?aw.value():null,m=w&&typeof w.model=='function'?w.model():null,raw=m&&typeof m.model=='function'?m.model():null;if(!raw)return 0;try{if(typeof raw.drawingsLimit=='function')raw.drawingsLimit=NO}catch(e){}try{var pr=Object.getPrototypeOf(raw);if(pr&&typeof pr.drawingsLimit=='function')pr.drawingsLimit=NO}catch(e){}try{return typeof raw.drawingsLimit=='function'&&null===raw.drawingsLimit()?1:0}catch(e){return 0}}catch(e){return 0}}function p(){if(ok()||n>120)return;n++;setTimeout(p,1e3)}p()}catch(e){}})();";


/// 注入到图表页的回测报告能力解锁脚本（客户端 gate）。
///
/// **同一张表，不同 key**：`STUDY_ON_STUDY` 那条改的是套餐配置表里的配额字段，
/// 这里改的是同一张表（模块 86528，`getProductFeaturesConfig` / `getConfig` / `enabled`
/// 三个出口）里「有没有这个能力」的 key。free 档没有这个 key，付费档的值是**空对象**
/// （3.5.1.8500 的 `76108.*.js` 原文：`"BACKTESTING_EXTENDED_METRICS":{}`）。所以这里补的是
/// **与付费档完全同形的 `{}`**，不是随手一个真值。
///
/// **只放一项**：`BACKTESTING_EXTENDED_METRICS`（2026-11-26 真机实测，免费账号
/// `plan=free` + CDP）。报告里的指标行由 `ReportField.getLockState(enabled)`
///     决定，锁定时那一格换成锁按钮（`[class*=lockIconButton]`，title
///     「升级即可完全访问策略报告数据」），数值不出现。报告数据本身照常算出来，只决定画数字
///     还是画锁 → 纯显示层，可解。
///     bundle 里声明了 `_lockedMetrics` 的字段共 14 个，**逐个真机验过**：关掉 key 触发一次
///     重算后，这 14 格全部变成锁按钮（去重后恰为 14 个 lock）；打开 key 再重算，14 格全部
///     出数值。清单（页签 → 字段）：
///     明细：手续费成本；期刊：夏普比率、Sortino比率；基准：相关性（与买入持有相关性）；
///     保证金使用：总清算量；增长与衰退：最大回撤额占初始资本的百分比；
///     分布：最大盈利、最大亏损；连续：平均连胜纪录、平均连败纪录；
///     时间形态：最佳入场时间、最佳入场日期、最佳入场月份、平均交易持续时间。
///     （`bestHourForEntries` 是唯一例外：它在这份数据上 `value` 是 `undefined`——该维度
///     没样本。这一格一旦按 free 渲染成过锁，之后重算也不会重画（组件的 `value` props
///     始终是 `undefined`，渲染结果不变），界面上就可能一直停在锁；即使解锁，
///     渲染分支给的也只是 `—`。其余 13 项在同样条件下稳定出数值。）
///     **锁定态是在「报告重算」那一刻烘进去的**：只切「明细 ↔ 期刊」不会刷新，
///     必须改一个输入（佣金/初始资金）触发重算，否则会误判成「删了 key 没效果」。
///     **冷启动首屏也是锁态**：页面先画出上一次的报告，补 key 之后不会自动重算
///     （实测冷启动 5 个锁 → 改佣金重算 → 13 项出数值）。
/// **故意不含**（实测放行后只会让回测变成空结果，比不解锁更糟）：
///   - `BACKTESTING_EXPORT_CSV` / `BACKTESTING_EXPORT_XLSX`：判定确实在客户端——导出
///     按钮的点击体是 `enabled(EXPORT_CSV) ? f().then(埋点) : openPaywall(PAYWALL_…)`，
///     `f()` 用的就是页面上已有的 `reportsData`；运行时 A/B 调 `enabled('BACKTESTING_EXPORT_CSV')`
///     也确实 true→删→false→补→true。**但 3.5.1.8500 的报告里根本没渲染出这个按钮**
///     （全文档 0 个 `downloadButton-PgtNiNYX`、0 个挂着 `BACKTESTING_EXPORT` 处理器的元素，
///     报告有数据时也没有），用户点不到 → 放了等于没放，所以不保留这两个 key。
///     哪天某个布局真把按钮画出来，再把它们加回 `FE` 即可。
///   - `BAR_DETALIZATION`（K线精度→高精度，每根K线约 32 tick）：
///     未放行时该选项**看得见但没有 lock 标记**，点下去无任何反应——因为条目 onClick 是
///     `e ? runOrGoProWithoutSync(PAYWALL_BAR_DETALIZATION, 回调, ProductFeatures.BAR_DETALIZATION)
///     : 直接赋值`，拿不到能力就走付费墙分支，而付费墙又被子系统拦截器隐藏了。
///     运行时把 key 补进 free 后重试：label 从「默认精度」变成「高精度」，报告立刻变
///     「该策略目前尚未进行任何交易」，底部跳出「升级到Premium方案，即可获得更高的K线细节」。
///     → 客户端放行只是让选项可选，算不出来的原因是服务端。
///   - `SCRIPT_EXECUTION_ON_HISTORY_BARS`（脚本执行→在历史K线tick）：
///     未放行时条目 HTML 一直是 `box-… unchecked`，点了不勾——onClick 是 `m?.(!d)`，
///     `m` 为空就是空调用。补上 key 后勾选生效，报告同样立刻变空。
///   - `DEEP_HISTORY_BACKTEST`：深回测走独立 WebSocket（`WEBSOCKET_HOST_FOR_DEEP_BACKTESTING`），
///     `set_auth_token` 后服务端回 `not_allowed`，客户端只能删会话。
/// 这三项都由服务端算力/数据决定，客户端改 flag 只会得到「能点、点了没结果」。
///
/// 为什么直接取表而不是扫源码特征：`getProductFeaturesConfig(plan)` 在免费档
/// 返回的就是 `c.free` 这个共享对象本身（`S(e)`：有 `extends` 才拷贝，free 没有），
/// 所以对返回值赋值等于改表；`getConfig`/`enabled` 每次调用都现读该表，改完立刻生效。
///
/// 定位方式分两步，都是只读现有模块、不新造状态：
///   1. 从 webpack 模块缓存 `rq.c` 里找 exports 带 `getProductFeaturesConfig` 的模块（便宜）；
///   2. 缓存里没有（模块还没被实例化）就退到源码扫描 + `rq(id)` 实例化，每 5 秒一次。
///
/// 常量仅在测试里用于逐字漂移检查（正式文本内嵌在补丁 new 里）。
#[cfg(test)]
pub const BACKTEST_UNLOCK: &str = "(function(){try{if(window.__tvxBT)return;window.__tvxBT=1;var FE=['BACKTESTING_EXTENDED_METRICS'];var n=0,rq=null;function R(){if(rq)return rq;try{var k=Object.keys(window).filter(function(x){return /webpackChunk/i.test(x)})[0];if(!k)return null;var r=null;window[k].push([[Date.now()%1e6],{},function(q){r=q}]);if(typeof r=='function')rq=r}catch(e){}return rq}function hit(e){try{if(!e||typeof e.getProductFeaturesConfig!='function')return 0;var f=e.getProductFeaturesConfig('free');if(!f||typeof f!='object')return 0;for(var j=0;j<FE.length;j++){if(!f[FE[j]])f[FE[j]]={}}return 1}catch(e){return 0}}function cached(){var r=R();if(!r)return 0;var c=r.c||{},ids=Object.keys(c),i;for(i=0;i<ids.length;i++){var e;try{e=c[ids[i]]&&c[ids[i]].exports}catch(x){continue}if(hit(e))return 1}return 0}function deep(){var r=R();if(!r)return 0;var ids=Object.keys(r.m||{}),i,s;for(i=0;i<ids.length;i++){try{s=String(r.m[ids[i]])}catch(x){continue}if(s.indexOf('getProductFeaturesConfig')<0)continue;var e;try{e=r(ids[i])}catch(x){continue}if(hit(e))return 1}return 0}function tick(){var ok=0;try{ok=cached()||(n>2&&n%5==0&&deep())}catch(e){}if(ok){window.__tvxBT=2;try{console.log('[tvx] backtest features unlocked')}catch(e){}return}if(n>120)return;n++;setTimeout(tick,1e3)}tick()}catch(e){}})();";
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
pub const DISCONNECT_RECOVER: &str = "(function(){try{if(window.__tvxRecover)return;window.__tvxRecover=1;var K=/恢复连接|关闭了这个连接|连接已关闭|连接已断开|關閉了這個連線|連線已中斷|We closed this connection|Connection closed|Verbindung wurde geschlossen|Verbindung getrennt|соединение закрыто|соединение было закрыто|この接続を閉じました|接続が切断されました|연결이 종료되었습니다|연결이 끊어졌습니다|conexión cerrada|conexión se ha cerrado|connexion fermée|connexion a été fermée|conexão fechada|conexão foi encerrada|bağlantı kapatıldı|bağlantı kesildi|kết nối đã bị đóng|kết nối đã ngắt|การเชื่อมต่อถูกปิด|koneksi ditutup|koneksi terputus|połączenie zostało zamknięte|connessione chiusa|connessione è stata chiusa|تم إغلاق الاتصال/i;var R=/恢复连接|重新连接|恢復連線|重新連線|Restore connection|Reconnect|Verbindung wiederherstellen|Erneut verbinden|Neu verbinden|Восстановить соединение|Переподключиться|接続を復元|再接続|연결 복원|다시 연결|Restaurar conexión|Reconectar|Rétablir la connexion|Reconnecter|Restaurar conexão|Reconectar|Bağlantıyı geri yükle|Yeniden bağlan|Khôi phục kết nối|Kết nối lại|เชื่อมต่อใหม่|กู้คืนการเชื่อมต่อ|Pulihkan koneksi|Hubungkan ulang|Przywróć połączenie|Połącz ponownie|Ripristina connessione|Riconnetti|إعادة الاتصال/i;var waits=[0,15000,60000,300000];function st(){try{return JSON.parse(sessionStorage.getItem('tvxRecover')||'{}')}catch(e){return {}}}function save(o){try{sessionStorage.setItem('tvxRecover',JSON.stringify(o))}catch(e){}}function dlg(){var q=document.querySelectorAll('[role=dialog],[data-name*=dialog],[class*=dialog]');for(var i=0;i<q.length;i++){var e=q[i],t=e.textContent||'';if(t.length<900&&K.test(t))return e}return null}function btn(d){var b=d.querySelectorAll('button,[role=button],a');for(var i=0;i<b.length;i++){var t=(b[i].textContent||'').trim();if(t&&t.length<40&&R.test(t))return b[i]}return null}var ok=0;function tick(){var d=dlg();if(!d){if(++ok>=75){var h=st();if(h.n||h.t)save({n:0,t:0});ok=0}return}ok=0;var s=st();if((s.n||0)>=4)return;d.setAttribute('data-tvx-recovering','1');if(!d.__tvxClicked){var b=btn(d);if(b){try{b.click()}catch(e){}d.__tvxClicked=1}}setTimeout(function(){if(!dlg())return;var o=st(),n=o.n||0;if(n>=4)return;var now=Date.now();if(now-(o.t||0)<(waits[n]||0))return;save({n:n+1,t:now});try{location.reload()}catch(e){}},4000)}setInterval(tick,800)}catch(e){}})();";

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
pub const PROMO_GUARD: &str = "(function(){try{if(window.__tvxGuard)return;window.__tvxGuard=1;var ATTR='data-tvx-hidden';var BUY=/upgrade|pricing|subscribe|subscription|plans?|升级|升級|订阅|訂閱|方案|套餐|abonn|подписк|тариф|プラン|요금|업그레이드/i;var LIM=/limit|maximum|reached|exceed|no more|上限|最大|最多|已达|已達|限制|превыш|достиг/i;var TIER=/basic|essential|premium|ultimate|advanced/gi;var BR=/tradingview/i;var DIS=/恢复连接|关闭了这个连接|连接已关闭|连接已断开|Restore connection|We closed this connection|Connection closed/i;function vw(){return Math.max(document.documentElement.clientWidth||0,window.innerWidth||0)}function vh(){return Math.max(document.documentElement.clientHeight||0,window.innerHeight||0)}function tiers(s){var m=s.match(TIER),o={},c=0,i,k;if(!m)return 0;for(i=0;i<m.length;i++){k=m[i].toLowerCase();if(!o[k]){o[k]=1;c++}}return c}function hasChart(e){try{return !!(e&&e.querySelector&&e.querySelector('canvas'))}catch(x){return !1}}function big(e){try{if(!e||!e.querySelectorAll)return 0;var q=e.querySelectorAll('video,img,iframe'),n=q.length>20?20:q.length,i,area=vw()*vh()*0.05,r;for(i=0;i<n;i++){r=q[i].getBoundingClientRect();if(r.width*r.height>=area)return 1}return 0}catch(x){return 0}}function hide(e){try{if(!e||e.nodeType!==1||e.__tvxH)return;if(e.closest&&e.closest('#tvx-alerts-host'))return;if(e===document.body||e===document.documentElement||e.id==='root')return;if(hasChart(e))return;e.__tvxH=1;e.style.setProperty('display','none','important');e.setAttribute(ATTR,'1');window.__tvxHid=(window.__tvxHid||0)+1;try{console.log('[tvx] hid paywall overlay',e.tagName,(e.getAttribute('data-name')||e.className||'').toString().slice(0,80))}catch(x){}try{if(document.body)document.body.style.removeProperty('overflow');document.documentElement.style.removeProperty('overflow')}catch(x){}}catch(x){}}function climb(e){var c=e,b=null;while(c&&c.parentNode&&c.parentNode!==document.documentElement){if(c===document.body||c.id==='root'||hasChart(c))break;var r=c.getBoundingClientRect();if(r.width>=vw()*0.8&&r.height>=vh()*0.8)b=c;c=c.parentElement}return b||e}function promo(e){try{if(!e||e.nodeType!==1)return 0;if(e.closest&&e.closest('#tvx-alerts-host'))return 0;if(e===document.body||e===document.documentElement||e.id==='root')return 0;if(hasChart(e))return 0;if(!e.querySelector)return 0;if(!e.querySelector('button,[role=button],a[href],video,img,iframe'))return 0;var t=e.textContent||'';if(!t||t.length>8000)return 0;if(DIS.test(t))return 0;var k=tiers(t),buy=BUY.test(t),lim=LIM.test(t),br=BR.test(t),m=big(e);if(k>=2&&(buy||br||lim))return 1;if(br&&buy&&m)return 1;if(buy&&lim&&(m||k>=1))return 1;return 0}catch(x){return 0}}function bigFrame(e){try{if(!e||e.tagName!=='IFRAME')return 0;var r=e.getBoundingClientRect();return r.width>=vw()*0.35&&r.height>=vh()*0.35?1:0}catch(x){return 0}}var O={childList:!0,subtree:!0},mo=null,budget=0;function watch(r){try{mo.observe(r,O)}catch(x){}}function scan(n){try{if(!n||n.nodeType!==1)return;if(bigFrame(n)){hide(n);return}if(n.shadowRoot)watch(n.shadowRoot);if(budget>80)return;if(!n.querySelector)return;if(!n.querySelector('button,[role=button],a[href],video,img,iframe'))return;budget++;if(promo(n))hide(climb(n))}catch(x){}}function sweep(){try{var q=document.querySelectorAll('iframe,[role=dialog],dialog,[class*=overlay],[class*=modal],[class*=popup],[class*=paywall],[data-name*=paywall],[data-name*=dialog]'),i,e;for(i=0;i<q.length;i++){e=q[i];if(e.__tvxH)continue;if(bigFrame(e)){hide(e);continue}if(promo(e))hide(climb(e))}}catch(x){}}try{mo=new MutationObserver(function(ms){budget=0;var i,j,a;for(i=0;i<ms.length;i++){a=ms[i].addedNodes;for(j=0;j<a.length;j++)scan(a[j])}});mo.observe(document,O)}catch(x){}setTimeout(sweep,0);setTimeout(sweep,1500);setInterval(sweep,1e3)}catch(e){}})();";

/// 追加到 `preload/platform-preload.js` 的推广守卫标记与脚本。
///
/// 为什么在 preload 里做：图表页里那个"在TradingView电脑版分析市场"的升级墙
/// 是**页面自己的定时器**，登录且非 Pro 时 `setTimeout(..., 600000)`（10 分钟）
/// 后调 `openPaywall('desktopPaidOnlyAccess')`，并且配置里写死 `closable:!1`
/// ——弹出来就没有关闭按钮。页面脚本里这段判断是：
///
/// ```js
/// isDesktopApp() && isFeatureEnabled("tvd_paid_only_access") && is_authenticated && !is_pro
///   && setTimeout(() => openPaywall(PAYWALL_DESKTOP_PAID_ONLY_ACCESS, ...), 6e5)
/// ```
///
/// `isFeatureEnabled` 会先读 `localStorage["forcefeaturetoggle.<name>"]`：
/// 值为 `"false"` 时直接判定功能关闭（这是应用自带的 `FeatureToggle.disableFeature`
/// 通道，见前端 `d()` 的实现）。所以只要在**页面脚本执行之前**把这个键写进去，
/// 定时器根本不会挂上。preload 正好比页面脚本早，且这里不碰任何 DOM、
/// 不猜文案、不限面积，只关掉这一个"自动弹墙"的开关。
///
/// 只关这一个 key：它管的正是这条推广墙；用户点「升级 / Get real-time」
/// 走的是别的链路（`openWall` + 用户手势），不受影响。
pub const PROMO_GUARD_MARK: &str = "/*tvx-promo-guard*/";

pub const PROMO_GUARD_JS: &str = "\n;(function(){/*tvx-promo-guard*/try{var L=window.localStorage;if(!L)return;L.setItem(\"forcefeaturetoggle.tvd_paid_only_access\",\"false\")}catch(e){}})();\n";

/// 上一版（计数只存内存，重载即清零）。已打过的副本要先迁移到当前版本。
/// handleDomReady 段的开头（补丁 new 与迁移文本都必须以它起头）
pub const DOM_READY_HEAD: &str =
    "handleDomReady(){this.isBrowserViewValid()&&(this.executeJavaScript(\"";

/// `handleDomReady` 段的两块稳定边界文本：注入脚本之后固定是 `").catch((()=>{}))`，
/// 再接 URL 判断；判断各版本写法不同（`this.url&&` / `""!==this.url&&`），但都以
/// `this.url&&` 收尾。所以段尾只要认这两块，不必按版本枚举。
pub const RECOVER_SCRIPT_TAIL: &str = "\").catch((()=>{})),";
pub const RECOVER_URL_TAIL: &str = "this.url&&";

/// `.on("dom-ready",…)` 的原始形态，Core 补丁的 old。
pub const DOM_READY_ORIG: &str = ".on(\"dom-ready\",(()=>{this.handleDomReady()}))";

/// 轮询壳里留给注入的"空位"：Core 补丁的 new 写成 `…t.handleDomReady(),void 0;…`，
/// 推广补丁把这一小段替换成 `…,void t.executeJavaScript("…")`。
/// 空位与填充分开，是为了让「等视图就绪」这件事不再挂在任何功能开关上。
// 只给测试用：迁移回归要按这一小段空位造"老副本"。
#[allow(dead_code)]
pub const SHELL_HOOK: &str = "t.handleDomReady(),void 0";

/// `.on("dom-ready",…)` 的轮询壳：等 BrowserView 就绪再调 `handleDomReady()`，
/// 每 250ms 试一次、最多 1200 次（5 分钟）。
///
/// **为什么必须等**：首次 dom-ready 触发时 `browserViewMember` 常常还是 null，
/// 而 `handleDomReady()` 的函数体（断线自愈补丁插入的）以 `this.isBrowserViewValid()&&(`
/// 开头——视图无效时整段短路且**不再重试**，于是断线自愈、多图表/指标/绘图注入、
/// 标题更新与任务通知订阅会一起丢掉几十秒（实测启动后 48~56s 页面没有任何注入）。
/// `__tvxDR` 是代次令牌：`dom-ready` 再触发（导航）时旧循环自动退出，
/// 保证同一页面只执行一次 `handleDomReady()`（改动前也是这个语义）。
pub const DOM_READY_SHELL: &str = ".on(\"dom-ready\",(()=>{var t=this,n=0,g=t.__tvxDR=(t.__tvxDR||0)+1;(function r(){if(g!==t.__tvxDR)return;if(t.isBrowserViewValid())return n>0&&console.log(\"[tvx] dom-ready ready after \"+n+\" retries\"),t.handleDomReady(),void 0;n<1200?((0===n&&console.log(\"[tvx] dom-ready: browser view not ready, retrying\"),n++,setTimeout(r,250))):console.warn(\"[tvx] dom-ready: browser view still invalid after 300s\")})()}))";

/// 历史形态（v2：`handleDomReady()` 与注入耦合在同一个回调里），只用于识别待迁移的旧副本。
pub const PROMO_SEG_HEAD_V2: &str = ".on(\"dom-ready\",(()=>{this.handleDomReady(),this.isBrowserViewValid()&&this.executeJavaScript(\"";

/// 待迁移段的识别前缀：v3 初版（壳与注入耦合，壳以 `var t=this` 开头）与 v2。
/// 用前缀而不是整段，是为了让壳自身的小改动（加日志、加令牌）都能被认出来。
pub const PROMO_SEG_HEADS: &[&str] = &[
    ".on(\"dom-ready\",(()=>{var t=this",
    PROMO_SEG_HEAD_V2,
];

/// 注入脚本的收尾标记：迁移测试用它确认脚本只落一处（配平算段尾不依赖它）。
#[cfg(test)]
pub const PROMO_SEG_END_MARK: &str = "\").catch((()=>{}))";

pub const LEGACY_RECOVER_V1: &str = r#"handleDomReady(){this.isBrowserViewValid()&&(this.executeJavaScript("(function(){try{if(window.__tvxRecover)return;window.__tvxRecover=1;var K=/恢复连接|获取更多连接|关闭了这个连接|We closed this connection|Restore connection|Get more connections/i;var n=0,last=0,waits=[15000,60000,300000];function dlg(){var q=document.querySelectorAll('[role=dialog],[data-name*=dialog],[class*=dialog]');for(var i=0;i<q.length;i++){var e=q[i],t=e.textContent||'';if(t.length<600&&K.test(t))return e}return null}function btn(d){var b=d.querySelectorAll('button,[role=button],a');for(var i=0;i<b.length;i++){var t=(b[i].textContent||'').trim();if(/恢复连接|重新连接|Restore connection|Reconnect/i.test(t))return b[i]}return null}function tick(){var d=dlg();if(!d)return;d.setAttribute('data-tvx-recovering','1');var b=btn(d);if(b){try{b.click()}catch(e){}}setTimeout(function(){if(!dlg())return;if(n>=3)return;var now=Date.now();if(now-last<(waits[n]||3e5))return;last=now;n++;try{location.reload()}catch(e){}},4000)}setInterval(tick,800)}catch(e){}})();").catch((()=>{})),this.url&&"#;

/// 旧版本注入过的「只隐藏不恢复」脚本。已装好的副本里是这段文本，
/// apply() 会先把它升级成新脚本（或按开关撤回成原始锚点），
/// 否则新版 old 锚点对不上，会被判成"锚点失配"而整体失败。
pub const LEGACY_HIDE_DISCONNECT: &str = r#"handleDomReady(){this.isBrowserViewValid()&&(this.executeJavaScript("(function(){try{if(window.__tvxNoDisconnect)return;window.__tvxNoDisconnect=1;var K=/恢复连接|获取更多连接|关闭了这个连接|We closed this connection|Restore connection|Get more connections/i;function s(){var n=document.querySelectorAll('[role=dialog],[data-name*=dialog],[class*=dialog]');for(var i=0;i<n.length;i++){var e=n[i],t=e.textContent||'';if(t.length<600&&K.test(t)){e.style.display='none';e.setAttribute('data-tvx-hidden','1')}}}setInterval(s,800)}catch(e){}})();").catch((()=>{})),this.url&&"#;

/// 上一版「页面开窗过滤」写进副本的文本（过滤正则里还没有 `paywall` / `pro-plans`）。
/// 这与注入脚本无关，但同样属于"副本里已是旧文本、新 old 锚点对不上"的情形：
/// 条件插在 `return;` 与 `const o` 之间，原来的 old 已经被拆开，只能按整条换。
pub const LEGACY_OPEN_WINDOW_V1: &str = "null==n||!1===(0,M.HU)(n))return;if(/(pricing|subscribe|subscription|billing|upgrade|go-?pro|promo|trial)/i.test(String(n)))return;const o={url:n};";

/// 再上一版：把 `paywall` / `pro-plans` 也一并拦掉的那一版。
/// 它把用户主动点「升级 / Get real-time」要开的付费页也拦了，现在收窄成只拦 `promo`。
pub const LEGACY_OPEN_WINDOW_V2: &str = "null==n||!1===(0,M.HU)(n))return;if(/(pricing|subscribe|subscription|billing|upgrade|go-?pro|promo|trial|paywall|pro-?plans)/i.test(String(n)))return;const o={url:n};";

/// 更早一版广告网络拦截：只断了 `/api/v1/offers/` 与常见买量域名，
/// 还没有拦 `paywall-desktop-paid-only-access` 那个自动弹墙模块。
pub const LEGACY_ADBLOCK_V1: &str = "r({cancel:!1,requestHeaders:e.requestHeaders})})),e.webRequest.onBeforeRequest(((e,r)=>{r({cancel:/(doubleclick\\.net|googlesyndication\\.com|googleadservices\\.com|googletagservices\\.com|adservice\\.google\\.|tradingview\\.com\\/api\\/v1\\/offers\\/|taboola\\.com|outbrain\\.com|amazon-adsystem\\.com|adsrvr\\.org|rubiconproject\\.com)/.test(e.url)})})),e.on(\"will-download\",Zu)";

/// 广告网络拦截的上一版：已经拦了 `paywall-desktop-paid-only-access`，
/// 但还没有拦「脚本计算超时」那面自动墙。老副本里是这一版，必须按整条就地升级，
/// 否则新 old 锚点对不上 → 报「锚点未命中」，网络层整条打不进去。
/// 再往前只拦 `/api/v1/offers/` 的那一版见 [`LEGACY_ADBLOCK_V1`]。
pub const LEGACY_ADBLOCK_V2: &str = "r({cancel:!1,requestHeaders:e.requestHeaders})})),e.webRequest.onBeforeRequest(((e,r)=>{r({cancel:/(doubleclick\\.net|googlesyndication\\.com|googleadservices\\.com|googletagservices\\.com|adservice\\.google\\.|tradingview\\.com\\/api\\/v1\\/offers\\/|paywall-desktop-paid-only-access|taboola\\.com|outbrain\\.com|amazon-adsystem\\.com|adsrvr\\.org|rubiconproject\\.com)/.test(e.url)})})),e.on(\"will-download\",Zu)";

/// 需要就地迁移的历史注入文本：(所属补丁名, 旧文本)
pub const LEGACY: &[(&str, &str)] = &[
    ("断线自动恢复", LEGACY_HIDE_DISCONNECT),
    ("断线自动恢复", LEGACY_RECOVER_V1),
    ("页面请求开窗时拦截推广链接", LEGACY_OPEN_WINDOW_V1),
    ("页面请求开窗时拦截推广链接", LEGACY_OPEN_WINDOW_V2),
    ("广告域名与推广数据网络拦截", LEGACY_ADBLOCK_V1),
    ("广告域名与推广数据网络拦截", LEGACY_ADBLOCK_V2),
];

/// 已下线、需要从老副本里"退役"的补丁：(说明, 已打形态, 原始形态)。
///
/// 与 [`LEGACY`] 方向相反：那里是"旧文本升级成新文本"，这里是"规则本身被判定误伤，
/// 无条件还原成原文"。开关没有关，所以走不到 `apply()` 里"开关关 → new 换回 old"的撤回分支，
/// 必须单独做一遍。
///
/// 为什么下线：这三条拦的是「用户点功能 → 没有权限 → 弹付费墙」，
/// 全部由用户操作触发（`requestProFeature` 是 `featureRequester` 包装器在访问被拒时调的），
/// 用户点「Get real-time」这类按钮会完全没反应。自动推广走 `/api/v1/offers/`，
/// 由网络层负责，不需要动这条链路。
pub const RETIRED: &[(&str, &str, &str)] = &[
    (
        "订阅遮罩窗口不再创建",
        "async function vL(e,t){return;const n=(0,Le.Qh)().getLastFocusedWindow();",
        "async function vL(e,t){const n=(0,Le.Qh)().getLastFocusedWindow();",
    ),
    (
        "showPaywall 变空操作",
        "showPaywall(e,t){return Promise.resolve()}",
        "showPaywall(e,t){return vL(e,t)}",
    ),
    (
        "网页内充值窗口不再注入",
        "async requestProFeature(){return;const e=(0,Le.Qh)().getLastFocusedWindow();",
        "async requestProFeature(){const e=(0,Le.Qh)().getLastFocusedWindow();",
    ),
];

pub const PATCHES: &[Patch] = &[
    // 恒开、排在最前：其余页内补丁的注入都挂在这层壳后面，必须先就位。
    Patch {
        name: "dom-ready 就绪重试壳",
        name_en: "dom-ready readiness retry shell",
        group: Group::Core,
        old: DOM_READY_ORIG,
        new: DOM_READY_SHELL,
    },
    // 这里原本还有三条壳层补丁：「订阅遮罩窗口不再创建（vL）」「showPaywall 变空操作」
    // 「网页内充值窗口不再注入（requestProFeature）」。
    //
    // 它们拦的其实不是广告，而是**用户主动点功能、没有权限、于是弹付费墙**这条链路：
    // `requestProFeature` 由 `featureRequester` 包装器在 `isFeatureAllowed()===false`
    // 时调用（见 `AC()`），`showPaywall` 再往下才创建窗口。三条一起上，用户点
    // 「Get real-time」这类按钮就完全没反应——这就是"误拦截手动点击"。
    //
    // 自动推广不走这里：它是页面拿 `/api/v1/offers/` 的活动数据自己渲染的，
    // 由下面的网络层拦数据源即可。老副本里残留的这三条由 [`RETIRED`] 无条件还原。
    Patch {
        name: "页面请求开窗时拦截推广链接",
        name_en: "Block promo windows requested by the page",
        group: Group::AdBlock,
        // 页面（web app）可以通过 IPC OPEN_NEW_WINDOW 让壳开任意 URL 的新窗口。
        // 只拦**推广**链接（`promo`）；`pricing`/`paywall`/`pro-plans`/`upgrade`
        // 这些一律放行——那些窗口是用户自己点「升级 / Get real-time」才会请求的，
        // 拦掉等于把用户主动点的功能吃掉（2026-11-27 用户实测反馈）。
        old: "null==n||!1===(0,M.HU)(n))return;const o={url:n};",
        new: "null==n||!1===(0,M.HU)(n))return;if(/promo/i.test(String(n)))return;const o={url:n};",
    },
    Patch {
        name: "广告域名与推广数据网络拦截",
        name_en: "Block ad domains and the promo offer feed",
        group: Group::AdBlock,
        // 挂在既有 initSession 的 webRequest 配置处（onBeforeSendHeaders 之后，
        // will-download 注册之前）。命中第三方广告网络（Google Ads 系/Taboola/Outbrain
        // 等常见买量网络）与推广数据源时直接取消。
        //
        // 三条与本项目直接相关：
        // - `tradingview.com/api/v1/offers/`：页面拿活动数据自己渲染的整屏推广；
        // - `paywall-desktop-paid-only-access`：桌面端"专属功能"升级墙的 JS 模块。
        //   它是 10 分钟定时器自动弹、且 `closable:!1`（没有关闭按钮）的那种。
        //   墙的加载器 `openWall` 在模块加载失败时只写日志并返回 `{shown:!1}`
        //   （实测前端 bundle 59818 里的实现），所以拦住模块 = 不弹，且不会报错弹窗。
        //   这是一层兜底：主路径是 preload 里关掉 `tvd_paid_only_access` 开关
        //   （见 `PROMO_GUARD_JS`），正常根本不会请求这个模块。
        // - `paywall-script-calculation-time`：**加载图表就自动弹**的整屏墙
        //   （实测每次导航后 4.9 秒出现，1152×830、`position:fixed`、没有关闭按钮，
        //   只能按 Esc 退）。触发条件在服务端：图上的脚本超出本方案的算力上限，
        //   服务端回错误 → 前端把 `scriptCalculationTime` 映射成这个墙
        //   （bundle 25597 的 `E()`）。客户端解锁改不了算力，所以这里只拦"弹窗"，
        //   A/B 实测：拦掉 → 0 个浮层、图表中心仍是 canvas、无报错弹窗；
        //   放行 → 每次加载都弹。其余 `paywall-*` 模块（用户点功能才弹的那批）一律放行。
        old: "r({cancel:!1,requestHeaders:e.requestHeaders})})),e.on(\"will-download\",Zu)",
        new: "r({cancel:!1,requestHeaders:e.requestHeaders})})),e.webRequest.onBeforeRequest(((e,r)=>{r({cancel:/(doubleclick\\.net|googlesyndication\\.com|googleadservices\\.com|googletagservices\\.com|adservice\\.google\\.|tradingview\\.com\\/api\\/v1\\/offers\\/|paywall-desktop-paid-only-access|paywall-script-calculation-time|taboola\\.com|outbrain\\.com|amazon-adsystem\\.com|adsrvr\\.org|rubiconproject\\.com)/.test(e.url)})})),e.on(\"will-download\",Zu)",
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
        name: "绘图数量解锁注入（客户端 gate）",
        name_en: "Drawings-per-chart unlock injection (client gate)",
        group: Group::Drawings,
        // 锚点选在 handleDomReadyForNonLocalFiles 的收尾（`allowTitleUpdate` 之后、
        // 提示音开关之前）——这条链上「多图表」用 insertCSS 那段、「指标叠加」用函数首语句，
        // 三处互不重叠，任一条不开都不影响其余两条命中。
        // 注入脚本把图表模型的 drawingsLimit() 打空（null 是执法函数的直接放行条件）。
        old: "this.allowTitleUpdate(),Ge().allowAlertNotificationsSoundOnce()",
        new: "this.allowTitleUpdate(),this.executeJavaScript(\"(function(){try{if(window.__tvxDraw)return;window.__tvxDraw=1;var n=0,NO=function(){return null};function ok(){try{var c=window._exposed_chartWidgetCollection,aw=c&&c.activeChartWidget,w=aw&&typeof aw.value=='function'?aw.value():null,m=w&&typeof w.model=='function'?w.model():null,raw=m&&typeof m.model=='function'?m.model():null;if(!raw)return 0;try{if(typeof raw.drawingsLimit=='function')raw.drawingsLimit=NO}catch(e){}try{var pr=Object.getPrototypeOf(raw);if(pr&&typeof pr.drawingsLimit=='function')pr.drawingsLimit=NO}catch(e){}try{return typeof raw.drawingsLimit=='function'&&null===raw.drawingsLimit()?1:0}catch(e){return 0}}catch(e){return 0}}function p(){if(ok()||n>120)return;n++;setTimeout(p,1e3)}p()}catch(e){}})();\").catch((()=>{})),Ge().allowAlertNotificationsSoundOnce()",
    },
    Patch {
        name: "回测扩展指标解锁（客户端 gate）",
        name_en: "Backtest extended metrics unlock (client gate)",
        group: Group::Backtest,
        // 锚点取 handleDomReadyForNonLocalFiles 里 `updateViewZoom()` 那句（全文唯一）：
        // 与本链上其余三条（指标叠加用的函数首语句、多图表用的 insertCSS 段、
        // 绘图用的 allowTitleUpdate 尾巴）互不重叠，任一开关关闭都不影响其余命中。
        // 插入的脚本改的是套餐配置表的共享对象（free 档返回的就是表本身），
        // 只补 `BACKTESTING_EXTENDED_METRICS` 这一个 key——导出判定虽在客户端但没有
        // 界面入口，K线精度/历史tick/深回测由服务端算力决定（实测打开只得到空报告 +
        // 升级提示），详见 BACKTEST_UNLOCK 的注释。
        old: "this.getURL()!==za.TU&&this.updateViewZoom(),(0,M.iT)()&&",
        new: "this.getURL()!==za.TU&&this.updateViewZoom(),this.executeJavaScript(\"(function(){try{if(window.__tvxBT)return;window.__tvxBT=1;var FE=['BACKTESTING_EXTENDED_METRICS'];var n=0,rq=null;function R(){if(rq)return rq;try{var k=Object.keys(window).filter(function(x){return /webpackChunk/i.test(x)})[0];if(!k)return null;var r=null;window[k].push([[Date.now()%1e6],{},function(q){r=q}]);if(typeof r=='function')rq=r}catch(e){}return rq}function hit(e){try{if(!e||typeof e.getProductFeaturesConfig!='function')return 0;var f=e.getProductFeaturesConfig('free');if(!f||typeof f!='object')return 0;for(var j=0;j<FE.length;j++){if(!f[FE[j]])f[FE[j]]={}}return 1}catch(e){return 0}}function cached(){var r=R();if(!r)return 0;var c=r.c||{},ids=Object.keys(c),i;for(i=0;i<ids.length;i++){var e;try{e=c[ids[i]]&&c[ids[i]].exports}catch(x){continue}if(hit(e))return 1}return 0}function deep(){var r=R();if(!r)return 0;var ids=Object.keys(r.m||{}),i,s;for(i=0;i<ids.length;i++){try{s=String(r.m[ids[i]])}catch(x){continue}if(s.indexOf('getProductFeaturesConfig')<0)continue;var e;try{e=r(ids[i])}catch(x){continue}if(hit(e))return 1}return 0}function tick(){var ok=0;try{ok=cached()||(n>2&&n%5==0&&deep())}catch(e){}if(ok){window.__tvxBT=2;try{console.log('[tvx] backtest features unlocked')}catch(e){}return}if(n>120)return;n++;setTimeout(tick,1e3)}tick()}catch(e){}})();\").catch((()=>{})),(0,M.iT)()&&",
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
        // `K`（弹窗文案）里**不再包含「获取更多连接 / Get more connections」**：那是
        // 连接数付费墙的标题（`PAYWALL_CONNECTIONS_LIMIT`），留着会让自愈把付费墙当成断线，
        // 找不到「恢复连接」按钮就重载页面，最多 4 次——用户看到的是"莫名重载"。
        // 真正的断线框标题是「关闭了这个连接 / We closed this connection」，两条都还在，
        // 且按这条路径已经实弹验证过（假断线框 → 按钮点击 1 次、重载 1 次）。
        // 这不改变服务端并发配额——配额在服务端强制，客户端没有可改的 gate。
        old: "handleDomReady(){this.isBrowserViewValid()&&(\"\"!==this.url&&",
        new: "handleDomReady(){this.isBrowserViewValid()&&(this.executeJavaScript(\"(function(){try{if(window.__tvxRecover)return;window.__tvxRecover=1;var K=/恢复连接|关闭了这个连接|连接已关闭|连接已断开|關閉了這個連線|連線已中斷|We closed this connection|Connection closed|Verbindung wurde geschlossen|Verbindung getrennt|соединение закрыто|соединение было закрыто|この接続を閉じました|接続が切断されました|연결이 종료되었습니다|연결이 끊어졌습니다|conexión cerrada|conexión se ha cerrado|connexion fermée|connexion a été fermée|conexão fechada|conexão foi encerrada|bağlantı kapatıldı|bağlantı kesildi|kết nối đã bị đóng|kết nối đã ngắt|การเชื่อมต่อถูกปิด|koneksi ditutup|koneksi terputus|połączenie zostało zamknięte|connessione chiusa|connessione è stata chiusa|تم إغلاق الاتصال/i;var R=/恢复连接|重新连接|恢復連線|重新連線|Restore connection|Reconnect|Verbindung wiederherstellen|Erneut verbinden|Neu verbinden|Восстановить соединение|Переподключиться|接続を復元|再接続|연결 복원|다시 연결|Restaurar conexión|Reconectar|Rétablir la connexion|Reconnecter|Restaurar conexão|Reconectar|Bağlantıyı geri yükle|Yeniden bağlan|Khôi phục kết nối|Kết nối lại|เชื่อมต่อใหม่|กู้คืนการเชื่อมต่อ|Pulihkan koneksi|Hubungkan ulang|Przywróć połączenie|Połącz ponownie|Ripristina connessione|Riconnetti|إعادة الاتصال/i;var waits=[0,15000,60000,300000];function st(){try{return JSON.parse(sessionStorage.getItem('tvxRecover')||'{}')}catch(e){return {}}}function save(o){try{sessionStorage.setItem('tvxRecover',JSON.stringify(o))}catch(e){}}function dlg(){var q=document.querySelectorAll('[role=dialog],[data-name*=dialog],[class*=dialog]');for(var i=0;i<q.length;i++){var e=q[i],t=e.textContent||'';if(t.length<900&&K.test(t))return e}return null}function btn(d){var b=d.querySelectorAll('button,[role=button],a');for(var i=0;i<b.length;i++){var t=(b[i].textContent||'').trim();if(t&&t.length<40&&R.test(t))return b[i]}return null}var ok=0;function tick(){var d=dlg();if(!d){if(++ok>=75){var h=st();if(h.n||h.t)save({n:0,t:0});ok=0}return}ok=0;var s=st();if((s.n||0)>=4)return;d.setAttribute('data-tvx-recovering','1');if(!d.__tvxClicked){var b=btn(d);if(b){try{b.click()}catch(e){}d.__tvxClicked=1}}setTimeout(function(){if(!dlg())return;var o=st(),n=o.n||0;if(n>=4)return;var now=Date.now();if(now-(o.t||0)<(waits[n]||0))return;save({n:n+1,t:now});try{location.reload()}catch(e){}},4000)}setInterval(tick,800)}catch(e){}})();\").catch((()=>{})),\"\"!==this.url&&",
    },
    Patch {
        name: "放宽离线判定超时",
        name_en: "Relax the offline-detection timeout",
        group: Group::AdBlock,
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

