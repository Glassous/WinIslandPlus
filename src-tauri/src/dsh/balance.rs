//! 官方账户余额：只在 dsh 使用官方登录时查询。
//!
//! 数据来源：`GET https://api.deepseek.com/user/balance`（Bearer 认证），
//! 密钥取自 dsh 自己的凭据文件 `~/.dsh/.credentials.yaml` 里的 `DEEPSEEK_API_KEY`。
//!
//! 实现要点：
//! - 请求跑在**独立线程**里（TLS 握手不可预测，绝不阻塞状态轮询线程）；
//! - 每 5 分钟最多查询一次；系统开了代理就用代理，否则直连；
//! - 设置了连接 / 接收超时，失败静默重试；
//! - 密钥只用于本次请求，**不写日志、不进快照**。

use std::sync::{Arc, Mutex};

use serde_json::Value;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Networking::WinHttp::{
    WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_NAMED_PROXY,
    WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_FLAG_SECURE,
};

use crate::log;

const HOST: &str = "api.deepseek.com";
const PATH: &str = "/user/balance";
/// 余额刷新间隔。
const REFRESH_MS: f64 = 5.0 * 60.0 * 1000.0;

type Shared = Arc<Mutex<Option<(f64, String)>>>;

/// 余额缓存：查询在后台线程完成，轮询线程只读结果。
pub struct BalanceCache {
    shared: Shared,
    last_kick: f64,
    started: bool,
    disabled: bool,
}

impl BalanceCache {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(None)),
            last_kick: 0.0,
            started: false,
            disabled: false,
        }
    }

    /// 到点就踢一次后台查询（立即返回，不阻塞）。
    pub fn refresh(&mut self, now: f64) {
        if self.disabled {
            return;
        }
        if self.started && now - self.last_kick < REFRESH_MS {
            return;
        }

        if !self.started {
            if api_key().is_none() {
                self.disabled = true;
                log::info("dsh: 未找到可用的 DEEPSEEK_API_KEY，跳过余额查询");
                return;
            }
        }

        self.started = true;
        self.last_kick = now;

        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name("winisland-balance".into())
            .spawn(move || {
                if let Some(pair) = worker::fetch() {
                    if let Ok(mut guard) = shared.lock() {
                        *guard = Some(pair);
                    }
                }
            })
            .ok();
    }

    pub fn value(&self) -> Option<(f64, String)> {
        self.shared.lock().ok().and_then(|guard| guard.clone())
    }
}

impl Default for BalanceCache {
    fn default() -> Self {
        Self::new()
    }
}

/// 从 dsh 凭据文件里取官方 API Key（只看 `DEEPSEEK_API_KEY:` 这一行）。
fn api_key() -> Option<String> {
    let home = crate::dsh::session::dsh_home()?;
    let text = std::fs::read_to_string(home.join(".credentials.yaml")).ok()?;

    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("DEEPSEEK_API_KEY") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let key = value.trim().trim_matches('"').trim_matches('\'').to_string();
        if key.starts_with("sk-") && key.len() > 8 {
            return Some(key);
        }
    }

    None
}

/// 系统代理（IE / 注册表配置）。开了才用，否则直连。
fn system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;

    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return None;
    }

    let server: String = key.get_value("ProxyServer").ok()?;
    let server = server.trim().to_string();
    if server.is_empty() {
        return None;
    }

    // 形如 "127.0.0.1:7897" 或 "http=host:port;https=host:port"
    let host = if server.contains('=') {
        server
            .split(';')
            .filter_map(|part| {
                let part = part.trim();
                part.strip_prefix("https=")
                    .or_else(|| part.strip_prefix("http="))
            })
            .next()
            .map(|value| value.to_string())?
    } else {
        server
    };

    // WinHTTP 的代理串是 "host:port"，不能带 http:// 前缀
    let host = host
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .to_string();

    Some(host)
}

mod worker {
    use super::*;

    pub fn fetch() -> Option<(f64, String)> {
        let key = api_key()?;
        let body = match https_get(&key) {
            Ok(body) => body,
            Err(reason) => {
                if !WARNED.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    log::warn(&format!("dsh: 余额请求失败（{reason}）"));
                }
                return None;
            }
        };
        let value = serde_json::from_str::<Value>(&body).ok()?;

        let info = value.get("balance_infos")?.as_array()?.first()?.clone();
        let currency = info
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or("CNY")
            .to_string();

