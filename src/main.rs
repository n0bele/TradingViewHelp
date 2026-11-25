#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! TradingView Desktop 本地助手（单机、无服务端）。
//!
//! 分工见各模块文档；`main` 只做三件事：解析命令行 → 执行无头命令 → 否则开界面。

mod app;
mod asar;
mod browser;
mod cli;
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
    // 无头维护路径（版本 / 自检 / 重打补丁 / 构建副本 / 深链 / 托盘唤醒）
    let cmd = cli::parse(std::env::args().skip(1));
    if cmd != cli::Cmd::Gui {
        cli::run(cmd);
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

    // 窗口标题跟随上次选择的界面语言；带上版本号，任务栏也能一眼看出跑的是哪一版
    let cfg = install::load_config();
    let title = format!("{} v{}", cfg.lang.s().app_title, env!("CARGO_PKG_VERSION"));

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
