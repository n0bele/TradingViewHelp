//! protocol 模块的回归测试。

use super::*;

#[test]
#[cfg(windows)]
fn windows_roundtrip() {
    use std::path::Path;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let path = r"Software\Classes\tradingview\shell\open\command";
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let original: Option<String> = hkcu
        .open_subkey(path)
        .ok()
        .and_then(|k| k.get_value("").ok());

    set_target(Path::new("tvx.exe"), None).expect("写入失败");
    let with: String = hkcu.open_subkey(path).unwrap().get_value("").unwrap();
    assert!(with.contains("_auth"), "应指向 tvx _auth：{with}");
    assert!(with.ends_with("\"%1\""), "应带 %1 占位：{with}");

    if let Some(o) = original {
        let (k, _) = hkcu.create_subkey(path).unwrap();
        k.set_value("", &o).unwrap();
    }
}

#[test]
#[cfg(windows)]
fn windows_command_line_shape() {
    let cmd = command_line();
    assert!(cmd.starts_with('"'), "路径要加引号：{cmd}");
    assert!(cmd.contains("_auth"), "必须走 tvx _auth：{cmd}");
    assert!(cmd.ends_with("\"%1\""), "必须以 %1 结尾：{cmd}");
}
