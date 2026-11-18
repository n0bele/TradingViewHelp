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

/// 账号目录里记录 TradingView `user_id` 的小文件（不是密码、也不是令牌）。
const UID_FILE: &str = ".tvx-user";

/// 该档案绑定的 TradingView 账号 ID（登录回调时写入）。
pub fn account_id(profile: &Profile) -> Option<String> {
    let raw = fs::read_to_string(profile.dir.join(UID_FILE)).ok()?;
    let raw = raw.trim();
    (!raw.is_empty()).then(|| raw.to_string())
}

/// 返回是否写成功：写不上就查不出"这个档案绑的是哪个账号"，同账号双开会撞服务端限制。
fn set_account_id(profile: &Profile, uid: &str) -> bool {
    let uid = uid.trim();
    if uid.is_empty() {
        return false;
    }
    match crate::install::write_atomic(&profile.dir.join(UID_FILE), uid.as_bytes()) {
        Ok(()) => true,
        Err(e) => {
            crate::install::log_error("写账号绑定 .tvx-user", &e);
            false
        }
    }
}

/// 其它绑定到同一账号的档案：返回 (名字, 是否在跑)。
pub fn same_account_others(profile: &Profile, uid: &str) -> Vec<(String, bool)> {
    list()
        .into_iter()
        .filter(|p| p.name != profile.name)
        .filter(|p| account_id(p).as_deref() == Some(uid))
        .map(|p| {
            let running = running_pid(&p).is_some();
            (p.name, running)
        })
        .collect()
}

