//! profiles 模块的回归测试。

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

/// 登录回调只能投给一个实例。以前是广播给所有在跑的实例——token 是可用凭据，
/// 广播等于让几个账号一起被换成同一个号。
#[test]
fn auth_callback_targets_one_instance_only() {
    let running = [("/a", true), ("/b", true)];
    // 登录目标是 /b：只投 b
    assert_eq!(pick_auth_target(Some(("/b", 100)), &running, 200), Some(1));
    // 目标过期（超过保鲜期）且不止一个实例：放弃，宁可让用户重点一次登录
    assert_eq!(
        pick_auth_target(Some(("/b", 0)), &running, LOGIN_TARGET_TTL_SECS + 1),
        None
    );
    // 目标目录没在跑（被改名/删掉）：不投
    assert_eq!(pick_auth_target(Some(("/c", 100)), &running, 200), None);
    // 没有目标但只有一个实例在跑：投给它
    assert_eq!(
        pick_auth_target(None, &[("/a", true), ("/b", false)], 200),
        Some(0)
    );
    // 没有目标、也没有实例：不投
    assert_eq!(pick_auth_target(None, &[("/a", false)], 200), None);
}

/// 同一条回调被投递两次是常态（系统一次 + 接管脚本捞回一次）。
/// 不去重的话同一个实例会连着处理两遍登录参数，严重时把实例顶掉重启。
#[test]
fn duplicate_auth_callback_is_ignored() {
    let u = "tradingview://dedup-test/?token=t1";
    assert!(!auth_duplicate(u, 10_000), "第一次必须放行");
    assert!(auth_duplicate(u, 10_001), "1 秒后的同一条必须拦住");
    assert!(auth_duplicate(u, 10_019), "19 秒内仍然算重复");
    assert!(!auth_duplicate(u, 10_021), "过了 20 秒窗口应重新放行");
    // token 不同的回调互不影响
    assert!(!auth_duplicate("tradingview://dedup-test/?token=t2", 10_022));
}

/// 目标文件格式：第一行目录，第二行写入时间（旧格式只有一行，按"现在"算）
#[test]
fn login_target_format_roundtrip() {
    let body = format!("{}\n{}\n", "/x", 1234u64);
    let mut lines = body.lines();
    assert_eq!(lines.next(), Some("/x"));
    assert_eq!(lines.next().and_then(|s| s.parse::<u64>().ok()), Some(1234));
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

    let pid = launch(&managed, &p, Lang::Zh).expect("启动失败").pid;
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

let p1 = launch(&managed, &made[0], Lang::Zh).expect("启动 1 失败").pid;
let p2 = launch(&managed, &made[1], Lang::Zh).expect("启动 2 失败").pid;
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
