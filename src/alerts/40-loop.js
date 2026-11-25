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

      var cur;
      var prev;
      var barTime;
      var plot;
      var hit;
      if (r.price) {
        // 价格报警：取主图当前 K 线收盘价，跟上一根比
        var pp = pricePoint(ctx);
        if (!pp) { rt.status = '读不到价格'; continue; }
        cur = pp.close;
        prev = pp.prev;
        barTime = pp.time;
        plot = al.plotTitle || '价格';
      } else {
        var ser = readSeries(ctx.a, r.study.id);
        if (!ser) { rt.status = '指标暂无数据'; continue; }
        var col = r.cond ? r.cond.col : al.plotIndex;
        cur = ser.cur[col + 1];
        prev = ser.prev ? ser.prev[col + 1] : null;
        barTime = ser.time;
        plot = r.cond ? r.cond.title : (r.study.plots[al.plotIndex] ? r.study.plots[al.plotIndex].title : al.plotTitle);
      }
      rt.value = cur;
      if (rt.bar !== barTime) {
        rt.bar = barTime;
        rt.firedForBar = al.lastBarTime === barTime;
      }
      if (rt.firedForBar) { rt.status = '就绪（本根已触发）'; continue; }
      if (r.price) hit = evalCond(al.condition, prev, cur, Number(al.value), Number(al.value2));
      else if (r.cond) hit = evalSignal(r.cond.mode, prev, cur);
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
      fire(al, ctx, cur, barTime, plot);
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
        if (ref.dot) ref.dot.className = 'tvxa-adot ' + chip.cls;
        if (ref.chip) {
          ref.chip.className = 'tvxa-chip ' + chip.cls;
          ref.chip.textContent = chip.text;
        }
        if (ref.time) ref.time.textContent = agoText(al.lastFiredAt);
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
      // 右侧工具栏不是每张页面都有：非图表页、用户隐藏工具栏、上游改版都会没有。
      // 以前直接 return，按钮永远不出现且没有任何提示；现在退到右下角浮动位，
      // 保证「报警入口」在任何页面上都存在。
      var host = bar;
      if (!host) {
        host = document.getElementById('tvxa-float-host');
        if (!host) {
          host = document.createElement('div');
          host.id = 'tvxa-float-host';
          host.style.cssText = 'position:fixed;right:6px;top:45%;z-index:60;display:flex;' +
            'flex-direction:column;gap:4px';
          (document.body || document.documentElement).appendChild(host);
          console.warn('[tvx-alerts] 未找到右侧工具栏（right-toolbar），报警按钮改为浮动显示');
        }
      }
      var existing = document.getElementById('tvxa-toolbar-btn');
      if (existing) {
        if (existing.parentNode !== host) host.insertBefore(existing, host.firstChild);
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
      var native = bar ? bar.querySelector('[data-name="alerts"]') : null;
      if (native && native.parentNode) bar.insertBefore(b, native.nextSibling);
      else if (bar) bar.insertBefore(b, bar.firstChild);
      else host.appendChild(b);
      UI.toolBtn = b;
      UI.toolBadge = b.querySelector('#tvxa-toolbar-badge');
      pulse();
    } catch (e) {}
  }

