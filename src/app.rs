//! 界面：账号卡片式管理。多账号是主角，环境配置退居顶部状态条。
//!
//! 所有操作先收集成一个 `Act`，等面板画完再统一执行，
//! 这样面板闭包里不需要可变借用整份 self，省掉一堆借用检查的坑。
//! 文案全部走 `i18n`（简中 / 繁中 / English），切换语言即时生效。

use crate::i18n::Lang;
use crate::install::{self, Config, Install, Managed};
use crate::patch;
use crate::profiles::{self, Profile};
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub fn open_path(p: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(p).spawn();
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}

/// 无头维护模式：`tvx _reapply` —— 按当前配置开关重打托管副本的补丁。
/// 与 GUI 里拨动任意开关触发的 `Act::ReapplyPatches` 走同一逻辑，
/// 供脚本化 / 远程维护使用，不启动界面。
pub fn reapply_headless() {
    let cfg = install::load_config();
    let lang = cfg.lang;
    let Some(m) = install::load_managed() else {
        println!("{}", lang.log_no_copy_headless());
        return;
    };
    match patch::apply(&m.index_js(), &cfg.flags()) {
        Ok(report) => {
            for (name, st) in report {
                let shown: &str = if lang == Lang::En {
                    patch::name_en(name)
                } else {
                    name
                };
                println!("[{}] {shown}", st.mark());
            }
            println!(
                "{}",
                lang.log_reapplied_headless(&m.index_js().display().to_string())
            );
        }
        Err(e) => eprintln!("✗ {e:#}"),
    }
}

/// 字体栈：Windows 上拉丁用 Segoe UI（系统原生 UI 字体，小字号锐利），中文回退雅黑；
/// 等宽区保持 Hack 主导、中文回退——不再把整条家族链替换成雅黑（那是"难看"的主因）。
/// macOS/Linux 直接用系统 CJK 字体（苹方/Noto 的拉丁本身清晰），行为与旧版一致。
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    const CJK: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhl.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\deng.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    ];
    let mut cjk = None;
    for p in CJK {
        if let Ok(bytes) = std::fs::read(p) {
            cjk = Some(egui::FontData::from_owned(bytes));
            break;
        }
    }
    if let Some(data) = cjk {
        fonts.font_data.insert("cjk".to_owned(), data);
        #[cfg(windows)]
        {
            let prop = fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default();
            match std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
                Ok(b) => {
                    fonts
                        .font_data
                        .insert("segoe".to_owned(), egui::FontData::from_owned(b));
                    prop.insert(0, "segoe".to_owned());
                    prop.insert(1, "cjk".to_owned());
                }
                Err(_) => prop.insert(0, "cjk".to_owned()),
            }
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("cjk".to_owned());
        }
        #[cfg(not(windows))]
        {
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "cjk".to_owned());
        }
        ctx.set_fonts(fonts);
    }
}

