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

  /** 主序列（K 线容器）：mainSeries().bars() 优先，退化到 chart.getSeries() */
  function seriesBars(ctx) {
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
      return b || null;
    } catch (e) {
      return null;
    }
  }

  /** 主序列最后一根 K 线：{time, open, high, low, close, volume} */
  function mainBar(ctx) {
    var b = seriesBars(ctx);
    if (!b) return null;
    try {
      var i = typeof b.lastIndex === 'function' ? b.lastIndex() : -1;
      if (i < 0) return null;
      var v = typeof b.valueAt === 'function' ? b.valueAt(i) : null;
      if (!v || typeof v[0] !== 'number') return null;
      return { time: v[0], open: v[1], high: v[2], low: v[3], close: v[4], volume: v[5] };
    } catch (e) {
      return null;
    }
  }

  /**
   * 价格报警的取值：当前 K 线收盘价 + 上一根收盘价。
   * 上穿/下穿必须拿「上一根」比，只拿当前值会永远判成第一次满足。
   */
  function pricePoint(ctx) {
    var b = seriesBars(ctx);
    if (!b) return null;
    try {
      var i = typeof b.lastIndex === 'function' ? b.lastIndex() : -1;
      if (i < 0) return null;
      var v = typeof b.valueAt === 'function' ? b.valueAt(i) : null;
      if (!v || typeof v[0] !== 'number') return null;
      var close = Number(v[4]);
      if (!isFinite(close)) return null;
      var p = i > 0 ? b.valueAt(i - 1) : null;
      var prev = p && isFinite(Number(p[4])) ? Number(p[4]) : null;
      return { time: v[0], close: close, prev: prev, bar: v };
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

  /**
   * 一个指标实例的元数据不止一处：StudyApi / `_study._metaInfo` / 图表模型里的 dataSource。
   * 条件名（alertConditions）与输出列定义可能只出现在其中某一个上，少读一个就会
   * 出现「脚本明明声明了条件，面板却只列出一堆无关输出」。
   * 这里只收**图表上这个实例**的来源，绝不按名字去公开脚本库找替身。
   */
  function metaSources(a, api, st, id) {
    var out = [];
    function push(src, v) {
      if (!v || typeof v !== 'object') return;
      for (var i = 0; i < out.length; i++) if (out[i].v === v) return;
      out.push({ src: src, v: v });
    }
    push('study._metaInfo', metaValue(st));
    try {
      if (st && typeof st.metaInfo === 'function') push('study.metaInfo()', st.metaInfo());
    } catch (e) {}
    try {
      if (api && typeof api.metaInfo === 'function') push('api.metaInfo()', api.metaInfo());
    } catch (e) {}
    try {
      if (api && api._metaInfo) push('api._metaInfo', metaValue(api));
    } catch (e) {}
    try {
      var m = a._chartWidget && typeof a._chartWidget.model === 'function' ? a._chartWidget.model() : null;
      if (m && typeof m.model === 'function') m = m.model();
      var ds = null;
      if (m && typeof m.dataSourceForId === 'function') ds = m.dataSourceForId(id);
      if (!ds && m && typeof m.dataSources === 'function') {
        var arr = m.dataSources() || [];
        for (var i2 = 0; i2 < arr.length; i2++) {
          var did = '';
          try { did = typeof arr[i2].id === 'function' ? String(arr[i2].id()) : String(arr[i2]._id || ''); } catch (e2) {}
          if (did && did === String(id)) { ds = arr[i2]; break; }
        }
      }
      if (ds) {
        try { if (typeof ds.metaInfo === 'function') push('ds.metaInfo()', ds.metaInfo()); } catch (e3) {}
        push('ds._metaInfo', metaValue(ds));
      }
    } catch (e4) {}
    return out;
  }

  /**
   * 原生「创建警报」对话框的条件名就来自这里：presentationData.alertConditions[alertCondId].title
   * （chart-alerts-facade 里 `t?.alertConditions?.[e]` 的用法）。它是映射表，不是数组。
   */
  function alertCondList(mi) {
    var out = [];
    if (!mi) return out;
    var ac = mi.alertConditions || mi.alert_conditions || mi.alertconditions;
    if (!ac) return out;
    try {
      if (Object.prototype.toString.call(ac) === '[object Array]') {
        for (var i = 0; i < ac.length; i++) {
          var it = ac[i];
          var t = (typeof it === 'string') ? it : String((it && (it.title || it.name)) || '');
          if (t) out.push({ id: String((it && it.id) || ('ac' + i)), title: t });
        }
      } else if (typeof ac === 'object') {
        var keys = Object.keys(ac);
        for (var k = 0; k < keys.length; k++) {
          var e = ac[keys[k]];
          var t2 = (typeof e === 'string') ? e : String((e && (e.title || e.name)) || '');
          if (t2) out.push({ id: keys[k], title: t2 });
        }
      }
    } catch (e5) {}
    return out;
  }

  /** 选哪个来源当主元数据：有 alertConditions 的最可信，其次要有 plots / styles */
  function metaScore(v) {
    if (!v) return -1;
    var s = 0;
    if (v.plots && v.plots.length) s += 2;
    if (v.styles) s += 1;
    if (v.fullId) s += 1;
    if (alertCondList(v).length) s += 4;
    return s;
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

