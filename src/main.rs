#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod browser;
mod asar;
mod i18n;
#[cfg(target_os = "macos")]
mod macos_app;
mod install;
mod patch;
mod profiles;
mod protocol;
mod tray;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // 无头维护模式：tvhelper _reapply —— 按当前配置重打托管副本补丁，不启动界面
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "_reapply") {
        app::reapply_headless();
        return Ok(());
    }
    // 深链分发：`tvhelper _auth tradingview://...`
    if args.iter().any(|a| a == "_auth") {
        if let Some(url) = args.iter().find(|a| a.starts_with("tradingview://")) {
            profiles::route_auth(url);
        }
        return Ok(());
    }

    // 抓取被商店版抢走的授权回调：`tvhelper _capture-handoff [秒数]`
    if args.iter().any(|a| a == "_capture-handoff") {
        let secs = args
            .iter()
            .filter_map(|a| a.parse::<u32>().ok())
            .next()
            .unwrap_or(20);
        match profiles::capture_handoff_once(secs) {
            Some((pid, url)) => println!("已接管回调 PID={pid} URL={url}"),
            None => println!("窗口期内没有捕获到回调"),
        }
        return Ok(());
    }

    // 恢复窗口：`tvhelper _show`（托盘「显示主窗口」用的同一套逻辑，可脚本验证）
    if args.iter().any(|a| a == "_show") {
        tray::show_any_tvx_window();
        return Ok(());
    }

    // Linux + Wayland：winit 的 Wayland 后端 set_visible 是空实现，窗口无法隐藏，
    // 「关闭收进托盘」会失效；有 Xwayland 时改走 X11 后端。
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some()
        && (std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var_os("WAYLAND_SOCKET").is_some())
    {
        std::env::remove_var("WAYLAND_DISPLAY");
        std::env::remove_var("WAYLAND_SOCKET");
    }

    // 窗口标题跟随上次选择的界面语言
    let cfg = install::load_config();
    let title = cfg.lang.s().app_title.to_string();

    // 窗口图标（PNG 解码为 RGBA）；失败则用系统默认图标，不影响功能
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
        .ok()
        .map(std::sync::Arc::new);

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([1020.0, 660.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(icon.unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native(
        "tvhelper",
        opts,
        Box::new(|cc| {
            app::setup_fonts(&cc.egui_ctx);
            app::setup_style(&cc.egui_ctx);
            Ok(Box::new(app::App::new()))
        }),
    )
}
