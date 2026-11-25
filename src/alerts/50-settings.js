  // ---------------------------------------------------------------- 设置页 UI

  var FORM = {
    /** 'study' | 'price'：新建报警时盯指标输出还是盯主图价格 */
    source: 'study',
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
    // 不用 Shadow DOM：影子树会把 document.activeElement / event.target 重定向成宿主 div，
    // 图表自己的全局键盘处理认不出「焦点在输入框里」，于是把每个按键当热键拦掉
    // （preventDefault），面板里的输入框就一个字符都打不进去。样式改用 #tvx-alerts-host 前缀隔离。
    var root = host;
    // 送礼给推广浮层清理器（patch.rs 的 __tvxGuard）：它是按文案/结构猜的，
    // 面板进 light DOM 后文本对它可见，标记一下让它永不误伤
    host.__tvxH = 1;

    var style = document.createElement('style');
    style.textContent = [
      '#tvx-alerts-host{all:initial}',
      '#tvx-alerts-host,#tvx-alerts-host *{box-sizing:border-box}',
      '#tvx-alerts-host *{font-family:-apple-system,"Segoe UI",Roboto,"Microsoft YaHei",sans-serif}',
      // 悬浮入口：品牌色实心胶囊，带铃铛和计数，比原来的灰字显眼得多
      '#tvx-alerts-host .tvxa-pill{pointer-events:auto;position:fixed;right:64px;bottom:28px;display:flex;align-items:center;gap:7px;',
      'padding:8px 13px 8px 11px;border-radius:20px;border:1px solid rgba(41,98,255,.75);background:#2962ff;',
      'color:#fff;font:600 12px/1 -apple-system,"Segoe UI",Roboto,"Microsoft YaHei",sans-serif;cursor:pointer;',
      'box-shadow:0 6px 18px rgba(41,98,255,.42);transition:transform .15s,box-shadow .15s,background .15s;z-index:2}',
      '#tvx-alerts-host .tvxa-pill:hover{background:#1e53e5;transform:translateY(-1px);box-shadow:0 8px 22px rgba(41,98,255,.55)}',
      '#tvx-alerts-host .tvxa-pill .tvxa-ico{display:flex;align-items:center}',
      '#tvx-alerts-host .tvxa-pill-count{background:rgba(255,255,255,.24);border-radius:9px;padding:1px 5px;font-size:10px}',
      '#tvx-alerts-host .tvxa-pill.toolbar-hidden{display:none}',
      '#tvx-alerts-host .tvxa-hot{animation:tvxaPulse 1s ease-in-out 3}',
      '@keyframes tvxaPulse{0%,100%{box-shadow:0 6px 18px rgba(41,98,255,.42)}',
      '50%{box-shadow:0 0 0 6px rgba(242,54,69,.55)}}',
      '#tvx-alerts-host .tvxa-mask{pointer-events:auto;position:fixed;inset:0;background:rgba(0,0,0,.45);display:flex;',
      'align-items:center;justify-content:center}',
      '#tvx-alerts-host .tvxa-card{width:680px;max-width:calc(100vw - 40px);max-height:calc(100vh - 24px);display:flex;flex-direction:column;',
      'background:#1e222d;color:#d1d4dc;border:1px solid #363a45;border-radius:10px;overflow:hidden;',
      'box-shadow:0 12px 48px rgba(0,0,0,.5);font:13px/1.45 -apple-system,"Segoe UI",Roboto,"Microsoft YaHei",sans-serif}',
      '#tvx-alerts-host .tvxa-head{display:flex;align-items:center;gap:10px;padding:12px 14px;border-bottom:1px solid #2a2e39}',
      '#tvx-alerts-host .tvxa-title{font-size:14px;font-weight:600;flex:1}',
      '#tvx-alerts-host .tvxa-chart{font-size:12px;color:#787b86}',
      '#tvx-alerts-host .tvxa-tabs{display:flex;gap:4px;padding:8px 14px 0}',
      '#tvx-alerts-host .tvxa-tab{padding:5px 12px;border-radius:6px 6px 0 0;background:transparent;border:1px solid transparent;',
      'color:#a3a6af;cursor:pointer;font:inherit}',
      '#tvx-alerts-host .tvxa-tab.on{background:#2a2e39;border-color:#363a45;border-bottom-color:#2a2e39;color:#e6e8ec}',
      '#tvx-alerts-host .tvxa-body{padding:12px 14px 16px;overflow:auto;flex:1}',
      '#tvx-alerts-host .tvxa-section{border:1px solid #2a2e39;border-radius:8px;padding:8px 10px;margin-bottom:8px;background:#1b1f29}',
      '#tvx-alerts-host .tvxa-sec-title{font-size:12px;font-weight:600;color:#e6e8ec;margin-bottom:6px;letter-spacing:.3px}',
      // 报警列表：照原生「警报」列表的样子——一行一条、无卡片框、细节收进展开区
      '#tvx-alerts-host .tvxa-listhead{font-size:11px;color:#787b86;padding:2px 2px 6px;letter-spacing:.2px}',
      '#tvx-alerts-host .tvxa-arow{border-bottom:1px solid #262a34}',
      '#tvx-alerts-host .tvxa-arow:last-child{border-bottom:0}',
      '#tvx-alerts-host .tvxa-arow.off{opacity:.62}',
      '#tvx-alerts-host .tvxa-ahead{display:flex;align-items:center;gap:9px;padding:9px 4px;border-radius:6px;cursor:pointer}',
      '#tvx-alerts-host .tvxa-ahead:hover{background:#232733}',
      '#tvx-alerts-host .tvxa-adot{width:7px;height:7px;border-radius:50%;background:#787b86;flex:0 0 auto}',
      '#tvx-alerts-host .tvxa-adot.ok{background:#26a69a}',
      '#tvx-alerts-host .tvxa-adot.fire{background:#2962ff}',
      '#tvx-alerts-host .tvxa-adot.warn{background:#e0a03a}',
      '#tvx-alerts-host .tvxa-adot.off{background:#4a4e5a}',
      '#tvx-alerts-host .tvxa-amain{flex:1;min-width:0}',
      '#tvx-alerts-host .tvxa-atitle{font-size:13px;font-weight:600;color:#e6e8ec;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}',
      '#tvx-alerts-host .tvxa-asub{font-size:11px;color:#787b86;margin-top:2px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}',
      '#tvx-alerts-host .tvxa-aright{display:flex;align-items:center;gap:6px;flex:0 0 auto;color:#787b86;font-size:11px}',
      '#tvx-alerts-host .tvxa-mini{background:transparent;border:0;color:#787b86;cursor:pointer;font:14px/1 inherit;padding:2px 5px;border-radius:4px}',
      '#tvx-alerts-host .tvxa-mini:hover{background:#333846;color:#d1d4dc}',
      '#tvx-alerts-host .tvxa-adetail{display:none;padding:2px 4px 10px 22px;font-size:12px;color:#9aa0aa}',
      '#tvx-alerts-host .tvxa-arow.open .tvxa-adetail{display:block}',
      '#tvx-alerts-host .tvxa-kv{margin-top:3px;line-height:1.5;word-break:break-word}',
      '#tvx-alerts-host .tvxa-chip{font-size:11px;padding:2px 7px;border-radius:10px;border:1px solid #3d4250;color:#a3a6af}',
      '#tvx-alerts-host .tvxa-chip.ok{color:#4caf50;border-color:#2e5c33}',
      '#tvx-alerts-host .tvxa-chip.warn{color:#e0a03a;border-color:#5c4a2a}',
      '#tvx-alerts-host .tvxa-chip.off{color:#787b86}',
      '#tvx-alerts-host .tvxa-chip.fire{color:#3d8bff;border-color:#2a4a7a}',
      '#tvx-alerts-host .tvxa-actions{display:flex;gap:6px;margin-top:8px;flex-wrap:wrap}',
      '#tvx-alerts-host .tvxa-btn{padding:4px 10px;border-radius:6px;border:1px solid #3d4250;background:#2a2e39;color:#d1d4dc;',
      'cursor:pointer;font:12px/1.4 inherit}',
      '#tvx-alerts-host .tvxa-btn:hover{background:#333846}',
      '#tvx-alerts-host .tvxa-btn.primary{background:#2962ff;border-color:#2962ff;color:#fff}',
      '#tvx-alerts-host .tvxa-btn.danger{color:#ff7b72;border-color:#5a3230}',
      '#tvx-alerts-host .tvxa-btn.ph{padding:2px 7px;font-size:11px;color:#8ab0ff;border-color:#2f3f63;background:#1a2130}',
      '#tvx-alerts-host .tvxa-field{margin-bottom:8px}',
      '#tvx-alerts-host .tvxa-label{font-size:12px;color:#9aa0aa;margin-bottom:4px}',
      '#tvx-alerts-host .tvxa-hint{font-size:11px;color:#787b86;margin-top:3px;line-height:1.5}',
      '#tvx-alerts-host .tvxa-input{width:100%;box-sizing:border-box;padding:5px 8px;border-radius:6px;border:1px solid #3d4250;',
      'background:#131722;color:#d1d4dc;font:12px/1.4 inherit;outline:none}',
      '#tvx-alerts-host .tvxa-input:focus{border-color:#2962ff}',
      'textarea.tvxa-input{resize:vertical;min-height:52px;font-family:inherit}',
      '#tvx-alerts-host .tvxa-two{display:grid;grid-template-columns:1fr 1fr;gap:10px}',
      '#tvx-alerts-host .tvxa-stack{display:flex;flex-direction:column;gap:10px}',
      '#tvx-alerts-host .tvxa-three{display:grid;grid-template-columns:1fr 1fr 1fr;gap:10px}',
      '#tvx-alerts-host .tvxa-check{display:flex;align-items:center;gap:6px;margin:6px 0;font-size:12px;color:#c9ccd3}',
      '#tvx-alerts-host .tvxa-check input{accent-color:#2962ff}',
      '#tvx-alerts-host .tvxa-info{display:flex;gap:14px;flex-wrap:wrap;font-size:12px;color:#a3a6af}',
      '#tvx-alerts-host .tvxa-info b{color:#d1d4dc;font-weight:600}',
      '#tvx-alerts-host .tvxa-empty{color:#787b86;font-size:12px;padding:18px 4px;text-align:center}',
      '#tvx-alerts-host .tvxa-banner{pointer-events:none;position:fixed;left:50%;top:16px;transform:translate(-50%,-8px);',
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
    // 样式进 head、面板进 body：light DOM 下才有真实的事件目标与 activeElement
    (document.head || document.documentElement).appendChild(style);
    root.appendChild(pill);
    root.appendChild(banner);
    root.appendChild(mask);
    (document.body || document.documentElement).appendChild(host);

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
      // 焦点在面板的输入框里时不抢 Alt+A（那是要打字符/系统快捷键的人）
      var t = e.target;
      var typing = !!(t && /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
      if (e.altKey && !e.ctrlKey && !e.metaKey && (e.key === 'a' || e.key === 'A') && !typing) {
        e.preventDefault();
        toggle(!UI.open);
      } else if (e.key === 'Escape' && UI.open && !typing) {
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

  /** 列表里的报警名，跟原生一样「条件为主，指标为辅」 */
  function alertName(al) {
    if (al.condSource === 'alertcondition') return String(al.condTitle || al.plotTitle || '脚本条件');
    var tail = rangeText(al);
    // 列表标题要短：「出现（从无到有）」这类括注只在设置页有用
    var cl = condLabel(al.condition).replace(/（[^）]*）/g, '');
    return String((al.plotTitle || '指标') + ' ' + cl + (tail ? ' ' + tail : ''));
  }

  /** 原生列表右侧那行时间：刚刚 / 12 分钟前 / 3 小时前 / 昨天 14:02 */
  function agoText(ts) {
    if (!ts) return '未触发';
    var d = Date.now() - Number(ts);
    if (!isFinite(d) || d < 0) return '未触发';
    if (d < 60e3) return '刚刚';
    if (d < 3600e3) return Math.floor(d / 60e3) + ' 分钟前';
    if (d < 864e5) return Math.floor(d / 3600e3) + ' 小时前';
    var dt = new Date(Number(ts));
    var hh = ('0' + dt.getHours()).slice(-2) + ':' + ('0' + dt.getMinutes()).slice(-2);
    return (d < 1728e5 ? '昨天 ' : (dt.getMonth() + 1) + '/' + dt.getDate() + ' ') + hh;
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
    // 与原生「警报」列表同样的结构：一行一条，左边状态点、中间标题+副标题、右边最近触发时间，
    // 细节与操作收进展开区。指标参数只在展开区里显示可读项，绝不铺内部签名/长文本。
    var head = el('div', 'tvxa-listhead', alerts.length + ' 条报警' +
      (armedCount() !== alerts.length ? '（' + armedCount() + ' 条运行中）' : ''));
    body.appendChild(head);
    alerts.forEach(function (al) {
      var chip = statusChip(al);
      var row = el('div', 'tvxa-arow' + (al.enabled ? '' : ' off'));
      var bar = el('div', 'tvxa-ahead');
      var dot = el('span', 'tvxa-adot ' + chip.cls);
      var main = el('div', 'tvxa-amain');
      main.appendChild(el('div', 'tvxa-atitle', alertName(al)));
      var sub = [];
      if (al.symbol) sub.push(al.symbol);
      if (al.resolution) sub.push(String(al.resolution));
      if (al.source === 'price') sub.push('价格');
      else if (al.studyName) sub.push(al.studyName);
      main.appendChild(el('div', 'tvxa-asub', sub.join(' · ')));
      var right = el('div', 'tvxa-aright');
      var time = el('div', 'tvxa-atime', agoText(al.lastFiredAt));
      var more = el('button', 'tvxa-mini', '⋯');
      more.type = 'button';
      more.title = '详情与操作';
      right.appendChild(time);
      right.appendChild(more);
      bar.appendChild(dot);
      bar.appendChild(main);
      bar.appendChild(right);
      row.appendChild(bar);

      var detail = el('div', 'tvxa-adetail');
      var rt = state(al.id);
      var chipEl = el('span', 'tvxa-chip ' + chip.cls, chip.text);
      var stLine = el('div', 'tvxa-kv', '状态：');
      stLine.appendChild(chipEl);
      detail.appendChild(stLine);
      var bits = [];
      bits.push('条件：' + describe(al));
      if (rt.value != null) bits.push('当前值：' + fmtNum(rt.value));
      if (al.lastFiredAt) bits.push('上次触发：' + new Date(al.lastFiredAt).toLocaleString());
      bits.push('通知：' + ((al.notify || []).indexOf('email') >= 0 ? '系统 + 邮件' : '系统'));
      for (var bi = 0; bi < bits.length; bi++) detail.appendChild(el('div', 'tvxa-kv', bits[bi]));
      if (al.msgTitle) detail.appendChild(el('div', 'tvxa-kv', '消息：' + al.msgTitle));
      var params = inputsText(al.studyInputs);
      if (params) detail.appendChild(el('div', 'tvxa-kv', '参数：' + params));

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
      detail.appendChild(acts);
      row.appendChild(detail);

      var toggleRow = function () { row.classList.toggle('open'); };
      bar.addEventListener('click', toggleRow);
      more.addEventListener('click', function (e) { e.stopPropagation(); toggleRow(); });

      UI.rows.push({ id: al.id, chip: chipEl, dot: dot, time: time });
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

    // ---- 报警对象：指标输出 / 价格。价格不依赖指标，所以放在"有没有指标"的判断之前。
    var srcSel = selectEl(SOURCES.map(function (s) {
      return { value: s.id, label: s.label };
    }), FORM.source || 'study');
    srcSel.addEventListener('change', function () {
      FORM.source = srcSel.value === 'price' ? 'price' : 'study';
      FORM.value = '';
      FORM.value2 = '';
      FORM.dvalue = '';
      renderForm();
    });
    body.appendChild(section('报警对象', [field('类型', srcSel)],
      FORM.source === 'price'
        ? '盯主图当前 K 线收盘价，不需要任何指标，也不占指标槽位。'
        : '盯图上已加载指标的输出列或脚本条件。'));

    if (FORM.source === 'price') {
      renderPriceForm(ctx);
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
    // 脚本指标若元数据里没有条件名，按**这个实例自己的脚本 id**读它自己的源码补条件名，
    // 读完自渲染一次；失败则退回按输出列给条件，不会反复请求。
    // 没有脚本 id 就不猜：绝不按指标名去公开库搜同名脚本（[MergeAlerts] 这类副本条件不同）
    var needList = meta.filter(function (s) { return s.pending; });
    for (var ni = 0; ni < needList.length; ni++) {
      (function (s) {
        if (!s.scriptId) {
          // 元数据可能还没到：重读一次实例元数据（限 4 次，不会打转）
          var k = 'w:' + s.id;
          META_WAIT[k] = (META_WAIT[k] || 0) + 1;
          if (META_WAIT[k] <= 4) {
            setTimeout(function () { if (UI && FORM.studyId === s.id) renderForm(); }, 1200);
          }
          return;
        }
        var done = function (list) {
          // 成功或失败都重画一次：失败后标签要变成「输出列」并把原因写出来
          if (UI && FORM.studyId === s.id) renderForm();
        };
        fetchSourceConds(s.scriptId, done);
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

    // 指标名可能很长（"Machine Learning Adaptive SuperTrend [MergeAlerts]"），
    // 指标/条件各占一整行，名字才不会被下拉宽度截断
    studySel.title = current.name;
    condSel.title = curCond.title;
    var condGrid = el('div', 'tvxa-stack');
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

    // 创建时的校验与字段：指标报警要按「脚本条件 / 离散列 / 数值列」分开处理，
    // 校验失败返回 {ok:false,msg}，由 renderTail 统一弹提示。
    var makeAlert = function () {
      if (curCond.source === 'alertcondition') {
        if (!curCond.available) return { ok: false, msg: '这个条件客户端读不到信号，换一个条件' };
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
          if (!isFinite(FORM.value) || !isFinite(FORM.value2)) return { ok: false, msg: '区间条件需要填写两个数值' };
        } else if (!isFinite(FORM.value)) {
          return { ok: false, msg: '阈值必须是数字' };
        }
      }
      return {
        ok: true,
        alert: {
          source: 'study',
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
          value2: FORM.value2
        }
      };
    };

    body.appendChild(condSection);
    renderTail(body, ctx, makeAlert);
  }

  /** 价格报警的表单：只比收盘价，条件 + 阈值，其余交给共享尾部 */
  function renderPriceForm(ctx) {
    var body = UI.body;
    var pp = pricePoint(ctx);
    var last = pp && isFinite(pp.close) ? pp.close : null;
    if (PRICE_CONDS.indexOf(FORM.condition) < 0) FORM.condition = 'cross_up';

    var condSel = selectEl(CONDS.filter(function (c) { return PRICE_CONDS.indexOf(c.id) >= 0; })
      .map(function (c) { return { value: c.id, label: c.label }; }), FORM.condition);
    var v1 = FORM.value === '' || FORM.value == null ? (last == null ? '' : String(last)) : FORM.value;
    var valueInput = inputEl(v1, 'number', 'any');
    var value2Input = inputEl(FORM.value2 == null ? '' : FORM.value2, 'number', 'any');
    // 数字框默认带 spinner；滚轮划过时先取消焦点，免得滚表单把阈值滚变了
    [valueInput, value2Input].forEach(function (n) {
      n.setAttribute('inputmode', 'decimal');
      n.addEventListener('wheel', function () { n.blur(); }, { passive: true });
    });

    var opGrid = el('div', 'tvxa-three');
    // 阈值可直接手输；空着时预填当前价，下面小字只是提示，不是让你去点
    var v1Wrap = field('阈值', valueInput, last == null ? '' : '当前价 ' + fmtNum(last));
    var v2Wrap = field('上限（区间条件用）', value2Input);
    opGrid.appendChild(field('比较方式', condSel));
    opGrid.appendChild(v1Wrap);
    opGrid.appendChild(v2Wrap);

    var syncOps = function () {
      var need = condNeedsValue(FORM.condition);
      var isRange = need && (FORM.condition === 'in_range' || FORM.condition === 'out_range');
      v1Wrap.style.display = need ? '' : 'none';
      v2Wrap.style.display = isRange ? '' : 'none';
    };
    FORM.condition = condSel.value;
    syncOps();
    condSel.addEventListener('change', function () {
      FORM.condition = condSel.value;
      syncOps();
    });

    body.appendChild(section('价格条件', [opGrid],
      '价格取主图当前 K 线的收盘价（实时）。上穿/下穿会拿上一根的收盘价比较，只在穿越那一下触发。'));

    var makeAlert = function () {
      if (!condNeedsValue(FORM.condition)) {
        FORM.value = 0;
        FORM.value2 = 0;
      } else if (FORM.condition === 'in_range' || FORM.condition === 'out_range') {
        FORM.value = Number(valueInput.value);
        FORM.value2 = Number(value2Input.value);
        if (!isFinite(FORM.value) || !isFinite(FORM.value2)) {
          return { ok: false, msg: '区间条件需要填写两个数值' };
        }
      } else {
        if (String(valueInput.value).trim() === '') return { ok: false, msg: '阈值必须是数字' };
        FORM.value = Number(valueInput.value);
        if (!isFinite(FORM.value)) return { ok: false, msg: '阈值必须是数字' };
      }
      return {
        ok: true,
        alert: {
          source: 'price',
          studyName: '',
          studyInputs: null,
          condId: '',
          condTitle: '',
          condSource: '',
          condMode: '',
          plotIndex: 0,
          plotTitle: '价格',
          plotKind: 'numeric',
          plotType: '',
          condition: FORM.condition,
          value: FORM.value,
          value2: FORM.value2
        }
      };
    };
    renderTail(body, ctx, makeAlert);
  }

  /**
   * 表单尾部（周期提示 / 消息 / 通知 / 创建按钮）——指标报警与价格报警共用。
   * `makeAlert()` 负责各自的字段与校验：返回 {ok:false,msg} 或 {ok:true,alert}。
   */
  function renderTail(body, ctx, makeAlert) {
    var info = el('div', 'tvxa-info');
    var infoPeriod = el('div');
    infoPeriod.innerHTML = '周期：<b>' + (ctx.resolution || '-') + '</b>（与图表一致）';
    var infoTrigger = el('div');
    infoTrigger.innerHTML = '触发：<b>每根 K 线最多一次</b>';
    info.appendChild(infoPeriod);
    info.appendChild(infoTrigger);
    body.appendChild(section('周期和触发', [info],
      '不单独设置，始终跟随当前图表的品种与周期。'));

    var isPrice = FORM.source === 'price';
    var tplTitle = isPrice ? DEFAULT_PRICE_TITLE : DEFAULT_TITLE;
    var tplBody = isPrice ? DEFAULT_PRICE_BODY : DEFAULT_BODY;
    // 输入框里显示的就是最终模板；当前值还是"另一类的默认模板"时换成这一类的
    var startTitle = FORM.msgTitle && FORM.msgTitle !== DEFAULT_TITLE && FORM.msgTitle !== DEFAULT_PRICE_TITLE
      ? FORM.msgTitle : tplTitle;
    var startBody = FORM.msgBody && FORM.msgBody !== DEFAULT_BODY && FORM.msgBody !== DEFAULT_PRICE_BODY
      ? FORM.msgBody : tplBody;

    var titleInput = inputEl(startTitle, 'text');
    var bodyInput = document.createElement('textarea');
    bodyInput.className = 'tvxa-input';
    bodyInput.value = startBody;
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
    body.appendChild(msgSection);

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

    body.appendChild(section('通知', [chkSys, chkMail, mailGrid],
      'SMTP 在「通知设置」里配一次；这里的收发件人只对这一条生效。'));

    var actions = el('div', 'tvxa-actions');
    actions.appendChild(btn('创建报警', 'primary', function () {
      FORM.msgTitle = titleInput.value;
      FORM.msgBody = bodyInput.value;
      FORM.notifySystem = !!sysChk.checked;
      FORM.notifyEmail = !!mailChk.checked;
      FORM.emailFrom = fromInput.value.trim();
      FORM.emailTo = toInput.value.trim();

      var built = makeAlert();
      if (!built || !built.ok) {
        showBanner((built && built.msg) || '条件不完整，检查后再建', true);
        return;
      }
      var notify = [];
      if (FORM.notifySystem) notify.push('system');
      if (FORM.notifyEmail) notify.push('email');
      if (!notify.length) notify.push('system');

      var al = built.alert;
      al.id = newAlertId();
      al.symbol = ctx.symbol;
      al.resolution = ctx.resolution;
      al.cooldownMin = 0;
      al.once = false;
      al.msgTitle = FORM.msgTitle;
      al.msgBody = FORM.msgBody;
      al.notify = notify;
      al.emailFrom = FORM.notifyEmail ? FORM.emailFrom : '';
      al.emailTo = FORM.notifyEmail ? FORM.emailTo : '';
      al.enabled = true;
      al.createdAt = Date.now();
      al.lastFiredAt = 0;
      al.lastBarTime = 0;
      CFG.alerts.push(al);
      saveConfig().then(function (ok) {
        showBanner(ok ? '已创建：' + describe(al) : '保存失败，检查是否允许写入配置', !ok);
        gotoList();
      });
    }));
    actions.appendChild(btn('取消', '', function () { gotoList(); }));
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

