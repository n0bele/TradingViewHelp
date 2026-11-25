//! 无头命令：不开界面也能维护副本（`_reapply` / `_build`）与打开目录。
//!
//! 这些函数只依赖 install / patch，不碰 egui 状态——放在这里是为了让
//! `main.rs` 的 CLI 分支与界面走同一份实现，而不是各写一遍。

use crate::i18n::Lang;
use crate::install;
use crate::patch;
use std::path::Path;

pub fn open_path(p: &Path) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(p).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(p).spawn();
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}

/// 无头维护模式：`tvx _reapply` —— 按当前配置开关重打托管副本的补丁。
/// 与 GUI 里拨动任意开关触发的 `Act::ReapplyPatches` 走同一逻辑，
/// 供脚本化 / 远程维护使用，不启动界面。
pub fn reapply_headless() {
    let cfg = install::load_config();
    let lang = cfg.lang;
    let Some(m) = install::load_managed() else {
        println!("{}", lang.log_no_copy_headless());
        return;
    };
    // 先清残留旧包：它在的时候补丁写了也永不执行。删不掉不阻断（补丁打到 index.js 无害），
    // 但必须把"为什么没生效、该做什么"说清楚。
    match m.clear_asar_residue() {
        Ok(true) => println!(
            "{}",
            lang.log_stale_cleared(&m.resources().join("app.asar").display().to_string())
        ),
        Ok(false) => {}
        Err(_) => eprintln!(
            "{}",
            lang.log_stale_locked(&m.resources().join("app.asar").display().to_string())
        ),
    }
    match patch::apply(&m.index_js(), &cfg.flags()) {
        Ok(report) => {
            for &(name, st) in &report.items {
                let shown: &str = if lang == Lang::En {
                    patch::name_en(name)
                } else {
                    name
                };
                println!("[{}] {shown}", st.mark());
            }
            if !report.failed.is_empty() {
                eprintln!("{}", lang.log_patch_partial(&report.failed));
            }
            // 报警功能的资源文件（引擎 / 主进程模块 / preload 桥）随开关一起落盘或清除
            if let Some(dir) = m.index_js().parent() {
                match patch::install_assets(dir, &cfg.flags()) {
                    Ok(ws) => {
                        for w in ws {
                            eprintln!("! {w}");
                        }
                    }
                    Err(e) => eprintln!("× 报警资源写入失败：{e:#}"),
                }
            }
            println!(
                "{}",
                lang.log_reapplied_headless(&m.index_js().display().to_string())
            );
        }
        Err(e) => eprintln!("× {e:#}"),
    }
}

/// 无头构建：`tvhelper _build` —— 从系统安装（优先已验证版本）构建/重建托管副本。
/// 与界面上的「构建副本」走同一条 install::build：客户端升级后不用点鼠标也能重建。
pub fn build_headless() {
    let cfg = install::load_config();
    let lang = cfg.lang;
    let installs = install::discover();
    let Some(idx) = install::preferred_index(&installs) else {
        println!("{}", lang.log_no_install_headless());
        return;
    };
    let src = &installs[idx];
    let dest = install::dest_for(src);
    let mut log = |m: String| println!("{m}");
    match install::build(src, &dest, &cfg.flags(), lang, &mut log) {
        Ok(m) => println!(
            "{}",
            lang.log_built_headless(&m.version, &m.dir.display().to_string())
        ),
        Err(e) => eprintln!("× {e:#}"),
    }
}

