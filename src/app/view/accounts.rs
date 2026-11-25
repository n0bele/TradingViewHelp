//! 账号区：添加表单、账号卡片（启动/停止/改名/删除/打开目录）、同账号查重提示。

use super::View;
use crate::app::*;
use eframe::egui;

impl App {
    pub(crate) fn accounts_panel(
        &mut self,
        ctx: &egui::Context,
        view: &View,
        busy: bool,
        act: &mut Option<Act>,
    ) {
        let t = view.t;
        let rows = &view.rows;
        let online = view.online;
        let has_profile = view.has_profile;
        let managed_ok = view.managed_ok;
        let can_build = view.can_build;
        // ------------------------------------------------------------ 账号
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t.accounts).strong());
                ui.label(
                    egui::RichText::new(
                        t.count
                            .replace("{n}", &rows.len().to_string())
                            .replace("{o}", &online.to_string()),
                    )
                    .small()
                    .weak(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // right_to_left：先加的在最右边，所以主操作「添加账号」放最后一个
                    if ui.add_enabled(!busy, egui::Button::new(t.add)).clicked() {
                        self.adding = true;
                        self.focus_add = true;
                        self.add_error = None;
                        self.new_name.clear();
                    }
                    if ui
                        .add_enabled(online > 0, egui::Button::new(t.stop_all))
                        .clicked()
                    {
                        *act = Some(Act::StopAll);
                    }
                    if ui
                        .add_enabled(
                            !busy && has_profile && managed_ok,
                            egui::Button::new(t.launch_all),
                        )
                        .on_disabled_hover_text(t.launch_all_hint)
                        .clicked()
                    {
                        *act = Some(Act::LaunchAll);
                    }
                });
            });

            ui.add_space(6.0);

            if self.adding {
                egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::same(10.0))
                    .fill(ui.style().visuals.faint_bg_color)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(t.name_label);
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.new_name)
                                    .hint_text(t.name_hint)
                                    .desired_width(180.0),
                            );
                            if self.focus_add {
                                resp.request_focus();
                                self.focus_add = false;
                            }
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                *act = Some(Act::SubmitAdd);
                            }
                            // 没副本时"添加账号"会顺带构建副本：源未验证就必须先确认
                            let can_submit = !busy
                                && !self.new_name.trim().is_empty()
                                && (managed_ok || can_build);
                            if ui
                                .add_enabled(can_submit, egui::Button::new(t.submit))
                                .on_disabled_hover_text(t.name_hint)
                                .clicked()
                            {
                                *act = Some(Act::SubmitAdd);
                            }
                            if busy {
                                ui.label(egui::RichText::new(t.creating).small().color(AMBER));
                            }
                            if ui.button(t.cancel).clicked() {
                                self.adding = false;
                                self.add_error = None;
                            }
                        });
                        if !managed_ok {
                            ui.colored_label(AMBER, egui::RichText::new(t.auto_build).small());
                        }
                        if let Some(e) = &self.add_error {
                            ui.colored_label(RED, egui::RichText::new(e).small());
                        }
                    });
                ui.add_space(6.0);
            }

            if rows.is_empty() {
                ui.add_space(12.0);
                ui.label(egui::RichText::new(t.empty_title).strong());
                ui.add_space(2.0);
                ui.label(egui::RichText::new(t.empty_desc).weak());
                ui.add_space(14.0);
                for (i, step) in [t.empty_s1, t.empty_s2, t.empty_s3].iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{}.", i + 1)).small().weak());
                        ui.label(egui::RichText::new(*step).small());
                    });
                }
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (name, pid, dir, uid, dup) in rows {
                        egui::Frame::group(ui.style())
                            .inner_margin(egui::Margin::same(12.0))
                            .fill(ui.style().visuals.faint_bg_color)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let starting = self
                                        .launching
                                        .get(name)
                                        .map(|at| at.elapsed().as_secs());
                                    let (dot, txt) = match pid {
                                        Some(id) => (GREEN, t.online.replace("{pid}", &id.to_string())),
                                        None => match starting {
                                            Some(s) => {
                                                (AMBER, t.launching.replace("{s}", &s.to_string()))
                                            }
                                            None => (GREY, t.offline.to_string()),
                                        },
                                    };
                                    ui.colored_label(dot, "●");
                                    ui.label(egui::RichText::new(name).strong().size(17.0));
                                    ui.label(egui::RichText::new(txt).small().color(dot));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            // 删除确认内联在卡片上：原先那条「确认删除」栏
                                            // 被放在占满剩余高度的滚动区之后，实际永远看不到
                                            if self.pending_delete.as_deref() == Some(name.as_str()) {
                                                if ui.button(t.cancel).clicked() {
                                                    self.pending_delete = None;
                                                }
                                                if ui
                                                    .button(
                                                        egui::RichText::new(t.confirm_delete)
                                                            .color(RED),
                                                    )
                                                    .clicked()
                                                {
                                                    self.pending_delete = None;
                                                    *act = Some(Act::Delete(name.clone()));
                                                }
                                                ui.label(
                                                    egui::RichText::new(t.del_confirm_short)
                                                        .small()
                                                        .color(RED),
                                                );
                                            } else if ui.button(t.delete).clicked() {
                                                self.pending_delete = Some(name.clone());
                                            }
                                            if ui.button(self.cfg.lang.btn_rename()).clicked() {
                                                self.name_edit = Some(NameEdit {
                                                    source: name.clone(),
                                                    text: name.clone(),
                                                });
                                                // 打开就聚焦并全选旧名：点完直接敲新名字即可
                                                self.focus_rename = true;
                                            }
                                            if ui.button(t.dir).clicked() {
                                                open_path(dir);
                                            }
                                            match pid {
                                                Some(_) => {
                                                    if ui.button(t.stop).clicked() {
                                                        *act = Some(Act::Stop(pid.unwrap()));
                                                    }
                                                }
                                                None => {
                                                    if ui
                                                        .add_enabled(
                                                            managed_ok && !busy && starting.is_none(),
                                                            egui::Button::new(t.launch),
                                                        )
                                                        .on_disabled_hover_text(t.launch_hint)
                                                        .clicked()
                                                    {
                                                        *act = Some(Act::Launch(name.clone()));
                                                    }
                                                }
                                            }
                                        },
                                    );
                                });
                                // 改名的内联输入：和删除确认一样贴在卡片上，不用弹窗
                                if self.name_edit.as_ref().is_some_and(|e| e.source == *name) {
                                    let hint = self.cfg.lang.e_rename_hint();
                                    let submit = t.submit.to_string();
                                    let cancel_label = t.cancel.to_string();
                                    let want_focus = std::mem::take(&mut self.focus_rename);
                                    let mut go = false;
                                    let mut cancel = false;
                                    let mut typed = String::new();
                                    // 直接绑到 name_edit.text：早先每帧 clone 一份局部 String 再交给
                                    // TextEdit，用户敲进去的字符会在下一帧被旧值覆盖，输入框等于只读。
                                    if let Some(edit) = self.name_edit.as_mut() {
                                        ui.horizontal(|ui| {
                                            ui.add_space(18.0);
                                            let output = egui::TextEdit::singleline(&mut edit.text)
                                                .hint_text(&hint)
                                                .desired_width(220.0)
                                                .show(ui);
                                            let resp = output.response;
                                            if want_focus {
                                                resp.request_focus();
                                                // 全选旧名：点完「改名」直接敲就是替换
                                                let mut state = output.state;
                                                let len = edit.text.chars().count();
                                                state.cursor.set_char_range(Some(
                                                    egui::text::CCursorRange::two(
                                                        egui::text::CCursor::new(0),
                                                        egui::text::CCursor::new(len),
                                                    ),
                                                ));
                                                state.store(ui.ctx(), resp.id);
                                            }
                                            let can = !busy && !edit.text.trim().is_empty();
                                            if ui
                                                .add_enabled(can, egui::Button::new(submit))
                                                .clicked()
                                                || (can
                                                    && resp.lost_focus()
                                                    && ui.input(|i| {
                                                        i.key_pressed(egui::Key::Enter)
                                                    }))
                                            {
                                                go = true;
                                            }
                                            if ui.button(cancel_label).clicked() {
                                                cancel = true;
                                            }
                                        });
                                        typed = edit.text.trim().to_string();
                                    }
                                    if go {
                                        *act = Some(Act::Rename(name.clone(), typed));
                                    }
                                    if go || cancel {
                                        self.name_edit = None;
                                    }
                                }
                                // 账号信息单独一行：同一行已有「名字 + 状态 + 4 个按钮」，
                                // 再塞账号 ID 在窄窗口会把按钮挤掉。
                                if uid.is_some() || dup.is_some() {
                                    ui.horizontal(|ui| {
                                        ui.add_space(18.0);
                                        if let Some(u) = uid {
                                            ui.label(
                                                egui::RichText::new(t.acct_bound.replace("{uid}", u))
                                                    .small()
                                                    .weak(),
                                            )
                                            .on_hover_text(t.usage_note);
                                        }
                                        if let Some(d) = dup {
                                            ui.colored_label(
                                                RED,
                                                egui::RichText::new(
                                                    t.acct_dup.replace("{name}", d),
                                                )
                                                .small(),
                                            )
                                            .on_hover_text(t.usage_note);
                                        }
                                    });
                                }
                            });
                        ui.add_space(4.0);
                    }
                });
        });
    }
}
