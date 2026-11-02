//! 「dsh 是否在运行」的轻量判定。
//!
//! 桌面端为一组 `DeepSeek Harness.exe` 进程，CLI 为 `dsh` / `dsh.exe`。
//! 用 Toolhelp 快照遍历进程名即可，无需权限、无额外依赖。

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// 进程名匹配：桌面端 + CLI。
fn is_dsh_process(name: &str) -> bool {
    let name = name.trim().trim_end_matches('\0').to_ascii_lowercase();
    name == "deepseek harness.exe" || name == "dsh.exe" || name == "dsh"
}

/// 是否有 dsh 进程在运行。
pub fn dsh_running() -> bool {
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        let mut found = false;
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = String::from_utf16_lossy(
                    &entry.szExeFile[..entry
                        .szExeFile
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(entry.szExeFile.len())],
                );
                if is_dsh_process(&name) {
                    found = true;
                    break;
                }

                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
        found
    }
}