/// 统一的视觉基调：明确的深色配色 + 高对比控件 + 舒朗间距。
///
/// 为什么要自定义配色：egui 默认深色里"未激活"控件是 #2C2C2C 灰底压在
/// #1B1B1B 面板上，两者几乎同色、也没有描边——按钮和复选框看起来全是
/// 禁用态。这里给控件真正的底色 + 可见描边 + 蓝色强调：可用态有底有边，
/// 禁用态是空心淡字，两种状态一眼分得开。
pub fn setup_style(ctx: &egui::Context) {
    use egui::Color32;

    const PANEL: Color32 = Color32::from_rgb(0x1a, 0x1d, 0x22);
    const WIN: Color32 = Color32::from_rgb(0x14, 0x16, 0x1a);
    const CARD: Color32 = Color32::from_rgb(0x22, 0x26, 0x2d);
    const CHIP: Color32 = Color32::from_rgb(0x2b, 0x31, 0x3a);
    const CHIP_HOVER: Color32 = Color32::from_rgb(0x34, 0x3c, 0x48);
    const BORDER: Color32 = Color32::from_rgb(0x3f, 0x48, 0x55);
    const TEXT: Color32 = Color32::from_rgb(0xe6, 0xed, 0xf3);
    const ACCENT: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xf6);

    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = WIN;
    v.faint_bg_color = CARD; // 账号卡片底色
    v.extreme_bg_color = Color32::from_rgb(0x0e, 0x11, 0x14); // 输入框内部
    v.selection.bg_fill = Color32::from_rgb(0x2f, 0x6f, 0xeb); // 选中项 / 文本选区
    v.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    v.hyperlink_color = Color32::from_rgb(0x58, 0xa6, 0xff);

    // 非交互元素：卡片框线（Frame::group 的描边来源）、普通标签文字
    v.widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
    v.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(0x30, 0x37, 0x42));
    v.widgets.noninteractive.fg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(0xc5, 0xce, 0xd8));
    v.widgets.noninteractive.rounding = egui::Rounding::same(8.0); // 卡片圆角来源

    let w = &mut v.widgets;
    w.inactive.bg_fill = CHIP;
    w.inactive.weak_bg_fill = CHIP;
    w.inactive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    w.inactive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    w.inactive.rounding = egui::Rounding::same(8.0);

    w.hovered.bg_fill = CHIP_HOVER;
    w.hovered.weak_bg_fill = CHIP_HOVER;
    w.hovered.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    w.hovered.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    w.hovered.rounding = egui::Rounding::same(8.0);

    w.active.bg_fill = ACCENT; // 按下 / 勾选：实心蓝，不存在"像禁用"的可能
    w.active.weak_bg_fill = ACCENT;
    w.active.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(0x7a, 0xae, 0xff));
    w.active.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    w.active.rounding = egui::Rounding::same(8.0);

    w.open.bg_fill = CHIP_HOVER;
    w.open.weak_bg_fill = CHIP_HOVER;
    w.open.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    w.open.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    w.open.rounding = egui::Rounding::same(8.0);

    ctx.style_mut(|style| {
        style.visuals = v;
        style.visuals.menu_rounding = egui::Rounding::same(8.0);
        // 文本可选中，方便从日志里复制路径 / PID
        style.interaction.selectable_labels = true;
        // 字号阶梯：正文/按钮 16、小字 13、标题 20、等宽 14 —— Segoe UI 拉丁 +
        // 雅黑中文的组合在这个尺寸最耐看，小字号中文发糊的问题一并解决
        style.text_styles = [
            (egui::TextStyle::Body, egui::FontId::proportional(16.0)),
            (egui::TextStyle::Button, egui::FontId::proportional(16.0)),
            (egui::TextStyle::Heading, egui::FontId::proportional(20.0)),
            (egui::TextStyle::Small, egui::FontId::proportional(13.0)),
            (egui::TextStyle::Monospace, egui::FontId::monospace(14.0)),
        ]
        .into();
        // 间距：行距拉开（默认纵向 3px 太挤）、按钮与卡片内边距给足呼吸感
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.menu_margin = egui::Margin::same(10.0);
    });
}

const GREEN: egui::Color32 = egui::Color32::from_rgb(86, 190, 130);
const GREY: egui::Color32 = egui::Color32::from_rgb(122, 122, 122);
const AMBER: egui::Color32 = egui::Color32::from_rgb(214, 160, 74);
const AMBER_DIM: egui::Color32 = egui::Color32::from_rgb(58, 45, 28);
const RED: egui::Color32 = egui::Color32::from_rgb(224, 106, 90);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4c, 0x8d, 0xf6);

const SCAN_INTERVAL: Duration = Duration::from_secs(2);

/// 「全部启动」时相邻实例的间隔（秒）
const STAGGER_SECS: u64 = 3;

#[derive(Default)]
struct Shared {
    log: Vec<String>,
    busy: bool,
    /// 创建账号的结果（表单要等它：成功才关闭，失败就在表单里显示原因）
    add_done: Option<Result<(), String>>,
    /// 后台扫描线程的最新盘点结果，UI 线程每帧取走消费
    scan: Option<Scan>,
    /// 向扫描线程发"立即加扫一轮"信号（无界通道，send 永不阻塞）
    kick: Option<std::sync::mpsc::Sender<()>>,
}

/// 一轮后台盘点的全部产出，与 App 的对应字段一一对应。
struct Scan {
    installs: Vec<Install>,
    managed: Option<Managed>,
    profiles: Vec<Profile>,
    status: HashMap<String, Option<u32>>,
    /// 扫描期间产生的提示（如"已在无痕窗口打开登录页"）
    log: Vec<String>,
}

/// 已处理过的登录页 URL，**按档案各记一条**（`~/.tvx/auth-seen.json`）。
///
/// 不能用"全局只存一个 URL"来去重：多个账号的登录 URL 里 `key=` 必然不同，
/// 单值基准会被另一个账号立刻顶掉 → 每轮扫描都判定"发现新 URL" → 反复打开
/// 登录页并叠加后台接管进程。这是真实踩过的死循环（一直闪命令行窗口）。
fn auth_seen_path() -> std::path::PathBuf {
    install::tvx_root().join("auth-seen.json")
}

