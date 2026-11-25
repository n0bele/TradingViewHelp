//! 界面：账号卡片式管理。多账号是主角，环境配置退居顶部状态条。
//!
//! 所有操作先收集成一个 `Act`，等面板画完再统一执行，
//! 这样面板闭包里不需要可变借用整份 self，省掉一堆借用检查的坑。
//! 文案全部走 `i18n`（中文 / English），切换语言即时生效。

use crate::i18n::Lang;
use crate::install::{self, Config, Install, Managed};
use crate::patch;
use crate::profiles::{self, Profile};
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod actions;
mod authwatch;
mod headless;
mod state;
mod theme;
mod view;

#[cfg(test)]
mod tests;

pub use headless::{build_headless, open_path, reapply_headless};
pub use theme::{setup_fonts, setup_style};
use theme::{ACCENT, AMBER, AMBER_DIM, GREEN, GREY, RED};

use crate::app::state::{run_job, spawn_scan_thread, wake, Shared};

/// 账号卡片上的内联改名输入：`source` 是原账号名
struct NameEdit {
    source: String,
    text: String,
}

pub(crate) enum Act {
    Launch(String),
    Stop(u32),
    LaunchAll,
    StopAll,
    Delete(String),
    Rename(String, String),
    SubmitAdd,
    ReapplyPatches,
    /// 按指定的安装重建托管副本（客户端升级后副本还停在旧版本时用）
    RebuildCopy(usize),
    /// 重新跑一遍环境体检，并把完整报告写进日志（`_doctor` 的界面入口）
    Doctor,
}

/// 构建门禁的判定：本次构建的源版本是否不在已验证白名单里。
///
/// 只回答"源有没有验证过"，不掺用户是否已经确认——确认与否只决定按钮能不能点。
/// 早先把两者混在一起，导致勾了确认后警告条整条消失，用户既看不到自己勾过什么，
/// 也没法反悔（只能切换来源才能重置）。
fn unverified_target(
    managed_ok: bool,
    sel_ver: Option<&str>,
    newer: Option<(&str, bool)>,
) -> Option<String> {
    let target = if managed_ok {
        // newer = (版本号, 是否已验证)；已验证的新版不需要确认
        newer.filter(|(_, verified)| !*verified).map(|(v, _)| v)
    } else {
        sel_ver
    };
    target
        .filter(|v| !patch::is_verified(v))
        .map(|v| v.to_string())
}

/// 能否真的执行构建：源已验证，或用户已经勾过风险确认。
fn build_allowed(target: Option<&str>, allow_unverified: bool) -> bool {
    target.is_none() || allow_unverified
}

/// 版本号在界面上的叫法：`custom` / `portable` / `app` 是内部占位值，别直接甩给用户看。
fn source_name(v: &str, lang: Lang) -> String {
    let t = lang.s();
    match v {
        "custom" => t.kind_custom.to_string(),
        "portable" => t.kind_portable.to_string(),
        "app" => "App".to_string(),
        other => other.to_string(),
    }
}

pub struct App {
    shared: Arc<Mutex<Shared>>,
    installs: Vec<Install>,
    selected: usize,
    managed: Option<Managed>,
    profiles: Vec<Profile>,
    status: HashMap<String, Option<u32>>,
    /// 后台扫描线程只启动一次
    scan_started: bool,

    /// 功能开关、语言与来源路径的持久化配置（~/.tvx/config.json）
    cfg: Config,
    adding: bool,
    /// 打开表单时把焦点丢进名字输入框（只请求一次）
    focus_add: bool,
    new_name: String,
    add_error: Option<String>,

    pending_delete: Option<String>,
    /// 用户已明确确认"用未验证版本构建副本"（风险确认，切换来源即失效）
    allow_unverified: bool,
    /// 改名输入框：打开时把焦点丢进去并全选旧名（只请求一次）
    focus_rename: bool,
    /// 改名的内联输入状态
    name_edit: Option<NameEdit>,

    /// 托盘图标（Windows/macOS 才有；Linux 为 None）
    tray: Option<crate::tray::Tray>,
    /// 托盘尝试次数（最多 5 次，失败会重试；结果写日志与状态文件）
    tray_attempts: u32,
    /// 托盘「退出」时才置真——否则关闭窗口只是收进托盘
    quitting: bool,
    /// 已点「启动」但还没被扫描到存活的账号（显示"启动中"）
    launching: HashMap<String, Instant>,
    /// 启动时只检查一次"托管副本的补丁是否需要升级"
    patch_checked: bool,
    /// 启动前的环境体检结果（空 = 正常）。非空时顶栏常驻红字提示，
    /// 并会阻止启动——见 `install::environment_problems`。
    env_warn: Vec<crate::install::EnvProblem>,
}

