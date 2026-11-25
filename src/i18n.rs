//! 界面文案：中文 / English。
//!
//! 只覆盖 tvx 自己的界面与日志；TradingView 自身的界面语言不在控制范围。
//! 新增文案时两张表都要补齐（字段顺序一致，方便对照）。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    Zh,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Zh, Lang::En];

    /// 语言选择器上显示的名字（各语言用自己的写法）
    pub fn label(self) -> &'static str {
        match self {
            Lang::Zh => "中文",
            Lang::En => "English",
        }
    }

    /// 界面文案表
    pub fn s(self) -> &'static S {
        match self {
            Lang::Zh => &ZH,
            Lang::En => &EN,
        }
    }
}

/// 一屏界面用到的全部静态文案。占位符：{v} {refv} {n} {o} {pid} {name}
pub struct S {
    pub app_title: &'static str,
    pub subtitle: &'static str,
    // 顶栏
    pub installed_at: &'static str,
    pub no_tv: &'static str,
    pub no_tv_alert: &'static str,
    pub rescan: &'static str,
    pub copy_none: &'static str,
    pub copy_none_hint: &'static str,
    pub copy_ok: &'static str,
    pub copy_ok_hint: &'static str,
    pub copy_bad: &'static str,
    pub copy_bad_hint: &'static str,
    /// 副本里还留着旧包 app.asar：补丁写了也不会执行，单独一条别混进「补丁异常」
    pub copy_stale: &'static str,
    pub copy_stale_hint: &'static str,
    pub copy_unverified: &'static str,
    pub copy_unverified_hint: &'static str,
    pub copy_newer: &'static str,
    pub copy_newer_hint: &'static str,
    pub btn_rebuild: &'static str,
    /// 环境体检不通过时的顶栏提示（启动会被阻止）
    pub env_warn: &'static str,
    /// 「自检」按钮：重跑体检并把完整报告写进日志
    pub btn_doctor: &'static str,
    pub features: &'static str,
    /// 拦截广告弹窗（广告域名 + 充值窗口 + 页内推广浮层，一个开关）
    pub cb_adblock: &'static str,
    pub cb_adblock_hint: &'static str,
    pub cb_charts: &'static str,
    pub cb_charts_hint: &'static str,
    pub cb_study: &'static str,
    pub cb_study_hint: &'static str,
    pub cb_drawings: &'static str,
    pub cb_drawings_hint: &'static str,
    pub cb_recover: &'static str,
    pub cb_recover_hint: &'static str,
    pub cb_noupdate: &'static str,
    pub cb_noupdate_hint: &'static str,
    pub cb_alerts: &'static str,
    pub cb_alerts_hint: &'static str,
    pub usage: &'static str,
    pub usage_note: &'static str,
    pub tray_show: &'static str,
    pub tray_quit: &'static str,
    pub launching: &'static str,
    pub auto_build: &'static str,
    pub tray_fail: &'static str,
    pub step_launch: &'static str,
    pub step_login: &'static str,
    pub cancel: &'static str,
    pub kind_store: &'static str,
    pub kind_portable: &'static str,
    pub kind_custom: &'static str,
    /// 版本白名单标记（列表里跟在版本号后面）
    pub tag_verified: &'static str,
    pub tag_unverified: &'static str,
    /// 选中的构建源不在白名单里时的警告条
    pub warn_unverified_src: &'static str,
    pub chk_allow_unverified: &'static str,
    pub btn_rebuild_anyway: &'static str,
    pub copy_newer_unverified: &'static str,
    pub copy_newer_unverified_hint: &'static str,
    // 日志面板
    pub log: &'static str,
    pub clear: &'static str,
    // 账号
    pub accounts: &'static str,
    pub count: &'static str,
    pub add: &'static str,
    pub submit: &'static str,
    pub launch_all: &'static str,
    pub stop_all: &'static str,
    pub launch_all_hint: &'static str,
    pub name_label: &'static str,
    pub name_hint: &'static str,
    pub empty_title: &'static str,
    pub empty_desc: &'static str,
    pub empty_s1: &'static str,
    pub empty_s2: &'static str,
    pub empty_s3: &'static str,
    pub online: &'static str,
    pub offline: &'static str,
    pub acct_bound: &'static str,
    pub acct_dup: &'static str,
    pub delete: &'static str,
    pub dir: &'static str,
    pub stop: &'static str,
    pub launch: &'static str,
    pub launch_hint: &'static str,
    pub del_confirm_short: &'static str,
    pub creating: &'static str,
    pub confirm_delete: &'static str,
}

