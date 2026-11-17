/*
 * TVHelper —— 本地指标报警（主进程侧）
 *
 * 职责：
 *   1. 把图表页注入的报警引擎挂到每个 chart 页（dom-ready 时 executeJavaScript）；
 *   2. 提供 IPC：配置读写（userData/tvx-alerts.json）、触发上报、通道测试；
 *   3. 触发时发系统通知（Electron Notification，Win/macOS/Linux 通用）与/或邮件（内置 SMTP 客户端）。
 *
 * 说明：这里不碰任何服务端配额，只是把「本机图表自己算出来的指标值」做成条件判断，
 * 通知也只发到本机/用户自己的邮箱。
 */
'use strict';

const fs = require('fs');
const path = require('path');
const net = require('net');
const tls = require('tls');
const { app, ipcMain, Notification, BrowserWindow, webContents } = require('electron');

const ENGINE_FILE = path.join(__dirname, 'tvx-alerts-engine.js');
let ENGINE = '';
try {
  ENGINE = fs.readFileSync(ENGINE_FILE, 'utf8');
} catch (e) {
  console.error('[tvx-alerts] 读取引擎脚本失败', e);
}

const FIRED = Object.create(null); // alertId -> 最近一次通知时间（多标签页去重）
const FIRE_WINDOW_MS = 15000;

const COND_LABEL = {
  cross_up: '上穿',
  cross_down: '下穿',
  gt: '大于',
  lt: '小于',
  in_range: '进入区间',
  out_range: '离开区间',
  changed_to: '变为'
};

function cfgPath() {
  return path.join(app.getPath('userData'), 'tvx-alerts.json');
}

function logPath() {
  return path.join(app.getPath('userData'), 'tvx-alerts.log');
}

function log(msg) {
  try {
    fs.appendFileSync(logPath(), new Date().toISOString() + ' ' + msg + '\n');
  } catch (e) {}
}

function emptyConfig() {
  return { version: 1, system: true, email: {}, alerts: [] };
}

function normalize(cfg) {
  const c = cfg && typeof cfg === 'object' ? cfg : {};
  if (!Array.isArray(c.alerts)) c.alerts = [];
  if (!c.email || typeof c.email !== 'object') c.email = {};
  if (typeof c.system !== 'boolean') c.system = true;
  c.alerts.forEach((a) => {
    if (!Array.isArray(a.notify) || !a.notify.length) a.notify = ['system'];
    if (typeof a.enabled !== 'boolean') a.enabled = true;
    if (typeof a.cooldownMin !== 'number') a.cooldownMin = 0;
    if (typeof a.lastFiredAt !== 'number') a.lastFiredAt = 0;
    if (typeof a.lastBarTime !== 'number') a.lastBarTime = 0;
    // 消息模板（v2 新增）：标题/正文可自定义，支持 {{ticker}} 等占位符
    if (typeof a.msgTitle !== 'string') a.msgTitle = '';
    if (typeof a.msgBody !== 'string') a.msgBody = '';
    // 邮件发件人/收件人可按报警覆盖全局默认值
    if (typeof a.emailFrom !== 'string') a.emailFrom = '';
    if (typeof a.emailTo !== 'string') a.emailTo = '';
  });
  return c;
}

function readConfig() {
  try {
    return normalize(JSON.parse(fs.readFileSync(cfgPath(), 'utf8')));
  } catch (e) {
    return emptyConfig();
  }
}

function writeConfig(cfg) {
  const p = cfgPath();
  const tmp = p + '.tmp';
  fs.writeFileSync(tmp, JSON.stringify(normalize(cfg), null, 2), 'utf8');
  fs.renameSync(tmp, p);
}

function parseList(v) {
  if (Array.isArray(v)) return v.map((s) => String(s).trim()).filter(Boolean);
  return String(v || '').split(/[,;]/).map((s) => s.trim()).filter(Boolean);
}

function fmtNum(v) {
  if (typeof v !== 'number' || !isFinite(v)) return '-';
  const abs = Math.abs(v);
  if (abs >= 1000) return v.toFixed(2);
  if (abs >= 1) return v.toFixed(abs >= 100 ? 2 : 3);
  return v.toFixed(4);
}

function emailSubject(p) {
  if (p.title) return p.title;
  const cond = COND_LABEL[p.condition] || p.condition || '';
  const target = p.condition === 'in_range' || p.condition === 'out_range'
    ? fmtNum(p.threshold) + '~' + fmtNum(p.threshold2)
    : fmtNum(p.threshold);
  return '[TVHelper] ' + (p.symbol || '') + ' ' + (p.resolution || '') + ' ' +
    (p.plot || '指标') + ' ' + cond + ' ' + target;
}

