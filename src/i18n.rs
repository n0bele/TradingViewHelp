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
    pub copy_unverified: &'static str,
    pub copy_unverified_hint: &'static str,
    pub copy_newer: &'static str,
    pub copy_newer_hint: &'static str,
    pub btn_rebuild: &'static str,
    pub features: &'static str,
    pub cb_ads: &'static str,
    pub cb_ads_hint: &'static str,
    pub cb_charts: &'static str,
    pub cb_charts_hint: &'static str,
    pub cb_study: &'static str,
    pub cb_study_hint: &'static str,
    pub cb_silence: &'static str,
    pub cb_silence_hint: &'static str,
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

static ZH: S = S {
    app_title: "TradingView 助手",
    subtitle: "多账号 · 本地补丁",
    installed_at: "安装在",
    no_tv: "未检测到 TradingView",
    no_tv_alert: "⚠ 未检测到 TradingView，请先安装 TradingView 桌面版后再使用本工具。\n安装完成后回到这里，点「重新检测」。",
    rescan: "重新检测",
    copy_none: "● 副本未生成",
    copy_none_hint: "点右侧「生成 / 更新副本」",
    copy_ok: "● 副本 {v} · 补丁完好 ✓",
    copy_ok_hint: "锚点按该版本 index.js 提取并端到端实测；改开关会自动重打补丁",
    copy_bad: "● 副本 {v} · 补丁异常",
    copy_bad_hint: "点右侧「生成 / 更新副本」修复",
    copy_unverified: "● 副本 {v} · 未验证（锚点按 {refv} 校验）",
    copy_unverified_hint: "补丁锚点是字面代码串，压缩标识符随版本漂移；失配时构建会显式报错，不会打一半",
    copy_newer: "· 发现新版本 {v}",
    copy_newer_hint: "系统里的 TradingView 已升到 {v}，副本仍停在当前版本。建议按新版本重建副本——新版本的 JS 结构可能已变，补丁锚点会失配",
    btn_rebuild: "重建副本",
    features: "功能",
    cb_ads: "拦截广告/追踪网络",
    cb_ads_hint: "网络层拦掉广告与追踪域名",
    cb_charts: "多图表布局解锁",
    cb_charts_hint: "客户端 gate：free 档上限 1，注入后放行",
    cb_study: "指标叠加解锁",
    cb_study_hint: "客户端 gate：子指标上限 free 档 1，注入后放大",
    cb_silence: "屏蔽充值窗口",
    cb_silence_hint: "不再弹出充值窗口",
    cb_recover: "断线自动恢复",
    cb_recover_hint: "检测到「连接已关闭 / 恢复连接」弹窗时自动点恢复，4 秒未恢复就重载页面，之后按 15 秒 / 1 分钟 / 5 分钟再试（最多 4 次，仍失败就保留弹窗）；连续 1 分钟无弹窗则重新计数。只做恢复，不改变服务端并发配额",
    cb_noupdate: "禁用自动更新",
    cb_noupdate_hint: "去掉启动时的更新检查，手动检查不受影响",
    cb_alerts: "本地指标报警",
    cb_alerts_hint: "在图表页加一个「报警」按钮：按当前图表的品种/周期/指标设条件，命中后发系统通知或邮件。只在本机运行，不占用也不绕过服务端报警配额",
    usage: "使用说明",
    usage_note: "每个账号请登录不同的 TradingView 账号——同一账号多开会被服务端限制，表现为行情停止更新",
    tray_show: "显示主窗口",
    tray_quit: "退出",
    launching: "启动中… {s}s",
    auto_build: "还没有副本：点「添加」会先自动生成副本，再创建账号（首次约 1 分钟）。",
    tray_fail: "⚠ 托盘创建失败：关闭窗口将直接退出（可看日志排查）",
    step_launch: "点「启动」",
    step_login: "在 TradingView 窗口里登录",
    cancel: "取消",
    kind_store: "Microsoft Store",
    kind_portable: "本地安装",
    kind_custom: "手动指定",
    log: "日志",
    clear: "清空",
    accounts: "账号",
    count: "{n} 个 · {o} 个在线",
    add: "＋ 添加账号",
    submit: "添加",
    launch_all: "全部启动",
    stop_all: "全部停止",
    launch_all_hint: "需要先有账号并生成副本",
    name_label: "账号名",
    name_hint: "例如 主号 / 小号",
    empty_title: "还没有账号。",
    empty_desc: "一个账号 = 一个独立配置目录 = 一份独立登录。\n建两个账号、分别登录不同的 TradingView 账号，它们就能同时在线、同时看盘。",
    empty_s1: "在上面填个名字（比如「主号」）→ 点「添加」",
    empty_s2: "账号卡片出现后，点它的「启动」",
    empty_s3: "在弹出的 TradingView 窗口里登录。要双开就再添加一个账号",
    online: "在线 · PID {pid}",
    offline: "未启动",
    acct_bound: "· 账号 {uid}",
    acct_dup: "⚠ 与「{name}」同账号",
    delete: "删除",
    dir: "目录",
    stop: "停止",
    launch: "启动",
    launch_hint: "先在上方生成免弹窗副本",
    del_confirm_short: "删除后本地配置目录会被永久删除",
    creating: "正在创建…",
    confirm_delete: "确认删除",
};

