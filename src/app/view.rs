//! 界面渲染：顶部状态条 + 功能开关、日志区、账号卡片。
//!
//! 三条约定：
//! 1. 面板只读 `View` 快照，不直接读 `self` 的盘点字段——画的时候数据是冻住的；
//! 2. 一切操作先收集成 `Act`，由调用方在本帧末尾统一执行（见 `actions.rs`）；
//! 3. 文案全部走 `i18n`，不在这里写死任何一句话。

use super::*;
use crate::i18n::S;
use eframe::egui;
use std::path::PathBuf;

/// 一帧的只读快照。字段与面板用到的局部变量一一对应。
pub(crate) struct View {
    pub(crate) t: &'static S,
    pub(crate) install_labels: Vec<String>,
    /// (名字, PID, 目录, 绑定的账号, 与谁重复)
    pub(crate) rows: Vec<(String, Option<u32>, PathBuf, Option<String>, Option<String>)>,
    pub(crate) online: usize,
    pub(crate) has_profile: bool,
    pub(crate) managed_ok: bool,
    pub(crate) managed_ver: Option<String>,
    pub(crate) newer: Option<(usize, String, bool)>,
    pub(crate) patched: bool,
    pub(crate) stale: bool,
    pub(crate) build_src_unverified: Option<String>,
    pub(crate) can_build: bool,
}


mod accounts;
mod head;

impl App {
    /// 把这一帧要画的东西算成快照（进程状态、副本健康、构建门禁都在这里定稿）。
    pub(super) fn view(&self) -> View {
        let t = self.cfg.lang.s();
        // 先把要画的数据摆成快照，避免面板闭包里反复借 self
        let install_labels: Vec<String> = self.installs.iter().map(|i| i.label(t)).collect();
        // 同账号查重：一个 TradingView 账号同时只应有一个档案在线（服务端按账号限连接）
        let mut owners: HashMap<String, Vec<String>> = HashMap::new();
        for p in &self.profiles {
            if let Some(uid) = profiles::account_id(p) {
                owners.entry(uid).or_default().push(p.name.clone());
            }
        }
        // (名字, PID, 目录, 绑定的账号, 与谁重复)
        let rows: Vec<(String, Option<u32>, PathBuf, Option<String>, Option<String>)> = self
            .profiles
            .iter()
            .map(|p| {
                let uid = profiles::account_id(p);
                let dup = uid.as_ref().and_then(|u| {
                    owners.get(u).filter(|v| v.len() > 1).map(|v| {
                        v.iter()
                            .filter(|n| *n != &p.name)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("、")
                    })
                });
                (
                    p.name.clone(),
                    self.status.get(&p.name).copied().flatten(),
                    p.dir.clone(),
                    uid,
                    dup,
                )
            })
            .collect();
        let online = rows.iter().filter(|r| r.1.is_some()).count();
        let has_profile = !rows.is_empty();
        let managed_ok = self.managed.is_some();
        let managed_ver = self.managed.as_ref().map(|m| m.version.clone());
        // 客户端升级后副本还停在旧版：这里只做提示，不自动重建（副本 200MB+，
        // 而且新旧版本可能同时在用，删谁必须由人决定）。
        // 第三项 = 该新版是否在白名单里，决定提示语气与是否允许一键重建。
        let newer: Option<(usize, String, bool)> = managed_ver.as_deref().and_then(|cur| {
            let hit = install::newest_than(&self.installs, cur)?;
            let idx = self.installs.iter().position(|i| i.root == hit.root)?;
            Some((idx, hit.version.clone(), patch::is_verified(&hit.version)))
        });
        // 判定「可用」必须同时看两件事：index.js 里的补丁齐不齐，以及 app.asar 有没有残留。
        // asar 残留时 Electron 优先加载它，补丁写了也不会执行——只看 is_patched 会误报"正常"。
        let patched = self
            .managed
            .as_ref()
            .map(|m| m.is_patched(&self.cfg.flags()) && !m.asar_residue())
            .unwrap_or(false);
        // 旧包残留要单独说：这时"补丁异常"的措辞会误导——文件里补丁是齐的，只是没被执行。
        let stale = self
            .managed
            .as_ref()
            .map(|m| m.asar_residue())
            .unwrap_or(false);

        // 本次若真要构建副本，源是哪个版本、是否已验证。没副本时源 = 选中的安装，
        // 有副本时只有「按新版重建」会触发构建——两种情况分开算。
        let sel_ver = self.installs.get(self.selected).map(|i| i.version.as_str());
        let build_src_unverified = unverified_target(
            managed_ok,
            sel_ver,
            newer.as_ref().map(|(_, v, ok)| (v.as_str(), *ok)),
        );
        let can_build = build_allowed(build_src_unverified.as_deref(), self.allow_unverified);

        View {
            t,
            install_labels,
            rows,
            online,
            has_profile,
            managed_ok,
            managed_ver,
            newer,
            patched,
            stale,
            build_src_unverified,
            can_build,
        }
    }

pub(super) fn log_panel(&mut self, ctx: &egui::Context, log: &[String]) {
    let t = self.cfg.lang.s();
    // ------------------------------------------------------------ 日志
    egui::TopBottomPanel::bottom("log")
        .resizable(true)
        .default_height(100.0)
        .min_height(48.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t.log).small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(t.clear).clicked() {
                        if let Ok(mut s) = self.shared.lock() {
                            s.log.clear();
                        }
                    }
                });
            });
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if log.is_empty() {
                        ui.label(egui::RichText::new("—").small().weak());
                    }
                    for line in log {
                        let mut rt = egui::RichText::new(line).monospace().small();
                        match line.chars().next() {
                            Some('●') => rt = rt.color(GREEN),
                            Some('×') => rt = rt.color(RED),
                            Some('⚠') => rt = rt.color(AMBER),
                            _ => {}
                        }
                        ui.label(rt);
                    }
                });
        });

}
}