function emailBody(p) {
  // 图表页已经按用户模板替换好占位符了，直接用；没有模板才回退到结构化正文
  if (p.body) return p.body;
  return [
    'TradingView 指标报警',
    '',
    '品种：' + (p.symbol || '-'),
    '周期：' + (p.resolution || '-'),
    '指标：' + (p.study || '-') + '（' + (p.plot || '-') + '）',
    '当前值：' + fmtNum(p.value),
    '条件：' + (COND_LABEL[p.condition] || p.condition),
    '阈值：' + fmtNum(p.threshold) + (p.threshold2 ? ' ~ ' + fmtNum(p.threshold2) : ''),
    'K 线时间：' + (p.barTime ? new Date(p.barTime * 1000).toLocaleString() : '-'),
    '触发时间：' + new Date().toLocaleString(),
    '',
    '—— 由 TVHelper 本地报警发出（服务端没有这条报警记录，关闭软件即停止监控）'
  ].join('\n');
}

// ------------------------------------------------------------------ 系统通知

function notifySystem(title, body) {
  try {
    if (typeof Notification.isSupported === 'function' && !Notification.isSupported()) {
      return { ok: false, error: '当前系统不支持通知' };
    }
    const n = new Notification({ title: title || 'TradingView 指标报警', body: body || '' });
    n.on('click', () => {
      try {
        const wins = BrowserWindow.getAllWindows();
        if (wins.length) {
          wins[0].show();
          wins[0].focus();
        }
      } catch (e) {}
    });
    n.show();
    return { ok: true };
  } catch (e) {
    return { ok: false, error: String((e && e.message) || e) };
  }
}

// ------------------------------------------------------------------ SMTP

function b64(s) {
  return Buffer.from(String(s), 'utf8').toString('base64');
}

function chunk76(s) {
  const out = [];
  for (let i = 0; i < s.length; i += 76) out.push(s.slice(i, i + 76));
  return out.join('\r\n');
}

class SmtpConn {
  constructor(sock) {
    this.sock = sock;
    this.buf = '';
    this.lines = [];
    this.waiters = [];
    this.ready = []; // 服务器先到的完整响应（例如 220 问候语），等 read() 来取
    this.err = null;
    this.closed = false;
    sock.setEncoding('binary');
    sock.on('data', (d) => {
      this.buf += d;
      this.drain();
    });
    sock.on('error', (e) => this.fail(e));
    sock.on('close', () => this.fail(new Error('SMTP 连接被关闭')));
    sock.setTimeout(25000, () => this.fail(new Error('SMTP 响应超时')));
  }

  drain() {
    let i;
    while ((i = this.buf.indexOf('\r\n')) >= 0) {
      const line = this.buf.slice(0, i);
      this.buf = this.buf.slice(i + 2);
      if (/^\d{3}-/.test(line)) {
        this.lines.push(line);
        continue;
      }
      const code = parseInt(line.slice(0, 3), 10);
      const res = { code: code, line: line, lines: this.lines.concat(line) };
      this.lines = [];
      const w = this.waiters.shift();
      if (w) w.resolve(res);
      else this.ready.push(res);
    }
  }

  fail(e) {
    if (this.closed) return;
    this.closed = true;
    this.err = e;
    const ws = this.waiters;
    this.waiters = [];
    ws.forEach((w) => w.reject(e));
    try {
      this.sock.destroy();
    } catch (x) {}
  }

  read() {
    if (this.ready.length) return Promise.resolve(this.ready.shift());
    if (this.err) return Promise.reject(this.err);
    return new Promise((resolve, reject) => this.waiters.push({ resolve, reject }));
  }

  async cmd(line, expect) {
    const p = this.read();
    if (line != null) this.sock.write(line + '\r\n', 'binary');
    const r = await p;
    if (expect && expect.indexOf(r.code) < 0) {
      throw new Error('SMTP ' + r.code + ' ' + r.line.slice(4));
    }
    return r;
  }

  quit() {
    try {
      if (!this.closed) this.sock.write('QUIT\r\n', 'binary');
      this.sock.end();
    } catch (e) {}
  }
}

function connectSocket(host, port, secure) {
  return new Promise((resolve, reject) => {
    const onErr = (e) => reject(e);
    if (secure) {
      const s = tls.connect({ host: host, port: port, servername: host }, () => resolve(s));
      s.once('error', onErr);
    } else {
      const s = net.connect({ host: host, port: port }, () => resolve(s));
      s.once('error', onErr);
    }
  });
}

