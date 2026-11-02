//! 轻量按日文件日志。
//!
//! 只记录异常与状态迁移，**绝不**写入曲目标题、艺术家或封面字节。
//! 所有失败都静默降级（宁可没有日志，也不能影响主流程）。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// `%LOCALAPPDATA%\WinIslandPlus`
pub fn root_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("LOCALAPPDATA") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("WinIslandPlus");
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn logs_dir() -> PathBuf {
    root_dir().join("logs")
}

/// 初始化日志文件（应用启动时调用一次）。
pub fn init() {
    let dir = logs_dir();
    let _ = fs::create_dir_all(&dir);

    let (date, _) = now_parts();
    let path = dir.join(format!("app-{date}.log"));
    let _ = LOG_PATH.set(path);
    info("app: 日志已初始化");
}

pub fn info(message: &str) {
    write("INFO ", message);
}

pub fn warn(message: &str) {
    write("WARN ", message);
}

pub fn error(message: &str) {
    write("ERROR", message);
}

fn write(level: &str, message: &str) {
    let line = {
        let (date, time) = now_parts();
        format!("{date} {time} [{level}] {message}\n")
    };

    if cfg!(debug_assertions) {
        eprint!("{line}");
    }

    let Some(path) = LOG_PATH.get() else {
        return;
    };

    let Ok(_guard) = LOG_LOCK.lock() else {
        return;
    };

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
}

fn now_parts() -> (String, String) {
    let parts = local_time();
    (
        format!("{:04}{:02}{:02}", parts.0, parts.1, parts.2),
        format!("{:02}:{:02}:{:02}", parts.3, parts.4, parts.5),
    )
}

/// `yyyyMMdd` 本地日期戳，供日志文件名与配置备份名共用。
pub fn date_stamp() -> String {
    now_parts().0
}

/// `HHmmss` 本地时间戳（去掉了冒号，可直接拼进文件名）。
pub fn time_stamp() -> String {
    now_parts().1.replace(':', "")
}

#[cfg(windows)]
fn local_time() -> (i32, u32, u32, u32, u32, u32) {
    use windows::Win32::System::SystemInformation::GetLocalTime;

    let st = unsafe { GetLocalTime() };
    (
        st.wYear as i32,
        st.wMonth as u32,
        st.wDay as u32,
        st.wHour as u32,
        st.wMinute as u32,
        st.wSecond as u32,
    )
}

#[cfg(not(windows))]
fn local_time() -> (i32, u32, u32, u32, u32, u32) {
    (1970, 1, 1, 0, 0, 0)
}

/// 保证目录存在（供设置 / 日志等写入前调用）。
pub fn ensure_root() -> PathBuf {
    let dir = root_dir();
    let _ = fs::create_dir_all(&dir);
    dir
}
