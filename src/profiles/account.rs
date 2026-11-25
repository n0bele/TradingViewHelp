//! 账号身份：从浏览器授权 URL 的 JWT 或 Chromium 存储里认出 `user_id`。
//!
//! 用途只有一个——判重：同一个 TradingView 账号同时开两个实例会撞服务端的连接限制。

use super::*;

/// 账号目录里记录 TradingView `user_id` 的小文件（不是密码、也不是令牌）。
pub(crate) const UID_FILE: &str = ".tvx-user";

/// 该档案绑定的 TradingView 账号 ID（登录回调时写入）。
pub fn account_id(profile: &Profile) -> Option<String> {
    let raw = fs::read_to_string(profile.dir.join(UID_FILE)).ok()?;
    let raw = raw.trim();
    (!raw.is_empty()).then(|| raw.to_string())
}

/// 返回是否写成功：写不上就查不出"这个档案绑的是哪个账号"，同账号双开会撞服务端限制。
pub(crate) fn set_account_id(profile: &Profile, uid: &str) -> bool {
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
pub(crate) fn b64url_decode(s: &str) -> Option<Vec<u8>> {
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
pub(crate) fn detect_uid_from_storage(profile: &Profile) -> Option<String> {
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
pub(crate) fn scan_user_records(b: &[u8]) -> Vec<String> {
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

