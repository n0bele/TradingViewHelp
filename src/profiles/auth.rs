//! 登录回调：深链路由、应用内标签页、登录目标选择，以及被商店版抢走回调时的接管。

use super::*;

/// 登录目标的保鲜期：超过这个时间还没收到回调，就认为那次登录已经放弃了。
/// 过期后不再按它路由，避免几天前的残留把今天的回调带到别的窗口。
pub(crate) const LOGIN_TARGET_TTL_SECS: u64 = 30 * 60;

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 记录"当前登录目标账号"（协议回调按它决定投给哪个实例）
pub(crate) fn write_login_target(profile: &Profile) {
    let path = crate::install::tvx_root().join("login-target");
    let body = format!("{}\n{}\n", profile.dir.display(), now_secs());
    if let Err(e) = crate::install::write_atomic(&path, body.as_bytes()) {
        // 写不上只影响深链路由的纠偏，不该阻断启动；但要有痕迹可查
        crate::install::log_error("写登录目标 login-target", &e);
    }
}

/// 读登录目标：账号目录 + 写入时间。旧格式只有一行目录，时间按"现在"算。
pub(crate) fn read_login_target() -> Option<(String, u64)> {
    let raw = fs::read_to_string(crate::install::tvx_root().join("login-target")).ok()?;
    let mut lines = raw.lines();
    let dir = lines.next()?.trim().to_string();
    if dir.is_empty() {
        return None;
    }
    let ts = lines
        .next()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or_else(now_secs);
    Some((dir, ts))
}