/// 从各档案的最新登录 URL 里挑出"这个档案还没处理过"的一条，并把它记进 `seen`。
///
/// 关键是**按档案分别记**：多个账号的 URL 里 `key=` 不同，如果只维护一个全局值，
/// 两个账号会互相把对方顶成"新 URL"，于是每轮扫描都触发一次打开登录页 +
/// 启动接管线程——就是那个"命令行窗口一直闪"的死循环。
fn pick_unseen_url(
    seen: &mut std::collections::BTreeMap<String, String>,
    items: impl Iterator<Item = (String, String)>,
) -> Option<(String, String)> {
    let mut newest = None;
    for (name, url) in items {
        if seen.get(&name).map(|s| s.as_str()) == Some(url.as_str()) {
            continue;
        }
        seen.insert(name.clone(), url.clone());
        newest = Some((name, url));
    }
    newest
}

/// 监听各账号日志里最新的 `TVX_AUTH_URL`，用**无痕窗口**打开（避开旧账号会话）。
/// 每个档案只对"自己没处理过"的 URL 触发一次。
fn open_auth_urls_if_new(ctx: Option<&egui::Context>) -> Option<String> {
    use std::collections::BTreeMap;

    let seen_file = auth_seen_path();
    let mut seen: BTreeMap<String, String> = std::fs::read_to_string(&seen_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let before = seen.len();
    let newest = pick_unseen_url(
        &mut seen,
        profiles::list().into_iter().filter_map(|p| {
            let text = std::fs::read_to_string(p.dir.join("tvx-launch.log")).ok()?;
            let line = text.lines().rev().find(|l| l.contains("TVX_AUTH_URL "))?;
            let url = line.split("TVX_AUTH_URL ").nth(1)?.trim().to_string();
            url.starts_with("http").then_some((p.name, url))
        }),
    );
    let changed = seen.len() != before;
    if changed {
        let _ = std::fs::create_dir_all(install::tvx_root());
        if let Ok(s) = serde_json::to_string_pretty(&seen) {
            let _ = std::fs::write(&seen_file, s);
        }
    }

    let (pname, url) = newest?;
    let url = url.as_str();

    // 先试"应用内标签页"：登录页在应用里渲染，cookie 直接落进该账号目录，
    // 不需要浏览器、也不需要 tradingview:// 回调（浏览器常把回调拦掉）。
    let opened_in_app = profiles::list()
        .into_iter()
        .find(|p| p.name == pname)
        .map(|p| profiles::open_in_app_tab(&p, url).is_ok())
        .unwrap_or(false);

    // 同时盯住被商店版抢走的回调（token 卡在那个进程的命令行里）
    if let Some(c) = ctx {
        profiles::spawn_handoff_watcher(c.clone());
    }
    Some(if opened_in_app {
        "已在应用内标签页打开登录页（不需要外部浏览器）".to_string()
    } else if crate::browser::open_private(url) {
        "已在无痕窗口打开登录页（不受你浏览器里已有账号影响）".to_string()
    } else {
        "已用默认浏览器打开登录页（未找到 Chrome/Edge）".to_string()
    })
}

fn scan_now(ctx: Option<&egui::Context>) -> Scan {
    let installs = install::discover();
    let managed = install::load_managed();
    let profiles = profiles::list();
    let status = profiles::status_snapshot(&profiles);
    // 应用自己会改写 tradingview:// 协议命令，这里每轮纠回来
    profiles::maintain_protocol_handler();
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
fn wake(shared: &Shared) {
    if let Some(tx) = &shared.kick {
        let _ = tx.send(());
    }
}

type Job = Box<dyn FnOnce(&mut dyn FnMut(String)) -> anyhow::Result<String> + Send>;

fn run_job(ctx: &egui::Context, shared: Arc<Mutex<Shared>>, clear: bool, job: Job) {
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
            Ok(m) => s.log.push(format!("✓ {m}")),
            Err(e) => s.log.push(format!("✗ {e:#}")),
        }
        drop(s);
        ctx.request_repaint();
    });
}

