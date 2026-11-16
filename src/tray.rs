//! 最小化到托盘：点窗口关闭 = 隐藏到托盘；托盘菜单「退出」才是真退出。
//!
//! 两个平台坑，这里都是踩过才这么写的：
//! 1) Windows 的托盘图标必须在**自己的消息循环**里创建。直接在 egui 的 update()
//!    里建会抢走 winit 的消息泵，表现为窗口变白不再绘制 → 所以开专属线程 + GetMessageW。
//! 2) 菜单点击事件**不能靠 UI 线程轮询**：窗口一隐藏，eframe 就不再每帧调用 update()，
//!    轮询永远看不到事件。改用 `MenuEvent::set_event_handler` 在托盘线程里直接回调，
//!    回调里 `ctx.request_repaint()` 把 UI 叫醒，事件经 channel 交给 update() 处理。
//!
//! Linux：托盘要 libappindicator 且各桌面行为不一，不启用——关闭即退出（编译期桩）。

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::mpsc::{channel, Receiver};
    use std::time::Duration;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    #[derive(Clone, Copy, Debug)]
    pub enum TrayEvent {
        Show,
        Quit,
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Msg {
        hwnd: *mut c_void,
        message: u32,
        w_param: usize,
        l_param: isize,
        time: u32,
        pt: Point,
        l_private: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
        fn EnumWindows(cb: extern "system" fn(*mut c_void, isize) -> i32, param: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
        fn ShowWindow(hwnd: *mut c_void, cmd: i32) -> i32;
        fn SetForegroundWindow(hwnd: *mut c_void) -> i32;
        fn GetWindowTextW(hwnd: *mut c_void, buf: *mut u16, max: i32) -> i32;
    }

    const SW_RESTORE: i32 = 9;
    const SW_SHOW: i32 = 5;

    /// param = 目标进程号；param = 0 表示"按标题找主窗口"（给 `tvhelper _show` 用）
    extern "system" fn show_cb(hwnd: *mut c_void, param: isize) -> i32 {
        let target = param as u32;
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), 256);
            if n <= 0 {
                return 1; // 无标题，跳过
            }
            let hit = if target != 0 {
                pid == target
            } else {
                let title = String::from_utf16_lossy(&buf[..n as usize]);
                // 窗口标题即 app_title（中/英），兼容旧版 "· tvx" 后缀
                title.ends_with("TradingView 助手")
                    || title.ends_with("TradingView Helper")
                    || title.ends_with("· tvx")
            };
            if hit {
                ShowWindow(hwnd, SW_RESTORE);
                ShowWindow(hwnd, SW_SHOW);
                SetForegroundWindow(hwnd);
                return 0; // 找到即停
            }
        }
        1 // 继续枚举
    }

    /// 把本进程的主窗口直接显示出来——绕开 egui 的视口命令队列，
    /// 窗口隐藏后即使 UI 线程不刷新也能弹出来。
    pub fn show_main_window() {
        unsafe {
            EnumWindows(show_cb, std::process::id() as isize);
        }
    }

    /// 按窗口标题找任意一个主窗口并显示（`tvhelper _show` 用）。
    pub fn show_any_tvx_window() {
        unsafe {
            EnumWindows(show_cb, 0);
        }
    }

    /// 32×32 图标：深色圆角底 + 三根 K 线柱（纯像素画，不依赖字体）
    fn make_icon() -> Icon {
        const S: u32 = 32;
        let mut px = vec![0u8; (S * S * 4) as usize];
        let bg = [0x1a, 0x1d, 0x22, 255u8];
        let accent = [0x4c, 0x8d, 0xf6, 255u8];
        let accent2 = [0x7a, 0xae, 0xff, 255u8];

        for y in 0..S {
            for x in 0..S {
                let fx = x as f32 + 0.5;
                let fy = y as f32 + 0.5;
                let r = 6.0;
                let cx = fx.clamp(r, (S as f32) - r);
                let cy = fy.clamp(r, (S as f32) - r);
                if ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt() <= r {
                    let i = ((y * S + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&bg);
                }
            }
        }
        for (x0, x1, top, bottom, col) in [
            (7u32, 10u32, 18u32, 26u32, accent),
            (14, 17, 9, 26, accent2),
            (21, 24, 13, 22, accent),
        ] {
            for y in top..bottom {
                for x in x0..x1 {
                    let i = ((y * S + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&col);
                }
            }
        }
        Icon::from_rgba(px, S, S).expect("图标数据非法")
    }

    pub struct Tray {
        events: Receiver<TrayEvent>,
        _thread: std::thread::JoinHandle<()>,
    }

    /// 本平台是否支持托盘
    pub fn supported() -> bool {
        true
    }

    impl Tray {
        /// 在专属线程里建图标、装菜单回调并跑消息循环；失败返回 None（退回"关闭即退出"）。
        pub fn new(
            ctx: eframe::egui::Context,
            show_label: &str,
            quit_label: &str,
            tooltip: &str,
        ) -> Option<Self> {
            let show_label = show_label.to_string();
            let quit_label = quit_label.to_string();
            let tooltip = tooltip.to_string();
            let (ev_tx, ev_rx) = channel::<TrayEvent>();
            let (ready_tx, ready_rx) = channel::<bool>();

            let thread = std::thread::spawn(move || {
                let menu = Menu::new();
                // 显式 ID：避免自动 id 冲突，也方便日志定位是哪一项被触发
                let show = MenuItem::with_id("tvx-show", show_label, true, None);
                let quit = MenuItem::with_id("tvx-quit", quit_label, true, None);
                if menu.append(&show).is_err() || menu.append(&quit).is_err() {
                    let _ = ready_tx.send(false);
                    return;
                }
                let Ok(_tray) = TrayIconBuilder::new()
                    .with_menu(Box::new(menu))
                    .with_tooltip(tooltip)
                    .with_icon(make_icon())
                    .build()
                else {
                    let _ = ready_tx.send(false);
                    return;
                };

                let show_id = show.id().clone();
                let quit_id = quit.id().clone();
                let born = std::time::Instant::now();
                MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
                    let action = if ev.id == quit_id {
                        Some(TrayEvent::Quit)
                    } else if ev.id == show_id {
                        Some(TrayEvent::Show)
                    } else {
                        None
                    };
                    // 图标刚注册时系统可能派发一次伪激活（无用户点击）——启动 1.5 秒内的
                    // 事件一律丢弃，避免"自己弹出来/自己退出"。
                    let fresh = born.elapsed().as_millis();
                    if fresh < 1500 {
                        return;
                    }
                    if matches!(action, Some(TrayEvent::Show)) {
                        show_main_window();
                    }
                    if let Some(a) = action {
                        let _ = ev_tx.send(a);
                        // 关键：把 UI 线程叫醒，否则隐藏状态下没人处理这个事件
                        ctx.request_repaint();
                        if matches!(a, TrayEvent::Quit) {
                            // 兜底：2 秒内窗口还没退干净就直接结束进程（配置已随改随存）
                            std::thread::spawn(|| {
                                std::thread::sleep(Duration::from_millis(2000));
                                std::process::exit(0);
                            });
                        }
                    }
                }));
                let _ = ready_tx.send(true);

                // 托盘线程自己泵消息，菜单点击才会被派发；_tray 活到这个循环结束
                unsafe {
                    let mut msg: Msg = std::mem::zeroed();
                    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            });

            match ready_rx.recv_timeout(Duration::from_millis(800)) {
                Ok(true) => Some(Self {
                    events: ev_rx,
                    _thread: thread,
                }),
                _ => None,
            }
        }

        /// 取一个待处理的托盘事件（update 里调用，tray 线程已负责叫醒 UI）。
        pub fn poll(&self) -> Option<TrayEvent> {
            self.events.try_recv().ok()
        }
    }
}

