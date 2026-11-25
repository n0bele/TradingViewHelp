//! 视觉：字体栈、全局样式与状态色。界面代码只用这里的常量，不写魔法色值。

use eframe::egui;

/// 字体栈：Windows 上拉丁用 Segoe UI（系统原生 UI 字体，小字号锐利），中文回退雅黑；
/// 等宽区保持 Hack 主导、中文回退——不再把整条家族链替换成雅黑（那是"难看"的主因）。
/// macOS/Linux 直接用系统 CJK 字体（苹方/Noto 的拉丁本身清晰），行为与旧版一致。
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    const CJK: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhl.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\deng.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    ];
    let mut cjk = None;
    for p in CJK {
        if let Ok(bytes) = std::fs::read(p) {
            cjk = Some(egui::FontData::from_owned(bytes));
            break;
        }
    }
    if let Some(data) = cjk {
        fonts.font_data.insert("cjk".to_owned(), data);
        #[cfg(windows)]
        {
            let prop = fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default();
            match std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
                Ok(b) => {
                    fonts
                        .font_data
                        .insert("segoe".to_owned(), egui::FontData::from_owned(b));
                    prop.insert(0, "segoe".to_owned());
                    prop.insert(1, "cjk".to_owned());
                }
                Err(_) => prop.insert(0, "cjk".to_owned()),
            }
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("cjk".to_owned());
        }
        #[cfg(not(windows))]
        {
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "cjk".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "cjk".to_owned());
        }
        ctx.set_fonts(fonts);
    }
}

/// 统一的视觉基调：明确的深色配色 + 高对比控件 + 舒朗间距。
///
/// 为什么要自定义配色：egui 默认深色里"未激活"控件是 #2C2C2C 灰底压在
/// #1B1B1B 面板上，两者几乎同色、也没有描边——按钮和复选框看起来全是
/// 禁用态。这里给控件真正的底色 + 可见描边 + 蓝色强调：可用态有底有边，
/// 禁用态是空心淡字，两种状态一眼分得开。
pub fn setup_style(ctx: &egui::Context) {
    use egui::Color32;

    const PANEL: Color32 = Color32::from_rgb(0x1a, 0x1d, 0x22);
    const WIN: Color32 = Color32::from_rgb(0x14, 0x16, 0x1a);
    const CARD: Color32 = Color32::from_rgb(0x22, 0x26, 0x2d);
    const CHIP: Color32 = Color32::from_rgb(0x2b, 0x31, 0x3a);
    const CHIP_HOVER: Color32 = Color32::from_rgb(0x34, 0x3c, 0x48);
    const BORDER: Color32 = Color32::from_rgb(0x3f, 0x48, 0x55);
    const TEXT: Color32 = Color32::from_rgb(0xe6, 0xed, 0xf3);
    const ACCENT: Color32 = Color32::from_rgb(0x4c, 0x8d, 0xf6);

    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = WIN;
    v.faint_bg_color = CARD; // 账号卡片底色
    v.extreme_bg_color = Color32::from_rgb(0x0e, 0x11, 0x14); // 输入框内部
    v.selection.bg_fill = Color32::from_rgb(0x2f, 0x6f, 0xeb); // 选中项 / 文本选区
    v.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    v.hyperlink_color = Color32::from_rgb(0x58, 0xa6, 0xff);

    // 非交互元素：卡片框线（Frame::group 的描边来源）、普通标签文字
    v.widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
    v.widgets.noninteractive.bg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(0x30, 0x37, 0x42));
    v.widgets.noninteractive.fg_stroke =
        egui::Stroke::new(1.0_f32, Color32::from_rgb(0xc5, 0xce, 0xd8));
    v.widgets.noninteractive.rounding = egui::Rounding::same(8.0); // 卡片圆角来源

    let w = &mut v.widgets;
    w.inactive.bg_fill = CHIP;
    w.inactive.weak_bg_fill = CHIP;
    w.inactive.bg_stroke = egui::Stroke::new(1.0_f32, BORDER);
    w.inactive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    w.inactive.rounding = egui::Rounding::same(8.0);

    w.hovered.bg_fill = CHIP_HOVER;
    w.hovered.weak_bg_fill = CHIP_HOVER;
    w.hovered.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    w.hovered.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    w.hovered.rounding = egui::Rounding::same(8.0);

    w.active.bg_fill = ACCENT; // 按下 / 勾选：实心蓝，不存在"像禁用"的可能
    w.active.weak_bg_fill = ACCENT;
    w.active.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(0x7a, 0xae, 0xff));
    w.active.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    w.active.rounding = egui::Rounding::same(8.0);

    w.open.bg_fill = CHIP_HOVER;
    w.open.weak_bg_fill = CHIP_HOVER;
    w.open.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
    w.open.fg_stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    w.open.rounding = egui::Rounding::same(8.0);

    ctx.style_mut(|style| {
        style.visuals = v;
        style.visuals.menu_rounding = egui::Rounding::same(8.0);
        // 文本可选中，方便从日志里复制路径 / PID
        style.interaction.selectable_labels = true;
        // 字号阶梯：正文/按钮 16、小字 13、标题 20、等宽 14 —— Segoe UI 拉丁 +
        // 雅黑中文的组合在这个尺寸最耐看，小字号中文发糊的问题一并解决
        style.text_styles = [
            (egui::TextStyle::Body, egui::FontId::proportional(16.0)),
            (egui::TextStyle::Button, egui::FontId::proportional(16.0)),
            (egui::TextStyle::Heading, egui::FontId::proportional(20.0)),
            (egui::TextStyle::Small, egui::FontId::proportional(13.0)),
            (egui::TextStyle::Monospace, egui::FontId::monospace(14.0)),
        ]
        .into();
        // 间距：行距拉开（默认纵向 3px 太挤）、按钮与卡片内边距给足呼吸感
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.menu_margin = egui::Margin::same(10.0);
    });
}

pub(crate) const GREEN: egui::Color32 = egui::Color32::from_rgb(86, 190, 130);
pub(crate) const GREY: egui::Color32 = egui::Color32::from_rgb(122, 122, 122);
pub(crate) const AMBER: egui::Color32 = egui::Color32::from_rgb(214, 160, 74);
pub(crate) const AMBER_DIM: egui::Color32 = egui::Color32::from_rgb(58, 45, 28);
pub(crate) const RED: egui::Color32 = egui::Color32::from_rgb(224, 106, 90);
pub(crate) const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4c, 0x8d, 0xf6);

