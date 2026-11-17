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

  var CONDS = [
    { id: 'cross_up', label: '上穿' },
    { id: 'cross_down', label: '下穿' },
    { id: 'gt', label: '大于' },
    { id: 'lt', label: '小于' },
    { id: 'in_range', label: '进入区间' },
    { id: 'out_range', label: '离开区间' }
  ];

  // 占位符清单（顺序即设置页里按钮的排列顺序）
  var PLACEHOLDERS = [
    'ticker', 'interval', 'open', 'high', 'low', 'close', 'volume',
    'time', 'study', 'plot', 'condition', 'threshold', 'value'
  ];

  var DEFAULT_TITLE = '{{ticker}} {{interval}} · {{study}} 报警';
  var DEFAULT_BODY = '{{ticker}} {{interval}} {{study}} · {{plot}} {{condition}} {{threshold}}\n当前 {{close}}（开 {{open}} 高 {{high}} 低 {{low}}）';

  function condLabel(id) {
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

  function studyMeta(a) {
    var out = [];
    var list = [];
    try { list = a.getAllStudies() || []; } catch (e) { return out; }
    for (var i = 0; i < list.length; i++) {
      var id = list[i] && list[i].id;
      var name = String((list[i] && list[i].name) || '');
      if (!id || !name) continue;
      var row = { id: String(id), name: name, inputs: [], plots: [], ok: false };
      try {
        var api = a.getStudyById(id);
        if (api && typeof api.getInputValues === 'function') {
          var iv = api.getInputValues() || [];
          for (var j = 0; j < iv.length; j++) row.inputs.push({ id: String(iv[j].id), value: iv[j].value });
        }
        var st = api && api._study;
        var d = plotList(st);
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
            for (var k = 0; k < v.length - 1; k++) {
              row.plots.push({ index: k, title: titles[k] || ('曲线 ' + (k + 1)), value: v[k + 1] });
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
    if (al.plotIndex >= s.plots.length) return { ok: false, reason: '曲线不存在' };
    return { ok: true, study: s };
  }

  // ---------------------------------------------------------------- 条件求值

  function evalCond(cond, prev, cur, v, v2) {
    if (typeof cur !== 'number' || !isFinite(cur)) return false;
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
    return al.condition === 'in_range' || al.condition === 'out_range'
      ? fmtNum(al.value) + ' ~ ' + fmtNum(al.value2)
      : fmtNum(al.value);
  }

  function describe(al) {
    return (al.plotTitle || '指标') + ' ' + condLabel(al.condition) + ' ' + rangeText(al);
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
      plot: plotTitle || al.plotTitle || '',
      condition: condLabel(al.condition),
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
      var cur = ser.cur[al.plotIndex + 1];
      var prev = ser.prev ? ser.prev[al.plotIndex + 1] : null;
      rt.value = cur;
      if (rt.bar !== ser.time) {
        rt.bar = ser.time;
        rt.firedForBar = al.lastBarTime === ser.time;
      }
      if (rt.firedForBar) { rt.status = '就绪（本根已触发）'; continue; }
      if (!evalCond(al.condition, prev, cur, Number(al.value), Number(al.value2))) {
        rt.status = '就绪';
        continue;
      }
      var now = Date.now();
      if (al.cooldownMin > 0 && al.lastFiredAt && now - al.lastFiredAt < al.cooldownMin * 60000) {
        rt.status = '就绪（冷却中）';
        continue;
      }
      rt.firedForBar = true;
      var plot = r.study.plots[al.plotIndex];
      fire(al, ctx, cur, ser.time, plot ? plot.title : al.plotTitle);
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
    plotIndex: 0,
    condition: 'cross_up',
    value: 70,
    value2: 30,
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
      FORM.plotIndex = 0;
    }
    var current = meta.filter(function (s) { return s.id === FORM.studyId; })[0] || meta[0];
    if (FORM.plotIndex >= current.plots.length) FORM.plotIndex = 0;

    // ---- 条件：指标 → 该指标定义的条件（曲线）→ 比较方式
    var studySel = selectEl(meta.map(function (s) {
      return { value: s.id, label: s.name + (s.inputs.length ? '（' + inputsText(s.inputs) + '）' : '') };
    }), FORM.studyId);
    studySel.addEventListener('change', function () {
      FORM.studyId = studySel.value;
      FORM.plotIndex = 0;
      renderForm();
    });

    var plotSel = selectEl(current.plots.map(function (p) {
      return { value: p.index, label: p.title + '（当前 ' + fmtNum(p.value) + '）' };
    }), FORM.plotIndex);
    plotSel.addEventListener('change', function () {
      FORM.plotIndex = Number(plotSel.value);
    });

    var condSel = selectEl(CONDS.map(function (c) { return { value: c.id, label: c.label }; }), FORM.condition);
    var valueInput = inputEl(FORM.value, 'number', 'any');
    var value2Input = inputEl(FORM.value2, 'number', 'any');

    var condGrid = el('div', 'tvxa-two');
    condGrid.appendChild(field('指标', studySel));
    condGrid.appendChild(field('条件（该指标定义的输出）', plotSel));

    var opGrid = el('div', 'tvxa-three');
    opGrid.appendChild(field('比较方式', condSel));
    var vWrap = field('阈值', valueInput);
    var v2Wrap = field('上限（区间条件用）', value2Input);
    opGrid.appendChild(vWrap);
    opGrid.appendChild(v2Wrap);
    var isRange = FORM.condition === 'in_range' || FORM.condition === 'out_range';
    v2Wrap.style.display = isRange ? '' : 'none';
    condSel.addEventListener('change', function () {
      FORM.condition = condSel.value;
      isRange = FORM.condition === 'in_range' || FORM.condition === 'out_range';
      v2Wrap.style.display = isRange ? '' : 'none';
    });

    var condSection = section('条件', [condGrid, opGrid],
      '先选指标，再选它自己的输出（如 RSI 的 RSI、布林带的上/下轨）。');

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
      FORM.value = Number(valueInput.value);
      FORM.value2 = Number(value2Input.value);
      FORM.msgTitle = titleInput.value;
      FORM.msgBody = bodyInput.value;
      FORM.notifySystem = !!sysChk.checked;
      FORM.notifyEmail = !!mailChk.checked;
      FORM.emailFrom = fromInput.value.trim();
      FORM.emailTo = toInput.value.trim();
      if (FORM.condition === 'in_range' || FORM.condition === 'out_range') {
        if (!isFinite(FORM.value) || !isFinite(FORM.value2)) {
          showBanner('区间条件需要填写两个数值', true);
          return;
        }
      } else if (!isFinite(FORM.value)) {
        showBanner('阈值必须是数字', true);
        return;
      }
      var plot = current.plots[FORM.plotIndex] || current.plots[0] || { index: 0, title: '' };
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
        plotIndex: plot.index,
        plotTitle: plot.title,
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
      window.addEventListener('focus', function () { loadConfig(false).then(pulse); });
    });
    // 调试/自检入口
    window.__tvxAlertsDebug = {
      tick: tick,
      ctx: chartCtx,
      bar: function () { return mainBar(chartCtx()); },
      studies: function () { var c = chartCtx(); return c ? studyMeta(c.a) : []; },
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