static EN: S = S {
    app_title: "TradingView Helper",
    subtitle: "Multi-account · local patches",
    installed_at: "Installed at",
    no_tv: "TradingView not found",
    no_tv_alert: "⚠ TradingView is not installed. Please install the TradingView desktop app before using this tool.\nAfter installing, come back and click \"Rescan\". If it is installed but not detected, specify the install folder manually below.",
    rescan: "Rescan",
    copy_none: "● Copy not built",
    copy_none_hint: "Click \"Build / Update copy\" on the right",
    copy_ok: "● Copy {v} · patches OK ✓",
    copy_ok_hint: "Anchors extracted from this version's index.js and verified end-to-end; toggling options re-applies automatically",
    copy_bad: "● Copy {v} · patches broken",
    copy_bad_hint: "Click \"Build / Update copy\" to repair",
    copy_unverified: "● Copy {v} · unverified (anchors checked against {refv})",
    copy_unverified_hint: "Patch anchors are literal code strings; minified identifiers drift between versions. On mismatch the build fails loudly instead of half-applying",
    copy_newer: "· newer {v} found",
    copy_newer_hint: "The installed TradingView is now {v} but the managed copy is still on the old version. Rebuild the copy - the JS structure may have changed and the anchors will miss",
    btn_rebuild: "Rebuild copy",
    features: "Features",
    cb_ads: "Block ads & trackers",
    cb_ads_hint: "Cancel ad/tracking domains at the network layer",
    cb_charts: "Multi-chart layouts",
    cb_charts_hint: "Client-side gate: free tier allows 1; the injection lifts it",
    cb_study: "Study-on-study",
    cb_study_hint: "Client-side gate: free tier child limit 1; the injection raises it",
    cb_silence: "Block billing popups",
    cb_silence_hint: "Hide billing popups",
    cb_recover: "Auto-recover on disconnect",
    cb_recover_hint: "When the site shows a \"connection closed\" / \"restore connection\" dialog, click restore; reload the page if still down after 4s, then retry after 15s / 1min / 5min (max 4 tries, dialog is kept if all fail). The counter resets after 1 minute without a dialog. Recovery only — it cannot raise the server-side connection quota",
    cb_noupdate: "Disable auto-update",
    cb_noupdate_hint: "Skip the update check at startup; manual check still works",
    cb_alerts: "Local indicator alerts",
    cb_alerts_hint: "Adds an Alerts button on the chart page: conditions on the current chart's symbol/timeframe/indicator, delivered via system notification or email. Runs locally only; it neither uses nor bypasses the server-side alert quota",
    usage: "How to use",
    usage_note: "Sign in with a different TradingView account per profile — using one account in several instances hits the server limit and feeds stop updating",
    tray_show: "Show window",
    tray_quit: "Quit",
    launching: "Starting… {s}s",
    auto_build: "No copy yet: \"Add\" builds the copy first, then creates the account (~1 min the first time).",
    tray_fail: "⚠ Tray unavailable: closing the window exits the app",
    step_launch: "click \"Launch\"",
    step_login: "sign in inside the TradingView window",
    cancel: "Cancel",
    kind_store: "Microsoft Store",
    kind_portable: "Local install",
    kind_custom: "Manual",
    log: "Log",
    clear: "Clear",
    accounts: "Accounts",
    count: "{n} total · {o} online",
    add: "＋ Add account",
    submit: "Add",
    launch_all: "Launch all",
    stop_all: "Stop all",
    launch_all_hint: "Needs an account and a built copy first",
    name_label: "Name",
    name_hint: "e.g. main / alt",
    empty_title: "No accounts yet.",
    empty_desc: "One account = one config folder = one login.\nCreate two accounts, sign in with different TradingView accounts, and both can run at the same time.",
    empty_s1: "Type a name above (e.g. \"main\") → click \"Add\"",
    empty_s2: "When the card appears, click \"Launch\"",
    empty_s3: "Sign in inside the TradingView window. Add another account for a second instance.",
    online: "Online · PID {pid}",
    offline: "Not running",
    acct_bound: "· account {uid}",
    acct_dup: "⚠ same account as \"{name}\"",
    delete: "Delete",
    dir: "Folder",
    stop: "Stop",
    launch: "Launch",
    launch_hint: "Build a copy above first",
    del_confirm_short: "The local config folder will be removed permanently",
    creating: "Creating…",
    confirm_delete: "Confirm delete",
};

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
        ads: bool,
        charts: bool,
        study: bool,
        silence: bool,
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
            Lang::Zh => "开关：广告拦截 {ads} · 多图表 {charts} · 指标叠加 {study} · 屏蔽充值窗口 {silence} · 自动更新 {upd}",
            Lang::En => "Options: ads {ads} · charts {charts} · study {study} · block billing popups {silence} · auto-update {upd}",
        };
        head.replace("{ads}", st(ads))
            .replace("{charts}", st(charts))
            .replace("{study}", st(study))
            .replace("{silence}", st(silence))
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

    pub fn log_no_copy_headless(&self) -> String {
        self.pick(
            "没有托管副本，先在界面里生成".into(),
            "No managed copy yet; build one in the GUI first".into(),
        )
    }

    pub fn log_reapplied_headless(&self, path: &str) -> String {
        self.pick(
            format!("✓ 补丁已按当前开关重打到 {path}"),
            format!("✓ Patches re-applied to {path}"),
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

    pub fn p_dir_exists(&self, name: &str) -> String {
        self.pick(
            format!("账号「{name}」已存在"),
            format!("Account \"{name}\" already exists"),
        )
    }

    /// 同账号双开被拒绝时的说明（不是配额不够，是"撞配额"）
    pub fn dup_account(&self, uid: &str, other: &str) -> String {
        self.pick(
            format!(
                "「{other}」在线且登录的是同一个 TradingView 账号（{uid}）。\
                 同一账号只能开一个实例：请换一个账号登录，或先停掉「{other}」"
            ),
            format!(
                "\"{other}\" is already online with the same TradingView account ({uid}). \
                 One account = one instance: sign in with a different account, or stop \"{other}\" first"
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
                "⚠ 「{name}」和「{other}」是同一个账号（{uid}）：同一账号多开会被服务端限制，\
                 表现为某个窗口行情停止更新；建议一个账号只留一个档案"
            ),
            format!(
                "⚠ \"{name}\" and \"{other}\" share the same account ({uid}): running one account \
                 in two instances hits the server limit and feeds stop updating; keep one profile per account"
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
            "该账号的实例还在运行，先停止再删除".into(),
            "This account's instance is still running; stop it first".into(),
        )
    }

    // ---------------------------------------------------------- 构建过程日志
    pub fn b_incomplete(&self, path: &str) -> String {
        self.pick(
            format!("这个来源不完整（缺可执行文件或 resources/app.asar）：{path}"),
            format!("Incomplete source (missing executable or resources/app.asar): {path}"),
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
