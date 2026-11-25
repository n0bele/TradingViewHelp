//! 动作执行：面板只收集 `Act`，真正的副作用集中在这里。
//!
//! 统一入口的好处是"按钮能点"与"点了会做什么"分离，且每个动作都能在
//! 后台线程里跑（`run_job`）而不阻塞 UI。

use super::{source_name, Act, App};
use crate::app::state::{run_job, STAGGER_SECS};
use crate::i18n::Lang;
use crate::install;
use crate::patch;
use crate::profiles::{self, Profile};
use eframe::egui;
use std::time::{Duration, Instant};

impl App {
    pub(crate) fn apply(&mut self, ctx: &egui::Context, act: Act) {
        let lang = self.cfg.lang;
        match act {
            Act::Doctor => {
                self.env_warn = crate::install::environment_problems();
                let report = crate::install::doctor_report();
                // 报告有几十行，用户要的是"到底好没好"。先给一行结论，再铺详情。
                let t = lang.s();
                let flags = self.cfg.flags();
                let verdict = match self.managed.as_ref() {
                    None => t.copy_none.to_string(),
                    Some(m) if m.asar_residue() => t.copy_stale.replace("{v}", &m.version),
                    Some(m) => {
                        let failed = patch::failing_patches(&m.index_js(), &flags);
                        if failed.is_empty() {
                            lang.log_health_ok(
                                env!("CARGO_PKG_VERSION"),
                                &m.version,
                                patch::PATCHES.len(),
                                patch::PATCHES.len(),
                            )
                        } else {
                            lang.log_patch_partial(&failed)
                        }
                    }
                };
                self.log(verdict);
                for line in report.lines() {
                    self.log(line.to_string());
                }
                // 报告也落盘一份：截图/日志区会被刷掉，文件不会
                let path = crate::install::tvx_root().join("doctor.txt");
                if let Err(e) = std::fs::write(&path, &report) {
                    self.log(format!("自检报告写入失败：{e}"));
                } else {
                    self.log(format!("自检报告已保存：{}", path.display()));
                }
            }
            Act::Launch(name) => {
                let Some(m) = self.managed.clone() else {
                    self.log(lang.log_need_copy());
                    return;
                };
                // 上限按"在跑的实例数"算：每个实例都是一整棵进程树
                let running = profiles::running_count();
                if running >= profiles::MAX_RUNNING {
                    self.log(lang.log_running_limit(running, profiles::MAX_RUNNING));
                    return;
                }
                if let Some(p) = self.profiles.iter().find(|p| p.name == name).cloned() {
                    match profiles::launch(&m, &p, lang) {
                        Ok(l) => {
                            let pid = l.pid;
                            self.launching.insert(p.name.clone(), Instant::now());
                            self.log(lang.log_launched(&p.name, pid));
                            if let Some(w) = l.warning {
                                self.log(w);
                            }
                            // 约 10 秒后（页面已把登录态落盘）回填账号：
                            // 本功能之前创建的档案没有绑定记录，只能这样认一次。
                            let p2 = p.clone();
                            let sh = self.shared.clone();
                            let c2 = ctx.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_secs(10));
                                if let Some((uid, dup)) = profiles::refresh_account(&p2) {
                                    if let Ok(mut st) = sh.lock() {
                                        st.log.push(lang.log_account_bound(&p2.name, &uid));
                                        if let Some(other) = dup {
                                            st.log.push(lang.log_account_same(
                                                &p2.name, &other, &uid,
                                            ));
                                        }
                                    }
                                    c2.request_repaint();
                                }
                            });
                        }
                        Err(e) => self.log(lang.log_launch_failed(&format!("{e:#}"))),
                    }
                }
                if let Some(addr) = profiles::proxy_unreachable() {
                    self.log(lang.log_proxy_down(&addr));
                }
                self.wake_scanner();
            }
            Act::Stop(pid) => {
                match profiles::stop(pid) {
                    Ok(()) => self.log(lang.log_stopped(pid)),
                    Err(e) => self.log(lang.log_stop_failed(&format!("{e:#}"))),
                }
                self.wake_scanner();
            }
            Act::LaunchAll => {
                let Some(m) = self.managed.clone() else {
                    return;
                };
                let running = profiles::running_count();
                let budget = profiles::MAX_RUNNING.saturating_sub(running);
                if budget == 0 {
                    self.log(lang.log_running_limit(running, profiles::MAX_RUNNING));
                    return;
                }
                let mut todo: Vec<Profile> = self
                    .profiles
                    .iter()
                    .filter(|p| self.status.get(&p.name).copied().flatten().is_none())
                    .cloned()
                    .collect();
                if todo.is_empty() {
                    return;
                }
                if todo.len() > budget {
                    todo.truncate(budget);
                    self.log(lang.log_launch_budget(todo.len(), running, profiles::MAX_RUNNING));
                }
                // 错峰启动：逐个间隔 STAGGER_SECS 秒，避免多个实例在同一瞬间一起建连
                // （服务端看到的是同一账号/同一出口的建连风暴，更容易被判超限）。
                run_job(
                    ctx,
                    self.shared.clone(),
                    false,
                    Box::new(move |push| {
                        push(lang.log_stagger(todo.len(), STAGGER_SECS));
                        let mut started: Vec<Profile> = Vec::new();
                        for (i, p) in todo.iter().enumerate() {
                            if i > 0 {
                                std::thread::sleep(Duration::from_secs(STAGGER_SECS));
                            }
                            match profiles::launch(&m, p, lang) {
                                Ok(l) => {
                                    started.push(p.clone());
                                    push(lang.log_launching(&p.name, l.pid));
                                    if let Some(w) = l.warning {
                                        push(w);
                                    }
                                }
                                Err(e) => {
                                    push(lang.log_launch_one_failed(&p.name, &format!("{e:#}")))
                                }
                            }
                        }
                        let n = started.len();
                        if n > 0 {
                            // 等页面把登录态落盘后统一回填一次账号，认出同账号的档案
                            std::thread::sleep(Duration::from_secs(8));
                            for p in &started {
                                if let Some((uid, dup)) = profiles::refresh_account(p) {
                                    push(lang.log_account_bound(&p.name, &uid));
                                    if let Some(other) = dup {
                                        push(lang.log_account_same(&p.name, &other, &uid));
                                    }
                                }
                            }
                        }
                        Ok(lang.log_launched_all(n))
                    }),
                );
            }
            Act::StopAll => {
                let pids: Vec<u32> = self.status.values().flatten().copied().collect();
                for pid in &pids {
                    let _ = profiles::stop(*pid);
                }
                if !pids.is_empty() {
                    self.log(lang.log_stopped_all(pids.len()));
                }
                self.wake_scanner();
            }
            Act::Delete(name) => {
                if let Some(p) = self.profiles.iter().find(|p| p.name == name).cloned() {
                    match profiles::remove(&p, lang) {
                        Ok(()) => {
                            self.log(lang.log_deleted(&name));
                            // 立刻从界面移除，不等下一轮后台扫描
                            self.profiles.retain(|x| x.name != name);
                            self.status.remove(&name);
                        }
                        Err(e) => self.log(lang.log_delete_failed(&format!("{e:#}"))),
                    }
                }
                self.wake_scanner();
            }
            Act::Rename(from, to) => {
                if let Some(p) = self.profiles.iter().find(|p| p.name == from).cloned() {
                    match profiles::rename(&p, &to, lang) {
                        Ok(np) => {
                            self.log(lang.log_renamed(&p.name, &np.name));
                            // 运行态按名字索引，改名后要跟着搬，否则卡片状态会错位
                            if let Some(st) = self.status.remove(&p.name) {
                                self.status.insert(np.name.clone(), st);
                            }
                            self.launching.remove(&p.name);
                        }
                        Err(e) => self.log(lang.log_rename_failed(&format!("{e:#}"))),
                    }
                }
                self.wake_scanner();
            }
            Act::ReapplyPatches => {
                let Some(m) = self.managed.clone() else {
                    return;
                };
                let flags = self.cfg.flags();
                run_job(
                    ctx,
                    self.shared.clone(),
                    false,
                    Box::new(move |push| {
                        // 拨开关 = 一次修复机会：残留旧包在这里顺手清掉，
                        // 用户不用为了一个文件去重建整个副本。
                        match m.clear_asar_residue() {
                            Ok(true) => push(lang.log_stale_cleared(
                                &m.resources().join("app.asar").display().to_string(),
                            )),
                            Ok(false) => {}
                            Err(_) => push(lang.log_stale_locked(
                                &m.resources().join("app.asar").display().to_string(),
                            )),
                        }
                        let report = patch::apply(&m.index_js(), &flags)?;
                        if let Some(dir) = m.index_js().parent() {
                            for w in patch::install_assets(dir, &flags)? {
                                push(format!("  ! {w}"));
                            }
                        }
                        push(lang.log_switches(
                            flags.adblock,
                            flags.charts_unlock,
                            flags.study_on_study,
                            flags.drawings_unlock,
                            flags.no_auto_update,
                        ));
                        for &(name, st) in &report.items {
                            let shown: &str = if lang == Lang::En {
                                patch::name_en(name)
                            } else {
                                name
                            };
                            push(format!("  [{}] {shown}", st.mark()));
                        }
                        if !report.failed.is_empty() {
                            push(lang.log_patch_partial(&report.failed));
                        }
                        Ok(lang.log_reapplied())
                    }),
                );
            }
            Act::RebuildCopy(idx) => {
                let Some(inst) = self.installs.get(idx).cloned() else {
                    return;
                };
                if !patch::is_verified(&inst.version) && !self.allow_unverified {
                    self.log(lang.e_unverified_blocked(&source_name(&inst.version, lang)));
                    return;
                }
                let Some(old) = self.managed.as_ref().map(|m| m.version.clone()) else {
                    return;
                };
                let flags = self.cfg.flags();
                let dest = install::dest_for(&inst);
                run_job(
                    ctx,
                    self.shared.clone(),
                    true,
                    Box::new(move |push| {
                        push(lang.log_rebuild_start(&inst.version));
                        let m = install::build(&inst, &dest, &flags, lang, &mut *push)?;
                        // 锁定新版本：多版本目录共存时，后续启动都走这一份
                        let _ = install::set_active_app(&m.version);
                        Ok(lang.log_rebuilt(&m.version, &old))
                    }),
                );
            }
            Act::SubmitAdd => self.submit_add(),
        }
        ctx.request_repaint();
    }
}
