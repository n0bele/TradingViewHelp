//! 命令行入口：`tvhelper <子命令>`。
//!
//! 界面之外的每条维护路径都在这里收口：参数解析（`parse`，纯函数、可测）与
//! 执行（`run`）分开，`main.rs` 只剩"解析 → 执行或开界面"。
//!
//! 这些子命令的存在理由都一样：**release 版没有控制台**。用户双击运行后出问题，
//! 只能靠命令行版本把结论打到 stdout 并落盘到 `~/.tvx/`。

use crate::{app, install, profiles, tray};

/// 一次启动要走哪条路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cmd {
    /// 正常启动界面
    Gui,
    /// 打印版本 + 自身路径 + 文件大小，并写 `~/.tvx/version.txt`
    Version,
    /// 按当前配置重打托管副本的补丁（不启界面）
    Reapply,
    /// 从系统安装构建 / 重建托管副本
    Build,
    /// 环境自检，报告落盘 `~/.tvx/doctor.txt`
    Doctor,
    /// 深链分发：`tradingview://…` 交给对应账号的实例
    Auth(String),
    /// 抓取被商店版抢走的授权回调，最多等 N 秒
    CaptureHandoff(u32),
    /// 把已经在托盘里的窗口显示出来
    Show,
}

/// 解析参数（不含 argv[0]）。
///
/// 认不出的参数一律当"正常启动"——用户可能只是双击时带了个拖进来的文件路径，
/// 这时报错退出比直接开界面更让人困惑。
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Cmd {
    let args: Vec<String> = args.into_iter().collect();
    let has = |name: &str| args.iter().any(|a| a == name);

    if has("_version") || has("--version") || has("-V") {
        return Cmd::Version;
    }
    if has("_reapply") {
        return Cmd::Reapply;
    }
    if has("_build") {
        return Cmd::Build;
    }
    if has("_doctor") {
        return Cmd::Doctor;
    }
    if has("_auth") {
        // 没带 URL 时什么都不做：这是浏览器把协议命令拼错的情形，不是要开界面
        return Cmd::Auth(
            args.iter()
                .find(|a| a.starts_with("tradingview://"))
                .cloned()
                .unwrap_or_default(),
        );
    }
    if has("_capture-handoff") {
        let secs = args
            .iter()
            .filter_map(|a| a.parse::<u32>().ok())
            .next()
            .unwrap_or(20);
        return Cmd::CaptureHandoff(secs);
    }
    if has("_show") {
        return Cmd::Show;
    }
    Cmd::Gui
}

/// 执行无头子命令。`Cmd::Gui` 不在处理范围内（由 `main` 继续启动界面）。
pub fn run(cmd: Cmd) {
    match cmd {
        Cmd::Gui => {}
        Cmd::Version => print_version(),
        Cmd::Reapply => app::reapply_headless(),
        Cmd::Build => app::build_headless(),
        Cmd::Doctor => print_doctor(),
        Cmd::Auth(url) => {
            if !url.is_empty() {
                profiles::route_auth(&url);
            }
        }
        Cmd::CaptureHandoff(secs) => match profiles::capture_handoff_once(secs) {
            Some((pid, url)) => println!("已接管回调 PID={pid} URL={url}"),
            None => println!("窗口期内没有捕获到回调"),
        },
        Cmd::Show => tray::show_any_tvx_window(),
    }
}

/// 版本身份：桌面和下载目录里可能同时躺着好几个同名 exe，只看界面分不出跑的哪个文件。
/// 路径 + 大小 + 版本号一起，才是这一份二进制的指纹。
fn print_version() {
    let exe = std::env::current_exe().unwrap_or_default();
    let size = std::fs::metadata(&exe).map(|m| m.len()).unwrap_or(0);
    let text = format!(
        "tvhelper v{}\nexe : {}\nsize: {} 字节 ({:.2} MB)\n",
        env!("CARGO_PKG_VERSION"),
        exe.display(),
        size,
        size as f64 / 1048576.0
    );
    print!("{text}");
    save_report("version.txt", &text);
}

fn print_doctor() {
    let report = install::doctor_report();
    print!("{report}");
    save_report("doctor.txt", &report);
}

fn save_report(name: &str, body: &str) {
    let path = install::tvx_root().join(name);
    match std::fs::write(&path, body) {
        Ok(()) => println!("报告已写入 {}", path.display()),
        Err(e) => println!("报告写入失败：{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(args: &[&str]) -> Cmd {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn bare_invocation_opens_gui() {
        assert_eq!(parse_str(&[]), Cmd::Gui);
        assert_eq!(parse_str(&["C:\\some\\file.txt"]), Cmd::Gui);
    }

    #[test]
    fn version_aliases_all_work() {
        for flag in ["_version", "--version", "-V"] {
            assert_eq!(parse_str(&[flag]), Cmd::Version);
        }
    }

    #[test]
    fn headless_maintenance_commands() {
        assert_eq!(parse_str(&["_reapply"]), Cmd::Reapply);
        assert_eq!(parse_str(&["_build"]), Cmd::Build);
        assert_eq!(parse_str(&["_doctor"]), Cmd::Doctor);
        assert_eq!(parse_str(&["_show"]), Cmd::Show);
    }

    #[test]
    fn auth_needs_a_url_but_still_routes() {
        assert_eq!(
            parse_str(&["_auth", "tradingview://browser-auth/?token=abc"]),
            Cmd::Auth("tradingview://browser-auth/?token=abc".into())
        );
        // 缺 URL：解析成空串，执行时静默跳过（浏览器拼错命令的情形）
        assert_eq!(parse_str(&["_auth"]), Cmd::Auth(String::new()));
    }

    #[test]
    fn capture_handoff_takes_first_number_and_defaults_to_20() {
        assert_eq!(parse_str(&["_capture-handoff"]), Cmd::CaptureHandoff(20));
        assert_eq!(parse_str(&["_capture-handoff", "45"]), Cmd::CaptureHandoff(45));
    }
}
