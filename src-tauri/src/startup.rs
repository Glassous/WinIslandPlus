//! 开机自启：写入 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。
//!
//! 全程不需要管理员权限，也不依赖第三方插件，值名与启动参数完全可控。

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

use crate::log;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "WinIslandPlus";

/// 当前进程可执行文件路径。
pub fn executable_path() -> String {
    std::env::current_exe()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default()
}

pub fn is_enabled() -> bool {
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ) else {
        return false;
    };

    let Ok(value) = key.get_value::<String, _>(VALUE_NAME) else {
        return false;
    };

    let recorded = value.trim().trim_matches('"');
    recorded.eq_ignore_ascii_case(&executable_path())
}

pub fn set_enabled(enabled: bool) -> bool {
    let result = (|| -> std::io::Result<()> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_WRITE)?;
        if enabled {
            let command = format!("\"{}\" --autostart", executable_path());
            key.set_value(VALUE_NAME, &command)?;
        } else {
            match key.delete_value(VALUE_NAME) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            log::info(&format!("startup: 自启已设置为 {enabled}"));
            true
        }
        Err(err) => {
            log::error(&format!("startup: 写入自启失败 {err}"));
            false
        }
    }
}

/// 系统当前是否为深色主题（用于原生标题栏）。
pub fn is_dark_mode() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            KEY_READ,
        )
        .and_then(|key| key.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|light| light == 0)
        .unwrap_or(true)
}