mod en;
mod zh;

pub use en::EN;
pub use zh::ZH;

// ------------------------------------------------------------------ 日志与提示

impl Lang {
    fn pick(&self, zh: String, en: String) -> String {
        match self {
            Lang::Zh => zh,
            Lang::En => en,
        }
    }

    /// 系统代理不可用时的提示（页面会一直转圈）
    pub fn log_proxy_down(&self, addr: &str) -> String {
        self.pick(
            format!("⚠ 系统代理 {addr} 连不上：TradingView 页面可能一直转圈，请检查代理 / VPN"),
            format!("⚠ System proxy {addr} is unreachable: the TradingView page may spin forever — check your proxy / VPN"),
        )
    }

    pub fn log_need_copy(&self) -> String {
        self.pick(
            "先在上方「生成 / 更新副本」".into(),
            "Build a copy first (button above)".into(),
        )
    }

    pub fn log_launched(&self, name: &str, pid: u32) -> String {
        self.pick(
            format!("启动「{name}」 PID {pid}"),
            format!("Launched \"{name}\" (PID {pid})"),
        )
    }

    pub fn log_launch_failed(&self, e: &str) -> String {
        self.pick(format!("启动失败：{e}"), format!("Launch failed: {e}"))
    }

    /// 批量启动时的单条进度
    pub fn log_launching(&self, name: &str, pid: u32) -> String {
        self.pick(
            format!("… 启动「{name}」 PID {pid}"),
            format!("… launching \"{name}\" (PID {pid})"),
        )
    }

    pub fn log_launch_one_failed(&self, name: &str, e: &str) -> String {
        self.pick(
            format!("× 「{name}」{e}"),
            format!("× \"{name}\": {e}"),
        )
    }

    /// 同时运行的实例数到顶：每个实例是一整棵 Electron 进程树，内存不是无限的
    pub fn log_running_limit(&self, running: usize, max: usize) -> String {
        self.pick(
            format!("已在运行 {running} 个实例（上限 {max}）；先停掉不用的账号再启动"),
            format!(
                "Already running {running} instance(s) (limit {max}); stop an unused account first"
            ),
        )
    }

    /// 批量启动被上限截断时：只启动额度内的账号
    pub fn log_launch_budget(&self, launching: usize, running: usize, max: usize) -> String {
        self.pick(
            format!("已运行 {running} 个实例（上限 {max}），本次只启动 {launching} 个"),
            format!("{running} instance(s) already running (limit {max}); launching {launching}"),
        )
    }

    /// 改名：按钮、输入提示与日志
    pub fn btn_rename(&self) -> String {
        self.pick("改名".into(), "Rename".into())
    }


    pub fn e_rename_hint(&self) -> String {
        self.pick(
            "新名字（回车确认）".into(),
            "New name (press Enter)".into(),
        )
    }



    pub fn log_renamed(&self, old: &str, new: &str) -> String {
        self.pick(
            format!("「{old}」已改名为「{new}」"),
            format!("\"{old}\" renamed to \"{new}\""),
        )
    }

    pub fn log_rename_failed(&self, e: &str) -> String {
        self.pick(format!("改名失败：{e}"), format!("Rename failed: {e}"))
    }




    /// 登录页打开方式（应用内标签页 / 无痕窗口 / 默认浏览器）
    pub fn log_login_in_app(&self) -> String {
        self.pick(
            "已在应用内标签页打开登录页（不需要外部浏览器）".into(),
            "Login page opened in an in-app tab (no external browser needed)".into(),
        )
    }

    pub fn log_login_private(&self) -> String {
        self.pick(
            "已在无痕窗口打开登录页（不受你浏览器里已有账号影响）".into(),
            "Login page opened in a private window (unaffected by accounts already signed in)"
                .into(),
        )
    }

    pub fn log_login_default_browser(&self) -> String {
        self.pick(
            "已用默认浏览器打开登录页（未找到 Chrome/Edge）".into(),
            "Login page opened in your default browser (Chrome/Edge not found)".into(),
        )
    }

    pub fn log_stopped(&self, pid: u32) -> String {
        self.pick(format!("已停止 PID {pid}"), format!("Stopped PID {pid}"))
    }

    pub fn log_stop_failed(&self, e: &str) -> String {
        self.pick(format!("停止失败：{e}"), format!("Stop failed: {e}"))
    }

    pub fn log_launched_all(&self, n: usize) -> String {
        self.pick(
            format!("已启动 {n} 个实例"),
            format!("Launched {n} instance(s)"),
        )
    }