impl App {
    pub fn new() -> Self {
        let cfg = install::load_config();
        let mut me = Self {
            shared: Arc::new(Mutex::new(Shared::default())),
            installs: Vec::new(),
            selected: 0,
            managed: None,
            profiles: Vec::new(),
            status: HashMap::new(),
            scan_started: false,
            cfg,
            adding: false,
            focus_add: false,
            new_name: String::new(),
            add_error: None,
            pending_delete: None,
            allow_unverified: false,
            focus_rename: false,
            name_edit: None,
            tray: None,
            tray_attempts: 0,
            quitting: false,
            launching: HashMap::new(),
            patch_checked: false,
            env_warn: crate::install::environment_problems(),
        };
        me.reload();
        if me.profiles.is_empty() {
            me.adding = true;
            me.focus_add = true;
        }
        me
    }

    fn reload(&mut self) {
        self.installs = install::discover();
        // 默认落在已验证版本上：客户端升级后旧目录常还在，"最新"恰恰是没验证过的那份
        self.selected = install::preferred_index(&self.installs).unwrap_or(0);
        self.managed = install::load_managed();
        self.profiles = profiles::list();
        self.status = profiles::status_snapshot(&self.profiles);
    }

    /// 立即触发一轮后台盘点（启动 / 停止 / 删除之后，状态点马上跟进）
    fn wake_scanner(&self) {
        if let Ok(s) = self.shared.lock() {
            wake(&s);
        }
    }

    fn log(&self, msg: impl Into<String>) {
        if let Ok(mut s) = self.shared.lock() {
            s.log.push(msg.into());
        }
    }

    fn submit_add(&mut self) {
        let lang = self.cfg.lang;
        let name = self.new_name.trim().to_string();
        if let Err(e) = profiles::validate_name(&name, lang) {
            self.add_error = Some(format!("{e}"));
            return;
        }
        if self
            .profiles
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(&name))
        {
            self.add_error = Some(lang.p_exists(&name));
            return;
        }

        let shared = self.shared.clone();
        let need_copy = self.managed.is_none();
        let inst = self.installs.get(self.selected).cloned();
        // 兜底：按钮被回车绕过时也拦住未验证来源（界面上的确认是唯一放行条件）
        if need_copy {
            if let Some(i) = &inst {
                if !patch::is_verified(&i.version) && !self.allow_unverified {
                    self.add_error =
                        Some(lang.e_unverified_blocked(&source_name(&i.version, lang)));
                    return;
                }
            }
        }
        let flags = self.cfg.flags();
        {
            let mut s = shared.lock().unwrap();
            s.busy = true;
            s.add_done = None;
        }

        std::thread::spawn(move || {
            let mut push = {
                let sh = shared.clone();
                move |m: String| {
                    if let Ok(mut s) = sh.lock() {
                        s.log.push(m);
                    }
                }
            };

            // 还没有副本就先自动生成："添加账号"本身就意味着"备好一个能跑的实例"
            if need_copy {
                let Some(inst) = inst else {
                    let msg = lang.log_no_source();
                    let mut s = shared.lock().unwrap();
                    s.busy = false;
                    s.log.push(format!("× {msg}"));
                    s.add_done = Some(Err(msg));
                    return;
                };
                let dest = install::dest_for(&inst);
                match install::build(&inst, &dest, &flags, lang, &mut push) {
                    Ok(m) => {
                        let _ = install::set_active_app(&m.version);
                        push(lang.log_built(&m.dir.display().to_string()))
                    }
                    Err(e) => {
                        let msg = lang.log_create_failed(&format!("{e:#}"));
                        let mut s = shared.lock().unwrap();
                        s.busy = false;
                        s.log.push(format!("× {msg}"));
                        s.add_done = Some(Err(msg));
                        return;
                    }
                }
            }

            let res = profiles::create(&name, lang);
            let mut s = shared.lock().unwrap();
            s.busy = false;
            wake(&s);
            match res {
                Ok(p) => {
                    s.log.push(format!(
                        "● {}",
                        lang.log_created(&p.name, &p.dir.display().to_string())
                    ));
                    s.add_done = Some(Ok(()));
                }
                Err(e) => {
                    let msg = lang.log_create_failed(&format!("{e:#}"));
                    s.log.push(format!("× {msg}"));
                    s.add_done = Some(Err(msg));
                }
            }
            drop(s);
        });