/// 后台盘点线程：每 SCAN_INTERVAL 扫一轮，收到 kick 提前唤醒就加扫一轮。
/// 进程枚举与注册表查询从此不再占用 UI 线程 —— 周期性卡顿的根治。
fn spawn_scan_thread(ctx: egui::Context, shared: Arc<Mutex<Shared>>) {
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

enum Act {
    Launch(String),
    Stop(u32),
    LaunchAll,
    StopAll,
    Delete(String),
    SubmitAdd,
    ReapplyPatches,
    /// 按指定的安装重建托管副本（客户端升级后副本还停在旧版本时用）
    RebuildCopy(usize),
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
            tray: None,
            tray_attempts: 0,
            quitting: false,
            launching: HashMap::new(),
            patch_checked: false,
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
        if self.selected >= self.installs.len() {
            self.selected = 0;
        }
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
                    s.log.push(format!("✗ {msg}"));
                    s.add_done = Some(Err(msg));
                    return;
                };
                let dest = install::dest_for(&inst);
                match install::build(&inst, &dest, &flags, lang, &mut push) {
                    Ok(m) => {
                        install::set_active_app(&m.version);
                        push(lang.log_built(&m.dir.display().to_string()))
                    }
                    Err(e) => {
                        let msg = lang.log_create_failed(&format!("{e:#}"));
                        let mut s = shared.lock().unwrap();
                        s.busy = false;
                        s.log.push(format!("✗ {msg}"));
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
                        "✓ {}",
                        lang.log_created(&p.name, &p.dir.display().to_string())
                    ));
                    s.add_done = Some(Ok(()));
                }
                Err(e) => {
                    let msg = lang.log_create_failed(&format!("{e:#}"));
                    s.log.push(format!("✗ {msg}"));
                    s.add_done = Some(Err(msg));
                }
            }
            drop(s);
        });

        // 表单先留着：成功才关，失败就把原因显示在表单里
        self.add_error = None;
    }

    fn apply(&mut self, ctx: &egui::Context, act: Act) {
        let lang = self.cfg.lang;
        match act {
            Act::Launch(name) => {
                let Some(m) = self.managed.clone() else {
                    self.log(lang.log_need_copy());
                    return;
                };
                if let Some(p) = self.profiles.iter().find(|p| p.name == name).cloned() {
                    match profiles::launch(&m, &p, lang) {
                        Ok(pid) => {
                            self.launching.insert(p.name.clone(), Instant::now());
                            self.log(lang.log_launched(&p.name, pid));
                            // 约 10 秒后（页面已把登录态落盘）回填账号：
                            // 本功能之前创建的档案没有绑定记录，只能这样认一次。
                            let p2 = p.clone();
                            let sh = self.shared.clone();
                            let c2 = ctx.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_secs(10));
                                if let Some((uid, dup)) = profiles::refresh_account(&p2) {
                                    if let Ok(mut st) = sh.lock() {
                                        st.log.push(lang.log_account_bound(&p2.name, &uid));
                                        if let Some(other) = dup {
                                            st.log.push(lang.log_account_same(
                                                &p2.name, &other, &uid,
                                            ));
                                        }
                                    }
                                    c2.request_repaint();
                                }
                            });
                        }
                        Err(e) => self.log(lang.log_launch_failed(&format!("{e:#}"))),
                    }
                }
                if let Some(addr) = profiles::proxy_unreachable() {
                    self.log(lang.log_proxy_down(&addr));
                }
                self.wake_scanner();
            }
            Act::Stop(pid) => {
                match profiles::stop(pid) {
                    Ok(()) => self.log(lang.log_stopped(pid)),
                    Err(e) => self.log(lang.log_stop_failed(&format!("{e:#}"))),
                }
                self.wake_scanner();
            }
            Act::LaunchAll => {
                let Some(m) = self.managed.clone() else {
                    return;
                };
                let todo: Vec<Profile> = self
                    .profiles
                    .iter()
                    .filter(|p| self.status.get(&p.name).copied().flatten().is_none())
                    .cloned()
                    .collect();
                if todo.is_empty() {
                    return;
                }
                // 错峰启动：逐个间隔 STAGGER_SECS 秒，避免多个实例在同一瞬间一起建连
                // （服务端看到的是同一账号/同一出口的建连风暴，更容易被判超限）。
                run_job(
                    ctx,
                    self.shared.clone(),
                    false,
                    Box::new(move |push| {
                        push(lang.log_stagger(todo.len(), STAGGER_SECS));
                        let mut started: Vec<Profile> = Vec::new();
                        for (i, p) in todo.iter().enumerate() {
                            if i > 0 {
                                std::thread::sleep(Duration::from_secs(STAGGER_SECS));
                            }
                            match profiles::launch(&m, p, lang) {
                                Ok(pid) => {
                                    started.push(p.clone());
                                    push(format!("… 启动「{}」 PID {pid}", p.name));
                                }
                                Err(e) => push(format!("✗ 「{}」{e:#}", p.name)),
                            }
                        }
                        let n = started.len();
                        if n > 0 {
                            // 等页面把登录态落盘后统一回填一次账号，认出同账号的档案
                            std::thread::sleep(Duration::from_secs(8));
                            for p in &started {
                                if let Some((uid, dup)) = profiles::refresh_account(p) {
                                    push(lang.log_account_bound(&p.name, &uid));
                                    if let Some(other) = dup {
                                        push(lang.log_account_same(&p.name, &other, &uid));
                                    }
                                }
                            }
                        }
                        Ok(lang.log_launched_all(n))
                    }),
                );
            }
            Act::StopAll => {
                let pids: Vec<u32> = self.status.values().flatten().copied().collect();
                for pid in &pids {
                    let _ = profiles::stop(*pid);
                }
                if !pids.is_empty() {
                    self.log(lang.log_stopped_all(pids.len()));
                }
                self.wake_scanner();
            }
            Act::Delete(name) => {
                if let Some(p) = self.profiles.iter().find(|p| p.name == name).cloned() {
                    match profiles::remove(&p, lang) {
                        Ok(()) => {
                            self.log(lang.log_deleted(&name));
                            // 立刻从界面移除，不等下一轮后台扫描
                            self.profiles.retain(|x| x.name != name);
                            self.status.remove(&name);
                        }
                        Err(e) => self.log(lang.log_delete_failed(&format!("{e:#}"))),
                    }
                }
                self.wake_scanner();
            }
            Act::ReapplyPatches => {
                let Some(m) = self.managed.clone() else {
                    return;
                };
                let flags = self.cfg.flags();
                run_job(
                    ctx,
                    self.shared.clone(),
                    false,
                    Box::new(move |push| {
                        let report = patch::apply(&m.index_js(), &flags)?;
                        push(lang.log_switches(
                            flags.ads_block,
                            flags.charts_unlock,
                            flags.study_on_study,
                            flags.silence_paywalls,
                            flags.no_auto_update,
                        ));
                        for (name, st) in report {
                            let shown: &str = if lang == Lang::En {
                                patch::name_en(name)
                            } else {
                                name
                            };
                            push(format!("  [{}] {shown}", st.mark()));
                        }
                        Ok(lang.log_reapplied())
                    }),
                );
            }
            Act::RebuildCopy(idx) => {
                let Some(inst) = self.installs.get(idx).cloned() else {
                    return;
                };
                let Some(old) = self.managed.as_ref().map(|m| m.version.clone()) else {
                    return;
                };
                let flags = self.cfg.flags();
                let dest = install::dest_for(&inst);
                run_job(
                    ctx,
                    self.shared.clone(),
                    true,
                    Box::new(move |push| {
                        push(lang.log_rebuild_start(&inst.version));
                        let m = install::build(&inst, &dest, &flags, lang, &mut *push)?;
                        // 锁定新版本：多版本目录共存时，后续启动都走这一份
                        install::set_active_app(&m.version);
                        Ok(lang.log_rebuilt(&m.version, &old))
                    }),
                );
            }
            Act::SubmitAdd => self.submit_add(),
        }
        ctx.request_repaint();
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
        if !self.patch_checked {
            self.patch_checked = true;
            if let Some(m) = self.managed.clone() {
                if !m.is_patched(&self.cfg.flags()) {
                    let flags = self.cfg.flags();
                    let lang = self.cfg.lang;
                    run_job(
                        ctx,
                        self.shared.clone(),
                        false,
                        Box::new(move |push| {
                            let report = patch::apply(&m.index_js(), &flags)?;
                            push(lang.log_patch_upgraded());
                            for (name, st) in report {
                                let shown: &str = if lang == Lang::En {
                                    patch::name_en(name)
                                } else {
                                    name
                                };
                                push(format!("  [{}] {shown}", st.mark()));
                            }
                            Ok(lang.log_reapplied())
                        }),
                    );
                }
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
                self.selected = 0;
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

        // 先把要画的数据摆成快照，避免面板闭包里反复借 self
        let install_labels: Vec<String> = self.installs.iter().map(|i| i.label(t)).collect();
        // 同账号查重：一个 TradingView 账号同时只应有一个档案在线（服务端按账号限连接）
        let mut owners: HashMap<String, Vec<String>> = HashMap::new();
        for p in &self.profiles {
            if let Some(uid) = profiles::account_id(p) {
                owners.entry(uid).or_default().push(p.name.clone());
            }
        }
        // (名字, PID, 目录, 绑定的账号, 与谁重复)
        let rows: Vec<(String, Option<u32>, PathBuf, Option<String>, Option<String>)> = self
            .profiles
            .iter()
            .map(|p| {
                let uid = profiles::account_id(p);
                let dup = uid.as_ref().and_then(|u| {
                    owners.get(u).filter(|v| v.len() > 1).map(|v| {
                        v.iter()
                            .filter(|n| *n != &p.name)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("、")
                    })
                });
                (
                    p.name.clone(),
                    self.status.get(&p.name).copied().flatten(),
                    p.dir.clone(),
                    uid,
                    dup,
                )
            })
            .collect();
        let online = rows.iter().filter(|r| r.1.is_some()).count();
        let has_profile = !rows.is_empty();
        let managed_ok = self.managed.is_some();
        let managed_ver = self.managed.as_ref().map(|m| m.version.clone());
        // 客户端升级后副本还停在旧版：这里只做提示，不自动重建（副本 200MB+，
        // 而且新旧版本可能同时在用，删谁必须由人决定）。
        let newer: Option<(usize, String)> = managed_ver.as_deref().and_then(|cur| {
            let hit = install::newest_than(&self.installs, cur)?;
            let idx = self.installs.iter().position(|i| i.root == hit.root)?;
            Some((idx, hit.version.clone()))
        });
        let patched = self
            .managed
            .as_ref()
            .map(|m| m.is_patched(&self.cfg.flags()))
            .unwrap_or(false);

        let mut act: Option<Act> = None;
        let mut lang_changed = false;

        // ------------------------------------------------------------ 顶部
        egui::TopBottomPanel::top("head").show(ctx, |ui| {
            ui.add_space(6.0);
            // 标题行：身份 + 语言切换
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t.app_title).heading().strong());
                ui.label(egui::RichText::new(t.subtitle).small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("lang")
                        .selected_text(self.cfg.lang.label())
                        .width(112.0)
                        .show_ui(ui, |ui| {
                            for l in Lang::ALL {
                                if ui
                                    .selectable_value(&mut self.cfg.lang, l, l.label())
                                    .changed()
                                {
                                    lang_changed = true;
                                }
                            }
                        });
                });
            });

            ui.add_space(4.0);
            // 状态行：安装在哪儿 + 副本状态（版本 / 补丁 / 校验合成一个徽标），
            // 右侧只留一个主操作；打开目录、删除副本属于低频维护，收进折叠区
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t.installed_at).small().weak());
                if install_labels.is_empty() {
                    ui.colored_label(AMBER, t.no_tv);
                } else {
                    for (i, label) in install_labels.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, i, label);
                    }
                }
                ui.separator();

                match &managed_ver {
                    None => {
                        ui.colored_label(AMBER, egui::RichText::new(t.copy_none).small())
                            .on_hover_text(t.copy_none_hint);
                    }
                    Some(v) if !patched => {
                        ui.colored_label(
                            AMBER,
                            egui::RichText::new(t.copy_bad.replace("{v}", v)).small(),
                        )
                        .on_hover_text(t.copy_bad_hint);
                    }
                    Some(v) if v == patch::VERIFIED_VERSION => {
                        ui.colored_label(
                            GREEN,
                            egui::RichText::new(t.copy_ok.replace("{v}", v)).small(),
                        )
                        .on_hover_text(t.copy_ok_hint);
                    }
                    Some(v) => {
                        ui.colored_label(
                            AMBER,
                            egui::RichText::new(
                                t.copy_unverified
                                    .replace("{v}", v)
                                    .replace("{refv}", patch::VERIFIED_VERSION),
                            )
                            .small(),
                        )
                        .on_hover_text(t.copy_unverified_hint);
                    }
                }

                if let Some((idx, v)) = &newer {
                    ui.separator();
                    ui.colored_label(
                        GREEN,
                        egui::RichText::new(t.copy_newer.replace("{v}", v)).small(),
                    )
                    .on_hover_text(t.copy_newer_hint);
                    if ui.small_button(t.btn_rebuild).clicked() {
                        act = Some(Act::RebuildCopy(*idx));
                    }
                }
            });

            // 未检测到 TradingView 时，给一个醒目的安装提醒（而非让用户对着空列表猜）
            if install_labels.is_empty() {
                ui.add_space(6.0);
                egui::Frame::none()
                    .fill(AMBER_DIM)
                    .rounding(6.0)
                    .inner_margin(egui::vec2(12.0, 10.0))
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(t.no_tv_alert).color(AMBER).strong());
                            if ui.button(t.rescan).clicked() {
                                self.installs = install::discover();
                                if self.selected >= self.installs.len() {
                                    self.selected = 0;
                                }
                                self.wake_scanner();
                            }
                        });
                    });
            }

            // ---------------------------------------------------------- 功能开关
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t.features).small().weak());
                let mut toggled = false;
                if ui
                    .checkbox(&mut self.cfg.ads_block, t.cb_ads)
                    .on_hover_text(t.cb_ads_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.charts_unlock, t.cb_charts)
                    .on_hover_text(t.cb_charts_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.study_on_study, t.cb_study)
                    .on_hover_text(t.cb_study_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.silence_paywalls, t.cb_silence)
                    .on_hover_text(t.cb_silence_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.auto_recover, t.cb_recover)
                    .on_hover_text(t.cb_recover_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.no_auto_update, t.cb_noupdate)
                    .on_hover_text(t.cb_noupdate_hint)
                    .changed()
                {
                    toggled = true;
                }
                if toggled {
                    install::save_config(&self.cfg.clone());
                    act = Some(Act::ReapplyPatches);
                }
            });

            // ---------------------------------------------------------- 使用说明
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t.usage).small().weak());
                for (i, step) in [t.add, t.step_launch, t.step_login].iter().enumerate() {
                    if i > 0 {
                        ui.label(egui::RichText::new("→").small().color(ACCENT));
                    }
                    ui.label(egui::RichText::new(*step).small());
                }
            });
            // 一条必须写进界面的约束：同账号多开会撞服务端的连接限制
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t.usage_note).small().color(AMBER));
            });

            ui.add_space(6.0);
        });

        // ------------------------------------------------------------ 日志
        egui::TopBottomPanel::bottom("log")
            .resizable(true)
            .default_height(100.0)
            .min_height(48.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(t.log).small().weak());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(t.clear).clicked() {
                            if let Ok(mut s) = self.shared.lock() {
                                s.log.clear();
                            }
                        }
                    });
                });
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if log.is_empty() {
                            ui.label(egui::RichText::new("—").small().weak());
                        }
                        for line in &log {
                            let mut rt = egui::RichText::new(line).monospace().small();
                            match line.chars().next() {
                                Some('✓') => rt = rt.color(GREEN),
                                Some('✗') => rt = rt.color(RED),
                                Some('⚠') => rt = rt.color(AMBER),
                                _ => {}
                            }
                            ui.label(rt);
                        }
                    });
            });

        // ------------------------------------------------------------ 账号
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t.accounts).strong());
                ui.label(
                    egui::RichText::new(
                        t.count
                            .replace("{n}", &rows.len().to_string())
                            .replace("{o}", &online.to_string()),
                    )
                    .small()
                    .weak(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // right_to_left：先加的在最右边，所以主操作「添加账号」放最后一个
                    if ui.add_enabled(!busy, egui::Button::new(t.add)).clicked() {
                        self.adding = true;
                        self.focus_add = true;
                        self.add_error = None;
                        self.new_name.clear();
                    }
                    if ui
                        .add_enabled(online > 0, egui::Button::new(t.stop_all))
                        .clicked()
                    {
                        act = Some(Act::StopAll);
                    }
                    if ui
                        .add_enabled(
                            !busy && has_profile && managed_ok,
                            egui::Button::new(t.launch_all),
                        )
                        .on_disabled_hover_text(t.launch_all_hint)
                        .clicked()
                    {
                        act = Some(Act::LaunchAll);
                    }
                });
            });

            ui.add_space(6.0);

            if self.adding {
                egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::same(10.0))
                    .fill(ui.style().visuals.faint_bg_color)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(t.name_label);
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.new_name)
                                    .hint_text(t.name_hint)
                                    .desired_width(180.0),
                            );
                            if self.focus_add {
                                resp.request_focus();
                                self.focus_add = false;
                            }
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                act = Some(Act::SubmitAdd);
                            }
                            let can_submit = !busy && !self.new_name.trim().is_empty();
                            if ui
                                .add_enabled(can_submit, egui::Button::new(t.submit))
                                .on_disabled_hover_text(t.name_hint)
                                .clicked()
                            {
                                act = Some(Act::SubmitAdd);
                            }
                            if busy {
                                ui.label(egui::RichText::new(t.creating).small().color(AMBER));
                            }
                            if ui.button(t.cancel).clicked() {
                                self.adding = false;
                                self.add_error = None;
                            }
                        });
                        if !managed_ok {
                            ui.colored_label(AMBER, egui::RichText::new(t.auto_build).small());
                        }
                        if let Some(e) = &self.add_error {
                            ui.colored_label(RED, egui::RichText::new(e).small());
                        }
                    });
                ui.add_space(6.0);
            }

            if rows.is_empty() {
                ui.add_space(12.0);
                ui.label(egui::RichText::new(t.empty_title).strong());
                ui.add_space(2.0);
                ui.label(egui::RichText::new(t.empty_desc).weak());
                ui.add_space(14.0);
                for (i, step) in [t.empty_s1, t.empty_s2, t.empty_s3].iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{}.", i + 1)).small().weak());
                        ui.label(egui::RichText::new(*step).small());
                    });
                }
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (name, pid, dir, uid, dup) in &rows {
                        egui::Frame::group(ui.style())
                            .inner_margin(egui::Margin::same(12.0))
                            .fill(ui.style().visuals.faint_bg_color)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let starting = self
                                        .launching
                                        .get(name)
                                        .map(|at| at.elapsed().as_secs());
                                    let (dot, txt) = match pid {
                                        Some(id) => (GREEN, t.online.replace("{pid}", &id.to_string())),
                                        None => match starting {
                                            Some(s) => {
                                                (AMBER, t.launching.replace("{s}", &s.to_string()))
                                            }
                                            None => (GREY, t.offline.to_string()),
                                        },
                                    };
                                    ui.colored_label(dot, "●");
                                    ui.label(egui::RichText::new(name).strong().size(17.0));
                                    ui.label(egui::RichText::new(txt).small().color(dot));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            // 删除确认内联在卡片上：原先那条「确认删除」栏
                                            // 被放在占满剩余高度的滚动区之后，实际永远看不到
                                            if self.pending_delete.as_deref() == Some(name.as_str()) {
                                                if ui.button(t.cancel).clicked() {
                                                    self.pending_delete = None;
                                                }
                                                if ui
                                                    .button(
                                                        egui::RichText::new(t.confirm_delete)
                                                            .color(RED),
                                                    )
                                                    .clicked()
                                                {
                                                    self.pending_delete = None;
                                                    act = Some(Act::Delete(name.clone()));
                                                }
                                                ui.label(
                                                    egui::RichText::new(t.del_confirm_short)
                                                        .small()
                                                        .color(RED),
                                                );
                                            } else if ui.button(t.delete).clicked() {
                                                self.pending_delete = Some(name.clone());
                                            }
                                            if ui.button(t.dir).clicked() {
                                                open_path(dir);
                                            }
                                            match pid {
                                                Some(_) => {
                                                    if ui.button(t.stop).clicked() {
                                                        act = Some(Act::Stop(pid.unwrap()));
                                                    }
                                                }
                                                None => {
                                                    if ui
                                                        .add_enabled(
                                                            managed_ok && !busy && starting.is_none(),
                                                            egui::Button::new(t.launch),
                                                        )
                                                        .on_disabled_hover_text(t.launch_hint)
                                                        .clicked()
                                                    {
                                                        act = Some(Act::Launch(name.clone()));
                                                    }
                                                }
                                            }
                                        },
                                    );
                                });
                                // 账号信息单独一行：同一行已有「名字 + 状态 + 4 个按钮」，
                                // 再塞账号 ID 在窄窗口会把按钮挤掉。
                                if uid.is_some() || dup.is_some() {
                                    ui.horizontal(|ui| {
                                        ui.add_space(18.0);
                                        if let Some(u) = uid {
                                            ui.label(
                                                egui::RichText::new(t.acct_bound.replace("{uid}", u))
                                                    .small()
                                                    .weak(),
                                            )
                                            .on_hover_text(t.usage_note);
                                        }
                                        if let Some(d) = dup {
                                            ui.colored_label(
                                                RED,
                                                egui::RichText::new(
                                                    t.acct_dup.replace("{name}", d),
                                                )
                                                .small(),
                                            )
                                            .on_hover_text(t.usage_note);
                                        }
                                    });
                                }
                            });
                        ui.add_space(4.0);
                    }
                });
        });

        if let Some(a) = act {
            self.apply(ctx, a);
        }
        if lang_changed {
            install::save_config(&self.cfg.clone());
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                self.cfg.lang.s().app_title.to_string(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// 回归：两个账号各有一条登录 URL 时，**不能每轮都判定"发现新 URL"**。
    /// 旧实现只维护一个全局 last，两个账号会互相顶掉 → 每 2 秒触发一次，
    /// 反复打开登录页并叠加接管线程（用户看到命令行窗口一直在闪）。
    #[test]
    fn two_accounts_do_not_retrigger_every_scan() {
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        let a = "https://auth.tradingview.com/connect/?key=AAA".to_string();
        let b = "https://auth.tradingview.com/connect/?key=BBB".to_string();

        let items = || {
            vec![
                ("colonychain".to_string(), a.clone()),
                ("dog000".to_string(), b.clone()),
            ]
            .into_iter()
        };

        // 第一轮：确实有新 URL，触发一次
        assert!(pick_unseen_url(&mut seen, items()).is_some());
        assert_eq!(seen.len(), 2, "两个档案都要被记住，否则会互相顶掉");

        // 第二轮、第三轮：同样的 URL，不能再触发
        assert_eq!(pick_unseen_url(&mut seen, items()), None);
        assert_eq!(pick_unseen_url(&mut seen, items()), None);

        // 某个账号重新登录（URL 变化）→ 只触发那一条
        let c = "https://auth.tradingview.com/connect/?key=CCC".to_string();
        let got = pick_unseen_url(
            &mut seen,
            vec![
                ("colonychain".to_string(), a.clone()),
                ("dog000".to_string(), c.clone()),
            ]
            .into_iter(),
        );
        assert_eq!(got, Some(("dog000".to_string(), c)));
        // 再变回旧 URL 仍算"这个档案又变了一次"（只看自己的上一条，日志只追加，
        // 真实场景不会来回跳）——重点是 colonychain 那条不会被顺带重触发。
        let back = pick_unseen_url(&mut seen, items());
        assert_eq!(back.as_ref().map(|(n, _)| n.as_str()), Some("dog000"));
    }
}