    pub fn log_stopped_all(&self, n: usize) -> String {
        self.pick(
            format!("已停止 {n} 个实例"),
            format!("Stopped {n} instance(s)"),
        )
    }

    pub fn log_deleted(&self, name: &str) -> String {
        self.pick(format!("已删除「{name}」"), format!("Deleted \"{name}\""))
    }

    pub fn log_delete_failed(&self, e: &str) -> String {
        self.pick(format!("删除失败：{e}"), format!("Delete failed: {e}"))
    }

    pub fn log_created(&self, name: &str, dir: &str) -> String {
        self.pick(
            format!("已创建「{name}」：{dir}"),
            format!("Created \"{name}\": {dir}"),
        )
    }

    pub fn log_create_failed(&self, e: &str) -> String {
        self.pick(format!("创建失败：{e}"), format!("Create failed: {e}"))
    }

    pub fn log_no_source(&self) -> String {
        self.pick("没有可用的安装来源".into(), "No usable install source".into())
    }

    pub fn log_built(&self, dir: &str) -> String {
        self.pick(format!("副本就绪：{dir}"), format!("Copy ready: {dir}"))
    }

    /// 开关摘要：开 / 关（自动更新用 禁用 / 照常）
    pub fn log_switches(
        &self,
        adblock: bool,
        charts: bool,
        study: bool,
        drawings: bool,
        no_update: bool,
    ) -> String {
        let (on, off) = match self {
            Lang::Zh => ("开", "关"),
            Lang::En => ("on", "off"),
        };
        let st = |b: bool| if b { on } else { off };
        let upd = match (self, no_update) {
            (Lang::En, true) => "disabled",
            (Lang::En, false) => "normal",
            (_, true) => "禁用",
            (_, false) => "照常",
        };
        let head = match self {
            Lang::Zh => "开关：拦截广告弹窗 {adblock} · 多图表 {charts} · 指标叠加 {study} · 绘图不限 {draw} · 自动更新 {upd}",
            Lang::En => "Options: adblock {adblock} · charts {charts} · study {study} · drawings {draw} · auto-update {upd}",
        };
        head.replace("{adblock}", st(adblock))
            .replace("{charts}", st(charts))
            .replace("{study}", st(study))
            .replace("{draw}", st(drawings))
            .replace("{upd}", upd)
    }

    pub fn log_reapplied(&self) -> String {
        self.pick(
            "补丁已按当前开关应用；正在运行的实例重启后生效".into(),
            "Patches applied with current options; running instances take effect after restart".into(),
        )
    }

    /// 按新版本重建副本：开始 / 完成
    pub fn log_rebuild_start(&self, v: &str) -> String {
        self.pick(
            format!("按 {v} 重建副本（复制 → 解包 → 打补丁，约需几十秒）…"),
            format!("Rebuilding the copy from {v} (copy → unpack → patch, may take a while)…"),
        )
    }

    pub fn log_rebuilt(&self, v: &str, old: &str) -> String {
        self.pick(
            format!("副本已切换到 {v}；旧版本目录 apps/{old} 未删除，可手动清理"),
            format!("Copy switched to {v}; the old apps/{old} directory was left in place and can be removed manually"),
        )
    }

    /// 启动时自动把托管副本升级到当前补丁集
    pub fn log_patch_upgraded(&self) -> String {
        self.pick(
            "托管副本的补丁已升级到当前版本（断线弹窗改为自动恢复）；正在运行的实例重启后生效"
                .into(),
            "Managed copy upgraded to the current patch set (disconnect dialogs now auto-recover); running instances take effect after restart"
                .into(),
        )
    }

    /// 部分补丁锚点未命中：单条失效不阻塞其余功能
    pub fn log_patch_partial(&self, failed: &[String]) -> String {
        self.pick(
            format!(
                "⚠ 有 {} 项补丁未生效（上游代码结构可能已变）：{}。其余功能正常；可在关闭 TradingView 后重建副本",
                failed.len(),
                failed.join("、")
            ),
            format!(
                "⚠ {} patch(es) did not apply (upstream code may have changed): {}. Other features are unaffected; rebuild the copy after closing TradingView",
                failed.len(),
                failed.join(", ")
            ),
        )
    }

