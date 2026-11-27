//! 后台状态与工作线程：UI 线程只取结果，不自己做进程枚举 / 注册表查询。
//!
//! - `Shared`：跨线程共享的日志、忙碌标志、待消费的盘点结果
//! - `spawn_scan_thread`：每 2 秒一轮盘点（可被 `wake` 提前唤醒）
//! - `run_job`：一次性后台任务，输出流式追加到日志

use crate::app::authwatch::open_auth_urls_if_new;
use crate::install::{self, Install, Managed};
use crate::profiles::{self, Profile};
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const SCAN_INTERVAL: Duration = Duration::from_secs(2);

/// 「全部启动」时相邻实例的间隔（秒）
pub(crate) const STAGGER_SECS: u64 = 3;

/// 账号启动后多久内保持"接管浏览器回调"的监听（秒）。
///
/// 覆盖「启动 → 点登录 → 浏览器里输入账号密码/二次验证 → 授权完成」这一段。
/// 再长就没意义了：超过这个时间用户要么已经登录完，要么去干别的了，
/// 而监听要每 1.2 秒查一次进程列表，没必要一直开着。
const HANDOFF_WINDOW_SECS: u64 = 15 * 60;

#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) log: Vec<String>,
    pub(crate) busy: bool,
    /// 创建账号的结果（表单要等它：成功才关闭，失败就在表单里显示原因）
    pub(crate) add_done: Option<Result<(), String>>,
    /// 后台扫描线程的最新盘点结果，UI 线程每帧取走消费
    pub(crate) scan: Option<Scan>,
    /// 向扫描线程发"立即加扫一轮"信号（无界通道，send 永不阻塞）
    pub(crate) kick: Option<std::sync::mpsc::Sender<()>>,
}

/// 一轮后台盘点的全部产出，与 App 的对应字段一一对应。
pub(crate) struct Scan {
    pub(crate) installs: Vec<Install>,
    pub(crate) managed: Option<Managed>,
    pub(crate) profiles: Vec<Profile>,
    pub(crate) status: HashMap<String, Option<u32>>,
    /// 扫描期间产生的提示（如"已在无痕窗口打开登录页"）
    pub(crate) log: Vec<String>,
}

pub(crate) fn scan_now(ctx: Option<&egui::Context>) -> Scan {
    let installs = install::discover();
    let managed = install::load_managed();
    let profiles = profiles::list();
    let status = profiles::status_snapshot(&profiles);
    // 应用自己会改写 tradingview:// 协议命令，这里每轮纠回来
    profiles::maintain_protocol_handler();
    // 登录兜底接管：装了 Microsoft Store 版（MSIX）或官方安装版的机器上，
    // `tradingview://` 回调会被它们抢走 —— AppX 声明的协议优先级高于 HKCU 注册，
    // 注册表改成什么都不管用（实测 2026-11-27：回调被拉起的是 WindowsApps 里的
    // 官方版，token 落进它自己的账号）。所以这里在"刚启动过账号"的窗口期内
    // 保一个监听：谁被系统拉起来，就从谁的命令行里把 token 捞回来。
    // 监听自身有单例锁、每轮 90 秒，由这里每 2 秒续期。
    if let Some(c) = ctx {
        if profiles::login_started_within(HANDOFF_WINDOW_SECS) {
            profiles::spawn_handoff_watcher(c.clone());
        }
    }
    let auth_note = open_auth_urls_if_new(ctx);
    let mut log = Vec::new();
    if let Some(note) = auth_note {
        log.push(note);
    }
    Scan {
        installs,
        managed,
        profiles,
        status,
        log,
    }
}

/// 给扫描线程发"立即扫一轮"的信号；线程尚未就绪时静默（下一节拍自然会扫）。
pub(crate) fn wake(shared: &Shared) {
    if let Some(tx) = &shared.kick {
        let _ = tx.send(());
    }
}

pub(crate) type Job = Box<dyn FnOnce(&mut dyn FnMut(String)) -> anyhow::Result<String> + Send>;

pub(crate) fn run_job(ctx: &egui::Context, shared: Arc<Mutex<Shared>>, clear: bool, job: Job) {
    {
        let mut s = shared.lock().unwrap();
        if clear {
            s.log.clear();
        }
        s.busy = true;
    }
    let ctx = ctx.clone();
    let sh = shared.clone();
    std::thread::spawn(move || {
        let push_sh = sh.clone();
        let mut push = move |m: String| {
            if let Ok(mut s) = push_sh.lock() {
                s.log.push(m);
            }
        };
        let res = job(&mut push);
        let mut s = sh.lock().unwrap();
        s.busy = false;
        wake(&s);
        match res {
            // 任务已经把细节自己 push 进日志时就返回空串：别再多出一行光秃秃的「●」
            Ok(m) if m.trim().is_empty() => {}
            Ok(m) => s.log.push(format!("● {m}")),
            Err(e) => s.log.push(format!("× {e:#}")),
        }
        drop(s);
        ctx.request_repaint();
    });
}

/// 后台盘点线程：每 SCAN_INTERVAL 扫一轮，收到 kick 提前唤醒就加扫一轮。
/// 进程枚举与注册表查询从此不再占用 UI 线程 —— 周期性卡顿的根治。
pub(crate) fn spawn_scan_thread(ctx: egui::Context, shared: Arc<Mutex<Shared>>) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    if let Ok(mut s) = shared.lock() {
        s.kick = Some(tx);
    }
    std::thread::spawn(move || loop {
        // 等 2 秒是常规节拍；被 kick 提前唤醒就立即扫
        let _ = rx.recv_timeout(SCAN_INTERVAL);
        let scan = scan_now(Some(&ctx));
        if let Ok(mut s) = shared.lock() {
            s.scan = Some(scan);
        }
        ctx.request_repaint();
    });
}