async function sendMail(cfg, subject, text) {
  const host = String(cfg.host || '').trim();
  const port = Number(cfg.port) || 465;
  const secure = cfg.secure !== false;
  const user = String(cfg.user || '');
  const pass = String(cfg.pass || '');
  const from = String(cfg.from || user).trim();
  const to = (Array.isArray(cfg.to) ? cfg.to : String(cfg.to || '').split(','))
    .map((s) => String(s).trim())
    .filter(Boolean);

  if (!host) throw new Error('未配置 SMTP 服务器');
  if (!from) throw new Error('未配置发件人');
  if (!to.length) throw new Error('未配置收件人');

  let sock = await connectSocket(host, port, secure);
  let conn = new SmtpConn(sock);
  try {
    await conn.cmd(null, [220]);
    let r = await conn.cmd('EHLO tvhelper', [250]);
    let caps = r.lines.join('\n').toUpperCase();

    if (!secure && caps.indexOf('STARTTLS') >= 0) {
      await conn.cmd('STARTTLS', [220]);
      const raw = sock;
      sock = await new Promise((resolve, reject) => {
        const s = tls.connect({ socket: raw, servername: host }, () => resolve(s));
        s.once('error', reject);
      });
      conn = new SmtpConn(sock);
      r = await conn.cmd('EHLO tvhelper', [250]);
      caps = r.lines.join('\n').toUpperCase();
    }

    if (user) {
      if (caps.indexOf('AUTH') >= 0 && caps.indexOf('PLAIN') >= 0) {
        await conn.cmd('AUTH PLAIN ' + b64('\u0000' + user + '\u0000' + pass), [235]);
      } else {
        await conn.cmd('AUTH LOGIN', [334]);
        await conn.cmd(b64(user), [334]);
        await conn.cmd(b64(pass), [235]);
      }
    }

    await conn.cmd('MAIL FROM:<' + from + '>', [250]);
    for (let i = 0; i < to.length; i++) {
      await conn.cmd('RCPT TO:<' + to[i] + '>', [250, 251]);
    }
    await conn.cmd('DATA', [354]);

    const head = [
      'From: ' + from,
      'To: ' + to.join(', '),
      'Subject: =?UTF-8?B?' + b64(subject) + '?=',
      'Date: ' + new Date().toUTCString(),
      'MIME-Version: 1.0',
      'Content-Type: text/plain; charset=UTF-8',
      'Content-Transfer-Encoding: base64',
      'X-Mailer: TVHelper',
      ''
    ].join('\r\n');

    const body = chunk76(b64(text));
    const p = conn.read();
    conn.sock.write(head + '\r\n' + body + '\r\n.\r\n', 'binary');
    const done = await p;
    if ([250, 251].indexOf(done.code) < 0) {
      throw new Error('SMTP ' + done.code + ' ' + done.line.slice(4));
    }
    conn.quit();
    return { ok: true };
  } catch (e) {
    try {
      conn.fail(e);
    } catch (x) {}
    throw e;
  }
}

// ------------------------------------------------------------------ IPC

ipcMain.handle('tvx-alerts:get', () => {
  try {
    return { ok: true, config: readConfig() };
  } catch (e) {
    return { ok: false, error: String((e && e.message) || e) };
  }
});

ipcMain.handle('tvx-alerts:save', (e, cfg) => {
  try {
    if (!cfg || typeof cfg !== 'object') return { ok: false, error: '配置格式不对' };
    writeConfig(cfg);
    return { ok: true };
  } catch (err) {
    return { ok: false, error: String((err && err.message) || err) };
  }
});

ipcMain.handle('tvx-alerts:state', (e, patch) => {
  try {
    if (!patch || !patch.id) return { ok: false, error: '缺少 id' };
    const cfg = readConfig();
    const al = cfg.alerts.filter((a) => a.id === patch.id)[0];
    if (!al) return { ok: false, error: '报警不存在' };
    if (typeof patch.lastFiredAt === 'number') al.lastFiredAt = patch.lastFiredAt;
    if (typeof patch.lastBarTime === 'number') al.lastBarTime = patch.lastBarTime;
    if (typeof patch.enabled === 'boolean') al.enabled = patch.enabled;
    writeConfig(cfg);
    return { ok: true };
  } catch (err) {
    return { ok: false, error: String((err && err.message) || err) };
  }
});