    /// 启动时的体检结论。以前"一切正常"是静默的（日志空白），用户看到空面板会以为
    /// 检查没跑 / 功能没生效；正常也要留一条能对照的记录。
    pub fn log_health_ok(&self, ver: &str, copy: &str, ok: usize, all: usize) -> String {
        self.pick(
            format!("● v{ver} · 副本 {copy} · {ok}/{all} 补丁已生效，环境正常"),
            format!("● v{ver} · copy {copy} · {ok}/{all} patches active, environment OK"),
        )
    }

    /// 重打补丁前顺手清掉旧包：不告诉用户的话，用户不知道"只是拨了个开关"就把病根除了
    pub fn log_stale_cleared(&self, path: &str) -> String {
        self.pick(
            format!("已清掉残留的旧包 {path}（它会让补丁全部失效）；正在运行的实例重启后生效"),
            format!("Removed the stale {path} (it silently disabled every patch); running instances take effect after restart"),
        )
    }

    /// 清不掉旧包时：说清原因和用户该做什么，别只说"失败"
    pub fn log_stale_locked(&self, path: &str) -> String {
        self.pick(
            format!(
                "⚠ 旧包 {path} 删不掉，补丁不会生效。请先点「全部停止」关掉客户端\
                 （必要时在任务管理器结束 TradingView.exe）或把它加入杀软白名单，再拨动任意开关重试"
            ),
            format!(
                "⚠ Could not remove {path}; patches will not take effect. Click \"Stop all\" to close the client \
                 (or end TradingView.exe in Task Manager), or whitelist it in your antivirus, then toggle any option to retry"
            ),
        )
    }

    pub fn log_no_copy_headless(&self) -> String {
        self.pick(
            "没有托管副本，先在界面里生成".into(),
            "No managed copy yet; build one in the GUI first".into(),
        )
    }

    pub fn log_no_install_headless(&self) -> String {
        self.pick(
            "系统里没找到已安装的 TradingView，无法构建副本".into(),
            "No installed TradingView found; cannot build a copy".into(),
        )
    }

    pub fn log_built_headless(&self, version: &str, path: &str) -> String {
        self.pick(
            format!("● 已按 {version} 构建托管副本：{path}"),
            format!("● Managed copy built from {version}: {path}"),
        )
    }

    pub fn log_reapplied_headless(&self, path: &str) -> String {
        self.pick(
            format!("● 补丁已按当前开关重打到 {path}"),
            format!("● Patches re-applied to {path}"),
        )
    }

    // ---------------------------------------------------------- 名称校验 / 账号
    pub fn v_empty(&self) -> String {
        self.pick("名称不能为空".into(), "Name must not be empty".into())
    }

    pub fn v_too_long(&self) -> String {
        self.pick(
            "名称太长（最多 40 字节）".into(),
            "Name too long (40 bytes max)".into(),
        )
    }

    pub fn v_dot(&self) -> String {
        self.pick(
            "名称不能以点开头".into(),
            "Name must not start with a dot".into(),
        )
    }

    pub fn v_chars(&self) -> String {
        self.pick(
            "名称里有不允许的字符".into(),
            "Name contains disallowed characters".into(),
        )
    }

    pub fn p_exists(&self, name: &str) -> String {
        self.pick(
            format!("「{name}」已经存在了"),
            format!("\"{name}\" already exists"),
        )
    }

    /// 拿未验证版本构建副本、但没勾确认时拦下
    pub fn e_unverified_blocked(&self, v: &str) -> String {
        self.pick(
            format!("版本 {v} 未经验证：请先在上方勾选「我了解风险，仍要用该版本构建副本」"),
            format!(
                "Version {v} is unverified - tick the risk confirmation above before building from it"
            ),
        )
    }

    pub fn p_dir_exists(&self, name: &str) -> String {
        self.pick(
            format!("账号「{name}」已存在"),
            format!("Account \"{name}\" already exists"),
        )
    }

    /// 同账号多开时的提示（允许启动，但要让用户知道风险）
    ///
    /// 说清楚是"服务端同一个账号"，不是"本地共用一份文件"——两个档案各存各的登录态，
    /// 本地互不影响；风险在服务端那侧。
    pub fn warn_same_account(&self, name: &str, other: &str, uid: &str) -> String {
        self.pick(
            format!(
                "「{name}」和「{other}」登录的是同一个 TradingView 账号（{uid}）：两份档案的登录态\
                 各自独立保存、互不影响，但服务端看到的是同一账号的两个连接，可能限制并发——\
                 出现行情停更就关掉其中一个"
            ),
            format!(
                "\"{name}\" and \"{other}\" are signed in to the same TradingView account ({uid}). \
                 Each profile keeps its own session, but the server sees two connections for one \
                 account and may throttle them; if feeds stop updating, close one window"
            ),
        )
    }

