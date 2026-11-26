//! 账号档案：每个账号一个独立 user-data-dir，因此可以同时开多个实例、各自登录。

use crate::i18n::Lang;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::install::{tvx_root, Managed};

/// CREATE_NO_WINDOW：tvx 是 GUI 程序（无控制台），起 tasklist/taskkill 这类
/// 控制台子进程时若不加此标志，Windows 会给每个子进程新建一个 cmd 风格的
/// 控制台窗口——后台扫描每 2 秒一次，表现为"一直在启动 cmd 进程"。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub dir: PathBuf,
}

pub fn profiles_root() -> PathBuf {
    tvx_root().join("profiles")
}

pub fn list() -> Vec<Profile> {
    let root = profiles_root();
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(&root) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = e.file_name().to_str() {
                    out.push(Profile {
                        name: name.to_string(),
                        dir: e.path(),
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

// ---------------------------------------------------------------- 账号身份
//
// 「一个账号 = 一个 profile」靠这里落地：登录回调里解出 JWT 的 user_id，
// 写进账号目录的 `.tvx-user`；启动时若同一 user_id 已有另一个实例在线，直接拒绝。
// 服务端按账号限制并发连接，同账号双开必被踢——这不是客户端能改的配额，
// 客户端能做的是别把自己撞上去。

mod account;
mod auth;
mod runtime;

pub use account::*;
pub use auth::*;
pub use runtime::*;

pub fn validate_name(name: &str, lang: Lang) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        bail!(lang.v_empty());
    }
    if n.len() > 40 {
        bail!(lang.v_too_long());
    }
    if n.starts_with('.') {
        bail!(lang.v_dot());
    }
    if n.chars().any(|c| {
        matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
    }) {
        bail!(lang.v_chars());
    }
    Ok(n.to_string())
}

/// 新建档案：只建一个空的 user-data 目录。
///
/// 登录态不复制、不迁移——每个实例首次启动时在 TradingView 自己的窗口里登录一次，
/// 之后由客户端自己把会话保存在这个目录里（Electron `--user-data-dir` 的语义）。
pub fn create(name: &str, lang: Lang) -> Result<Profile> {
    let name = validate_name(name, lang)?;
    let dir = profiles_root().join(&name);
    if dir.exists() {
        bail!(lang.p_dir_exists(&name));
    }
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建账号目录 {}", dir.display()))?;
    Ok(Profile { name, dir })
}

pub fn remove(profile: &Profile, lang: Lang) -> Result<()> {
    if running_pid(profile).is_some() {
        bail!(lang.p_running());
    }
    #[cfg(target_os = "macos")]
    crate::macos_app::remove_profile_app(profile);

    fs::remove_dir_all(&profile.dir)
        .with_context(|| format!("删除 {}", profile.dir.display()))?;
    Ok(())
}

/// 改名：目录跟着改，登录态原地保留（不复制任何数据）。
/// 实例在跑时 Windows 会锁住目录，先要求停掉。
pub fn rename(profile: &Profile, new_name: &str, lang: Lang) -> Result<Profile> {
    if running_pid(profile).is_some() {
        bail!(lang.p_running());
    }
    let new_name = validate_name(new_name, lang)?;
    if new_name == profile.name {
        return Ok(profile.clone());
    }
    let new_dir = profiles_root().join(&new_name);
    if new_dir.exists() {
        bail!(lang.p_dir_exists(&new_name));
    }
    fs::rename(&profile.dir, &new_dir).with_context(|| {
        format!(
            "改名 {} → {}",
            profile.dir.display(),
            new_dir.display()
        )
    })?;
    // macOS 每个账号有一份 .app 副本，bundle id 里带着旧账号名：删掉，下次启动按新名重建
    #[cfg(target_os = "macos")]
    crate::macos_app::remove_profile_app(profile);
    Ok(Profile {
        name: new_name,
        dir: new_dir,
    })
}

// ---------------------------------------------------------------- 进程

fn pid_file(profile_dir: &Path) -> PathBuf {
    profile_dir.join(".tvx-pid")
}

/// PID 文件格式：`<pid>:<创建时间>`；旧格式只有 `<pid>`（创建时间未知）。
fn read_pid(profile_dir: &Path) -> Option<(u32, Option<u64>)> {
    let raw = fs::read_to_string(pid_file(profile_dir)).ok()?;
    let raw = raw.trim();
    let mut it = raw.split(':');
    let pid = it.next()?.parse::<u32>().ok()?;
    let ctime = it.next().and_then(|v| v.parse::<u64>().ok());
    Some((pid, ctime))
}

fn clear_pid(profile_dir: &Path) {
    let _ = fs::remove_file(pid_file(profile_dir));
}

/// 一次调用拿到“所有活着的 TradingView 进程号”。
/// 仅在非 Windows 平台使用（Windows 走 win_proc 原生检查，不起子进程）。
#[cfg(not(windows))]
pub fn alive_set() -> std::collections::HashSet<u32> {
    let mut set = std::collections::HashSet::new();
    let out = Command::new("ps").args(["-A", "-o", "pid="]).output();
    if let Ok(o) = out {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Ok(p) = line.trim().parse::<u32>() {
                set.insert(p);
            }
        }
    }
    set
}

/// 进程存活检查的 Windows 原生实现：直接问内核，零子进程。
///
/// 旧实现每次盘点都 spawn 一次 `tasklist`——后台扫描每 2 秒一次，既是多余的
/// 进程开销，也是"一直在启动 cmd 进程"的来源。OpenProcess + 查询映像名
/// 是微秒级调用，且能区分 PID 复用（比对映像文件名）。
#[cfg(windows)]
mod win_proc {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn CloseHandle(h: *mut c_void) -> i32;
        fn QueryFullProcessImageNameW(
            h: *mut c_void,
            flags: u32,
            buf: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn GetProcessTimes(
            h: *mut c_void,
            creation: *mut u64,
            exit: *mut u64,
            kernel: *mut u64,
            user: *mut u64,
        ) -> i32;
    }

    /// 进程创建时间（FILETIME，100ns 计数）。用于区分 PID 复用。
    pub fn creation_time(pid: u32) -> Option<u64> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
            let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
            CloseHandle(h);
            (ok != 0 && c != 0).then_some(c)
        }
    }

    /// PROCESS_QUERY_LIMITED_INFORMATION：同用户权限下即可查询映像路径
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    /// PID 是否存活，且可执行映像文件名（忽略大小写）等于 `exe_name`。
    pub fn alive_with_image(pid: u32, exe_name: &str) -> bool {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false; // 进程不存在 / 权限不足
            }
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut size);
            CloseHandle(h);
            if ok == 0 {
                return false;
            }
            let path = String::from_utf16_lossy(&buf[..size as usize]);
            let file = path.rsplit(|c| c == '\\' || c == '/').next().unwrap_or("");
            file.eq_ignore_ascii_case(exe_name)
        }
    }
}

