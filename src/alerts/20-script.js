  // ---------------------------------------------------------- 脚本条件（源码兜底）
  //
  // 元数据里完全没有条件名时，才按**这个实例的脚本 id** 去读它自己的源码，把
  // alertcondition(...) 的条件名原样取出来。绝不按指标名去公开库搜同名脚本：
  // "…[MergeAlerts]" 这类改名副本与公开版条件并不相同，按名字会拿到别人的条件。
  // 版本号变更即作废旧缓存：v1/v2 曾按名字命中过 AlgoAlpha 的同名公开脚本
  var PINE_KEY = 'tvx-pine-conds-v3';
  var PINE_MEM = null;
  var PINE_BUSY = Object.create(null);
  var PINE_FAIL = Object.create(null);
  // 指标刚加到图上时元数据可能还没到；按次数限流地重读几次，避免一直停在输出列
  var META_WAIT = Object.create(null);

  function pineCache() {
    if (PINE_MEM) return PINE_MEM;
    try { PINE_MEM = JSON.parse(localStorage.getItem(PINE_KEY) || '{}') || {}; } catch (e) { PINE_MEM = {}; }
    return PINE_MEM;
  }
  function pineSave() {
    try { localStorage.setItem(PINE_KEY, JSON.stringify(pineCache())); } catch (e) {}
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
        pineSave();
        return list;
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
        scriptId: '', fullId: '', miKeys: '', pending: false,
        miFrom: '', miSrcs: '', metaConds: 0, apiKeys: ''
      };
      try {
        var api = a.getStudyById(id);
        if (api && typeof api.getInputValues === 'function') {
          var iv = api.getInputValues() || [];
          for (var j = 0; j < iv.length; j++) row.inputs.push({ id: String(iv[j].id), value: iv[j].value });
        }
        var st = api && api._study;
        var d = plotList(st);
        // 诊断：StudyApi 上哪些字段与元数据/报警相关（不同编译版本位置不同）
        try {
          var aks = [];
          for (var ak2 in api) if (/meta|alert|cond|present/i.test(ak2)) aks.push(ak2);
          row.apiKeys = aks.slice(0, 30).join(',');
        } catch (e6) {}
        // 图上这个实例的所有元数据视图（StudyApi / _study / 图表模型 dataSource）
        var srcs = metaSources(a, api, st, id);
        var mi = {};
        var bestScore = -1;
        var miFrom = '';
        for (var sc1 = 0; sc1 < srcs.length; sc1++) {
          var scv = metaScore(srcs[sc1].v);
          if (scv > bestScore) { bestScore = scv; mi = srcs[sc1].v; miFrom = srcs[sc1].src; }
        }
        row.miFrom = miFrom;
        row.miSrcs = srcs.map(function (x) { return x.src; }).join(',');
        var defs = mi.plots || [];
        var styles = mi.styles || {};
        // 条件名可能只挂在其中一个视图上：全部收上来按顺序去重
        var metaConds = [];
        for (var sc3 = 0; sc3 < srcs.length; sc3++) {
          var cl = alertCondList(srcs[sc3].v);
          for (var c3 = 0; c3 < cl.length; c3++) {
            var dup = false;
            for (var mc = 0; mc < metaConds.length; mc++) if (metaConds[mc].title === cl[c3].title) { dup = true; break; }
            if (!dup) metaConds.push(cl[c3]);
          }
        }
        row.metaConds = metaConds.length;
        // 脚本 id：元数据里一定有，用来直接读「这个脚本自己」的源码，而不是同名公开脚本
        try {
          row.miKeys = Object.keys(mi).join(',');
          row.fullId = String(mi.fullId || '');
          var blob = '';
          for (var sb = 0; sb < srcs.length; sb++) {
            try { blob += JSON.stringify(srcs[sb].v); } catch (e0) {}
          }
          var sm = /(?:^|[^A-Za-z0-9])((?:PUB|USER);[0-9a-fA-F]{32})/.exec(blob);
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

            // 任何时候都先备一份「按输出列」的兜底条件，避免脚本源码读不到时面板一片空白
            var plotConds = [];
            // 是不是 Pine 脚本：有脚本 id 就是；否则看 fullId（内置指标是 @tv-basicstudies-…，
            // Pine 脚本是 @tv-scripting-js），都没有才退回按名字长短判断
            var scriptLike = !!row.scriptId ||
              (row.fullId ? /tv-scripting/i.test(row.fullId) : looksLikeScript(name));
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

            var srcConds = row.scriptId ? pineCache()['id:' + row.scriptId] : null;
            if (condDefs.length) {
              // plots 里就有 alertcondition：按它的 id 定位数据列最准。
              // 名字优先 styles[plotId].title，占位名（plot_19）时用实例的 alertConditions、
              // 再不行用脚本源码同序号的名字补。
              for (var ci = 0; ci < condDefs.length; ci++) {
                var cd = condDefs[ci];
                var cTitle = (styles[cd.id] && styles[cd.id].title) || '';
                if (!cTitle || /^plot_\d+$/i.test(String(cTitle))) {
                  cTitle = (metaConds[ci] && metaConds[ci].title) ||
                    (srcConds && srcConds[ci] && srcConds[ci].title) || ('条件 ' + (ci + 1));
                  if (srcConds && srcConds[ci] && !(metaConds[ci] && metaConds[ci].title)) row.condFromSource = true;
                }
                row.conds.push(buildCond(cd, cTitle, ci, items, row.condCols, defs, d));
              }
            } else if (metaConds.length) {
              // 这个实例自带 alertConditions（与原生「创建警报」对话框同一个来源）：
              // 名字、条数、顺序都以它为准，不看它叫什么名字、是不是公开脚本。
              row.condFromSource = false;
              for (var li2 = 0; li2 < metaConds.length; li2++) {
                row.conds.push(buildCond(
                  { id: metaConds[li2].id, type: 'alertcondition' },
                  metaConds[li2].title, li2, items, false, defs, d
                ));
              }
            } else if (srcConds && srcConds.length) {
              // 元数据里没有条件名：用**这个实例自己的脚本 id** 读到的源码条件名
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
              var metaLoaded = !!(defs && defs.length);
              if (scriptLike) {
                // 脚本指标：先显示输出列 / 图形信号（别让面板空着），同时去读它自己的源码
                var failed = row.scriptId ? PINE_FAIL['id:' + row.scriptId] : false;
                var waited = META_WAIT['w:' + row.id] || 0;
                // 没有脚本 id 时不去猜（绝不按名字搜公开库）；元数据还没到时限流重读几次
                row.pending = !failed && (!!row.scriptId || (!metaLoaded && waited < 4));
                row.needPine = row.pending;
                row.note = (row.note ? row.note + '；' : '') + (failed
                  ? '没读到这个脚本的源码（不是公开脚本，或当前账号无权限），下面列出的是它在图上的输出与图形信号'
                  : (row.scriptId
                    ? '条件名还在读取，当前列出的是指标输出列与图形信号'
                    : (!metaLoaded
                      ? '正在读取这个指标实例的元数据…'
                      : '没拿到这个脚本在图上的脚本 id，只能按它的输出列与图形信号列条件')));
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

  /**
   * 指标参数的可读摘要：只留人能看懂的输入。
   * Pine 的 getInputValues() 里混着 pineId / pineVersion / pineFeatures / in_0… 这些内部项，
   * 还可能带一大坨 base64 长文本（脚本自己的配置串），原样显示会糊满整行。
   */
  function inputsText(inputs) {
    if (!inputs || !inputs.length) return '';
    var out = [];
    for (var i = 0; i < inputs.length; i++) {
      var id = String((inputs[i] && inputs[i].id) || '');
      if (!id || /^(pine|__|in_\d|in$)/i.test(id)) continue;
      var v = String(inputs[i].value);
      if (v.length > 32) continue;
      out.push(id + '=' + v);
    }
    return out.join(', ');
  }

  /** 把一条报警对到当前图表上：品种 / 周期 / 指标（名字+参数）/ 曲线 */
  function resolveAlert(al, meta, ctx) {
    if (!ctx) return { ok: false, reason: '图表未就绪' };
    if (al.symbol && al.symbol !== ctx.symbol) return { ok: false, reason: '品种不匹配：当前 ' + ctx.symbol };
    if (al.resolution && String(al.resolution) !== ctx.resolution) {
      return { ok: false, reason: '周期不匹配：当前 ' + ctx.resolution };
    }
    // 价格报警不依赖任何指标：只要品种/周期对上就能判定，
    // 图上没有指标（哪怕指标槽位用完了）也照样能建。
    if (al.source === 'price') return { ok: true, price: true, study: null, cond: null };
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

