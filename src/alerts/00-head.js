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
  if (!BR || typeof BR.get !== 'function') {
    // preload 桥没注入（preload 补丁没命中 / 上游改了 preload 文件名）时，
    // 以前这里是静默 return：用户只看到"报警按钮不见了"，日志里一个字都没有。
    // 留一条可检索的错误，并在页面上挂一个可 hover 的小标记，别让故障无声无息。
    console.error('[tvx-alerts] preload 桥缺失，报警功能未启动（检查 preload 补丁是否命中）');
    try {
      if (!document.getElementById('tvxa-bridge-missing')) {
        var d = document.createElement('div');
        d.id = 'tvxa-bridge-missing';
        d.textContent = '报警不可用';
        d.title = '报警功能未启动：preload 桥缺失。\n' +
          '处理：关掉客户端 → 在助手里重建副本 → 重新启动。\n' +
          '若仍不出现，请把这条红色提示截图反馈（鼠标停在上面就能看到本说明）';
        d.style.cssText = 'position:fixed;right:6px;top:45%;z-index:61;padding:4px 6px;' +
          'border-radius:6px;background:#f23645;color:#fff;cursor:help;' +
          'font:600 10px/1.4 -apple-system,"Segoe UI",sans-serif';
        (document.body || document.documentElement).appendChild(d);
      }
    } catch (e) {}
    return;
  }
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

  /** 报警对象：盯指标输出，还是盯主图价格 */
  var SOURCES = [
    { id: 'study', label: '指标输出' },
    { id: 'price', label: '价格（主图）' }
  ];
  /** 价格报警能用的比较方式：出现/消失是给离散列准备的，价格用不上 */
  var PRICE_CONDS = ['cross_up', 'cross_down', 'gt', 'lt', 'in_range', 'out_range'];

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
  // 价格报警没有指标名，模板单独一套，免得消息里出现空格子
  var DEFAULT_PRICE_TITLE = '{{ticker}} {{interval}} · 价格报警';
  var DEFAULT_PRICE_BODY = '{{ticker}} {{interval}} 价格 {{condition}} {{threshold}}\n当前 {{value}}（开 {{open}} 高 {{high}} 低 {{low}}）';

  function condLabel(id) {
    if (id === COND_CHANGED) return '变为';
    for (var i = 0; i < CONDS.length; i++) if (CONDS[i].id === id) return CONDS[i].label;
    return id;
  }