// ---------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::mpsc::{channel, Receiver};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    #[derive(Clone, Copy, Debug)]
    pub enum TrayEvent {
        Show,
        Quit,
    }

    fn make_icon() -> Icon {
        const S: u32 = 32;
        let mut px = vec![0u8; (S * S * 4) as usize];
        for y in 0..S {
            for x in 0..S {
                let fx = x as f32 + 0.5;
                let fy = y as f32 + 0.5;
                let r = 6.0;
                let cx = fx.clamp(r, (S as f32) - r);
                let cy = fy.clamp(r, (S as f32) - r);
                if ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt() <= r {
                    let i = ((y * S + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[0x1a, 0x1d, 0x22, 255]);
                }
            }
        }
        for (x0, x1, top, bottom) in [
            (7u32, 10u32, 18u32, 26u32),
            (14, 17, 9, 26),
            (21, 24, 13, 22),
        ] {
            for y in top..bottom {
                for x in x0..x1 {
                    let i = ((y * S + x) * 4) as usize;
                    px[i..i + 4].copy_from_slice(&[0x4c, 0x8d, 0xf6, 255]);
                }
            }
        }
        Icon::from_rgba(px, S, S).expect("图标数据非法")
    }

    pub struct Tray {
        _tray: TrayIcon,
        events: Receiver<TrayEvent>,
    }

    /// 本平台是否支持托盘
    pub fn supported() -> bool {
        true
    }

    impl Tray {
        /// macOS 的菜单栏项必须在主线程创建（winit 主线程满足）。
        pub fn new(
            ctx: eframe::egui::Context,
            show_label: &str,
            quit_label: &str,
            tooltip: &str,
        ) -> Option<Self> {
            let menu = Menu::new();
            let show = MenuItem::with_id("tvx-show", show_label, true, None);
            let quit = MenuItem::with_id("tvx-quit", quit_label, true, None);
            menu.append(&show).ok()?;
            menu.append(&quit).ok()?;
            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip(tooltip)
                .with_icon(make_icon())
                .build()
                .ok()?;

            let (tx, rx) = channel::<TrayEvent>();
            let show_id = show.id().clone();
            let quit_id = quit.id().clone();
            let born = std::time::Instant::now();
            MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
                let action = if ev.id == quit_id {
                    Some(TrayEvent::Quit)
                } else if ev.id == show_id {
                    Some(TrayEvent::Show)
                } else {
                    None
                };
                // 图标刚注册时系统可能派发一次伪激活，启动 1.5 秒内一律丢弃
                if born.elapsed().as_millis() < 1500 {
                    return;
                }
                if let Some(a) = action {
                    let _ = tx.send(a);
                    ctx.request_repaint();
                    if matches!(a, TrayEvent::Quit) {
                        // 兜底：窗口隐藏时帧循环可能不跑，2 秒后直接结束进程
                        std::thread::spawn(|| {
                            std::thread::sleep(std::time::Duration::from_millis(2000));
                            std::process::exit(0);
                        });
                    }
                }
            }));

            Some(Self {
                _tray: tray,
                events: rx,
            })
        }

        pub fn poll(&self) -> Option<TrayEvent> {
            self.events.try_recv().ok()
        }
    }
}

// ---------------------------------------------------------------- Linux 桩

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    #[allow(dead_code)]
    #[derive(Clone, Copy, Debug)]
    pub enum TrayEvent {
        Show,
        Quit,
    }

    /// Linux 桩：没有托盘，关闭窗口即退出。
    pub struct Tray;

    impl Tray {
        pub fn new(
            _ctx: eframe::egui::Context,
            _show: &str,
            _quit: &str,
            _tooltip: &str,
        ) -> Option<Self> {
            None
        }
        pub fn poll(&self) -> Option<TrayEvent> {
            None
        }
    }
}

pub use imp::{Tray, TrayEvent};

#[cfg(windows)]
pub use imp::show_any_tvx_window;
#[cfg(any(windows, target_os = "macos"))]
pub use imp::supported;

/// 非 Windows 平台没有这个兜底（Linux 无托盘、macOS 走主线程视口命令）。
#[cfg(not(windows))]
#[allow(dead_code)]
pub fn show_main_window() {}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn supported() -> bool {
    false
}

#[cfg(not(windows))]
pub fn show_any_tvx_window() {}