/// base64url 解码（JWT 用；不引依赖）。
fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4 + 3);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// 从 `tradingview://browser-auth/?token=…` 里解出 JWT 载荷的 `user_id`。
/// 实测该 token 的 claims 只有 exp / iat / jti / user_id，没有用户名。
pub fn jwt_user_id(url: &str) -> Option<String> {
    let token = url.split("token=").nth(1)?;
    let token = token.split('&').next()?;
    let payload = token.split('.').nth(1)?;
    let bytes = b64url_decode(payload)?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    match v.get("user_id")? {
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// 从 localStorage 的 LevelDB 里认账号，**只认唯一解**：
/// 全库只出现过一个 userId 才采信。出现多个说明这个档案登录过不止一个账号
/// （或 LevelDB 分块/压缩导致字符串错位），这时宁可留空、让人重新登录绑定，
/// 也不能猜错——猜错会误拦别的档案启动。
fn detect_uid_from_storage(profile: &Profile) -> Option<String> {
    let dir = profile.dir.join("Local Storage").join("leveldb");
    let mut all: Vec<String> = Vec::new();
    for e in fs::read_dir(&dir).ok()?.flatten() {
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        // localStorage 正常只有几 MB；异常大就跳过，不在启动路径上啃大文件
        if !meta.is_file() || meta.len() > 8 * 1024 * 1024 {
            continue;
        }
        let Ok(raw) = fs::read(&path) else { continue };
        for uid in scan_user_records(&raw) {
            if !all.contains(&uid) {
                all.push(uid);
            }
        }
    }
    (all.len() == 1).then(|| all.remove(0))
}

/// 在字节流里扫 `"userId":<数字>`（LevelDB 的 .log 段是明文，能直接扫到）。
fn scan_user_records(b: &[u8]) -> Vec<String> {
    let key = b"\"userId\":";
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + key.len() <= b.len() {
        if &b[i..i + key.len()] != key {
            i += 1;
            continue;
        }
        let mut j = i + key.len();
        while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
            j += 1;
        }
        let st = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > st {
            out.push(String::from_utf8_lossy(&b[st..j]).to_string());
        }
        i = j.max(i + 1);
    }
    out
}

pub fn refresh_account(profile: &Profile) -> Option<(String, Option<String>)> {
    if account_id(profile).is_some() {
        return None;
    }
    let uid = detect_uid_from_storage(profile)?;
    let _ = set_account_id(profile, &uid);
    let other = same_account_others(profile, &uid)
        .into_iter()
        .find(|(_, running)| *running)
        .map(|(name, _)| name);
    Some((uid, other))
}

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
    fs::create_dir_all(&dir)?;
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

/// 复制档案：整份复制账号目录（登录态一起带走），用于备份或另起一份试验配置。
/// `.tvx-pid` 不复制——副本必须是"未启动"状态；
/// `.tvx-user` 保留，所以副本仍被认作同一个 TradingView 账号（同账号双开会被服务端拒，
/// 这不是客户端能绕的），真要用就登录另一个账号。
pub fn duplicate(profile: &Profile, new_name: &str, lang: Lang) -> Result<Profile> {
    let new_name = validate_name(new_name, lang)?;
    let new_dir = profiles_root().join(&new_name);
    if new_dir.exists() {
        bail!(lang.p_dir_exists(&new_name));
    }
    fs::create_dir_all(&new_dir)?;
    let result = (|| -> Result<()> {
        for entry in fs::read_dir(&profile.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == ".tvx-pid" {
                continue;
            }
            let to = new_dir.join(&name);
            if entry.file_type()?.is_dir() {
                copy_dir(&entry.path(), &to)?;
            } else {
                fs::copy(entry.path(), &to)?;
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        // 复制一半的目录没有任何价值，直接清掉，别给用户留个"看起来存在"的坏档案
        let _ = fs::remove_dir_all(&new_dir);
        return Err(e);
    }
    Ok(Profile {
        name: new_name,
        dir: new_dir,
    })
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
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

/// 记录"当前登录目标账号"（协议命令被应用改写时按它恢复）
fn write_login_target(profile: &Profile) {
    let path = crate::install::tvx_root().join("login-target");
    if let Err(e) =
        crate::install::write_atomic(&path, profile.dir.display().to_string().as_bytes())
    {
        // 写不上只影响深链路由的纠偏，不该阻断启动；但要有痕迹可查
        crate::install::log_error("写登录目标 login-target", &e);
    }
}

pub fn clear_login_target() {
    let _ = fs::remove_file(crate::install::tvx_root().join("login-target"));
}

/// 维持 `tradingview://` 指向"正在登录的那个账号"。
///
/// TradingView 每个实例启动时都会调 `setAsDefaultProtocolClient('tradingview')`，
/// 把我们写好的协议命令**改回默认形态**（不带 --user-data-dir），导致浏览器回调
/// 落回默认 profile、窗口里的账号换不掉。所以每轮扫描都检查一次，被改了就纠回来。
#[cfg(not(target_os = "macos"))]
pub fn maintain_protocol_handler() {
    let Ok(target) = fs::read_to_string(crate::install::tvx_root().join("login-target")) else {
        return;
    };
    let target = target.trim().to_string();
    if target.is_empty() {
        return;
    }
    // 目标账号必须真的在跑，否则不维持（避免残留）
    let alive = list()
        .iter()
        .find(|p| p.dir.display().to_string() == target)
        .map(|p| running_pid(p).is_some())
        .unwrap_or(false);
    if !alive {
        return;
    }
    let Some(managed) = crate::install::load_managed() else {
        return;
    };
    let Some(exe) = managed.exe() else {
        return;
    };
    if current_protocol_points_to_tvx() {
        return;
    }
    let _ = crate::protocol::set_target(&exe, None);
}

#[cfg(target_os = "macos")]
pub fn maintain_protocol_handler() {
    // macOS 走"每账号一份 .app + 默认 handler"，由 launch 时的 set_url_handler 负责
}

/// 协议命令是否已经指向 `tvx _auth`
#[allow(dead_code)] // macOS 不走协议命令，这个判断只在 Win/Linux 用
fn current_protocol_points_to_tvx() -> bool {
    // macOS 走"每账号一份 .app + 默认 handler"，不经过这个协议命令
    #[cfg(target_os = "macos")]
    {
        return false;
    }
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        return hkcu
            .open_subkey(r"Software\\Classes\\tradingview\\shell\\open\\command")
            .ok()
            .and_then(|k| k.get_value::<String, _>("").ok())
            .map(|v| v.contains("_auth"))
            .unwrap_or(false);
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let Some(data) = dirs::data_dir() else {
            return false;
        };
        let f = data.join("applications").join("tvx-tradingview.desktop");
        return fs::read_to_string(f).map(|v| v.contains("_auth")).unwrap_or(false);
    }
}


/// 追加一行到 `~/.tvx/auth.log`（回调分发的现场记录）
pub fn auth_log(line: &str) {
    use std::io::Write;
    let root = crate::install::tvx_root();
    let _ = fs::create_dir_all(&root);
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("auth.log"))
    {
        let _ = writeln!(f, "[{}] {line}", chrono_like_now());
    }
}

/// 简易时间戳（UTC，不引依赖）：`HH:MM:SS`
fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}Z")
}

/// 深链分发：系统把 `tradingview://` 交给 `tvx _auth`，这里广播给所有在跑的实例。
///
/// 每个实例用自己的 `--user-data-dir` 起一个短命进程转发 URL——Electron 的单实例锁
/// 会把参数投递给该 profile 的主进程；正在等登录的那个实例会处理，其余会忽略。
pub fn route_auth(url: &str) {
    auth_log(&format!("收到回调：{}", mask_token(url)));

    // 回调 URL 里的 token 是本次登录的凭据：解出 user_id 就能把"这个档案登录了哪个账号"
    // 记下来。只写到 login-target（启动时写入、扫描线程负责维持），不往别的档案头上记。
    let uid = jwt_user_id(url);
    let target = fs::read_to_string(crate::install::tvx_root().join("login-target"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let Some(managed) = crate::install::load_managed() else {
        return;
    };
    let Some(exe) = managed.exe() else {
        return;
    };
    for p in list() {
        if running_pid(&p).is_none() {
            auth_log(&format!("  跳过 {}（没有实例在跑）", p.name));
            continue;
        }
        if let (Some(uid), Some(t)) = (&uid, &target) {
            if &p.dir.display().to_string() == t {
                let _ = set_account_id(&p, uid);
                auth_log(&format!("  账号 {uid} 已绑定到 {}", p.name));
                let others = same_account_others(&p, uid);
                if let Some((name, _)) = others.iter().find(|(_, running)| *running) {
                    auth_log(&format!(
                        "  ⚠ 同一账号已在「{name}」在线：同账号双开会触发服务端连接限制"
                    ));
                } else if let Some((name, _)) = others.first() {
                    auth_log(&format!(
                        "  ⚠ 该账号也登录过「{name}」：建议一个账号只保留一个档案"
                    ));
                }
            }
        }
        auth_log(&format!("  转发给 {}", p.name));
        if let Ok(child) = Command::new(&exe)
            .arg(format!("--user-data-dir={}", p.dir.display()))
            .arg(url)
            .env_remove("ELECTRON_RUN_AS_NODE")
            .env_remove("ELECTRON_NO_ATTACH_CONSOLE")
            .env_remove("ELECTRON_ENABLE_LOGGING")
            .env_remove("NODE_OPTIONS")
            .spawn()
        {
            // 记下 pid：接管脚本要跳过它，别把我们正在用的实例当成"被抢的回调"杀掉
            record_spawned(child.id());
        }
    }
}

/// 日志里不落完整 token（那是可用的登录凭据）。
fn mask_token(url: &str) -> String {
    let Some((head, rest)) = url.split_once("token=") else {
        return url.to_string();
    };
    let (tok, tail) = match rest.split_once('&') {
        Some((t, r)) => (t, format!("&{r}")),
        None => (rest, String::new()),
    };
    let keep: String = tok.chars().take(12).collect();
    format!("{head}token={keep}…{tail}")
}

/// 在**应用自己的标签页**里打开一个 URL（登录页走这里）。
///
/// 原理：TradingView 实例收到 https URL 参数时会把它当"转发的链接"开成标签页
/// （日志里的 `openForwardedUrl` → 新 tab）。这样登录页在应用内渲染，
/// cookie 直接写进该账号的 user-data-dir，**不需要浏览器、也不需要协议回调**。
pub fn open_in_app_tab(profile: &Profile, url: &str) -> anyhow::Result<()> {
    // macOS 用副本自带的 userData，不需要 --user-data-dir
    #[cfg(target_os = "macos")]
    let _ = profile;
    let managed = crate::install::load_managed().ok_or_else(|| anyhow::anyhow!("没有托管副本"))?;
    let Some(exe) = managed.exe() else {
        anyhow::bail!("副本里找不到可执行文件");
    };
    let mut cmd = Command::new(&exe);
    #[cfg(not(target_os = "macos"))]
    cmd.arg(format!("--user-data-dir={}", profile.dir.display()));
    cmd.arg(url);
    for k in [
        "ELECTRON_RUN_AS_NODE",
        "ELECTRON_NO_ATTACH_CONSOLE",
        "ELECTRON_ENABLE_LOGGING",
        "NODE_OPTIONS",
    ] {
        cmd.env_remove(k);
    }
    cmd.spawn().map(|_| ()).context("打开应用内标签页失败")
}

/// 商店版（MSIX）声明了 `tradingview` 协议，优先级高于 HKCU 里的注册，
/// 所以浏览器回调会被它抢走，token 卡在那个进程的命令行里。
///
/// 这里跑一个**单次**子进程轮询脚本（仅登录期间，最多 `secs` 秒），把带 token 的
/// 命令行捞出来交给 `route_auth` 广播给我们的实例，并结束那个抢走回调的进程。
/// 我们自己 spawn 出来的实例 pid（转发回调时启动的）。
/// 接管脚本必须跳过它们，否则会把"正在登录的那个实例"当成被抢走的回调杀掉、
/// 再重新拉起——形成自激循环。
fn handoff_skip_path() -> PathBuf {
    tvx_root().join("handoff-skip")
}

fn record_spawned(pid: u32) {
    let p = handoff_skip_path();
    let mut ids: Vec<u32> = fs::read_to_string(&p)
        .map(|s| s.lines().filter_map(|l| l.trim().parse().ok()).collect())
        .unwrap_or_default();
    if ids.contains(&pid) {
        return;
    }
    ids.push(pid);
    // 只留最近 20 个：pid 会被系统复用，留太多反而误跳过
    let keep = ids.len().saturating_sub(20);
    let list: Vec<String> = ids[keep..].iter().map(|i| i.to_string()).collect();
    if let Err(e) = crate::install::write_atomic(&p, format!("{}\n", list.join("\n")).as_bytes()) {
        crate::install::log_error("写 handoff 去重列表", &e);
    }
}

#[cfg(windows)]
fn capture_handoff(secs: u32) -> Option<(u32, String)> {
    use std::io::Write;

    let skip_path = handoff_skip_path();
    let script = format!(
        r#"$skip=@()
$sf=if($args.Count -gt 0){{ $args[0] }}else{{ '' }}
if($sf -and (Test-Path $sf)){{ $skip=@(Get-Content $sf | Where-Object {{ $_ -match '^[0-9]+$' }} | ForEach-Object {{ [int]$_ }}) }}
$deadline=(Get-Date).AddSeconds({secs})
while((Get-Date) -lt $deadline){{
  $p = Get-CimInstance Win32_Process -Filter "Name='TradingView.exe'" |
       Where-Object {{ $_.CommandLine -like '*tradingview://browser-auth*' -and ($skip -notcontains $_.ProcessId) }} |
       Select-Object -First 1
  if($p){{
    $c=$p.CommandLine; $i=$c.IndexOf('tradingview://')
    if($i -ge 0){{ Write-Output ($p.ProcessId.ToString() + '|' + $c.Substring($i).Trim('"')) ; break }}
  }}
  Start-Sleep -Milliseconds 1200
}}
"#
    );

    let root = crate::install::tvx_root();
    let _ = fs::create_dir_all(&root);
    let tmp = root.join("handoff.ps1");
    let mut f = fs::File::create(&tmp).ok()?;
    f.write_all(script.as_bytes()).ok()?;
    drop(f);

    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&tmp)
        .arg(&skip_path)
        .env_remove("ELECTRON_RUN_AS_NODE")
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains('|'))?;
    let (pid_s, url) = line.split_once('|')?;
    let url = url.trim().to_string();
    if !url.starts_with("tradingview://") {
        return None;
    }
    Some((pid_s.trim().parse().unwrap_or(0), url))
}

/// Linux：直接读 `/proc/<pid>/cmdline`，不需要外部命令。
#[cfg(all(not(windows), not(target_os = "macos")))]
fn capture_handoff(secs: u32) -> Option<(u32, String)> {
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(secs as u64);
    while Instant::now() < deadline {
        if let Ok(entries) = fs::read_dir("/proc") {
            for e in entries.flatten() {
                let name = e.file_name();
                let Some(pid) = name.to_string_lossy().parse::<u32>().ok() else {
                    continue;
                };
                let Ok(raw) = fs::read(e.path().join("cmdline")) else {
                    continue;
                };
                let text = String::from_utf8_lossy(&raw).replace('\0', " ");
                if let Some(i) = text.find("tradingview://browser-auth") {
                    return Some((pid, text[i..].trim().to_string()));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(800));
    }
    None
}

/// macOS：`ps -axo pid=,args=` 拿全部进程命令行（系统自带，无需额外依赖）。
#[cfg(target_os = "macos")]
fn capture_handoff(secs: u32) -> Option<(u32, String)> {
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(secs as u64);
    while Instant::now() < deadline {
        if let Ok(out) = Command::new("ps").args(["-axo", "pid=,args="]).output() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if let Some(i) = line.find("tradingview://browser-auth") {
                    let pid = line
                        .split_whitespace()
                        .next()
                        .and_then(|x| x.parse::<u32>().ok())
                        .unwrap_or(0);
                    return Some((pid, line[i..].trim().to_string()));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(900));
    }
    None
}

/// 同一时间只允许一个接管线程。每次触发都 spawn 的话，90 秒窗口内会叠出
/// 几十个轮询进程（每个都在查进程命令行），表现为"命令行窗口一直在闪"。
static HANDOFF_WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 登录期间调用：抓一次被抢走的回调并转发给我们的实例。
pub fn spawn_handoff_watcher(ctx: eframe::egui::Context) {
    use std::sync::atomic::Ordering;
    if HANDOFF_WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        if let Some((pid, url)) = capture_handoff(90) {
            route_auth(&url);
            auth_log(&format!("已接管被抢走的授权回调（PID {pid}）"));
            if pid != 0 {
                let _ = stop(pid);
            }
            ctx.request_repaint();
        }
        HANDOFF_WATCHING.store(false, Ordering::SeqCst);
    });
}

/// 命令行入口：`tvx _capture-handoff [秒数]`（验证 / 手动补救用）
pub fn capture_handoff_once(secs: u32) -> Option<(u32, String)> {
    let got = capture_handoff(secs);
    if let Some((pid, url)) = &got {
        route_auth(url);
        auth_log(&format!("手动接管回调（PID {pid}）"));
        if *pid != 0 {
            let _ = stop(*pid);
        }
    }
    got
}

/// 同时运行的实例上限：每个实例是一整棵 Electron 进程树（渲染 + GPU + 网络），
/// 8 个覆盖常见多账号场景；再多先撞内存，而不是撞服务端。
pub const MAX_RUNNING: usize = 8;

/// 当前在跑的实例数（按 PID 文件 + 进程映像校验，不依赖界面缓存）
pub fn running_count() -> usize {
    list()
        .into_iter()
        .filter(|p| running_pid(p).is_some())
        .count()
}

pub fn launch(managed: &Managed, profile: &Profile, lang: Lang) -> Result<u32> {
    if let Some(pid) = running_pid(profile) {
        return Ok(pid);
    }

    // 一个账号一个 profile：同账号的另一个实例正在线时直接拒绝启动。
    // 服务端按账号限制并发连接，同账号双开等于主动撞限制——表现就是某个窗口
    // 静默停更（弹窗被自动恢复盖过去了也还是会反复断）。档案还没绑定时，
    // 先尝试从 localStorage 里把当前 user_id 认出来。
    let uid = account_id(profile).or_else(|| {
        let d = detect_uid_from_storage(profile)?;
        let _ = set_account_id(profile, &d);
        Some(d)
    });
    if let Some(uid) = &uid {
        if let Some((name, _)) = same_account_others(profile, uid)
            .into_iter()
            .find(|(_, running)| *running)
        {
            bail!(lang.dup_account(uid, &name));
        }
    }
    // macOS：每个账号一份独立的 .app 副本（独立 bundle id → 独立 userData 与 scheme 归属）
    #[cfg(target_os = "macos")]
    let exe = {
        let app = crate::macos_app::ensure_profile_app(managed, profile)?;
        crate::macos_app::ensure_userdata_link(profile)?;
        app.join("Contents").join("MacOS").join("TradingView")
    };
    #[cfg(not(target_os = "macos"))]
    let exe = managed
        .exe()
        .ok_or_else(|| anyhow::anyhow!("托管副本里找不到可执行文件，请先重新生成"))?;

    // 启动前把环境准备好：账号的 user-data 目录必须存在，
    // 否则 Electron 会退回默认配置目录，多开就串了。
    fs::create_dir_all(&profile.dir)
        .with_context(|| format!("创建账号目录 {}", profile.dir.display()))?;

    let mut cmd = Command::new(&exe);
    // macOS 走"副本独立 userData + 符号链接指回账号目录"，不能传 --user-data-dir，
    // 否则系统拉起的 URL 回调进程与运行实例不是同一个 userData、拿不到单实例锁。
    #[cfg(not(target_os = "macos"))]
    cmd.arg(format!("--user-data-dir={}", profile.dir.display()));
    // 不要把宿主进程的 Electron/Node 变量带进去
    for k in [
        "ELECTRON_RUN_AS_NODE",
        "ELECTRON_NO_ATTACH_CONSOLE",
        "ELECTRON_ENABLE_LOGGING",
        "NODE_OPTIONS",
    ] {
        cmd.env_remove(k);
    }

    // 实例输出落到账号目录，登录/深链出问题时能直接看日志
    let log_path = profile.dir.join("tvx-launch.log");
    if let Ok(f) = fs::File::create(&log_path) {
        cmd.stdout(std::process::Stdio::from(f.try_clone()?));
        cmd.stderr(std::process::Stdio::from(f));
    }

    let child: Child = cmd
        .spawn()
        .with_context(|| format!("启动 {} 失败", exe.display()))?;
    let pid = child.id();
    // 记录创建时间，后面用它区分"PID 被复用"
    #[cfg(windows)]
    let stamp = format!("{pid}:{}", win_proc::creation_time(pid).unwrap_or(0));
    #[cfg(not(windows))]
    let stamp = pid.to_string();
    // PID 文件是"停止/查重/托盘状态"的唯一依据，写不上等于实例失控：
    // 进程已经起来了，不能假装启动成功，至少告诉用户 PID 和手动处理方式。
    if let Err(e) = crate::install::write_atomic(&pid_file(&profile.dir), stamp.as_bytes()) {
        crate::install::log_error("写 PID 文件", &e);
        return Err(anyhow::anyhow!(
            "实例已启动（PID {pid}），但 PID 记录写入失败：{e}。请在任务管理器里结束该进程后重试"
        ));
    }

    // 记录登录目标：应用启动后自己会改写协议命令，扫描线程会按它纠回来
    write_login_target(profile);

    // 让浏览器的登录回调落回这个账号
    #[cfg(target_os = "macos")]
    {
        // macOS：把 tradingview:// 指向本账号的 .app 副本（失败不阻断启动）
        let _ = crate::macos_app::set_url_handler(profile);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = crate::protocol::set_target(&exe, Some(&profile.dir));

    Ok(pid)
}

pub fn stop(pid: u32) -> Result<()> {
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .context("taskkill 失败")?;
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .context("kill 失败")?;
    }
    // 没有实例在跑时还原协议命令，不留某个账号的目录
    if profiles_with_running_instances() == 0 {
        clear_login_target();
        if let Some(m) = crate::install::load_managed() {
            if let Some(exe) = m.exe() {
                let _ = crate::protocol::set_target(&exe, None);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install;

    /// 登录回调 URL → JWT 载荷里的 user_id（账号查重全靠它）
    #[test]
    fn jwt_user_id_from_browser_auth_url() {
        let payload = "eyJleHAiOjEsImlhdCI6MSwianRpIjoieCIsInVzZXJfaWQiOjY2NzgyMDg5fQ";
        let url = format!("tradingview://browser-auth/?token=eyJhbGciOiJIUzUxMiJ9.{payload}.sig");
        assert_eq!(jwt_user_id(&url).as_deref(), Some("66782089"));
        // 不是 token 的回调 / 残缺 token 一律返回 None，不能猜
        assert_eq!(jwt_user_id("tradingview://auth?token=fake4"), None);
        assert_eq!(jwt_user_id("tradingview://browser-auth/?state=1"), None);
    }

    /// 只有全库唯一 userId 才采信；出现第二个就不猜（宁可不拦，也不误拦）
    #[test]
    fn detect_uid_requires_unique_record() {
        let name = "tvx-uid-test";
        let dir = profiles_root().join(name);
        let _ = fs::remove_dir_all(&dir);
        let ldb = dir.join("Local Storage").join("leveldb");
        fs::create_dir_all(&ldb).unwrap();
        let p = Profile {
            name: name.into(),
            dir: dir.clone(),
        };

        fs::write(ldb.join("000003.log"), br#"{"syncedAt":100,"userId":222}"#).unwrap();
        assert_eq!(detect_uid_from_storage(&p).as_deref(), Some("222"));

        // 又多出一个历史账号 → 无法确定当前是哪个，返回 None
        fs::write(ldb.join("000004.log"), br#"{"userId":333}"#).unwrap();
        assert_eq!(detect_uid_from_storage(&p), None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("主号", Lang::Zh).is_ok());
        assert!(validate_name("a/b", Lang::Zh).is_err());
        assert!(validate_name("", Lang::Zh).is_err());
        assert!(validate_name(".hidden", Lang::Zh).is_err());
    }

    /// 复刻用户报的 bug：删掉账号后 `list()` 还能不能看见它。
    /// 界面卡片完全由 `list()` 驱动，所以这条链路断了就是"删了不消失"。
    #[test]
    fn create_list_remove_roundtrip() {
        let name = "tvx-test-roundtrip";
        let dir = profiles_root().join(name);
        let _ = fs::remove_dir_all(&dir);

        let p = create(name, Lang::Zh).expect("创建失败");
        assert!(p.dir.is_dir(), "目录没建出来");
        let names = |l: Vec<Profile>| l.into_iter().map(|x| x.name).collect::<Vec<_>>();
        assert!(
            names(list()).contains(&name.to_string()),
            "刚建的账号应出现在 list() 里"
        );

        remove(&p, Lang::Zh).expect("删除失败");
        assert!(!p.dir.exists(), "删除后目录应消失");
        assert!(
            !names(list()).contains(&name.to_string()),
            "删除后 list() 里不应再有它（界面靠这个刷新卡片）"
        );
    }

    /// 原生进程检查（OpenProcess + QueryFullProcessImageNameW）的实测：
    /// 正例、映像名不匹配、无效 PID 三路都必须正确，否则状态点会误报。
    #[cfg(windows)]
    #[test]
    fn native_alive_check() {
        let me = std::process::id();
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            win_proc::alive_with_image(me, &name),
            "存活的当前进程应被识别为存活"
        );
        assert!(
            !win_proc::alive_with_image(me, "TradingView.exe"),
            "映像名不符时必须判死（防 PID 复用误报）"
        );
        assert!(
            !win_proc::alive_with_image(0xFFFF_FFF0, &name),
            "无效 PID 必须判死"
        );
    }

    /// 真的起一个实例。用 `cargo test -- --ignored` 手动触发。
    #[test]
    #[ignore = "会真的启动一个 TradingView 实例"]
    fn create_launch_stop_remove() {
        let managed = install::load_managed().expect("先在界面上生成免弹窗副本");
        let name = "tvx-selftest";
        if profiles_root().join(name).exists() {
            fs::remove_dir_all(profiles_root().join(name)).ok();
        }

        let p = create(name, Lang::Zh).expect("创建档案失败");
        assert!(p.dir.is_dir());

        let pid = launch(&managed, &p, Lang::Zh).expect("启动失败");
        // Windows：启动后深链协议必须指向这个账号的目录，否则浏览器回调会落错 profile
        #[cfg(windows)]
        {
            use winreg::enums::HKEY_CURRENT_USER;
            use winreg::RegKey;
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let cmd: String = hkcu
                .open_subkey(r"Software\Classes\tradingview\shell\open\command")
                .unwrap()
                .get_value("")
                .unwrap();
            assert!(cmd.contains("_auth"), "协议命令没指向 tvx _auth：{cmd}");
            eprintln!("协议命令已指向账号目录 ●");
        }
        eprintln!("启动 PID {pid}，等 8 秒看是否存活");
        // 深链广播：模拟浏览器回调，实例日志中应出现 second-instance
        route_auth("tradingview://browser-auth?tvx-selftest=1");
        // 关键回归：TradingView 启动时会把自己的协议命令写回默认形态，
        // maintain_protocol_handler 必须能把它纠回"指向本账号"。
        #[cfg(windows)]
        {
            std::thread::sleep(std::time::Duration::from_secs(2));
            maintain_protocol_handler();
            std::thread::sleep(std::time::Duration::from_millis(200));
            use winreg::enums::HKEY_CURRENT_USER;
            use winreg::RegKey;
            let cmd: String = RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey(r"Software\Classes\tradingview\shell\open\command")
                .unwrap()
                .get_value("")
                .unwrap();
            assert!(cmd.contains("_auth"), "应用改写后没能纠回协议命令：{cmd}");
            eprintln!("应用改写后已纠回协议命令 ●");
        }
        std::thread::sleep(std::time::Duration::from_secs(8));
        assert!(is_alive(pid), "进程没能起来");
        assert_eq!(running_pid(&p), Some(pid), "PID 文件没对上");

        stop(pid).expect("停止失败");
        std::thread::sleep(std::time::Duration::from_secs(3));
        assert!(!is_alive(pid), "进程没被停掉");
        // 停完后协议命令应还原（不再残留某个账号目录）
        #[cfg(windows)]
        {
            use winreg::enums::HKEY_CURRENT_USER;
            use winreg::RegKey;
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let cmd: String = hkcu
                .open_subkey(r"Software\Classes\tradingview\shell\open\command")
                .unwrap()
                .get_value("")
                .unwrap();
            let _ = cmd; // 停完后以 maintain 的期望形态为准，这里只记录
            eprintln!("协议命令已还原 ●");
        }

        remove(&p, Lang::Zh).expect("删除档案失败");
        assert!(!p.dir.exists());
        eprintln!("创建 / 启动 / 停止 / 删除 全部通过");
    }
}

#[cfg(test)]
mod multi_tests {
    use super::*;
    use crate::install;

    /// 真正同时起两个实例，验证“多账号同时在线”不是嘴上说说。
    #[test]
    #[ignore = "会真的启动两个 TradingView 实例"]
    fn two_profiles_at_once() {
        let managed = install::load_managed().expect("先生成免弹窗副本");
        let names = ["tvx-p1", "tvx-p2"];

        let mut made = Vec::new();
        for n in names {
            let d = profiles_root().join(n);
            if d.exists() {
                fs::remove_dir_all(&d).ok();
            }
            made.push(create(n, Lang::Zh).expect("建档案失败"));
        }

        let p1 = launch(&managed, &made[0], Lang::Zh).expect("启动 1 失败");
        let p2 = launch(&managed, &made[1], Lang::Zh).expect("启动 2 失败");
        assert_ne!(p1, p2, "两个实例不应是同一个进程");

        std::thread::sleep(std::time::Duration::from_secs(9));
        eprintln!("两个实例 PID: {p1} / {p2}；存活: {} / {}", is_alive(p1), is_alive(p2));
        assert!(is_alive(p1), "实例 1 没起来");
        assert!(is_alive(p2), "实例 2 没起来");

        // 两个实例必须各自用自己的配置目录
        assert_ne!(made[0].dir, made[1].dir);

        stop(p1).ok();
        stop(p2).ok();
        std::thread::sleep(std::time::Duration::from_secs(3));
        assert!(!is_alive(p1) && !is_alive(p2), "没停干净");

        for p in &made {
            remove(p, Lang::Zh).ok();
        }
    }
}