/// 当前有多少个账号实例在跑（用于决定协议命令是否还原）
fn profiles_with_running_instances() -> usize {
    list().iter().filter(|p| running_pid(p).is_some()).count()
}

pub fn is_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        win_proc::alive_with_image(pid, "TradingView.exe")
    }
    #[cfg(not(windows))]
    {
        alive_set().contains(&pid)
    }
}

/// 该账号的实例是否在跑：PID 存活 + 映像名匹配 + 进程创建时间一致。
///
/// 第三项是关键：用户自己结束进程后，Windows 可能把同一个 PID 分配给别的
/// TradingView 实例，只比 PID 会误判成"还在跑"。创建时间对不上就说明是别人的
/// 进程，顺手清掉陈旧 PID 文件，界面随即回到"未启动"。
pub fn running_pid(profile: &Profile) -> Option<u32> {
    let (pid, ctime) = read_pid(&profile.dir)?;
    #[cfg(not(windows))]
    let _ = ctime;
    if !is_alive(pid) {
        clear_pid(&profile.dir);
        return None;
    }
    #[cfg(windows)]
    if let Some(expected) = ctime {
        match win_proc::creation_time(pid) {
            Some(actual) if actual == expected => {}
            _ => {
                clear_pid(&profile.dir);
                return None;
            }
        }
    }
    Some(pid)
}

/// 一次性把一批账号的运行状态算出来（Windows 上是纯内核查询，无子进程）。
pub fn status_snapshot(profiles: &[Profile]) -> std::collections::HashMap<String, Option<u32>> {
    profiles
        .iter()
        .map(|p| (p.name.clone(), running_pid(p)))
        .collect()
}

/// 用独立 user-data-dir 启动一个实例。返回进程号。
/// 系统代理是否可连（只看端口通不通）。返回 Some(地址) 表示配置了但连不上。
pub fn proxy_unreachable() -> Option<String> {
    let raw = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .ok()?;
    let addr = raw
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string();
    if addr.is_empty() {
        return None;
    }
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration as D;
    match addr.to_socket_addrs().ok()?.next() {
        Some(sa) => match TcpStream::connect_timeout(&sa, D::from_millis(400)) {
            Ok(_) => None,
            Err(_) => Some(addr),
        },
        None => Some(addr),
    }
}

#[cfg(test)]
mod tests;
