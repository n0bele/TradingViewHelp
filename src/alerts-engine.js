/*
 * TVHelper —— 本地指标报警（图表页引擎 + 报警设置页）
 *
 * 设计约束（来自需求）：
 *   1. 只做「指标报警」，不做价格报警；
 *   2. 条件必须绑定在「当前图表标签」上：品种 + 周期 + 该图上真实存在的指标（含参数）；
 *   3. 设置页结构对齐 TradingView 原生报警对话框：条件 / 周期与触发 / 消息 / 通知；
 *   4. 消息支持占位符（{{ticker}} {{open}} {{close}} ...），标题与内容都可自定义；
 *   5. 通知走系统弹窗和/或邮件，邮件可按报警覆盖发件人与收件人；
 *   6. 设置持久化，重启后自动恢复；如果图表不存在 / 品种周期不符 / 指标不在图上，
 *      只标记「未就绪」，绝不硬触发。
 *
 * 数据全部来自图表自身的模型（实测于 3.5.0.8375）：
 *   window.TradingViewApi.activeChart()             → symbol() / resolution() / getAllStudies()
 *   chart.getStudyById(id)._study.data()            → PlotList，valueAt(i) = [time, plot0, plot1, ...]
 *   study._valuesProvider.getValues()               → [{index, title, visible}]，用于曲线命名
 *   chart._chartWidget.model().mainSeries().bars()  → PlotList，valueAt(i) = [time, open, high, low, close, volume]
 * 注意 PlotList 的 size() 含左侧填充，必须用 lastIndex() 取最后一根。
 */
