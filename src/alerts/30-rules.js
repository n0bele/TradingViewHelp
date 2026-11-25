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

  /** 阈值显示：跟图表报价一样去掉无意义的零，大数加千分位（70 而不是 70.000，10,000,000 而不是 10000000.00） */
  function fmtThreshold(v) {
    if (typeof v !== 'number' || !isFinite(v)) return String(v == null ? '' : v);
    var abs = Math.abs(v);
    var s = (Number.isInteger(v) || abs >= 1000) ? v.toFixed(2) : v.toFixed(4);
    if (s.indexOf('.') >= 0) s = s.replace(/0+$/, '').replace(/\.$/, '');
    var p = s.split('.');
    p[0] = p[0].replace(/\B(?=(\d{3})+(?!\d))/g, ',');
    return p.join('.');
  }

  function rangeText(al) {
    if (al.condSource === 'alertcondition') return '';
    if (al.plotKind === 'discrete') return String(al.value);
    if (!condNeedsValue(al.condition)) return '';
    return al.condition === 'in_range' || al.condition === 'out_range'
      ? fmtThreshold(al.value) + ' ~ ' + fmtThreshold(al.value2)
      : fmtThreshold(al.value);
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
    var isPrice = al.source === 'price';
    var vars = msgVars(ctx, al, plotTitle, cur);
    var title = renderTemplate(al.msgTitle || (isPrice ? DEFAULT_PRICE_TITLE : DEFAULT_TITLE), vars) ||
      (isPrice ? 'TradingView 价格报警' : 'TradingView 指标报警');
    var body = renderTemplate(al.msgBody || (isPrice ? DEFAULT_PRICE_BODY : DEFAULT_BODY), vars) ||
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

