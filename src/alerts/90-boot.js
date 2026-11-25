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
        apiKeys: s.apiKeys || '', alertish: s.alertish || [],
        scriptId: s.scriptId || '', fullId: s.fullId || '', miKeys: s.miKeys || '',
        miFrom: s.miFrom || '', miSrcs: s.miSrcs || '', metaConds: s.metaConds || 0,
        pending: !!s.pending, fromSource: !!s.condFromSource,
        plotTypes: s.plots.map(function (x) { return x.id + ':' + x.type; })
      });
    }
    var payload = { symbol: c.symbol, resolution: c.resolution, at: Date.now(), studies: data };
    // 诊断：图表模型上哪些字段可能挂着「指标的呈现数据」（alertConditions 的真正来源）
    try {
      var w = c.a._chartWidget;
      var m = w && typeof w.model === 'function' ? w.model() : null;
      if (m && typeof m.model === 'function') m = m.model();
      var mk = [];
      if (m) for (var k in m) { if (/present|metainfo|study|alert/i.test(k)) mk.push(k); }
      payload.modelKeys = mk.slice(0, 120).join(',');
      payload.engineVer = 'v3-instance-only';
    } catch (e) {}
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
      pine: function (sid) { return pineCache()['id:' + sid] || null; },
      fetchPine: function (sid) { return new Promise(function (r) { fetchSourceConds(sid, r); }); },
      /** 图上这个实例到底从哪些对象读到了元数据 / 条件名（排查用，不猜、不改图） */
      meta: function (name) {
        var m = studies();
        var out = [];
        for (var i = 0; i < m.length; i++) {
          if (name && m[i].name !== name) continue;
          out.push({
            name: m[i].name, scriptId: m[i].scriptId, fullId: m[i].fullId,
            srcs: m[i].miSrcs, from: m[i].miFrom, metaConds: m[i].metaConds,
            conds: m[i].conds.map(function (x) { return x.title; })
          });
        }
        return name ? (out[0] || null) : out;
      },
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