        // 表单先留着：成功才关，失败就把原因显示在表单里
        self.add_error = None;
    }

}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 首帧：启动后台盘点线程，进程枚举 / 注册表查询不占 UI 线程
        if !self.scan_started {
            self.scan_started = true;
            spawn_scan_thread(ctx.clone(), self.shared.clone());
        }
        // 首帧：托管副本里的补丁如果不是当前版本（例如还带着旧版"只隐藏弹窗"的注入脚本），
        // 自动升级一次——否则新版锚点对不上，界面会一直显示"补丁异常"。
        // 旧包残留（app.asar）同样走这条：补丁齐了也没用，它会让补丁一条都不执行。
        if !self.patch_checked {
            self.patch_checked = true;
            if let Some(m) = self.managed.clone() {
                if !m.is_patched(&self.cfg.flags()) || m.asar_residue() {
                    let flags = self.cfg.flags();
                    let lang = self.cfg.lang;
                    run_job(
                        ctx,
                        self.shared.clone(),
                        false,
                        Box::new(move |push| {
                            match m.clear_asar_residue() {
                                Ok(true) => push(lang.log_stale_cleared(
                                    &m.resources().join("app.asar").display().to_string(),
                                )),
                                Ok(false) => {}
                                Err(_) => push(lang.log_stale_locked(
                                    &m.resources().join("app.asar").display().to_string(),
                                )),
                            }
                            let report = patch::apply(&m.index_js(), &flags)?;
                            push(lang.log_patch_upgraded());
                            for &(name, st) in &report.items {
                                let shown: &str = if lang == Lang::En {
                                    patch::name_en(name)
                                } else {
                                    name
                                };
                                push(format!("  [{}] {shown}", st.mark()));
                            }
                            if !report.failed.is_empty() {
                                push(lang.log_patch_partial(&report.failed));
                            }
                            Ok(lang.log_reapplied())
                        }),
                    );
                } else {
                    // 一切正常也要留一条：日志空白会让用户以为"检查没跑、功能没生效"，
                    // 而不是"检查过了，没问题"。
                    let flags = self.cfg.flags();
                    let ok = patch::PATCHES.len()
                        - patch::failing_patches(&m.index_js(), &flags).len();
                    self.log(self.cfg.lang.log_health_ok(
                        env!("CARGO_PKG_VERSION"),
                        &m.version,
                        ok,
                        patch::PATCHES.len(),
                    ));
                }
            } else {
                // 没有副本时同样是"空日志"陷阱：给一句状态 + 下一步，别让用户对着空白猜
                let t = self.cfg.lang.s();
                self.log(format!("{} —— {}", t.copy_none, t.copy_none_hint));
            }
        }
        let (log, busy, scan, add_done) = {
            let mut s = self.shared.lock().unwrap();
            let scan = s.scan.take();
            let done = s.add_done.take();
            (s.log.clone(), s.busy, scan, done)
        };
        if let Some(r) = add_done {
            match r {
                Ok(()) => {
                    self.adding = false;
                    self.add_error = None;
                    self.new_name.clear();
                }
                Err(e) => self.add_error = Some(e),
            }
        }
        if let Some(sc) = scan {
            self.installs = sc.installs;
            if self.selected >= self.installs.len() {
                self.selected = install::preferred_index(&self.installs).unwrap_or(0);
            }
            self.managed = sc.managed;
            self.profiles = sc.profiles;
            self.status = sc.status;
            for line in sc.log {
                self.log(line);
            }
        }
        if busy {
            // 任务进行中：日志流式追加，低频重绘即可见
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        // 当前语言文案表（'static，闭包里用不会和 &mut self 打架）
        let t = self.cfg.lang.s();

        // 托盘：首帧创建；关闭窗口 = 收进托盘，托盘菜单「退出」才是真退出
        if self.tray.is_none()
            && self.tray_attempts < 5
            && crate::tray::supported()
            && std::env::var("TVX_NO_TRAY").is_err()
        {
            self.tray_attempts += 1;
            self.tray =
                crate::tray::Tray::new(ctx.clone(), t.tray_show, t.tray_quit, t.app_title);
            // 只在彻底失败时提示（没有托盘＝关窗即退出，用户需要知道）；成功不刷日志
            if self.tray.is_none() && self.tray_attempts >= 5 {
                self.log(t.tray_fail);
            }
        }
        if let Some(ev) = self.tray.as_ref().and_then(|tr| tr.poll()) {
            match ev {
                crate::tray::TrayEvent::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                crate::tray::TrayEvent::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        // 只有真的建出了托盘，关闭才是"收进托盘"；否则（如 Linux）保持关闭即退出
        if self.tray.is_some()
            && !self.quitting
            && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        // 收在托盘里也要能收到菜单点击：保持低频重绘
        ctx.request_repaint_after(Duration::from_millis(400));

        // 「启动中」状态：进程被扫到存活、或超过 30 秒就结束
        self.launching.retain(|name, at| {
            let up = self.status.get(name).copied().flatten().is_some();
            !up && at.elapsed() < Duration::from_secs(30)
        });

        // 快照定稿后，三个面板各画各的；它们只能通过 `act` 反馈意图
        let view = self.view();
        let mut act: Option<Act> = None;
        let mut lang_changed = false;
        self.head_panel(ctx, &view, &mut act, &mut lang_changed);
        self.log_panel(ctx, &log);
        self.accounts_panel(ctx, &view, busy, &mut act);

        if let Some(a) = act {
            self.apply(ctx, a);
        }
        if lang_changed {
            if let Err(e) = install::save_config(&self.cfg) {
                install::log_error("保存界面语言 config.json", &e);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                self.cfg.lang.s().app_title.to_string(),
            ));
        }
    }
}

