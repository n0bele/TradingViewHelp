//! 构建托管副本：复制 → 解包 asar → 打补丁 →（macOS）重签名。
//!
//! 一切都在暂存目录里完成，成功才替换正式目录——中途失败不毁用户已有的副本。

use super::*;

/// 复制 → 解包 asar → 按配置打补丁（macOS 再重签名）。
///
/// 先在旁边的暂存目录里完整构建，成功后才替换正式目录：复制几百 MB 的中途失败、
/// 或补丁写回失败，都只丢暂存目录，用户已有的副本原封不动。
pub fn build(
    src: &Install,
    dest: &Path,
    flags: &patch::Flags,
    lang: Lang,
    log: &mut dyn FnMut(String),
) -> Result<Managed> {
    if !src.looks_valid() {
        bail!(lang.b_incomplete(&src.root.display().to_string()));
    }

    // 未在白名单里的版本：先明说"锚点不是按它提的"，再让它尽力打。
    // 打不上的条目会在 build_into 里逐条标记失败，不会静默当成功。
    if !patch::is_verified(&src.version) {
        log(lang.b_unverified(&src.version, patch::VERIFIED_VERSION));
    }

    let staging = staging_dir(dest);
    // 上次中断留下的暂存目录先清掉（可能被占用；删不掉就让下面的构建自己报错）
    let _ = remove_dir_retry(&staging);

    if let Err(e) = build_into(src, &staging, flags, lang, log) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    if dest.exists() {
        log(lang.b_cleanup(&dest.display().to_string()));
        // 旧副本可能正被运行中的实例占着（Windows 会锁 exe），重试几次再放弃
        remove_dir_retry(dest)
            .with_context(|| lang.b_delete_failed(&dest.display().to_string()))?;
    }
    fs::rename(&staging, dest).with_context(|| {
        format!(
            "把构建好的副本从 {} 移到 {} 失败",
            staging.display(),
            dest.display()
        )
    })?;

    Ok(Managed {
        dir: dest.to_path_buf(),
        app: app_dir_in(dest),
        version: src.version.clone(),
    })
}

/// 托管副本里的可执行根：macOS 是 .app 外壳，其他平台就是目录本身。
fn app_dir_in(dest: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        dest.join("TradingView.app")
    }
    #[cfg(not(target_os = "macos"))]
    {
        dest.to_path_buf()
    }
}

/// 与 dest 同级的隐藏暂存目录（同一分区，最后的 rename 才是原子的）。
fn staging_dir(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "copy".into());
    dest.with_file_name(format!(".{name}.building"))
}

/// 删除目录；Windows 上遇到实例占用 / 杀软扫描时重试几次，而不是一次失败就放弃。
fn remove_dir_retry(dir: &Path) -> std::io::Result<()> {
    use std::time::Duration;
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..4u64 {
        match fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if !dir.exists() {
                    return Ok(());
                }
                last = Some(e);
                if attempt < 3 {
                    std::thread::sleep(Duration::from_millis(250 * (attempt + 1)));
                }
            }
        }
    }
    Err(last.expect("至少失败过一次"))
}

/// 删除单个文件；同 `remove_dir_retry`，Windows 上杀软扫描 / 句柄未释放会短暂占用。
pub(super) fn remove_file_retry(path: &Path) -> std::io::Result<()> {
    use std::time::Duration;
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..4u64 {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if !path.exists() {
                    return Ok(());
                }
                last = Some(e);
                if attempt < 3 {
                    std::thread::sleep(Duration::from_millis(250 * (attempt + 1)));
                }
            }
        }
    }
    Err(last.expect("至少失败过一次"))
}