    /// 启动后认出这个档案登录的是哪个账号
    pub fn log_account_bound(&self, name: &str, uid: &str) -> String {
        self.pick(
            format!("「{name}」登录的是 TradingView 账号 {uid}"),
            format!("\"{name}\" is signed in as TradingView account {uid}"),
        )
    }

    /// 启动后认出同账号还有一个实例在跑
    pub fn log_account_same(&self, name: &str, other: &str, uid: &str) -> String {
        self.pick(
            format!(
                "⚠ 「{name}」和「{other}」是同一个账号（{uid}）：多开会不会被服务端限流我们说了不算，\
                 出现行情停更就关掉其中一个"
            ),
            format!(
                "⚠ \"{name}\" and \"{other}\" are the same account ({uid}): whether the server \
                 throttles concurrent sessions is out of our hands — if feeds stop updating, close one"
            ),
        )
    }

    /// 全部启动时的错峰提示
    pub fn log_stagger(&self, n: usize, secs: u64) -> String {
        self.pick(
            format!("全部启动：{n} 个账号，按 {secs} 秒间隔错峰启动（避免瞬间建连风暴）"),
            format!("Launching {n} account(s) staggered by {secs}s (avoid a connection burst)"),
        )
    }

    pub fn p_running(&self) -> String {
        self.pick(
            "该账号的实例还在运行，先停止再改名或删除".into(),
            "This account's instance is still running; stop it before renaming or deleting".into(),
        )
    }


    // ---------------------------------------------------------- 构建过程日志
    pub fn b_incomplete(&self, path: &str) -> String {
        self.pick(
            format!("这个来源不完整（缺可执行文件或 resources/app.asar）：{path}"),
            format!("Incomplete source (missing executable or resources/app.asar): {path}"),
        )
    }

    pub fn b_unverified(&self, v: &str, refv: &str) -> String {
        self.pick(
            format!("⚠ 来源版本 {v} 不在已验证名单（锚点按 {refv} 提取），下面逐条标记结果"),
            format!(
                "⚠ Source version {v} is not on the verified list (anchors from {refv}); each patch below is reported individually"
            ),
        )
    }

    pub fn b_cleanup(&self, dir: &str) -> String {
        self.pick(format!("清理旧副本 {dir}"), format!("Removing old copy {dir}"))
    }

    pub fn b_delete_failed(&self, dir: &str) -> String {
        self.pick(format!("删除 {dir}"), format!("Delete {dir}"))
    }

    pub fn b_copying(&self, from: &str, to: &str) -> String {
        self.pick(
            format!("复制 {from} → {to}"),
            format!("Copying {from} → {to}"),
        )
    }

    pub fn b_copied(&self, n: usize) -> String {
        self.pick(
            format!("复制完成，共 {n} 个文件"),
            format!("Copy done, {n} files"),
        )
    }

    pub fn b_unpack(&self) -> String {
        self.pick("解包 app.asar …".into(), "Unpacking app.asar …".into())
    }

    pub fn b_unpacked(&self, files: usize, skipped: usize) -> String {
        let tail_zh = if skipped > 0 {
            format!("（跳过 {skipped} 个链接）")
        } else {
            String::new()
        };
        let tail_en = if skipped > 0 {
            format!(" ({skipped} links skipped)")
        } else {
            String::new()
        };
        self.pick(
            format!("解包完成：{files} 个文件{tail_zh}"),
            format!("Unpacked: {files} files{tail_en}"),
        )
    }

    pub fn b_patching(&self) -> String {
        self.pick("打补丁 …".into(), "Applying patches …".into())
    }

    pub fn b_progress(&self, n: usize) -> String {
        self.pick(
            format!("  已复制 {n} 个文件 …"),
            format!("  copied {n} files …"),
        )
    }

    #[allow(dead_code)] // 只有 macOS 构建会用到（resign_if_needed）
    pub fn b_resigned(&self) -> String {
        self.pick("已 ad-hoc 重新签名".into(), "Ad-hoc re-signed".into())
    }

    #[allow(dead_code)] // 只有 macOS 构建会用到（resign_if_needed）
    pub fn b_resign_warn(&self) -> String {
        self.pick(
            "⚠ codesign 不可用或失败，macOS 上可能被 Gatekeeper 拦，需要手动重签".into(),
            "⚠ codesign unavailable or failed; Gatekeeper may block it on macOS — re-sign manually".into(),
        )
    }
}