ipcMain.handle('tvx-alerts:fire', async (e, p) => {
  try {
    if (!p || !p.alertId) return { ok: false, error: '缺少 alertId' };
    const now = Date.now();
    if (FIRED[p.alertId] && now - FIRED[p.alertId] < FIRE_WINDOW_MS) {
      return { ok: true, ignored: true, delivered: [], errors: [] };
    }
    FIRED[p.alertId] = now;

    const cfg = readConfig();
    const notify = Array.isArray(p.notify) && p.notify.length ? p.notify : ['system'];
    const delivered = [];
    const errors = [];

    if (notify.indexOf('system') >= 0 && cfg.system !== false) {
      const r = notifySystem(p.title || 'TradingView 指标报警', p.body || '');
      if (r.ok) delivered.push('system');
      else errors.push('系统通知：' + r.error);
    }

    if (notify.indexOf('email') >= 0) {
      if (!cfg.email || !cfg.email.host) {
        errors.push('邮件：还没有配置 SMTP');
      } else {
        // 单条报警可以覆盖发件人/收件人；没填就用「通知设置」里的默认值
        const mail = Object.assign({}, cfg.email);
        if (p.emailFrom) mail.from = String(p.emailFrom).trim();
        const to = parseList(p.emailTo);
        if (to.length) mail.to = to;
        try {
          await sendMail(mail, emailSubject(p), emailBody(p));
          delivered.push('email');
        } catch (err) {
          errors.push('邮件：' + String((err && err.message) || err));
        }
      }
    }

    log('FIRE ' + p.alertId + ' ' + (p.symbol || '') + ' ' + (p.plot || '') +
      ' value=' + fmtNum(p.value) + ' delivered=' + delivered.join(',') +
      (errors.length ? ' errors=' + errors.join('|') : ''));
    return { ok: errors.length === 0, delivered: delivered, errors: errors };
  } catch (err) {
    return { ok: false, error: String((err && err.message) || err) };
  }
});

ipcMain.handle('tvx-alerts:test', async (e, kind) => {
  try {
    if (kind === 'system') {
      return notifySystem('TVHelper 系统通知测试', '如果你看到这条，说明系统通知通道正常。');
    }
    if (kind === 'email') {
      const cfg = readConfig();
      if (!cfg.email || !cfg.email.host) return { ok: false, error: '还没有配置 SMTP' };
      await sendMail(cfg.email, '[TVHelper] 邮件通道测试',
        '这是一封测试邮件。\n\n收到它说明 SMTP 配置可用，指标报警可以直接发到邮箱。');
      return { ok: true };
    }
    return { ok: false, error: '未知的测试类型' };
  } catch (err) {
    return { ok: false, error: String((err && err.message) || err) };
  }
});

// ------------------------------------------------------------------ 注入

const INJECTED = new Set();

function injectInto(wc, force) {
  if (!ENGINE) return;
  try {
    const url = String(wc.getURL() || '');
    if (!/tradingview\.com\/chart/i.test(url)) return;
    if (!force && INJECTED.has(wc.id)) return;
    // 先占位再执行：扫描与多个事件可能同时命中同一个 webContents，
    // 不先占位会把同一份 34KB 脚本并发执行多次（引擎虽防重复，但没必要）。
    INJECTED.add(wc.id);
    wc.executeJavaScript(ENGINE, true)
      .then(() => {
        log('inject ok ' + wc.id + ' ' + url);
      })
      .catch((e) => {
        INJECTED.delete(wc.id);
        log('inject fail ' + wc.id + ' ' + url + ' ' + String((e && e.message) || e));
      });
  } catch (e) {
    log('inject throw ' + String((e && e.message) || e));
  }
}

app.on('web-contents-created', (e, wc) => {
  try {
    wc.on('dom-ready', () => injectInto(wc));
    wc.on('did-finish-load', () => injectInto(wc));
    wc.on('did-navigate', () => injectInto(wc));
    wc.on('did-navigate-in-page', () => injectInto(wc));
    // 导航开始后旧页面已经没了，标记作废，等新页 dom-ready 重注入
    wc.on('did-start-loading', () => INJECTED.delete(wc.id));
    wc.on('destroyed', () => INJECTED.delete(wc.id));
  } catch (err) {}
});

// 兜底：主进程模块是在 index.js 末尾才加载的，图表 BrowserView 可能比监听器更早建好，
// 那一次 dom-ready 就漏掉了。这里定时扫一遍所有 webContents 补注入（引擎自身防重复）。
let SCAN_LOGGED = false;
setInterval(() => {
  try {
    const all = webContents.getAllWebContents();
    if (!SCAN_LOGGED) {
      SCAN_LOGGED = true;
      log('scan: ' + all.length + ' webContents, engine ' + ENGINE.length + ' bytes');
    }
    all.forEach((wc) => injectInto(wc));
  } catch (e) {
    log('scan fail ' + String((e && e.message) || e));
  }
}, 1000);

try {
  console.log('[tvx-alerts] 本地报警已加载，配置：' + cfgPath());
} catch (e) {}