fn build_into(
    src: &Install,
    dest: &Path,
    flags: &patch::Flags,
    lang: Lang,
    log: &mut dyn FnMut(String),
) -> Result<()> {
    // macOS 必须保留 .app 外壳，否则 Info.plist / 签名校验会出问题
    let app = app_dir_in(dest);

    log(lang.b_copying(
        &src.root.display().to_string(),
        &app.display().to_string(),
    ));
    let mut n = 0usize;
    copy_tree(&src.root, &app, &mut n, lang, log)?;
    log(lang.b_copied(n));

    let resources = {
        #[cfg(target_os = "macos")]
        {
            app.join("Contents").join("Resources")
        }
        #[cfg(not(target_os = "macos"))]
        {
            app.join("resources")
        }
    };
    let asar_path = resources.join("app.asar");
    let app_dir = resources.join("app");

    log(lang.b_unpack());
    let st = asar::extract(&asar_path, &app_dir)?;
    log(lang.b_unpacked(st.files, st.skipped_links));

    // 改成目录加载后，asar 与 unpacked 必须移除：Electron 的加载顺序是
    // `app.asar` 优先于 `app/` 目录。asar 只要还在，补丁就写进了永远不会被读的文件里，
    // 用户看到的是「一个功能都没有、也没有任何报错」，而界面读的是 app/index.js，
    // 徽标还会显示"补丁完整"。所以这里绝不能静默忽略删除失败。
    remove_file_retry(&asar_path).with_context(|| {
        format!(
            "删除 {} 失败（多半被杀软扫描或仍被实例占用）。\
             asar 残留时 Electron 会优先加载它，补丁不会生效，因此本次构建作废。\
             处理：先点「全部停止」关掉客户端（或在任务管理器结束 TradingView.exe），\
             或把该目录加入杀软白名单，然后重新点「生成 / 更新副本」",
            asar_path.display()
        )
    })?;
    if asar_path.exists() {
        bail!(format!(
            "{} 仍然存在，构建已中止（asar 残留会导致补丁全部失效）。\
             处理：关掉客户端后重试，或手动删除该文件；删不掉就加杀软白名单",
            asar_path.display()
        ));
    }
    let unpacked = PathBuf::from(format!("{}.unpacked", asar_path.to_string_lossy()));
    if unpacked.is_dir() {
        fs::remove_dir_all(&unpacked).ok();
    }

    log(lang.b_patching());
    // 补丁逐条应用：单条锚点失配只跳过该功能，不再让整次构建失败
    let applied = patch::apply(&app_dir.join("index.js"), flags)?;
    for &(name, st) in &applied.items {
        // + 打上 / = 已打过 / - 撤回 / · 未启用 / ! 未命中
        let shown: &str = if lang == Lang::En {
            patch::name_en(name)
        } else {
            name
        };
        log(format!("  [{}] {shown}", st.mark()));
    }
    for w in &applied.warnings {
        log(format!("  ! {w}"));
    }
    if !applied.failed.is_empty() {
        log(lang.log_patch_partial(&applied.failed));
    }
    // 报警功能是「文件 + 一处 require」，不只改 index.js，所以补丁之后还要落资源
    for w in patch::install_assets(&app_dir, flags)? {
        log(format!("  ! {w}"));
    }

    resign_if_needed(&app, lang, log)?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn resign_if_needed(app: &Path, lang: Lang, log: &mut dyn FnMut(String)) -> Result<()> {
    use std::process::Command;
    // 改过 app.asar 之后原签名必然失效，必须 ad-hoc 重签，否则 Gatekeeper 会拦
    let _ = Command::new("xattr").arg("-cr").arg(app).status();
    match Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(app)
        .status()
    {
        Ok(s) if s.success() => {
            log(lang.b_resigned());
            Ok(())
        }
        _ => {
            log(lang.b_resign_warn());
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn resign_if_needed(_app: &Path, _lang: Lang, _log: &mut dyn FnMut(String)) -> Result<()> {
    Ok(())
}

fn copy_tree(
    from: &Path,
    to: &Path,
    n: &mut usize,
    lang: Lang,
    log: &mut dyn FnMut(String),
) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from).with_context(|| format!("枚举 {}", from.display()))? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_tree(&src, &dst, n, lang, log)?;
        } else if ft.is_file() {
            fs::copy(&src, &dst).with_context(|| format!("复制 {}", src.display()))?;
            *n += 1;
            if *n % 40 == 0 {
                log(lang.b_progress(*n));
            }
        }
    }
    Ok(())
}