        // total_balance 是字符串（如 "110.00"），也兼容数字
        let total = info
            .get("total_balance")
            .and_then(|value| {
                value
                    .as_f64()
                    .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
            })
            .filter(|amount| amount.is_finite())?;

        Some((total, currency))
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn https_get(key: &str) -> Result<String, String> {
        let agent = wide("WinIslandPlus/1.1");
        let host = wide(HOST);
        let path = wide(PATH);
        let verb = wide("GET");
        let headers = wide(&format!(
            "Authorization: Bearer {key}\r\nAccept: application/json\r\n"
        ));

        // 有系统代理就传进 WinHttpOpen（WinHTTP 默认不走 IE / 注册表代理）；
        // 注意：NAMED_PROXY 必须同时给出代理地址，否则 WinHttpOpen 直接返回错误 87。
        let proxy = system_proxy();
        let proxy_wide = proxy.as_ref().map(|value| wide(value));
        let access_type = if proxy_wide.is_some() {
            WINHTTP_ACCESS_TYPE_NAMED_PROXY
        } else {
            WINHTTP_ACCESS_TYPE_NO_PROXY
        };
        let proxy_ptr = proxy_wide
            .as_ref()
            .map(|value| PCWSTR(value.as_ptr()))
            .unwrap_or(PCWSTR::null());

        unsafe {
            let session = WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                access_type,
                proxy_ptr,
                PCWSTR::null(),
                0,
            );
            if session.is_null() {
                return Err(format!(
                    "open proxy={:?} err={}",
                    proxy,
                    windows::Win32::Foundation::GetLastError().0
                ));
            }

            // 解析 4s / 连接 4s / 发送 4s / 接收 6s：失败也只是慢一点，不会挂死线程
            let _ = WinHttpSetTimeouts(session, 4000, 4000, 4000, 6000);

            let connect = WinHttpConnect(session, PCWSTR(host.as_ptr()), 443, 0);
            if connect.is_null() {
                let code = windows::Win32::Foundation::GetLastError().0;
                let _ = CloseHandle(HANDLE(session));
                return Err(format!("connect err={code}"));
            }

            let request = WinHttpOpenRequest(
                connect,
                PCWSTR(verb.as_ptr()),
                PCWSTR(path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            );
            if request.is_null() {
                let code = windows::Win32::Foundation::GetLastError().0;
                let _ = CloseHandle(HANDLE(connect));
                let _ = CloseHandle(HANDLE(session));
                return Err(format!("request err={code}"));
            }

            // 注意：必须剥掉结尾的 NUL，否则 WinHTTP 会以 E_INVALIDARG 拒绝这次发送
            let header_slice = &headers[..headers.len().saturating_sub(1)];
            let sent = WinHttpSendRequest(request, Some(header_slice), None, 0, 0, 0);
            let mut body = Vec::new();

            if let Err(err) = &sent {
                let _ = CloseHandle(HANDLE(request));
                let _ = CloseHandle(HANDLE(connect));
                let _ = CloseHandle(HANDLE(session));
                return Err(format!("send {err:?}"));
            }

            if let Err(err) = WinHttpReceiveResponse(request, std::ptr::null_mut()) {
                let _ = CloseHandle(HANDLE(request));
                let _ = CloseHandle(HANDLE(connect));
                let _ = CloseHandle(HANDLE(session));
                return Err(format!("receive {err:?}"));
            }

            {
                let mut chunk = [0u8; 4096];
                loop {
                    let mut available = 0u32;
                    if WinHttpQueryDataAvailable(request, &mut available).is_err() || available == 0
                    {
                        break;
                    }

                    let to_read = available.min(chunk.len() as u32);
                    let mut read = 0u32;
                    if WinHttpReadData(
                        request,
                        chunk.as_mut_ptr() as *mut core::ffi::c_void,
                        to_read,
                        &mut read,
                    )
                    .is_err()
                        || read == 0
                    {
                        break;
                    }

                    body.extend_from_slice(&chunk[..read as usize]);
                    if body.len() > 64 * 1024 {
                        break;
                    }
                }
            }

            let _ = CloseHandle(HANDLE(request));
            let _ = CloseHandle(HANDLE(connect));
            let _ = CloseHandle(HANDLE(session));

            if body.is_empty() {
                return Err("empty body".to_string());
            }
            String::from_utf8(body).map_err(|err| format!("utf8 {err}"))
        }
    }
}

/// 只提示一次失败，避免日志刷屏。
static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