(function () {
  'use strict';
  if (window.__tvxAlerts) return;
  var BR = window.__TVX_ALERTS__;
  if (!BR || typeof BR.get !== 'function') return;
  window.__tvxAlerts = '1';

  var TICK_MS = 1500;
  var CFG_RELOAD_MS = 60000;
  var CFG = null;
  var LOADED_AT = 0;
  var RUNTIME = Object.create(null); // alertId -> {bar, firedForBar, status, value}
  var UI = null;
  var BANNER_TIMER = 0;
  var HOT_TIMER = 0;

  // 数值型输出（line / histogram / area / columns）才需要比较方式与阈值。
  // 离散型输出（colorer，如 SuperTrend 的涨跌着色）只有「变成哪个值」，
  // 没有阈值可言——那是指标自己算好的状态，不是连续数值。
  var CONDS = [
    { id: 'cross_up', label: '上穿' },
    { id: 'cross_down', label: '下穿' },
    { id: 'gt', label: '大于' },
    { id: 'lt', label: '小于' },
    { id: 'in_range', label: '进入区间' },
    { id: 'out_range', label: '离开区间' },
    { id: 'appear', label: '出现（从无到有）', noValue: true },
    { id: 'disappear', label: '消失（从有到无）', noValue: true }
  ];
  var COND_CHANGED = 'changed_to';

  function isNum(v) {
    return typeof v === 'number' && isFinite(v);
  }

  function condNeedsValue(id) {
    for (var i = 0; i < CONDS.length; i++) if (CONDS[i].id === id) return !CONDS[i].noValue;
    return id !== COND_CHANGED;
  }

  // 占位符清单（顺序即设置页里按钮的排列顺序）
  var PLACEHOLDERS = [
    'ticker', 'interval', 'open', 'high', 'low', 'close', 'volume',
    'time', 'study', 'plot', 'condition', 'threshold', 'value'
  ];

  var DEFAULT_TITLE = '{{ticker}} {{interval}} · {{study}} 报警';
  var DEFAULT_BODY = '{{ticker}} {{interval}} {{study}} · {{plot}} {{condition}} {{threshold}}\n当前 {{close}}（开 {{open}} 高 {{high}} 低 {{low}}）';

  function condLabel(id) {
    if (id === COND_CHANGED) return '变为';
    for (var i = 0; i < CONDS.length; i++) if (CONDS[i].id === id) return CONDS[i].label;
    return id;
  }

  // ---------------------------------------------------------------- 图表访问

  function chartApi() {
    try {
      var a = window.TradingViewApi;
      if (!a || typeof a.activeChart !== 'function') return null;
      return a.activeChart();
    } catch (e) {
      return null;
    }
  }

  function chartCtx() {
    var a = chartApi();
    if (!a) return null;
    var sym = '';
    var res = '';
    try { sym = String(a.symbol() || ''); } catch (e) {}
    try { res = String(a.resolution() || ''); } catch (e) {}
    if (!sym) return null;
    return { a: a, symbol: sym, resolution: res };
  }

  function plotList(st) {
    if (!st) return null;
    try {
      var d = typeof st.data === 'function' ? st.data() : st._data;
      if (d && typeof d.valueAt === 'function' && typeof d.lastIndex === 'function') return d;
    } catch (e) {}
    return null;
  }

  /** 主序列最后一根 K 线：{time, open, high, low, close, volume} */
  function mainBar(ctx) {
    if (!ctx) return null;
    try {
      var a = ctx.a;
      var model = a._chartWidget && typeof a._chartWidget.model === 'function' ? a._chartWidget.model() : null;
      var ms = model && typeof model.mainSeries === 'function' ? model.mainSeries() : null;
      var b = ms && typeof ms.bars === 'function' ? ms.bars() : null;
      if (!b && typeof a.getSeries === 'function') {
        var s = a.getSeries();
        b = s && typeof s.bars === 'function' ? s.bars() : null;
      }
      if (!b) return null;
      var i = typeof b.lastIndex === 'function' ? b.lastIndex() : -1;
      if (i < 0) return null;
      var v = typeof b.valueAt === 'function' ? b.valueAt(i) : null;
      if (!v || typeof v[0] !== 'number') return null;
      return { time: v[0], open: v[1], high: v[2], low: v[3], close: v[4], volume: v[5] };
    } catch (e) {
      return null;
    }
  }

  /** study 的 metainfo（WatchedValue 拆包）：plots / styles / palettes / inputs 都在这里 */
  function metaValue(st) {
    try {
      var w = st && st._metaInfo;
      if (!w) return null;
      if (typeof w.value === 'function') return w.value();
      return w._value || w;
    } catch (e) {
      return null;
    }
  }

  /** plot 类型 → 条件类别：离散（colorer）、图形（plotshape，不占数据列）、数值 */
  function kindOf(type) {
    if (type === 'colorer' || type === 'text_colorer') return 'discrete';
    if (type === 'plotshape' || type === 'plotchar' || type === 'shapes' || type === 'shape' || type === 'chars') return 'shape';
    if (type === 'alertcondition') return 'condition';
    return 'numeric';
  }

  // 会产出数据列的 plot 类型（其余类型不占列，列映射时必须先剔除，否则整体错位）
  var COL_TYPES = {
    line: 1, linebr: 1, stepline: 1, histogram: 1, area: 1, columns: 1,
    colorer: 1, text_colorer: 1, data: 1
  };

  function isColType(t) {
    return !!COL_TYPES[String(t || 'line')];
  }

  // 条件名归一化：去标点、去这类「后缀词」，让 "Bullish Trend Shift" 能对上 "Bullish Trend"
  var NAME_STOP = {
    shift: 1, shifts: 1, signal: 1, signals: 1, trigger: 1, triggers: 1,
    alert: 1, alerts: 1, condition: 1, conditions: 1, the: 1, of: 1, and: 1
  };

  function nameWords(s) {
    var raw = String(s || '').toLowerCase().split(/[^a-z0-9\u4e00-\u9fa5]+/);
    var out = [];
    for (var i = 0; i < raw.length; i++) if (raw[i] && !NAME_STOP[raw[i]]) out.push(raw[i]);
    return out;
  }

  /** 两个名字的相似度：100 = 归一化后完全相同；0 = 无关 */
  function nameScore(a, b) {
    var wa = nameWords(a);
    var wb = nameWords(b);
    var x = wa.join('');
    var y = wb.join('');
    if (!x || !y) return 0;
    if (x === y) return 100;
    if (x.indexOf(y) === 0 || y.indexOf(x) === 0) {
      return 80 + 10 * (Math.min(x.length, y.length) / Math.max(x.length, y.length));
    }
    var hit = 0;
    for (var i = 0; i < wa.length; i++) if (wb.indexOf(wa[i]) >= 0) hit++;
    if (!hit) return 0;
    var jac = hit / (wa.length + wb.length - hit);
    return jac >= 0.5 ? 40 + 40 * jac : 0;
  }

  function plotTitleOf(def, idx, titles, styles) {
    if (titles[idx]) return titles[idx];
    var s = styles && styles[def && def.id];
    if (s && s.title) return s.title;
    return (def && def.id) || ('输出 ' + (idx + 1));
  }

  /** colorer 的调色板：值 → 颜色，用来把 0/1/2 显示成可辨认的色块 */
  function paletteOf(mi, def) {
    try {
      var name = def && def.palette;
      var pal = name ? (mi.palettes || {})[name] : null;
      var colors = pal && pal.colors;
      if (!colors) return null;
      return function (v) {
        var c = colors[String(v)];
        if (!c) return null;
        return c.color || (typeof c === 'string' ? c : null);
      };
    } catch (e) {
      return null;
    }
  }

  /** 某列最近 n 根的取值（用于枚举离散输出的历史取值；空值与 NaN 要剔除） */
  function columnHistory(d, col, n) {
    var arr = [];
    try {
      var li = d.lastIndex();
      var from = Math.max(0, li - n + 1);
      for (var i = from; i <= li; i++) {
        var v = d.valueAt(i);
        var cv = v ? v[col] : undefined;
        if (cv === undefined || cv === null || cv === '') continue;
        if (typeof cv === 'number' && !isFinite(cv)) continue;
        arr.push(cv);
      }
    } catch (e) {}
    return arr;
  }

  /** 单个数据列的描述（含离散列的历史取值枚举） */
  function makeItem(def, k, v, d, mi, styles, titles) {
    var kind = kindOf(def && def.type);
    var item = {
      index: k,
      id: String((def && def.id) || ('col' + k)),
      title: plotTitleOf(def, k, titles, styles),
      type: String((def && def.type) || 'line'),
      kind: kind,
      value: v ? v[k + 1] : undefined
    };
    if (kind === 'discrete') {
      var hist = columnHistory(d, k + 1, 400);
      var seen = Object.create(null);
      var choices = [];
      for (var h = 0; h < hist.length; h++) {
        var key = String(hist[h]);
        if (!seen[key]) { seen[key] = 0; choices.push(hist[h]); }
        seen[key]++;
      }
      item.choices = choices.slice(0, 12);
      item.counts = seen;
      item.color = paletteOf(mi, def);
    }
    return item;
  }

  /**
   * 某列是不是「na 交替」型：有明显的空段（上下轨这种只在一边有值），
   * 且不是全程为空。判据用 15% 以上为空——SuperTrend 上下轨大约各占一半。
   */
  function isNaAlternating(d, col, n) {
    try {
      var li = d.lastIndex();
      var from = Math.max(0, li - (n || 300) + 1);
      var total = 0;
      var na = 0;
      var has = 0;
      for (var i = from; i <= li; i++) {
        total++;
        var v = d.valueAt(i);
        if (!isNum(v ? v[col] : null)) na++;
        else has++;
      }
      return total >= 3 && has > 0 && na / total >= 0.15;
    } catch (e) {
      return false;
    }
  }

  // 方向词白名单（按词判断，避免 "Support" 里的 up 这种误命中）
  var UP_WORDS = { bull: 1, bullish: 1, long: 1, longs: 1, buy: 1, up: 1, uptrend: 1, upmove: 1 };
  var DOWN_WORDS = { bear: 1, bearish: 1, short: 1, shorts: 1, sell: 1, down: 1, downtrend: 1, downmove: 1 };

  function dirTokens(s) {
    return String(s || '')
      .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
      .toLowerCase()
      .split(/[^a-z0-9\u4e00-\u9fa5]+/)
      .filter(function (w) { return !!w; });
  }

  function dirOfName(s) {
    var w = dirTokens(s);
    for (var i = 0; i < w.length; i++) {
      if (UP_WORDS[w[i]] || /多|涨/.test(w[i])) return 'up';
      if (DOWN_WORDS[w[i]] || /空|跌/.test(w[i])) return 'down';
    }
    return null;
  }

  function dirOfTitle(s) {
    return dirOfName(s);
  }

  /**
   * 把一个 alertcondition 落到客户端真能读到的信号上。
   *   1) 它自己占数据列       → 直接读布尔（真值边沿）
   *   2) 不占列 → 找同名输出：形状列 → 出现；状态列 → 切换；na 交替的线 → 出现
   *   3) 名字对不上 → 用方向词（多/空）配 na 交替的上下轨
   *   4) 都不行 → 标记不可用（服务端条件，客户端算不出来）
   */
  function buildCond(cd, title, ci, items, condCols, defs, d) {
    var c = {
      id: String(cd.id), index: ci, title: title, source: 'alertcondition',
      kind: 'signal', col: null, mode: 'none', available: false, note: '', linkedTitle: ''
    };
    if (condCols) {
      var pos = 0;
      for (var i = 0; i < defs.length; i++) {
        var dd = defs[i] || {};
        if (dd.id === cd.id) break;
        var dt2 = String(dd.type || 'line');
        if (isColType(dt2) || kindOf(dt2) === 'shape' || dt2 === 'alertcondition') pos++;
      }
      c.col = pos;
      c.mode = 'truthy';
      c.kind = 'bool';
      c.available = true;
      c.note = '脚本条件，直接读取指标输出的布尔信号列。';
      return c;
    }
    var best = null;
    var bestScore = 0;
    for (var k = 0; k < items.length; k++) {
      var it = items[k];
      var s = nameScore(title, it.title);
      if (it.kind === 'shape') s *= 1.15; // 脚本里 alertcondition 与 plotshape 常常同源
      if (s > bestScore) { bestScore = s; best = it; }
    }
    if (best && bestScore >= 50) {
      if (best.kind === 'shape') {
        c.linkedTitle = best.title;
        c.col = best.index;
        c.mode = 'appear';
        c.available = true;
        c.note = '对应图上信号「' + best.title + '」出现，与原生报警同源。';
        return c;
      }
      if (best.kind === 'discrete') {
        c.linkedTitle = best.title;
        c.col = best.index;
        c.mode = 'changed';
        c.kind = 'signal_discrete';
        c.available = true;
        c.note = '对应状态列「' + best.title + '」切换。';
        return c;
      }
      if (isNaAlternating(d, best.index + 1)) {
        c.linkedTitle = best.title;
        c.col = best.index;
        c.mode = 'appear';
        c.available = true;
        c.note = '对应图上分段线「' + best.title + '」出现。';
        return c;
      }
    }
    // 名字对不上：先找同方向的图形信号（"Bullish Trend Shift" ↔ plotshape "Bullish Trend"），
    // 那是脚本画出来的翻转箭头，与 alertcondition 同一条表达式，语义最准
    var dir = dirOfName(title);
    if (dir) {
      for (var sm = 0; sm < items.length; sm++) {
        var it3 = items[sm];
        if (it3.kind !== 'shape') continue;
        if (dirOfTitle(it3.title) !== dir) continue;
        c.linkedTitle = it3.title;
        c.col = it3.index;
        c.mode = 'appear';
        c.available = true;
        c.note = '对应图上信号「' + it3.title + '」出现，与原生报警同源。';
        return c;
      }
    }
    // 再退回方向词配 na 交替的上下轨（Bullish Trend Shift ↔ upTrend）
    if (dir) {
      for (var m = 0; m < items.length; m++) {
        var it2 = items[m];
        if (it2.kind !== 'numeric') continue;
        if (dirOfTitle(it2.title) !== dir) continue;
        if (!isNaAlternating(d, it2.index + 1)) continue;
        c.linkedTitle = it2.title;
        c.col = it2.index;
        c.mode = 'appear';
        c.available = true;
        c.note = '对应图上「' + it2.title + '」出现（脚本用同一方向量算出来的）。';
        return c;
      }
    }
    c.note = best
      ? ('最接近的输出是「' + best.title + '」，还原不出这个条件，客户端不能计算。')
      : '脚本在服务端定义了这个条件，但图上没有能对应上的输出列，客户端算不出来。';
    return c;
  }

  // ---------------------------------------------------------- 脚本条件（源码兜底）
  //
  // 免费账号的客户端 metainfo 不一定带 alertcondition 条目（技术报警属于付费能力），
  // 这时按指标名去公开脚本库找同名脚本、读它的源码，把 alertcondition(...) 的条件名
  // 原样取出来。名字与原生「创建警报」对话框一致，客户端再按名字映射到图上可读的信号。
  // 版本号变了就作废旧缓存：v1 曾把 AlgoAlpha 的条件错误地挂到别人的同名脚本上
  var PINE_KEY = 'tvx-pine-conds-v2';
  var PINE_MEM = null;
  var PINE_BUSY = Object.create(null);
  var PINE_FAIL = Object.create(null);

  function pineCache() {
    if (PINE_MEM) return PINE_MEM;
    try { PINE_MEM = JSON.parse(localStorage.getItem(PINE_KEY) || '{}') || {}; } catch (e) { PINE_MEM = {}; }
    return PINE_MEM;
  }
  function pineSave() {
    try { localStorage.setItem(PINE_KEY, JSON.stringify(pineCache())); } catch (e) {}
  }
  function normName(s) {
    return String(s || '').toLowerCase().replace(/\[[^\]]*\]/g, '').replace(/[^a-z0-9\u4e00-\u9fa5]+/g, '');
  }
  /** 脚本名末尾的 [作者]，例如 "Machine Learning Adaptive SuperTrend [MergeAlerts]" */
  function authorOf(name) {
    var m = /\[\s*([^\[\]]+?)\s*\]\s*$/.exec(String(name || ''));
    return m ? m[1] : '';
  }
  /** 像 Pine 脚本名（够长且有空格）；Volume / RSI / MACD 这类内置名不会命中 */
  function looksLikeScript(name) {
    var n = String(name || '');
    return n.length >= 10 && /\s/.test(n);
  }
  /** 解析 Pine 源码里的 alertcondition 条件名（去重，保持源码顺序） */
  function parsePineConds(src) {
    var out = [];
    var seen = Object.create(null);
    // 表达式里本身可能带逗号（ta.crossunder(dir, 0)），用贪婪匹配到行内最后一个 , "标题"
    // 第三个参数（alert.freq_once_per_bar_close 之类）要允许存在，
    // 只要求标题字符串之后到行尾括号闭合
    var re = /^[ \t]*alertcondition\s*\((.*),\s*["']([^"']+)["'][^)]*\)/gm;
    var m;
    while ((m = re.exec(src))) {
      var t = String(m[2] || '').trim();
      if (!t || seen[t]) continue;
      seen[t] = 1;
      out.push({ title: t, expr: String(m[1] || '').trim() });
    }
    return out;
  }

  /**
   * 直接按脚本 id 取它自己的源码，再解析 alertcondition 名字。
   *
   * 这是唯一不会认错脚本的路径：图表上的指标名可能被别人改过（
   * "Machine Learning Adaptive SuperTrend [MergeAlerts]" 就不是 AlgoAlpha 那个公开脚本），
   * 按名字搜索会命中同名的公开脚本，条件名对不上。脚本 id 来自图表的元数据本身。
   * pine-facade 在 .tradingview.com 域下，页面会话 cookie 会带上；邀请制脚本只要账号有权限就能读。
   */
  function fetchSourceConds(sid, cb) {
    var key = 'id:' + sid;
    var cache = pineCache();
    if (cache[key]) { cb(cache[key]); return; }
    if (PINE_BUSY[key]) return;
    PINE_BUSY[key] = true;
    var enc = encodeURIComponent(sid);
    var done = function (list) {
      PINE_BUSY[key] = false;
      if (!list) PINE_FAIL[key] = true;
      cb(list || null);
    };
    fetch('https://pine-facade.tradingview.com/pine-facade/versions/' + enc + '/last', { credentials: 'include' })
      .then(function (r) { return r.json(); })
      .then(function (v) {
        var ver = '';
        if (Object.prototype.toString.call(v) === '[object Array]' && v.length) ver = v[0].version;
        else if (v && v.version) ver = v.version;
        if (!ver) throw new Error('no-version');
        return fetch('https://pine-facade.tradingview.com/pine-facade/get/' + enc + '/' + encodeURIComponent(ver),
          { credentials: 'include' }).then(function (r) { return r.json(); });
      })
      .then(function (meta) {
        var list = parsePineConds(String((meta && meta.source) || ''));
        if (!list.length) return null;
        cache[key] = list;
        var sn = String((meta && meta.scriptName) || '');
        if (sn) cache[sn] = list; // 同名命中，后续直接可用
        pineSave();
        return list;
      })
      .then(done)
      .catch(function () { done(null); });
  }

  /** 按名字去公开脚本库找（只对公开脚本有效；名字带 [作者] 时作者必须一致） */
  function fetchPineConds(name, cb) {
    var cache = pineCache();
    if (cache[name]) { cb(cache[name]); return; }
    if (PINE_BUSY[name]) return;
    PINE_BUSY[name] = true;
    var want = normName(name);
    var wantAuthor = normName(authorOf(name));
    var done = function (list) {
      PINE_BUSY[name] = false;
      if (!list) PINE_FAIL[name] = true;
      cb(list || null);
    };
    var q = String(name).replace(/\[[^\]]*\]/g, ' ').replace(/[^\w\s.-]+/g, ' ').replace(/\s+/g, ' ').trim();
    fetch('/pubscripts-suggest-json/?search=' + encodeURIComponent(q))
      .then(function (r) { return r.json(); })
      .then(function (j) {
        var arr = (j && (j.results || j)) || [];
        var hit = null;
        var loose = null;
        for (var i = 0; i < arr.length; i++) {
          var it = arr[i];
          var cands = [it.scriptName, it.title, it.shortTitle];
          var nameOk = false;
          for (var c2 = 0; c2 < cands.length; c2++) {
            var n = normName(cands[c2]);
            if (!n || n.length < 6) continue;
            if (n === want || n.indexOf(want) >= 0 || want.indexOf(n) >= 0) { nameOk = true; break; }
          }
          if (!nameOk) continue;
          var au = normName((it.author && (it.author.username || it.author.name)) || '');
          // 图表名带 [作者]：作者不一致就是别人改过的副本，条件名不能拿它的
          if (wantAuthor && au && au !== wantAuthor) { if (!loose) loose = it; continue; }
          hit = it;
          break;
        }
        if (!hit && !wantAuthor) hit = loose;
        if (!hit) return null;
        var sid = hit.scriptIdPart || hit.id;
        return new Promise(function (res) { fetchSourceConds(sid, res); });
      })
      .then(done)
      .catch(function () { done(null); });
  }

  /**
   * 指标自己的输出列 + 它自己声明的报警条件。
   *
   * 实测结论（3.5.0.8375 + 服务端元数据）：
   *   - `_metaInfo.plots` 里 type==='alertcondition' 的条目，就是脚本里 alertcondition() 的
   *     个数与顺序；它们的**名字**在 `_metaInfo.styles[plotId].title`，例如
   *     "Bullish Trend Shift" / "Bearish Trend Shift" / "High Volatility"。
   *     这正是原生「创建警报」对话框条件下拉的来源——旧版只按数据列列条件，所以列出一堆
   *     无关输出，却漏掉脚本真正声明的那几条。
   *   - alertcondition 不画线、不进 valuesProvider；是否占数据列按实际列数判定：
   *     cols === 可画类型数 → 不占列（实测如此）；cols === 全部 plots 数 → 占列。
   *   - 不占列时用同名/近名输出列还原：脚本里 alertcondition 常与 plotshape 用同一表达式
   *     （如 `ta.crossunder(dir, 0)`），形状列「出现」即等价于该条件成立。
   */
  function studyMeta(a) {
    var out = [];
    var list = [];
    try { list = a.getAllStudies() || []; } catch (e) { return out; }
    for (var i = 0; i < list.length; i++) {
      var id = list[i] && list[i].id;
      var name = String((list[i] && list[i].name) || '');
      if (!id || !name) continue;
      var row = {
        id: String(id), name: name, inputs: [], plots: [], conds: [],
        ok: false, note: '', serverConds: 0, condCols: false,
        needPine: false, condFromSource: false,
        scriptId: '', fullId: '', miKeys: '', pending: false
      };
      try {
        var api = a.getStudyById(id);
        if (api && typeof api.getInputValues === 'function') {
          var iv = api.getInputValues() || [];
          for (var j = 0; j < iv.length; j++) row.inputs.push({ id: String(iv[j].id), value: iv[j].value });
        }
        var st = api && api._study;
        var d = plotList(st);
        var mi = metaValue(st) || {};
        var defs = mi.plots || [];
        var styles = mi.styles || {};
        // 脚本 id：元数据里一定有，用来直接读「这个脚本自己」的源码，而不是同名公开脚本
        try {
          row.miKeys = Object.keys(mi).join(',');
          row.fullId = String(mi.fullId || '');
          var miJson = JSON.stringify(mi);
          var sm = /(?:^|[^A-Za-z0-9])((?:PUB|USER);[0-9a-fA-F]{32})/.exec(miJson);
          if (sm) row.scriptId = sm[1];
          else if (/^(PUB|USER);[0-9a-fA-F]{32}/.test(row.fullId)) row.scriptId = row.fullId.split('@')[0];
          // 自检用：把元数据里任何带 alert/cond 的字段原样记下来，
          // 不同编译版本的客户端可能把条件放在别处（plots / alertConditions / graphics…）
          var ash = [];
          try {
            for (var ak in mi) {
              if (!/alert|cond/i.test(ak)) continue;
              var av = '';
              try { av = JSON.stringify(mi[ak]).slice(0, 400); } catch (e2) { av = String(mi[ak]); }
              ash.push(ak + '=' + av);
            }
          } catch (e3) {}
          row.alertish = ash;
        } catch (e) {}
        if (d) {
          var li = d.lastIndex();
          var v = li >= 0 ? d.valueAt(li) : null;
          if (v && v.length >= 2) {
            row.ok = true;
            var titles = [];
            try {
              var pv = st._valuesProvider && typeof st._valuesProvider.getValues === 'function'
                ? st._valuesProvider.getValues()
                : null;
              if (pv && pv.length) {
                for (var p = 0; p < pv.length; p++) {
                  var pi = typeof pv[p].index === 'number' ? pv[p].index : pv[p].orderIndex;
                  if (typeof pi === 'number') titles[pi] = pv[p].title;
                }
              }
            } catch (e) {}

            var cols = v.length - 1;
            var colDefs = [];
            var condDefs = [];
            // 可能占数据列的类型：数值/状态列 + 图形列（plotshape 要画在某个价位上，
            // 客户端必须收到坐标，所以它同样有数据列）
            var dataDefs = [];
            for (var di = 0; di < defs.length; di++) {
              var dt = String((defs[di] && defs[di].type) || 'line');
              if (dt === 'alertcondition') { condDefs.push(defs[di]); continue; }
              if (isColType(dt)) colDefs.push(defs[di]);
              if (isColType(dt) || kindOf(dt) === 'shape') dataDefs.push(defs[di]);
            }
            row.serverConds = condDefs.length;

            // 列映射：按「谁的数量对得上实际列数」来判定，不靠猜
            var cands = [{ defs: dataDefs, condCols: false }];
            if (condDefs.length) cands.push({ defs: defs, condCols: true });
            if (colDefs.length && colDefs.length !== dataDefs.length) {
              cands.push({ defs: colDefs, condCols: false });
            }
            var mapDefs = null;
            row.condCols = false;
            for (var ci2 = 0; ci2 < cands.length; ci2++) {
              if (cands[ci2].defs.length === cols) {
                mapDefs = cands[ci2].defs;
                row.condCols = cands[ci2].condCols;
                break;
              }
            }
            var items = [];
            if (mapDefs) {
              for (var k = 0; k < cols; k++) items.push(makeItem(mapDefs[k], k, v, d, mi, styles, titles));
            } else {
              // 对不上就按「数值/状态/图形」的顺序兜底，并把差异写进 note
              mapDefs = dataDefs;
              row.note = '输出定义 ' + defs.length + ' 条（数据列 ' + dataDefs.length +
                ' 条）/ 实际数据列 ' + cols + ' 列，按输出顺序兜底';
              for (var q = 0; q < cols; q++) {
                items.push(makeItem(mapDefs[q] || { id: '', type: 'line' }, q, v, d, mi, styles, titles));
              }
            }
            row.plots = items;

            // 少数编译版本把条件单独放在 alertConditions 字段，而不是 plots 里
            var listedConds = [];
            try {
              var ac = mi.alertConditions || mi.alert_conditions || mi.alertconditions;
              if (ac && ac.length) {
                for (var ai = 0; ai < ac.length; ai++) {
                  var a0 = ac[ai];
                  var t0 = (typeof a0 === 'string') ? a0 : String((a0 && (a0.title || a0.name)) || '');
                  if (t0) listedConds.push(t0);
                }
              }
            } catch (e) {}

            // 任何时候都先备一份「按输出列」的兜底条件，避免脚本源码读不到时面板一片空白
            var plotConds = [];
            var scriptLike = looksLikeScript(name);
            for (var ii0 = 0; ii0 < items.length; ii0++) {
              var it0 = items[ii0];
              if (it0.kind === 'shape') {
                // 脚本指标里 plotshape 往往就是 alertcondition 的那条表达式（趋势翻转箭头），
                // 名字读不到时先按「信号出现」给你用；内置指标的图形列不加进来，避免噪声。
                if (!scriptLike) continue;
                plotConds.push({
                  id: it0.id, index: ii0, title: it0.title, source: 'plot', kind: 'signal',
                  col: it0.index, mode: 'appear', available: true,
                  note: '图上这个信号从无到有就触发（脚本里通常与 alertcondition 同一条表达式）。',
                  value: it0.value, type: it0.type
                });
                continue;
              }
              plotConds.push({
                id: it0.id, index: ii0, title: it0.title, source: 'plot', kind: it0.kind,
                col: it0.index, mode: it0.kind === 'discrete' ? 'changed' : 'numeric',
                available: true, note: '', choices: it0.choices, counts: it0.counts,
                color: it0.color, value: it0.value, type: it0.type
              });
            }

            var srcConds = pineCache()[name] || (row.scriptId ? pineCache()['id:' + row.scriptId] : null);
            if (listedConds.length) {
              // 元数据自带条件名：最权威（与原生对话框同源）
              row.condFromSource = false;
              for (var li2 = 0; li2 < listedConds.length; li2++) {
                row.conds.push(buildCond(
                  { id: 'ac_' + li2, type: 'alertcondition' },
                  listedConds[li2], li2, items, false, defs, d
                ));
              }
            } else if (condDefs.length) {
              // 元数据里就有 alertcondition：优先用它的名字与顺序（与客户端原生对话框同源）。
              // 个别编译产物会把标题留成 plot_19 这种占位名，这时按同序号用脚本源码的名字补。
              for (var ci = 0; ci < condDefs.length; ci++) {
                var cd = condDefs[ci];
                var cTitle = (styles[cd.id] && styles[cd.id].title) || '';
                if (!cTitle || /^plot_\d+$/i.test(String(cTitle))) {
                  cTitle = (srcConds && srcConds[ci] && srcConds[ci].title) || ('条件 ' + (ci + 1));
                  if (srcConds && srcConds[ci]) row.condFromSource = true;
                }
                row.conds.push(buildCond(cd, cTitle, ci, items, row.condCols, defs, d));
              }
            } else if (srcConds && srcConds.length) {
              // 元数据里没有 alertcondition（免费账号常见）：改用脚本源码里的条件名，
              // 名字、条数、顺序与原生对话框一致
              row.condFromSource = true;
              for (var si = 0; si < srcConds.length; si++) {
                row.conds.push(buildCond(
                  { id: 'src_' + si, type: 'alertcondition' },
                  srcConds[si].title, si, items, false, defs, d
                ));
              }
            } else {
              // 内置指标（RSI / Volume / MACD…）没有条件名，按输出列给条件就够了
              row.conds = plotConds;
              if (scriptLike) {
                // 脚本指标：先显示输出列 / 图形信号（别让面板空着），同时去读它自己的源码
                var failed = row.scriptId ? PINE_FAIL['id:' + row.scriptId] : PINE_FAIL[name];
                row.pending = !failed;
                row.needPine = row.pending;
                row.note = (row.note ? row.note + '；' : '') + (failed
                  ? '没读到这个脚本的源码（不是公开脚本，或当前账号无权限），下面列出的是它在图上的输出与图形信号'
                  : '条件名还在读取，当前列出的是指标输出列与图形信号');
              }
            }
          }
        }
      } catch (e) {}
      out.push(row);
    }
    return out;
  }

  function readSeries(a, studyId) {
    try {
      var api = a.getStudyById(studyId);
      var d = plotList(api && api._study);
      if (!d) return null;
      var li = d.lastIndex();
      if (li < 0) return null;
      var cur = d.valueAt(li);
      var prev = li > 0 ? d.valueAt(li - 1) : null;
      if (!cur || typeof cur[0] !== 'number') return null;
      return { time: cur[0], cur: cur, prev: prev };
    } catch (e) {
      return null;
    }
  }

  function sameInputs(a, b) {
    if (!b || !b.length) return true;
    if (!a || a.length !== b.length) return false;
    for (var i = 0; i < b.length; i++) {
      var key = b[i].id;
      var found = null;
      for (var j = 0; j < a.length; j++) if (a[j].id === key) { found = a[j]; break; }
      if (!found || String(found.value) !== String(b[i].value)) return false;
    }
    return true;
  }

  function inputsText(inputs) {
    if (!inputs || !inputs.length) return '';
    return inputs.map(function (x) { return x.id + '=' + x.value; }).join(', ');
  }

  /** 把一条报警对到当前图表上：品种 / 周期 / 指标（名字+参数）/ 曲线 */
  function resolveAlert(al, meta, ctx) {
    if (!ctx) return { ok: false, reason: '图表未就绪' };
    if (al.symbol && al.symbol !== ctx.symbol) return { ok: false, reason: '品种不匹配：当前 ' + ctx.symbol };
    if (al.resolution && String(al.resolution) !== ctx.resolution) {
      return { ok: false, reason: '周期不匹配：当前 ' + ctx.resolution };
    }
    var want = String(al.studyName || '').toLowerCase();
    var same = [];
    for (var i = 0; i < meta.length; i++) if (meta[i].name.toLowerCase() === want) same.push(meta[i]);
    if (!same.length) return { ok: false, reason: '指标不在图上' };
    var withInputs = [];
    for (var j = 0; j < same.length; j++) if (sameInputs(same[j].inputs, al.studyInputs)) withInputs.push(same[j]);
    var s = withInputs[0] || same[0];
    if (!s.ok) return { ok: false, reason: '指标暂无数据' };
    if (al.condId) {
      var cond = null;
      for (var c = 0; c < s.conds.length; c++) if (s.conds[c].id === al.condId) { cond = s.conds[c]; break; }
      if (!cond) return { ok: false, reason: '条件已不存在（脚本改版？）' };
      if (!cond.available) return { ok: false, reason: '该条件客户端读不到信号' };
      return { ok: true, study: s, cond: cond };
    }
    if (al.plotIndex >= s.plots.length) return { ok: false, reason: '曲线不存在' };
    return { ok: true, study: s, cond: null };
  }

  // ---------------------------------------------------------------- 条件求值

  function evalCond(cond, prev, cur, v, v2) {
    var cn = isNum(cur);
    var pn = isNum(prev);
    // na 交替的列（SuperTrend 的上/下轨就是这样）用「出现 / 消失」表达趋势切换
    if (cond === 'appear') return cn && !pn;
    if (cond === 'disappear') return !cn && pn;
    if (!cn) return false;
    var lo = Math.min(v, v2);
    var hi = Math.max(v, v2);
    switch (cond) {
      case 'cross_up':
        return typeof prev === 'number' && prev < v && cur >= v;
      case 'cross_down':
        return typeof prev === 'number' && prev > v && cur <= v;
      case 'gt':
        return cur > v;
      case 'lt':
        return cur < v;
      case 'in_range':
        return cur >= lo && cur <= hi && !(typeof prev === 'number' && prev >= lo && prev <= hi);
      case 'out_range':
        return (cur < lo || cur > hi) && typeof prev === 'number' && prev >= lo && prev <= hi;
      default:
        return false;
    }
  }

  /**
   * 离散输出（colorer）的求值：本根与上一根不同、且本根等于目标值。
   * 这类输出是「指标算好的状态」（涨/跌/中性），没有阈值，只有切换。
   */
  function evalChangedTo(prev, cur, target) {
    if (cur === null || cur === undefined) return false;
    if (String(cur) !== String(target)) return false;
    if (prev === null || prev === undefined || prev === '') return false;
    return String(prev) !== String(cur);
  }

  /** 布尔类信号：非空/非零即为真 */
  function signalOn(v) {
    if (v === null || v === undefined || v === '') return false;
    if (typeof v === 'boolean') return v;
    if (typeof v === 'number') return isFinite(v) && v !== 0;
    return true;
  }

  /**
   * 脚本条件（alertcondition）的求值：
   *   appear  —— 输出列从「空」变「有值」（plotshape / 分段线的信号出现）
   *   changed —— 状态列切换
   *   truthy  —— 指标真的输出了布尔信号列，取上升沿
   */
  function evalSignal(mode, prev, cur) {
    if (mode === 'appear') return isNum(cur) && !isNum(prev);
    if (mode === 'changed') {
      if (cur === null || cur === undefined || cur === '') return false;
      if (prev === null || prev === undefined || prev === '') return false;
      return String(prev) !== String(cur);
    }
    if (mode === 'truthy') return signalOn(cur) && !signalOn(prev);
    return false;
  }

  // ---------------------------------------------------------------- 配置读写

  function state(id) {
    if (!RUNTIME[id]) RUNTIME[id] = { bar: 0, firedForBar: false, status: '', value: null };
    return RUNTIME[id];
  }

  function normalize(cfg) {
    var c = cfg && typeof cfg === 'object' ? cfg : {};
    if (!Array.isArray(c.alerts)) c.alerts = [];
    if (!c.email || typeof c.email !== 'object') c.email = {};
    if (typeof c.system !== 'boolean') c.system = true;
    return c;
  }

  function loadConfig(force) {
    var now = Date.now();
    if (!force && CFG && now - LOADED_AT < CFG_RELOAD_MS) return Promise.resolve(CFG);
    return BR.get().then(function (r) {
      if (r && r.ok && r.config) {
        CFG = normalize(r.config);
        LOADED_AT = Date.now();
      } else if (!CFG) {
        CFG = normalize({});
      }
      return CFG;
    }).catch(function () {
      if (!CFG) CFG = normalize({});
      return CFG;
    });
  }

  function saveConfig() {
    if (!CFG) return Promise.resolve(false);
    LOADED_AT = Date.now();
    return BR.save(CFG).then(function (r) { return !!(r && r.ok); }).catch(function () { return false; });
  }

  // ---------------------------------------------------------------- 消息模板

  function fmtNum(v) {
    if (typeof v !== 'number' || !isFinite(v)) return '-';
    var abs = Math.abs(v);
    if (abs >= 1000) return v.toFixed(2);
    if (abs >= 1) return v.toFixed(abs >= 100 ? 2 : 3);
    return v.toFixed(4);
  }

  function rangeText(al) {
    if (al.condSource === 'alertcondition') return '';
    if (al.plotKind === 'discrete') return String(al.value);
    if (!condNeedsValue(al.condition)) return '';
    return al.condition === 'in_range' || al.condition === 'out_range'
      ? fmtNum(al.value) + ' ~ ' + fmtNum(al.value2)
      : fmtNum(al.value);
  }

  function describe(al) {
    if (al.condSource === 'alertcondition') {
      return (al.condTitle || '脚本条件') + '（脚本定义的条件）';
    }
    var head = (al.plotTitle || '指标') + ' ' + condLabel(al.condition);
    var tail = rangeText(al);
    return tail ? head + ' ' + tail : head;
  }

  /** 占位符取值表；键全部小写，未命中占位符原样保留（方便用户发现拼错） */
  function msgVars(ctx, al, plotTitle, cur) {
    var sym = String((ctx && ctx.symbol) || al.symbol || '');
    var parts = sym.split(':');
    var bar = mainBar(ctx);
    var v = {
      ticker: parts.length > 1 ? parts[parts.length - 1] : sym,
      symbol: sym,
      exchange: parts.length > 1 ? parts[0] : '',
      interval: String((ctx && ctx.resolution) || al.resolution || ''),
      study: al.studyName || '',
      plot: plotTitle || al.condTitle || al.plotTitle || '',
      condition: al.condSource === 'alertcondition' ? '出现' : condLabel(al.condition),
      threshold: rangeText(al),
      value: fmtNum(cur),
      time: bar ? new Date(bar.time * 1000).toLocaleString() : '',
      timenow: new Date().toLocaleString()
    };
    if (bar) {
      v.open = fmtNum(bar.open);
      v.high = fmtNum(bar.high);
      v.low = fmtNum(bar.low);
      v.close = fmtNum(bar.close);
      v.volume = fmtNum(bar.volume);
    }
    return v;
  }

  function renderTemplate(tpl, vars) {
    if (!tpl) return '';
    return String(tpl).replace(/\{\{\s*([a-zA-Z_][a-zA-Z0-9_]*)\s*\}\}/g, function (m, k) {
      var key = String(k).toLowerCase();
      return Object.prototype.hasOwnProperty.call(vars, key) ? String(vars[key] == null ? '' : vars[key]) : m;
    });
  }

  // ---------------------------------------------------------------- 触发

  function fire(al, ctx, cur, barTime, plotTitle) {
    var now = Date.now();
    var vars = msgVars(ctx, al, plotTitle, cur);
    var title = renderTemplate(al.msgTitle || DEFAULT_TITLE, vars) || 'TradingView 指标报警';
    var body = renderTemplate(al.msgBody || DEFAULT_BODY, vars) ||
      (al.symbol + ' · ' + al.resolution + '\n' + describe(al));

    var payload = {
      alertId: al.id,
      title: title,
      body: body,
      symbol: al.symbol,
      resolution: al.resolution,
      study: al.studyName,
      plot: vars.plot,
      condition: al.condition,
      kind: al.plotKind || 'numeric',
      value: cur,
      threshold: al.value,
      threshold2: al.value2,
      barTime: barTime,
      notify: Array.isArray(al.notify) && al.notify.length ? al.notify : ['system'],
      emailFrom: al.emailFrom || '',
      emailTo: al.emailTo || ''
    };
    al.lastFiredAt = now;
    al.lastBarTime = barTime;
    showBanner(title + '\n' + body);
    var st = state(al.id);
    st.status = '已触发 ' + new Date(now).toLocaleTimeString();
    st.value = cur;
    flash();
    BR.fire(payload).then(function (r) {
      if (r && r.errors && r.errors.length) showBanner(title + '\n通知失败：' + r.errors.join('；'), true);
    }).catch(function (e) {
      showBanner(title + '\n通知失败：' + (e && e.message ? e.message : '未知错误'), true);
    });
    BR.state({ id: al.id, lastFiredAt: al.lastFiredAt, lastBarTime: barTime }).catch(function () {});
    // 这里不整份回写配置：多个图表标签共用同一份配置，整份覆盖会吞掉别的标签页刚加的报警。
    // lastFiredAt / lastBarTime 走 state 通道由主进程合并；只有 "once" 停用才需要改整份配置。
  }

  // ---------------------------------------------------------------- 主循环

  function tick() {
    ensureToolbarButton();
    if (!CFG || !CFG.alerts.length) { pulse(); return; }
    var ctx = chartCtx();
    if (!ctx) { pulse(); return; }
    var meta = studyMeta(ctx.a);
    var dirty = false;
    for (var i = 0; i < CFG.alerts.length; i++) {
      var al = CFG.alerts[i];
      if (!al.enabled) { state(al.id).status = '已停用'; continue; }
      var r = resolveAlert(al, meta, ctx);
      var rt = state(al.id);
      if (!r.ok) { rt.status = r.reason; continue; }
      var ser = readSeries(ctx.a, r.study.id);
      if (!ser) { rt.status = '指标暂无数据'; continue; }
      var col = r.cond ? r.cond.col : al.plotIndex;
      var cur = ser.cur[col + 1];
      var prev = ser.prev ? ser.prev[col + 1] : null;
      rt.value = cur;
      if (rt.bar !== ser.time) {
        rt.bar = ser.time;
        rt.firedForBar = al.lastBarTime === ser.time;
      }
      if (rt.firedForBar) { rt.status = '就绪（本根已触发）'; continue; }
      var hit;
      if (r.cond) hit = evalSignal(r.cond.mode, prev, cur);
      else if (al.plotKind === 'discrete') hit = evalChangedTo(prev, cur, al.value);
      else hit = evalCond(al.condition, prev, cur, Number(al.value), Number(al.value2));
      if (!hit) {
        rt.status = '就绪';
        continue;
      }
      var now = Date.now();
      if (al.cooldownMin > 0 && al.lastFiredAt && now - al.lastFiredAt < al.cooldownMin * 60000) {
        rt.status = '就绪（冷却中）';
        continue;
      }
      rt.firedForBar = true;
      var plot = r.cond ? r.cond.title : (r.study.plots[al.plotIndex] ? r.study.plots[al.plotIndex].title : al.plotTitle);
      fire(al, ctx, cur, ser.time, plot);
      if (al.once) { al.enabled = false; dirty = true; }
    }
    if (dirty) saveConfig();
    pulse();
  }

  function armedCount() {
    if (!CFG) return 0;
    var n = 0;
    for (var i = 0; i < CFG.alerts.length; i++) if (CFG.alerts[i].enabled) n++;
    return n;
  }

  function pulse() {
    if (!UI) return;
    var n = armedCount();
    UI.pillLabel.textContent = n ? '报警 ' + n : '报警';
    UI.pillCount.textContent = n ? String(n) : '';
    UI.pillCount.style.display = n ? 'inline-block' : 'none';
    if (UI.toolBadge) {
      UI.toolBadge.textContent = n ? String(n) : '';
      UI.toolBadge.style.display = n ? 'block' : 'none';
    }
    // 只就地刷新状态徽标，不重建列表（否则会打断点击/滚动）
    if (UI.open && UI.tab === 'list' && UI.rows) {
      UI.rows.forEach(function (ref) {
        var al = null;
        for (var i = 0; i < CFG.alerts.length; i++) if (CFG.alerts[i].id === ref.id) { al = CFG.alerts[i]; break; }
        if (!al) return;
        var chip = statusChip(al);
        ref.chip.className = 'tvxa-chip ' + chip.cls;
        ref.chip.textContent = chip.text;
        var rt = state(al.id);
        var bits = [];
        if (rt.value != null) bits.push('当前值 ' + fmtNum(rt.value));
        if (al.lastFiredAt) bits.push('上次触发 ' + new Date(al.lastFiredAt).toLocaleString());
        if (al.notify && al.notify.indexOf('email') >= 0) bits.push('系统+邮件');
        else bits.push('仅系统');
        ref.line.textContent = bits.join(' · ');
      });
    }
  }

  function flash() {
    if (!UI) return;
    var nodes = [UI.pill, UI.toolBtn];
    nodes.forEach(function (n) {
      if (!n) return;
      n.classList.add('tvxa-hot');
      setTimeout(function () { n.classList.remove('tvxa-hot'); }, 3000);
    });
    clearTimeout(HOT_TIMER);
    HOT_TIMER = setTimeout(function () {
      if (UI && UI.pill) UI.pill.classList.remove('tvxa-hot');
    }, 3200);
  }

  // ---------------------------------------------------------------- 页内横幅

  function showBanner(text, warn) {
    if (!UI) return;
    var b = UI.banner;
    b.textContent = text;
    b.style.background = warn ? 'rgba(120, 30, 30, 0.95)' : 'rgba(24, 90, 60, 0.95)';
    b.style.opacity = '1';
    b.style.transform = 'translate(-50%, 0)';
    clearTimeout(BANNER_TIMER);
    BANNER_TIMER = setTimeout(function () {
      b.style.opacity = '0';
      b.style.transform = 'translate(-50%, -8px)';
    }, 8000);
  }

  // ---------------------------------------------------------------- 入口按钮

  var BELL_SVG = '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" ' +
    'stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M18 8a6 6 0 1 0-12 0c0 7-3 9-3 9h18s-3-2-3-9"></path>' +
    '<path d="M13.7 21a2 2 0 0 1-3.4 0"></path></svg>';

  /** 在右侧绘图工具栏里挂一个仿原生的闹钟按钮（最像「原来」的位置） */
  function ensureToolbarButton() {
    if (!UI) return;
    try {
      var bar = document.querySelector('[data-name="right-toolbar"]');
      if (!bar) return;
      var existing = document.getElementById('tvxa-toolbar-btn');
      if (existing) {
        if (existing.parentNode !== bar) bar.insertBefore(existing, bar.firstChild);
        return;
      }
      var b = document.createElement('button');
      b.id = 'tvxa-toolbar-btn';
      b.type = 'button';
      b.title = '本地指标报警（Alt+A）';
      b.setAttribute('aria-label', '本地指标报警');
      b.style.cssText = 'all:unset;box-sizing:border-box;display:flex;align-items:center;justify-content:center;' +
        'width:36px;height:36px;min-height:36px;flex:0 0 36px;margin:2px auto;border-radius:8px;cursor:pointer;' +
        'position:relative;color:#2962ff;background:rgba(41,98,255,.10)';
      b.innerHTML = BELL_SVG.replace('<svg ', '<svg style="flex:none;display:block" ') +
        '<span id="tvxa-toolbar-badge" style="position:absolute;top:1px;right:1px;min-width:14px;height:14px;' +
        'padding:0 3px;border-radius:7px;background:#f23645;color:#fff;font:700 9px/14px -apple-system,' +
        '"Segoe UI",sans-serif;text-align:center;display:none"></span>';
      b.addEventListener('mouseenter', function () { b.style.background = 'rgba(41,98,255,.22)'; });
      b.addEventListener('mouseleave', function () { b.style.background = 'rgba(41,98,255,.10)'; });
      b.addEventListener('click', function (e) {
        e.preventDefault();
        e.stopPropagation();
        toggle(true);
      });
      // 工具栏按钮在文档里、不在 shadow root 内，闪烁动画需要一条文档级样式
      if (!document.getElementById('tvxa-toolbar-style')) {
        var st = document.createElement('style');
        st.id = 'tvxa-toolbar-style';
        st.textContent = '#tvxa-toolbar-btn.tvxa-hot{animation:tvxaToolPulse .9s ease-in-out 3}' +
          '@keyframes tvxaToolPulse{0%,100%{background:rgba(41,98,255,.10);color:#2962ff}' +
          '50%{background:#f23645;color:#fff}}';
        (document.head || document.documentElement).appendChild(st);
      }
      var native = bar.querySelector('[data-name="alerts"]');
      if (native && native.parentNode) bar.insertBefore(b, native.nextSibling);
      else bar.insertBefore(b, bar.firstChild);
      UI.toolBtn = b;
      UI.toolBadge = b.querySelector('#tvxa-toolbar-badge');
      pulse();
    } catch (e) {}
  }

  // ---------------------------------------------------------------- 设置页 UI

  var FORM = {
    studyId: '',
    condIndex: 0,
    condition: 'cross_up',
    value: '',
    value2: '',
    dvalue: '',
    msgTitle: DEFAULT_TITLE,
    msgBody: DEFAULT_BODY,
    notifySystem: true,
    notifyEmail: false,
    emailFrom: '',
    emailTo: ''
  };

  var LAST_FIELD = null;

  function el(tag, cls, text) {
    var n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  }

  function btn(label, cls, fn) {
    var b = el('button', 'tvxa-btn ' + (cls || ''), label);
    b.type = 'button';
    b.addEventListener('click', function (e) {
      e.preventDefault();
      e.stopPropagation();
      fn(e);
    });
    return b;
  }

  function field(label, node, hint) {
    var wrap = el('div', 'tvxa-field');
    wrap.appendChild(el('div', 'tvxa-label', label));
    wrap.appendChild(node);
    if (hint) wrap.appendChild(el('div', 'tvxa-hint', hint));
    return wrap;
  }

  function selectEl(options, value) {
    var s = document.createElement('select');
    s.className = 'tvxa-input';
    for (var i = 0; i < options.length; i++) {
      var o = document.createElement('option');
      o.value = String(options[i].value);
      o.textContent = options[i].label;
      if (String(options[i].value) === String(value)) o.selected = true;
      s.appendChild(o);
    }
    return s;
  }

  function inputEl(value, type, step) {
    var i = document.createElement('input');
    i.className = 'tvxa-input';
    i.type = type || 'text';
    if (step) i.step = step;
    i.value = value == null ? '' : String(value);
    return i;
  }

  function section(title, nodes, hint) {
    var s = el('div', 'tvxa-section');
    s.appendChild(el('div', 'tvxa-sec-title', title));
    for (var i = 0; i < nodes.length; i++) s.appendChild(nodes[i]);
    if (hint) s.appendChild(el('div', 'tvxa-hint', hint));
    return s;
  }

  function buildUI() {
    var host = document.createElement('div');
    host.id = 'tvx-alerts-host';
    host.style.cssText = 'position:fixed;inset:0;z-index:2147483000;pointer-events:none;';
    var root = host.attachShadow ? host.attachShadow({ mode: 'open' }) : host;

    var style = document.createElement('style');
    style.textContent = [
      ':host{all:initial}',
      // 悬浮入口：品牌色实心胶囊，带铃铛和计数，比原来的灰字显眼得多
      '.tvxa-pill{pointer-events:auto;position:fixed;right:64px;bottom:28px;display:flex;align-items:center;gap:7px;',
      'padding:8px 13px 8px 11px;border-radius:20px;border:1px solid rgba(41,98,255,.75);background:#2962ff;',
      'color:#fff;font:600 12px/1 -apple-system,"Segoe UI",Roboto,"Microsoft YaHei",sans-serif;cursor:pointer;',
      'box-shadow:0 6px 18px rgba(41,98,255,.42);transition:transform .15s,box-shadow .15s,background .15s;z-index:2}',
      '.tvxa-pill:hover{background:#1e53e5;transform:translateY(-1px);box-shadow:0 8px 22px rgba(41,98,255,.55)}',
      '.tvxa-pill .tvxa-ico{display:flex;align-items:center}',
      '.tvxa-pill-count{background:rgba(255,255,255,.24);border-radius:9px;padding:1px 5px;font-size:10px}',
      '.tvxa-pill.toolbar-hidden{display:none}',
      '.tvxa-hot{animation:tvxaPulse 1s ease-in-out 3}',
      '@keyframes tvxaPulse{0%,100%{box-shadow:0 6px 18px rgba(41,98,255,.42)}',
      '50%{box-shadow:0 0 0 6px rgba(242,54,69,.55)}}',
      '.tvxa-mask{pointer-events:auto;position:fixed;inset:0;background:rgba(0,0,0,.45);display:flex;',
      'align-items:center;justify-content:center}',
      '.tvxa-card{width:680px;max-width:calc(100vw - 40px);max-height:calc(100vh - 24px);display:flex;flex-direction:column;',
      'background:#1e222d;color:#d1d4dc;border:1px solid #363a45;border-radius:10px;overflow:hidden;',
      'box-shadow:0 12px 48px rgba(0,0,0,.5);font:13px/1.45 -apple-system,"Segoe UI",Roboto,"Microsoft YaHei",sans-serif}',
      '.tvxa-head{display:flex;align-items:center;gap:10px;padding:12px 14px;border-bottom:1px solid #2a2e39}',
      '.tvxa-title{font-size:14px;font-weight:600;flex:1}',
      '.tvxa-chart{font-size:12px;color:#787b86}',
      '.tvxa-tabs{display:flex;gap:4px;padding:8px 14px 0}',
      '.tvxa-tab{padding:5px 12px;border-radius:6px 6px 0 0;background:transparent;border:1px solid transparent;',
      'color:#a3a6af;cursor:pointer;font:inherit}',
      '.tvxa-tab.on{background:#2a2e39;border-color:#363a45;border-bottom-color:#2a2e39;color:#e6e8ec}',
      '.tvxa-body{padding:12px 14px 16px;overflow:auto;flex:1}',
      '.tvxa-section{border:1px solid #2a2e39;border-radius:8px;padding:8px 10px;margin-bottom:8px;background:#1b1f29}',
      '.tvxa-sec-title{font-size:12px;font-weight:600;color:#e6e8ec;margin-bottom:6px;letter-spacing:.3px}',
      '.tvxa-row{border:1px solid #2a2e39;border-radius:8px;padding:10px;margin-bottom:8px;background:#22262f}',
      '.tvxa-row-head{display:flex;align-items:center;gap:8px}',
      '.tvxa-row-title{flex:1;font-weight:600;color:#e6e8ec;font-size:13px}',
      '.tvxa-row-sub{color:#9aa0aa;font-size:12px;margin-top:4px;white-space:pre-wrap}',
      '.tvxa-chip{font-size:11px;padding:2px 7px;border-radius:10px;border:1px solid #3d4250;color:#a3a6af}',
      '.tvxa-chip.ok{color:#4caf50;border-color:#2e5c33}',
      '.tvxa-chip.warn{color:#e0a03a;border-color:#5c4a2a}',
      '.tvxa-chip.off{color:#787b86}',
      '.tvxa-chip.fire{color:#3d8bff;border-color:#2a4a7a}',
      '.tvxa-actions{display:flex;gap:6px;margin-top:8px;flex-wrap:wrap}',
      '.tvxa-btn{padding:4px 10px;border-radius:6px;border:1px solid #3d4250;background:#2a2e39;color:#d1d4dc;',
      'cursor:pointer;font:12px/1.4 inherit}',
      '.tvxa-btn:hover{background:#333846}',
      '.tvxa-btn.primary{background:#2962ff;border-color:#2962ff;color:#fff}',
      '.tvxa-btn.danger{color:#ff7b72;border-color:#5a3230}',
      '.tvxa-btn.ph{padding:2px 7px;font-size:11px;color:#8ab0ff;border-color:#2f3f63;background:#1a2130}',
      '.tvxa-field{margin-bottom:8px}',
      '.tvxa-label{font-size:12px;color:#9aa0aa;margin-bottom:4px}',
      '.tvxa-hint{font-size:11px;color:#787b86;margin-top:3px;line-height:1.5}',
      '.tvxa-input{width:100%;box-sizing:border-box;padding:5px 8px;border-radius:6px;border:1px solid #3d4250;',
      'background:#131722;color:#d1d4dc;font:12px/1.4 inherit;outline:none}',
      '.tvxa-input:focus{border-color:#2962ff}',
      'textarea.tvxa-input{resize:vertical;min-height:52px;font-family:inherit}',
      '.tvxa-two{display:grid;grid-template-columns:1fr 1fr;gap:10px}',
      '.tvxa-three{display:grid;grid-template-columns:1fr 1fr 1fr;gap:10px}',
      '.tvxa-check{display:flex;align-items:center;gap:6px;margin:6px 0;font-size:12px;color:#c9ccd3}',
      '.tvxa-check input{accent-color:#2962ff}',
      '.tvxa-info{display:flex;gap:14px;flex-wrap:wrap;font-size:12px;color:#a3a6af}',
      '.tvxa-info b{color:#d1d4dc;font-weight:600}',
      '.tvxa-empty{color:#787b86;font-size:12px;padding:18px 4px;text-align:center}',
      '.tvxa-banner{pointer-events:none;position:fixed;left:50%;top:16px;transform:translate(-50%,-8px);',
      'max-width:600px;padding:10px 14px;border-radius:8px;color:#f0f3fa;white-space:pre-wrap;',
      'font:12px/1.5 -apple-system,"Segoe UI",Roboto,"Microsoft YaHei",sans-serif;opacity:0;',
      'transition:opacity .2s,transform .2s;box-shadow:0 6px 24px rgba(0,0,0,.4)}'
    ].join('');

    var pill = el('button', 'tvxa-pill', '');
    pill.type = 'button';
    pill.title = '本地指标报警（Alt+A）';
    var pillIco = el('span', 'tvxa-ico');
    pillIco.innerHTML = BELL_SVG;
    var pillLabel = el('span', 'tvxa-pill-label', '报警');
    var pillCount = el('span', 'tvxa-pill-count', '');
    pill.appendChild(pillIco);
    pill.appendChild(pillLabel);
    pill.appendChild(pillCount);

    var banner = el('div', 'tvxa-banner');

    var mask = el('div', 'tvxa-mask');
    mask.style.display = 'none';
    var card = el('div', 'tvxa-card');
    var head = el('div', 'tvxa-head');
    var title = el('div', 'tvxa-title', 'TVHelper 指标报警');
    var chartInfo = el('div', 'tvxa-chart', '');
    var close = btn('关闭', '', function () { toggle(false); });
    head.appendChild(title);
    head.appendChild(chartInfo);
    head.appendChild(close);

    var tabs = el('div', 'tvxa-tabs');
    var tabList = el('button', 'tvxa-tab on', '报警列表');
    var tabNew = el('button', 'tvxa-tab', '新建报警');
    var tabNotify = el('button', 'tvxa-tab', '通知设置');
    tabList.type = tabNew.type = tabNotify.type = 'button';
    tabs.appendChild(tabList);
    tabs.appendChild(tabNew);
    tabs.appendChild(tabNotify);

    var body = el('div', 'tvxa-body');
    card.appendChild(head);
    card.appendChild(tabs);
    card.appendChild(body);
    mask.appendChild(card);
    root.appendChild(style);
    root.appendChild(pill);
    root.appendChild(banner);
    root.appendChild(mask);
    document.documentElement.appendChild(host);

    UI = {
      host: host, root: root, pill: pill, pillLabel: pillLabel, pillCount: pillCount,
      banner: banner, mask: mask, body: body, toolBtn: null, toolBadge: null,
      chartInfo: chartInfo, tab: 'list', open: false, tabEls: { list: tabList, new: tabNew, notify: tabNotify }
    };

    switchTab = function (name) {
      UI.tab = name;
      Object.keys(UI.tabEls).forEach(function (k) {
        UI.tabEls[k].className = 'tvxa-tab' + (k === name ? ' on' : '');
      });
      if (name === 'list') renderList();
      else if (name === 'new') renderForm();
      else renderNotify();
    };

    pill.addEventListener('click', function () { toggle(!UI.open); });
    tabList.addEventListener('click', function () { switchTab('list'); });
    tabNew.addEventListener('click', function () { switchTab('new'); });
    tabNotify.addEventListener('click', function () { switchTab('notify'); });
    mask.addEventListener('click', function (e) { if (e.target === mask) toggle(false); });

    document.addEventListener('keydown', function (e) {
      if (e.altKey && !e.ctrlKey && !e.metaKey && (e.key === 'a' || e.key === 'A')) {
        e.preventDefault();
        toggle(!UI.open);
      } else if (e.key === 'Escape' && UI.open) {
        toggle(false);
      }
    }, true);
  }

  var switchTab = function () {};

  function toggle(open) {
    if (!UI) return;
    UI.open = open;
    UI.mask.style.display = open ? 'flex' : 'none';
    if (open) {
      loadConfig(true).then(function () {
        var ctx = chartCtx();
        UI.chartInfo.textContent = ctx ? ctx.symbol + ' · ' + ctx.resolution : '图表未就绪';
        if (UI.tab === 'list') renderList();
        else if (UI.tab === 'new') renderForm();
        else renderNotify();
        // 打开面板就落一次自检，用户真机上的指标元数据/条件名能直接拿来核对
        setTimeout(dumpStudies, 600);
      });
    }
  }

  function statusChip(al) {
    var rt = state(al.id);
    if (!al.enabled) return { cls: 'off', text: '已停用' };
    if (/已触发/.test(rt.status)) return { cls: 'fire', text: rt.status };
    if (/就绪/.test(rt.status)) return { cls: 'ok', text: rt.status };
    if (!rt.status) return { cls: 'warn', text: '等待检测' };
    return { cls: 'warn', text: rt.status };
  }

  function renderList() {
    if (!UI) return;
    var ctx = chartCtx();
    UI.chartInfo.textContent = ctx ? ctx.symbol + ' · ' + ctx.resolution : '图表未就绪';
    var body = UI.body;
    body.textContent = '';
    UI.rows = [];
    var alerts = CFG ? CFG.alerts : [];
    if (!alerts.length) {
      body.appendChild(el('div', 'tvxa-empty', '还没有报警。切到「新建报警」，用当前图表的指标建一条。'));
      return;
    }
    alerts.forEach(function (al) {
      var row = el('div', 'tvxa-row');
      var head = el('div', 'tvxa-row-head');
      var title = el('div', 'tvxa-row-title', (al.studyName || '') + ' · ' + (al.plotTitle || ''));
      var chip = statusChip(al);
      var chipEl = el('span', 'tvxa-chip ' + chip.cls, chip.text);
      head.appendChild(title);
      head.appendChild(chipEl);
      row.appendChild(head);
      row.appendChild(el('div', 'tvxa-row-sub',
        al.symbol + ' · ' + al.resolution + ' · ' + describe(al) +
        (al.studyInputs && al.studyInputs.length ? ' · ' + inputsText(al.studyInputs) : '')));
      if (al.msgTitle) {
        row.appendChild(el('div', 'tvxa-row-sub', '消息标题：' + al.msgTitle));
      }
      var rt = state(al.id);
      var extra = [];
      if (rt.value != null) extra.push('当前值 ' + fmtNum(rt.value));
      if (al.lastFiredAt) extra.push('上次触发 ' + new Date(al.lastFiredAt).toLocaleString());
      extra.push('通知：' + ((al.notify || []).indexOf('email') >= 0 ? '系统+邮件' : '系统'));
      var line = el('div', 'tvxa-row-sub', extra.join(' · '));
      row.appendChild(line);
      UI.rows.push({ id: al.id, chip: chipEl, line: line });

      var acts = el('div', 'tvxa-actions');
      acts.appendChild(btn(al.enabled ? '停用' : '启用', '', function () {
        al.enabled = !al.enabled;
        saveConfig();
        renderList();
      }));
      acts.appendChild(btn('重新绑定当前图表', '', function () {
        var c = chartCtx();
        if (!c) return;
        al.symbol = c.symbol;
        al.resolution = c.resolution;
        var meta = studyMeta(c.a);
        for (var i = 0; i < meta.length; i++) {
          if (meta[i].name.toLowerCase() === String(al.studyName || '').toLowerCase()) {
            al.studyInputs = meta[i].inputs;
            break;
          }
        }
        al.lastBarTime = 0;
        saveConfig();
        renderList();
      }));
      acts.appendChild(btn('预览消息', '', function () {
        var c = chartCtx();
        var vars = msgVars(c, al, al.plotTitle, rt.value);
        var t = renderTemplate(al.msgTitle || DEFAULT_TITLE, vars);
        var b = renderTemplate(al.msgBody || DEFAULT_BODY, vars);
        showBanner(t + '\n' + b);
      }));
      acts.appendChild(btn('删除', 'danger', function () {
        CFG.alerts = CFG.alerts.filter(function (x) { return x.id !== al.id; });
        saveConfig();
        renderList();
      }));
      row.appendChild(acts);
      body.appendChild(row);
    });
  }

  function newAlertId() {
    return 'a' + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
  }

  function gotoList() {
    UI.tab = 'list';
    Object.keys(UI.tabEls).forEach(function (k) {
      UI.tabEls[k].className = 'tvxa-tab' + (k === 'list' ? ' on' : '');
    });
    renderList();
  }

  function renderForm() {
    if (!UI) return;
    var body = UI.body;
    body.textContent = '';
    var ctx = chartCtx();
    if (!ctx) {
      body.appendChild(el('div', 'tvxa-empty', '当前没有可用的图表。打开一个图表标签再试。'));
      return;
    }
    var meta = studyMeta(ctx.a).filter(function (s) { return s.ok; });
    if (!meta.length) {
      body.appendChild(el('div', 'tvxa-empty', '当前图表上还没有加载完成的指标。先在图上添加指标，再回来建报警。'));
      return;
    }
    if (!FORM.studyId || !meta.some(function (s) { return s.id === FORM.studyId; })) {
      FORM.studyId = meta[0].id;
      FORM.condIndex = 0;
    }
    // 脚本指标若元数据里没有 alertcondition，按指标名去公开脚本库读源码补条件名，
    // 读完自渲染一次；失败则退回按输出列给条件，不会反复请求。
    // 读到条件名后重渲染一次；先按脚本 id 读（不会认错脚本），id 拿不到才退回按名字搜
    var needList = meta.filter(function (s) { return s.pending; });
    for (var ni = 0; ni < needList.length; ni++) {
      (function (s) {
        var done = function (list) {
          // 成功或失败都重画一次：失败后标签要变成「输出列」并把原因写出来
          if (UI && FORM.studyId === s.id) renderForm();
        };
        if (s.scriptId) {
          fetchSourceConds(s.scriptId, function (list) {
            if (list) { done(list); return; }
            fetchPineConds(s.name, done);
          });
        } else {
          fetchPineConds(s.name, done);
        }
      })(needList[ni]);
    }

    var current = meta.filter(function (s) { return s.id === FORM.studyId; })[0] || meta[0];
    var conds = current.conds || [];
    if (!conds.length) {
      body.appendChild(el('div', 'tvxa-empty',
        '「' + current.name + '」没有可用的条件输出。换一个指标，或给它加一条 alertcondition。'));
      return;
    }
    if (FORM.condIndex >= conds.length) FORM.condIndex = 0;
    var curCond = conds[FORM.condIndex] || conds[0];

    // ---- 条件：指标 → 该指标自己声明的条件
    // 脚本指标（Pine）的 alertcondition 名字在 metainfo.styles[plotId].title 里，
    // 与原生「创建警报」对话框的条件列表完全一致（如 "Bullish Trend Shift"）。
    var studySel = selectEl(meta.map(function (s) {
      // 名字只显示指标名（和原生一致），参数放到下面的小字里，避免长参数把名字挤没
      return { value: s.id, label: s.name };
    }), FORM.studyId);
    studySel.addEventListener('change', function () {
      FORM.studyId = studySel.value;
      FORM.condIndex = 0;
      FORM.value = '';
      FORM.dvalue = '';
      renderForm();
    });

    var condSel = selectEl(conds.map(function (c, i) {
      var tag;
      if (c.source === 'alertcondition') tag = c.available ? '｜脚本条件' : '｜客户端不可读';
      else if (current.pending) tag = '｜输出列·过渡';
      else if (c.kind === 'discrete') tag = '｜状态列';
      else tag = '｜数值列';
      return { value: i, label: c.title + tag };
    }), FORM.condIndex);
    condSel.addEventListener('change', function () {
      FORM.condIndex = Number(condSel.value);
      FORM.value = '';
      FORM.dvalue = '';
      renderForm(); // 条件类型变了，下面控件必须跟着换
    });

    var condGrid = el('div', 'tvxa-two');
    condGrid.appendChild(field('指标', studySel, inputsText(current.inputs) ? '参数：' + inputsText(current.inputs) : ''));
    condGrid.appendChild(field('条件', condSel));

    var opGrid = el('div', 'tvxa-three');
    var numCondSel = null;
    var discreteSel = null;
    var valueInput = null;
    var value2Input = null;
    var v2Wrap = null;
    var isRange = false;
    var condHint = '';
    var blocked = false;

    if (curCond.source === 'alertcondition') {
      opGrid.style.display = 'none';
      if (!curCond.available) {
        blocked = true;
        condHint = curCond.note || '这个条件由脚本在服务端定义，客户端读不到信号，暂时做不了本地报警。';
      } else {
        condHint = curCond.note ||
          ('脚本自己定义的报警条件：信号出现即触发，与原生「' + curCond.title + '」报警一致。');
        if (current.condFromSource) condHint += ' 条件名取自脚本源码，与原生对话框一致。';
      }
    } else if (curCond.mode === 'appear' || curCond.mode === 'disappear') {
      opGrid.style.display = 'none';
      FORM.condition = curCond.mode;
      FORM.value = 0;
      FORM.value2 = 0;
      condHint = curCond.note ||
        (curCond.mode === 'appear' ? '这个输出从无到有就触发，不需要阈值。' : '这个输出从有值变成空就触发。');
    } else if (curCond.kind === 'discrete') {
      var choices = (curCond.choices || []).slice();
      var curStr = String(curCond.value);
      var has = function (c) { return String(c) === curStr; };
      if (!choices.length) choices.push(curCond.value);
      if (!choices.some(has)) choices.push(curCond.value);
      var want = FORM.dvalue ? String(FORM.dvalue) : curStr;
      if (!choices.some(function (c) { return String(c) === want; })) choices.push(want);
      FORM.dvalue = want;
      discreteSel = selectEl(choices.map(function (c) {
        var hex = typeof curCond.color === 'function' ? curCond.color(c) : null;
        var cnt = curCond.counts ? curCond.counts[String(c)] : 0;
        return {
          value: String(c),
          label: '值 ' + c + (hex ? '（' + hex + '）' : '') + (cnt ? ' · 近 400 根出现 ' + cnt + ' 次' : '')
        };
      }), FORM.dvalue);
      discreteSel.addEventListener('change', function () { FORM.dvalue = discreteSel.value; });
      opGrid.appendChild(field('触发条件', discreteSel));
      condHint = '这是指标算好的离散状态列（如涨跌着色）：只在「从别的值变成这个值」时触发，没有阈值。';
    } else {
      numCondSel = selectEl(CONDS.map(function (c) { return { value: c.id, label: c.label }; }), FORM.condition);
      valueInput = inputEl(FORM.value, 'number', 'any');
      if (FORM.value === undefined || FORM.value === '' || !isFinite(Number(FORM.value))) {
        valueInput.value = fmtNum(curCond.value);
      }
      value2Input = inputEl(FORM.value2, 'number', 'any');
      opGrid.appendChild(field('比较方式', numCondSel));
      var v1Wrap = field('阈值', valueInput);
      opGrid.appendChild(v1Wrap);
      v2Wrap = field('上限（区间条件用）', value2Input);
      opGrid.appendChild(v2Wrap);
      var syncOps = function () {
        var need = condNeedsValue(FORM.condition);
        isRange = need && (FORM.condition === 'in_range' || FORM.condition === 'out_range');
        v1Wrap.style.display = need ? '' : 'none';
        v2Wrap.style.display = isRange ? '' : 'none';
      };
      FORM.condition = numCondSel.value;
      syncOps();
      numCondSel.addEventListener('change', function () {
        FORM.condition = numCondSel.value;
        syncOps();
      });
      condHint = '这个指标没有声明条件名，只能按数值列比较；阈值默认填当前值，改成你要的水平。';
    }

    if (curCond.source === 'plot' && current.note && !current.pending) condHint = current.note + ' ' + condHint;
    if (curCond.source === 'plot' && current.note && !current.pending) condHint = current.note + ' ' + condHint;
    if (current.pending) {
      condHint += ' 正在按脚本 id 读取这个脚本自己声明的条件，读到后列表会自动换成脚本里的条件名。';
    }
    if (current.serverConds > 0 && curCond.source !== 'alertcondition') {
      condHint += ' 该脚本还声明了 ' + current.serverConds + ' 个服务端条件，名字还没读到。';
    }
    var condSection = section('条件', [condGrid, opGrid], condHint);

    // ---- 周期和触发：不做输入，只标注与图表一致
    var info = el('div', 'tvxa-info');
    var infoPeriod = el('div');
    infoPeriod.innerHTML = '周期：<b>' + (ctx.resolution || '-') + '</b>（与图表一致）';
    var infoTrigger = el('div');
    infoTrigger.innerHTML = '触发：<b>每根 K 线最多一次</b>';
    info.appendChild(infoPeriod);
    info.appendChild(infoTrigger);
    var periodSection = section('周期和触发', [info],
      '不单独设置，始终跟随当前图表的品种与周期。');

    // ---- 消息：标题 + 内容 + 占位符
    var titleInput = inputEl(FORM.msgTitle || '', 'text');
    var bodyInput = document.createElement('textarea');
    bodyInput.className = 'tvxa-input';
    bodyInput.value = FORM.msgBody || '';
    bodyInput.spellcheck = false;
    LAST_FIELD = bodyInput;
    [titleInput, bodyInput].forEach(function (n) {
      n.addEventListener('focus', function () { LAST_FIELD = n; });
    });

    var chips = el('div', 'tvxa-actions');
    PLACEHOLDERS.forEach(function (name) {
      chips.appendChild(btn('{{' + name + '}}', 'ph', function () {
        var target = LAST_FIELD || bodyInput;
        var token = '{{' + name + '}}';
        try {
          var s = target.selectionStart == null ? target.value.length : target.selectionStart;
          var e = target.selectionEnd == null ? s : target.selectionEnd;
          target.value = target.value.slice(0, s) + token + target.value.slice(e);
          target.selectionStart = target.selectionEnd = s + token.length;
          target.focus();
        } catch (err) {
          target.value += token;
        }
        if (target === titleInput) FORM.msgTitle = target.value;
        else FORM.msgBody = target.value;
      }));
    });

    var msgSection = section('消息', [
      field('标题', titleInput),
      field('内容', bodyInput, '点击标签插入占位符：'),
      chips
    ], '触发时替换占位符后，作为通知标题/正文与邮件主题/正文发送。');

    // ---- 通知：系统 / 邮件（可覆盖发件人与收件人）
    var sysChk = document.createElement('input');
    sysChk.type = 'checkbox';
    sysChk.checked = FORM.notifySystem !== false;
    var mailChk = document.createElement('input');
    mailChk.type = 'checkbox';
    mailChk.checked = !!FORM.notifyEmail;

    var chkSys = el('label', 'tvxa-check');
    chkSys.appendChild(sysChk);
    chkSys.appendChild(el('span', null, '系统通知（桌面弹窗）'));
    var chkMail = el('label', 'tvxa-check');
    chkMail.appendChild(mailChk);
    chkMail.appendChild(el('span', null, '邮件通知'));

    var fromInput = inputEl(FORM.emailFrom || '', 'text');
    fromInput.placeholder = '留空用通知设置里的发件人';
    var toInput = inputEl(FORM.emailTo || '', 'text');
    toInput.placeholder = '留空用通知设置里的收件人；多个用逗号分隔';
    var mailGrid = el('div', 'tvxa-two');
    mailGrid.appendChild(field('发件人', fromInput));
    mailGrid.appendChild(field('收件人', toInput));
    mailGrid.style.display = mailChk.checked ? '' : 'none';
    mailChk.addEventListener('change', function () {
      mailGrid.style.display = mailChk.checked ? '' : 'none';
    });

    var notifySection = section('通知', [chkSys, chkMail, mailGrid],
      'SMTP 在「通知设置」里配一次；这里的收发件人只对这一条生效。');

    var actions = el('div', 'tvxa-actions');
    actions.appendChild(btn('创建报警', 'primary', function () {
      FORM.msgTitle = titleInput.value;
      FORM.msgBody = bodyInput.value;
      FORM.notifySystem = !!sysChk.checked;
      FORM.notifyEmail = !!mailChk.checked;
      FORM.emailFrom = fromInput.value.trim();
      FORM.emailTo = toInput.value.trim();
      if (curCond.source === 'alertcondition') {
        if (!curCond.available) {
          showBanner('这个条件客户端读不到信号，换一个条件', true);
          return;
        }
        FORM.condition = 'appear';
        FORM.value = 0;
        FORM.value2 = 0;
      } else if (curCond.mode === 'appear' || curCond.mode === 'disappear') {
        FORM.condition = curCond.mode;
        FORM.value = 0;
        FORM.value2 = 0;
      } else if (curCond.kind === 'discrete') {
        FORM.condition = COND_CHANGED;
        FORM.value = String(discreteSel.value);
        FORM.value2 = '';
      } else {
        FORM.value = Number(valueInput.value);
        FORM.value2 = Number(value2Input.value);
        if (!condNeedsValue(FORM.condition)) {
          FORM.value = 0;
          FORM.value2 = 0;
        } else if (FORM.condition === 'in_range' || FORM.condition === 'out_range') {
          if (!isFinite(FORM.value) || !isFinite(FORM.value2)) {
            showBanner('区间条件需要填写两个数值', true);
            return;
          }
        } else if (!isFinite(FORM.value)) {
          showBanner('阈值必须是数字', true);
          return;
        }
      }
      var notify = [];
      if (FORM.notifySystem) notify.push('system');
      if (FORM.notifyEmail) notify.push('email');
      if (!notify.length) notify.push('system');
      var al = {
        id: newAlertId(),
        symbol: ctx.symbol,
        resolution: ctx.resolution,
        studyName: current.name,
        studyInputs: current.inputs,
        condId: curCond.source === 'alertcondition' ? curCond.id : '',
        condTitle: curCond.title,
        condSource: curCond.source,
        condMode: curCond.mode || '',
        plotIndex: curCond.col,
        plotTitle: curCond.title,
        plotKind: curCond.source === 'alertcondition' ? 'signal' : curCond.kind,
        plotType: curCond.type || '',
        condition: FORM.condition,
        value: FORM.value,
        value2: FORM.value2,
        cooldownMin: 0,
        once: false,
        msgTitle: FORM.msgTitle,
        msgBody: FORM.msgBody,
        notify: notify,
        emailFrom: FORM.notifyEmail ? FORM.emailFrom : '',
        emailTo: FORM.notifyEmail ? FORM.emailTo : '',
        enabled: true,
        createdAt: Date.now(),
        lastFiredAt: 0,
        lastBarTime: 0
      };
      CFG.alerts.push(al);
      saveConfig().then(function (ok) {
        showBanner(ok ? '已创建：' + describe(al) : '保存失败，检查是否允许写入配置', !ok);
        gotoList();
      });
    }));
    actions.appendChild(btn('取消', '', function () { gotoList(); }));

    body.appendChild(condSection);
    body.appendChild(periodSection);
    body.appendChild(msgSection);
    body.appendChild(notifySection);
    body.appendChild(actions);
  }

  function renderNotify() {
    if (!UI) return;
    var body = UI.body;
    body.textContent = '';
    var email = (CFG && CFG.email) || {};

    var hostI = inputEl(email.host || '', 'text');
    var portI = inputEl(email.port || 465, 'number', '1');
    var userI = inputEl(email.user || '', 'text');
    var passI = inputEl(email.pass || '', 'password');
    var fromI = inputEl(email.from || '', 'text');
    var toI = inputEl((email.to || []).join(', '), 'text');
    var secureChk = document.createElement('input');
    secureChk.type = 'checkbox';
    secureChk.checked = email.secure !== false;
    var sysChk = document.createElement('input');
    sysChk.type = 'checkbox';
    sysChk.checked = CFG ? CFG.system !== false : true;

    var grid = el('div', 'tvxa-two');
    grid.appendChild(field('SMTP 服务器', hostI, '例如 smtp.163.com / smtp.qq.com'));
    grid.appendChild(field('端口', portI, '465 走 SSL；587 走 STARTTLS'));
    var grid2 = el('div', 'tvxa-two');
    grid2.appendChild(field('账号', userI));
    grid2.appendChild(field('授权码/密码', passI, '保存在本机 profile 目录，明文，注意安全'));
    var grid3 = el('div', 'tvxa-two');
    grid3.appendChild(field('发件人', fromI, '默认发件地址'));
    grid3.appendChild(field('收件人', toI, '多个用逗号分隔'));

    var mailSection = section('邮件（SMTP）', [grid, grid2, grid3], '这里是全局默认值；单条报警可以在「新建报警 → 通知」里覆盖发件人/收件人。');

    var secureWrap = el('label', 'tvxa-check');
    secureWrap.appendChild(secureChk);
    secureWrap.appendChild(el('span', null, 'SSL 直连（465 端口勾选；587 端口不勾）'));
    var sysWrap = el('label', 'tvxa-check');
    sysWrap.appendChild(sysChk);
    sysWrap.appendChild(el('span', null, '允许系统通知（弹窗）'));
    var sysSection = section('系统通知', [sysWrap, secureWrap], '');

    function readMail() {
      return {
        host: hostI.value.trim(),
        port: Number(portI.value) || 465,
        secure: !!secureChk.checked,
        user: userI.value.trim(),
        pass: passI.value,
        from: fromI.value.trim(),
        to: toI.value.split(',').map(function (s) { return s.trim(); }).filter(Boolean)
      };
    }

    var actions = el('div', 'tvxa-actions');
    actions.appendChild(btn('保存通知设置', 'primary', function () {
      CFG.email = readMail();
      CFG.system = !!sysChk.checked;
      saveConfig().then(function (ok) {
        showBanner(ok ? '通知设置已保存' : '保存失败', !ok);
      });
    }));
    actions.appendChild(btn('测试系统通知', '', function () {
      BR.test('system').then(function (r) {
        showBanner(r && r.ok ? '已发送系统通知' : '系统通知失败：' + ((r && r.error) || '未知'), !(r && r.ok));
      });
    }));
    actions.appendChild(btn('测试邮件', '', function () {
      CFG.email = readMail();
      saveConfig().then(function () {
        showBanner('正在发送测试邮件…');
        BR.test('email').then(function (r) {
          showBanner(r && r.ok ? '测试邮件已发出' : '邮件发送失败：' + ((r && r.error) || '未知'), !(r && r.ok));
        });
      });
    }));

    body.appendChild(mailSection);
    body.appendChild(sysSection);
    body.appendChild(el('div', 'tvxa-hint',
      '系统通知走操作系统通知中心（Windows 右下角 / macOS 通知中心 / Linux libnotify）。邮件只在勾了「邮件通知」的报警触发时发送。'));
    body.appendChild(actions);
  }

  // ---------------------------------------------------------------- 启动

  // ---------------------------------------------------------------- 自检转储

  /**
   * 把每个指标读到的「条件名 / 列映射 / 是否可读」写到 userData/tvx-alerts-study-dump.json。
   * 用来核对脚本声明的 alertcondition（如 "Bullish Trend Shift"）是否被正确取到。
   */
  function dumpStudies() {
    var c = chartCtx();
    if (!c) return false;
    var meta = studyMeta(c.a);
    var data = [];
    for (var i = 0; i < meta.length; i++) {
      var s = meta[i];
      var plots = [];
      for (var p = 0; p < s.plots.length; p++) {
        var it = s.plots[p];
        plots.push(it.id + ':' + it.type + ':' + it.title);
      }
      var items = [];
      for (var q = 0; q < s.conds.length; q++) {
        var x = s.conds[q];
        items.push({
          title: x.title, source: x.source, mode: x.mode, col: x.col,
          available: !!x.available, note: x.note || '', linked: x.linkedTitle || ''
        });
      }
      data.push({
        name: s.name, ok: s.ok, note: s.note, serverConds: s.serverConds,
        condCols: s.condCols, cols: s.plots.length, plots: plots, conds: items,
        scriptId: s.scriptId || '', fullId: s.fullId || '', miKeys: s.miKeys || '',
        pending: !!s.pending, fromSource: !!s.condFromSource,
        plotTypes: s.plots.map(function (x) { return x.id + ':' + x.type; })
      });
    }
    var payload = { symbol: c.symbol, resolution: c.resolution, at: Date.now(), studies: data };
    try {
      if (BR.dump) BR.dump(payload);
    } catch (e) {}
    return payload;
  }

  function start() {
    buildUI();
    loadConfig(true).then(function () {
      // 重启恢复：把上次记录的最后触发 K 线带上，避免重启后对同一根重复报
      if (CFG) {
        CFG.alerts.forEach(function (al) {
          var rt = state(al.id);
          rt.bar = al.lastBarTime || 0;
          rt.firedForBar = !!al.lastBarTime;
        });
      }
      pulse();
      ensureToolbarButton();
      setInterval(tick, TICK_MS);
      setInterval(ensureToolbarButton, 2000);
      setTimeout(tick, 800);
      setTimeout(dumpStudies, 7000);
      window.addEventListener('focus', function () { loadConfig(false).then(pulse); });
    });
    // 调试/自检入口
      var studies = function () { var c = chartCtx(); return c ? studyMeta(c.a) : []; };
      window.__tvxAlertsDebug = {
      tick: tick,
      ctx: chartCtx,
      bar: function () { return mainBar(chartCtx()); },
      studies: studies,
      dump: dumpStudies,
      pine: function (name) { return pineCache()[name] || null; },
      fetchPine: function (name) { return new Promise(function (r) { fetchPineConds(name, r); }); },
      conds: function () {
        var m = studies();
        var out = [];
        for (var i = 0; i < m.length; i++) {
          for (var j = 0; j < m[i].conds.length; j++) {
            out.push(m[i].name + ' · ' + m[i].conds[j].title + ' · ' + m[i].conds[j].mode + ' · col=' + m[i].conds[j].col);
          }
        }
        return out;
      },
      config: function () { return CFG; },
      render: function (tpl, al) {
        var c = chartCtx();
        return renderTemplate(tpl, msgVars(c, al || { condition: 'cross_up', value: 0 }, '', null));
      },
      vars: function () {
        var c = chartCtx();
        return msgVars(c, { condition: 'cross_up', value: 0, studyName: 'RSI', plotTitle: 'RSI' }, '', null);
      },
      open: function (tab) { toggle(true); if (tab) switchTab(tab); },
      fire: fire
    };
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true });
  } else {
    start();
  }
})();
