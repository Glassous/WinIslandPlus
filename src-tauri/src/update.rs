//! 版本更新检测：只查 GitHub Releases 的 latest，不下载、不弹窗、不打断用户。
//!
//! 结果仅用于设置窗口里的「发现新版本」一行文字，点击后交给系统浏览器打开发布页。
//! 任一环节失败都静默降级：宁可不提示，也不打扰。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::log;

/// 发布仓库（与 `git remote` 保持一致）。
pub const RELEASES_URL: &str = "https://github.com/Glassous/WinIslandPlus/releases/latest";

const API_LATEST: &str = "https://api.github.com/repos/Glassous/WinIslandPlus/releases/latest";

/// GitHub API 要求带 User-Agent，否则直接 403。
const USER_AGENT: &str = concat!("WinIslandPlus/", env!("CARGO_PKG_VERSION"));

/// 检测结果的有效期：常驻进程里没必要频繁请求，也避开未认证的 60 次/小时限流。
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// 启动后延迟首轮检测：把网络开销让到窗口稳定之后。
const STARTUP_DELAY: Duration = Duration::from_secs(15);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// 检测结果变化时广播（设置窗口据此刷新那一行文字）。
pub const EVENT_UPDATE_STATUS: &str = "update://status";

/// 检测结果（同时作为 IPC 契约，字段名与 `src/common/types.ts` 一一对应）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct UpdateStatus {
    pub current_version: String,
    /// 最新发布版本号（去掉 `v` 前缀）；尚未检测成功时为 null。
    pub latest_version: Option<String>,
    pub has_update: bool,
    /// 新版本发布页；无更新时为 null。
    pub release_url: Option<String>,
    /// 后台正在检测。
    pub checking: bool,
    /// 是否至少成功检测过一次（失败时保持 false，界面据此不做任何提示）。
    pub checked: bool,
}

impl UpdateStatus {
    fn fresh() -> Self {
        Self {
            current_version: current_version(),
            latest_version: None,
            has_update: false,
            release_url: None,
            checking: false,
            checked: false,
        }
    }
}

struct LatestRelease {
    /// 去掉 `v` 前缀的版本号。
    version: String,
    url: String,
}

struct Inner {
    status: Mutex<UpdateStatus>,
    last_check: Mutex<Option<Instant>>,
    /// 已有检测线程在跑：避免启动检测与设置窗口同时触发多个请求。
    checking: AtomicBool,
}

/// 更新检测服务（克隆成本极低，内部 `Arc` 共享）。
#[derive(Clone)]
pub struct UpdateService {
    inner: Arc<Inner>,
}

impl UpdateService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                status: Mutex::new(UpdateStatus::fresh()),
                last_check: Mutex::new(None),
                checking: AtomicBool::new(false),
            }),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.inner
            .status
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| UpdateStatus::fresh())
    }

    /// 启动后延迟一轮静默检测（失败不影响任何功能）。
    pub fn start(&self, app: &AppHandle) {
        let service = self.clone();
        let app = app.clone();
        let _ = std::thread::Builder::new()
            .name("winisland-update".into())
            .spawn(move || {
                std::thread::sleep(STARTUP_DELAY);
                service.refresh_if_stale(&app);
            });
    }

    /// 结果过期（或从未检测）时发起一次后台检测；并发调用只会真正跑一份。
    pub fn refresh_if_stale(&self, app: &AppHandle) {
        if !self.is_stale() || self.inner.checking.swap(true, Ordering::SeqCst) {
            return;
        }

        self.mutate(|status| status.checking = true);
        let _ = app.emit(EVENT_UPDATE_STATUS, self.status());

        let service = self.clone();
        let app = app.clone();
        let spawned = std::thread::Builder::new()
            .name("winisland-update".into())
            .spawn(move || {
                let outcome = fetch_latest();
                service.apply(outcome);
                service.inner.checking.store(false, Ordering::SeqCst);
                let _ = app.emit(EVENT_UPDATE_STATUS, service.status());
            });

        if spawned.is_err() {
            log::warn("update: 无法创建检测线程");
            self.inner.checking.store(false, Ordering::SeqCst);
            self.mutate(|status| status.checking = false);
        }
    }

    fn is_stale(&self) -> bool {
        match self.inner.last_check.lock() {
            Ok(guard) => match *guard {
                Some(at) => at.elapsed() >= CHECK_INTERVAL,
                None => true,
            },
            Err(_) => true,
        }
    }

    fn apply(&self, outcome: Result<LatestRelease, String>) {
        // 失败也算「查过了」：避免网络不通时每次打开设置都重试。
        if let Ok(mut guard) = self.inner.last_check.lock() {
            *guard = Some(Instant::now());
        }

        match outcome {
            Ok(release) => {
                let current = current_version();
                let has_update = is_newer(&release.version, &current);

                self.mutate(|status| {
                    status.latest_version = Some(release.version.clone());
                    status.has_update = has_update;
                    status.release_url = Some(release.url.clone());
                    status.checked = true;
                    status.checking = false;
                });

                if has_update {
                    log::info(&format!(
                        "update: 发现新版本 {}（当前 {current}）",
                        release.version
                    ));
                } else {
                    log::info(&format!("update: 已是最新版本 {current}"));
                }
            }
            Err(err) => {
                // 保留上一次结论：一次网络抖动不该把「有新版本」抹掉。
                log::warn(&format!("update: 检测失败 {err}"));
                self.mutate(|status| status.checking = false);
            }
        }
    }

    fn mutate(&self, f: impl FnOnce(&mut UpdateStatus)) {
        if let Ok(mut guard) = self.inner.status.lock() {
            f(&mut guard);
        }
    }
}

fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn fetch_latest() -> Result<LatestRelease, String> {
    let response = ureq::get(API_LATEST)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .timeout(REQUEST_TIMEOUT)
        .call()
        .map_err(|err| err.to_string())?;

    let body: serde_json::Value = response.into_json().map_err(|err| err.to_string())?;

    let tag = body
        .get("tag_name")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "响应缺少 tag_name".to_string())?;

    let url = body
        .get("html_url")
        .and_then(|value| value.as_str())
        .unwrap_or(RELEASES_URL);

    Ok(LatestRelease {
        version: tag.trim().trim_start_matches('v').trim_start_matches('V').to_string(),
        url: url.to_string(),
    })
}

/// 取版本号的数字段（忽略 `-beta.1` / `+build` 之类的后缀）。
fn numeric_parts(version: &str) -> Vec<u64> {
    let core = version
        .split(|c: char| c == '-' || c == '+')
        .next()
        .unwrap_or(version);

    core.split('.')
        .map(|part| part.trim().parse::<u64>().unwrap_or(0))
        .collect()
}

/// `latest` 是否比 `current` 新（逐段按数字比较，缺位补 0）。
fn is_newer(latest: &str, current: &str) -> bool {
    let latest = numeric_parts(latest);
    let current = numeric_parts(current);
    let len = latest.len().max(current.len());

    for index in 0..len {
        let a = *latest.get(index).unwrap_or(&0);
        let b = *current.get(index).unwrap_or(&0);
        if a != b {
            return a > b;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_numeric_segments() {
        assert!(is_newer("1.1.11", "1.1.10"));
        assert!(is_newer("1.2.0", "1.1.99"));
        assert!(!is_newer("1.1.10", "1.1.10"));
        assert!(!is_newer("1.0.9", "1.1.0"));
        assert!(!is_newer("1.1", "1.1.0"));
        assert!(is_newer("1.1.1", "1.1"));
    }
}
