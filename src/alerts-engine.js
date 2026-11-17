/*
 * TVHelper —— 本地指标报警（图表页引擎 + 报警设置页）
 *
 * 设计约束（来自需求）：
 *   1. 只做「指标报警」，不做价格报警；
 *   2. 条件必须绑定在「当前图表标签」上：品种 + 周期 + 该图上真实存在的指标（含参数）；
 *   3. 通知走系统弹窗和/或邮件，两个通道都要能单独开关、能单独测试；
 *   4. 设置持久化，重启后自动恢复；如果图表不存在 / 品种周期不符 / 指标不在图上，
 *      只标记「未就绪」，绝不硬触发。
 *
 * 数据全部来自图表自身的模型（实测于 3.5.0.8375）：
 *   window.TradingViewApi.activeChart()             → symbol() / resolution() / getAllStudies()
 *   chart.getStudyById(id)._study.data()            → PlotList，valueAt(i) = [time, plot0, plot1, ...]
 *   study._valuesProvider.getValues()               → [{index, title, visible}]，用于曲线命名
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

  var CONDS = [
    { id: 'cross_up', label: '上穿' },
    { id: 'cross_down', label: '下穿' },
    { id: 'gt', label: '大于' },
    { id: 'lt', label: '小于' },
    { id: 'in_range', label: '进入区间' },
    { id: 'out_range', label: '离开区间' }
  ];

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

  // ---------------------------------------------------------------- 触发

  function fmtNum(v) {
    if (typeof v !== 'number' || !isFinite(v)) return '-';
    var abs = Math.abs(v);
    if (abs >= 1000) return v.toFixed(2);
    if (abs >= 1) return v.toFixed(abs >= 100 ? 2 : 3);
    return v.toFixed(4);
  }

  function describe(al) {
    var t = al.condition === 'in_range' || al.condition === 'out_range'
      ? fmtNum(al.value) + ' ~ ' + fmtNum(al.value2)
      : fmtNum(al.value);
    return (al.plotTitle || '指标') + ' ' + condLabel(al.condition) + ' ' + t;
  }

  function fire(al, ctx, cur, barTime) {
    var now = Date.now();
    var title = 'TradingView 指标报警';
    var body = [
      al.symbol + ' · ' + al.resolution,
      (al.studyName || '') + '（' + (al.plotTitle || '') + '）= ' + fmtNum(cur),
      '条件：' + describe(al)
    ].join('\n');
    var payload = {
      alertId: al.id,
      title: title,
      body: body,
      symbol: al.symbol,
      resolution: al.resolution,
      study: al.studyName,
      plot: al.plotTitle,
      condition: al.condition,
      value: cur,
      threshold: al.value,
      threshold2: al.value2,
      barTime: barTime,
      notify: Array.isArray(al.notify) && al.notify.length ? al.notify : ['system']
    };
    al.lastFiredAt = now;
    al.lastBarTime = barTime;
    showBanner(body);
    var st = state(al.id);
    st.status = '已触发 ' + new Date(now).toLocaleTimeString();
    st.value = cur;
    BR.fire(payload).then(function (r) {
      if (r && r.errors && r.errors.length) showBanner(body + '\n通知失败：' + r.errors.join('；'), true);
    }).catch(function (e) {
      showBanner(body + '\n通知失败：' + (e && e.message ? e.message : '未知错误'), true);
    });
    BR.state({ id: al.id, lastFiredAt: al.lastFiredAt, lastBarTime: barTime }).catch(function () {});
    // 这里不整份回写配置：多个图表标签共用同一份配置，整份覆盖会吞掉别的标签页刚加的报警。
    // lastFiredAt / lastBarTime 走 state 通道由主进程合并；只有 "once" 停用才需要改整份配置。
  }

  // ---------------------------------------------------------------- 主循环

  function tick() {
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
      fire(al, ctx, cur, ser.time);
      if (al.once) { al.enabled = false; dirty = true; }
    }
    if (dirty) saveConfig();
    pulse();
  }

  function pulse() {
    if (!UI) return;
    var armed = CFG ? CFG.alerts.filter(function (a) { return a.enabled; }).length : 0;
    UI.pill.textContent = armed ? '报警 ' + armed : '报警';
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
        if (al.cooldownMin > 0) bits.push('冷却 ' + al.cooldownMin + ' 分钟');
        if (al.once) bits.push('只触发一次');
        bits.push('通知：' + ((al.notify || []).indexOf('email') >= 0 ? '系统+邮件' : '系统'));
        ref.line.textContent = bits.join(' · ');
      });
    }
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

  // ---------------------------------------------------------------- 设置页 UI

  var FORM = {
    studyId: '',
    plotIndex: 0,
    condition: 'cross_up',
    value: 70,
    value2: 30,
    cooldownMin: 0,
    once: false,
    notifySystem: true,
    notifyEmail: false
  };

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

  function buildUI() {
    var host = document.createElement('div');
    host.id = 'tvx-alerts-host';
    host.style.cssText = 'position:fixed;inset:0;z-index:2147483000;pointer-events:none;';
    var root = host.attachShadow ? host.attachShadow({ mode: 'open' }) : host;

    var style = document.createElement('style');
    style.textContent = [
      ':host{all:initial}',
      '.tvxa-pill{pointer-events:auto;position:fixed;right:14px;bottom:64px;padding:6px 12px;border-radius:14px;',
      'border:1px solid rgba(120,130,150,.5);background:rgba(30,34,45,.92);color:#d1d4dc;font:12px/1.2 -apple-system,',
      '"Segoe UI",Roboto,"Microsoft YaHei",sans-serif;cursor:pointer;box-shadow:0 2px 8px rgba(0,0,0,.35)}',
      '.tvxa-pill:hover{background:rgba(42,46,57,.95)}',
      '.tvxa-mask{pointer-events:auto;position:fixed;inset:0;background:rgba(0,0,0,.45);display:flex;',
      'align-items:center;justify-content:center}',
      '.tvxa-card{width:660px;max-width:calc(100vw - 40px);max-height:78vh;display:flex;flex-direction:column;',
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
      '.tvxa-row{border:1px solid #2a2e39;border-radius:8px;padding:10px;margin-bottom:8px;background:#22262f}',
      '.tvxa-row-head{display:flex;align-items:center;gap:8px}',
      '.tvxa-row-title{flex:1;font-weight:600;color:#e6e8ec;font-size:13px}',
      '.tvxa-row-sub{color:#9aa0aa;font-size:12px;margin-top:4px}',
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
      '.tvxa-field{margin-bottom:10px}',
      '.tvxa-label{font-size:12px;color:#9aa0aa;margin-bottom:4px}',
      '.tvxa-hint{font-size:11px;color:#787b86;margin-top:3px}',
      '.tvxa-input{width:100%;box-sizing:border-box;padding:5px 8px;border-radius:6px;border:1px solid #3d4250;',
      'background:#131722;color:#d1d4dc;font:12px/1.4 inherit;outline:none}',
      '.tvxa-input:focus{border-color:#2962ff}',
      '.tvxa-two{display:grid;grid-template-columns:1fr 1fr;gap:10px}',
      '.tvxa-three{display:grid;grid-template-columns:1fr 1fr 1fr;gap:10px}',
      '.tvxa-check{display:flex;align-items:center;gap:6px;margin:6px 0;font-size:12px;color:#c9ccd3}',
      '.tvxa-check input{accent-color:#2962ff}',
      '.tvxa-empty{color:#787b86;font-size:12px;padding:18px 4px;text-align:center}',
      '.tvxa-banner{pointer-events:none;position:fixed;left:50%;top:16px;transform:translate(-50%,-8px);',
      'max-width:600px;padding:10px 14px;border-radius:8px;color:#f0f3fa;white-space:pre-wrap;',
      'font:12px/1.5 -apple-system,"Segoe UI",Roboto,"Microsoft YaHei",sans-serif;opacity:0;',
      'transition:opacity .2s,transform .2s;box-shadow:0 6px 24px rgba(0,0,0,.4)}'
    ].join('');

    var pill = el('button', 'tvxa-pill', '报警');
    pill.type = 'button';
    pill.title = 'TVHelper 指标报警（Alt+A）';

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
      host: host, root: root, pill: pill, banner: banner, mask: mask, body: body,
      chartInfo: chartInfo, tab: 'list', open: false, tabEls: { list: tabList, new: tabNew, notify: tabNotify }
    };

    function switchTab(name) {
      UI.tab = name;
      Object.keys(UI.tabEls).forEach(function (k) {
        UI.tabEls[k].className = 'tvxa-tab' + (k === name ? ' on' : '');
      });
      if (name === 'list') renderList();
      else if (name === 'new') renderForm();
      else renderNotify();
    }

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
      var rt = state(al.id);
      var extra = [];
      if (rt.value != null) extra.push('当前值 ' + fmtNum(rt.value));
      if (al.lastFiredAt) extra.push('上次触发 ' + new Date(al.lastFiredAt).toLocaleString());
      if (al.cooldownMin > 0) extra.push('冷却 ' + al.cooldownMin + ' 分钟');
      if (al.once) extra.push('只触发一次');
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
    plotSel.addEventListener('change', function () { FORM.plotIndex = Number(plotSel.value); });

    var condSel = selectEl(CONDS.map(function (c) { return { value: c.id, label: c.label }; }), FORM.condition);
    var valueInput = inputEl(FORM.value, 'number', 'any');
    var value2Input = inputEl(FORM.value2, 'number', 'any');
    var cooldownInput = inputEl(FORM.cooldownMin, 'number', '1');
    var onceChk = document.createElement('input');
    onceChk.type = 'checkbox';
    onceChk.checked = !!FORM.once;
    var sysChk = document.createElement('input');
    sysChk.type = 'checkbox';
    sysChk.checked = FORM.notifySystem !== false;
    var mailChk = document.createElement('input');
    mailChk.type = 'checkbox';
    mailChk.checked = !!FORM.notifyEmail;

    var rangeRow = el('div', 'tvxa-two');
    rangeRow.appendChild(field('下限/阈值', valueInput));
    rangeRow.appendChild(field('上限（区间条件用）', value2Input));
    rangeRow.style.display = (FORM.condition === 'in_range' || FORM.condition === 'out_range') ? 'grid' : 'none';
    condSel.addEventListener('change', function () {
      FORM.condition = condSel.value;
      rangeRow.style.display = (FORM.condition === 'in_range' || FORM.condition === 'out_range') ? 'grid' : 'none';
    });

    var grid = el('div', 'tvxa-two');
    grid.appendChild(field('指标（当前图表）', studySel));
    grid.appendChild(field('曲线', plotSel));

    var grid2 = el('div', 'tvxa-three');
    grid2.appendChild(field('条件', condSel));
    grid2.appendChild(field('冷却（分钟，0=不限）', cooldownInput));
    grid2.appendChild(field('触发一次后停用', onceChk));

    var note = el('div', 'tvxa-hint',
      '报警只在当前图表标签上报这个品种、这个周期、这个指标（含参数）时生效。图表关了或指标被删，会显示「未就绪」而不会触发。');

    var chkSys = el('label', 'tvxa-check');
    chkSys.appendChild(sysChk);
    chkSys.appendChild(el('span', null, '系统通知（弹窗）'));
    var chkMail = el('label', 'tvxa-check');
    chkMail.appendChild(mailChk);
    chkMail.appendChild(el('span', null, '邮件通知（需先在「通知设置」里配置 SMTP）'));

    var actions = el('div', 'tvxa-actions');
    actions.appendChild(btn('创建报警', 'primary', function () {
      FORM.value = Number(valueInput.value);
      FORM.value2 = Number(value2Input.value);
      FORM.cooldownMin = Number(cooldownInput.value) || 0;
      FORM.once = !!onceChk.checked;
      FORM.notifySystem = !!sysChk.checked;
      FORM.notifyEmail = !!mailChk.checked;
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
        cooldownMin: FORM.cooldownMin,
        once: FORM.once,
        notify: notify,
        enabled: true,
        createdAt: Date.now(),
        lastFiredAt: 0,
        lastBarTime: 0
      };
      CFG.alerts.push(al);
      saveConfig().then(function (ok) {
        showBanner(ok ? '已创建：' + describe(al) : '保存失败，检查是否允许写入配置', !ok);
        UI.tab = 'list';
        Object.keys(UI.tabEls).forEach(function (k) {
          UI.tabEls[k].className = 'tvxa-tab' + (k === 'list' ? ' on' : '');
        });
        renderList();
      });
    }));
    actions.appendChild(btn('取消', '', function () {
      UI.tab = 'list';
      Object.keys(UI.tabEls).forEach(function (k) {
        UI.tabEls[k].className = 'tvxa-tab' + (k === 'list' ? ' on' : '');
      });
      renderList();
    }));

    body.appendChild(grid);
    body.appendChild(grid2);
    body.appendChild(rangeRow);
    body.appendChild(field('通知方式', (function () {
      var w = el('div');
      w.appendChild(chkSys);
      w.appendChild(chkMail);
      return w;
    })()));
    body.appendChild(note);
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
    grid3.appendChild(field('发件人（留空用账号）', fromI));
    grid3.appendChild(field('收件人（逗号分隔）', toI));

    var secureWrap = el('label', 'tvxa-check');
    secureWrap.appendChild(secureChk);
    secureWrap.appendChild(el('span', null, 'SSL 直连（465 端口勾选；587 端口不勾）'));
    var sysWrap = el('label', 'tvxa-check');
    sysWrap.appendChild(sysChk);
    sysWrap.appendChild(el('span', null, '允许系统通知（弹窗）'));

    var actions = el('div', 'tvxa-actions');
    actions.appendChild(btn('保存通知设置', 'primary', function () {
      CFG.email = {
        host: hostI.value.trim(),
        port: Number(portI.value) || 465,
        secure: !!secureChk.checked,
        user: userI.value.trim(),
        pass: passI.value,
        from: fromI.value.trim(),
        to: toI.value.split(',').map(function (s) { return s.trim(); }).filter(Boolean)
      };
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
      CFG.email = {
        host: hostI.value.trim(),
        port: Number(portI.value) || 465,
        secure: !!secureChk.checked,
        user: userI.value.trim(),
        pass: passI.value,
        from: fromI.value.trim(),
        to: toI.value.split(',').map(function (s) { return s.trim(); }).filter(Boolean)
      };
      saveConfig().then(function () {
        showBanner('正在发送测试邮件…');
        BR.test('email').then(function (r) {
          showBanner(r && r.ok ? '测试邮件已发出' : '邮件发送失败：' + ((r && r.error) || '未知'), !(r && r.ok));
        });
      });
    }));

    body.appendChild(grid);
    body.appendChild(grid2);
    body.appendChild(grid3);
    body.appendChild(secureWrap);
    body.appendChild(sysWrap);
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
      setInterval(tick, TICK_MS);
      setTimeout(tick, 800);
      window.addEventListener('focus', function () { loadConfig(false).then(pulse); });
    });
    // 调试/自检入口
    window.__tvxAlertsDebug = {
      tick: tick,
      ctx: chartCtx,
      studies: function () { var c = chartCtx(); return c ? studyMeta(c.a) : []; },
      config: function () { return CFG; },
      fire: fire
    };
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true });
  } else {
    start();
  }
})();
