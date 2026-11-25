//! asar 模块的回归测试。

use super::*;


/// 用系统里真实的 app.asar 验证解包器；没有就跳过。
#[test]
fn extract_real_asar() {
    let candidates = [
        r"C:\Program Files\WindowsApps\TradingView.Desktop_3.5.1.8500_x64__n534cwy3pjxzj\resources\app.asar",
    ];
    let Some(asar) = candidates.iter().map(Path::new).find(|p| p.is_file()) else {
        eprintln!("跳过：没有可用的 app.asar");
        return;
    };
    let tmp = std::env::temp_dir().join("tvx-asar-test");
    let _ = fs::remove_dir_all(&tmp);
    let st = extract(asar, &tmp).expect("解包失败");
    eprintln!("{} 个文件, {} 个链接被跳过", st.files, st.skipped_links);
    assert!(st.files > 100, "文件数太少，解包可疑");
    assert!(tmp.join("index.js").is_file(), "缺少 index.js");
    assert!(tmp.join("package.json").is_file(), "缺少 package.json");
    assert!(tmp.join("keytar.node").is_file(), "unpacked 原生模块没带出来");
    let _ = fs::remove_dir_all(&tmp);
}
