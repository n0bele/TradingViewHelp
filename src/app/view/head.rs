//! 顶栏：身份与版本、环境告警、来源/副本状态徽标、功能开关、使用说明。

use super::View;
use crate::app::*;
use eframe::egui;

impl App {
    pub(crate) fn head_panel(
        &mut self,
        ctx: &egui::Context,
        view: &View,
        act: &mut Option<Act>,
        lang_changed: &mut bool,
    ) {
        let t = view.t;
        let install_labels = &view.install_labels;
        let patched = view.patched;
        let stale = view.stale;
        let can_build = view.can_build;
        // ------------------------------------------------------------ 顶部
        egui::TopBottomPanel::top("head").show(ctx, |ui| {
            ui.add_space(6.0);
            // 标题行：身份 + 语言切换
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t.app_title).heading().strong());
                // 自身版本号必须显示：桌面/下载目录里可能同时躺着几个 tvhelper.exe，
                // 用户跑错文件时只看界面根本分不出来（反馈也说不清是哪一版）。
                ui.label(
                    egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .small()
                        .weak(),
                );
                ui.label(egui::RichText::new(t.subtitle).small().weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("lang")
                        .selected_text(self.cfg.lang.label())
                        .width(112.0)
                        .show_ui(ui, |ui| {
                            for l in Lang::ALL {
                                if ui
                                    .selectable_value(&mut self.cfg.lang, l, l.label())
                                    .changed()
                                {
                                    *lang_changed = true;
                                }
                            }
                        });
                });
            });

            // 环境体检不通过时，顶栏常驻红字：这类问题不修，点「启动」也只会
            // 得到一个英文崩溃框（ENOENT mkdir …\TradingView\logs）。
            if !self.env_warn.is_empty() {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(RED, egui::RichText::new(t.env_warn).small().strong());
                    if ui.small_button(t.btn_doctor).clicked() {
                        *act = Some(Act::Doctor);
                    }
                });
                for p in self.env_warn.iter().take(3) {
                    ui.label(
                        egui::RichText::new(format!("• {}", p.detail))
                            .small()
                            .color(RED),
                    );
                }
            }

            ui.add_space(4.0);
            // 状态行：安装在哪儿 + 副本状态（版本 / 补丁 / 校验合成一个徽标），
            // 右侧只留一个主操作；打开目录、删除副本属于低频维护，收进折叠区
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t.installed_at).small().weak());
                if install_labels.is_empty() {
                    ui.colored_label(AMBER, t.no_tv);
                } else {
                    let sel_before = self.selected;
                    for (i, label) in install_labels.iter().enumerate() {
                        ui.selectable_value(&mut self.selected, i, label);
                    }
                    if self.selected != sel_before {
                        // 换来源 = 换风险：之前对"未验证版本"的确认必须作废
                        self.allow_unverified = false;
                    }
                }
                ui.separator();

                match &view.managed_ver {
                    None => {
                        ui.colored_label(AMBER, egui::RichText::new(t.copy_none).small())
                            .on_hover_text(t.copy_none_hint);
                    }
                    Some(v) if stale => {
                        ui.colored_label(
                            RED,
                            egui::RichText::new(t.copy_stale.replace("{v}", v)).small(),
                        )
                        .on_hover_text(t.copy_stale_hint);
                    }
                    Some(v) if !patched => {
                        ui.colored_label(
                            AMBER,
                            egui::RichText::new(t.copy_bad.replace("{v}", v)).small(),
                        )
                        .on_hover_text(t.copy_bad_hint);
                    }
                    Some(v) if patch::is_verified(v) => {
                        ui.colored_label(
                            GREEN,
                            egui::RichText::new(t.copy_ok.replace("{v}", v)).small(),
                        )
                        .on_hover_text(t.copy_ok_hint);
                    }
                    Some(v) => {
                        ui.colored_label(
                            AMBER,
                            egui::RichText::new(
                                t.copy_unverified
                                    .replace("{v}", v)
                                    .replace("{refv}", patch::VERIFIED_VERSION),
                            )
                            .small(),
                        )
                        .on_hover_text(t.copy_unverified_hint);
                    }
                }

                if let Some((idx, v, verified)) = &view.newer {
                    ui.separator();
                    if *verified {
                        ui.colored_label(
                            GREEN,
                            egui::RichText::new(t.copy_newer.replace("{v}", v)).small(),
                        )
                        .on_hover_text(t.copy_newer_hint);
                        if ui.small_button(t.btn_rebuild).clicked() {
                            *act = Some(Act::RebuildCopy(*idx));
                        }
                    } else {
                        // 新版不在白名单：不劝重建，重建按钮也上锁——副本本身是自包含的，
                        // 客户端升级不会影响它，多数情况下"什么都不做"才是对的。
                        ui.colored_label(
                            AMBER,
                            egui::RichText::new(t.copy_newer_unverified.replace("{v}", v)).small(),
                        )
                        .on_hover_text(
                            t.copy_newer_unverified_hint
                                .replace("{refv}", patch::VERIFIED_VERSION),
                        );
                        if ui
                            .add_enabled(can_build, egui::Button::new(t.btn_rebuild_anyway).small())
                            .on_disabled_hover_text(t.chk_allow_unverified)
                            .clicked()
                        {
                            *act = Some(Act::RebuildCopy(*idx));
                        }
                    }
                }
            });

            // 未验证版本的风险确认条：源没验证过就一直显示，勾不勾只影响按钮能不能点
            if let Some(v) = &view.build_src_unverified {
                let shown = source_name(v, self.cfg.lang);
                ui.add_space(6.0);
                egui::Frame::none()
                    .fill(AMBER_DIM)
                    .rounding(6.0)
                    .inner_margin(egui::vec2(12.0, 8.0))
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(
                                    t.warn_unverified_src
                                        .replace("{v}", &shown)
                                        .replace("{refv}", patch::VERIFIED_VERSION),
                                )
                                .color(AMBER)
                                .small(),
                            );
                            ui.checkbox(
                                &mut self.allow_unverified,
                                egui::RichText::new(t.chk_allow_unverified).small(),
                            );
                        });
                    });
            }

            // 未检测到 TradingView 时，给一个醒目的安装提醒（而非让用户对着空列表猜）
            if install_labels.is_empty() {
                ui.add_space(6.0);
                egui::Frame::none()
                    .fill(AMBER_DIM)
                    .rounding(6.0)
                    .inner_margin(egui::vec2(12.0, 10.0))
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(t.no_tv_alert).color(AMBER).strong());
                            if ui.button(t.rescan).clicked() {
                                self.installs = install::discover();
                                if self.selected >= self.installs.len() {
                                    self.selected =
                                        install::preferred_index(&self.installs).unwrap_or(0);
                                }
                                self.wake_scanner();
                            }
                        });
                    });
            }

            // ---------------------------------------------------------- 功能开关
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t.features).small().weak());
                let mut toggled = false;
                if ui
                    .checkbox(&mut self.cfg.adblock, t.cb_adblock)
                    .on_hover_text(t.cb_adblock_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.charts_unlock, t.cb_charts)
                    .on_hover_text(t.cb_charts_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.study_on_study, t.cb_study)
                    .on_hover_text(t.cb_study_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.drawings_unlock, t.cb_drawings)
                    .on_hover_text(t.cb_drawings_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.auto_recover, t.cb_recover)
                    .on_hover_text(t.cb_recover_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.alerts, t.cb_alerts)
                    .on_hover_text(t.cb_alerts_hint)
                    .changed()
                {
                    toggled = true;
                }
                if ui
                    .checkbox(&mut self.cfg.no_auto_update, t.cb_noupdate)
                    .on_hover_text(t.cb_noupdate_hint)
                    .changed()
                {
                    toggled = true;
                }
                if toggled {
                    // 配置写失败=开关下次启动会"自己变回去"，必须留下痕迹
                    if let Err(e) = install::save_config(&self.cfg) {
                        install::log_error("保存功能开关 config.json", &e);
                    }
                    *act = Some(Act::ReapplyPatches);
                }
            });

            // ---------------------------------------------------------- 使用说明
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t.usage).small().weak());
                for (i, step) in [t.add, t.step_launch, t.step_login].iter().enumerate() {
                    if i > 0 {
                        ui.label(egui::RichText::new("→").small().color(ACCENT));
                    }
                    ui.label(egui::RichText::new(*step).small());
                }
            });
            // 一条必须写进界面的约束：同账号多开会撞服务端的连接限制
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(t.usage_note).small().color(AMBER));
            });

            ui.add_space(6.0);
        });
    }
}
