//! 把 assets/icon.ico 作为 exe 图标嵌入（仅 Windows）。
//! 直接调用系统 Windows SDK 的 rc.exe，不引入额外 crate 依赖。
//! 找不到 rc.exe 时静默跳过——图标缺失不影响功能，也不该让开源项目在别人机器上编译失败。

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.rc");

    if !cfg!(target_os = "windows") {
        return;
    }

    let out = std::env::var("OUT_DIR").unwrap_or_default();
    let res = format!("{}/tvhelper_icon.res", out);

    // 找 rc.exe：优先 PATH，其次常见的 Windows Kits 安装位置
    let rc = find_rc().unwrap_or_default();
    if rc.is_empty() {
        println!("cargo:warning=未找到 Windows SDK 的 rc.exe，跳过 exe 图标嵌入（窗口图标仍生效）");
        return;
    }

    let status = std::process::Command::new(&rc)
        .args(["/nologo", "/fo", &res, "assets/icon.rc"])
        .status();
    match status {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg={}", res),
        _ => println!("cargo:warning=rc.exe 编译图标资源失败，跳过 exe 图标嵌入"),
    }
}

fn find_rc() -> Option<String> {
    // 1) PATH 里的 rc.exe
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(';') {
            let p = std::path::Path::new(dir.trim()).join("rc.exe");
            if p.is_file() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    // 2) Program Files(x86) 下的 Windows Kits
    let pf = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
    let kits = std::path::Path::new(&pf).join("Windows Kits\\10\\bin");
    if let Ok(vers) = std::fs::read_dir(&kits) {
        // 取版本号最大的 x64 目录
        let mut cands: Vec<_> = vers
            .flatten()
            .map(|e| e.path().join("x64\\rc.exe"))
            .filter(|p| p.is_file())
            .collect();
        cands.sort();
        if let Some(p) = cands.last() {
            return Some(p.to_string_lossy().to_string());
        }
    }
    None
}