/// 回调该投给谁。`running` 是 (账号目录, 是否在跑)。
///
/// 只投给"最近一次登录目标"，绝不广播：回调 URL 里带着可用的登录 token，
/// 广播给所有在跑的实例等于让几个账号同时被换成同一个账号。
pub(crate) fn pick_auth_target(
    target: Option<(&str, u64)>,
    running: &[(&str, bool)],
    now: u64,
) -> Option<usize> {
    if let Some((dir, ts)) = target {
        if now.saturating_sub(ts) <= LOGIN_TARGET_TTL_SECS {
            if let Some(i) = running.iter().position(|(d, alive)| *d == dir && *alive) {
                return Some(i);
            }
        }
    }
    // 目标缺失或过期：只有在跑的实例恰好一个时才投，多个就放弃（宁可让用户重点一次登录）
    let mut alive = running.iter().enumerate().filter(|(_, (_, a))| *a);
    match (alive.next(), alive.next()) {
        (Some((i, _)), None) => Some(i),
        _ => None,
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
    let Some((target, ts)) = read_login_target() else {
        return;
    };
    // 过期的登录目标不再维持：否则几天后有个实例刚好叫这个名字且还开着，
    // 协议命令就被重新指回来，用户手动改过的默认处理程序会被悄悄覆盖。
    if now_secs().saturating_sub(ts) > LOGIN_TARGET_TTL_SECS {
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
pub(crate) fn current_protocol_points_to_tvx() -> bool {
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
pub(crate) fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}Z")
}

/// 同一条回调在极短时间内被投递两次是很常见的：系统协议投递一次，
/// 接管脚本从被商店版截走的进程里再捞一次。两次都转发会让同一个实例
/// 连着处理两遍登录参数（重复开标签页、把登录态写两遍，严重时把实例顶掉）。
///
/// 只按完整 URL（含 token）做短时去重：不同账号的 token 必然不同，不会互相顶。
pub(crate) fn auth_duplicate(url: &str, now: u64) -> bool {
    use std::sync::{Mutex, OnceLock};

    static LAST: OnceLock<Mutex<Option<(String, u64)>>> = OnceLock::new();
    /// 窗口取 20 秒：够盖住"系统投递 + 接管捞回"的间隔，又不至于挡住用户
    /// 立刻重试同一条链接。
    const WINDOW_SECS: u64 = 20;

    let mutex = LAST.get_or_init(|| Mutex::new(None));
    let mut last = match mutex.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    if let Some((prev, at)) = last.as_ref() {
        if prev == url && now.saturating_sub(*at) < WINDOW_SECS {
            return true;
        }
    }
    *last = Some((url.to_string(), now));
    false
}

/// 深链分发：系统把 `tradingview://` 交给 `tvx _auth`，这里广播给所有在跑的实例。
///
/// 每个实例用自己的 `--user-data-dir` 起一个短命进程转发 URL——Electron 的单实例锁
/// 会把参数投递给该 profile 的主进程；正在等登录的那个实例会处理，其余会忽略。
pub fn route_auth(url: &str) {
    auth_log(&format!("收到回调：{}", mask_token(url)));
    if auth_duplicate(url, now_secs()) {
        auth_log("  同一条回调刚处理过，忽略这次重复投递");
        return;
    }

    // 回调 URL 里的 token 是本次登录的凭据：解出 user_id 就能把"这个档案登录了哪个账号"
    // 记下来。只能记到**一个**档案头上——广播式投递会让几个账号一起被换成同一个号。
    let uid = jwt_user_id(url);
    let target = read_login_target();

    let Some(managed) = crate::install::load_managed() else {
        return;
    };
    let Some(exe) = managed.exe() else {
        return;
    };

    let profiles = list();
    let running: Vec<(String, bool)> = profiles
        .iter()
        .map(|p| {
            let alive = running_pid(p).is_some();
            if !alive {
                auth_log(&format!("  跳过 {}（没有实例在跑）", p.name));
            }
            (p.dir.display().to_string(), alive)
        })
        .collect();
    let running_ref: Vec<(&str, bool)> = running
        .iter()
        .map(|(d, a)| (d.as_str(), *a))
        .collect();

    let picked = pick_auth_target(
        target.as_ref().map(|(d, t)| (d.as_str(), *t)),
        &running_ref,
        now_secs(),
    );
    let Some(idx) = picked else {
        auth_log(
            "  没有唯一可投递的目标（多个实例在跑，但没有新鲜的登录目标）：\
             已丢弃本次回调，避免把登录态发给全部实例。请在要登录的那个窗口里重新点一次登录",
        );
        return;
    };

    let p = &profiles[idx];
    if let (Some(uid), Some((dir, _))) = (&uid, target.as_ref()) {
        if dir == &running[idx].0 {
            let _ = set_account_id(p, uid);
            auth_log(&format!("  账号 {uid} 已绑定到 {}", p.name));
            let others = same_account_others(p, uid);
            if let Some((name, _)) = others.iter().find(|(_, running)| *running) {
                auth_log(&format!(
                    "  ⚠ 同一账号也在「{name}」里运行：同账号多开可能撞服务端并发限制"
                ));
            } else if let Some((name, _)) = others.first() {
                auth_log(&format!("  ⚠ 该账号也登录过「{name}」：建议一个账号只保留一个档案"));
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

/// 日志里不落完整 token（那是可用的登录凭据）。
pub(crate) fn mask_token(url: &str) -> String {
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
pub(crate) fn handoff_skip_path() -> PathBuf {
    tvx_root().join("handoff-skip")
}

pub(crate) fn record_spawned(pid: u32) {
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
pub(crate) fn capture_handoff(secs: u32) -> Option<(u32, String)> {
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
pub(crate) fn capture_handoff(secs: u32) -> Option<(u32, String)> {
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
pub(crate) fn capture_handoff(secs: u32) -> Option<(u32, String)> {
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
pub(crate) static HANDOFF_WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 这个 pid 是不是我们正在用的实例（按 PID 文件校验过启动时间）。
///
/// 接管脚本的判据只有"命令行里有 `tradingview://browser-auth`"。用户从浏览器
/// 点登录时，系统会**带着这个 URL 再拉起一个 TradingView.exe**——如果它在抢单实例
/// 锁之前被当成"被商店版截走的转发进程"杀掉，用户看到的就是客户端莫名其妙退出、
/// 再被拉起来、又被杀掉，也就是"一直在重启"。
pub(crate) fn is_managed_instance(pid: u32) -> bool {
    list().into_iter().any(|p| running_pid(&p) == Some(pid))
}

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
                if is_managed_instance(pid) {
                    auth_log("  该进程是我们正在用的实例，只转发不结束");
                } else {
                    let _ = stop(pid);
                }
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
        if *pid != 0 && !is_managed_instance(*pid) {
            let _ = stop(*pid);
        }
    }
    got
}

